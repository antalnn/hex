//! The Vocabulary row's state, fixture only.

use super::Ui;

#[derive(Default)]
pub(crate) struct VocabularyEditor {
    pub(crate) open: bool,
}

pub(super) fn unused_note(_ui: &Ui) -> Option<String> {
    None
}

pub(super) fn summary(ui: &Ui) -> String {
    ui.settings.vocabulary.len().to_string()
}

pub(super) fn open(ui: &mut Ui) {
    ui.vocabulary.open = true;
}
