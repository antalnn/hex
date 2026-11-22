//! Finishing a Windows capture: the clip goes to the dictation worker as the
//! job its output stage reads.

use super::*;

/// Submits the finished capture. `true` when a job was queued; a rewrite's job
/// also returns the flag Escape sets to drop it before it pastes.
#[allow(clippy::too_many_arguments)]
pub(super) fn submit_capture(
    capture: &mut DictationCapture,
    ended_at: CaptureInstant,
    tail: Duration,
    engine: &TranscriptionEngine,
    jobs: &SyncSender<OutputJob>,
    events: &mut EventLog,
    pending: &mut usize,
    print: Option<PrintReply>,
    target: Option<crate::windows::paste_target::PasteTarget>,
    rewrite_selection: Option<String>,
    flavour: crate::dictation::DictationFlavour,
    by_shortcut: bool,
    context: crate::windows::context::PendingContext,
    stop_phrase: Option<String>,
    return_to: Option<crate::windows::paste::CaptureWindow>,
) -> Result<(bool, Option<Arc<AtomicBool>>)> {
    let mut submitted = false;
    let mut rewrite_cancel = None;
    let no_speech = |print: Option<PrintReply>| {
        if let Some(print) = print {
            print.send(PrintedTranscript::NoSpeech);
        }
    };
    match capture.finish_with_tail(ended_at, tail) {
        Finish::Discard => {
            no_speech(print);
            events.dictation(DictationPhase::Discarded, "")?
        }
        Finish::Transcribe(clip) if clip.is_near_silent() => {
            if !keeps_silent_clip(rewrite_selection.as_deref()) {
                no_speech(print);
                events.dictation(DictationPhase::Discarded, "")?;
            } else {
                tracing::info!("silent rewrite uses the default prompt");
                let audio_ms = clip.duration_ms();
                let language = engine
                    .held_selection()
                    .unwrap_or_else(|| crate::app_settings::transcription_selection().1)
                    .language;
                let cancel = Some(Arc::new(AtomicBool::new(false)));
                let job = OutputJob::Dictation(Box::new(Job {
                    context,
                    transcript: JobTranscript::Empty,
                    audio_ms,
                    language,
                    recording: None,
                    target,
                    rewrite_selection,
                    cancel: cancel.clone(),
                    flavour,
                    print: None,
                    by_shortcut,
                    stop_phrase,
                    return_to,
                }));
                match jobs.try_send(job) {
                    Ok(()) => {
                        *pending += 1;
                        submitted = true;
                        rewrite_cancel = cancel;
                        events.dictation(DictationPhase::Transcribing, "")?;
                    }
                    Err(TrySendError::Full(_)) => fail(events, "dictation queue is full")?,
                    Err(TrySendError::Disconnected(_)) => {
                        return Err(eyre!("Windows dictation worker is unavailable"));
                    }
                }
            }
        }
        Finish::Transcribe(mut clip) => {
            let audio_ms = clip.duration_ms();
            let stream = clip.take_stream();
            let samples = clip.into_transcription_samples();
            let language = engine
                .held_selection()
                .unwrap_or_else(|| crate::app_settings::transcription_selection().1)
                .language;
            let recording = crate::history::Recording::keep(&samples);
            let cancel = rewrite_selection
                .is_some()
                .then(|| Arc::new(AtomicBool::new(false)));
            let job = OutputJob::Dictation(Box::new(Job {
                context,
                transcript: JobTranscript::Pending(engine.finish(stream, samples)?),
                audio_ms,
                language,
                recording,
                target,
                print,
                rewrite_selection,
                cancel: cancel.clone(),
                flavour,
                by_shortcut,
                stop_phrase,
                return_to,
            }));
            match jobs.try_send(job) {
                Ok(()) => {
                    *pending += 1;
                    submitted = true;
                    rewrite_cancel = cancel;
                    events.dictation(DictationPhase::Transcribing, "")?;
                }
                Err(TrySendError::Full(_)) => fail(events, "dictation queue is full")?,
                Err(TrySendError::Disconnected(_)) => {
                    return Err(eyre!("Windows dictation worker is unavailable"));
                }
            }
        }
    }
    Ok((submitted, rewrite_cancel))
}
