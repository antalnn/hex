//! Silero voice activity detection, used to keep audio without speech away
//! from the transcription models, which tend to invent text for it.
//!
//! The detector is optional. While its file is missing it downloads in the
//! background, and every caller falls back to its level-only silence check.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard};

use color_eyre::eyre::Result;
use transcribe_cpp::{Backend, Model, ModelOptions, VadOptions, VadSession};

use crate::transcription::models as transcription_models;

/// Speech shorter than this is ignored. Silero's default of 250 ms drops a
/// short "Ok" or "Ciao"; 160 ms keeps those and still rejects clicks, keys
/// and room noise.
const MIN_SPEECH_MS: i32 = 160;

/// Speech probability that counts as speech. Silero's default of 0.5 misses
/// a whisper at -43 dBFS, which the level gate deliberately keeps; 0.4 hears
/// it and still rejects the silence in a recorded meeting.
pub(crate) const THRESHOLD: f64 = 0.4;

#[cfg_attr(test, allow(dead_code))]
enum State {
    Unloaded,
    Loaded(VadSession),
    Failed,
}

#[cfg_attr(test, allow(dead_code))]
static STATE: Mutex<State> = Mutex::new(State::Unloaded);
#[cfg_attr(test, allow(dead_code))]
static DOWNLOAD_STARTED: AtomicBool = AtomicBool::new(false);
#[cfg_attr(test, allow(dead_code))]
static RUN_FAILURE_LOGGED: AtomicBool = AtomicBool::new(false);

/// Whether 16 kHz mono `samples` contain speech, or `None` when the detector
/// is not available and the caller should keep its current behavior.
pub(crate) fn contains_speech(samples: &[f32]) -> Option<bool> {
    #[cfg(test)]
    {
        let _ = samples;
        OVERRIDE.with(|verdict| verdict.get())
    }
    #[cfg(not(test))]
    {
        let mut state = lock_state();
        let session = loaded_session(&mut state)?;
        match detect(session, samples) {
            Ok(speech) => Some(speech),
            Err(error) => {
                if !RUN_FAILURE_LOGGED.swap(true, Ordering::Relaxed) {
                    tracing::warn!(%error, "speech detection failed");
                }
                None
            }
        }
    }
}

/// Loads an installed detector in the background so the first dictation does
/// not pay for it. It never downloads: preparation at launch stays offline.
pub(crate) fn prewarm() {
    #[cfg(not(test))]
    if transcription_models::speech_detector_installed() {
        let _ = std::thread::Builder::new()
            .name("hex-speech-detector".into())
            .spawn(|| {
                let _ = loaded_session(&mut lock_state());
            });
    }
}

#[cfg_attr(test, allow(dead_code))]
fn lock_state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|error| error.into_inner())
}

/// The loaded session, loading it on first use. A missing detector starts a
/// background download and stays unloaded, so it loads once the file lands.
#[cfg_attr(test, allow(dead_code))]
fn loaded_session(state: &mut State) -> Option<&mut VadSession> {
    if matches!(state, State::Unloaded) {
        if !transcription_models::speech_detector_installed() {
            start_download();
            return None;
        }
        *state = match transcription_models::speech_detector_path().and_then(|path| load(&path)) {
            Ok(session) => State::Loaded(session),
            Err(error) => {
                tracing::warn!(%error, "could not load the speech detector");
                State::Failed
            }
        };
    }
    match state {
        State::Loaded(session) => Some(session),
        State::Unloaded | State::Failed => None,
    }
}

/// Starts a background download of the detector unless one is running.
/// Tests only count the request.
pub(crate) fn start_download() {
    #[cfg(test)]
    DOWNLOAD_REQUESTS.with(|requests| requests.set(requests.get() + 1));
    #[cfg(not(test))]
    start_download_once();
}

#[cfg(not(test))]
fn start_download_once() {
    if DOWNLOAD_STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("hex-speech-detector-download".into())
        .spawn(|| {
            if let Err(error) =
                transcription_models::download_speech_detector(&AtomicBool::new(false))
            {
                tracing::warn!(%error, "could not download the speech detector");
                // The next dictation or meeting chunk tries again.
                DOWNLOAD_STARTED.store(false, Ordering::Relaxed);
            }
        });
}

/// Loads the detector on the CPU: it loads in tens of milliseconds there,
/// while a first Metal start can take seconds and would hold up dictation.
fn load(path: &Path) -> Result<VadSession> {
    // Its load would otherwise register every backend unguarded (#662).
    #[cfg(target_os = "windows")]
    crate::transcription::local_transcriber::init_backends()?;
    let model = Model::load_with(
        path,
        &ModelOptions {
            backend: Backend::Cpu,
            ..Default::default()
        },
    )?;
    Ok(model.vad_session()?)
}

