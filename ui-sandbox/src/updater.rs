//! Update channels, fixture only.

use crate::app_settings::UpdateChannel;

/// The channel this build follows when the setting names none.
pub(crate) fn build_channel() -> UpdateChannel {
    UpdateChannel::Stable
}
