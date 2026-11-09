//! A standalone shell for Hex's settings window, for UI work that needs fast
//! rebuilds and no audio, hotkey, or model code.
//!
//! The panes are copies of the real ones (`src/rune_ui/` in the hex
//! checkout); the host around them is a fixture. Changes made here are moved
//! back into the app once the sandbox is green.

// The fixture carries the surface the copied panes read, which is wider than
// what the one pane the window draws touches today.
#![allow(dead_code)]

pub(crate) mod app_settings;
pub(crate) mod audio;
pub(crate) mod desktop;
pub(crate) mod dictation;
pub(crate) mod feedback;
pub(crate) mod history;
pub(crate) mod labs;
pub(crate) mod local_model;
pub(crate) mod rune_ui;
pub(crate) mod transcription;
pub(crate) mod updater;
pub(crate) mod windows;

use rune::AppSpec;

/// Opens the sandbox's window over its fixture state.
pub fn run() {
    rune::run(AppSpec::<rune_ui::Ui> {
        fonts: rune_theme_opencode::FONTS,
        families: rune_theme_opencode::FAMILIES,
        ..AppSpec::new(
            "Hex UI sandbox",
            (980.0, 760.0),
            rune_ui::Ui::fixture,
            rune_ui::view,
        )
    });
}
