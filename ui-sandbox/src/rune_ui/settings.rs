//! Shortcut rows and the recorder's state, fixture only: the same helpers the
//! General pane calls in the real `src/rune_ui/settings.rs`, without the
//! keyboard hook behind the recorder.

use rune::{El, row, text};

use super::controls::{
    ButtonKind, CardRow, Choice, Paint, icon_button, icon_row, key_tray, keycap, value_font,
};
use super::{Menu, Request, ShortcutTarget, Ui};
use crate::app_settings::{AppSettings, HotkeyBinding, HotkeyModifiers};
use crate::desktop::host::DesktopShortcuts;
use crate::desktop::i18n::{tr, tr_fill};

/// The volumes the Sound volume row offers, loudest first.
pub(super) const SOUND_VOLUME_STEPS: [(&str, f32); 5] = [
    ("Off", 0.0),
    ("Quiet", 0.25),
    ("Medium", 0.5),
    ("Loud", 0.75),
    ("Full", 1.0),
];

/// A recorder's state, as the General pane reads it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Capture {
    #[default]
    Idle,
    Listening {
        target: ShortcutTarget,
    },
}

/// What a shortcut's key button shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Face {
    Listening(&'static str),
    Shortcut { saved: bool },
}

pub(super) fn face(ui: &Ui, target: ShortcutTarget) -> Face {
    match ui.capture {
        Capture::Listening { target: active } if active == target => {
            Face::Listening(tr("Press shortcut"))
        }
        _ => Face::Shortcut { saved: false },
    }
}

pub(super) fn face_label(face: Face) -> &'static str {
    match face {
        Face::Listening(prompt) => prompt,
        Face::Shortcut { .. } => tr("Shortcut"),
    }
}

pub(super) fn recorder_face(p: Paint, face: Face, _live: bool) -> El {
    let copy = match face {
        Face::Listening(prompt) => prompt,
        Face::Shortcut { .. } => tr("Shortcut"),
    };
    row()
        .key("prompt")
        .h(24.0)
        .px(8.0)
        .center()
        .child(text(copy).key("label").font(value_font(p)).color(p.muted()))
}

pub(super) fn begin_capture(ui: &mut Ui, target: ShortcutTarget) {
    ui.send(Request::Desktop(
        crate::desktop::host::DesktopAction::ConfigureShortcuts,
    ));
    ui.capture = Capture::Listening { target };
}

pub(super) fn binding(settings: &AppSettings, target: ShortcutTarget) -> Option<&HotkeyBinding> {
    match target {
        ShortcutTarget::Dictation => Some(&settings.dictation_hotkey),
        ShortcutTarget::PasteLast => Some(&settings.paste_last_hotkey),
        ShortcutTarget::RawDictation => settings.raw_dictation_hotkey.as_ref(),
        ShortcutTarget::RewriteSelection => Some(&settings.rewrite_hotkey),
        ShortcutTarget::Cancel => Some(&settings.cancel_hotkey),
        ShortcutTarget::Capture | ShortcutTarget::TerminalPaste => None,
    }
}

pub(super) fn set_binding(
    settings: &mut AppSettings,
    target: ShortcutTarget,
    binding: HotkeyBinding,
) {
    match target {
        ShortcutTarget::Dictation => settings.dictation_hotkey = binding,
        ShortcutTarget::PasteLast => settings.paste_last_hotkey = binding,
        ShortcutTarget::RawDictation => settings.raw_dictation_hotkey = Some(binding),
        ShortcutTarget::RewriteSelection => settings.rewrite_hotkey = binding,
        ShortcutTarget::Cancel => settings.cancel_hotkey = binding,
        ShortcutTarget::Capture | ShortcutTarget::TerminalPaste => {}
    }
}

pub(super) fn clear_binding(settings: &mut AppSettings, target: ShortcutTarget) {
    match target {
        ShortcutTarget::RawDictation => settings.raw_dictation_hotkey = None,
        ShortcutTarget::RewriteSelection => {
            settings.rewrite_hotkey = HotkeyBinding::default();
        }
        _ => {}
    }
}

pub(super) fn restore_default(ui: &mut Ui, target: ShortcutTarget, default: HotkeyBinding) {
    ui.update(|settings| set_binding(settings, target, default));
}

pub(super) fn toggle_menu(ui: &mut Ui, menu: Menu) {
    ui.open = (ui.open != Some(menu)).then_some(menu);
}

pub(super) fn sound_volume_index(settings: &AppSettings) -> usize {
    if !settings.sound_effects {
        return 0;
    }
    SOUND_VOLUME_STEPS[1..]
        .iter()
        .enumerate()
        .min_by(|(_, (_, left)), (_, (_, right))| {
            (settings.sound_effect_volume - left)
                .abs()
                .total_cmp(&(settings.sound_effect_volume - right).abs())
        })
        .map_or(0, |(index, _)| index + 1)
}

pub(super) fn missing_microphone(ui: &Ui) -> Option<&str> {
    let saved = ui.settings.microphone.as_deref()?;
    let listed = ui.microphones.as_ref().ok()?;
    (!listed.devices.iter().any(|device| device.name == saved)).then_some(saved)
}

