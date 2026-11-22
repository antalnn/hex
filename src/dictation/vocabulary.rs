//! The shared vocabulary: names Hex should know (people, projects,
//! products), each with spellings a transcript may show instead ("sounds
//! like"). Search uses it to find a term's likely mis-transcriptions,
//! prompts can read it through [`terms`], Whisper and Qwen3-ASR read it as
//! their context prompt through [`transcription_prompt`], and a prompt run
//! replaces its "sounds like" spellings afterwards through
//! [`correct_sounds_like`].

use std::cell::{Cell, RefCell};
use std::io::Read as _;
use std::path::Path;
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::dictation::context::ContextSnapshot;
use crate::dictation::text_replacements::{ReplacementSet, TextReplacement};

pub(crate) mod csv;

/// Longest term or "sounds like" entry, in characters.
pub(crate) const MAX_TERM_CHARS: usize = 64;
/// Most entries in the list, and most "sounds like" spellings per entry.
pub const MAX_ENTRIES: usize = 500;
pub(crate) const MAX_SOUNDS_LIKE: usize = 16;
/// Read limit for settings.json when reading it without loading `AppSettings`.
const SETTINGS_READ_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(from = "SavedEntry")]
pub struct VocabularyEntry {
    pub term: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sounds_like: Vec<String>,
    /// Wrap the term in backticks when it replaces a heard spelling. It only
    /// acts while the processing setting asks for it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub code: bool,
}

/// An entry as `settings.json` holds it: a plain term, or the term with its
/// "sounds like" spellings and its code flag.
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum SavedEntry {
    Term(String),
    Entry {
        term: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        sounds_like: Vec<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        code: bool,
    },
}

impl From<SavedEntry> for VocabularyEntry {
    fn from(saved: SavedEntry) -> Self {
        match saved {
            SavedEntry::Term(term) => Self {
                term,
                sounds_like: Vec::new(),
                code: false,
            },
            SavedEntry::Entry {
                term,
                sounds_like,
                code,
            } => Self {
                term,
                sounds_like,
                code,
            },
        }
    }
}

/// Writes a list for `settings.json`: a term with no "sounds like"
/// spellings as a plain string. Either form loads.
pub(crate) fn serialize_saved<S: serde::Serializer>(
    entries: &[VocabularyEntry],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(entries.iter().map(|entry| {
        if entry.sounds_like.is_empty() && !entry.code {
            SavedEntry::Term(entry.term.clone())
        } else {
            SavedEntry::Entry {
                term: entry.term.clone(),
                sounds_like: entry.sounds_like.clone(),
                code: entry.code,
            }
        }
    }))
}

static SAVED: OnceLock<RwLock<Vec<VocabularyEntry>>> = OnceLock::new();
/// The focused app and page as last seen (macOS), whose mode's own terms a
/// dictation hears with ahead of the shared list.
static FOCUS: OnceLock<RwLock<Option<ContextSnapshot>>> = OnceLock::new();

thread_local! {
    static DICTATING: Cell<bool> = const { Cell::new(false) };
    /// Labs › Words on screen's terms for the capture this thread is
    /// transcribing, set by [`for_capture`]; empty for a job that has none.
    static SCREEN_TERMS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Records the focused app and page, so the next dictation's transcription
/// hears with its mode's vocabulary.
pub(crate) fn focus(context: &ContextSnapshot) {
    *FOCUS
        .get_or_init(Default::default)
        .write()
        .unwrap_or_else(|error| error.into_inner()) = Some(context.clone());
}

/// Runs `work` as a dictation's transcription on this thread: its runs hear
/// with the focused mode's terms too. Meetings, captures, and other hosts'
/// audio hear with the shared list alone.
pub(crate) fn for_dictation<R>(work: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            DICTATING.with(|flag| flag.set(self.0));
        }
    }
    let _restore = Restore(DICTATING.with(|flag| flag.replace(true)));
    work()
}

/// Runs one dictation's inference with the terms Labs › Words on screen
/// read from its window. The terms belong to that run alone: a nested run
/// restores the outside value, and a job with none clears it, so a meeting or
/// a local API request never hears another capture's screen.
pub(crate) fn for_capture<R>(terms: &[String], work: impl FnOnce() -> R) -> R {
    struct Restore(Vec<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SCREEN_TERMS.with(|slot| *slot.borrow_mut() = std::mem::take(&mut self.0));
        }
    }
    let previous =
        SCREEN_TERMS.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), terms.to_vec()));
    let _restore = Restore(previous);
    work()
}

/// The focused mode's own terms first, so they lead the context prompt, then
/// the shared list, then the terms this capture read from its window, each
/// term once. Screen terms are lowest priority because they change with every
/// window, and only a dictation hears them.
fn transcription_entries() -> Vec<VocabularyEntry> {
    let shared = entries();
    if !DICTATING.with(Cell::get) {
        return shared;
    }
    let focused = FOCUS
        .get_or_init(Default::default)
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    #[cfg(target_os = "macos")]
    let shared = match &focused {
        Some(context) => {
            let own = crate::dictation::config::dictation_profiles().vocabulary_for(context);
            merged(&own, &shared)
        }
        None => shared,
    };
    #[cfg(not(target_os = "macos"))]
    let _ = focused;
    let screen = SCREEN_TERMS.with(|terms| screen_entries(&terms.borrow()));
    if screen.is_empty() {
        return shared;
    }
    merged(&shared, &screen)
}

