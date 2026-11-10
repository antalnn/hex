//! Dictation modes for the Windows and Linux listeners. The focused
//! application or website selects a mode, and its replacements,
//! post-processing, and transformations run in the shared
//! `dictation_processor` order. macOS selects modes through `config`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use url::Url;

use crate::app_settings::{AppSettings, DictationMode, DictationProcessingSettings};
use crate::dictation::processor::{PostProcessingSettings, Processed, Profile, Profiles};
use crate::dictation::text_replacements::ReplacementSet;

/// Live mode-processing snapshot. The output worker clones it before any
/// deadline-bounded OpenCode call, so settings writes never wait on
/// inference.
#[derive(Clone)]
pub struct ModeRuntime {
    replacements: ReplacementSet,
    default: PostProcessingSettings,
    default_transformations: Vec<String>,
    default_vocabulary: Vec<crate::dictation::vocabulary::VocabularyEntry>,
    /// The shared terms marked `code`, wrapped in backticks after the rewrite
    /// when `wrap_code_terms` is on.
    code_terms: Vec<crate::dictation::vocabulary::VocabularyEntry>,
    wrap_code_terms: bool,
    /// Where dictation cleanup's model call goes, and the local server's port.
    ai_source: crate::app_settings::ModelSource,
    llm_port: u16,
    /// Whether the `ai-rewrite` step runs at all.
    ai_cleanup_enabled: bool,
    /// The instruction the model receives; empty falls back in
    /// [`crate::dictation::ai_rewrite::clean`].
    ai_cleanup_prompt: String,
    /// How long the cleanup call may wait before it gives up.
    ai_deadline: Duration,
    /// The Global mode's steps switched off in the Output list.
    default_skipped: Vec<String>,
    modes: Vec<DictationMode>,
    reads_page: bool,
}

impl ModeRuntime {
    pub fn from_settings(settings: &AppSettings) -> Self {
        let processing = &settings.dictation_processing;
        // The vocabulary's "sounds like" spellings correct the transcript on
        // every model, so they run here with the default mode's own swaps.
        let mut global = crate::dictation::vocabulary::replacements(&settings.vocabulary);
        global.extend(processing.default_mode.replacements.iter().cloned());
        Self {
            replacements: ReplacementSet::new(&global),
            default: processing.default_mode.post_processing.clone(),
            default_transformations: processing.default_mode.transformations.clone(),
            default_vocabulary: processing.default_mode.vocabulary.clone(),
            code_terms: settings
                .vocabulary
                .iter()
                .filter(|entry| entry.code)
                .cloned()
                .collect(),
            wrap_code_terms: processing.wrap_code_terms,
            ai_source: settings.dictation_rewrite.clone(),
            llm_port: settings.llm.port,
            ai_cleanup_enabled: settings.ai_cleanup.enabled,
            ai_cleanup_prompt: settings.ai_cleanup.prompt.clone(),
            ai_deadline: Duration::from_secs(settings.ai_cleanup.deadline_seconds.max(1)),
            default_skipped: processing.default_mode.skipped.clone(),
            modes: processing.modes.clone(),
            reads_page: uses_page(processing),
        }
    }

    /// Whether processing looks at the focused page: a mode chosen by
    /// website, an OpenCode rewrite, or a script transformation. Otherwise
    /// the listener never asks the browser for its address, as on macOS.
    pub fn reads_page(&self) -> bool {
        self.reads_page
    }

    /// Global replacements, then the matching mode's corrections,
    /// post-processing, and transformations.
    pub fn process(
        &self,
        raw_text: &str,
        language: &str,
        context: &crate::dictation::context::ContextSnapshot,
        transformations: &crate::personal_commands::TransformationClient,
        source: crate::dictation::processor::Source,
    ) -> Processed {
        self.process_cancellable(
            raw_text,
            language,
            context,
            transformations,
            &AtomicBool::new(false),
            source,
        )
    }

