//! The shortcut guard: a sheet from General's Excluded apps row with one
//! executable per row, and the switch that pauses the shortcuts in any
//! fullscreen window. A game keeps `Control+Space` while its name is in the
//! list.

use rune::{El, TextEdit, Tx, col, edit_access, el, row, text};
use rune_kit::{ControlSize, TextInput};

use super::Ui;
use super::controls::{
    ButtonKind, DESCRIPTION, Paint, button, modal_scrim_transition, modal_transition, toggle_row,
};
use super::lists::remove_button;
use crate::app_settings::ShortcutGuard;
use crate::desktop::i18n::{tr, tr_fill};

const MODAL_WIDTH: f32 = 560.0;
const PADDING: f32 = 18.0;
/// One app row: a regular text field and its padding.
const ROW_HEIGHT: f32 = 36.0;
/// The list's fades; the bottom one is the gap before the footer at rest.
const FADES: (f32, f32) = (0.0, 6.0);
/// The rows scroll past six whole rows, so a list scrolled to its end shows no
/// sliver of a row above.
const ROWS_HEIGHT: f32 = 6.0 * ROW_HEIGHT + FADES.0 + FADES.1;
/// Room beside the rows inside the list's clip, so a field's border isn't cut
/// at its sides.
const SIDE_ROOM: f32 = 2.0;

/// One app row and the identity that stays with it as rows before it are
/// removed, so a callback built for a removed row never reaches another one.
struct AppRow {
    id: u64,
    name: TextEdit,
}

/// The app rows, and whether the sheet is open.
#[derive(Default)]
pub(super) struct GuardEditor {
    rows: Vec<AppRow>,
    next_id: u64,
    /// What an edit for a row that is gone lands in; never saved.
    detached: TextEdit,
    pub(super) open: bool,
}

impl GuardEditor {
    pub(super) fn new(guard: &ShortcutGuard) -> Self {
        let mut editor = Self::default();
        editor.reload(guard);
        editor
    }

    /// Replaces the rows with `guard`'s apps, keeping the sheet open or shut.
    pub(super) fn reload(&mut self, guard: &ShortcutGuard) {
        self.rows.clear();
        for app in &guard.excluded_processes {
            self.push(TextEdit::new(app.clone()));
        }
    }

    fn push(&mut self, name: TextEdit) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.rows.push(AppRow { id, name });
        id
    }

    /// The name field of row `id`. A row already removed gets a detached field
    /// instead, so a late edit for it touches nothing that is saved.
    fn field(&mut self, id: u64) -> &mut TextEdit {
        match self.rows.iter_mut().find(|row| row.id == id) {
            Some(row) => &mut row.name,
            None => {
                self.detached = TextEdit::default();
                &mut self.detached
            }
        }
    }

    /// The rows as the list saves them: trimmed, lowercased, `.exe` gone, each
    /// app once.
    pub(super) fn project(&self) -> Vec<String> {
        let names: Vec<String> = self.rows.iter().map(|row| row.name.text.clone()).collect();
        ShortcutGuard::clean(&names)
    }

    /// Whether a row repeats an app above it, which the list does not save.
    fn repeated(&self) -> bool {
        let mut seen: Vec<String> = Vec::new();
        for row in &self.rows {
            let Some(name) = ShortcutGuard::cleaned(&row.name.text) else {
                continue;
            };
            if seen.contains(&name) {
                return true;
            }
            seen.push(name);
        }
        false
    }
}

/// What General's row shows: how many apps are saved, and whether the
/// fullscreen pause is on.
pub(super) fn summary(ui: &Ui) -> String {
    let count = ui.settings.shortcuts.excluded_processes.len();
    let apps = match count {
        0 => tr("None").to_string(),
        1 => tr_fill("{} app", &count.to_string()),
        count => tr_fill("{} apps", &count.to_string()),
    };
    if ui.settings.shortcuts.disable_in_fullscreen {
        tr_fill("{} · fullscreen", &apps)
    } else {
        apps
    }
}

