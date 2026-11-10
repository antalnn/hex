//! The dictation cleanup's model call: one native step in the mode's
//! transformation list, where the `local-rewrite` QuickJS transformation used
//! to sit.
//!
//! Native, not a script, for two reasons: the settings decide between Hex's
//! own server and a remote endpoint, and a compiled step needs no trust
//! approval, so editing nothing under `~/.config/hex` and the "needs
//! permission" pause never happens for this step.
//!
//! A failure keeps the input: a dictation has to produce something, and the
//! rule the script had is kept here. The guards are the script's own, ported:
//! a reply that is empty, much shorter than the input, or that invents too many
//! words is not a cleanup.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use color_eyre::eyre::{Result, eyre};
use serde::{Deserialize, Serialize};

use crate::app_settings::ModelSource;

/// The transformation id this step answers to. It sits in the mode's list at
/// the point the model runs: after the rules decided from the raw words
/// (bullets, orphans) and before the rules applied to the model's output
/// (paragraphs, colon cues, snippets).
///
/// Serialised into `settings.json`, so renaming it silently drops the step
/// from every saved mode. Change the label ([`NAME`]) instead.
pub const TRANSFORMATION_ID: &str = "ai-rewrite";

/// What the app calls this step.
pub const NAME: &str = "AI cleanup";

/// What the Modes pane says it does.
pub const DESCRIPTION: &str =
    "Clean up dictation with a model: the local one, or the remote endpoint the settings name";

/// How long one call may take by default, the script's own budget. The saved
/// `ai_cleanup.deadline_seconds` replaces it.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
/// The reply budget for the local server, which runs with a 4,096-token
/// context: a long dictation plus this cap must still fit inside it.
const MAX_TOKENS: u32 = 1_500;
/// The reply budget for a remote endpoint, which covers reasoning as well as
/// the text; [`crate::dictation::rewrite`] explains the measurement.
const REMOTE_MAX_TOKENS: u32 = 4_096;
/// Shorter than this is not worth a model call.
const MIN_CHARS: usize = 12;

/// What the model is told. Ported verbatim from the `local-rewrite`
/// transformation, so the wording the prompt was measured with is unchanged.
pub(crate) const SYSTEM_PROMPT: &str = "You clean up dictated speech. You are not an editor.

Fix only:
- grammar, word order, and agreement that is not English
- false starts, repeated words, and abandoned clauses
- punctuation and capitalization
- a full stop in the middle of a thought, where the transcriber paused:
  \"so I decided. To just make something\" becomes \"so I decided to just make
  something\"

If a sentence is unclear but what it means is obvious, repair it in the
speaker's own words. If not, leave it alone.

Paragraphs: break where the topic shifts, never inside a sentence. A dictation
over about 90 words gets at least two paragraphs. Keep every line break and
paragraph break the speaker dictated.

Formatting: those blank lines are the only thing you may add. Never add a
heading, bullet, number or code block, and never turn prose into a list. If the
text already contains a list, keep every item and its marker exactly as
written.

Keep backticks, file paths, code and identifiers exactly as written: never
change their spelling or case.

Never:
- add, remove, reorder, or summarize content
- replace the speaker's words with synonyms
- address the reader, or add a greeting, sign-off, or any sentence the speaker
  did not say

Reply with the corrected text only, starting with its first word.";

#[derive(Clone, Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct Request {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    messages: Vec<Message>,
    temperature: f32,
    max_tokens: u32,
    stream: bool,
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Content,
}

#[derive(Deserialize)]
struct Content {
    #[serde(default)]
    content: String,
}

/// The instruction the model receives: the saved prompt, or the shipped
/// wording when the saved one is empty.
fn system_instruction(saved: &str) -> &str {
    let saved = saved.trim();
    if saved.is_empty() {
        SYSTEM_PROMPT
    } else {
        saved
    }
}

