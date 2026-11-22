//! Labs › Transcription endpoint's card while on: whether Hex is listening,
//! the Server URL and Endpoint path another app needs, each with a Copy
//! button, and how the model is chosen.

use rune::{El, col, row, text};

use super::super::controls::{ButtonKind, DESCRIPTION, Paint, button, status_dot};
use crate::desktop::i18n::{tr, tr_fill};

/// The path an app adds to the Server URL.
const PATH: &str = "/audio/transcriptions";
/// The label and value columns of a copy line, wide enough for the longest
/// translated label and the Server URL.
const LABEL_WIDTH: f32 = 168.0;
const VALUE_WIDTH: f32 = 190.0;

/// Where the endpoint stands, as the card says it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum State {
    /// The listener is not up yet, as when the lab was just switched on.
    Off,
    Listening,
    /// Another program holds the port.
    PortInUse,
    Failed,
}

pub(super) fn body(p: Paint, state: State, port: u16) -> El {
    let url = format!("http://127.0.0.1:{port}/v1");
    col()
        .key("transcription-endpoint")
        .gap(6.0)
        .children(status(p, state, port))
        .child(copy_line(p, "server-url", tr("Server URL"), url))
        .child(copy_line(
            p,
            "endpoint-path",
            tr("Endpoint path"),
            PATH.to_owned(),
        ))
        .child(
            text(tr(
                "Use any model name; Hex uses the model selected in General.",
            ))
            .key("model-note")
            .font(DESCRIPTION)
            .color(p.muted())
            .wrap(),
        )
}

fn status(p: Paint, state: State, port: u16) -> Option<El> {
    let (dot, line) = match state {
        State::Off => return None,
        State::Listening => (p.success(), tr("Listening").to_owned()),
        State::PortInUse => (
            p.warning(),
            tr_fill("Port {} is in use by another program", &port.to_string()),
        ),
        State::Failed => (p.danger(), tr("Could not start").to_owned()),
    };
    Some(
        row()
            .key("status")
            .gap(6.0)
            .center_y()
            .child(status_dot(dot))
            .child(
                text(line)
                    .key("state")
                    .font(DESCRIPTION)
                    .color(p.muted())
                    .wrap(),
            ),
    )
}

/// One value another app needs, with the button that copies it. The row is
/// named by its label and value, so the two Copy buttons are told apart.
fn copy_line(p: Paint, key: &'static str, label: &str, value: String) -> El {
    let copied = value.clone();
    row()
        .key(key)
        .role("group")
        .label(format!("{label}: {value}"))
        .gap(8.0)
        .center_y()
        .flex_wrap()
        // Fixed columns, so both Copy buttons line up.
        .child(
            text(label.to_owned())
                .key("label")
                .font(DESCRIPTION)
                .color(p.muted())
                .w(LABEL_WIDTH)
                .a11y_hidden(),
        )
        .child(
            text(value)
                .key("value")
                .font(p.k.ty.mono)
                .color(p.text())
                .w(VALUE_WIDTH)
                .a11y_hidden(),
        )
        .child(button(
            p,
            &format!("{key}-copy"),
            tr("Copy"),
            ButtonKind::Secondary,
            move |_, tx| tx.copy(copied.clone()),
        ))
}
