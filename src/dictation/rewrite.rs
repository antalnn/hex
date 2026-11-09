//! The rewrite call for the rewrite hotkey: one POST to the remote endpoint
//! the `rewrite` settings name.
//!
//! Rewrite selection always calls a remote OpenAI-compatible endpoint, through
//! the machine's proxy and trusted roots, with the configured bearer token
//! when the endpoint wants one. The local llama.cpp server serves the dictation
//! cleanup instead; a small local model follows no spoken rewrite instruction
//! (measured 2026-11-08).
//!
//! A failure is an error, not the original text. The caller pastes nothing in
//! that case, which leaves the selection exactly as it was; pasting the
//! original back over it would strip the app's own formatting for no gain. The
//! transformation scripts keep their own "return the input" behaviour because a
//! dictation has to produce something.

use std::time::Duration;

use color_eyre::eyre::{Result, eyre};
use serde::{Deserialize, Serialize};

use crate::app_settings::{AppSettings, RemoteModel};

/// How long one attempt may take. The same 30 s the plan fixed; a rewrite of a
/// selection is one pass, so a server this slow is not going to answer.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Transport failures get one more attempt: the server may be between a
/// restart and its first answer.
const RETRIES: usize = 1;
/// The line that frames the selected text in the user message.
const SENTINEL: &str = "-----";
/// The reply budget for a remote endpoint. It has to cover reasoning as well as
/// the text: a reasoning model such as Muse Spark spends its first 1,500 tokens
/// thinking and returns nothing when that is the whole budget (measured
/// 2026-11-08, 1,498 of 1,500 on reasoning with an empty reply).
const REMOTE_MAX_TOKENS: u32 = 4_096;
/// The snippet match answers with one name, but a reasoning model still spends
/// tokens thinking first, as it does for a rewrite.
const MATCH_MAX_TOKENS: u32 = 1_024;
/// The match runs ahead of the rewrite, so it gets a shorter deadline than the
/// rewrite itself: a slow match is worth less than the instruction as spoken.
const MATCH_TIMEOUT: Duration = Duration::from_secs(12);
/// A recording shorter than this is a fragment, not an instruction the model
/// can classify.
const MATCH_MIN_WORDS: usize = 3;
/// A recording longer than this carries detail a saved snippet would discard.
/// The history probe measured 5 false picks over 10 rows, every one of them on
/// a passage longer than this; the gate leaves those to the instruction as
/// spoken.
const MATCH_MAX_WORDS: usize = 24;
/// The verbs a rewrite instruction opens with. The probe measured the model
/// picking a snippet for a statement ("I tested the rewrite selection…") that
/// asks for nothing; a recording that does not open with one of these stays the
/// instruction it is.
const REWRITE_OPENERS: &[&str] = &[
    "make",
    "turn",
    "polish",
    "tighten",
    "clean",
    "rewrite",
    "reword",
    "shorten",
    "summarize",
    "summarise",
    "fix",
    "improve",
    "change",
    "convert",
    "reformat",
    "restructure",
    "format",
    "add",
    "cut",
    "trim",
    "reduce",
    "clarify",
    "simplify",
    "proofread",
    "edit",
    "expand",
    "elaborate",
];
/// The system prompt for the snippet match: a classification, not an edit.
const MATCH_PROMPT: &str = "You match a spoken instruction to one saved snippet. \
The snippets you may choose are listed by name. Reply with exactly one name from \
the list, copied as written, or the word NONE when no snippet fits. Reply with \
nothing else: no punctuation, no explanation, and no quotes.";

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
    /// The remote endpoint's model name; a rewrite always names one.
    model: String,
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
pub(crate) fn remote_headers(remote: &RemoteModel) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        ("x-opencode-session", session_id().to_string()),
        (
            "user-agent",
            concat!("hex/", env!("CARGO_PKG_VERSION")).to_string(),
        ),
    ];
    let key = remote.api_key.trim();
    if !key.is_empty() {
        headers.push(("authorization", format!("Bearer {key}")));
    }
    headers
}

/// What a rewrite job reads: the remote endpoint, the snippets and the default
/// prompt. The listener shares one with the output worker, which outlives a
/// settings save, so a change is applied here rather than captured at spawn.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RewriteRuntime {
    pub(crate) model: RemoteModel,
    pub(crate) snippets: Vec<(String, String)>,
    pub(crate) default_prompt: String,
}