/// General's row: the count, opening the sheet.
pub(super) fn general_row(ui: &Ui, p: Paint) -> El {
    super::controls::icon_row(
        p,
        "excluded-apps",
        &rune_theme_opencode::icons::KEYBOARD,
        tr("Excluded apps"),
        None,
        super::controls::value_button(
            p,
            "excluded-apps-control",
            tr("Excluded apps"),
            &summary(ui),
            &rune_theme_opencode::icons::CHEVRON_RIGHT,
            open,
        ),
    )
}

/// Opens the sheet alone, with an empty row to type into when the list is
/// empty.
pub(super) fn open(ui: &mut Ui) {
    super::settings::cancel_capture(ui);
    super::picker::close(ui);
    super::explainer::close(ui);
    super::sound_sets::close(ui);
    super::vocabulary::close(ui);
    ui.open = None;
    ui.shortcut_guard.open = true;
    if ui.shortcut_guard.rows.is_empty() {
        add_row(ui);
    }
}

/// Saves what was typed, drops rows left blank, and closes the sheet.
pub(super) fn close(ui: &mut Ui) {
    if !ui.shortcut_guard.open {
        return;
    }
    ui.shortcut_guard.open = false;
    ui.flush_drafts();
    ui.shortcut_guard
        .rows
        .retain(|row| !row.name.text.trim().is_empty());
}

fn add_row(ui: &mut Ui) -> u64 {
    for row in &mut ui.shortcut_guard.rows {
        row.name.focused = false;
    }
    let mut name = TextEdit::default();
    name.focused = true;
    ui.shortcut_guard.push(name)
}

/// Add app: a new focused row at the end, which the list glides down to as the
/// row arrives.
fn add_app(ui: &mut Ui, tx: &mut Tx) {
    add_row(ui);
    tx.scroll_to_bottom("app-scroll");
}

fn remove_row(ui: &mut Ui, id: u64) {
    if let Some(index) = ui.shortcut_guard.rows.iter().position(|row| row.id == id) {
        ui.shortcut_guard.rows.remove(index);
        ui.flush_drafts();
    }
}

/// Flips the sheet's fullscreen switch and saves at once. Writing the field
/// and asking `flush_drafts` to save it does nothing: drafts start from the
/// same settings, so the copy matches and no commit runs.
fn flip_fullscreen(ui: &mut Ui) {
    ui.update(|settings| {
        settings.shortcuts.disable_in_fullscreen = !settings.shortcuts.disable_in_fullscreen;
    });
}

fn app_row(p: Paint, entry: &AppRow) -> El {
    let id = entry.id;
    row()
        .key(format!("app{id}"))
        .h(ROW_HEIGHT)
        .gap(8.0)
        .center_y()
        .child(
            TextInput::new(&entry.name)
                .placeholder(tr("App (e.g. cs2)"))
                .width(None)
                .on_edit(edit_access(move |ui: &mut Ui| ui.shortcut_guard.field(id)))
                .build(p.k)
                .key("app")
                .grow(1.0)
                .shrink(),
        )
        .child(remove_button(
            p,
            ControlSize::Regular,
            move |ui: &mut Ui| remove_row(ui, id),
        ))
}