fn detect(session: &mut VadSession, samples: &[f32]) -> Result<bool> {
    let options = VadOptions {
        threshold: THRESHOLD,
        min_speech_ms: MIN_SPEECH_MS,
        ..VadOptions::default()
    };
    Ok(!session.run(samples, &options)?.segments.is_empty())
}

/// Silence kept on each side of a speech span when a whole clip is trimmed.
/// Wide enough to keep a soft onset the detector scored late, narrow enough
/// that a thinking pause still goes.
const TRIM_PAD_MS: i32 = 200;

/// A trim that saves less than this is not worth a shorter clip: the model
/// gains nothing and the only risk left is a clipped word.
const MIN_TRIMMED_SAMPLES: usize = 4_000;

/// `samples` with everything outside its speech removed, or `None` when the
/// detector is unavailable, hears no speech at all, or would save less than a
/// quarter second.
pub(crate) fn without_silence(samples: &[f32]) -> Option<Vec<f32>> {
    trim_with(samples, speech_ranges(samples, TRIM_PAD_MS))
}

/// Speech ranges of 16 kHz mono `samples`, padded by `pad_ms` and merged
/// where the padding meets. `None` when the detector is not available.
pub(crate) fn speech_ranges(samples: &[f32], pad_ms: i32) -> Option<Vec<(usize, usize)>> {
    #[cfg(test)]
    {
        let _ = (samples, pad_ms);
        None
    }
    #[cfg(not(test))]
    {
        let mut state = lock_state();
        let session = loaded_session(&mut state)?;
        ranges_from(session, samples, pad_ms)
    }
}

/// One detector run over `samples`, padded and merged.
#[cfg_attr(test, allow(dead_code))]
fn ranges_from(
    session: &mut VadSession,
    samples: &[f32],
    pad_ms: i32,
) -> Option<Vec<(usize, usize)>> {
    let options = VadOptions {
        threshold: THRESHOLD,
        min_speech_ms: MIN_SPEECH_MS,
        speech_pad_ms: pad_ms,
        ..VadOptions::default()
    };
    match session.run(samples, &options) {
        Ok(result) => {
            let spans: Vec<(i64, i64)> = result
                .segments
                .iter()
                .map(|segment| (segment.start_sample, segment.end_sample))
                .collect();
            Some(merged_ranges(&spans, samples.len()))
        }
        Err(error) => {
            if !RUN_FAILURE_LOGGED.swap(true, Ordering::Relaxed) {
                tracing::warn!(%error, "speech detection failed");
            }
            None
        }
    }
}

/// The detector's answer applied to the clip: the speech spans joined, or
/// `None` when there is nothing to trim or too little to gain.
fn trim_with(samples: &[f32], ranges: Option<Vec<(usize, usize)>>) -> Option<Vec<f32>> {
    let ranges = ranges?;
    if ranges.is_empty() {
        return None;
    }
    let kept = keep_ranges(samples, &ranges);
    (samples.len().saturating_sub(kept.len()) >= MIN_TRIMMED_SAMPLES).then_some(kept)
}

