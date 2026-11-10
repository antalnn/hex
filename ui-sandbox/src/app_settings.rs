//! The settings the General pane reads, fixture only: the same field names
//! and shapes as `src/app_settings.rs` in the hex checkout, with the
//! persistence and host side effects left out.

use serde::{Deserialize, Serialize};

use crate::dictation::ShortcutMode;
use crate::dictation::VocabularyEntry;
pub use crate::feedback::SoundSet;
use crate::history::HistoryRetention;
use crate::transcription::models::{TranscriptionModelId, TranscriptionSelection};

// ---------------------------------------------------------------------------
// Shortcuts

/// Which side of a modifier a shortcut needs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ModifierSide {
    Left,
    Either,
    Right,
    Both,
}

impl ModifierSide {
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Either => "Either",
            Self::Right => "Right",
            Self::Both => "Both",
        }
    }
}

/// The modifiers held with a key.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HotkeyModifiers {
    pub function: bool,
    pub control: Option<ModifierSide>,
    pub option: Option<ModifierSide>,
    pub shift: Option<ModifierSide>,
    pub command: Option<ModifierSide>,
}

impl HotkeyModifiers {
    pub fn control() -> Self {
        Self {
            control: Some(ModifierSide::Either),
            ..Self::default()
        }
    }

    pub fn control_option() -> Self {
        Self {
            control: Some(ModifierSide::Either),
            option: Some(ModifierSide::Either),
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.function
            && self.control.is_none()
            && self.option.is_none()
            && self.shift.is_none()
            && self.command.is_none()
    }

    pub fn count(&self) -> usize {
        usize::from(self.function)
            + [self.control, self.option, self.shift, self.command]
                .into_iter()
                .flatten()
                .count()
    }

    pub fn keycaps(&self) -> impl Iterator<Item = &'static str> {
        let mut caps: Vec<&'static str> = Vec::new();
        if self.function {
            caps.push("Fn");
        }
        for (side, name) in [
            (self.control, "Ctrl"),
            (self.option, "Alt"),
            (self.shift, "Shift"),
            (self.command, "Win"),
        ] {
            if let Some(side) = side {
                caps.push(match side {
                    ModifierSide::Left => match name {
                        "Ctrl" => "LCtrl",
                        "Alt" => "LAlt",
                        "Shift" => "LShift",
                        _ => "LWin",
                    },
                    ModifierSide::Right => match name {
                        "Ctrl" => "RCtrl",
                        "Alt" => "RAlt",
                        "Shift" => "RShift",
                        _ => "RWin",
                    },
                    _ => name,
                });
            }
        }
        caps.into_iter()
    }

    pub fn sided_keycaps(&self) -> impl Iterator<Item = (&'static str, ModifierSide)> {
        self.keycaps().map(|cap| (cap, ModifierSide::Either))
    }
}

/// One non-modifier key of a shortcut.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HotkeyKey {
    pub code: u16,
    pub label: String,
}

impl HotkeyKey {
    pub fn named(name: &str) -> Self {
        Self {
            code: 0,
            label: key_label(name),
        }
    }

    pub fn keycap(&self) -> String {
        self.label.clone()
    }

    pub fn letter(&self) -> Option<char> {
        let mut characters = self.label.chars();
        match (characters.next(), characters.next()) {
            (Some(letter), None) if letter.is_ascii_alphabetic() => Some(letter),
            _ => None,
        }
    }

    pub fn mouse_number(label: &str) -> Option<u8> {
        label.strip_prefix("Mouse ")?.parse().ok()
    }

    pub fn same_key(&self, other: &Self) -> bool {
        self.code == other.code && self.label.eq_ignore_ascii_case(&other.label)
    }
}

fn key_label(name: &str) -> String {
    match name {
        "space" => "Space".into(),
        "escape" => "Esc".into(),
        other if other.chars().count() == 1 => other.to_uppercase(),
        other => other.to_string(),
    }
}

/// A shortcut as a chord.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HotkeyBinding {
    pub modifiers: HotkeyModifiers,
    pub key: Option<HotkeyKey>,
}

impl Default for HotkeyBinding {
    fn default() -> Self {
        Self::chord(HotkeyModifiers::control(), "space")
    }
}

