//! The local model's files: the llama.cpp server Hex runs and the GGUF it
//! loads. A sheet from the Models card with one file button each, so a
//! portable copy needs no hand-edited settings.

use rune::{El, col, el, row, text};

use super::Ui;
use super::controls::{
    ButtonKind, DESCRIPTION, Paint, button, modal_scrim_transition, modal_transition,
};
use crate::desktop::file_dialog::{self, Dialog, Filter, Pending, Target};
use crate::desktop::i18n::tr;

const MODAL_WIDTH: f32 = 560.0;
const PADDING: f32 = 18.0;

/// Which file the open dialog will answer for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Choosing {
    Program,
    Model,
}

/// The sheet's state, and the dialog while an OS file picker is open.
#[derive(Default)]
pub(super) struct LocalModels {
    pub(super) open: bool,
    chooser: Option<(Choosing, Pending)>,
    /// Why the last choice failed, or why nothing can start yet.
    error: Option<String>,
}

/// Opens the sheet alone.
pub(super) fn open(ui: &mut Ui) {
    super::settings::cancel_capture(ui);
    super::picker::close(ui);
    super::explainer::close(ui);
    super::sound_sets::close(ui);
    super::vocabulary::close(ui);
    super::shortcut::guard::close(ui);
    super::ai_models::close_prompts(ui);
    ui.open = None;
    ui.local_models.open = true;
    ui.local_models.error = None;
}

/// Closes the sheet. Nothing typed is pending, so there is nothing to save.
pub(super) fn close(ui: &mut Ui) {
    if !ui.local_models.open {
        return;
    }
    ui.local_models.open = false;
    ui.local_models.chooser = None;
    ui.local_models.error = None;
}

fn choose(ui: &mut Ui, which: Choosing) {
    if !ui.live() || ui.local_models.chooser.is_some() {
        return;
    }
    let (title, filters, directory) = match which {
        Choosing::Program => (
            tr("Choose llama-server…"),
            vec![
                Filter {
                    label: tr("Programs").into(),
                    extensions: &["exe", "com", "bat"],
                },
                Filter {
                    label: tr("All files").into(),
                    extensions: &["*"],
                },
            ],
            dirs::home_dir(),
        ),
        Choosing::Model => (
            tr("Add model file…"),
            vec![
                Filter {
                    label: tr("GGUF models").into(),
                    extensions: &["gguf"],
                },
                Filter {
                    label: tr("All files").into(),
                    extensions: &["*"],
                },
            ],
            dirs::download_dir(),
        ),
    };
    let waker = ui.modes.waker.clone();
    let wake = move || {
        if let Some(waker) = &waker {
            waker();
        }
    };
    let pending = file_dialog::open(
        Dialog {
            title: title.into(),
            target: Target::File(filters),
            directory,
        },
        wake,
    );
    ui.local_models.chooser = Some((which, pending));
}

/// Takes the dialog's answer. True when it changed what the sheet shows.
pub(super) fn poll(ui: &mut Ui) -> bool {
    let Some((which, pending)) = ui.local_models.chooser.take() else {
        return false;
    };
    let Some(choice) = pending.poll() else {
        ui.local_models.chooser = Some((which, pending));
        return false;
    };
    match choice {
        Ok(Some(path)) => {
            ui.local_models.error = None;
            ui.update(move |settings| match which {
                Choosing::Program => settings.llm.program = Some(path),
                Choosing::Model => settings.llm.model = Some(path),
            });
        }
        Ok(None) => {}
        Err(error) => ui.local_models.error = Some(error),
    }
    true
}