/// Cleans `text` with the model `source` names. `instruction` is the saved AI
/// cleanup prompt; empty uses [`SYSTEM_PROMPT`], so clearing the box never
/// degrades silently. `deadline` is how long the call may wait. `Err` means
/// "keep the text as it is"; the caller records why.
pub(crate) fn clean(
    source: &ModelSource,
    port: u16,
    deadline: Duration,
    instruction: &str,
    text: &str,
) -> Result<String> {
    // A snippet cue is a command, not prose: answer it here, before the model
    // can reword it into a miss, and so a cue costs no model call. The final
    // `snippets` step still covers everything this misses.
    if let Some(snippet) = snippet_text(text) {
        return Ok(snippet);
    }
    if text.trim().chars().count() < MIN_CHARS {
        return Ok(text.to_string());
    }
    if let Some(missing) = source.missing() {
        return Err(eyre!("{missing}"));
    }
    let who = if source.is_remote() {
        "model"
    } else {
        "local model"
    };
    let url = source.chat_url(port);
    let system = system_instruction(instruction);
    let body = Request {
        model: source
            .is_remote()
            .then(|| source.remote.model.trim().to_string()),
        messages: vec![
            Message {
                role: "system",
                content: system.to_string(),
            },
            Message {
                role: "user",
                content: text.to_string(),
            },
        ],
        temperature: 0.0,
        max_tokens: if source.is_remote() {
            REMOTE_MAX_TOKENS
        } else {
            MAX_TOKENS
        },
        stream: false,
    };
    let agent = if source.is_remote() {
        crate::net::agent(&url, |settings| settings.timeout_global(Some(deadline)))
    } else {
        crate::net::loopback_agent(|settings| settings.timeout_global(Some(deadline)))
    };
    let json = serde_json::to_string(&body)
        .map_err(|error| eyre!("the cleanup request could not be built: {error}"))?;

    let mut request = agent.post(&url).header("content-type", "application/json");
    if source.is_remote() {
        for (name, value) in crate::dictation::rewrite::remote_headers(&source.remote) {
            request = request.header(name, &value);
        }
    }
    let mut response = request
        .send(&json)
        .map_err(|error| eyre!("the {who} did not answer: {}", crate::net::reason(&error)))?;
    let reply = response
        .body_mut()
        .read_to_string()
        .map_err(|error| eyre!("the {who}'s reply could not be read: {error}"))?;
    let parsed: Response = serde_json::from_str(&reply)
        .map_err(|error| eyre!("the {who}'s reply was not JSON: {error}"))?;
    let output = parsed
        .choices
        .first()
        .map(|choice| choice.message.content.trim().to_string())
        .unwrap_or_default();
    accept(text, &output, who)
}

/// The script's guards: a reply is kept only when it is a cleanup of the same
/// words. Empty, much shorter, or too many new words all mean the model
/// wandered, and the input is better than that.
fn accept(input: &str, output: &str, who: &str) -> Result<String> {
    if output.is_empty() {
        return Err(eyre!("the {who} returned nothing"));
    }
    if output.chars().count() < input.chars().count() / 2 {
        return Err(eyre!(
            "the {who} returned less than half the dictation, so it was kept"
        ));
    }
    if drifted(input, output) {
        return Err(eyre!(
            "the {who} changed too many words, so the dictation was kept"
        ));
    }
    Ok(output.to_string())
}

/// A cue as a person could say it: curly quotes straightened, whitespace
/// collapsed, surrounding quotes and a trailing sentence mark dropped, and
/// lowercased. The script's `normalize`, ported.
fn normalized_cue(text: &str) -> String {
    let collapsed = text
        .replace(['\u{2018}', '\u{2019}'], "'")
        .replace(['\u{201c}', '\u{201d}'], "\"")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    collapsed
        .trim()
        .trim_start_matches(['"', '\''])
        .trim_end_matches(['"', '\''])
        .trim_end_matches(['.', '!', '?', ';', ':'])
        .trim()
        .to_lowercase()
}