impl HotkeyBinding {
    pub fn chord(modifiers: HotkeyModifiers, key: &str) -> Self {
        Self {
            modifiers,
            key: Some(HotkeyKey::named(key)),
        }
    }

    pub fn paste_last_default() -> Self {
        Self::chord(HotkeyModifiers::control_option(), "v")
    }

    pub fn cancel_default() -> Self {
        Self::chord(HotkeyModifiers::default(), "escape")
    }

    pub fn keycaps(&self) -> Vec<String> {
        self.modifiers
            .keycaps()
            .map(str::to_string)
            .chain(self.key.iter().map(HotkeyKey::keycap))
            .collect()
    }

    pub fn sided_keycaps(&self) -> Vec<(String, ModifierSide)> {
        self.modifiers
            .sided_keycaps()
            .map(|(label, side)| (label.to_string(), side))
            .chain(
                self.key
                    .iter()
                    .map(|key| (key.keycap(), ModifierSide::Either)),
            )
            .collect()
    }

    pub fn is_bare_primary_click(&self) -> bool {
        false
    }
}

/// Which shortcuts are saved.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Shortcuts {
    pub excluded_processes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Settings

/// Where the dictation indicator hugs the screen.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum IndicatorPosition {
    #[default]
    Top,
    Bottom,
    Hidden,
}

/// What Hex does with other audio while recording.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RecordingAudioBehavior {
    #[default]
    PauseMedia,
    Duck,
    Keep,
}

impl RecordingAudioBehavior {
    pub const ALL: [Self; 3] = [Self::PauseMedia, Self::Duck, Self::Keep];

    pub fn label(self) -> &'static str {
        match self {
            Self::PauseMedia => "Pause media",
            Self::Duck => "Lower the volume",
            Self::Keep => "Leave it playing",
        }
    }
}

/// Where the speech model runs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TranscriptionDevice {
    #[default]
    Auto,
    Gpu,
    Cpu,
}

/// The update channel a build follows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateChannel {
    Stable,
    Dev,
}

impl UpdateChannel {
    pub const ALL: [Self; 2] = [Self::Stable, Self::Dev];

    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::Dev => "Dev",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Updates {
    pub channel: Option<UpdateChannel>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LabsSettings {
    pub settings_search: bool,
    pub restore_shortcut_default: bool,
    pub second_dictation_shortcut: bool,
    pub terminal_paste_shortcut: bool,
    pub gnome_clipboard_restore: bool,
    pub vocabulary_biasing: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KeyboardModelSwitching {
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DictationProcessing {
    pub paste_to_start_window: bool,
    pub wrap_code_terms: bool,
}

/// The local model server (`llm`), fixture only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmSettings {
    pub enabled: bool,
    pub port: u16,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 8081,
        }
    }
}

/// Which endpoint a model call uses.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AiSource {
    #[default]
    Local,
    Remote,
}

/// A remote model call.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RemoteModel {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
}

impl RemoteModel {
    pub fn missing(&self) -> Option<&'static str> {
        let endpoint = self.endpoint.trim();
        if endpoint.is_empty() {
            return Some("the remote endpoint address is empty");
        }
        if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
            return Some("the remote endpoint address must start with http:// or https://");
        }
        if self.model.trim().is_empty() {
            return Some("the remote model name is empty");
        }
        None
    }
}

/// One feature's model call.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelSource {
    pub source: AiSource,
    pub remote: RemoteModel,
}

impl ModelSource {
    pub fn missing(&self) -> Option<&'static str> {
        if self.source == AiSource::Remote {
            self.remote.missing()
        } else {
            None
        }
    }
}

/// One named instruction Rewrite selection can use.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RewriteSnippet {
    pub name: String,
    pub prompt: String,
}

/// Rewrite selection's settings.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RewriteSettings {
    pub remote: RemoteModel,
    pub snippets: Vec<RewriteSnippet>,
    pub default_prompt: String,
}

/// AI cleanup's settings; the shipped wording is a fixture stand-in.
pub const CLEANUP_PROMPT: &str =
    "Clean up dictated speech: fix grammar, punctuation and paragraphs.";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AiCleanupSettings {
    pub enabled: bool,
    pub prompt: String,
    pub deadline_seconds: u64,
}

