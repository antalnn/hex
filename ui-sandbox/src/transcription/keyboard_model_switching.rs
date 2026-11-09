//! Keyboard input sources, fixture only.

/// One keyboard input source and the speech model it switches to.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InputSource {
    pub name: String,
    pub language: String,
}