impl RewriteRuntime {
    pub(crate) fn from_settings(settings: &AppSettings) -> Self {
        Self {
            model: settings.rewrite.remote.clone(),
            snippets: settings.rewrite.pairs(),
            default_prompt: settings.rewrite.default_prompt.clone(),
        }
    }

    /// Takes the saved settings as they are now, so the next rewrite uses
    /// them without a restart.
    pub(crate) fn apply(&mut self, settings: &AppSettings) {
        self.model = settings.rewrite.remote.clone();
        self.snippets = settings.rewrite.pairs();
        self.default_prompt = settings.rewrite.default_prompt.clone();
    }

    /// The snippet match for one instruction, gated by the Labs switch and
    /// the runtime's own snippets and endpoint.
    pub(crate) fn layer_two(&self, instruction: String) -> String {
        layer_two(
            instruction,
            &self.snippets,
            &self.model,
            crate::labs::rewrite_snippet_match(),
        )
    }
}

/// Rewrites `selection` according to `instruction`, through the remote
/// endpoint `remote` names.
pub(crate) fn rewrite(remote: &RemoteModel, instruction: &str, selection: &str) -> Result<String> {
    let output = chat(
        remote,
        messages(instruction, selection),
        REMOTE_MAX_TOKENS,
        TIMEOUT,
    )?;
    checked(&output, instruction)
}

/// One POST to the remote endpoint, returning the reply's text. The caller
/// decides what a reply means.
fn chat(
    remote: &RemoteModel,
    messages: Vec<Message>,
    max_tokens: u32,
    timeout: Duration,
) -> Result<String> {
    if let Some(missing) = remote.missing() {
        return Err(eyre!("{missing}"));
    }
    let url = remote.chat_url();
    let body = Request {
        model: remote.model.trim().to_string(),
        messages,
        temperature: 0.0,
        max_tokens,
        stream: false,
    };
    let agent = crate::net::agent(&url, |settings| settings.timeout_global(Some(timeout)));
    let json = serde_json::to_string(&body)
        .map_err(|error| eyre!("the model request could not be built: {error}"))?;

    let mut last: Option<ureq::Error> = None;
    for attempt in 0..=RETRIES {
        let mut request = agent.post(&url).header("content-type", "application/json");
        for (name, value) in remote_headers(remote) {
            request = request.header(name, &value);
        }
        match request.send(&json) {
            Ok(mut response) => {
                let text = response
                    .body_mut()
                    .read_to_string()
                    .map_err(|error| eyre!("the model's reply could not be read: {error}"))?;
                let parsed: Response = serde_json::from_str(&text)
                    .map_err(|error| eyre!("the model's reply was not JSON: {error}"))?;
                return Ok(parsed
                    .choices
                    .first()
                    .map(|choice| choice.message.content.clone())
                    .unwrap_or_default());
            }
            Err(error) => {
                last = Some(error);
                if attempt < RETRIES {
                    tracing::debug!(attempt, "the model did not answer; retrying once");
                }
            }
        }
    }
    let reason = last.map_or_else(
        || "no reply".to_string(),
        |error| crate::net::reason(&error),
    );
    Err(eyre!("the model did not answer: {reason}"))
}

/// Keeps a reply only when it is a rewrite. An empty reply and a reply that is
/// the instruction itself are both what a small model does when it misreads the
/// frame, and pasting either over the selection would destroy it. A leading
/// line that only repeats the instruction is dropped first: a model sometimes
/// writes it before the text, and it is not part of the rewrite.
fn accept(output: &str, instruction: &str) -> Result<String> {
    let output = strip_leading_instruction(output.trim(), instruction).trim();
    if output.is_empty() {
        return Err(eyre!("the model returned nothing"));
    }
    // The frame is a line of its own. A Markdown table separator such as
    // `|-------|----------|` contains the dashes but is not the frame.
    if output.lines().any(|line| line.trim() == SENTINEL) {
        let mut excerpt: String = output.split_whitespace().collect::<Vec<_>>().join(" ");
        if excerpt.chars().count() > 40 {
            excerpt = excerpt.chars().take(40).collect::<String>();
            excerpt.push_str(" ...");
        }
        return Err(eyre!(
            "the model repeated the \"{SENTINEL}\" frame around the selection instead of the rewritten text; reply began \"{excerpt}\""
        ));
    }
    if normalized(output) == normalized(instruction) {
        return Err(eyre!(
            "the model repeated the instruction instead of rewriting the text"
        ));
    }
    Ok(output.to_string())
}

