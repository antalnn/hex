//! Wall-clock helpers for the fixture host.

/// Milliseconds since the Unix epoch.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// How long History keeps dictations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HistoryRetention {
    #[default]
    Forever,
}
