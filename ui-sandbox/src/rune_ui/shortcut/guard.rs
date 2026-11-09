//! The Excluded apps row that opens General's Advanced › Dictation section,
//! fixture only: the count of apps where Hex's shortcuts stay out of the way.

use super::super::Ui;
use super::super::controls::{CardRow, Paint, icon_row, value_button};
use crate::desktop::i18n::{tr, tr_fill};

/// What General's row shows: how many apps are saved.
fn summary(ui: &Ui) -> String {
    let count = ui.settings.shortcuts.excluded_processes.len();
    match count {
        0 => tr("None").to_string(),
        1 => tr_fill("{} app", &count.to_string()),
        count => tr_fill("{} apps", &count.to_string()),
    }
}

pub(in crate::rune_ui) fn general_row(ui: &Ui, p: Paint) -> CardRow {
    icon_row(
        p,
        "excluded-apps",
        &rune_theme_opencode::icons::KEYBOARD,
        tr("Excluded apps"),
        None,
        value_button(
            p,
            "excluded-apps-control",
            tr("Excluded apps"),
            &summary(ui),
            &rune_theme_opencode::icons::CHEVRON_RIGHT,
            |_ui: &mut Ui| {},
        ),
    )
    .into()
}
