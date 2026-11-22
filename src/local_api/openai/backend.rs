//! What turns one uploaded file into the text a paste in Global mode would
//! produce. The file is staged in a temp file only because the shared pieces
//! reader opens a path; every path deletes it. It is decoded in pieces of at
//! most 30 s, so each piece runs on the dictation engine's model while it
//! holds that exact model and language; a request language it lacks makes the
//! service load its own copy for that request. The joined transcript
//! goes through the dictation pipeline and nothing else: no paste, no
//! History, no Paste Last, no clipboard.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use super::reply::{Failure, Outcome};
use super::{Transcribe, Upload};
use crate::local_api::processing::Processor;
use crate::transcription::models::{self, ModelRuntime, TranscriptionSelection};
use crate::transcription::pieces::{PieceError, Pieces};
use crate::transcription::service::{
    AudioClip, AudioError, TranscriptionService, TranscriptionServiceError,
    TranscriptionServiceHandle,
};
use crate::transcription::transcript::TimestampLevel;

/// One upload, from its first byte to its reply.
const REQUEST_DEADLINE: Duration = Duration::from_secs(10 * 60);
/// How often a waiting piece checks for shutdown and its deadline.
const POLL: Duration = Duration::from_millis(100);
/// A staged file this old belongs to a Hex that died mid-request.
const STALE_AFTER: Duration = Duration::from_secs(10 * 60);
const FOLDER: &str = "hex-transcription-endpoint";

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

/// What cancels a piece: Hex shutting down, or the request's own deadline.
pub(crate) struct Limits<'a> {
    pub(crate) shutdown: &'a AtomicBool,
    pub(crate) deadline: Instant,
}

/// The model and language for one request, or why none can run. `requested`
/// is the client's language, already `None` for `auto`.
pub(crate) type Select =
    Box<dyn Fn(Option<&str>) -> Result<TranscriptionSelection, Failure> + Send + Sync>;

/// The text of one piece of at most 30 s of 16 kHz audio.
pub(crate) type RunPiece = Box<
    dyn Fn(&TranscriptionSelection, Vec<f32>, &Limits<'_>) -> Result<String, Failure> + Send + Sync,
>;

pub(crate) struct Backend {
    select: Select,
    // Dropped before `service`: the service's worker ends once every sender,
    // the one this closure holds included, is gone.
    run_piece: RunPiece,
    processor: Processor,
    shutdown: Arc<AtomicBool>,
    /// One request at a time; a second one is told Hex is busy.
    busy: AtomicBool,
    folder: PathBuf,
    _service: Option<TranscriptionService>,
}

impl Backend {
    pub(crate) fn new(
        select: Select,
        run_piece: RunPiece,
        processor: Processor,
        shutdown: Arc<AtomicBool>,
        service: Option<TranscriptionService>,
    ) -> Self {
        let folder = std::env::temp_dir().join(FOLDER);
        sweep(&folder);
        Self {
            select,
            run_piece,
            processor,
            shutdown,
            busy: AtomicBool::new(false),
            folder,
            _service: service,
        }
    }

    /// Stages uploads in `folder` instead of the system temp folder.
    #[cfg(test)]
    pub(crate) fn in_folder(mut self, folder: PathBuf) -> Self {
        self.folder = folder;
        self
    }

    /// The running app's backend: the selection Hex's window holds, the
    /// shared transcription service, and the Global-mode processor.
    pub(crate) fn for_app(
        selection: Arc<RwLock<TranscriptionSelection>>,
        processor: Processor,
        shutdown: Arc<AtomicBool>,
    ) -> color_eyre::Result<Transcribe> {
        let (service, handle) = TranscriptionService::start()?;
        Ok(Self::new(
            selected(selection),
            service_piece(handle),
            processor,
            shutdown,
            Some(service),
        )
        .into_transcribe())
    }

    pub(crate) fn into_transcribe(self) -> Transcribe {
        let backend = Arc::new(self);
        Arc::new(move |upload| backend.run(upload))
    }

    fn run(&self, upload: Upload) -> Result<Outcome, Failure> {
        let _busy = Busy::claim(&self.busy).ok_or_else(Failure::busy)?;
        let selection = (self.select)(upload.language.as_deref())?;
        let file = Staged::write(&self.folder, &upload.bytes).map_err(|error| {
            tracing::warn!(%error, "could not stage the endpoint's audio");
            Failure::server("Hex could not stage the audio.")
        })?;
        drop(upload);
        let mut pieces = Pieces::open(file.path()).map_err(piece_failure)?;
        pieces.validate().map_err(piece_failure)?;
        let limits = Limits {
            shutdown: &self.shutdown,
            deadline: Instant::now() + REQUEST_DEADLINE,
        };
        let mut texts = Vec::new();
        let mut samples = 0_u64;
        while let Some(piece) = pieces.next().map_err(piece_failure)? {
            if self.shutdown.load(Ordering::Acquire) {
                return Err(shutting_down());
            }
            if Instant::now() >= limits.deadline {
                return Err(Failure::timeout());
            }
            samples += piece.len() as u64;
            let text = (self.run_piece)(&selection, piece, &limits)?;
            if !text.is_empty() {
                texts.push(text);
            }
        }
        drop(pieces);
        drop(file);
        let raw = texts.join(" ");
        let text = if raw.trim().is_empty() {
            String::new()
        } else {
            (self.processor)(&raw, &selection.language, &self.shutdown)
                .trim()
                .to_owned()
        };
        Ok(Outcome {
            text,
            duration_ms: samples * 1_000 / 16_000,
            language: selection.language,
        })
    }
}

fn shutting_down() -> Failure {
    Failure::unavailable("Hex is shutting down.")
}

/// The one request slot, freed when the request ends however it ends.
struct Busy<'a>(&'a AtomicBool);

