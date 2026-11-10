//! The settings window's fixture shell: the pieces `general.rs` expects from
//! the real `src/rune_ui.rs`, with a fixture service instead of the host.

use rune::{Cx, El, TextEdit, col, row, text};

use crate::app_settings::AppSettings;
use crate::desktop::host::{
    DesktopAction, DesktopListenerSnapshot, DesktopShortcuts, DesktopTranscriptionSnapshot,
    DesktopUpdateStatus,
};
use crate::desktop::i18n::tr;

mod ai_models;
pub(crate) mod controls;
mod explainer;
mod general;
mod icons;
mod labs;
mod list_detail;
mod lists;
mod local_models;
mod model_import;
mod onboarding;
mod picker;
mod platform;
mod platform_settings;
mod settings;
mod shortcut;
mod sound_sets;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod offscreen;
mod vocabulary;

use controls::Paint;
use platform::{Capabilities, Os, PlatformRequest, PlatformState};
pub(crate) use rune::ScrollSpec;
use settings::Capture;

/// One pane of the window. The sandbox carries General and the Models cards.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Pane {
    General,
    Models,
}

impl Pane {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::General => tr("General"),
            Self::Models => tr("Models"),
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::General => "nav-general",
            Self::Models => "nav-models",
        }
    }
}

/// The one anchored menu or popover that is open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Menu {
    Microphone,
    /// A settings row's quiet select, by the row's key.
    Pick(&'static str),
}

/// A shortcut the listener registers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ShortcutTarget {
    #[default]
    Dictation,
    PasteLast,
    Capture,
    RawDictation,
    RewriteSelection,
    TerminalPaste,
    Cancel,
}

/// What the window asks the host for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Request {
    Desktop(DesktopAction),
    ChooseModel {
        model: crate::transcription::models::TranscriptionModelId,
        language: String,
    },
    SetLaunchAtLogin(bool),
    OpenLoginItemSettings,
    SystemAudio,
}

/// Where the window sends its requests.
pub(crate) enum Service {
    /// Snapshots and tests: requests are recorded, never sent.
    Fixture(Vec<Request>),
}

/// The host's latest state.
#[derive(Clone, Debug)]
pub(crate) struct ServiceView {
    pub(crate) listener: DesktopListenerSnapshot,
    pub(crate) transcription: DesktopTranscriptionSnapshot,
    pub(crate) update_status: DesktopUpdateStatus,
    pub(crate) local_model: Option<crate::local_model::Snapshot>,
}

/// The sandbox's whole window state.
pub(crate) struct Ui {
    pub(crate) settings: AppSettings,
    pub(crate) model_sources: ai_models::ModelSources,
    service: Service,
    pub(crate) window: Option<rune::AppHandle<Ui>>,
    pub(crate) view: Option<ServiceView>,
    pub(crate) error: Option<String>,
    pub(crate) pane: Pane,
    pub(crate) open: Option<Menu>,
    pub(crate) microphones: Result<crate::audio::InputDevices, String>,
    pub(crate) mic_level: general::MicLevel,
    pub(crate) general_search: TextEdit,
    pub(crate) advanced: Vec<Pane>,
    pub(crate) labs_open: bool,
    pub(crate) capture: Capture,
    pub(crate) vocabulary: vocabulary::VocabularyEditor,
    pub(crate) picker: picker::Picker,
    pub(crate) explainer: explainer::Explainer,
    pub(crate) sound_sets: sound_sets::SoundSets,
    pub(crate) tour: Option<()>,
    pub(crate) platform: PlatformState,
    pub(crate) duck_volume_drag: Option<f32>,
    pub(crate) saved: usize,
}

impl Default for Ui {
    fn default() -> Self {
        Self::fixture()
    }
}

impl Ui {
    /// The state a headless render starts from: default settings and a
    /// fixture service, built without touching a data folder.
    pub(crate) fn fixture() -> Self {
        Self::with(AppSettings::default(), Service::Fixture(Vec::new()))
    }

    pub(crate) fn with(settings: AppSettings, service: Service) -> Self {
        let model_sources = ai_models::ModelSources::new(&settings);
        Self {
            settings,
            model_sources,
            service,
            window: None,
            view: None,
            error: None,
            pane: Pane::General,
            open: None,
            microphones: crate::audio::input_devices(),
            mic_level: general::MicLevel::default(),
            general_search: TextEdit::default(),
            advanced: Vec::new(),
            labs_open: false,
            capture: Capture::Idle,
            vocabulary: vocabulary::VocabularyEditor::default(),
            picker: picker::Picker::default(),
            explainer: explainer::Explainer::default(),
            sound_sets: sound_sets::SoundSets::default(),
            tour: None,
            platform: PlatformState::new(Capabilities::WINDOWS),
            duck_volume_drag: None,
            saved: 0,
        }
    }

    pub(crate) fn live(&self) -> bool {
        false
    }

