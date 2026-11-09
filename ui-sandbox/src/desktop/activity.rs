//! What the listener last did, fixture only: the snapshot type the desktop
//! host carries; the sandbox has no event log.

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct DesktopActivity {
    pub(crate) session_started_at: Option<u64>,
    pub(crate) device: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) last_failure: Option<(u64, String)>,
}
