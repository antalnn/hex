//! The sound picker, fixture only.

use super::Ui;

#[derive(Default)]
pub(crate) struct SoundSets {
    pub(crate) open: bool,
}

pub(super) fn open(ui: &mut Ui) {
    ui.sound_sets.open = true;
}
