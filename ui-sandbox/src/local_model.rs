//! The local model server's state, fixture only: the desktop host carries an
//! optional snapshot.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum State {
    #[default]
    Stopped,
    Starting,
    Ready,
    External,
    Failed,
    Missing,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub state: State,
}
