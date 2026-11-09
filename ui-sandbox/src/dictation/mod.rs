//! Dictation types the General pane reads, fixture only.

/// How the dictation shortcut triggers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ShortcutMode {
    #[default]
    Hold,
    Toggle,
}

/// One shared-vocabulary entry.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VocabularyEntry {
    pub term: String,
    pub sounds_like: Vec<String>,
    pub code: bool,
}
