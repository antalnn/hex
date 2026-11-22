use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use color_eyre::eyre::{Result, eyre};

use crate::audio::linger::MicrophoneLinger;
use crate::audio::{CaptureInstant, RecoveringAudioInput, RecoveringAudioInputEvent};
use crate::continuation::InputActivity;
use crate::control::{CaptureAction, PrintReply, PrintedTranscript};
use crate::desktop::events::{
    DictationPhase, EventLog, TranscriptPhase, VoiceEvent, VoiceState, now_ms,
};
use crate::dictation::service::{DictationQueue, Effect, MICROPHONE_LOST, ServiceDictations};
use crate::dictation::{DictationCapture, Finish, POST_RELEASE_TAIL, ReleaseWait};
use crate::transcription::engine::{PendingTranscript, TranscriptionEngine};
use crate::transcription::local_transcriber::LocalTranscriber;
use crate::transcription::models::TranscriptionSelection;
use crate::windows::input::WindowsHotkey;
use crate::windows::input::{HotkeyAction, WindowsHotkeyMonitor};
use crate::windows::paste::WindowsPaster;

mod finish;
mod output;
mod shortcuts;

use finish::submit_capture;
use output::{job_result, process_dictation_job, process_paste_last};
use shortcuts::{Shortcuts, next_action};

const UPDATE_INTERVAL: Duration = Duration::from_millis(5);
/// The longest wait with the microphone streaming, idle or recording. Its
/// chunks (about every 10 ms) wake the loop, which then takes shortcut
/// events and commands and ends a release's tail, so it needs no wakeup of
/// its own between them. A shortcut's audio boundaries come from its
/// event's own time, so taking it at the next chunk moves nothing.
const STREAM_WAIT: Duration = Duration::from_millis(50);
/// The longest wait while idle with the microphone released: a shortcut
/// event or a command ends it at once ([`wake`]); settings and Annotate's
/// controls are taken within it.
const IDLE_RELEASED_WAIT: Duration = Duration::from_millis(50);
/// A rewrite recording stops itself after this long: an instruction is a
/// sentence, and a trigger still down ten seconds later is a stuck key. The
/// dictate shortcut has no cap.
const REWRITE_RECORDING_LIMIT: Duration = Duration::from_secs(10);
/// What the HUD and the event log say when the cap stops a rewrite.
const REWRITE_RECORDING_LIMIT_MESSAGE: &str =
    "the rewrite recording stopped after 10 seconds; press the rewrite shortcut again";
/// How often a cancellable rewrite checks whether Escape dropped it.
const REWRITE_CANCEL_POLL: Duration = Duration::from_millis(20);
/// The error a rewrite reports when Escape drops it before it pastes.
const REWRITE_CANCELLED: &str = "the rewrite was cancelled";

/// Whether a recording has run past the rewrite cap. Only Rewrite is capped.
fn rewrite_recording_past_limit(
    flavour: crate::dictation::DictationFlavour,
    elapsed: Duration,
) -> bool {
    flavour == crate::dictation::DictationFlavour::Rewrite && elapsed >= REWRITE_RECORDING_LIMIT
}

/// The running listener's thread, which [`wake`] unparks.
static LISTENER: Mutex<Option<thread::Thread>> = Mutex::new(None);

/// Ends the listener's idle wait at once: the keyboard hook calls it for
/// each shortcut event, and the tray and `hex` commands after sending one.
pub fn wake() {
    if let Some(listener) = LISTENER
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
    {
        listener.unpark();
    }
}

/// What a job transcribes: the engine's pending run, or nothing at all for a
/// silent rewrite, which falls back to the default prompt.
enum JobTranscript {
    Pending(PendingTranscript),
    Empty,
}

impl JobTranscript {
    /// A dictation always has a pending run; `Empty` belongs to a rewrite.
    fn wait(self) -> Result<crate::transcription::engine::Transcribed, String> {
        match self {
            Self::Pending(pending) => pending.wait(),
            Self::Empty => Err("a dictation had no transcript".into()),
        }
    }
}

struct Job {
    context: crate::windows::context::PendingContext,
    transcript: JobTranscript,
    audio_ms: u64,
    /// The language the model was asked for at submission.
    language: String,
    /// The clip, kept for History while Labs › Keep recordings is on.
    recording: Option<crate::history::Recording>,
    /// The window dictation started in, for the paste to return to.
    target: Option<crate::windows::paste_target::PasteTarget>,
    /// Rewrite the selected text: the transcript is an instruction and this is
    /// the text it applies to. Not pasted as a dictation, not kept in history.
    rewrite_selection: Option<String>,
    /// Set by Escape after release to drop a rewrite before it pastes.
    cancel: Option<Arc<AtomicBool>>,
    /// The shortcut that started this capture (dictation, raw, terminal
    /// paste, or second dictation).
    flavour: crate::dictation::DictationFlavour,
    /// `hex dictation stop --print`: the text goes here, never pasted.
    print: Option<PrintReply>,
    /// A physical dictation shortcut, not a control or API request.
    by_shortcut: bool,
    /// Labs › Stop phrase: the words its capture ended with, read when the
    /// phrase was heard and removed from the transcript before processing.
    stop_phrase: Option<String>,
    /// The window in front when the capture started, where a paste returns
    /// when the desktop or a taskbar is in front instead.
    return_to: Option<crate::windows::paste::CaptureWindow>,
}

/// A release whose capture is still collecting its post-release tail.
struct Finishing {
    context: crate::windows::context::PendingContext,
    release: ReleaseWait,
    print: Option<PrintReply>,
    rewrite_selection: Option<String>,
    target: Option<crate::windows::paste_target::PasteTarget>,
    flavour: crate::dictation::DictationFlavour,
    /// A shortcut, not a `hex dictation` command, started the capture.
    by_shortcut: bool,
    /// Labs › Stop phrase: the words that ended this capture, when they did.
    stop_phrase: Option<String>,
    /// The window in front when the capture started.
    return_to: Option<crate::windows::paste::CaptureWindow>,
}

enum OutputJob {
    Dictation(Box<Job>),
    PasteLast,
}

#[derive(Clone, Copy, PartialEq)]
enum OutputKind {
    Dictation,
    Repaste,
    Printed,
    Rewrite,
}

struct JobResult {
    kind: OutputKind,
    text: Option<String>,
    latency_ms: u32,
    result: Result<(), String>,
    /// Something the job survived: the cleanup's reason for keeping the
    /// text. The paste still happens; the pill shows why.
    warning: Option<String>,
    /// Labs › Stop phrase took every word the capture heard: an empty result
    /// reads as a plain discard, not a voice prompt that asked for nothing.
    stop_phrase_only: bool,
    /// The text went to the clipboard instead of into a window: the shell was
    /// in front, or Labs › Copy dictation only is on.
    copied: bool,
}

pub struct WindowsDictationConfig {
    pub device: Option<String>,
    pub hotkey: WindowsHotkey,
    pub paste_last_hotkey: Option<WindowsHotkey>,
    /// Dictate without processing, while it is bound.
    pub raw_dictation_hotkey: Option<WindowsHotkey>,
    /// Rewrite the selected text, while it is bound.
    pub rewrite_hotkey: Option<WindowsHotkey>,
    /// Labs › Terminal paste shortcut, while the lab is on and it is bound.
    pub terminal_paste_hotkey: Option<WindowsHotkey>,
    /// Labs › Second dictation shortcut, while the lab is on and it is bound.
    pub second_dictation_hotkey: Option<WindowsHotkey>,
    /// The Annotate key, while Annotate is on.
    pub annotate_hotkey: Option<WindowsHotkey>,
    /// Toggle mode's cancel shortcut; none in Hold mode.
    pub cancel_hotkey: Option<WindowsHotkey>,
    pub double_tap_lock: bool,
    pub double_tap_only: bool,
    pub shortcut_mode: crate::dictation::ShortcutMode,
    pub share_dictation_shortcut: bool,
    pub while_dictating: crate::app_settings::RecordingAudioBehavior,
    pub release_microphone_while_idle: bool,
    pub feedback_volume: u8,
    pub sound_theme: crate::feedback::SoundSet,
    pub history: Option<crate::history::History>,
    pub last_dictation: Arc<Mutex<Option<String>>>,
    pub indicator: Option<crate::rune_hud::HudSender>,
    pub mode_runtime: Arc<RwLock<crate::dictation::modes::ModeRuntime>>,
    pub transformations: Arc<crate::personal_commands::TransformationClient>,
    pub fallback_to_default_device: bool,
    /// Terminal paste: Ctrl+Shift+V instead of Ctrl+V.
    pub paste_with_shift: bool,
    /// What Rewrite selection reads at each job. The listener shares it with
    /// the output worker and replaces its contents when settings change, so a
    /// changed endpoint or prompt needs no restart.
    pub rewrite: crate::dictation::rewrite::RewriteRuntime,
    /// Paste into the window dictation started in.
    pub paste_to_start_window: bool,
    /// Where the listener reports a model change that failed to load.
    pub swap_failures: Option<mpsc::Sender<crate::transcription::local_transcriber::SwapFailed>>,
    /// What the tray asks of the listener.
    pub commands: Option<mpsc::Receiver<ListenerCommand>>,
    /// The local API's `/dictations`: a client's dictation recorded beside
    /// the shortcut, one capture at a time.
    pub dictations: DictationQueue,
    /// The apps in which no shortcut fires.
    pub shortcut_guard: crate::app_settings::ShortcutGuard,
}