impl Default for AiCleanupSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            prompt: CLEANUP_PROMPT.into(),
            deadline_seconds: 20,
        }
    }
}

impl AiCleanupSettings {
    pub fn instruction(&self) -> &str {
        let saved = self.prompt.trim();
        if saved.is_empty() {
            CLEANUP_PROMPT
        } else {
            saved
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CaptureSettings {
    pub enabled: bool,
}

/// The whole settings file, as the pane reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct AppSettings {
    pub microphone: Option<String>,
    pub release_microphone_while_idle: bool,
    pub pause_browser_media: bool,
    pub avoid_bluetooth_microphone: bool,
    pub shortcut_mode: ShortcutMode,
    pub double_tap_lock: bool,
    pub double_tap_only: bool,
    pub dictation_hotkey: HotkeyBinding,
    pub paste_last_hotkey: HotkeyBinding,
    pub cancel_hotkey: HotkeyBinding,
    pub rewrite_hotkey: HotkeyBinding,
    pub raw_dictation_hotkey: Option<HotkeyBinding>,
    pub transcription: TranscriptionSelection,
    pub transcription_device: TranscriptionDevice,
    pub keyboard_model_switching: KeyboardModelSwitching,
    pub dictation_processing: DictationProcessing,
    pub llm: LlmSettings,
    pub dictation_rewrite: ModelSource,
    pub rewrite: RewriteSettings,
    pub ai_cleanup: AiCleanupSettings,
    pub recording_audio_behavior: RecordingAudioBehavior,
    pub recording_duck_volume: f32,
    pub indicator_position: IndicatorPosition,
    pub paste_with_shift: bool,
    pub enable_on_launch: bool,
    pub show_dock_icon: bool,
    pub sound_effects: bool,
    pub sound_effect_volume: f32,
    pub sound_theme: SoundSet,
    pub ui_language: Option<String>,
    pub history_retention: HistoryRetention,
    pub updates: Updates,
    pub labs: LabsSettings,
    pub capture: CaptureSettings,
    pub vocabulary: Vec<VocabularyEntry>,
    pub shortcuts: Shortcuts,
    pub rewrite_snippet_match: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            microphone: None,
            release_microphone_while_idle: false,
            pause_browser_media: true,
            avoid_bluetooth_microphone: true,
            shortcut_mode: ShortcutMode::Hold,
            double_tap_lock: true,
            double_tap_only: false,
            dictation_hotkey: HotkeyBinding::default(),
            paste_last_hotkey: HotkeyBinding::paste_last_default(),
            cancel_hotkey: HotkeyBinding::cancel_default(),
            rewrite_hotkey: HotkeyBinding::chord(HotkeyModifiers::control(), "r"),
            raw_dictation_hotkey: None,
            transcription: TranscriptionSelection::new(),
            transcription_device: TranscriptionDevice::Auto,
            keyboard_model_switching: KeyboardModelSwitching::default(),
            dictation_processing: DictationProcessing::default(),
            llm: LlmSettings::default(),
            dictation_rewrite: ModelSource::default(),
            rewrite: RewriteSettings::default(),
            ai_cleanup: AiCleanupSettings::default(),
            recording_audio_behavior: RecordingAudioBehavior::default(),
            recording_duck_volume: 0.25,
            indicator_position: IndicatorPosition::default(),
            paste_with_shift: false,
            enable_on_launch: false,
            show_dock_icon: true,
            sound_effects: true,
            sound_effect_volume: 0.5,
            sound_theme: SoundSet::default(),
            ui_language: None,
            history_retention: HistoryRetention::default(),
            updates: Updates::default(),
            labs: LabsSettings::default(),
            capture: CaptureSettings::default(),
            vocabulary: Vec::new(),
            shortcuts: Shortcuts::default(),
            rewrite_snippet_match: false,
        }
    }
}

impl AppSettings {
    /// Whether the dictation shortcut is known to be installed (fixture).
    pub(crate) fn model(&self) -> TranscriptionModelId {
        self.transcription.model
    }
}