/// Labs › Words on screen's terms as entries with no "sounds like"
/// spellings.
fn screen_entries(terms: &[String]) -> Vec<VocabularyEntry> {
    terms
        .iter()
        .map(|term| VocabularyEntry {
            term: term.clone(),
            sounds_like: Vec::new(),
            code: false,
        })
        .collect()
}

/// `first`, then the entries of `then` whose term `first` lacks, in any case.
pub(crate) fn merged(first: &[VocabularyEntry], then: &[VocabularyEntry]) -> Vec<VocabularyEntry> {
    if first.is_empty() {
        return then.to_vec();
    }
    let mut list = first.to_vec();
    for entry in then {
        if !contains(&list, &entry.term) {
            list.push(entry.clone());
        }
    }
    list
}

/// The saved terms, as last applied from settings.
pub fn terms() -> Vec<String> {
    entries().into_iter().map(|entry| entry.term).collect()
}

/// The saved entries, as last applied from settings.
pub fn entries() -> Vec<VocabularyEntry> {
    SAVED
        .get_or_init(Default::default)
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

/// Whether the selected dictation model would read vocabulary terms at all:
/// Whisper and Qwen3-ASR read them as a prompt; Parakeet, Apple Speech and
/// other models ignore them. Read as a capture becomes real, before reading
/// the screen, and again before giving up its live stream.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn uses_terms() -> bool {
    let (_, selection) = crate::app_settings::transcription_selection();
    let definition = crate::transcription::models::definition(selection.model);
    terms_used(definition.vocabulary_use())
}

fn terms_used(vocabulary_use: crate::transcription::models::VocabularyUse) -> bool {
    use crate::transcription::models::VocabularyUse;
    match vocabulary_use {
        VocabularyUse::Prompt => true,
        VocabularyUse::None => false,
    }
}

/// The whole-word, case-insensitive "sounds like" corrections a prompt-based
/// transcript needs: each listed spelling replaced by its term. `None` when
/// the list holds no usable spelling.
pub(crate) fn sounds_like_corrections() -> Option<SoundsLikeCorrections> {
    corrections_for(&transcription_entries())
}

/// The spelling corrections one run applies, split by where they can match.
pub(crate) struct SoundsLikeCorrections {
    /// Every listed spelling, for the text and its segments.
    phrases: ReplacementSet,
    /// Only single-word spellings, for the timed words: a multi-word spelling
    /// cannot match inside one word.
    words: ReplacementSet,
}

/// Every usable spelling replaced by its term, for entries that list one.
/// Each entry's term is a protection rule, so a spelling inside its own term
/// cannot win the longest match: "next" never turns "Next.js" into
/// "Next.js.js", and "open code" never turns "Open Code Hub" into
/// "Open Code Hub Hub". A protection rule writes back the text it matched, so
/// other occurrences of the term keep their own casing too. These replace text
/// after a run instead of steering its decode.
fn corrections_for(entries: &[VocabularyEntry]) -> Option<SoundsLikeCorrections> {
    let mut phrases = Vec::new();
    let mut words = Vec::new();
    // Every entry's term is protected, even one that lists no spellings, so
    // a spelling from another entry cannot rewrite it or change its casing.
    let protected = entries
        .iter()
        .map(|entry| entry.term.trim())
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>();
    for entry in entries {
        let term = entry.term.trim();
        // An empty term would compile as a deletion rule.
        if term.is_empty() {
            continue;
        }
        let spellings = entry
            .sounds_like
            .iter()
            .filter(|spelling| spelling_is_usable(spelling) && !same(spelling, term))
            .collect::<Vec<_>>();
        if spellings.is_empty() {
            continue;
        }
        for spelling in spellings {
            let replacement = TextReplacement {
                matched_phrase: spelling.clone(),
                output: term.to_string(),
            };
            if !spelling.contains(char::is_whitespace) {
                words.push(replacement.clone());
            }
            phrases.push(replacement);
        }
    }
    (!phrases.is_empty()).then(|| SoundsLikeCorrections {
        phrases: ReplacementSet::new(&phrases).protecting(&protected),
        words: ReplacementSet::new(&words).protecting(&protected),
    })
}

/// Whether a spelling is long enough for the engine's exact-spelling pass: at
/// least two characters, counting ASCII letters and digits and every non-ASCII
/// code point, after dropping ASCII spaces and punctuation. The engine never
/// lowercases, so `İ` counts once; it ignores a shorter spelling, so this does.
fn spelling_is_usable(spelling: &str) -> bool {
    spelling
        .chars()
        .filter(|character| !character.is_ascii() || character.is_ascii_alphanumeric())
        .count()
        >= 2
}

/// Replaces each listed "sounds like" spelling in `transcript` with its term.
/// Called after a whole-clip run whose model read the vocabulary as a prompt
/// (Whisper, Qwen3-ASR). A model with no vocabulary slot never read the list,
/// so it is not corrected here. The text and its segments get every spelling;
/// the timed words get only single-word spellings and keep their times.
pub(crate) fn correct_sounds_like(transcript: &mut transcribe_cpp::Transcript) {
    let Some(corrections) = sounds_like_corrections() else {
        return;
    };
    correct(transcript, &corrections);
}