/// A request from outside the keyboard hook, from the tray or a `hex`
/// command (`crate::control::host`), handled as its shortcut would be.
#[derive(Debug)]
pub enum ListenerCommand {
    /// `raw`: the dictation this starts skips processing.
    Dictation {
        action: CaptureAction,
        raw: bool,
    },
    PasteLast,
    /// `hex dictation stop --print`.
    StopAndPrint(PrintReply),
}

impl WindowsDictationConfig {
    /// The listener configuration `settings` describe, with fresh runtime
    /// state; a host swaps in the state it shares with the listener.
    pub fn from_settings(settings: &crate::app_settings::AppSettings) -> Self {
        Self {
            device: settings.microphone.clone(),
            hotkey: WindowsHotkey::from(&settings.dictation_hotkey),
            paste_last_hotkey: settings.paste_last_hotkey.as_ref().map(WindowsHotkey::from),
            raw_dictation_hotkey: settings
                .raw_dictation_hotkey
                .as_ref()
                .map(WindowsHotkey::from),
            rewrite_hotkey: settings.rewrite_hotkey.as_ref().map(WindowsHotkey::from),
            terminal_paste_hotkey: settings
                .active_terminal_paste_hotkey()
                .map(WindowsHotkey::from),
            second_dictation_hotkey: settings
                .active_second_dictation_hotkey()
                .map(WindowsHotkey::from),
            annotate_hotkey: settings
                .capture_on()
                .then(|| WindowsHotkey::from(&settings.capture.hotkey)),
            cancel_hotkey: settings.active_cancel_hotkey().map(WindowsHotkey::from),
            double_tap_lock: settings.double_tap_lock,
            double_tap_only: settings.double_tap_only,
            shortcut_mode: settings.shortcut_mode,
            share_dictation_shortcut: settings.share_dictation_shortcut,
            while_dictating: settings.recording_audio_behavior,
            release_microphone_while_idle: settings.release_microphone_while_idle,
            feedback_volume: if settings.sound_effects {
                (settings.sound_effect_volume.clamp(0.0, 1.0) * 100.0).round() as u8
            } else {
                0
            },
            sound_theme: settings.sound_theme,
            history: None,
            last_dictation: Arc::new(Mutex::new(None)),
            indicator: None,
            mode_runtime: Arc::new(RwLock::new(
                crate::dictation::modes::ModeRuntime::from_settings(settings),
            )),
            transformations: Arc::new(crate::personal_commands::TransformationClient::default()),
            fallback_to_default_device: true,
            paste_with_shift: settings.paste_with_shift,
            rewrite: crate::dictation::rewrite::RewriteRuntime::from_settings(settings),
            paste_to_start_window: settings.dictation_processing.paste_to_start_window,
            swap_failures: None,
            commands: None,
            dictations: DictationQueue::default(),
            shortcut_guard: settings.shortcuts.clone(),
        }
    }
}

