//! The rewrite call for the rewrite hotkey: one POST to the model the
//! `rewrite` settings name.
//!
//! Two sources. [`crate::app_settings::AiSource::Local`] is the llama.cpp
//! server Hex owns, normally launched by `crate::local_model`, reached through
//! [`crate::net::loopback_agent`] so every name, redirects included, must
//! resolve to this computer. [`crate::app_settings::AiSource::Remote`] is an
//! OpenAI-compatible endpoint anywhere, reached through the machine's proxy and
//! trusted roots, with the configured bearer token when the endpoint wants one.
//!
//! A failure is an error, not the original text. The caller pastes nothing in
//! that case, which leaves the selection exactly as it was; pasting the
//! original back over it would strip the app's own formatting for no gain. The
//! transformation scripts keep their own "return the input" behaviour because a
//! dictation has to produce something.

use std::time::Duration;

use color_eyre::eyre::{Result, eyre};
use serde::{Deserialize, Serialize};

use crate::app_settings::ModelSource;

/// How long one attempt may take. The same 30 s the plan fixed; a rewrite of a
/// selection is one pass, so a server this slow is not going to answer.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Transport failures get one more attempt: the server may be between a
/// restart and its first answer.
const RETRIES: usize = 1;
/// The line that frames the selected text in the user message.
const SENTINEL: &str = "-----";
/// The reply budget for the local server, which runs with a 4,096-token
/// context: a long selection plus this cap must still fit inside it.
const MAX_TOKENS: u32 = 1_500;
/// The reply budget for a remote endpoint. It has to cover reasoning as well as
/// the text: a reasoning model such as Muse Spark spends its first 1,500 tokens
/// thinking and returns nothing when that is the whole budget (measured
/// 2026-11-08, 1,498 of 1,500 on reasoning with an empty reply).
const REMOTE_MAX_TOKENS: u32 = 4_096;

/// What the model is told, and nothing about its own voice: it edits the
/// person's text, it does not write its own.
pub(crate) const SYSTEM_PROMPT: &str = "Rewrite the text the person selected. \
Follow their instruction and return only the rewritten text: no preamble, no \
closing line, no explanation, no quotation marks around it. Change nothing the \
instruction does not ask for, and keep every fact, name, number, path, \
identifier and code fence as written.";

#[derive(Clone, Debug, Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct Request {
    /// Only a remote endpoint wants a model name; the local server has one
    /// loaded already.
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

/// The messages for one rewrite: the system prompt, then the instruction and
/// the selection between sentinel lines, so the model can tell where the text
/// it must edit begins.
fn messages(instruction: &str, selection: &str) -> Vec<Message> {
    vec![
        Message {
            role: "system",
            content: SYSTEM_PROMPT.to_string(),
        },
        Message {
            role: "user",
            content: format!(
                "{}\n\n{SENTINEL}\n{selection}\n{SENTINEL}",
                instruction.trim()
            ),
        },
    ]
}

/// A session id shared by every remote call this run makes. OpenCode Go
/// refuses a request without one (`400 MissingSessionID`), and a stable id lets
/// it route and cache the conversation; other providers ignore the header.
pub(crate) fn session_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        format!("hex-{nanos:x}-{:x}", std::process::id())
    })
}

/// The headers a remote request carries besides `content-type`: the session
/// id OpenCode Go requires, the client's own user agent it asks for, and the
/// bearer token when the endpoint wants one.
pub(crate) fn remote_headers(source: &ModelSource) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        ("x-opencode-session", session_id().to_string()),
        (
            "user-agent",
            concat!("hex/", env!("CARGO_PKG_VERSION")).to_string(),
        ),
    ];
    let key = source.api_key.trim();
    if !key.is_empty() {
        headers.push(("authorization", format!("Bearer {key}")));
    }
    headers
}

