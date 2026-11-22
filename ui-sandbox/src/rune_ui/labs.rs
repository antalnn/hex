//! The Labs pane, fixture only: one card, the Transcription endpoint's, drawn
//! as the app draws a lab card, so its look can be reviewed. General's
//! search shortcut asks it for focus.

mod transcription_endpoint;

use rune::{El, col, el, row, text};
use rune_kit::{ControlSize, Switch};

use super::controls::{DESCRIPTION, METRICS, Paint};
use super::icons::{PASTE, tile};
use super::{Ui, scroll_body};
use crate::desktop::i18n::tr;

pub(crate) use transcription_endpoint::State;

pub(super) fn focus_search(_ui: &mut Ui) {}

/// The port the endpoint listens on in the app.
const PORT: u16 = 8787;
const CARD_PADDING: f32 = 12.0;
const TILE: f32 = 32.0;

/// The Labs pane with the one card, under its group's header.
pub(super) fn pane(ui: &Ui, p: Paint, _layer: &mut Vec<El>) -> El {
    let content = col()
        .key("labs")
        .px(METRICS.gutter)
        .py(12.0)
        .gap(12.0)
        .child(
            text(tr("Output"))
                .key("group-output")
                .role("heading")
                .font(p.k.ty.ui_medium)
                .color(p.text()),
        )
        .child(card(ui, p));
    scroll_body(p.k, "labs-scroll", content)
}

/// The lab as a card, in the app's own layout: tile, name and switch on one
/// line, the description under them, and while on the lab's own rows.
fn card(ui: &Ui, p: Paint) -> El {
    let c = &p.k.c;
    let on = ui.settings.labs.transcription_endpoint;
    let title = tr("Transcription endpoint for other apps");
    let header = row()
        .key("title-row")
        .min_w(0.0)
        .gap(6.0)
        .center_y()
        .child(
            text(title)
                .key("title")
                .font(p.k.ty.ui_medium)
                .color(p.text())
                .truncate()
                .shrink()
                .min_w(0.0),
        )
        .child(el().key("spacer").grow(1.0).min_w(8.0))
        .child(
            Switch::new(on)
                .control_size(ControlSize::Mini)
                .label(title)
                .hide_label()
                .on_toggle(|ui: &mut Ui| {
                    let on = !ui.settings.labs.transcription_endpoint;
                    ui.update(|settings| settings.labs.transcription_endpoint = on);
                })
                .build(p.k)
                .size(44.0, 20.0)
                .justify_end()
                .key("lab-transcription-endpoint"),
        );
    let mut copy = col()
        .key("copy")
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .gap(2.0)
        .child(header)
        .child(
            text(tr(
                "Lets apps on this PC, such as OpenChamber, send audio to Hex and get the finished text back",
            ))
            .key("description")
            .font(DESCRIPTION)
            .color(p.muted())
            .max_lines(2),
        );
    if on {
        copy = copy.child(transcription_endpoint::body(p, ui.endpoint, PORT).mt(8.0));
    }
    row()
        .key("lab-card-transcription-endpoint")
        .role("group")
        .label(title)
        .padding(CARD_PADDING)
        .gap(10.0)
        .items_start()
        .radius(p.k.radius.lg)
        .child(tile(p, &PASTE, TILE))
        .child(copy)
        .fill(c.bg_base)
        .border(p.hair, c.border_weak)
}