/// Runs hotkey dictation until `shutdown`. Settings from `updates` apply in
/// place while no dictation records, so changing one never stops listening.
pub fn run(
    event_path: &Path,
    selection: &TranscriptionSelection,
    config: WindowsDictationConfig,
    updates: mpsc::Receiver<Box<crate::app_settings::AppSettings>>,
    shutdown: &AtomicBool,
) -> Result<()> {
    // Labs › Free memory when idle: a launch stays parked until the first
    // dictation, as after an idle unload.
    let transcriber = if crate::labs::unload_idle().is_some() {
        None
    } else {
        Some(LocalTranscriber::load(selection)?)
    };
    // The hook's shortcuts from the same settings, before its fields are
    // destructured below.
    let mut shortcuts = Shortcuts::from_config(&config);
    let WindowsDictationConfig {
        device,
        hotkey,
        while_dictating,
        release_microphone_while_idle,
        feedback_volume,
        sound_theme,
        history,
        last_dictation,
        indicator,
        mode_runtime,
        transformations,
        fallback_to_default_device,
        paste_with_shift,
        paste_to_start_window,
        rewrite,
        swap_failures,
        commands,
        dictations,
        shortcut_guard,
        ..
    } = config;
    let mut shortcut_guard = shortcut_guard;
    // Read by the output worker at each paste, so a change applies in place.
    let paste_with_shift = Arc::new(AtomicBool::new(paste_with_shift));
    let paste_to_start_window = Arc::new(AtomicBool::new(paste_to_start_window));
    let _indicator_guard = IndicatorGuard(indicator.clone());
    let _woken = Woken::register();
    crate::feedback::set_enabled(feedback_volume > 0);
    crate::feedback::set_volume(f32::from(feedback_volume) / 100.0);
    crate::feedback::set_sound_set(sound_theme);
    if feedback_volume > 0
        && let Err(error) = crate::feedback::preload()
    {
        tracing::warn!(%error, "Windows recording feedback is unavailable");
    }
    let mut release_while_idle = release_microphone_while_idle;
    // Labs › Keep mic open after dictating: a finished capture's
    // microphone stays open for its window before release.
    let mut linger = MicrophoneLinger::default();
    let mut device = device;
    let mut input = create_input(
        device.as_deref(),
        fallback_to_default_device,
        release_while_idle,
    )?;
    let mut events = EventLog::create(event_path)?;
    // With the microphone released the real sample rate is unknown until the
    // first open; this placeholder capture is replaced on Reopened.
    let mut capture = DictationCapture::new(if input.is_open() {
        input.sample_rate()
    } else {
        16_000
    });
    let mut captured_through = if release_while_idle {
        CaptureInstant::ZERO
    } else {
        prime_microphone(&mut input, &mut capture, shutdown)?
    };
    let mut audio_ready = !release_while_idle;
    let mut device_label = if input.is_open() {
        input.device_name().to_string()
    } else {
        device.clone().unwrap_or_else(|| "System default".into())
    };
    // A hotkey press that arrived while the microphone could not capture;
    // recording begins at the first fresh chunk.
    let mut start_pending = false;
    // The window the current dictation started in, captured on its press so a
    // later Alt-Tab does not move its text.
    let mut paste_target = None;
    if shutdown.load(Ordering::Relaxed) {
        return Ok(());
    }
    let hotkey_label = hotkey.label();
    // Whether a rewrite job is still transcribing or waiting on the model.
    // The hook reads it so Escape can stop the job after the recording ended.
    let rewrite_pending = Arc::new(AtomicBool::new(false));
    let mut toggle_mode = shortcuts.shortcut_mode == crate::dictation::ShortcutMode::Toggle;
    let mut hotkey = shortcuts.start(
        InputActivity::default(),
        &shortcut_guard,
        Arc::clone(&rewrite_pending),
    )?;
    // A shortcut event the idle wait took, handled at the next turn.
    let mut held_event = None;
    let mut while_dictating = while_dictating;
    let mut audio_suppressor =
        crate::windows::audio_control::AudioSuppressor::start(while_dictating);
    let mut model = crate::transcription::local_transcriber::ModelSwap::new(selection);
    let mut engine = match transcriber {
        Some(transcriber) => Arc::new(TranscriptionEngine::start(transcriber)?),
        None => Arc::new(TranscriptionEngine::start_parked(selection.clone())?),
    };
    // The local API transcribes host audio on this engine's model.
    engine.share();
    // The `capture` experiment: its host, the audio it copies from this
    // listener's microphone, and the capsule's commands.
    let timeline = crate::capture_host::ListenerTimeline::default();
    let environment = crate::capture_host::Environment;
    let mut capture_host =
        crate::capture_host::CaptureHost::start(Some(engine.clone()), &environment);
    let capture_controls = crate::capture_host::listen_for_controls();
    let mut finishing: Option<Finishing> = None;
    // The output worker outlives one settings save, so both sides share the
    // rewrite settings and the worker copies them per job.
    let rewrite = Arc::new(RwLock::new(rewrite));
    let worker_rewrite = Arc::clone(&rewrite);
    let (jobs, job_receiver) = mpsc::sync_channel::<OutputJob>(2);
    let (result_sender, results) = mpsc::channel();
    let worker_paste_with_shift = paste_with_shift.clone();
    let worker_paste_to_start_window = paste_to_start_window.clone();
    let worker_activity = hotkey.activity.clone();
    let capture_modes = mode_runtime.clone();
    let worker = thread::Builder::new()
        .name("windows-dictation-output".into())
        .spawn(move || {
            let mut paster = WindowsPaster::new(
                worker_paste_with_shift,
                worker_paste_to_start_window,
                worker_activity,
            );
            while let Ok(job) = job_receiver.recv() {
                let result = match job {
                    // A rewrite is a dictation job whose transcript is the
                    // instruction, so the selection decides the path.
                    OutputJob::Dictation(job) if job.rewrite_selection.is_some() => {
                        let rewrite = worker_rewrite
                            .read()
                            .unwrap_or_else(|error| error.into_inner())
                            .clone();
                        process_rewrite_job(*job, &mut paster, &rewrite)
                    }
                    OutputJob::Dictation(job) => process_dictation_job(
                        *job,
                        &mut paster,
                        history.as_ref(),
                        &last_dictation,
                        &mode_runtime,
                        transformations.as_ref(),
                    ),
                    OutputJob::PasteLast => process_paste_last(&mut paster, &last_dictation),
                };
                if result_sender.send(result).is_err() {
                    break;
                }
            }
        })?;

    let mut recording = false;
    // The recording rewrites the selection (Rewrite selection).
    let mut recording_rewrite: Option<String> = None;
    // The shortcut that started this recording.
    let mut recording_flavour = crate::dictation::DictationFlavour::Normal;
    // When the current rewrite recording began, for its ten-second cap.
    let mut rewrite_started: Option<Instant> = None;
    // Rewrite jobs still in flight, oldest first. Escape sets the newest
    // one's flag; the worker drops it without pasting.
    let mut pending_rewrites: std::collections::VecDeque<Arc<AtomicBool>> =
        std::collections::VecDeque::new();
    let mut recording_by_shortcut = false;
    // The window in front when this capture started, where its paste returns
    // if the desktop or a taskbar is in front instead.
    let mut recording_return_to = None;
    let mut recording_feedback_started = false;
    // Whether this capture's start tone already played on its press.
    let mut start_tone_played = false;
    // Labs › Stop phrase: this capture's detector, fresh for every capture.
    let mut stop_phrase = crate::dictation::stop_phrase::Detector::new();
    let mut pending = 0_usize;
    // The worker drains its queue in order, so a FIFO of ids pairs each
    // result with the HUD job it belongs to. Repaste jobs get ids too (to
    // keep the queue aligned) but are never announced to the HUD.
    let mut next_job_id = 0_u64;
    let mut job_ids = std::collections::VecDeque::new();
    // A local API client's dictation, and its transcripts as they finish.
    let mut service = ServiceDictations::default();
    // Whether the client's dictation records, shown on the HUD.
    let mut service_recording = false;
    let (service_results, service_finished) = mpsc::channel::<(u64, Result<String, String>)>();
    events.emit(&VoiceEvent::SessionStarted {
        timestamp_ms: now_ms(),
    })?;
    emit_state(&mut events, VoiceState::Listening, &device_label)?;
    println!(
        "Hex is ready on {}. Hold {} to dictate, release to paste; Ctrl-C stops.",
        device_label, hotkey_label
    );

    // Labs › Keep mic open after dictating: the one release check the pass and
    // its event sites share, so a press handled in the same pass still sees a
    // closed microphone at once when the lab is off.
    macro_rules! release_microphone {
        () => {
            if release_while_idle
                && input.is_active()
                && linger.may_close(
                    Instant::now(),
                    recording
                        || start_pending
                        || finishing.is_some()
                        || service.is_active()
                        || timeline.wants_audio(),
                )
            {
                input.close();
                audio_ready = false;
            }
        };
    }

    // The window in front when the loop last looked: switching windows ends
    // a paste continuation, as a context change does on macOS.
    let mut foreground_window = crate::windows::input::foreground_window();
    while !shutdown.load(Ordering::Relaxed) {
        // A rewrite recording held past its cap stops itself through the
        // ordinary failure surface: nothing is transcribed and nothing pastes.
        if recording
            && rewrite_recording_past_limit(
                recording_flavour,
                rewrite_started.map_or(Duration::ZERO, |started| started.elapsed()),
            )
        {
            audio_suppressor.restore();
            crate::feedback::retract_start();
            paste_target = None;
            recording_rewrite = None;
            recording = false;
            recording_feedback_started = false;
            rewrite_started = None;
            capture.cancel();
            crate::windows::input::cancel_rewrite_capture();
            if let Some(indicator) = &indicator {
                indicator.send(crate::rune_hud::HudEvent::Failed {
                    message: Some(REWRITE_RECORDING_LIMIT_MESSAGE.into()),
                });
            }
            fail(&events, REWRITE_RECORDING_LIMIT_MESSAGE)?;
            emit_current_state(&mut events, recording, pending, &device_label)?;
            release_microphone!();
        }
        let window = crate::windows::input::foreground_window();
        if window != foreground_window {
            foreground_window = window;
            hotkey.activity.invalidate();
        }
        if let Some(failed) = model.poll() {
            fail(&events, failed.error.clone())?;
            if let Some(swap_failures) = &swap_failures {
                let _ = swap_failures.send(failed);
            }
        }
        // Only the newest saved settings matter; they wait for a moment
        // without a recording.
        if !recording
            && !start_pending
            && finishing.is_none()
            && !service.is_active()
            && let Some(next) = updates.try_iter().last()
        {
            let next_config = WindowsDictationConfig::from_settings(&next);
            paste_with_shift.store(next_config.paste_with_shift, Ordering::Relaxed);
            paste_to_start_window.store(next_config.paste_to_start_window, Ordering::Relaxed);
            let next_shortcuts = Shortcuts::from_config(&next_config);
            if next_shortcuts != shortcuts {
                // The old hook goes first; one keyboard hook runs at a time.
                // The activity survives the restart, so a pending
                // continuation still sees the same input session.
                let activity = hotkey.activity.clone();
                drop(hotkey);
                held_event = None;
                shortcuts = next_shortcuts;
                hotkey = shortcuts.start(
                    activity,
                    &next_config.shortcut_guard,
                    Arc::clone(&rewrite_pending),
                )?;
                toggle_mode = shortcuts.shortcut_mode == crate::dictation::ShortcutMode::Toggle;
            }
            // The guard applies in place: no binding changed, only which apps
            // the next key respects.
            if next_config.shortcut_guard != shortcut_guard {
                shortcut_guard = next_config.shortcut_guard;
                crate::windows::input::set_shortcut_guard(shortcut_guard.clone());
            }
            if next_config.while_dictating != while_dictating {
                while_dictating = next_config.while_dictating;
                audio_suppressor =
                    crate::windows::audio_control::AudioSuppressor::start(while_dictating);
            }
            if next_config.device != device
                || next_config.release_microphone_while_idle != release_while_idle
            {
                drop(input);
                device = next_config.device;
                release_while_idle = next_config.release_microphone_while_idle;
                input = create_input(
                    device.as_deref(),
                    fallback_to_default_device,
                    release_while_idle,
                )?;
                capture = DictationCapture::new(if input.is_open() {
                    input.sample_rate()
                } else {
                    16_000
                });
                if !release_while_idle {
                    captured_through = prime_microphone(&mut input, &mut capture, shutdown)?;
                }
                audio_ready = !release_while_idle;
                device_label = if input.is_open() {
                    input.device_name().to_string()
                } else {
                    device.clone().unwrap_or_else(|| "System default".into())
                };
                emit_current_state(&mut events, recording, pending, &device_label)?;
            }
            model.want(&next.transcription);
            rewrite
                .write()
                .unwrap_or_else(|error| error.into_inner())
                .apply(&next);
        }
        // A GPU without room for two copies: the engine swaps in place.
        if !recording
            && !start_pending
            && finishing.is_none()
            && !service.is_active()
            && pending == 0
            && let Some(selection) = model.take_reload()
            && let Err(error) = engine.reload(selection)
        {
            tracing::warn!(%error, "could not replace the transcription model");
        }
        if !recording
            && !start_pending
            && finishing.is_none()
            && !service.is_active()
            && pending == 0
            && let Some(next) = model.take_ready()
        {
            let old = std::mem::replace(&mut engine, Arc::new(TranscriptionEngine::start(next)?));
            engine.share();
            crate::capture_host::use_engine(&engine);
            // A request on the local API or a capture still transcribing
            // holds the old engine a moment.
            if let Ok(old) = Arc::try_unwrap(old) {
                old.retire();
            }
        }
        crate::capture_host::CaptureHost::reconcile(
            &mut capture_host,
            Some(&engine),
            &environment,
            &timeline,
        );
        while let Ok(control) = capture_controls.try_recv() {
            let reply = match (&mut capture_host, control) {
                (Some(host), crate::capture_host::CaptureControl::Scrawl(action)) => {
                    host.control(action, &timeline)
                }
                (Some(host), crate::capture_host::CaptureControl::Command(command)) => {
                    host.command(command, &timeline)
                }
                (host, crate::capture_host::CaptureControl::Reply(action, answer)) => {
                    let _ = answer.send(match host {
                        Some(host) => host.control(action, &timeline),
                        None => crate::control::ControlReply::error(
                            "feature-off",
                            "Annotate is off; turn it on with `hex settings set capture.enabled on`",
                        ),
                    });
                    continue;
                }
                (None, _) => crate::control::ControlReply::error("feature-off", "captures are off"),
            };
            if let crate::control::ControlReply::Error { message, .. } = reply {
                tracing::warn!(message, "a capture command did nothing");
            }
        }
        // A capture copies this microphone: a released one opens for it, and
        // the one release check below closes it once nothing records.
        if release_while_idle && timeline.wants_audio() && !input.is_active() {
            input.request_open();
        }
        if let Ok(error) = hotkey.errors.try_recv() {
            return Err(eyre!("Windows hotkey monitor stopped: {error}"));
        }
        // A release keeps recording through its tail; shortcut events wait,
        // so a press inside the tail starts after this clip is submitted.
        if let Some(finish) = &finishing
            && let Some(tail) = finish.release.tail(&capture, Instant::now())
        {
            let Some(Finishing {
                release,
                context,
                print,
                rewrite_selection,
                target,
                flavour,
                by_shortcut,
                stop_phrase,
                return_to,
            }) = finishing.take()
            else {
                unreachable!("checked above");
            };
            let (submitted, rewrite_cancel) = submit_capture(
                &mut capture,
                release.ended_at(),
                tail,
                &engine,
                &jobs,
                &mut events,
                &mut pending,
                print,
                target,
                rewrite_selection,
                flavour,
                by_shortcut,
                context,
                stop_phrase,
                return_to,
            )?;
            if submitted {
                next_job_id += 1;
                job_ids.push_back(next_job_id);
            }
            if let Some(cancel) = rewrite_cancel {
                pending_rewrites.push_back(cancel);
                rewrite_pending.store(true, Ordering::Release);
            }
            if let Some(indicator) = &indicator {
                indicator.send(if submitted {
                    crate::rune_hud::HudEvent::Submitted {
                        job_id: next_job_id,
                    }
                } else {
                    crate::rune_hud::HudEvent::Discarded
                });
            }
            emit_current_state(&mut events, recording, pending, &device_label)?;
            // The capture's end releases the microphone here, as before, when
            // the lab is off.
            release_microphone!();
        }
        // `/dictations`: a client's dictation never ends a shortcut capture,
        // and only its owner (or its lapsed lease) ends it.
        let now = Instant::now();
        let mut service_ended = service.expire(now);
        for control in dictations.drain() {
            let busy = recording
                || start_pending
                || finishing.is_some()
                || capture_host
                    .as_mut()
                    .is_some_and(|host| host.recording_since(&timeline).is_some());
            match service.control(control, busy, now) {
                Effect::None => {}
                Effect::Start if audio_ready && input.is_open() => {
                    service_recording = start_service_capture(
                        &mut capture,
                        &mut service,
                        &audio_suppressor,
                        indicator.as_ref(),
                        captured_through,
                        input.sample_rate(),
                    );
                    service_ended |= !service.is_active();
                }
                // Recording starts at the released microphone's first chunk.
                // A failed stream keeps its backoff, but the client's start
                // asks for the microphone now.
                Effect::Start => {
                    input.retry_now();
                    if !input.is_active() {
                        input.request_open();
                    }
                }
                Effect::Finish(id) => {
                    // The client's finish is its exact end: no tail.
                    match capture.finish_with_tail(captured_through, Duration::ZERO) {
                        Finish::Discard => service.complete(id, Err("capture-discarded".into())),
                        Finish::Transcribe(mut clip) => {
                            service.transcribing(id, clip.duration_ms());
                            match engine
                                .finish(clip.take_stream(), clip.into_transcription_samples())
                            {
                                Ok(transcript) => {
                                    let results = service_results.clone();
                                    thread::spawn(move || {
                                        let result =
                                            transcript.wait().map(|transcribed| transcribed.text);
                                        let _ = results.send((id, result));
                                    });
                                }
                                Err(error) => service.complete(id, Err(format!("{error:#}"))),
                            }
                        }
                    }
                    service_ended = true;
                }
                Effect::Cancel => service_ended = true,
            }
        }
        if service_ended && !service.is_active() {
            end_service_capture(
                &mut capture,
                &audio_suppressor,
                indicator.as_ref(),
                &mut service_recording,
            );
            // A client's dictation ended here before too.
            release_microphone!();
        }
        for (id, result) in service_finished.try_iter() {
            service.complete(id, result);
        }
        // A tray or `hex` command acts as its shortcut would, at once; a
        // printing stop's reply rides with the capture it finishes.
        let mut print = None;
        while finishing.is_none()
            && let Some((action, occurred_at, deliberate, flavour)) = next_action(
                &hotkey,
                &mut held_event,
                commands.as_ref(),
                recording || start_pending,
                captured_through,
                &mut print,
            )
        {
            // Rewrite selection: the selection is read before anything
            // records, so an empty or oversized one ends here with nothing
            // captured and nothing pasted. Only the rewrite shortcut carries
            // this flavour; a `hex` command never does.
            let rewrite = flavour == crate::dictation::DictationFlavour::Rewrite;
            if rewrite && action == HotkeyAction::Start && !recording {
                let target = crate::windows::paste_target::PasteTarget::capture();
                match crate::windows::selection::read_selection() {
                    Ok(selection) => {
                        recording_rewrite = Some(selection);
                        paste_target = target;
                    }
                    Err(error) => {
                        let message = format!("{error}");
                        fail(&events, message.clone())?;
                        if let Some(indicator) = &indicator {
                            indicator.send(crate::rune_hud::HudEvent::Failed {
                                message: Some(message),
                            });
                        }
                        continue;
                    }
                }
            }
            // Labs › Stop phrase: only the finish its words ended keeps them;
            // any other action leaves them for that finish.
            let finish_phrase = if matches!(action, HotkeyAction::Finish) {
                hotkey.take_stop_phrase()
            } else {
                None
            };
            match action {
                // A client's dictation records: the shortcut and `hex
                // dictation` neither start nor end anything.
                HotkeyAction::Start | HotkeyAction::Finish | HotkeyAction::Cancel
                    if service.is_active() => {}
                // Annotate records the narration itself: while it records,
                // dictation starts nothing.
                HotkeyAction::Start
                    if capture_host
                        .as_mut()
                        .is_some_and(|host| host.recording_since(&timeline).is_some()) => {}
                // One key: a tap starts and stops; held while recording, it
                // draws until it comes up.
                HotkeyAction::Capture => {
                    if let Some(host) = &mut capture_host {
                        host.annotate_down(&timeline);
                    }
                }
                HotkeyAction::CaptureUp => {
                    if let Some(host) = &mut capture_host {
                        host.annotate_up(&timeline);
                    }
                }
                // A press while the microphone cannot capture yet (a stream
                // that stopped delivering, or one still opening) is kept and
                // starts at the first fresh chunk, as the released
                // microphone's does. A failed stream keeps its backoff, but
                // the press asks for the microphone now.
                HotkeyAction::Start if !recording && !start_pending => {
                    // Taken on the press, before the HUD and before any
                    // switch away: this is where the text belongs.
                    paste_target = crate::windows::paste_target::PasteTarget::capture();
                    recording_flavour = flavour;
                    recording_by_shortcut = !deliberate;
                    // The window in front now, where a paste returns if the
                    // desktop or a taskbar is in front when it is sent.
                    recording_return_to = crate::windows::paste::CaptureWindow::read();
                    // Labs › Stop phrase: a new capture listens for the
                    // phrase from its own words.
                    stop_phrase = crate::dictation::stop_phrase::Detector::new();
                    if deliberate {
                        hotkey.command_capture(Some(flavour));
                    }
                    if audio_ready && !input.is_recovering() && !input.is_stalled() {
                        capture.start_hotkey_at(
                            occurred_at,
                            crate::dictation::hotkey_pre_roll(while_dictating),
                        );
                        recording = true;
                        rewrite_started = (flavour == crate::dictation::DictationFlavour::Rewrite)
                            .then(Instant::now);
                        // A toggle press or a `hex` command is deliberate, so
                        // it streams at once.
                        recording_feedback_started = toggle_mode || deliberate;
                        if recording_feedback_started {
                            audio_suppressor.suppress();
                        }
                        // The tone confirms the press at once; a short tap or a
                        // chord meant for another application retracts it.
                        crate::feedback::play_provisional_start();
                        start_tone_played = true;
                    } else {
                        // The microphone is not delivering yet; recording
                        // begins at the first fresh chunk. A failed stream
                        // keeps its backoff, but the press asks for it now.
                        if input.is_recovering() || input.is_stalled() {
                            input.retry_now();
                        }
                        start_pending = true;
                        input.request_open();
                        crate::feedback::prepare();
                    }
                    if let Some(indicator) = &indicator {
                        indicator.send(crate::rune_hud::HudEvent::Started);
                    }
                    events.dictation(DictationPhase::Started, "")?;
                    emit_state(&mut events, VoiceState::Dictating, &device_label)?;
                }
                HotkeyAction::Finish | HotkeyAction::Cancel if start_pending => {
                    if deliberate {
                        hotkey.command_capture(None);
                    }
                    // Released before the opening microphone produced audio:
                    // nothing was captured, so treat it as a discard.
                    start_pending = false;
                    paste_target = None;
                    recording_rewrite = None;
                    audio_suppressor.restore();
                    release_microphone!();
                    if let Some(print) = print.take() {
                        print.send(PrintedTranscript::NoSpeech);
                    }
                    let cancelled = matches!(action, HotkeyAction::Cancel);
                    events.dictation(
                        if cancelled {
                            DictationPhase::Cancelled
                        } else {
                            DictationPhase::Discarded
                        },
                        "",
                    )?;
                    if let Some(indicator) = &indicator {
                        indicator.send(if cancelled {
                            crate::rune_hud::HudEvent::Cancelled
                        } else {
                            crate::rune_hud::HudEvent::Discarded
                        });
                    }
                    emit_current_state(&mut events, recording, pending, &device_label)?;
                }
                HotkeyAction::Finish if recording => {
                    if deliberate {
                        hotkey.command_capture(None);
                    }
                    audio_suppressor.restore();
                    if !recording_feedback_started && capture.become_intentional(occurred_at) {
                        if !start_tone_played {
                            crate::feedback::play(crate::feedback::Tone::DictationStart);
                        }
                        recording_feedback_started = true;
                    }
                    if recording_feedback_started {
                        crate::feedback::play(crate::feedback::Tone::DictationStop);
                    } else {
                        crate::feedback::retract_start();
                    }
                    recording = false;
                    recording_feedback_started = false;
                    // The clip keeps its tail and drops anything captured
                    // after it, so the live stream must hear no further; the
                    // engine refuses a stream fed well past the clip.
                    capture.limit_stream_to(occurred_at, POST_RELEASE_TAIL);
                    let rewrite_selected = recording_rewrite.is_some();
                    finishing = Some(Finishing {
                        rewrite_selection: recording_rewrite.take(),
                        flavour: recording_flavour,
                        by_shortcut: recording_by_shortcut,
                        print: print.take(),
                        target: paste_target.take(),
                        // Labs › Stop phrase: the phrase's own finish carries
                        // its words, which the job removes.
                        stop_phrase: finish_phrase,
                        return_to: recording_return_to,
                        context: if capture.would_transcribe(occurred_at) {
                            // A rewrite never reads the page: its instruction
                            // is the spoken words and its text the selection.
                            crate::windows::context::begin_capture(
                                !recording_flavour.raw()
                                    && !rewrite_selected
                                    && capture_modes
                                        .read()
                                        .unwrap_or_else(|error| error.into_inner())
                                        .reads_page(),
                            )
                        } else {
                            Default::default()
                        },
                        release: ReleaseWait::new(&capture, occurred_at),
                    });
                }
                HotkeyAction::Cancel if recording => {
                    if deliberate {
                        hotkey.command_capture(None);
                    }
                    audio_suppressor.restore();
                    crate::feedback::retract_start();
                    paste_target = None;
                    recording_rewrite = None;
                    rewrite_started = None;
                    if recording_feedback_started {
                        crate::feedback::play(crate::feedback::Tone::Cancel);
                    }
                    recording = false;
                    recording_feedback_started = false;
                    capture.cancel();
                    if let Some(indicator) = &indicator {
                        indicator.send(crate::rune_hud::HudEvent::Cancelled);
                    }
                    events.dictation(DictationPhase::Cancelled, "")?;
                    emit_current_state(&mut events, recording, pending, &device_label)?;
                    release_microphone!();
                }
                // Escape after a rewrite's release: drop the newest job before
                // it pastes, so the selection stays exactly as it was.
                HotkeyAction::Cancel
                    if !recording && !start_pending && !pending_rewrites.is_empty() =>
                {
                    if let Some(cancel) = pending_rewrites.back() {
                        cancel.store(true, Ordering::Release);
                    }
                }
                HotkeyAction::PasteLast if !recording => {
                    submit_paste_last(
                        &jobs,
                        &mut events,
                        &mut pending,
                        &mut next_job_id,
                        &mut job_ids,
                    )?;
                    emit_state(&mut events, VoiceState::Transcribing, &device_label)?;
                }
                _ => {}
            }
            if let Some(print) = print.take() {
                print.send(PrintedTranscript::NotRecording);
            }
        }
        while let Ok(result) = results.try_recv() {
            pending = pending.saturating_sub(1);
            let job_id = job_ids.pop_front();
            if result.kind == OutputKind::Rewrite {
                pending_rewrites.pop_front();
                rewrite_pending.store(!pending_rewrites.is_empty(), Ordering::Release);
            }
            let nothing = result.result.is_ok() && result.text.as_deref() == Some("");
            if let (Some(indicator), Some(job_id)) = (&indicator, job_id) {
                // Repaste ids were never announced; a failed Paste last gets no notice.
                let failure = (result.kind != OutputKind::Repaste)
                    .then(|| result.result.as_ref().err().cloned())
                    .flatten();
                let warning = (result.kind != OutputKind::Repaste)
                    .then(|| result.warning.clone())
                    .flatten()
                    .map(|reason| crate::dictation::ai_rewrite::human_cleanup_reason(&reason));
                indicator.send(if nothing {
                    crate::rune_hud::HudEvent::JobCancelled { job_id }
                } else if let Some(message) = failure.or(warning) {
                    crate::rune_hud::HudEvent::JobFailed {
                        job_id,
                        message: Some(message),
                    }
                } else {
                    crate::rune_hud::HudEvent::JobCompleted { job_id }
                });
            }
            match result.result {
                // A capture that heard only Labs › Stop phrase's words ends
                // as any silent one, not as a voice prompt that asked nothing.
                Ok(()) if nothing && result.stop_phrase_only => {
                    events.dictation(DictationPhase::Discarded, "")?;
                }
                Ok(()) if nothing => events.discarded_voice_prompt()?,
                Ok(()) => {
                    let text = result.text.unwrap_or_default();
                    if !matches!(result.kind, OutputKind::Repaste) {
                        crate::feedback::play(crate::feedback::Tone::DictationDone);
                    }
                    match result.kind {
                        OutputKind::Printed => {
                            events.dictation(DictationPhase::Printed, text)?;
                        }
                        OutputKind::Dictation => {
                            events.emit(&VoiceEvent::Transcript {
                                timestamp_ms: now_ms(),
                                phase: TranscriptPhase::Completed,
                                latency_ms: result.latency_ms,
                                text: text.clone(),
                            })?;
                            events.dictation(
                                if result.copied {
                                    // Nothing in front took the chord: the
                                    // words are on the clipboard instead.
                                    DictationPhase::Copied
                                } else {
                                    DictationPhase::Pasted
                                },
                                text,
                            )?;
                        }
                        OutputKind::Repaste if result.copied => {
                            events.dictation(DictationPhase::Copied, text)?;
                        }
                        OutputKind::Repaste => {
                            events.dictation(DictationPhase::Repasted, text)?;
                        }
                        // A rewrite replaces a selection; it is not a
                        // dictation, so the event log records the paste alone.
                        OutputKind::Rewrite => {
                            events.dictation(DictationPhase::Pasted, text)?;
                        }
                    }
                }
                Err(error) => fail(&events, error)?,
            }
            emit_current_state(&mut events, recording, pending, &device_label)?;
        }

        // The released microphone closes once nothing records, no client
        // dictates, no release tail runs, no capture copies it, and Labs ›
        // Keep mic open after dictating's window has passed. This also
        // releases one a stream interruption left open, which the old
        // latch-based sites never did.
        release_microphone!();

        dictations.answer_status(service.snapshot(
            recording || start_pending || finishing.is_some(),
            pending,
            input.is_recovering(),
        ));
        let idle = !recording
            && !start_pending
            && finishing.is_none()
            && !service.is_active()
            && pending == 0
            && !timeline.wants_audio();
        // A released microphone delivers nothing: wait for the next shortcut
        // event, which is taken at once, instead of sleeping 5 ms at a time.
        if idle && !input.is_active() {
            // Shortcut events and commands unpark the loop ([`wake`]).
            held_event = hotkey.events.try_recv().ok();
            if held_event.is_none() {
                thread::park_timeout(IDLE_RELEASED_WAIT);
                held_event = hotkey.events.try_recv().ok();
            }
            continue;
        }
        // Only an opening or recovering microphone is polled.
        let streaming = audio_ready && input.is_open();
        match input.recv_timeout(
            if streaming {
                STREAM_WAIT
            } else {
                UPDATE_INTERVAL
            },
            crate::audio::capture_idle(
                recording || service_recording || timeline.wants_audio(),
                start_pending || service.is_starting(),
                finishing.is_some(),
            ),
        ) {
            RecoveringAudioInputEvent::Chunk {
                samples,
                captured_through: chunk_captured_through,
            } => {
                captured_through = chunk_captured_through;
                crate::audio::input_level::record(&samples);
                timeline.feed(&samples, input.sample_rate(), captured_through);
                let first_chunk = !audio_ready;
                if first_chunk {
                    // A reopened or newly opened stream keeps its own timeline.
                    capture = DictationCapture::new(input.sample_rate());
                    audio_ready = true;
                    device_label = input.device_name().to_string();
                }
                capture.ingest_with_pending(&samples, captured_through, None);
                // A press that could not start when it was taken begins at
                // the first chunk that can capture, whether the stream just
                // opened or is one the press distrusted (a stalled stream the
                // loop had not read yet). The chunk anchors it with no
                // pre-roll: audio from before it may be from another stream.
                if start_pending {
                    start_pending = false;
                    capture.start_hotkey_at(chunk_captured_through, Duration::ZERO);
                    recording = true;
                    rewrite_started = (recording_flavour
                        == crate::dictation::DictationFlavour::Rewrite)
                        .then(Instant::now);
                    // The released microphone is ready only now.
                    recording_feedback_started = toggle_mode;
                    start_tone_played = recording_feedback_started;
                    if recording_feedback_started {
                        crate::feedback::play(crate::feedback::Tone::DictationStart);
                        audio_suppressor.suppress();
                    }
                    emit_state(&mut events, VoiceState::Dictating, &device_label)?;
                    continue;
                }
                if first_chunk {
                    if service.is_starting() {
                        // A client's dictation waited for the released
                        // microphone: it records from here.
                        service_recording = start_service_capture(
                            &mut capture,
                            &mut service,
                            &audio_suppressor,
                            indicator.as_ref(),
                            chunk_captured_through,
                            input.sample_rate(),
                        );
                        // A start that gave up releases the microphone here,
                        // as before, when the lab is off.
                        release_microphone!();
                    }
                    emit_current_state(&mut events, recording, pending, &device_label)?;
                    continue;
                }
                if service_recording {
                    if let Some(indicator) = &indicator {
                        indicator.meter(&samples);
                    }
                    service.audio(&samples, Instant::now());
                    // A client's dictation counts at once, so it streams at
                    // once: its finish is quicker, and it has live text.
                    capture.begin_stream(&engine);
                    if let Some(partial) = capture.report_live_text() {
                        service.partial(&partial);
                    }
                }
                if recording {
                    if let Some(indicator) = &indicator {
                        indicator.meter(&samples);
                    }
                    if capture.become_intentional(captured_through) && !recording_feedback_started {
                        if !start_tone_played {
                            crate::feedback::play(crate::feedback::Tone::DictationStart);
                        }
                        recording_feedback_started = true;
                        // Ordinary shortcut chords and short taps never mute,
                        // lower, or pause audio, or prevent sleep.
                        audio_suppressor.suppress();
                    }
                    // Short taps and shortcut chords never reach the model;
                    // the stream starts with the backlog once the hold counts.
                    if recording_feedback_started {
                        capture.begin_stream(&engine);
                    }
                    if let Some(partial) = capture.report_live_text()
                        && crate::labs::stop_phrase()
                    {
                        // Labs › Stop phrase: a phrase the model committed, or
                        // words read twice in a row, end a hands-free capture
                        // as its shortcut's press would.
                        let phrase = crate::labs::stop_phrase_text();
                        if stop_phrase.observe(&partial.stable, &partial.tentative, &phrase) {
                            hotkey.finish_hands_free(phrase);
                        }
                    }
                }
            }
            RecoveringAudioInputEvent::Timeout => {}
            RecoveringAudioInputEvent::Interrupted => {
                timeline.restart();
                // A release still in its tail finishes with the audio it has.
                if let Some(finish) = &mut finishing {
                    finish.release.stop_waiting();
                }
                // A press that was already waiting meets a stream that still
                // delivered nothing: the microphone is not coming back for
                // it, so it is discarded and reported as lost.
                let discard_pending = start_pending && !audio_ready;
                if discard_pending {
                    start_pending = false;
                }
                audio_ready = false;
                // An idle, tail-only, or annotate-only interruption is the
                // stream's own recovery and writes no failed dictation; a
                // capture or a press the stream took does.
                let mut lost = false;
                if service.is_active() {
                    end_service_capture(
                        &mut capture,
                        &audio_suppressor,
                        indicator.as_ref(),
                        &mut service_recording,
                    );
                    service.lost(MICROPHONE_LOST);
                    lost = true;
                } else if recording {
                    if capture.tentative_start().is_some() {
                        // A capture that has not passed the hold threshold
                        // would be discarded as a short tap anyway: reopen for
                        // the same press.
                        capture.cancel();
                        recording = false;
                        recording_feedback_started = false;
                        start_tone_played = false;
                        audio_suppressor.restore();
                        crate::feedback::retract_start();
                        start_pending = true;
                    } else {
                        audio_suppressor.restore();
                        crate::feedback::retract_start();
                        recording_feedback_started = false;
                        recording = false;
                        capture.cancel();
                        hotkey.command_capture(None);
                        events.dictation(DictationPhase::Cancelled, "")?;
                        if let Some(indicator) = &indicator {
                            indicator.send(crate::rune_hud::HudEvent::Cancelled);
                        }
                        lost = true;
                    }
                }
                if discard_pending {
                    hotkey.command_capture(None);
                    events.dictation(DictationPhase::Discarded, "")?;
                    if let Some(indicator) = &indicator {
                        indicator.send(crate::rune_hud::HudEvent::Discarded);
                    }
                    // The reopened stream never delivered, so the press
                    // never started: report it as lost, not as a quiet
                    // discard.
                    lost = true;
                }
                if start_pending {
                    // The held press asks for the microphone now, as a press
                    // during any recovery does.
                    input.retry_now();
                }
                if lost {
                    events.dictation(
                        DictationPhase::Failed("microphone stream interrupted; reopening".into()),
                        "",
                    )?;
                }
                emit_current_state(
                    &mut events,
                    recording || start_pending,
                    pending,
                    &device_label,
                )?;
            }
            RecoveringAudioInputEvent::Reopened => {
                timeline.restart();
                capture = DictationCapture::new(input.sample_rate());
                audio_ready = false;
                device_label = input.device_name().to_string();
                // A press waiting for the reopen is still dictating.
                emit_current_state(
                    &mut events,
                    recording || start_pending,
                    pending,
                    &device_label,
                )?;
            }
            RecoveringAudioInputEvent::OpenFailed(error) => {
                if !release_while_idle {
                    return Err(eyre!("could not reopen microphone: {error}"));
                }
                // On-demand open failed (device busy or unplugged): report
                // and stay released rather than killing the listener.
                if std::mem::take(&mut start_pending) {
                    hotkey.command_capture(None);
                }
                if service.is_active() {
                    end_service_capture(
                        &mut capture,
                        &audio_suppressor,
                        indicator.as_ref(),
                        &mut service_recording,
                    );
                    service.lost(MICROPHONE_LOST);
                }
                paste_target = None;
                audio_suppressor.restore();
                let error = format!("could not open microphone: {error}");
                tracing::warn!(error = %error, "the microphone would not open");
                fail(&events, error.clone())?;
                if let Some(indicator) = &indicator {
                    indicator.send(crate::rune_hud::HudEvent::Failed {
                        message: Some(error),
                    });
                }
                emit_current_state(&mut events, recording, pending, &device_label)?;
            }
        }
    }

    if recording || finishing.is_some() {
        capture.cancel();
        events.dictation(DictationPhase::Cancelled, "")?;
    }
    if let Some(mut host) = capture_host.take() {
        host.shutdown(&timeline);
    }
    emit_state(&mut events, VoiceState::Stopping, &device_label)?;
    drop(hotkey);
    drop(input);
    drop(jobs);
    // Accepted jobs still wait on the engine, so it stops after them.
    if worker.join().is_err() {
        return Err(eyre!("Windows dictation worker panicked"));
    }
    drop(engine);
    events.flush()?;
    println!("Stopped.");
    Ok(())
}

