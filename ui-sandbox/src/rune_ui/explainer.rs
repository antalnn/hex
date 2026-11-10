//! The shortcut-mode explainer, fixture only: the mode row opens nothing.

use super::Ui;

#[derive(Default)]
pub(crate) struct Explainer {
    pub(crate) open: bool,
}

pub(super) fn open(ui: &mut Ui) {
    ui.explainer.open = true;
}

pub(super) fn close(ui: &mut Ui) {
    ui.explainer.open = false;
}