impl<'a> Busy<'a> {
    fn claim(flag: &'a AtomicBool) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(flag))
    }
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// An uploaded file on disk for as long as this lives.
struct Staged(PathBuf);

impl Staged {
    fn write(folder: &Path, bytes: &[u8]) -> std::io::Result<Self> {
        fs::create_dir_all(folder)?;
        let path = folder.join(format!(
            "{}-{}.audio",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let staged = Self(path);
        file.write_all(bytes)?;
        file.flush()?;
        Ok(staged)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "could not delete the endpoint's staged audio");
        }
    }
}

/// Deletes staged files a dead Hex left behind.
fn sweep(folder: &Path) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > STALE_AFTER);
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn piece_failure(error: PieceError) -> Failure {
    match error {
        PieceError::Audio(AudioError::ResourceExhausted) => Failure::too_large(),
        PieceError::Audio(_) => {
            Failure::invalid("The audio's sample rate or channels are not supported.")
        }
        PieceError::NotMedia => Failure::invalid("The upload is not audio Hex can read."),
        PieceError::NoAudioTrack => Failure::invalid("The upload has no audio track."),
        PieceError::UnsupportedCodec(codec) => Failure::invalid(format!(
            "The audio uses a codec Hex cannot decode ({codec})."
        )),
        PieceError::Empty => Failure::invalid("The upload holds no audio."),
        PieceError::Incomplete | PieceError::Changed { .. } | PieceError::Damaged { .. } => {
            Failure::invalid("The audio is truncated or damaged.")
        }
        PieceError::Io(error) => {
            tracing::warn!(%error, "could not read the endpoint's staged audio");
            Failure::server("Hex could not read the staged audio.")
        }
    }
}

fn service_failure(error: TranscriptionServiceError) -> Failure {
    match error {
        TranscriptionServiceError::QueueFull => Failure::busy(),
        TranscriptionServiceError::Unavailable | TranscriptionServiceError::Cancelled => {
            Failure::unavailable("Hex's transcription service stopped.")
        }
        TranscriptionServiceError::Model(error) => {
            tracing::error!(%error, "the endpoint's speech model could not load");
            Failure::server("Hex's speech model could not be loaded.")
        }
        TranscriptionServiceError::Inference(error) => {
            tracing::error!(%error, "the endpoint's transcription failed");
            Failure::server("The transcription failed.")
        }
        TranscriptionServiceError::Timestamps => Failure::server("The transcription failed."),
    }
}

/// Selects the model and language Hex's window holds now; a client's
/// language replaces the language only when that model lists it.
pub(crate) fn selected(current: Arc<RwLock<TranscriptionSelection>>) -> Select {
    Box::new(move |requested| {
        let saved = current
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let selection = requested
            .map(|language| TranscriptionSelection {
                language: language.to_owned(),
                ..saved.clone()
            })
            .filter(|candidate| models::validate(candidate).is_ok())
            .unwrap_or(saved);
        let model = models::validate(&selection).map_err(|error| {
            Failure::unavailable(format!("Hex's selected speech model cannot run: {error}"))
        })?;
        let ready = match model.runtime {
            ModelRuntime::Gguf(_) => models::is_verified(model),
            ModelRuntime::AppleSpeech => models::is_installed(model, &selection.language),
        };
        if !ready {
            return Err(Failure::unavailable(
                "Hex's speech model is not downloaded or loaded yet.",
            ));
        }
        Ok(selection)
    })
}

/// Runs each piece on the transcription service, waiting for a cold model.
pub(crate) fn service_piece(handle: TranscriptionServiceHandle) -> RunPiece {
    Box::new(move |selection, samples, limits| {
        let duration_ms = samples.len() as u64 * 1_000 / 16_000;
        let clip = AudioClip {
            samples,
            duration_ms,
        };
        let admission = handle.admit_audio().ok_or_else(Failure::busy)?;
        let job = handle
            .transcribe(selection.clone(), clip, admission, TimestampLevel::None)
            .map_err(service_failure)?;
        loop {
            if limits.shutdown.load(Ordering::Acquire) {
                job.cancel();
                return Err(shutting_down());
            }
            match job.wait_timeout(POLL) {
                Ok(Some(Ok(transcript))) => return Ok(transcript.text.trim().to_owned()),
                Ok(Some(Err(error))) | Err(error) => return Err(service_failure(error)),
                Ok(None) if Instant::now() >= limits.deadline => {
                    job.cancel();
                    return Err(Failure::timeout());
                }
                Ok(None) => {}
            }
        }
    })
}