/// Runs blocking rewrite work on a helper thread so Escape can drop it.
/// Without a flag it runs in place. A cancelled job returns before its result
/// can paste; the helper thread's eventual result is discarded.
fn run_cancellable<T, F>(cancel: Option<&Arc<AtomicBool>>, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let Some(cancel) = cancel else {
        return work();
    };
    if cancel.load(Ordering::Acquire) {
        return Err(REWRITE_CANCELLED.into());
    }
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(work());
    });
    loop {
        match receiver.recv_timeout(REWRITE_CANCEL_POLL) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if cancel.load(Ordering::Acquire) {
                    return Err(REWRITE_CANCELLED.into());
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("the rewrite worker stopped".into());
            }
        }
    }
}

/// A near-silent clip is discarded for a dictation, but a rewrite keeps it: an
/// empty instruction falls back to the default prompt.
fn keeps_silent_clip(rewrite_selection: Option<&str>) -> bool {
    rewrite_selection.is_some()
}

/// The instruction a rewrite sends: the spoken words when there are any, else
/// the default prompt (which may name a snippet).
fn rewrite_instruction(
    transcript: JobTranscript,
    rewrite: &crate::dictation::rewrite::RewriteRuntime,
) -> Result<String, String> {
    let transcript = match transcript {
        JobTranscript::Pending(pending) => pending.wait().map(|transcribed| transcribed.text)?,
        JobTranscript::Empty => String::new(),
    };
    let transcript = transcript.trim().to_string();
    let instruction =
        crate::dictation::rewrite::resolve(&transcript, &rewrite.snippets, &rewrite.default_prompt)
            .map_err(|error| format!("{error:#}"))?;
    Ok(rewrite.layer_two(instruction))
}