/// The two files the server needs, with the current choice as its value.
fn file_row(
    p: Paint,
    key: &'static str,
    title: &'static str,
    path: &std::path::Path,
    button_key: &'static str,
    label: &'static str,
    which: Choosing,
) -> El {
    let present = path.is_file();
    let detail = path.display().to_string();
    row()
        .key(key)
        .min_h(super::controls::METRICS.row_h_described)
        .px(super::controls::METRICS.row_px)
        .py(8.0)
        .gap(12.0)
        .center_y()
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .shrink()
                .gap(2.0)
                .child(text(title).key("title").font(p.k.ty.ui).color(p.text()))
                .child(
                    text(detail)
                        .key("detail")
                        .font(DESCRIPTION)
                        .color(if present { p.muted() } else { p.danger() })
                        .wrap(),
                ),
        )
        .child(button(
            p,
            button_key,
            label,
            ButtonKind::Secondary,
            move |ui: &mut Ui, _| choose(ui, which),
        ))
}

pub(super) fn modal(ui: &Ui, p: Paint) -> Option<[El; 2]> {
    if !ui.local_models.open {
        return None;
    }
    let c = &p.k.c;
    let config = crate::local_model::Config::from_settings(&ui.settings.llm);
    let mut heading = col()
        .key("heading")
        .gap(4.0)
        .child(
            text(tr("Local model files"))
                .key("title")
                .font(p.k.ty.body_medium)
                .color(p.text()),
        )
        .child(
            text(tr("llama.cpp and the GGUF the server loads"))
                .key("description")
                .font(DESCRIPTION)
                .color(p.muted())
                .wrap(),
        );
    if !config.program.is_file() {
        heading = heading.child(
            text(tr(
                "llama.cpp was not found, so the local model stays off until it is",
            ))
            .key("missing")
            .font(DESCRIPTION)
            .color(p.danger())
            .wrap(),
        );
    }
    if let Some(error) = &ui.local_models.error {
        heading = heading.child(
            text(error.clone())
                .key("error")
                .font(DESCRIPTION)
                .color(p.danger())
                .wrap(),
        );
    }
    let files = col()
        .key("files")
        .child(file_row(
            p,
            "llama-server",
            tr("llama.cpp"),
            &config.program,
            "choose-llama-server",
            tr("Choose llama-server…"),
            Choosing::Program,
        ))
        .child(file_row(
            p,
            "local-model-file",
            tr("Model file"),
            &config.model,
            "choose-local-model",
            tr("Add model file…"),
            Choosing::Model,
        ));
    let footer = row()
        .key("footer")
        .gap(12.0)
        .center_y()
        .child(el().key("spacer").grow(1.0))
        .child(
            button(
                p,
                "close-local-models",
                tr("Done"),
                ButtonKind::Secondary,
                |ui: &mut Ui, _| close(ui),
            )
            .default_button(),
        );
    let mut panel = col()
        .key("local-models")
        .role("dialog")
        .label(tr("Local model files"))
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
        .child(files)
        .child(footer);
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
        layer("local-models-scrim")
            .fill(c.scrim)
            .on_press(close)
            .transition(modal_scrim_transition(p)),
        layer("local-models-layer")
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
    fn the_sheet_shows_both_file_rows_and_their_buttons() {
        let mut ui = ui();
        open(&mut ui);
        assert!(ui.local_models.open);
        let env = rune::Env::new(true, 1.0);
        let text =
            rune::text::TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES);
        let mut rt =
            crate::rune_ui::offscreen::runtime(ui, super::super::view, (980.0, 760.0), env, text);
        rt.settle_all();
        rt.frame(0.0);
        assert!(rt.find("llama-server").is_some(), "the program row");
        assert!(rt.find("local-model-file").is_some(), "the model row");
        assert!(
            rt.find("choose-llama-server").is_some(),
            "the program file button"
        );
        assert!(
            rt.find("choose-local-model").is_some(),
            "the Add model file button"
        );

        close(&mut rt.state);
        assert!(!rt.state.local_models.open);
    }

    #[test]
    fn a_fixture_never_opens_a_dialog() {
        let mut ui = ui();
        choose(&mut ui, Choosing::Program);
        assert!(ui.local_models.chooser.is_none(), "not a live window");
        assert!(!poll(&mut ui));
    }
}