    pub(crate) fn send(&mut self, request: Request) {
        if let Service::Fixture(sent) = &mut self.service {
            sent.push(request);
        }
    }

    /// Saves settings through the fixture host, which adopts the change.
    pub(crate) fn update(&mut self, change: impl FnOnce(&mut AppSettings)) -> bool {
        let mut candidate = self.settings.clone();
        change(&mut candidate);
        self.model_sources.project(&mut candidate);
        self.settings = candidate;
        self.saved += 1;
        true
    }

    /// Writes the typed Models fields into the settings, as the app's draft
    /// flush does.
    pub(crate) fn flush_drafts(&mut self) {
        let mut candidate = self.settings.clone();
        self.model_sources.project(&mut candidate);
        if candidate != self.settings {
            self.settings = candidate;
            self.saved += 1;
        }
    }

    /// Asks the OS host for something only it can do.
    pub(crate) fn ask(&mut self, request: PlatformRequest) {
        match request {
            PlatformRequest::SetLaunchAtLogin(enabled) => {
                self.platform.set_login_item(Some(Ok(if enabled {
                    crate::desktop::login_item::LoginItemStatus::Enabled
                } else {
                    crate::desktop::login_item::LoginItemStatus::Disabled
                })));
            }
            PlatformRequest::SystemAudio => self.platform.system_audio = true,
            PlatformRequest::Permission(_) => {}
            _ => {}
        }
        self.send(match request {
            PlatformRequest::SetLaunchAtLogin(enabled) => Request::SetLaunchAtLogin(enabled),
            PlatformRequest::OpenLoginItemSettings => Request::OpenLoginItemSettings,
            PlatformRequest::SystemAudio => Request::SystemAudio,
            PlatformRequest::Permission(_) => return,
        });
    }

    /// Whether this OS records shortcuts in Hex (Windows does).
    pub(crate) fn records_shortcuts(&self) -> bool {
        self.platform.caps.os == Os::Windows
    }

    pub(crate) fn shortcuts(&self) -> Option<DesktopShortcuts> {
        self.view
            .as_ref()
            .map(|view| view.listener.shortcuts.clone())
    }

    pub(crate) fn request_count(&self) -> usize {
        match &self.service {
            Service::Fixture(sent) => sent.len(),
        }
    }

    pub(crate) fn poll(&mut self, _now: f64, _tx: &mut rune::Tx) -> bool {
        false
    }

    pub(crate) fn on_key(&mut self, event: &rune::KeyEvent, _tx: &mut rune::Tx) -> bool {
        general::focus_search_key(self, event)
    }
}

/// A row the organization manages says so instead of its control.
fn managed_note() -> &'static str {
    tr("Managed by your organization")
}

/// The pane the window shows, in the same panel the app draws.
pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let mut layer = Vec::new();
    let body = match ui.pane {
        Pane::General => general::pane(ui, p, &mut layer),
        Pane::Models => {
            let (mut extra, body) = ai_models::pane(ui, p, &mut layer);
            layer.append(&mut extra);
            if let Some(modal) = ai_models::cleanup_modal(ui, p) {
                layer.extend(modal);
            }
            if let Some(modal) = ai_models::prompts_modal(ui, p) {
                layer.extend(modal);
            }
            body
        }
    };
    let switch = controls::segmented(
        p,
        "sandbox-pane",
        &[tr("General"), tr("Models")],
        usize::from(ui.pane == Pane::Models),
        |ui: &mut Ui, index| {
            ui.pane = if index == 0 {
                Pane::General
            } else {
                Pane::Models
            };
        },
    );
    let panel = col()
        .key("panel")
        .role("main")
        .grow(1.0)
        .shrink()
        .mt(8.0)
        .mr(8.0)
        .mb(8.0)
        .radius(p.k.radius.lg)
        .fill(p.k.c.bg_base)
        .border(p.hair, p.k.c.border_weak)
        .padding(p.hair)
        .clip_soft(0.0)
        .child(
            row()
                .key("pane-header")
                .h(52.0)
                .px(controls::METRICS.gutter)
                .center_y()
                .gap(12.0)
                .child(
                    text(ui.pane.title())
                        .key("pane-title")
                        .role("heading")
                        .font(p.k.ty.body_medium),
                )
                .child(rune::el().key("spacer").grow(1.0))
                .child(switch),
        )
        .child(body);
    let _ = &mut layer;
    row()
        .key("window")
        .role("none")
        .fill(p.k.c.bg_deep)
        .size(980.0, 760.0)
        .child(panel)
        .children(layer)
}

const SCROLL_FADE: (f32, f32) = (4.0, 24.0);

pub(crate) fn scroll_body(k: &rune_kit::Kit, key: &str, content: El) -> El {
    faded_body(k, key, content, SCROLL_FADE)
}

/// A scroll body with its own top and bottom fades.
fn faded_body(k: &rune_kit::Kit, key: &str, content: El, fades: (f32, f32)) -> El {
    list_detail::faded_body_sticky(k, key, content, fades, false)
}