/// Rewrites the selection with the instruction the person just spoke. A
/// failed or empty rewrite pastes nothing, so the selection stays exactly as it
/// was, and a rewrite never enters History: it is an edit, not a dictation.
fn process_rewrite_job(
    mut job: Job,
    paster: &mut Result<WindowsPaster>,
    rewrite: &crate::dictation::rewrite::RewriteRuntime,
) -> JobResult {
    let started = Instant::now();
    let selection = job.rewrite_selection.take().unwrap_or_default();
    let cancel = job.cancel.take();
    let transcript = job.transcript;
    let rewrite = rewrite.clone();
    let result = run_cancellable(cancel.as_ref(), move || {
        let instruction = rewrite_instruction(transcript, &rewrite)?;
        crate::dictation::rewrite::rewrite(&rewrite.model, &instruction, &selection)
            .map_err(|error| format!("{error:#}"))
    });
    let result = result.and_then(|rewritten| {
        paster
            .as_mut()
            .map_err(|error| format!("{error:#}"))?
            .paste(&rewritten, job.target.take(), false, job.return_to)
            .map_err(|error| format!("{error:#}"))?;
        Ok(rewritten)
    });
    let latency_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    tracing::info!(
        audio_ms = job.audio_ms,
        latency_ms,
        "completed Windows rewrite job"
    );
    job_result(OutputKind::Rewrite, result, latency_ms)
}