pub(super) fn microphone_choices(ui: &Ui) -> Vec<Choice> {
    let mut choices = Vec::new();
    let selected = ui.settings.microphone.clone();
    let listed = ui.microphones.as_ref().ok();
    choices.push(Choice {
        label: tr("System default").to_owned(),
        detail: listed.and_then(|devices| devices.system_default.clone()),
        selected: selected.is_none(),
        choose: Box::new(|ui: &mut Ui| {
            ui.update(|settings| settings.microphone = None);
            ui.open = None;
        }),
    });
    for device in listed
        .map(|devices| devices.devices.as_slice())
        .unwrap_or_default()
    {
        let name = device.name.clone();
        let choice_name = name.clone();
        choices.push(Choice {
            label: name,
            detail: None,
            selected: selected.as_deref() == Some(&choice_name),
            choose: Box::new(move |ui: &mut Ui| {
                let name = choice_name.clone();
                ui.update(|settings| settings.microphone = Some(name));
                ui.open = None;
            }),
        });
    }
    choices
}

/// The listener card above the basics, when the OS controls the listener.
pub(super) fn listener_card(ui: &Ui, p: Paint) -> (String, El) {
    let title = tr("Listening");
    let running = ui.view.as_ref().is_some_and(|view| view.listener.running);
    let copy = if running {
        tr("Listening")
    } else {
        tr("Disabled")
    };
    (
        title.to_owned(),
        super::controls::card(
            p,
            "listener",
            vec![icon_row(
                p,
                "listener-state",
                &super::icons::MIC,
                title,
                None,
                text(copy)
                    .key("listener-value")
                    .font(value_font(p))
                    .color(p.muted()),
            )],
        ),
    )
}

/// What a shortcuts portal granted, when it did; the fixture never does.
pub(super) fn granted(
    _ui: &Ui,
    _p: Paint,
    _target: ShortcutTarget,
) -> Option<(String, rune::Color)> {
    None
}

pub(super) fn gnome_clash_note(_ui: &Ui, _p: Paint, _target: ShortcutTarget) -> Option<El> {
    None
}

pub(super) fn refused_note(_ui: &Ui, _p: Paint, _target: ShortcutTarget) -> Option<El> {
    None
}

/// Where the desktop owns its shortcuts, the pane says how to bind them.
pub(super) fn desktop_commands(ui: &Ui, p: Paint) -> El {
    let _ = ui;
    super::controls::card(
        p,
        "desktop-shortcuts",
        vec![icon_row(
            p,
            "desktop-shortcuts-row",
            &super::icons::KEYBOARD,
            tr("Desktop shortcuts"),
            None,
            text(tr("Your desktop has no shortcuts portal. Bind these commands in its keyboard settings."))
                .key("desktop-shortcuts-value")
                .font(value_font(p))
                .color(p.muted()),
        )],
    )
}

/// Keyboard-language mappings: none in the fixture.
pub(super) fn input_source_rows(_ui: &Ui, _p: Paint, _layer: &mut Vec<El>) -> Vec<El> {
    Vec::new()
}

/// The speech model's state line beside its picker.
pub(super) fn transcription_status(ui: &Ui, p: Paint) -> Option<(String, rune::Color)> {
    use crate::transcription::models;
    let selection = &ui.settings.transcription;
    let model = models::definition(selection.model);
    Some((
        format!(
            "{} listens for {}",
            model.name,
            models::language_name(&selection.language)
        ),
        p.muted(),
    ))
}

pub(super) fn hotkey_key(target: ShortcutTarget) -> &'static str {
    match target {
        ShortcutTarget::Dictation => "dictation-shortcut",
        ShortcutTarget::PasteLast => "paste-last",
        ShortcutTarget::Capture => "annotate-shortcut",
        ShortcutTarget::RawDictation => "raw-dictation-shortcut",
        ShortcutTarget::RewriteSelection => "rewrite-shortcut",
        ShortcutTarget::TerminalPaste => "terminal-paste-shortcut",
        ShortcutTarget::Cancel => "cancel-shortcut",
    }
}

pub(super) fn shortcut_caps(binding: &HotkeyBinding, _side_picker: bool) -> Vec<String> {
    binding.keycaps()
}

pub(super) fn updates_itself(_ui: &Ui) -> bool {
    true
}

/// The shortcut's bound keys as a tray.
pub(super) fn keys_tray(p: Paint, key: &str, binding: &HotkeyBinding) -> El {
    let caps = shortcut_caps(binding, false);
    key_tray(
        p,
        key,
        crate::rune_ui::controls::KeySize::Small,
        caps.iter().enumerate().map(|(index, cap)| {
            keycap(
                p,
                index,
                cap,
                crate::rune_ui::controls::KeySize::Small,
                false,
            )
        }),
    )
}

/// A shortcut's clear button, used by the fixture's shortcut rows.
pub(super) fn clear_button(p: Paint, key: &'static str, target: ShortcutTarget) -> El {
    icon_button(
        p,
        key,
        &rune_theme_opencode::icons::TRASH,
        tr("Clear shortcut"),
        move |ui: &mut Ui| {
            ui.update(|settings| clear_binding(settings, target));
        },
    )
}

/// Unused in the fixture, but part of the real module's surface.
pub(super) fn hold(ui: &mut Ui, modifiers: HotkeyModifiers) {
    if !modifiers.is_empty() {
        let _ = ui;
    }
}

/// A shortcut row's card row type, for callers that build one directly.
pub(super) fn row_of(row: El) -> CardRow {
    CardRow::Shown(Box::new(row))
}

/// A secondary button that opens a shortcut's recorder, used by fixture rows.
pub(super) fn record_button(p: Paint, key: &'static str, target: ShortcutTarget) -> El {
    super::controls::button(
        p,
        key,
        tr("Record"),
        ButtonKind::Secondary,
        move |ui: &mut Ui, _| begin_capture(ui, target),
    )
}

/// The portal's granted shortcuts, if any.
pub(super) fn portal_shortcuts(ui: &Ui) -> Option<DesktopShortcuts> {
    ui.shortcuts()
}
