//! GPU health, fixture only.

pub(crate) struct Failure {
    pub(crate) slow: bool,
    pub(crate) device: String,
}

pub(crate) fn current() -> Option<Failure> {
    None
}