fn prime_microphone(
    input: &mut RecoveringAudioInput,
    capture: &mut DictationCapture,
    shutdown: &AtomicBool,
) -> Result<CaptureInstant> {
    while !shutdown.load(Ordering::Relaxed) {
        match input.recv_timeout(Duration::from_millis(100), true) {
            RecoveringAudioInputEvent::Chunk {
                samples,
                captured_through,
            } => {
                capture.ingest_with_pending(&samples, captured_through, None);
                return Ok(captured_through);
            }
            RecoveringAudioInputEvent::Timeout | RecoveringAudioInputEvent::Interrupted => {}
            RecoveringAudioInputEvent::Reopened => {
                *capture = DictationCapture::new(input.sample_rate());
            }
            RecoveringAudioInputEvent::OpenFailed(error) => {
                return Err(eyre!("could not reopen microphone: {error}"));
            }
        }
    }
    Ok(CaptureInstant::ZERO)
}

fn create_input(
    device: Option<&str>,
    fallback_to_default_device: bool,
    release_while_idle: bool,
) -> Result<RecoveringAudioInput> {
    let build = if release_while_idle {
        RecoveringAudioInput::closed
    } else {
        RecoveringAudioInput::open
    };
    if fallback_to_default_device {
        return Ok(build(None, 0, device));
    }
    let exact_name = device.map(resolve_device_name).transpose()?;
    Ok(build(exact_name.as_deref(), 0, None))
}