/// Checks a reply and logs the raw text when a guard rejects it. Without the
/// raw text in the log a rejection only says which guard fired, and the prompt
/// cannot be judged from that.
fn checked(output: &str, instruction: &str) -> Result<String> {
    accept(output, instruction).inspect_err(|error| {
        tracing::warn!(
            reply = %output,
            %error,
            "the model's reply was rejected"
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
        // A default prompt that names a snippet is that snippet's cue, the
        // same as if it had been spoken.
        if let Some(prompt) = snippet_prompt(fallback, snippets) {
            return Ok(prompt);
        }
        return Ok(fallback.to_string());
    }
    Err(eyre!(
        "there was no instruction and no default prompt, so there is nothing to do"
    ))
}

/// Runs the snippet match when it can do anything: the lab is on, snippets
/// exist, the remote is configured, and `resolve` has not already picked one.
/// The instruction comes back unchanged on every skip, a NONE, or a failed
/// call, so a model that cannot classify costs only its latency.
pub(crate) fn layer_two(
    instruction: String,
    snippets: &[(String, String)],
    remote: &RemoteModel,
    enabled: bool,
) -> String {
    if !needs_mapping(&instruction, snippets, remote, enabled) {
        return instruction;
    }
    match match_snippet(&instruction, snippets, remote) {
        Ok(Some(prompt)) => {
            tracing::info!(instruction = %instruction, "layer two matched a saved snippet");
            prompt
        }
        Ok(None) => instruction,
        Err(error) => {
            tracing::warn!(%error, "the snippet match failed; keeping the instruction");
            instruction
        }
    }
}

/// Whether the match call has anything to decide.
fn needs_mapping(
    instruction: &str,
    snippets: &[(String, String)],
    remote: &RemoteModel,
    enabled: bool,
) -> bool {
    enabled
        && (MATCH_MIN_WORDS..=MATCH_MAX_WORDS).contains(&instruction.split_whitespace().count())
        && asks_for_a_rewrite(instruction)
        && !snippets.is_empty()
        && remote.missing().is_none()
        && !already_resolved(instruction, snippets)
}

/// Whether the instruction opens with a rewrite request, after the same
/// courtesy words [`cue`] drops. "I want you to…" counts; a statement about
/// the text does not.
fn asks_for_a_rewrite(instruction: &str) -> bool {
    let spoken = cue(instruction);
    let spoken = spoken
        .strip_prefix("i want you to ")
        .or_else(|| spoken.strip_prefix("i would like you to "))
        .or_else(|| spoken.strip_prefix("id like you to "))
        .unwrap_or(&spoken);
    spoken
        .split(' ')
        .next()
        .is_some_and(|first| REWRITE_OPENERS.contains(&first))
}

/// `resolve` already picked a snippet: its saved prompt is the instruction, or
/// the instruction still names a snippet that the deterministic layer handles.
fn already_resolved(instruction: &str, snippets: &[(String, String)]) -> bool {
    let instruction = instruction.trim();
    snippets
        .iter()
        .any(|(_, prompt)| prompt.trim() == instruction)
        || snippet_prompt(instruction, snippets).is_some()
}

/// One classification call to the remote endpoint; the reply is accepted only
/// when it names a saved snippet after [`cue`] normalization.
fn match_snippet(
    instruction: &str,
    snippets: &[(String, String)],
    remote: &RemoteModel,
) -> Result<Option<String>> {
    let names = snippets
        .iter()
        .map(|(name, _)| format!("- {name}"))
        .collect::<Vec<_>>()
        .join("\n");
    let messages = vec![
        Message {
            role: "system",
            content: MATCH_PROMPT.to_string(),
        },
        Message {
            role: "user",
            content: format!("Instruction: {}\n\nSnippets:\n{names}", instruction.trim()),
        },
    ];
    let reply = chat(remote, messages, MATCH_MAX_TOKENS, MATCH_TIMEOUT)?;
    Ok(match_reply(&reply, snippets))
}

/// The prompt a match reply names, or nothing. Only a reply that is a saved
/// name after case, quotes and punctuation are ignored counts; NONE, an
/// unknown name and anything wordier leave the instruction as spoken.
fn match_reply(reply: &str, snippets: &[(String, String)]) -> Option<String> {
    let spoken = cue(reply);
    if spoken.is_empty() || spoken == "none" {
        return None;
    }
    snippets
        .iter()
        .find(|(name, _)| cue(name) == spoken)
        .map(|(_, prompt)| prompt.clone())
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
        let error = accept("   \n", "make this shorter").unwrap_err();
        assert_eq!(error.to_string(), "the model returned nothing");
    }

    #[test]
    fn a_reply_that_is_only_the_instruction_is_refused() {
        let error = accept("Make this shorter.", "make this shorter").unwrap_err();
        assert_eq!(error.to_string(), "the model returned nothing");
    }

    #[test]
    fn a_reply_that_echoes_the_frame_is_refused() {
        let echoed = format!("make it shorter\n\n{SENTINEL}\noriginal\n{SENTINEL}");
        let error = accept(&echoed, "make it shorter").unwrap_err();
        assert!(
            error.to_string().contains("frame around the selection"),
            "{error}"
        );
    }

    #[test]
    fn a_markdown_table_separator_is_not_the_frame() {
        let output = "Goal\n\n| Item | Result |\n|-------|----------|\n| one | two |";
        assert_eq!(accept(output, "turn this into a prompt").unwrap(), output);
    }

    #[test]
    fn a_real_rewrite_is_returned_trimmed() {
        assert_eq!(
            accept("  Short and clear.\n", "make this shorter").unwrap(),
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
            accept(output, "make this shorter").unwrap(),
            "The text is now shorter."
        );
    }

    #[test]
    fn a_reply_that_only_repeats_the_frame_is_refused() {
        let error = accept(SENTINEL, "tighten it").unwrap_err();
        assert_eq!(
            error.to_string(),
            "the model repeated the \"-----\" frame around the selection instead of the rewritten text; reply began \"-----\""
        );
    }

    #[test]
    fn an_ordinary_multiline_rewrite_is_left_alone() {
        let output = "First paragraph.\n\nSecond paragraph.";
        assert_eq!(accept(output, "tighten it").unwrap(), output);
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
            assert!(checked(&echoed, "tighten it").is_err());
        });

        let logged = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        assert!(
            logged.contains("the model's reply was rejected"),
            "{logged}"
        );
        assert!(logged.contains(SENTINEL), "{logged}");
    }

    fn remote(endpoint: &str, model: &str) -> RemoteModel {
        RemoteModel {
            endpoint: endpoint.into(),
            model: model.into(),
            api_key: String::new(),
        }
    }

    #[test]
    fn a_server_that_is_not_listening_is_an_error_naming_the_reason() {
        // Port 9 answers nothing, so the call fails at the transport and the
        // error carries the reason rather than a model reply.
        let error = rewrite(
            &remote("http://127.0.0.1:9/v1", "m"),
            "make this shorter",
            "some text",
        )
        .unwrap_err();
        assert!(
            error.to_string().starts_with("the model did not answer"),
            "{error}"
        );
    }

    #[test]
    fn a_call_without_an_endpoint_is_refused_before_any_call() {
        let error = rewrite(&RemoteModel::default(), "make this shorter", "some text").unwrap_err();
        assert!(
            error.to_string().contains("endpoint address is empty"),
            "{error}"
        );
    }

    #[test]
    fn a_call_without_a_model_name_is_refused_before_any_call() {
        let source = RemoteModel {
            endpoint: "https://api.example.com/v1".into(),
            ..RemoteModel::default()
        };
        let error = rewrite(&source, "make this shorter", "some text").unwrap_err();
        assert!(error.to_string().contains("model name is empty"), "{error}");
    }

    #[test]
    fn the_request_names_the_remote_model_and_keeps_the_endpoint() {
        let source = remote("https://api.example.com/v1", "big-model");
        let body = Request {
            model: source.model.trim().to_string(),
            messages: messages("tighten it", "the text"),
            temperature: 0.0,
            max_tokens: REMOTE_MAX_TOKENS,
            stream: false,
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], "big-model");
        assert_eq!(
            source.chat_url(),
            "https://api.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn a_remote_request_carries_the_session_and_the_client_name() {
        let headers = remote_headers(&remote("https://api.example.com/v1", "m"));
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
    fn a_key_becomes_a_bearer_header() {
        let source = RemoteModel {
            api_key: " secret ".into(),
            ..remote("https://api.example.com/v1", "m")
        };
        let headers = remote_headers(&source);
        assert!(
            headers
                .iter()
                .any(|(name, value)| *name == "authorization" && value == "Bearer secret")
        );
        assert_eq!(session_id(), session_id());
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

    /// The output worker outlives one settings save; the listener replaces the
    /// shared runtime in place so the next rewrite uses the saved endpoint.
    #[test]
    fn a_saved_change_replaces_the_runtime_the_worker_reads() {
        use std::sync::{Arc, RwLock};

        let mut settings = AppSettings::default();
        settings.rewrite.remote.endpoint = "https://one.example/v1".into();
        settings.rewrite.remote.model = "small".into();
        let rewrite = Arc::new(RwLock::new(RewriteRuntime::from_settings(&settings)));
        let worker_rewrite = Arc::clone(&rewrite);

        settings.rewrite.remote.endpoint = "https://two.example/v1".into();
        settings.rewrite.remote.model = "big".into();
        settings.rewrite.default_prompt = "Tighten it.".into();
        rewrite
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .apply(&settings);

        let seen = worker_rewrite
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        assert_eq!(seen.model.endpoint, "https://two.example/v1");
        assert_eq!(seen.model.model, "big");
        assert_eq!(seen.default_prompt, "Tighten it.");
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

    #[test]
    fn a_default_prompt_that_names_a_snippet_uses_its_instruction() {
        assert_eq!(
            resolve("", &spoken_snippets(), "polish").unwrap(),
            "Polish the wording."
        );
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

    #[test]
    fn a_match_reply_naming_a_saved_snippet_yields_its_prompt() {
        for reply in [
            "structured prompt",
            "Structured Prompt.",
            "\"structured prompt\"",
        ] {
            assert_eq!(
                match_reply(reply, &snippets()).as_deref(),
                Some("Turn it into a prompt."),
                "{reply}"
            );
        }
    }

    #[test]
    fn a_match_reply_naming_no_saved_snippet_is_ignored() {
        for reply in [
            "an unknown name",
            "the structured prompt",
            "Sure: structured prompt",
            "NONE",
            " none. ",
            "",
        ] {
            assert_eq!(match_reply(reply, &snippets()), None, "{reply}");
        }
    }

    #[test]
    fn the_match_call_is_skipped_while_the_lab_is_off() {
        assert!(!needs_mapping(
            "make this shorter",
            &snippets(),
            &remote("https://api.example.com/v1", "m"),
            false,
        ));
    }

    #[test]
    fn the_match_call_is_skipped_without_saved_snippets() {
        assert!(!needs_mapping(
            "make this shorter",
            &[],
            &remote("https://api.example.com/v1", "m"),
            true,
        ));
    }

    #[test]
    fn the_match_call_is_skipped_when_resolve_already_picked_a_snippet() {
        assert!(!needs_mapping(
            "Turn it into a prompt.",
            &snippets(),
            &remote("https://api.example.com/v1", "m"),
            true,
        ));
        assert!(!needs_mapping(
            "use the structured prompt",
            &snippets(),
            &remote("https://api.example.com/v1", "m"),
            true,
        ));
    }

    #[test]
    fn the_match_call_is_skipped_while_the_remote_is_unconfigured() {
        assert!(!needs_mapping(
            "make this shorter",
            &snippets(),
            &RemoteModel::default(),
            true,
        ));
    }

    #[test]
    fn a_vague_instruction_is_worth_matching() {
        assert!(needs_mapping(
            "make this shorter and clearer",
            &snippets(),
            &remote("https://api.example.com/v1", "m"),
            true,
        ));
    }

    #[test]
    fn the_match_call_is_skipped_for_a_fragment_or_a_passage() {
        assert!(!needs_mapping(
            "There is.",
            &snippets(),
            &remote("https://api.example.com/v1", "m"),
            true,
        ));
        let passage = "word ".repeat(MATCH_MAX_WORDS + 1);
        assert!(!needs_mapping(
            passage.trim(),
            &snippets(),
            &remote("https://api.example.com/v1", "m"),
            true,
        ));
    }

    #[test]
    fn the_match_call_reads_only_an_instruction_that_opens_with_a_request() {
        assert!(!needs_mapping(
            "Alright, I tested the rewrite selection, and it seems that something went wrong in it.",
            &snippets(),
            &remote("https://api.example.com/v1", "m"),
            true,
        ));
        for instruction in [
            "turn this into a prompt for an agent",
            "I want you to make this shorter",
            "can you polish the wording here",
        ] {
            assert!(
                needs_mapping(
                    instruction,
                    &snippets(),
                    &remote("https://api.example.com/v1", "m"),
                    true
                ),
                "{instruction}"
            );
        }
    }
}