    /// [`Self::process`] with a cancellation flag the stages check, for work
    /// that ends early when Hex quits.
    pub fn process_cancellable(
        &self,
        raw_text: &str,
        language: &str,
        context: &crate::dictation::context::ContextSnapshot,
        transformations: &crate::personal_commands::TransformationClient,
        cancelled: &AtomicBool,
        source: crate::dictation::processor::Source,
    ) -> Processed {
        let text = self.replacements.replace(raw_text);
        let profile = self.profile(context);
        let mut processed =
            Profiles::new(profile).process_cancellable(&text, language, context, cancelled, source);
        let ids = processed.transformations.clone();
        if !self.ai_cleanup_enabled {
            // Off drops the step from the ordering entirely: the rules on
            // either side run back to back, with no model call and no gap.
            processed.transformations = ids
                .into_iter()
                .filter(|id| id != crate::dictation::ai_rewrite::TRANSFORMATION_ID)
                .collect();
            processed.apply_transformations(transformations, context, cancelled, source);
        } else {
            match ids
                .iter()
                .position(|id| id == crate::dictation::ai_rewrite::TRANSFORMATION_ID)
            {
                None => {
                    processed.apply_transformations(transformations, context, cancelled, source);
                }
                Some(index) => {
                    // The model step sits mid-list: the rules decided from the raw
                    // words (bullets, orphans) run before it, and the rules applied
                    // to its output (paragraphs, colon cues, snippets) after.
                    processed.transformations = ids[..index].to_vec();
                    processed.apply_transformations(transformations, context, cancelled, source);
                    self.run_ai(&mut processed, cancelled);
                    processed.transformations = ids[index + 1..].to_vec();
                    processed.apply_transformations(transformations, context, cancelled, source);
                    processed.transformations = ids;
                }
            }
        }
        // After the rewrite: the model may lowercase or drop a backtick it saw
        // in its input, so the wrap is the last step that touches the text.
        if self.wrap_code_terms {
            processed.text =
                crate::dictation::vocabulary::wrap_code_terms(&processed.text, &self.code_terms);
        }
        processed
    }

    /// The model step itself, between the two halves of the chain. A failure
    /// keeps the text and records why, the way the script returned its input: a
    /// dictation has to produce something.
    fn run_ai(&self, processed: &mut Processed, cancelled: &AtomicBool) {
        if cancelled.load(Ordering::Acquire) {
            return;
        }
        let started = std::time::Instant::now();
        let result = crate::dictation::ai_rewrite::clean(
            &self.ai_source,
            self.llm_port,
            self.ai_deadline,
            &self.ai_cleanup_prompt,
            &processed.text,
        );
        let observation = processed.observation.get_or_insert_with(|| {
            crate::dictation::processor::ProcessingObservation {
                profile: "Custom transformations".into(),
                latency_ms: 0,
                fallback: None,
            }
        });
        observation.latency_ms = observation
            .latency_ms
            .saturating_add(started.elapsed().as_millis() as u64);
        match result {
            Ok(text) => processed.text = text,
            Err(error) => {
                tracing::warn!(%error, "the dictation cleanup was skipped");
                observation.fallback = Some(format!("{error:#}"));
            }
        }
    }

    /// The matching mode's profile, or Global's, with the steps switched off
    /// in the Output list left out of the ordering. They keep their place in
    /// the saved list, so turning one back on restores its order.
    fn profile(&self, context: &crate::dictation::context::ContextSnapshot) -> Profile {
        let mode = mode_for_context(
            &self.modes,
            context.application.as_deref(),
            context.browser_host(),
        );
        mode.map_or_else(
            || {
                let transformations = self
                    .default_transformations
                    .iter()
                    .filter(|id| !self.default_skipped.contains(id))
                    .cloned()
                    .collect();
                Profile::configured(
                    "Global",
                    ReplacementSet::new(&crate::dictation::vocabulary::replacements(
                        &self.default_vocabulary,
                    )),
                    transformations,
                    &self.default,
                )
                .vocabulary(self.default_vocabulary.clone())
            },
            |mode| {
                let skipped = mode.skipped.clone();
                let mut mode = mode.clone();
                mode.transformations.retain(|id| !skipped.contains(id));
                Profile::from_mode(&mode)
            },
        )
    }
}