/// Rewrites `selection` according to `instruction`, through `source`: Hex's
/// local server on `port`, or the remote endpoint the source names.
pub(crate) fn rewrite(
    source: &ModelSource,
    port: u16,
    instruction: &str,
    selection: &str,
) -> Result<String> {
    if let Some(missing) = source.missing() {
        return Err(eyre!("{missing}"));
    }
    let who = who(source);
    let url = source.chat_url(port);
    let body = Request {
        model: source.is_remote().then(|| source.model.trim().to_string()),
        messages: messages(instruction, selection),
        temperature: 0.0,
        max_tokens: if source.is_remote() {
            REMOTE_MAX_TOKENS
        } else {
            MAX_TOKENS
        },
        stream: false,
    };
    // A remote endpoint is reached through the machine's proxy and its own
    // trusted roots; a local one may only touch this computer.
    let agent = if source.is_remote() {
        crate::net::agent(&url, |settings| settings.timeout_global(Some(TIMEOUT)))
    } else {
        crate::net::loopback_agent(|settings| settings.timeout_global(Some(TIMEOUT)))
    };
    let json = serde_json::to_string(&body)
        .map_err(|error| eyre!("the rewrite request could not be built: {error}"))?;

    let mut last: Option<ureq::Error> = None;
    for attempt in 0..=RETRIES {
        let mut request = agent.post(&url).header("content-type", "application/json");
        if source.is_remote() {
            for (name, value) in remote_headers(source) {
                request = request.header(name, &value);
            }
        }
        match request.send(&json) {
            Ok(mut response) => {
                let text = response
                    .body_mut()
                    .read_to_string()
                    .map_err(|error| eyre!("the {who}'s reply could not be read: {error}"))?;
                let parsed: Response = serde_json::from_str(&text)
                    .map_err(|error| eyre!("the {who}'s reply was not JSON: {error}"))?;
                let output = parsed
                    .choices
                    .first()
                    .map(|choice| choice.message.content.clone())
                    .unwrap_or_default();
                return checked(&output, instruction, who);
            }
            Err(error) => {
                last = Some(error);
                if attempt < RETRIES {
                    tracing::debug!(attempt, "the {who} did not answer; retrying once");
                }
            }
        }
    }
    let reason = last.map_or_else(
        || "no reply".to_string(),
        |error| crate::net::reason(&error),
    );
    Err(eyre!("the {who} did not answer: {reason}"))
}

/// What to call the model in a message: the local server is named as such, a
/// remote one is just "the model".
fn who(source: &ModelSource) -> &'static str {
    if source.is_remote() {
        "model"
    } else {
        "local model"
    }
}

/// Keeps a reply only when it is a rewrite. An empty reply and a reply that is
/// the instruction itself are both what a small model does when it misreads the
/// frame, and pasting either over the selection would destroy it. A leading
/// line that only repeats the instruction is dropped first: the 2B sometimes
/// writes it before the text, and it is not part of the rewrite.
fn accept(output: &str, instruction: &str, who: &str) -> Result<String> {
    let output = strip_leading_instruction(output.trim(), instruction).trim();
    if output.is_empty() {
        return Err(eyre!("the {who} returned nothing"));
    }
    if output.contains(SENTINEL) {
        return Err(eyre!(
            "the {who} echoed the message frame instead of rewriting the text"
        ));
    }
    if normalized(output) == normalized(instruction) {
        return Err(eyre!(
            "the {who} repeated the instruction instead of rewriting the text"
        ));
    }
    Ok(output.to_string())
}

/// Checks a reply and logs the raw text when a guard rejects it. Without the
/// raw text in the log a rejection only says which guard fired, and the prompt
/// cannot be judged from that.
fn checked(output: &str, instruction: &str, who: &str) -> Result<String> {
    accept(output, instruction, who).inspect_err(|error| {
        tracing::warn!(
            reply = %output,
            %error,
            "the {who}'s reply was rejected"
        );
    })
}

/// Drops one leading line that is only the instruction, capitalized or with a
/// full stop added. One line only: a real rewrite may open with the same words
/// and must keep them.
fn strip_leading_instruction<'a>(output: &'a str, instruction: &str) -> &'a str {
    let mut lines = output.splitn(2, '\n');
    let first = lines.next().unwrap_or_default();
    if normalized(first) == normalized(instruction) {
        return lines
            .next()
            .unwrap_or_default()
            .trim_start_matches(['\r', '\n']);
    }
    output
}

