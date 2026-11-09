//! Launch at login state, fixture only.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginItemStatus {
    Disabled,
    Enabled,
    RequiresApproval,
}

#[derive(Debug)]
pub struct LoginItemFailure {
    pub status: Option<LoginItemStatus>,
    pub message: String,
}

pub type LoginItemResult = Result<LoginItemStatus, LoginItemFailure>;
