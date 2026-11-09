//! Rows only some OSes show, fixture only: the Windows build has no
//! permission or Meetings rows, and the shortcut side picker's choices.

use rune::El;

use super::controls::Paint;
use super::{ShortcutTarget, Ui};
use crate::app_settings::{AppSettings, HotkeyBinding, ModifierSide};

pub(crate) const SIDES: [ModifierSide; 3] = [
    ModifierSide::Left,
    ModifierSide::Either,
    ModifierSide::Right,
];

/// None: the sandbox runs no permission-gated OS.
pub(super) fn permissions_section(_ui: &Ui, _p: Paint) -> Option<El> {
    None
}

/// None: nothing has failed in the fixture.
pub(super) fn error_section(_ui: &Ui, _p: Paint) -> Option<El> {
    None
}

/// The side a modifier-only shortcut must use; the fixture has none.
pub(super) fn picked_side(_ui: &Ui, _target: ShortcutTarget) -> Option<ModifierSide> {
    None
}

/// The binding with its modifier moved to `side`.
pub(super) fn side_binding(
    _settings: &AppSettings,
    target: ShortcutTarget,
    side: ModifierSide,
) -> Option<HotkeyBinding> {
    let _ = (target, side);
    None
}