fn resolve_device_name(query: &str) -> Result<String> {
    let query_lower = query.to_lowercase();
    crate::audio::input_device_names()?
        .into_iter()
        .find(|name| name.to_lowercase().contains(&query_lower))
        .ok_or_else(|| eyre!("microphone is unavailable: {query}"))
}

/// Starts a client's dictation, which counts at once (no hold threshold),
/// and answers its start. A client that already gave up gets no recording.
/// True when it records.
fn start_service_capture(
    capture: &mut DictationCapture,
    service: &mut ServiceDictations,
    suppressor: &crate::windows::audio_control::AudioSuppressor,
    indicator: Option<&crate::rune_hud::HudSender>,
    started_at: CaptureInstant,
    sample_rate: u32,
) -> bool {
    capture.start_programmatic_at(started_at);
    if !service.started(sample_rate) {
        capture.cancel();
        return false;
    }
    suppressor.suppress();
    if let Some(indicator) = indicator {
        indicator.send(crate::rune_hud::HudEvent::Started);
    }
    true
}

/// Drops what is left of a client's ended dictation (its finish already took
/// the clip) and clears the HUD it showed.
fn end_service_capture(
    capture: &mut DictationCapture,
    suppressor: &crate::windows::audio_control::AudioSuppressor,
    indicator: Option<&crate::rune_hud::HudSender>,
    recording: &mut bool,
) {
    capture.cancel();
    suppressor.restore();
    if std::mem::take(recording)
        && let Some(indicator) = indicator
    {
        indicator.send(crate::rune_hud::HudEvent::Cancelled);
    }
}

