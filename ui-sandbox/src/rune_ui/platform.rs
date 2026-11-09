//! The OS's capabilities and its only-state, fixture only.

use crate::desktop::login_item::{LoginItemResult, LoginItemStatus};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Os {
    #[default]
    Windows,
    Macos,
    Linux,
}

/// Which panes and rows each OS shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Capabilities {
    pub(crate) os: Os,
    pub(crate) meetings: bool,
    pub(crate) calendar: bool,
    pub(crate) permissions: bool,
    pub(crate) launch_at_login: bool,
    pub(crate) dock_icon: bool,
    pub(crate) listener_control: bool,
    pub(crate) terminal_paste: bool,
    pub(crate) share_shortcut: bool,
    pub(crate) hex_microphone: bool,
    pub(crate) practice_dictation: bool,
    pub(crate) fun: bool,
    pub(crate) modifier_sides: bool,
    pub(crate) keyboard_model_switching: bool,
    pub(crate) typed_text: bool,
}

impl Capabilities {
    pub(crate) const WINDOWS: Self = Self {
        os: Os::Windows,
        meetings: true,
        calendar: false,
        permissions: false,
        launch_at_login: true,
        dock_icon: false,
        listener_control: true,
        terminal_paste: false,
        share_shortcut: true,
        hex_microphone: false,
        practice_dictation: true,
        fun: false,
        modifier_sides: true,
        keyboard_model_switching: true,
        typed_text: false,
    };
}

/// The OS-only state the panes show.
#[derive(Clone, Debug)]
pub(crate) struct PlatformState {
    pub(crate) caps: Capabilities,
    pub(crate) input_sources: Vec<crate::transcription::keyboard_model_switching::InputSource>,
    pub(crate) permissions_asked: Vec<crate::desktop::onboarding::PermissionKind>,
    pub(crate) system_audio: bool,
    pub(crate) login_item: Option<LoginItemStatus>,
    pub(crate) login_item_checking: bool,
    pub(crate) login_item_error: Option<String>,
    pub(crate) login_item_requested: Option<bool>,
    pub(crate) clipboard_unrestorable: bool,
    pub(crate) developer: bool,
}

impl PlatformState {
    pub(crate) fn new(caps: Capabilities) -> Self {
        Self {
            caps,
            input_sources: Vec::new(),
            permissions_asked: Vec::new(),
            system_audio: false,
            login_item: Some(LoginItemStatus::Disabled),
            login_item_checking: false,
            login_item_error: None,
            login_item_requested: None,
            clipboard_unrestorable: false,
            developer: true,
        }
    }

    pub(crate) fn asked(&self, kind: crate::desktop::onboarding::PermissionKind) -> bool {
        self.permissions_asked.contains(&kind)
    }

    pub(crate) fn note_asked(&mut self, kind: crate::desktop::onboarding::PermissionKind) {
        if !self.permissions_asked.contains(&kind) {
            self.permissions_asked.push(kind);
        }
    }

    pub(crate) fn clear_stale(&mut self, _kind: crate::desktop::onboarding::PermissionKind) {}

    pub(crate) fn login_item_on(&self) -> bool {
        self.login_item_requested.unwrap_or(matches!(
            self.login_item,
            Some(LoginItemStatus::Enabled | LoginItemStatus::RequiresApproval)
        ))
    }

    pub(crate) fn set_login_item(&mut self, result: Option<LoginItemResult>) {
        match result {
            Some(Ok(status)) => {
                self.login_item = Some(status);
                self.login_item_error = None;
            }
            Some(Err(failure)) => {
                if let Some(status) = failure.status {
                    self.login_item = Some(status);
                }
                self.login_item_error = Some(failure.message);
            }
            None => self.login_item = None,
        }
        self.login_item_requested = None;
    }

    pub(crate) fn request_login_item(&mut self, enabled: bool) {
        self.login_item_requested = Some(enabled);
    }
}

/// What the panes ask the OS for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PlatformRequest {
    Permission(crate::desktop::onboarding::PermissionAction),
    SystemAudio,
    SetLaunchAtLogin(bool),
    OpenLoginItemSettings,
}
