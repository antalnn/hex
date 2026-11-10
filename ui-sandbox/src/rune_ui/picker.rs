//! The speech-model picker, fixture only.

use super::Ui;
use crate::transcription::models::TranscriptionSelection;

#[derive(Default)]
pub(crate) struct Picker {
    pub(crate) open: bool,
    pub(crate) installed: Option<TranscriptionSelection>,
}

impl Picker {
    pub(super) fn installed(
        &self,
        _model: &crate::transcription::models::ModelDefinition,
        _language: &str,
    ) -> bool {
        true
    }
}

pub(super) fn open(ui: &mut Ui) {
    ui.picker.open = true;
}

pub(super) fn close(ui: &mut Ui) {
    ui.picker.open = false;
}

pub(super) fn fallback_note(_ui: &Ui) -> Option<String> {
    None
}

pub(super) fn choose_language(ui: &mut Ui, code: &str) {
    let language = code.to_string();
    ui.update(|settings| settings.transcription.language = language);
}