pub(super) fn modal(ui: &Ui, p: Paint) -> Option<[El; 2]> {
    if !ui.shortcut_guard.open {
        return None;
    }
    let c = &p.k.c;
    let rows = col()
        .key("apps")
        .px(SIDE_ROOM)
        .children(ui.shortcut_guard.rows.iter().map(|entry| app_row(p, entry)));
    let height = ui.shortcut_guard.rows.len() as f32 * ROW_HEIGHT + FADES.0 + FADES.1;
    let apps = col()
        .key("app-column")
        .h(height.min(ROWS_HEIGHT))
        .mx(-SIDE_ROOM)
        .child(super::faded_body(p.k, "app-scroll", rows, FADES));
    let add = button(p, "add-app", tr("Add app"), ButtonKind::Secondary, add_app);
    let footer = row()
        .key("footer")
        .gap(12.0)
        .center_y()
        .child(add)
        .child(el().key("spacer").grow(1.0))
        .child(
            button(
                p,
                "close-guard",
                tr("Done"),
                ButtonKind::Secondary,
                |ui: &mut Ui, _| close(ui),
            )
            .default_button(),
        );
    let mut heading = col()
        .key("heading")
        .gap(4.0)
        .child(
            text(tr("Excluded apps"))
                .key("title")
                .font(p.k.ty.body_medium)
                .color(p.text()),
        )
        .child(
            text(tr(
                "Apps where Hex's shortcuts do nothing, so a game keeps its keys",
            ))
            .key("description")
            .font(DESCRIPTION)
            .color(p.muted())
            .wrap(),
        )
        .child(toggle_row(
            p,
            "pause-fullscreen",
            tr("Pause shortcuts in fullscreen"),
            Some(tr("Also pause in any window that covers the whole screen")),
            ui.settings.shortcuts.disable_in_fullscreen,
            flip_fullscreen,
        ));
    if ui.shortcut_guard.repeated() {
        heading = heading.child(
            text(tr(
                "Each app is listed once, so a repeated name is not saved",
            ))
            .key("repeated")
            .font(DESCRIPTION)
            .color(p.muted())
            .wrap(),
        );
    }
    let mut panel = col()
        .key("shortcut-guard")
        .role("dialog")
        .label(tr("Excluded apps"))
        .w(MODAL_WIDTH)
        .pt(PADDING)
        .px(PADDING)
        .pb(14.0)
        .gap(12.0)
        .radius(p.k.radius.xl)
        .fill(c.bg_base)
        .border(p.hair, c.border_weak)
        // Presses inside the panel stay inside.
        .on_press(|_: &mut Ui| {})
        .child(heading)
        .child(apps);
    panel = panel.child(footer);
    for shadow in &p.k.elevation.overlay {
        panel = panel.box_shadow(*shadow);
    }
    let layer = |key: &str| {
        el().key(key)
            .role("none")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
    };
    Some([
        layer("guard-scrim")
            .fill(c.scrim)
            .on_press(close)
            .transition(modal_scrim_transition(p)),
        layer("guard-layer")
            .center()
            .transition(modal_transition(p))
            // Keys stay inside, Escape or a press outside closes, and focus
            // returns to the row's button that opened it.
            .child(panel.modal(true, close)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        Ui::with(
            crate::app_settings::AppSettings::default(),
            super::super::Service::Fixture(Vec::new()),
        )
    }

    #[test]
    fn a_row_is_saved_cleaned_and_only_once() {
        let mut ui = ui();
        open(&mut ui);
        ui.shortcut_guard.rows[0].name = TextEdit::new(" CS2.exe ");
        add_row(&mut ui);
        ui.shortcut_guard.rows[1].name = TextEdit::new("cs2");
        assert!(ui.shortcut_guard.repeated());
        add_row(&mut ui);
        ui.shortcut_guard.rows[2].name = TextEdit::new("EldenRing.exe");
        close(&mut ui);
        assert!(!ui.shortcut_guard.open);
        assert_eq!(
            ui.settings.shortcuts.excluded_processes,
            vec!["cs2".to_string(), "eldenring".to_string()]
        );
        assert_eq!(summary(&ui), "2 apps");
    }

    #[test]
    fn the_fullscreen_switch_saves_at_once_and_the_summary_says_so() {
        let mut ui = ui();
        assert_eq!(summary(&ui), "None");
        flip_fullscreen(&mut ui);
        assert!(ui.settings.shortcuts.disable_in_fullscreen);
        assert_eq!(summary(&ui), "None · fullscreen");
    }

    #[test]
    fn reload_keeps_the_sheet_open_or_shut() {
        let guard = ShortcutGuard {
            excluded_processes: vec!["cs2".to_string()],
            disable_in_fullscreen: true,
        };
        let mut editor = GuardEditor::new(&guard);
        assert_eq!(editor.rows.len(), 1);
        editor.open = true;
        editor.reload(&guard);
        assert!(editor.open);
        assert_eq!(editor.rows.len(), 1);
    }
}