/// The live mode runtime and transformation client a listener shares with
/// the local API's `process=global` processor, so saved settings and the
/// personal-commands host apply to both.
#[cfg(any(target_os = "linux", target_os = "windows"))]
#[derive(Clone)]
pub struct ProcessingState {
    pub modes: std::sync::Arc<std::sync::RwLock<ModeRuntime>>,
    pub transformations: std::sync::Arc<crate::personal_commands::TransformationClient>,
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
impl ProcessingState {
    /// Reads `settings` once; a listener replaces `modes` as settings change.
    pub fn from_settings(settings: &AppSettings) -> Self {
        Self {
            modes: std::sync::Arc::new(std::sync::RwLock::new(ModeRuntime::from_settings(
                settings,
            ))),
            transformations: std::sync::Arc::new(
                crate::personal_commands::TransformationClient::default(),
            ),
        }
    }
}

/// Whether anything set up in `settings` looks at the focused page: a mode
/// chosen by website, an OpenCode rewrite (its prompt names the site), or a
/// script transformation (it receives the page address). Every OS asks the
/// browser for its page only then (macOS through `config::reads_browser`).
pub(crate) fn uses_page(settings: &DictationProcessingSettings) -> bool {
    settings
        .modes
        .iter()
        .any(|mode| !mode.browser_hosts.is_empty())
        || std::iter::once(&settings.default_mode)
            .chain(&settings.modes)
            .any(|mode| {
                mode.post_processing.enabled
                    || mode
                        .transformations
                        .iter()
                        .any(|id| !crate::personal_commands::is_builtin_transformation(id))
            })
}

/// The history entry for one pasted dictation.
pub fn history_draft(
    raw_text: String,
    processed: &Processed,
    application: Option<String>,
    audio_ms: u64,
    inference_ms: u64,
    total_ms: u64,
    recording: Option<crate::history::Recording>,
) -> crate::history::HistoryDraft {
    crate::history::HistoryDraft {
        kind: crate::history::HistoryKind::Dictation,
        raw_text,
        final_text: processed.text.clone(),
        application,
        processing: processed.observation.as_ref().map(|processing| {
            crate::history::HistoryProcessing {
                profile: processing.profile.clone(),
                latency_ms: processing.latency_ms,
                fallback: processing.fallback.clone(),
            }
        }),
        audio_ms,
        inference_ms,
        total_ms,
        recording,
        model: crate::history::dictation_model(),
    }
}

/// Whether a mode's applications match the focused application. Windows
/// and Linux name applications by executable, and a rule matches any
/// case-insensitive substring of that name ("code" matches "vscode").
fn matches_application(mode: &DictationMode, application: &str) -> bool {
    let application = application.to_lowercase();
    mode.applications.iter().any(|candidate| {
        let candidate = candidate.trim().to_lowercase();
        !candidate.is_empty() && application.contains(&candidate)
    })
}

/// Whether a mode's websites match the focused page's exact host.
fn matches_website(mode: &DictationMode, host: &str) -> bool {
    let host = normalize_host(host);
    mode.browser_hosts.iter().any(|candidate| {
        // Entries are stored as typed; pasted URLs, ports, and IDN text
        // reduce to their host here.
        canonical_host(candidate).is_some_and(|candidate| candidate == host)
    })
}

/// The first mode matching the focused context. A website rule is more
/// specific than an application rule, so browser-host matches win, like
/// the macOS selector ordering.
fn mode_for_context<'a>(
    modes: &'a [DictationMode],
    application: Option<&str>,
    browser_host: Option<&str>,
) -> Option<&'a DictationMode> {
    if let Some(host) = browser_host
        && let Some(mode) = modes.iter().find(|mode| matches_website(mode, host))
    {
        return Some(mode);
    }
    let application = application?;
    modes
        .iter()
        .find(|mode| matches_application(mode, application))
}

/// Hosts compare case-insensitively with any trailing dot removed,
/// matching the macOS browser-host semantics.
pub fn normalize_host(host: &str) -> String {
    host.trim_end_matches('.').to_lowercase()
}