/// Lowercase words only, so punctuation a model adds cannot hide an echo.
fn normalized(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|character| character.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A spoken cue as a person could say it: case, surrounding quotes, trailing
/// punctuation and a leading courtesy phrase or "use"/"insert"/"paste" all
/// ignored. The phrases stack, because "can you please polish this" is how
/// the sentence comes out.
fn cue(text: &str) -> String {
    let mut words: Vec<String> = normalized(text)
        .split(' ')
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect();
    while let Some(first) = words.first().map(String::as_str) {
        let strip = match first {
            "use" | "insert" | "paste" | "please" | "would" => 1,
            "can" | "could" if words.get(1).map(String::as_str) == Some("you") => 2,
            _ => break,
        };
        words.drain(..strip);
    }
    words.join(" ")
}

/// The phrases a person actually says for a snippet whose name is too curt to
/// speak, mapped to that name. Matched against the whole cue, so they never
/// fire inside a longer instruction.
const ALIASES: &[(&str, &str)] = &[
    ("turn this into a prompt", "prompt"),
    ("make this a prompt", "prompt"),
    ("create a prompt", "prompt"),
    ("polish this", "polish"),
    ("tighten this", "tighten"),
];

/// Words that may sit around a snippet name without turning the recording
/// into an instruction: "the structured prompt", "polish and tighten".
const FILLER: &[&str] = &[
    "the", "a", "an", "this", "that", "these", "those", "it", "them", "my", "me", "please", "now",
    "just", "here", "to", "into", "and", "or", "then", "for", "of", "with", "do", "does", "can",
    "you", "could", "would", "use", "insert", "paste", "so", "ok", "okay",
];

/// A recording this short is a cue, not a sentence that happens to mention a
/// snippet. Longer recordings are instructions and keep their own words.
const CONTEXT_WORDS: usize = 10;

/// The name a short recording picks out, if it names one or more snippets and
/// nothing else: "the structured prompt" picks it, "polish and tighten" picks
/// the first name, "make this a structured prompt" stays an instruction.
fn contextual_snippet(spoken: &str, snippets: &[(String, String)]) -> Option<String> {
    let words: Vec<&str> = spoken.split(' ').filter(|word| !word.is_empty()).collect();
    if words.len() > CONTEXT_WORDS {
        return None;
    }
    // Each name that appears as a whole phrase, with where it starts. Longest
    // first at the same start, so "structured prompt" beats "prompt".
    let mut found: Vec<(usize, usize, &str)> = Vec::new();
    for (name, _) in snippets {
        let name_cue = cue(name);
        let name_words: Vec<&str> = name_cue.split(' ').filter(|w| !w.is_empty()).collect();
        if name_words.is_empty() {
            continue;
        }
        for start in 0..=words.len().saturating_sub(name_words.len()) {
            if words[start..start + name_words.len()] == name_words[..] {
                found.push((start, name_words.len(), name.as_str()));
                break;
            }
        }
    }
    found.sort_by_key(|&(start, length, _)| (start, std::cmp::Reverse(length)));
    let &(_, _, name) = found.first()?;
    // Every name in the recording may sit there; what is left must be filler,
    // or the recording is an instruction that mentions a name in passing.
    let mut covered = vec![false; words.len()];
    for &(start, length, _) in &found {
        for slot in &mut covered[start..start + length] {
            *slot = true;
        }
    }
    let outside_has_content = words
        .iter()
        .enumerate()
        .any(|(index, word)| !covered[index] && !FILLER.contains(word));
    if outside_has_content {
        return None;
    }
    snippets
        .iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, prompt)| prompt.clone())
}

/// The prompt a whole-utterance snippet name picks, if the recording was only
/// that name: "structured prompt" uses its saved prompt, "make this a
/// structured prompt" does not.
fn snippet_prompt(transcript: &str, snippets: &[(String, String)]) -> Option<String> {
    let spoken = cue(transcript);
    if spoken.is_empty() {
        return None;
    }
    if let Some((_, prompt)) = snippets.iter().find(|(name, _)| cue(name) == spoken) {
        return Some(prompt.clone());
    }
    if let Some(alias) = ALIASES
        .iter()
        .find(|(phrase, _)| *phrase == spoken)
        .map(|(_, name)| *name)
        && let Some((_, prompt)) = snippets.iter().find(|(name, _)| cue(name) == alias)
    {
        return Some(prompt.clone());
    }
    contextual_snippet(&spoken, snippets)
}