fn correct(transcript: &mut transcribe_cpp::Transcript, corrections: &SoundsLikeCorrections) {
    transcript.text = corrections.phrases.replace(&transcript.text);
    for segment in &mut transcript.segments {
        segment.text = corrections.phrases.replace(&segment.text);
    }
    for word in &mut transcript.words {
        word.text = corrections.words.replace(&word.text);
    }
}

/// How much of the vocabulary a context prompt carries, in characters, for a
/// model of this architecture; `None` for one without a prompt. Whisper's
/// prompt slot holds 223 tokens and its engine trims an overlong one from
/// the front, dropping the first (most important) terms, so the list stays
/// well inside it. Qwen3-ASR's system message has far more room; the bound
/// keeps a long list from crowding out the audio or being echoed.
pub(crate) fn prompt_chars(architecture: &str) -> Option<usize> {
    match architecture {
        "whisper" => Some(500),
        "qwen3_asr" => Some(1_000),
        _ => None,
    }
}

/// How many of `entries`, first in list order, a model of this
/// architecture reads. The same budget [`transcription_prompt`] applies.
pub(crate) fn terms_read(entries: &[VocabularyEntry], architecture: &str) -> usize {
    prompt_chars(architecture).map_or(0, |max_chars| prompt_terms(entries, max_chars))
}

/// The context prompt Whisper and Qwen3-ASR runs read: the first terms in
/// list order, comma-separated so the echo filter can recognise the list,
/// and as many as fit `max_chars`. The character budget is the prompt's own
/// limit. `None` while the list is empty. Read at the start of each run.
pub(crate) fn transcription_prompt(max_chars: usize) -> Option<String> {
    prompt_for(&transcription_entries(), max_chars)
}

fn prompt_for(entries: &[VocabularyEntry], max_chars: usize) -> Option<String> {
    let terms = &entries[..prompt_terms(entries, max_chars)];
    (!terms.is_empty()).then(|| {
        terms
            .iter()
            .map(|entry| entry.term.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    })
}

/// How many of the first terms fit `max_chars` joined by ", ".
fn prompt_terms(entries: &[VocabularyEntry], max_chars: usize) -> usize {
    let mut chars = 0;
    for (index, entry) in entries.iter().enumerate() {
        let separator = if index == 0 { 0 } else { 2 };
        chars += separator + entry.term.chars().count();
        if chars > max_chars {
            return index;
        }
    }
    entries.len()
}

/// Adds each name in recognition hints saved before they merged into the
/// vocabulary (comma, semicolon, or line separated), skipping names already
/// listed in any case. Returns whether the list changed.
pub(crate) fn fold_hints(list: &mut Vec<VocabularyEntry>, hints: &str) -> bool {
    let before = list.len();
    for hint in hints.split([',', ';', '\n']) {
        add_clamped(list, hint, &[], false);
    }
    list.len() != before
}

/// Called by `AppSettings` whenever settings load or save.
pub(crate) fn apply(entries: &[VocabularyEntry]) {
    *SAVED
        .get_or_init(Default::default)
        .write()
        .unwrap_or_else(|error| error.into_inner()) = entries.to_vec();
}

/// The entries saved in `root/settings.json`, read without loading
/// `AppSettings` (which migrates and writes), for read-only commands. Empty
/// when the file is missing or unreadable.
pub fn read_saved(root: &Path) -> Vec<VocabularyEntry> {
    match read_from(&root.join("settings.json")) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(%error, "could not read the vocabulary; searching without it");
            Vec::new()
        }
    }
}

