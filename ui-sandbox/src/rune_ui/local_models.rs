//! The local model files sheet, fixture only: the sandbox has no file picker,
//! so the two rows show the paths the real sheet reads and their Choose
//! buttons do nothing. Its place in the layer order is real: it draws above
//! the setup window, as the app's sheet does.
//!
//! The pane itself is the real `ai_models.rs` copy; only this module is a
//! stand-in for the sheet.

use rune::{El, col, el, row, text};

use super::Ui;
use super::controls::{
    ButtonKind, DESCRIPTION, METRICS, Paint, button, modal_scrim_transition, modal_transition,
};
use crate::desktop::i18n::tr;

const MODAL_WIDTH: f32 = 560.0;
const PADDING: f32 = 18.0;

/// The sheet's state: open or not.
#[derive(Default)]
pub(super) struct LocalModels {
    pub(super) open: bool,
}

/// Opens the sheet over the setup window, which stays open.
pub(super) fn open(ui: &mut Ui) {
    ui.model_sources.prompts_open = false;
    ui.model_sources.cleanup_open = false;
    ui.local_models.open = true;
}

/// Closes the sheet.
pub(super) fn close(ui: &mut Ui) {
    ui.local_models.open = false;
}

/// One path row: what the file is, the path it holds, and a Choose button that
/// changes nothing here.
fn file_row(p: Paint, key: &'static str, title: &'static str, path: &str) -> El {
    row()
        .key(key)
        .min_h(METRICS.row_h_described)
        .px(METRICS.row_px)
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
                    text(path.to_string())
                        .key("detail")
                        .font(DESCRIPTION)
                        .color(p.muted())
                        .wrap(),
                ),
        )
        .child(button(
            p,
            match key {
                "llama-server" => "choose-llama-server",
                _ => "choose-local-model",
            },
            tr("Choose…"),
            ButtonKind::Secondary,
            |_: &mut Ui, _| {},
        ))
}

pub(super) fn modal(ui: &Ui, p: Paint) -> Option<[El; 2]> {
    if !ui.local_models.open {
        return None;
    }
    let c = &p.k.c;
    let program = ui
        .settings
        .llm
        .program
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "llama-server.exe".to_string());
    let model = ui
        .settings
        .llm
        .model
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "model.gguf".to_string());
    let heading = col()
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
    let files = col()
        .key("files")
        .child(file_row(p, "llama-server", tr("llama.cpp"), &program))
        .child(file_row(p, "local-model-file", tr("Model file"), &model));
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
            .child(panel.modal(true, close)),
    ])
}
