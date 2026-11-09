//! Labs switches the General pane reads, fixture only: each feature is a
//! plain switch on the settings.

use crate::app_settings::AppSettings;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Feature {
    SettingsSearch,
    RestoreShortcutDefault,
    SecondDictationShortcut,
    TerminalPasteShortcut,
    GnomeClipboardRestore,
}

impl Feature {
    pub fn on(self, settings: &AppSettings) -> bool {
        match self {
            Self::SettingsSearch => settings.labs.settings_search,
            Self::RestoreShortcutDefault => settings.labs.restore_shortcut_default,
            Self::SecondDictationShortcut => settings.labs.second_dictation_shortcut,
            Self::TerminalPasteShortcut => settings.labs.terminal_paste_shortcut,
            Self::GnomeClipboardRestore => settings.labs.gnome_clipboard_restore,
        }
    }

    pub fn shipped(self) -> bool {
        true
    }
}

/// What Labs changes when a switch moves; the fixture has nothing to apply.
pub(crate) fn apply(_settings: &AppSettings) {}
