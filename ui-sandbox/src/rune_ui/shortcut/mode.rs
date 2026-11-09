//! Shortcut mode: General's row choosing how the dictation shortcut
//! triggers.

use rune::El;

use super::super::controls::{Paint, copy_row, value_button};
use super::super::icons::HAND;
use super::super::{Ui, explainer};
use crate::app_settings::{AppSettings, HotkeyKey};
use crate::desktop::i18n::{tr, tr_fill};
use crate::dictation::ShortcutMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::rune_ui) enum Trigger {
    HoldOrDoubleTap,
    Hold,
    Tap,
    DoubleTap,
}

impl Trigger {
    pub(in crate::rune_ui) fn of(settings: &AppSettings) -> Self {
        match settings.shortcut_mode {
            ShortcutMode::Toggle => Self::Tap,
            ShortcutMode::Hold if !settings.double_tap_lock => Self::Hold,
            ShortcutMode::Hold if settings.double_tap_only => Self::DoubleTap,
            ShortcutMode::Hold => Self::HoldOrDoubleTap,
        }
    }

    pub(in crate::rune_ui) fn name(self) -> &'static str {
        match self {
            Self::HoldOrDoubleTap => tr("Hold or double-tap"),
            Self::Hold => tr("Hold"),
            Self::Tap => tr("Tap"),
            Self::DoubleTap => tr("Double-tap"),
        }
    }
}

/// General's row, under the shortcut.
pub(in crate::rune_ui) fn row(ui: &Ui, p: Paint) -> El {
    copy_row(
        p,
        "shortcut-mode",
        Some(&HAND),
        tr("Shortcut mode"),
        None,
        value_button(
            p,
            "shortcut-mode-control",
            tr("Shortcut mode"),
            Trigger::of(&ui.settings).name(),
            &rune_theme_opencode::icons::CHEVRON_RIGHT,
            explainer::open,
        ),
        false,
    )
}

/// Kept for the unavailable-double-tap note; always available in the
/// fixture because the dictation shortcut has a key.
#[allow(dead_code)]
fn unavailable(settings: &AppSettings) -> Option<String> {
    if settings.dictation_hotkey.key.is_none() {
        let mut example = settings.dictation_hotkey.clone();
        example.key = Some(HotkeyKey::named("space"));
        Some(tr_fill(
            "Needs a shortcut with a key, like {}",
            &example.keycaps().join("+"),
        ))
    } else {
        None
    }
}
