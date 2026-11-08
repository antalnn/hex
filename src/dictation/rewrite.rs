//! The rewrite call for the rewrite hotkey: one POST to the llama.cpp server
//! Hex owns, normally launched by `crate::local_model`.
//!
//! Local only. The agent is [`crate::net::loopback_agent`], so every name,
//! redirect included, must resolve to this computer: nothing off the machine
//! can receive the selection.
//!
//! A failure is an error, not the original text. The caller pastes nothing in
//! that case, which leaves the selection exactly as it was; pasting the
//! original back over it would strip the app's own formatting for no gain. The
//! transformation scripts keep their own "return the input" behaviour because a
//! dictation has to produce something.

use std::time::Duration;

use color_eyre::eyre::{Result, eyre};
use serde::{Deserialize, Serialize};

/// How long one attempt may take. The same 30 s the plan fixed; a rewrite of a
/// selection is one pass, so a server this slow is not going to answer.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Transport failures get one more attempt: the server may be between a
/// restart and its first answer.
const RETRIES: usize = 1;
/// The line that frames the selected text in the user message.
const SENTINEL: &str = "-----";
const MAX_TOKENS: u32 = 1_500;

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

/// Rewrites `selection` according to `instruction`, through the local server
/// on `port`.
pub(crate) fn rewrite(port: u16, instruction: &str, selection: &str) -> Result<String> {
    let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
    let body = Request {
        messages: messages(instruction, selection),
        temperature: 0.0,
        max_tokens: MAX_TOKENS,
        stream: false,
    };
    let agent = crate::net::loopback_agent(|settings| settings.timeout_global(Some(TIMEOUT)));
    let json = serde_json::to_string(&body)
        .map_err(|error| eyre!("the rewrite request could not be built: {error}"))?;

    let mut last: Option<ureq::Error> = None;
    for attempt in 0..=RETRIES {
        match agent
            .post(&url)
            .header("content-type", "application/json")
            .send(&json)
        {
            Ok(mut response) => {
                let text = response
                    .body_mut()
                    .read_to_string()
                    .map_err(|error| eyre!("the local model's reply could not be read: {error}"))?;
                let parsed: Response = serde_json::from_str(&text)
                    .map_err(|error| eyre!("the local model's reply was not JSON: {error}"))?;
                let output = parsed
                    .choices
                    .first()
                    .map(|choice| choice.message.content.clone())
                    .unwrap_or_default();
                return accept(&output, instruction);
            }
            Err(error) => {
                last = Some(error);
                if attempt < RETRIES {
                    tracing::debug!(attempt, "the local model did not answer; retrying once");
                }
            }
        }
    }
    let reason = last.map_or_else(|| "no reply".to_string(), |error| crate::net::reason(&error));
    Err(eyre!("the local model did not answer: {reason}"))
}

/// Keeps a reply only when it is a rewrite. An empty reply and a reply that is
/// the instruction itself are both what a small model does when it misreads the
/// frame, and pasting either over the selection would destroy it.
fn accept(output: &str, instruction: &str) -> Result<String> {
    let output = output.trim();
    if output.is_empty() {
        return Err(eyre!("the local model returned nothing"));
    }
    if output.contains(SENTINEL) {
        return Err(eyre!(
            "the local model echoed the message frame instead of rewriting the text"
        ));
    }
    if normalized(output) == normalized(instruction) {
        return Err(eyre!(
            "the local model repeated the instruction instead of rewriting the text"
        ));
    }
    Ok(output.to_string())
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
/// punctuation and one leading "use"/"insert"/"paste" all ignored.
fn cue(text: &str) -> String {
    let mut words: Vec<String> = normalized(text)
        .split(' ')
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect();
    if matches!(
        words.first().map(String::as_str),
        Some("use" | "insert" | "paste")
    ) {
        words.remove(0);
    }
    words.join(" ")
}

/// The prompt a whole-utterance snippet name picks, if the recording was only
/// that name: "structured prompt" uses its saved prompt, "make this a
/// structured prompt" does not.
fn snippet_prompt(transcript: &str, snippets: &[(String, String)]) -> Option<String> {
    let spoken = cue(transcript);
    if spoken.is_empty() {
        return None;
    }
    snippets
        .iter()
        .find(|(name, _)| cue(name) == spoken)
        .map(|(_, prompt)| prompt.clone())
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
        let expected = format!("make this shorter\n\n{SENTINEL}\nthe original sentence\n{SENTINEL}");
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
        let error = accept("   \n", "make this shorter").unwrap_err();
        assert_eq!(error.to_string(), "the local model returned nothing");
    }

    #[test]
    fn a_reply_that_repeats_the_instruction_is_refused() {
        let error = accept("Make this shorter.", "make this shorter").unwrap_err();
        assert!(
            error.to_string().contains("repeated the instruction"),
            "{error}"
        );
    }

    #[test]
    fn a_reply_that_echoes_the_frame_is_refused() {
        let echoed = format!("make it shorter\n\n{SENTINEL}\noriginal\n{SENTINEL}");
        let error = accept(&echoed, "make it shorter").unwrap_err();
        assert!(
            error.to_string().contains("echoed the message frame"),
            "{error}"
        );
    }

    #[test]
    fn a_real_rewrite_is_returned_trimmed() {
        assert_eq!(
            accept("  Short and clear.\n", "make this shorter").unwrap(),
            "Short and clear."
        );
    }

    #[test]
    fn a_server_that_is_not_listening_is_an_error_naming_the_reason() {
        // Port 9 answers nothing, so the call fails at the transport and the
        // error carries the reason rather than a model reply.
        let error = rewrite(9, "make this shorter", "some text").unwrap_err();
        assert!(
            error.to_string().starts_with("the local model did not answer"),
            "{error}"
        );
    }

    fn snippets() -> Vec<(String, String)> {
        vec![
            ("structured prompt".to_string(), "Turn it into a prompt.".to_string()),
            ("bullet list".to_string(), "Turn it into bullets.".to_string()),
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
}
