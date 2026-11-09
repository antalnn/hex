//! Custom speech-model imports, fixture only.

use rune::El;

use super::Ui;
use super::controls::{ButtonKind, Paint};
use crate::desktop::i18n::tr;

pub(super) fn selection_notice(_ui: &Ui) -> Option<String> {
    None
}

pub(super) fn download_failure(_ui: &Ui) -> Option<String> {
    None
}

pub(super) fn use_file_button(_ui: &Ui, p: Paint) -> El {
    super::controls::button(
        p,
        "transcription-file",
        tr("Add model file…"),
        ButtonKind::Secondary,
        |_ui: &mut Ui, _| {},
    )
}