/// A website rule entry reduced to its comparable host: bare hosts,
/// pasted URLs, host:port forms, and IDN text all work, matching the
/// macOS Websites field.
pub fn canonical_host(entry: &str) -> Option<String> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    let parsed = Url::parse(entry)
        .ok()
        .filter(|url| url.host_str().is_some())
        .or_else(|| Url::parse(&format!("https://{entry}")).ok())?;
    parsed.host_str().map(normalize_host)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(name: &str, applications: &[&str], browser_hosts: &[&str]) -> DictationMode {
        DictationMode {
            name: name.into(),
            applications: applications.iter().map(|value| value.to_string()).collect(),
            browser_hosts: browser_hosts
                .iter()
                .map(|value| value.to_string())
                .collect(),
            ..DictationMode::default()
        }
    }

    fn matched<'a>(
        modes: &'a [DictationMode],
        application: Option<&str>,
        host: Option<&str>,
    ) -> Option<&'a str> {
        mode_for_context(modes, application, host).map(|mode| mode.name.as_str())
    }

    fn runtime(settings: impl FnOnce(&mut AppSettings)) -> ModeRuntime {
        let mut value = AppSettings::default();
        settings(&mut value);
        ModeRuntime::from_settings(&value)
    }

    #[test]
    fn a_vocabulary_spelling_corrects_the_transcript() {
        use crate::dictation::vocabulary::VocabularyEntry;

        let runtime = runtime(|settings| {
            settings.vocabulary = vec![VocabularyEntry {
                term: "OpenCode".into(),
                sounds_like: vec!["open code".into()],
                code: false,
            }];
            settings
                .dictation_processing
                .default_mode
                .transformations
                .clear();
            settings
                .dictation_processing
                .default_mode
                .post_processing
                .enabled = false;
        });
        let context = crate::dictation::context::ContextSnapshot {
            application: None,
            browser_url: None,
            window_title: None,
            input_revision: None,
        };
        let processed = runtime.process(
            "I opened open code today",
            "en",
            &context,
            &crate::personal_commands::TransformationClient::default(),
            crate::dictation::processor::Source::Dictation,
        );
        assert_eq!(processed.text, "I opened OpenCode today");
    }

    #[test]
    fn a_skipped_step_keeps_its_place_but_does_not_run() {
        let context = crate::dictation::context::ContextSnapshot {
            application: None,
            browser_url: None,
            window_title: None,
            input_revision: None,
        };
        let client = crate::personal_commands::TransformationClient::default();
        let run = |skipped: Vec<String>| {
            let runtime = runtime(move |settings| {
                let mode = &mut settings.dictation_processing.default_mode;
                mode.transformations = vec!["lowercase".into()];
                mode.skipped = skipped;
                mode.post_processing.enabled = false;
            });
            runtime
                .process(
                    "MiXeD Words",
                    "en",
                    &context,
                    &client,
                    crate::dictation::processor::Source::Dictation,
                )
                .text
        };
        assert_eq!(run(Vec::new()), "mixed words");
        assert_eq!(run(vec!["lowercase".into()]), "MiXeD Words");
    }

    #[test]
    fn a_code_term_wraps_in_backticks_when_the_setting_is_on() {
        use crate::dictation::vocabulary::VocabularyEntry;

        let build = |on: bool| {
            runtime(move |settings| {
                settings.vocabulary = vec![VocabularyEntry {
                    term: "useState".into(),
                    sounds_like: vec!["use state".into()],
                    code: true,
                }];
                settings.dictation_processing.wrap_code_terms = on;
                settings
                    .dictation_processing
                    .default_mode
                    .transformations
                    .clear();
                settings
                    .dictation_processing
                    .default_mode
                    .post_processing
                    .enabled = false;
            })
        };
        let context = crate::dictation::context::ContextSnapshot {
            application: None,
            browser_url: None,
            window_title: None,
            input_revision: None,
        };
        let client = crate::personal_commands::TransformationClient::default();
        let process = |runtime: ModeRuntime, text: &str| {
            runtime
                .process(
                    text,
                    "en",
                    &context,
                    &client,
                    crate::dictation::processor::Source::Dictation,
                )
                .text
        };

        // The setting corrects the heard spelling and then wraps it.
        assert_eq!(
            process(build(true), "I called use state today"),
            "I called `useState` today"
        );
        // The transcriber already spelled it right: the wrap still finds it.
        assert_eq!(
            process(build(true), "Open useState docs"),
            "Open `useState` docs"
        );
        assert_eq!(
            process(build(false), "I called use state today"),
            "I called useState today"
        );
        assert_eq!(
            process(build(false), "Open useState docs"),
            "Open useState docs"
        );
    }

    #[test]
    fn modes_match_case_insensitive_application_substrings() {
        let modes = [
            mode("Code", &["Code", "rustrover"], &[]),
            mode("Chat", &["slack"], &[]),
        ];
        assert_eq!(matched(&modes, Some("code"), None), Some("Code"));
        assert_eq!(matched(&modes, Some("slack"), None), Some("Chat"));
        assert_eq!(matched(&modes, Some("vscode"), None), Some("Code"));
        assert_eq!(matched(&modes, Some("notepad"), None), None);
        assert_eq!(matched(&modes, None, None), None);
    }

    #[test]
    fn website_rules_match_exact_hosts_and_beat_application_rules() {
        let modes = [
            mode("Browser", &["chrome"], &[]),
            mode("X", &[], &["X.com"]),
        ];
        assert_eq!(matched(&modes, Some("chrome"), Some("x.com.")), Some("X"));
        assert_eq!(
            matched(&modes, Some("chrome"), Some("news.x.com")),
            Some("Browser")
        );
        assert_eq!(matched(&modes, Some("chrome"), None), Some("Browser"));
    }

    #[test]
    fn the_page_is_read_only_when_processing_uses_it() {
        assert!(!runtime(|_| {}).reads_page());
        assert!(
            !runtime(|settings| {
                settings.dictation_processing.modes = vec![DictationMode {
                    transformations: vec!["lowercase".into()],
                    ..mode("Code", &["code"], &[])
                }];
            })
            .reads_page()
        );
        assert!(
            runtime(|settings| {
                settings.dictation_processing.modes = vec![mode("X", &[], &["x.com"])];
            })
            .reads_page()
        );
        assert!(
            runtime(|settings| {
                settings
                    .dictation_processing
                    .default_mode
                    .post_processing
                    .enabled = true;
            })
            .reads_page()
        );
    }

    #[test]
    fn blank_rules_never_match() {
        let modes = [mode("Broken", &["  ", ""], &[" "])];
        assert_eq!(matched(&modes, Some("anything"), Some("x.com")), None);
    }

    #[test]
    fn global_replacements_run_before_mode_corrections() {
        let runtime = runtime(|settings| {
            let processing = &mut settings.dictation_processing;
            processing.default_mode.replacements =
                vec![crate::dictation::text_replacements::TextReplacement {
                    matched_phrase: "hex".into(),
                    output: "Hex".into(),
                }];
            processing.modes = vec![DictationMode {
                replacements: vec![crate::dictation::text_replacements::TextReplacement {
                    matched_phrase: "open code".into(),
                    output: "OpenCode".into(),
                }],
                ..mode("Code", &["code"], &[])
            }];
        });
        let context = crate::dictation::context::ContextSnapshot {
            application: Some("Visual Studio Code".into()),
            ..crate::dictation::context::ContextSnapshot::default()
        };
        let transformations = crate::personal_commands::TransformationClient::default();

        let processed = runtime.process(
            "Use open code in hex.",
            "en",
            &context,
            &transformations,
            crate::dictation::processor::Source::Dictation,
        );

        assert_eq!(processed.text, "Use OpenCode in Hex.");
        assert!(processed.observation.is_none());
    }

    #[test]
    fn cleanup_on_runs_the_model_step_between_the_rules() {
        // Port 9 answers nothing, so the model step fails and keeps the text:
        // what the test checks is where it sat, not what it said.
        let runtime = runtime(|settings| {
            settings.llm.port = 9;
            settings.dictation_processing.default_mode.transformations = vec![
                "lowercase".into(),
                crate::dictation::ai_rewrite::TRANSFORMATION_ID.into(),
                "no-trailing-punctuation".into(),
            ];
        });
        let transformations = crate::personal_commands::TransformationClient::default();

        let processed = runtime.process(
            "Keep OpenCode Exact!",
            "en",
            &crate::dictation::context::ContextSnapshot::default(),
            &transformations,
            crate::dictation::processor::Source::Dictation,
        );

        // Lowercase ran before the model, no-trailing-punctuation after it, and
        // the model's own failure left the text alone.
        assert_eq!(processed.text, "keep opencode exact");
        let observation = processed.observation.as_ref().expect("the step ran");
        assert!(
            observation
                .fallback
                .as_deref()
                .is_some_and(|reason| reason.contains("did not answer")),
            "{observation:?}"
        );
        // The full list is left on the result, as before the split.
        assert_eq!(
            processed.transformations,
            [
                "lowercase".to_string(),
                crate::dictation::ai_rewrite::TRANSFORMATION_ID.to_string(),
                "no-trailing-punctuation".to_string()
            ]
        );
    }

    #[test]
    fn cleanup_off_skips_the_model_step_and_keeps_the_rule_order() {
        let build = |enabled: bool| {
            runtime(|settings| {
                settings.llm.port = 9;
                settings.ai_cleanup.enabled = enabled;
                settings.dictation_processing.default_mode.transformations = vec![
                    "lowercase".into(),
                    crate::dictation::ai_rewrite::TRANSFORMATION_ID.into(),
                    "no-trailing-punctuation".into(),
                ];
            })
        };
        let absent = runtime(|settings| {
            settings.dictation_processing.default_mode.transformations =
                vec!["lowercase".into(), "no-trailing-punctuation".into()];
        });
        let transformations = crate::personal_commands::TransformationClient::default();
        let context = crate::dictation::context::ContextSnapshot::default();
        let run = |runtime: &ModeRuntime| {
            runtime.process(
                "Keep OpenCode Exact!",
                "en",
                &context,
                &transformations,
                crate::dictation::processor::Source::Dictation,
            )
        };

        let off = run(&build(false));
        let without = run(&absent);

        // The rules on either side ran back to back; no model call happened.
        assert_eq!(off.text, "keep opencode exact");
        assert!(
            off.observation
                .as_ref()
                .is_none_or(|observation| observation.fallback.is_none()),
            "{:?}",
            off.observation
        );
        assert_eq!(off.text, without.text);
        assert_eq!(off.transformations, without.transformations);
    }

    #[test]
    fn cleanup_applies_its_saved_deadline_with_a_one_second_floor() {
        let saved = runtime(|settings| settings.ai_cleanup.deadline_seconds = 45);
        assert_eq!(saved.ai_deadline, Duration::from_secs(45));

        let floored = runtime(|settings| settings.ai_cleanup.deadline_seconds = 0);
        assert_eq!(floored.ai_deadline, Duration::from_secs(1));
    }

    #[test]
    fn modes_execute_selected_transformations_after_corrections() {
        let runtime = runtime(|settings| {
            settings.dictation_processing.default_mode.transformations = vec!["lowercase".into()];
        });
        let transformations = crate::personal_commands::TransformationClient::default();

        let processed = runtime.process(
            "Keep OpenCode Exact.",
            "en",
            &crate::dictation::context::ContextSnapshot::default(),
            &transformations,
            crate::dictation::processor::Source::Dictation,
        );

        assert_eq!(processed.text, "keep opencode exact.");
        let observation = processed.observation.as_ref().expect("transformation ran");
        assert_eq!(observation.profile, "Custom transformations");
        assert_eq!(observation.fallback, None);
    }

    #[test]
    fn saved_post_processing_enables_the_global_rewrite() {
        let path = std::env::temp_dir().join(format!("hex-970-{}.json", std::process::id()));
        std::fs::write(
            &path,
            br#"{"dictation_processing":{"default_mode":{"post_processing":{"enabled":true,"prompt":"Fix it.","model":"opencode/big-pickle","deadline_seconds":30}}}}"#,
        )
        .unwrap();
        let (settings, _) = AppSettings::read_from(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let runtime = ModeRuntime::from_settings(&settings);
        let context = crate::dictation::context::ContextSnapshot {
            application: Some("kitty".into()),
            ..crate::dictation::context::ContextSnapshot::default()
        };
        assert!(runtime.profile(&context).rewrites());
    }

    #[test]
    fn website_entries_reduce_to_their_host() {
        assert_eq!(normalize_host("X.com."), "x.com");
        assert_eq!(
            canonical_host("https://x.com/home").as_deref(),
            Some("x.com")
        );
        assert_eq!(
            canonical_host("github.com/hex/pull/1").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            canonical_host("localhost:3000").as_deref(),
            Some("localhost")
        );
        assert_eq!(canonical_host("   "), None);
    }
}
