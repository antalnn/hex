//! First-run and model progress, fixture only: the selected model is always
//! installed, so the pane's missing-model rows never show.

use super::Ui;
use crate::desktop::onboarding::ModelProgress;

pub(super) fn progress(_ui: &Ui) -> (bool, ModelProgress) {
    (true, ModelProgress::default())
}