/// `ranges` clamped to `0..len`, empty ones dropped, sorted, and the ones
/// that touch merged.
#[cfg_attr(test, allow(dead_code))]
fn merged_ranges(ranges: &[(i64, i64)], len: usize) -> Vec<(usize, usize)> {
    let limit = len as i64;
    let mut clamped: Vec<(usize, usize)> = ranges
        .iter()
        .filter_map(|&(start, end)| {
            let start = start.clamp(0, limit) as usize;
            let end = end.clamp(0, limit) as usize;
            (end > start).then_some((start, end))
        })
        .collect();
    clamped.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(clamped.len());
    for (start, end) in clamped {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// The concatenation of `samples[ranges]`.
#[cfg_attr(test, allow(dead_code))]
fn keep_ranges(samples: &[f32], ranges: &[(usize, usize)]) -> Vec<f32> {
    let mut kept = Vec::with_capacity(samples.len());
    for &(start, end) in ranges {
        kept.extend_from_slice(&samples[start..end]);
    }
    kept
}

#[cfg(test)]
thread_local! {
    static OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static DOWNLOAD_REQUESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many detector downloads this thread requested.
#[cfg(test)]
pub(crate) fn download_requests_for_test() -> usize {
    DOWNLOAD_REQUESTS.with(std::cell::Cell::get)
}

/// Makes `contains_speech` answer `verdict` on this thread until the guard
/// drops. Tests never load the real detector.
#[cfg(test)]
pub(crate) fn override_for_test(verdict: Option<bool>) -> OverrideGuard {
    OverrideGuard(OVERRIDE.with(|current| current.replace(verdict)))
}

#[cfg(test)]
pub(crate) struct OverrideGuard(Option<bool>);

#[cfg(test)]
impl Drop for OverrideGuard {
    fn drop(&mut self) {
        OVERRIDE.with(|current| current.set(self.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_decides_and_restores() {
        {
            let _guard = override_for_test(Some(false));
            assert_eq!(contains_speech(&[0.0; 16_000]), Some(false));
        }
        assert_eq!(contains_speech(&[0.0; 16_000]), None);
    }

    #[test]
    fn merged_ranges_clamps_sorts_and_joins_touching_spans() {
        let merged = merged_ranges(&[(30, 40), (0, 10), (10, 20), (-5, 3), (60, 90)], 50);
        assert_eq!(merged, vec![(0, 20), (30, 40)]);
    }

    #[test]
    fn trim_with_joins_the_speech_and_declines_a_trivial_saving() {
        let samples = vec![0.1_f32; 32_000];
        let kept = trim_with(&samples, Some(vec![(8_000, 12_000), (20_000, 24_000)])).unwrap();
        assert_eq!(kept.len(), 8_000);
        // No speech found: the clip stays whole.
        assert!(trim_with(&samples, Some(Vec::new())).is_none());
        // A saving under a quarter second is not worth the risk.
        assert!(trim_with(&samples, Some(vec![(0, 31_900)])).is_none());
    }

    #[test]
    fn a_corrupt_detector_file_is_not_loaded() {
        let path = std::env::temp_dir().join(format!(
            "hex-corrupt-speech-detector-{}.gguf",
            std::process::id()
        ));
        std::fs::write(&path, b"not a gguf file").unwrap();
        let loaded = load(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(loaded.is_err());
    }

    #[test]
    #[ignore = "requires HEX_VAD_MODEL (Silero VAD GGUF)"]
    fn real_detector_rejects_silence_and_noise() {
        let mut session = load(Path::new(&std::env::var("HEX_VAD_MODEL").unwrap())).unwrap();
        assert!(!detect(&mut session, &[0.0; 32_000]).unwrap());
        // -60 dBFS white noise from a fixed linear congruential generator.
        let mut seed = 0x2545_f491_u32;
        let noise: Vec<f32> = (0..32_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * 0.001 * 3f32.sqrt()
            })
            .collect();
        assert!(!detect(&mut session, &noise).unwrap());
    }

    #[test]
    #[ignore = "requires HEX_VAD_MODEL and HEX_VAD_SPEECH_WAV (16 kHz mono float speech)"]
    fn real_detector_trims_a_long_gap_between_two_speech_spans() {
        let mut session = load(Path::new(&std::env::var("HEX_VAD_MODEL").unwrap())).unwrap();
        let speech = clip("HEX_VAD_SPEECH_WAV");
        let mut samples = speech.clone();
        samples.resize(samples.len() + 32_000, 0.0);
        samples.extend_from_slice(&speech);
        let ranges = ranges_from(&mut session, &samples, TRIM_PAD_MS).unwrap();
        let kept = trim_with(&samples, Some(ranges)).unwrap();
        assert!(
            kept.len() < samples.len(),
            "the two-second gap should be gone"
        );
        assert!(kept.len() > speech.len(), "both speech spans should stay");
    }

    fn clip(variable: &str) -> Vec<f32> {
        let mut reader = hound::WavReader::open(std::env::var(variable).unwrap()).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        reader.samples::<f32>().map(Result::unwrap).collect()
    }

    #[test]
    #[ignore = "requires HEX_VAD_MODEL and HEX_VAD_SPEECH_WAV (16 kHz mono float speech)"]
    fn real_detector_hears_speech() {
        let mut session = load(Path::new(&std::env::var("HEX_VAD_MODEL").unwrap())).unwrap();
        assert!(detect(&mut session, &clip("HEX_VAD_SPEECH_WAV")).unwrap());
    }

    #[test]
    #[ignore = "requires HEX_VAD_MODEL and HEX_VAD_WHISPER_WAV (16 kHz mono float whisper at about -43 dBFS)"]
    fn real_detector_hears_quiet_whispers() {
        let mut session = load(Path::new(&std::env::var("HEX_VAD_MODEL").unwrap())).unwrap();
        assert!(detect(&mut session, &clip("HEX_VAD_WHISPER_WAV")).unwrap());
    }
}