/// The text saved for a cue that is the whole utterance, if `table` has one.
/// Entries whose cue starts with `_` are notes, as in `snippets.json`.
fn snippet_in(table: &HashMap<String, String>, text: &str) -> Option<String> {
    let cue = normalized_cue(text);
    if cue.is_empty() {
        return None;
    }
    let find = |want: &str| {
        table
            .iter()
            .filter(|(name, _)| !name.starts_with('_'))
            .find(|(name, _)| normalized_cue(name) == want)
            .map(|(_, saved)| saved.clone())
    };
    if let Some(saved) = find(&cue) {
        return Some(saved);
    }
    // "Insert my email" and "paste my email" ask for the cue out loud.
    for trigger in ["insert ", "paste ", "add "] {
        if let Some(rest) = cue.strip_prefix(trigger)
            && let Some(saved) = find(rest)
        {
            return Some(saved);
        }
    }
    None
}

/// The cue table from the transformation workspace, when it loads.
fn snippets() -> Option<HashMap<String, String>> {
    let workspace = crate::app_paths::personal_commands_workspace().ok()?;
    let bytes = std::fs::read(workspace.join("snippets.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// What a snippet cue says, or None for ordinary speech.
fn snippet_text(text: &str) -> Option<String> {
    snippet_in(&snippets()?, text)
}

/// The words in `text`, lowercased, with punctuation treated as a separator.
fn words(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character == '\'' {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// A small model sometimes paraphrases. Removing filler and false starts is
/// the job, so words may disappear; inventing words is not, so additions are
/// the thing to police. The thresholds are the script's.
fn drifted(before: &str, after: &str) -> bool {
    let input = words(before);
    let output = words(after);
    if output.is_empty() {
        return true;
    }
    let missing = input.difference(&output).count();
    let added = output.difference(&input).count();
    let added_limit = 3.max((input.len() as f64 * 0.08).floor() as usize);
    let missing_limit = 8.max((input.len() as f64 * 0.3).floor() as usize);
    added > added_limit || missing > missing_limit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_keeps_the_measured_wording() {
        assert!(SYSTEM_PROMPT.contains("You clean up dictated speech. You are not an editor."));
        assert!(SYSTEM_PROMPT.contains("Paragraphs: break where the topic shifts"));
        assert!(SYSTEM_PROMPT.contains("Reply with the corrected text only"));
    }

    #[test]
    fn a_short_dictation_costs_no_call() {
        let source = ModelSource {
            source: crate::app_settings::AiSource::Remote,
            ..ModelSource::default()
        };
        // A remote source with nothing set would fail, so an Ok here proves the
        // length gate ran before the request was built.
        assert_eq!(
            clean(&source, 9, DEFAULT_TIMEOUT, "", "too short").unwrap(),
            "too short"
        );
    }

    #[test]
    fn a_local_dictation_that_cannot_reach_its_server_reports_why() {
        let error = clean(
            &ModelSource::default(),
            9,
            DEFAULT_TIMEOUT,
            "",
            "a dictation long enough to send",
        )
        .unwrap_err();
        assert!(error.to_string().contains("the local model did not answer"));
    }

    #[test]
    fn a_remote_dictation_without_an_endpoint_reports_why() {
        let source = ModelSource {
            source: crate::app_settings::AiSource::Remote,
            ..ModelSource::default()
        };
        let error = clean(
            &source,
            9,
            DEFAULT_TIMEOUT,
            "",
            "a dictation long enough to send",
        )
        .unwrap_err();
        assert!(error.to_string().contains("endpoint address is empty"));
    }

    #[test]
    fn cleanup_uses_the_saved_prompt_and_falls_back_when_empty() {
        assert_eq!(system_instruction("Fix the words."), "Fix the words.");
        assert_eq!(system_instruction("  Fix the words.  "), "Fix the words.");
        assert_eq!(system_instruction(""), SYSTEM_PROMPT);
        assert_eq!(system_instruction("   "), SYSTEM_PROMPT);
    }

    #[test]
    fn an_empty_reply_is_refused() {
        let error = accept("some words here", "", "local model").unwrap_err();
        assert_eq!(error.to_string(), "the local model returned nothing");
    }

    #[test]
    fn a_reply_under_half_the_input_is_refused() {
        let error = accept(
            "a long dictation with plenty of words in it",
            "short",
            "model",
        )
        .unwrap_err();
        assert!(error.to_string().contains("less than half"), "{error}");
    }

    #[test]
    fn a_cleanup_that_only_removes_filler_is_kept() {
        let input = "So I wanted to, um, just make something that works today";
        let output = "So I wanted to just make something that works today.";
        assert_eq!(accept(input, output, "local model").unwrap(), output);
    }

    #[test]
    fn a_reply_that_invents_many_words_is_refused() {
        let input = "one two three four five six seven eight nine ten";
        let output = "one two three four five six seven eight nine ten eleven twelve \
                      thirteen fourteen fifteen sixteen";
        let error = accept(input, output, "local model").unwrap_err();
        assert!(
            error.to_string().contains("changed too many words"),
            "{error}"
        );
    }

    #[test]
    fn a_reply_that_drops_most_of_the_words_is_refused() {
        let input = "one two three four five six seven eight nine ten eleven twelve \
                     thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty";
        let output = "one two three four five six seven eight nine ten";
        assert!(drifted(input, output));
    }

    #[test]
    fn a_snippet_cue_is_answered_before_the_model() {
        let table: HashMap<String, String> = [
            ("my email".to_string(), "a@example.com".to_string()),
            (
                "link to github".to_string(),
                "https://github.com/x".to_string(),
            ),
            ("_comment".to_string(), "ignored".to_string()),
        ]
        .into_iter()
        .collect();
        for spoken in [
            "my email",
            "My email.",
            "\"my email\"",
            "insert my email",
            "Paste my email.",
            "  my   email  ",
        ] {
            assert_eq!(
                snippet_in(&table, spoken).as_deref(),
                Some("a@example.com"),
                "{spoken}"
            );
        }
        assert_eq!(snippet_in(&table, "email me the link"), None);
        assert_eq!(snippet_in(&table, "_comment"), None);
    }

    #[test]
    fn a_remote_request_carries_the_session_and_the_client_name() {
        let source = ModelSource {
            source: crate::app_settings::AiSource::Remote,
            remote: crate::app_settings::RemoteModel {
                endpoint: "https://opencode.ai/inference/go/openai/v1/chat/completions".into(),
                model: "mimo-v2.6-flash".into(),
                ..crate::app_settings::RemoteModel::default()
            },
        };
        let headers = crate::dictation::rewrite::remote_headers(&source.remote);
        let name = |want: &str| {
            headers
                .iter()
                .find(|(name, _)| *name == want)
                .map(|(_, value)| value.clone())
        };
        assert!(
            name("x-opencode-session").is_some_and(|id| id.starts_with("hex-")),
            "{headers:?}"
        );
        assert!(name("user-agent").is_some_and(|agent| agent.starts_with("hex/")));
        assert_eq!(name("authorization"), None);
    }

    #[test]
    fn a_key_becomes_a_bearer_header_and_the_session_stays_the_same() {
        let source = ModelSource {
            source: crate::app_settings::AiSource::Remote,
            remote: crate::app_settings::RemoteModel {
                endpoint: "https://api.example.com/v1".into(),
                model: "m".into(),
                api_key: " secret ".into(),
            },
        };
        let headers = crate::dictation::rewrite::remote_headers(&source.remote);
        assert!(
            headers
                .iter()
                .any(|(name, value)| *name == "authorization" && value == "Bearer secret")
        );
        assert_eq!(
            crate::dictation::rewrite::session_id(),
            crate::dictation::rewrite::session_id()
        );
    }

    #[test]
    fn a_request_without_a_snippet_is_ordinary_speech() {
        let table: HashMap<String, String> = [("_note".to_string(), "x".to_string())]
            .into_iter()
            .collect();
        assert_eq!(snippet_in(&table, "link to github"), None);
        assert_eq!(snippet_in(&table, ""), None);
    }

    #[test]
    fn the_request_carries_the_model_only_when_remote() {
        let local = Request {
            model: None,
            messages: Vec::new(),
            temperature: 0.0,
            max_tokens: MAX_TOKENS,
            stream: false,
        };
        assert!(serde_json::to_value(&local).unwrap().get("model").is_none());
        let remote = Request {
            model: Some("big-model".into()),
            ..local
        };
        assert_eq!(serde_json::to_value(&remote).unwrap()["model"], "big-model");
    }
}