/// Logs a failed dictation or paste with the sound set's error sound.
fn fail(events: &EventLog, error: impl Into<String>) -> Result<()> {
    let error = error.into();
    crate::feedback::play(crate::feedback::Tone::Error);
    tracing::warn!(%error, "Windows dictation failed");
    events.dictation(DictationPhase::Failed(error), "")?;
    Ok(())
}

/// Queues a repaste of the last dictation. Its job id keeps the result
/// queue aligned and is never announced to the HUD.
fn submit_paste_last(
    jobs: &SyncSender<OutputJob>,
    events: &mut EventLog,
    pending: &mut usize,
    next_job_id: &mut u64,
    job_ids: &mut std::collections::VecDeque<u64>,
) -> Result<()> {
    match jobs.try_send(OutputJob::PasteLast) {
        Ok(()) => {
            *pending += 1;
            *next_job_id += 1;
            job_ids.push_back(*next_job_id);
        }
        Err(TrySendError::Full(_)) => fail(events, "dictation output queue is full")?,
        Err(TrySendError::Disconnected(_)) => {
            return Err(eyre!("Windows dictation worker is unavailable"));
        }
    }
    Ok(())
}

fn emit_current_state(
    events: &mut EventLog,
    recording: bool,
    pending: usize,
    device: &str,
) -> Result<()> {
    let state = if recording {
        VoiceState::Dictating
    } else if pending > 0 {
        VoiceState::Transcribing
    } else {
        VoiceState::Listening
    };
    emit_state(events, state, device)
}

fn emit_state(events: &mut EventLog, state: VoiceState, device: &str) -> Result<()> {
    events.emit(&VoiceEvent::State {
        timestamp_ms: now_ms(),
        state,
        device: device.into(),
    })?;
    Ok(())
}

/// This thread as the one [`wake`] unparks, until the listener returns.
struct Woken(thread::ThreadId);

impl Woken {
    fn register() -> Self {
        let current = thread::current();
        let id = current.id();
        *LISTENER.lock().unwrap_or_else(|error| error.into_inner()) = Some(current);
        Self(id)
    }
}

impl Drop for Woken {
    fn drop(&mut self) {
        let mut listener = LISTENER.lock().unwrap_or_else(|error| error.into_inner());
        if listener
            .as_ref()
            .is_some_and(|thread| thread.id() == self.0)
        {
            *listener = None;
        }
    }
}

struct IndicatorGuard(Option<crate::rune_hud::HudSender>);

impl Drop for IndicatorGuard {
    fn drop(&mut self) {
        if let Some(indicator) = &self.0 {
            indicator.send(crate::rune_hud::HudEvent::Reset);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silent_rewrite_keeps_the_clip_while_a_silent_dictation_discards_it() {
        assert!(keeps_silent_clip(Some("selected text")));
        assert!(!keeps_silent_clip(None));
    }

    #[test]
    fn a_silent_rewrite_uses_the_default_prompt() {
        let rewrite = crate::dictation::rewrite::RewriteRuntime {
            model: Default::default(),
            snippets: Vec::new(),
            default_prompt: "Tighten it.".into(),
        };
        assert_eq!(
            rewrite_instruction(JobTranscript::Empty, &rewrite),
            Ok("Tighten it.".to_string())
        );
    }

    #[test]
    fn a_silent_rewrite_with_no_default_prompt_reports_nothing_to_do() {
        let rewrite = crate::dictation::rewrite::RewriteRuntime {
            model: Default::default(),
            snippets: Vec::new(),
            default_prompt: "  ".into(),
        };
        let error = rewrite_instruction(JobTranscript::Empty, &rewrite).unwrap_err();
        assert!(error.contains("nothing to do"), "{error}");
    }

    #[test]
    fn a_settings_update_with_an_excluded_app_pauses_the_shortcut() {
        let mut settings = crate::app_settings::AppSettings::default();
        settings.shortcuts.excluded_processes = vec!["game".into()];
        let next = WindowsDictationConfig::from_settings(&settings);
        assert!(next.shortcut_guard.pauses(Some("game"), false));
        assert!(!next.shortcut_guard.pauses(Some("notepad"), false));

        // The same config, after a second update, pauses in any fullscreen
        // window: both keys reach the listener through from_settings.
        settings.shortcuts.disable_in_fullscreen = true;
        let next = WindowsDictationConfig::from_settings(&settings);
        assert!(next.shortcut_guard.pauses(Some("notepad"), true));
        assert!(!next.shortcut_guard.pauses(Some("notepad"), false));
    }

    #[test]
    fn a_rewrite_recording_stops_after_ten_seconds_and_dictation_does_not() {
        use crate::dictation::DictationFlavour;

        assert!(!rewrite_recording_past_limit(
            DictationFlavour::Rewrite,
            Duration::from_millis(9_999)
        ));
        assert!(rewrite_recording_past_limit(
            DictationFlavour::Rewrite,
            REWRITE_RECORDING_LIMIT
        ));
        for flavour in [
            DictationFlavour::Normal,
            DictationFlavour::Raw,
            DictationFlavour::TerminalPaste,
        ] {
            assert!(
                !rewrite_recording_past_limit(flavour, Duration::from_secs(600)),
                "{flavour:?} has no cap"
            );
        }
        assert!(REWRITE_RECORDING_LIMIT_MESSAGE.contains("10 seconds"));
    }

    #[test]
    fn a_cancelled_rewrite_returns_before_its_result() {
        let cancel = Arc::new(AtomicBool::new(false));
        let (started, started_rx) = mpsc::channel::<()>();
        let (release, released) = mpsc::channel::<()>();
        let worker_cancel = Arc::clone(&cancel);
        let worker = thread::spawn(move || {
            run_cancellable(Some(&worker_cancel), move || {
                let _ = started.send(());
                let _ = released.recv();
                Ok("rewritten".to_string())
            })
        });
        started_rx.recv().unwrap();
        cancel.store(true, Ordering::Release);
        assert_eq!(worker.join().unwrap(), Err(REWRITE_CANCELLED.to_string()));
        let _ = release.send(());
    }

    #[test]
    fn a_rewrite_cancelled_before_it_starts_never_runs() {
        let cancel = Arc::new(AtomicBool::new(true));
        let result: Result<(), String> = run_cancellable(Some(&cancel), || panic!("must not run"));
        assert_eq!(result, Err(REWRITE_CANCELLED.to_string()));
    }

    #[test]
    fn a_rewrite_with_no_cancel_flag_still_runs() {
        assert_eq!(run_cancellable(None, || Ok::<_, String>(7)), Ok(7));
    }
}