fn read_from(path: &Path) -> Result<Vec<VocabularyEntry>, String> {
    #[derive(Deserialize)]
    struct Settings {
        #[serde(default)]
        vocabulary: Vec<VocabularyEntry>,
    }
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(SETTINGS_READ_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > SETTINGS_READ_LIMIT {
        return Err("settings file exceeds the read limit".into());
    }
    let settings: Settings = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    Ok(settings.vocabulary)
}

/// Splits a comma-separated "sounds like" field into trimmed spellings.
pub fn parse_sounds_like(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(clean)
        .filter(|spelling| !spelling.is_empty())
        .collect()
}

pub(crate) fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn same(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

/// The corrections the vocabulary implies: every "sounds like" spelling maps
/// to its term. A spelling equal to the term would only rewrite itself, and an
/// entry with no spellings corrects nothing, so both are skipped.
///
/// The output is always the plain term. Backticks for a term marked `code` are
/// added once, after the rewrite, by [`wrap_code_terms`].
pub(crate) fn replacements(
    entries: &[VocabularyEntry],
) -> Vec<super::text_replacements::TextReplacement> {
    let mut pairs = Vec::new();
    for entry in entries {
        let term = entry.term.trim();
        if term.is_empty() {
            continue;
        }
        for spelling in &entry.sounds_like {
            let heard = spelling.trim();
            if heard.is_empty() || same(heard, term) {
                continue;
            }
            pairs.push(super::text_replacements::TextReplacement {
                matched_phrase: heard.to_owned(),
                output: term.to_owned(),
            });
        }
    }
    pairs
}

/// Wraps every standalone occurrence of a term marked `code` in backticks.
///
/// Runs on the finished text, so the model cannot strip the backticks, and it
/// covers a term the transcriber already spelled right, which a correction
/// never touches. Leaves a code fence alone, and the boundary checks keep it
/// out of an occurrence already in backticks, a path, and a longer identifier.
pub(crate) fn wrap_code_terms(text: &str, entries: &[VocabularyEntry]) -> String {
    if text.contains("```") || text.contains("~~~") {
        return text.to_owned();
    }
    let mut terms: Vec<&str> = Vec::new();
    for entry in entries {
        let term = entry.term.trim();
        if entry.code && !term.is_empty() && !terms.contains(&term) {
            terms.push(term);
        }
    }
    // Longest first, so a short term cannot match inside a longer one.
    terms.sort_by_key(|term| std::cmp::Reverse(term.chars().count()));
    let mut output = text.to_owned();
    for term in terms {
        output = wrap_term(&output, term);
    }
    output
}

/// Backticks every standalone `term` in `text`. A boundary is the start or end
/// of the text, or a character that cannot be part of the same word: the
/// checks also catch a term already between backticks and one inside a path.
fn wrap_term(text: &str, term: &str) -> String {
    let Ok(pattern) = regex::Regex::new(&regex::escape(term)) else {
        return text.to_owned();
    };
    let mut output = String::with_capacity(text.len());
    let mut last = 0;
    let mut wrapped = false;
    for found in pattern.find_iter(text) {
        let (start, end) = (found.start(), found.end());
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        let joins_before = |character: Option<char>| {
            character.is_some_and(|character| {
                character.is_alphanumeric() || matches!(character, '_' | '`' | '\\' | '/')
            })
        };
        let joins_after = |character: Option<char>| {
            character.is_some_and(|character| {
                character.is_alphanumeric() || matches!(character, '_' | '`')
            })
        };
        if joins_before(before) || joins_after(after) {
            continue;
        }
        output.push_str(&text[last..start]);
        output.push('`');
        output.push_str(&text[start..end]);
        output.push('`');
        last = end;
        wrapped = true;
    }
    if !wrapped {
        return text.to_owned();
    }
    output.push_str(&text[last..]);
    output
}

/// Adds `term`, or adds new `sounds_like` spellings and the code flag to an
/// existing entry with the same term in any case. Returns whether the list
/// changed.
pub fn add(
    list: &mut Vec<VocabularyEntry>,
    term: &str,
    sounds_like: &[String],
    code: bool,
) -> Result<bool, String> {
    let term = clean(term);
    if term.is_empty() {
        return Err("a vocabulary term can't be empty".into());
    }
    let mut spellings: Vec<String> = Vec::new();
    for spelling in sounds_like.iter().map(|spelling| clean(spelling)) {
        if spelling.is_empty()
            || same(&spelling, &term)
            || spellings.iter().any(|kept| same(kept, &spelling))
        {
            continue;
        }
        spellings.push(spelling);
    }
    if let Some(long) = std::iter::once(&term)
        .chain(&spellings)
        .find(|text| text.chars().count() > MAX_TERM_CHARS)
    {
        return Err(format!(
            "“{long}” is longer than {MAX_TERM_CHARS} characters"
        ));
    }
    let existing = list.iter().position(|entry| same(&entry.term, &term));
    if existing.is_none() && list.len() >= MAX_ENTRIES {
        return Err(format!("the vocabulary holds at most {MAX_ENTRIES} terms"));
    }
    let mut merged = existing
        .map(|index| list[index].sounds_like.clone())
        .unwrap_or_default();
    let before = merged.len();
    for spelling in spellings {
        if !merged.iter().any(|kept| same(kept, &spelling)) {
            merged.push(spelling);
        }
    }
    if merged.len() > MAX_SOUNDS_LIKE {
        return Err(format!(
            "a term takes at most {MAX_SOUNDS_LIKE} “sounds like” spellings"
        ));
    }
    match existing {
        Some(index) => {
            let code_changed = code && !list[index].code;
            list[index].code |= code;
            list[index].sounds_like = merged;
            Ok(list[index].sounds_like.len() != before || code_changed)
        }
        None => {
            list.push(VocabularyEntry {
                term,
                sounds_like: merged,
                code,
            });
            Ok(true)
        }
    }
}

/// Like [`add`], but keeps what fits instead of refusing: a long term is cut
/// to its first 64 characters, and a long spelling or one past the 16th is
/// left out. A full list still takes no new term. Returns whether anything
/// was cut or left out.
pub fn add_clamped(
    list: &mut Vec<VocabularyEntry>,
    term: &str,
    sounds_like: &[String],
    code: bool,
) -> bool {
    let mut term = clean(term);
    let mut clamped = false;
    if term.chars().count() > MAX_TERM_CHARS {
        term = clean(&term.chars().take(MAX_TERM_CHARS).collect::<String>());
        clamped = true;
    }
    if term.is_empty() {
        return clamped;
    }
    if add(list, &term, &[], code).is_err() {
        return true;
    }
    for spelling in sounds_like {
        clamped |= add(list, &term, std::slice::from_ref(spelling), code).is_err();
    }
    clamped
}

/// Whether the list has an entry for `term`, in any case.
pub fn contains(list: &[VocabularyEntry], term: &str) -> bool {
    let term = clean(term);
    list.iter().any(|entry| same(&entry.term, &term))
}

/// Removes the entry for `term`, in any case. Returns whether it was there.
pub fn remove(list: &mut Vec<VocabularyEntry>, term: &str) -> bool {
    let term = clean(term);
    let before = list.len();
    list.retain(|entry| !same(&entry.term, &term));
    list.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(term: &str, sounds_like: &[&str]) -> VocabularyEntry {
        VocabularyEntry {
            term: term.into(),
            sounds_like: sounds_like.iter().map(|&text| text.into()).collect(),
            code: false,
        }
    }

    #[test]
    fn plain_terms_load_and_save_as_strings() {
        #[derive(Deserialize, Serialize)]
        struct File {
            #[serde(serialize_with = "serialize_saved")]
            vocabulary: Vec<VocabularyEntry>,
        }
        let file: File = serde_json::from_str(
            r#"{"vocabulary": ["Rune", {"term": "OpenCode", "sounds_like": ["open code"]},
                {"term": "Kaffar"}]}"#,
        )
        .unwrap();
        assert_eq!(
            file.vocabulary,
            [
                entry("Rune", &[]),
                entry("OpenCode", &["open code"]),
                entry("Kaffar", &[])
            ]
        );
        assert_eq!(
            serde_json::to_string(&file).unwrap(),
            r#"{"vocabulary":["Rune",{"term":"OpenCode","sounds_like":["open code"]},"Kaffar"]}"#
        );
        // Replies to `hex vocabulary` keep the object form.
        assert_eq!(
            serde_json::to_string(&entry("Rune", &[])).unwrap(),
            r#"{"term":"Rune"}"#
        );
    }

    #[test]
    fn a_modes_terms_come_first_and_each_term_once() {
        let own = [entry("useEffect", &[]), entry("Rune", &["rooney"])];
        let shared = [entry("rune", &[]), entry("OpenCode", &[])];
        assert_eq!(
            merged(&own, &shared),
            [
                entry("useEffect", &[]),
                entry("Rune", &["rooney"]),
                entry("OpenCode", &[])
            ]
        );
        assert_eq!(merged(&[], &shared), shared);
    }

    #[test]
    fn only_dictation_hears_with_the_focused_modes_terms() {
        apply(&[entry("OpenCode", &[])]);
        assert_eq!(transcription_entries(), [entry("OpenCode", &[])]);
        let inside = for_dictation(|| {
            assert!(DICTATING.with(Cell::get));
            transcription_entries()
        });
        assert!(inside.iter().any(|entry| entry.term == "OpenCode"));
        assert!(!DICTATING.with(Cell::get));
    }

    #[test]
    fn a_captures_screen_terms_come_last_and_leave_with_it() {
        apply(&[entry("OpenCode", &[])]);
        let terms = vec!["opencode".to_string(), "Qdrant".to_string()];
        let inside = for_dictation(|| for_capture(&terms, transcription_entries));
        assert_eq!(inside, [entry("OpenCode", &[]), entry("Qdrant", &[])]);
        // A later job, such as a local API request or a meeting, hears none.
        assert_eq!(
            for_dictation(transcription_entries),
            [entry("OpenCode", &[])]
        );
    }

    #[test]
    fn only_prompt_models_read_terms() {
        use crate::transcription::models::VocabularyUse;
        assert!(terms_used(VocabularyUse::Prompt));
        assert!(!terms_used(VocabularyUse::None));
    }

    #[test]
    fn adding_dedupes_by_case_and_merges_spellings() {
        let mut list = Vec::new();
        assert!(
            add(
                &mut list,
                "  Cappuccino ",
                &["capuchin".into(), "".into()],
                false
            )
            .unwrap()
        );
        assert_eq!(list, [entry("Cappuccino", &["capuchin"])]);
        assert!(!add(&mut list, "cappuccino", &["Capuchin".into()], false).unwrap());
        assert!(add(&mut list, "CAPPUCCINO", &["cap a chino".into()], false).unwrap());
        assert_eq!(list, [entry("Cappuccino", &["capuchin", "cap a chino"])]);
        // A spelling identical to its term adds nothing.
        assert!(!add(&mut list, "Cappuccino", &["cappuccino".into()], false).unwrap());
        assert!(add(&mut list, "   ", &[], false).is_err());
        assert!(add(&mut list, &"x".repeat(65), &[], false).is_err());
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn clamped_adds_keep_the_term_and_the_spellings_that_fit() {
        let spellings: Vec<String> = (0..20).map(|index| format!("spelling {index}")).collect();
        let mut list = Vec::new();
        let mut with_long = spellings[..2].to_vec();
        with_long.push("y".repeat(65));
        assert!(add_clamped(&mut list, "Cappuccino", &with_long, false));
        assert_eq!(list, [entry("Cappuccino", &["spelling 0", "spelling 1"])]);
        assert!(add_clamped(&mut list, "cappuccino", &spellings, false));
        assert_eq!(list[0].sounds_like, spellings[..MAX_SOUNDS_LIKE]);

        assert!(add_clamped(
            &mut list,
            &"x".repeat(70),
            &["ex".into()],
            false
        ));
        assert_eq!(list[1], entry(&"x".repeat(MAX_TERM_CHARS), &["ex"]));
        assert!(!add_clamped(&mut list, "Rune", &["roon".into()], false));
        assert!(!add_clamped(&mut list, "  ", &[], false));
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn removing_ignores_case() {
        let mut list = vec![entry("OpenCode", &[]), entry("Rune", &[])];
        assert!(remove(&mut list, "opencode"));
        assert!(!remove(&mut list, "opencode"));
        assert_eq!(list, [entry("Rune", &[])]);
    }

    #[test]
    fn sounds_like_spellings_become_their_terms_after_transcription() {
        let entries = [
            entry("GitHub", &["JetHub", "гитхаб"]),
            entry("OpenCode", &["open code"]),
        ];
        let corrections = corrections_for(&entries).unwrap();
        assert_eq!(
            corrections.phrases.replace("push to JetHub"),
            "push to GitHub"
        );
        assert_eq!(corrections.phrases.replace("jethub"), "GitHub");
        assert_eq!(
            corrections.phrases.replace("JETHUB, jEtHuB."),
            "GitHub, GitHub."
        );
        assert_eq!(
            corrections.phrases.replace("запушить в гитхаб"),
            "запушить в GitHub"
        );
        assert_eq!(corrections.phrases.replace("open code"), "OpenCode");
        // Only a whole word: a longer word keeps its spelling, and so does a
        // spelling split across two words that is not listed.
        assert_eq!(corrections.phrases.replace("JetHubber"), "JetHubber");
        assert_eq!(corrections.phrases.replace("Jet Hub"), "Jet Hub");
    }

    #[test]
    fn a_spelling_inside_its_own_term_keeps_the_term() {
        // The term is a longer match than the spelling inside it.
        let next = corrections_for(&[entry("Next.js", &["next"])]).unwrap();
        assert_eq!(next.phrases.replace("Next.js"), "Next.js");
        assert_eq!(next.phrases.replace("next.js"), "next.js");
        assert_eq!(
            next.phrases.replace("deploy Next.js now"),
            "deploy Next.js now"
        );
        assert_eq!(
            next.phrases.replace("wait for next week"),
            "wait for Next.js week"
        );

        let hub = corrections_for(&[entry("Open Code Hub", &["open code"])]).unwrap();
        assert_eq!(hub.phrases.replace("Open Code Hub"), "Open Code Hub");
        assert_eq!(hub.phrases.replace("open code"), "Open Code Hub");
    }

    fn corrected_text(entries: &[VocabularyEntry], text: &str) -> String {
        corrections_for(entries).map_or_else(
            || text.to_owned(),
            |corrections| corrections.phrases.replace(text),
        )
    }

    #[test]
    fn a_term_without_spellings_still_protects_itself() {
        let corrections =
            corrections_for(&[entry("Next.js", &[]), entry("Nest", &["next"])]).unwrap();
        assert_eq!(corrections.phrases.replace("next.js"), "next.js");
        assert_eq!(corrections.phrases.replace("Next.js"), "Next.js");
        assert_eq!(
            corrections.phrases.replace("wait for next"),
            "wait for Nest"
        );
    }

    #[test]
    fn an_entry_without_a_term_is_skipped() {
        assert!(corrections_for(&[entry("  ", &["next"])]).is_none());
        let corrections =
            corrections_for(&[entry("", &["next"]), entry("Rust", &["rost"])]).unwrap();
        assert_eq!(
            corrections.phrases.replace("wait for next"),
            "wait for next"
        );
        assert_eq!(corrections.phrases.replace("push to rost"), "push to Rust");
    }

    #[test]
    fn a_term_keeps_its_casing_outside_its_spellings() {
        let corrections = corrections_for(&[entry("Rust", &["rost"])]).unwrap();
        assert_eq!(
            corrections.phrases.replace("rust on the car"),
            "rust on the car"
        );
        assert_eq!(
            corrections.phrases.replace("RUST on the car"),
            "RUST on the car"
        );
        assert_eq!(corrections.phrases.replace("push to rost"), "push to Rust");
    }

    #[test]
    fn a_timed_word_keeps_its_casing_and_takes_a_spelling() {
        let corrections = corrections_for(&[entry("Rust", &["rost"])]).unwrap();
        let mut transcript = timed_transcript("rust or rost", &["rust", "or", "rost"]);
        correct(&mut transcript, &corrections);
        assert_eq!(timed_words(&transcript), ["rust", "or", "Rust"]);
    }

    #[test]
    fn a_one_letter_spelling_is_ignored() {
        // The engine ignores a spelling under two letters, counted after
        // dropping ASCII spaces and punctuation, so "I" for "AI" leaves
        // "I think" alone.
        assert_eq!(corrected_text(&[entry("AI", &["I"])], "I think"), "I think");
        assert_eq!(
            corrected_text(&[entry("AI", &["a."])], "a. test"),
            "a. test"
        );
        assert!(!spelling_is_usable("Ж"), "a non-ASCII letter counts once");
        assert!(
            !spelling_is_usable("İ"),
            "the engine never lowercases, so İ stays one character"
        );
        assert!(spelling_is_usable("Жж"));
        assert_eq!(
            corrections_for(&[entry("AI", &["a.b"])])
                .unwrap()
                .phrases
                .replace("a.b test"),
            "AI test"
        );
    }

    #[test]
    fn sounds_like_corrections_skip_the_terms_own_spelling() {
        let entries = [
            entry("GitHub", &["jethub", "GitHub", "JETHUB"]),
            entry("Rune", &[]),
        ];
        let corrected = corrections_for(&entries).unwrap();
        assert_eq!(
            corrected.phrases.replace("GitHub is jethub"),
            "GitHub is GitHub"
        );
        assert_eq!(corrected_text(&entries, "push to jethub"), "push to GitHub");
        assert!(corrections_for(&[entry("Rune", &[])]).is_none());
        // An entry whose only spelling restates its term adds nothing, so no
        // identity rule survives on its own.
        assert!(corrections_for(&[entry("Next.js", &["next.js"])]).is_none());
    }

    fn timed_transcript(text: &str, words: &[&str]) -> transcribe_cpp::Transcript {
        transcribe_cpp::Transcript {
            text: text.into(),
            segments: vec![transcribe_cpp::Segment {
                text: text.into(),
                ..Default::default()
            }],
            words: words
                .iter()
                .map(|text| transcribe_cpp::Word {
                    text: (*text).into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn timed_words(transcript: &transcribe_cpp::Transcript) -> Vec<&str> {
        transcript
            .words
            .iter()
            .map(|word| word.text.as_str())
            .collect()
    }

    #[test]
    fn a_transcripts_text_segments_and_words_are_corrected() {
        let replacement = || TextReplacement {
            matched_phrase: "JetHub".into(),
            output: "GitHub".into(),
        };
        let corrections = SoundsLikeCorrections {
            phrases: ReplacementSet::new(&[replacement()]),
            words: ReplacementSet::new(&[replacement()]),
        };
        let mut transcript = timed_transcript("push to JetHub.", &["push", "to", "JetHub."]);
        correct(&mut transcript, &corrections);
        assert_eq!(transcript.text, "push to GitHub.");
        assert_eq!(transcript.segments[0].text, "push to GitHub.");
        assert_eq!(timed_words(&transcript), ["push", "to", "GitHub."]);
    }

    #[test]
    fn a_multi_word_spelling_corrects_text_and_segments_but_not_words() {
        let corrections = corrections_for(&[entry("OpenCode", &["open code"])]).unwrap();
        let mut transcript = timed_transcript("use open code now", &["use", "open", "code", "now"]);
        correct(&mut transcript, &corrections);
        assert_eq!(transcript.text, "use OpenCode now");
        assert_eq!(transcript.segments[0].text, "use OpenCode now");
        // A multi-word spelling cannot match inside one timed word.
        assert_eq!(timed_words(&transcript), ["use", "open", "code", "now"]);
    }

    #[test]
    fn the_prompt_keeps_the_first_terms_that_fit() {
        let list = vec![
            entry("Kaffar", &["Caffer"]),
            entry("Rodolfo", &[]),
            entry("Bragelonne", &[]),
        ];
        assert_eq!(
            prompt_for(&list, 100).as_deref(),
            Some("Kaffar, Rodolfo, Bragelonne")
        );
        assert_eq!(prompt_for(&list, 20).as_deref(), Some("Kaffar, Rodolfo"));
        assert_eq!(prompt_for(&list, 3), None);
        assert_eq!(prompt_for(&[], 100), None);
        let many: Vec<_> = (0..100).map(|n| entry(&format!("T{n}"), &[])).collect();
        let prompt = prompt_for(&many, 10_000).unwrap();
        assert_eq!(prompt.split(", ").count(), 100);
        let prompt = prompt_for(&many, 1_000).unwrap();
        assert!(prompt.split(", ").count() > 64);
        assert!(prompt.chars().count() <= 1_000);
    }

    #[test]
    fn terms_read_counts_exactly_what_transcription_takes() {
        let many: Vec<_> = (0..200)
            .map(|n| entry(&format!("Term number {n}"), &[]))
            .collect();
        for count in [0, 1, 20, 40, 64, 65, 200] {
            let list = &many[..count];
            for architecture in ["whisper", "qwen3_asr"] {
                let prompt = prompt_for(list, prompt_chars(architecture).unwrap());
                let taken = prompt.map_or(0, |prompt| prompt.split(", ").count());
                assert_eq!(
                    terms_read(list, architecture),
                    taken,
                    "{architecture} {count}"
                );
            }
            assert_eq!(terms_read(list, "parakeet"), 0);
            assert_eq!(terms_read(list, "sense_voice"), 0);
        }
        assert_eq!(terms_read(&many, "parakeet"), 0);
        assert!(terms_read(&many, "whisper") < terms_read(&many, "qwen3_asr"));
        assert!(terms_read(&many, "qwen3_asr") < many.len());
    }

    #[test]
    fn hints_fold_into_the_list_without_duplicates() {
        let mut list = vec![entry("OpenCode", &["open code"])];
        assert!(fold_hints(&mut list, "opencode, Effect;Rune\n  ,Effect"));
        assert_eq!(
            list,
            [
                entry("OpenCode", &["open code"]),
                entry("Effect", &[]),
                entry("Rune", &[]),
            ]
        );
        assert!(!fold_hints(&mut list, "Rune, effect"));
        assert!(!fold_hints(&mut list, "  "));
    }

    #[test]
    fn sounds_like_splits_on_commas() {
        assert_eq!(
            parse_sounds_like(" capuchin,, cap  a chino ,"),
            ["capuchin", "cap a chino"]
        );
        assert!(parse_sounds_like(" , ").is_empty());
    }

    #[test]
    fn saved_entries_read_without_loading_settings() {
        let directory = std::env::temp_dir().join(format!("hex-vocabulary-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        assert_eq!(read_from(&path).unwrap(), []);
        std::fs::write(&path, r#"{"sound_effects": false}"#).unwrap();
        assert_eq!(read_from(&path).unwrap(), []);
        std::fs::write(
            &path,
            r#"{"vocabulary": [{"term": "Rune"}, {"term": "Cappuccino", "sounds_like": ["capuchin"]}]}"#,
        )
        .unwrap();
        assert_eq!(
            read_from(&path).unwrap(),
            [entry("Rune", &[]), entry("Cappuccino", &["capuchin"])]
        );
        std::fs::write(&path, "{").unwrap();
        assert!(read_from(&path).is_err());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn spellings_become_replacements() {
        let entries = [
            entry(
                "cappuccino",
                &["capuchin", "cap a chino", "Cappuccino", "  "],
            ),
            entry("Rune", &[]),
        ];
        let pairs = replacements(&entries);
        assert_eq!(pairs.len(), 2, "only real, different spellings survive");
        assert_eq!(pairs[0].matched_phrase, "capuchin");
        assert_eq!(pairs[0].output, "cappuccino");
        assert_eq!(pairs[1].matched_phrase, "cap a chino");
        assert_eq!(pairs[1].output, "cappuccino");
    }

    fn code(term: &str) -> VocabularyEntry {
        VocabularyEntry {
            term: term.into(),
            sounds_like: Vec::new(),
            code: true,
        }
    }

    #[test]
    fn a_correction_never_adds_backticks() {
        let entries = [VocabularyEntry {
            term: "useState".into(),
            sounds_like: vec!["use state".into()],
            code: true,
        }];
        assert_eq!(replacements(&entries)[0].output, "useState");
    }

    #[test]
    fn a_code_term_wraps_where_it_stands() {
        let entries = [code(".gitignore")];
        assert_eq!(
            wrap_code_terms("edit the .gitignore file", &entries),
            "edit the `.gitignore` file"
        );
        // Already wrapped: the backtick on either side blocks a second pass.
        assert_eq!(
            wrap_code_terms("edit the `.gitignore` file", &entries),
            "edit the `.gitignore` file"
        );
        // A path is left alone.
        assert_eq!(
            wrap_code_terms(r"C:\project\.gitignore", &entries),
            r"C:\project\.gitignore"
        );
        // At the very start and the very end.
        assert_eq!(wrap_code_terms(".gitignore", &entries), "`.gitignore`");
        assert_eq!(
            wrap_code_terms("write .gitignore", &entries),
            "write `.gitignore`"
        );
    }

    #[test]
    fn a_longer_identifier_and_a_fence_are_untouched() {
        let entries = [code("useState")];
        assert_eq!(
            wrap_code_terms("const useStateValue = 1", &entries),
            "const useStateValue = 1"
        );
        let fenced = "```ts\nconst useState = 1\n```";
        assert_eq!(wrap_code_terms(fenced, &entries), fenced);
        let tildes = "~~~\nuseState\n~~~";
        assert_eq!(wrap_code_terms(tildes, &entries), tildes);
    }

    #[test]
    fn the_longest_code_term_wins_inside_a_shared_name() {
        let entries = [code("use"), code("useState")];
        assert_eq!(
            wrap_code_terms("call useState now", &entries),
            "call `useState` now"
        );
    }

    #[test]
    fn a_term_not_marked_code_is_never_wrapped() {
        let entries = [VocabularyEntry {
            term: "useState".into(),
            sounds_like: Vec::new(),
            code: false,
        }];
        assert_eq!(wrap_code_terms("call useState", &entries), "call useState");
    }

    #[test]
    fn a_code_term_without_spellings_saves_as_an_object() {
        let entry = VocabularyEntry {
            term: "useState".into(),
            sounds_like: Vec::new(),
            code: true,
        };
        let saved = serde_json::to_string(&entry).unwrap();
        assert_eq!(saved, r#"{"term":"useState","code":true}"#);
        assert_eq!(
            serde_json::from_str::<VocabularyEntry>(&saved).unwrap(),
            entry
        );
    }

    #[test]
    fn adding_code_marks_an_existing_term() {
        let mut list = Vec::new();
        assert!(add(&mut list, "useState", &[], false).unwrap());
        assert!(!list[0].code);
        assert!(add(&mut list, "usestate", &[], true).unwrap());
        assert!(list[0].code);
        assert!(
            !add(&mut list, "useState", &[], true).unwrap(),
            "already code"
        );
    }

    #[test]
    fn wrap_code_terms_defaults_off_and_round_trips() {
        let mut settings = crate::app_settings::AppSettings::default();
        assert!(!settings.dictation_processing.wrap_code_terms);
        settings.dictation_processing.wrap_code_terms = true;
        let saved = serde_json::to_string(&settings).unwrap();
        let loaded: crate::app_settings::AppSettings = serde_json::from_str(&saved).unwrap();
        assert!(loaded.dictation_processing.wrap_code_terms);
    }
}