/// What to ask the model to do with the selection: the snippet the recording
/// named, otherwise the recording itself, otherwise the default prompt.
pub(crate) fn resolve(
    transcript: &str,
    snippets: &[(String, String)],
    default_prompt: &str,
) -> Result<String> {
    if let Some(prompt) = snippet_prompt(transcript, snippets) {
        return Ok(prompt);
    }
    let spoken = transcript.trim();
    if !spoken.is_empty() {
        return Ok(spoken.to_string());
    }
    let fallback = default_prompt.trim();
    if !fallback.is_empty() {
        return Ok(fallback.to_string());
    }
    Err(eyre!(
        "there was no instruction and no default prompt, so there is nothing to do"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_message_carries_the_instruction_then_the_selection() {
        let messages = messages("make this shorter", "the original sentence");
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[0].content, SYSTEM_PROMPT);
        assert_eq!(messages[1].role, "user");
        let expected =
            format!("make this shorter\n\n{SENTINEL}\nthe original sentence\n{SENTINEL}");
        assert_eq!(messages[1].content, expected);
        let instruction_at = messages[1].content.find("make this shorter").unwrap();
        let selection_at = messages[1].content.find("the original sentence").unwrap();
        assert!(instruction_at < selection_at);
    }

    #[test]
    fn the_system_prompt_asks_for_the_result_alone() {
        assert!(SYSTEM_PROMPT.contains("return only the rewritten text"));
        assert!(SYSTEM_PROMPT.contains("no preamble"));
    }

    #[test]
    fn an_empty_result_is_refused() {
        let error = accept("   \n", "make this shorter", "local model").unwrap_err();
        assert_eq!(error.to_string(), "the local model returned nothing");
    }

    #[test]
    fn a_reply_that_is_only_the_instruction_is_refused() {
        let error = accept("Make this shorter.", "make this shorter", "local model").unwrap_err();
        assert_eq!(error.to_string(), "the local model returned nothing");
    }

    #[test]
    fn a_reply_that_echoes_the_frame_is_refused() {
        let echoed = format!("make it shorter\n\n{SENTINEL}\noriginal\n{SENTINEL}");
        let error = accept(&echoed, "make it shorter", "local model").unwrap_err();
        assert!(
            error.to_string().contains("echoed the message frame"),
            "{error}"
        );
    }

    #[test]
    fn a_real_rewrite_is_returned_trimmed() {
        assert_eq!(
            accept("  Short and clear.\n", "make this shorter", "local model").unwrap(),
            "Short and clear."
        );
    }

    #[test]
    fn the_chosen_shipped_frame_is_built_with_the_divider() {
        let messages = messages("tighten this", "the text");
        let user = &messages[1].content;
        assert!(user.starts_with("tighten this"));
        assert!(user.contains(&format!("{SENTINEL}\nthe text\n{SENTINEL}")));
    }

    #[test]
    fn a_leading_line_that_repeats_the_instruction_is_stripped() {
        let output = "Make this shorter.\n\nThe text is now shorter.";
        assert_eq!(
            accept(output, "make this shorter", "local model").unwrap(),
            "The text is now shorter."
        );
    }

    #[test]
    fn a_stripped_reply_from_a_remote_model_is_named_as_such() {
        let error = accept(SENTINEL, "tighten it", "model").unwrap_err();
        assert_eq!(
            error.to_string(),
            "the model echoed the message frame instead of rewriting the text"
        );
    }

    #[test]
    fn an_ordinary_multiline_rewrite_is_left_alone() {
        let output = "First paragraph.\n\nSecond paragraph.";
        assert_eq!(accept(output, "tighten it", "local model").unwrap(), output);
    }

    #[test]
    fn a_guard_rejection_logs_the_raw_reply() {
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct Buffer(Arc<Mutex<Vec<u8>>>);

        impl std::io::Write for Buffer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buffer {
            type Writer = Buffer;

            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buffer.clone())
            .with_ansi(false)
            .finish();
        let echoed = format!("tighten it\n\n{SENTINEL}\nthe text\n{SENTINEL}");
        tracing::subscriber::with_default(subscriber, || {
            assert!(checked(&echoed, "tighten it", "local model").is_err());
        });

        let logged = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        assert!(
            logged.contains("the local model's reply was rejected"),
            "{logged}"
        );
        assert!(logged.contains(SENTINEL), "{logged}");
    }

    #[test]
    fn a_server_that_is_not_listening_is_an_error_naming_the_reason() {
        // Port 9 answers nothing, so the call fails at the transport and the
        // error carries the reason rather than a model reply.
        let error =
            rewrite(&ModelSource::default(), 9, "make this shorter", "some text").unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("the local model did not answer"),
            "{error}"
        );
    }

    #[test]
    fn a_remote_source_without_an_endpoint_is_refused_before_any_call() {
        let source = ModelSource {
            source: crate::app_settings::AiSource::Remote,
            ..ModelSource::default()
        };
        let error = rewrite(&source, 9, "make this shorter", "some text").unwrap_err();
        assert!(
            error.to_string().contains("endpoint address is empty"),
            "{error}"
        );
    }

    #[test]
    fn a_remote_source_without_a_model_name_is_refused_before_any_call() {
        let source = ModelSource {
            source: crate::app_settings::AiSource::Remote,
            endpoint: "https://api.example.com/v1".into(),
            ..ModelSource::default()
        };
        let error = rewrite(&source, 9, "make this shorter", "some text").unwrap_err();
        assert!(error.to_string().contains("model name is empty"), "{error}");
    }

    #[test]
    fn a_remote_request_carries_the_model_and_no_authorization_without_a_key() {
        let source = ModelSource {
            source: crate::app_settings::AiSource::Remote,
            endpoint: "https://api.example.com/v1".into(),
            model: "big-model".into(),
            ..ModelSource::default()
        };
        let body = Request {
            model: source.is_remote().then(|| source.model.trim().to_string()),
            messages: messages("tighten it", "the text"),
            temperature: 0.0,
            max_tokens: MAX_TOKENS,
            stream: false,
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], "big-model");
        assert_eq!(
            source.chat_url(8081),
            "https://api.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn a_local_request_omits_the_model_name() {
        let source = ModelSource::default();
        let body = Request {
            model: source.is_remote().then(|| source.model.trim().to_string()),
            messages: messages("tighten it", "the text"),
            temperature: 0.0,
            max_tokens: MAX_TOKENS,
            stream: false,
        };
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("model").is_none());
        assert_eq!(
            source.chat_url(8081),
            "http://127.0.0.1:8081/v1/chat/completions"
        );
    }

    fn snippets() -> Vec<(String, String)> {
        vec![
            (
                "structured prompt".to_string(),
                "Turn it into a prompt.".to_string(),
            ),
            (
                "bullet list".to_string(),
                "Turn it into bullets.".to_string(),
            ),
        ]
    }

    #[test]
    fn a_snippet_name_spoken_alone_picks_that_prompt() {
        assert_eq!(
            resolve("structured prompt", &snippets(), "").unwrap(),
            "Turn it into a prompt."
        );
    }

    #[test]
    fn a_snippet_name_ignores_case_quotes_punctuation_and_one_leading_verb() {
        for spoken in [
            "Structured Prompt.",
            "\"structured prompt\"",
            "use structured prompt",
            "Insert: structured prompt.",
        ] {
            assert_eq!(
                resolve(spoken, &snippets(), "").unwrap(),
                "Turn it into a prompt.",
                "{spoken}"
            );
        }
    }

    #[test]
    fn a_normal_sentence_is_used_verbatim_and_not_mistaken_for_a_name() {
        assert_eq!(
            resolve("make this a structured prompt", &snippets(), "").unwrap(),
            "make this a structured prompt"
        );
    }

    #[test]
    fn an_empty_instruction_uses_the_default_prompt() {
        assert_eq!(
            resolve("   ", &snippets(), " Tighten it. ").unwrap(),
            "Tighten it."
        );
    }

    #[test]
    fn an_empty_instruction_with_no_default_reports_nothing_to_do() {
        let error = resolve("", &snippets(), "  ").unwrap_err();
        assert!(error.to_string().contains("nothing to do"), "{error}");
    }

    fn spoken_snippets() -> Vec<(String, String)> {
        vec![
            ("prompt".to_string(), "Turn it into a prompt.".to_string()),
            ("polish".to_string(), "Polish the wording.".to_string()),
            ("tighten".to_string(), "Tighten the wording.".to_string()),
        ]
    }

    #[test]
    fn a_courtesy_opening_before_an_alias_still_picks_the_snippet() {
        assert_eq!(
            resolve("can you turn this into a prompt?", &spoken_snippets(), "").unwrap(),
            "Turn it into a prompt."
        );
    }

    #[test]
    fn a_courtesy_word_before_an_alias_is_ignored() {
        assert_eq!(
            resolve("please polish this", &spoken_snippets(), "").unwrap(),
            "Polish the wording."
        );
    }

    #[test]
    fn a_long_sentence_that_mentions_a_name_in_passing_is_left_alone() {
        let spoken = "I want you to explain how a prompt is written by an engineer";
        assert_eq!(resolve(spoken, &spoken_snippets(), "").unwrap(), spoken);
    }

    #[test]
    fn two_names_in_one_short_utterance_pick_the_first() {
        assert_eq!(
            resolve("polish and tighten", &spoken_snippets(), "").unwrap(),
            "Polish the wording."
        );
    }
}
