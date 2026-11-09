//! Permission kinds and setup progress, fixture only.

/// A permission Hex asks the OS for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionKind {
    Microphone,
    Accessibility,
    SystemAudio,
    ScreenRecording,
}

/// What a permission row can do when allowed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionAction {
    RequestMicrophone,
    ShowGuide(PermissionKind),
    ResetAndPrompt(PermissionKind),
}

/// A permission that stays untrusted after the guide.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionWarning {
    pub kind: PermissionKind,
    pub stale: bool,
}

/// A missing speech model's download.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelProgress {
    pub downloading: Option<f32>,
    pub received: u64,
}
