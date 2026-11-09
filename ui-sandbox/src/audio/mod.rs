//! Audio devices, fixture only: the General pane lists inputs and shows the
//! system default standing in for a missing saved one.

/// One input device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputDevice {
    pub name: String,
    pub id: String,
}

/// The inputs this machine has.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InputDevices {
    pub devices: Vec<InputDevice>,
    pub system_default: Option<String>,
    /// A MacBook's built-in microphone, off while the lid is closed.
    pub off_with_lid_closed: Option<String>,
    pub default_is_hex_microphone: bool,
    pub default_is_off_with_lid_closed: bool,
    /// A Bluetooth input passed over for a wired one, and the one used.
    pub bluetooth_default_passed_over: Option<String>,
}

pub(crate) fn input_devices() -> Result<InputDevices, String> {
    Ok(InputDevices {
        devices: vec![
            InputDevice {
                name: "Microphone (USB)".into(),
                id: "fixture-usb".into(),
            },
            InputDevice {
                name: "Built-in Microphone".into(),
                id: "fixture-built-in".into(),
            },
        ],
        system_default: Some("Microphone (USB)".into()),
        ..InputDevices::default()
    })
}

pub(crate) fn input_route_generation() -> u64 {
    0
}

pub const LID_CLOSED_ERROR: &str = "the lid is closed";
pub const NO_MICROPHONE_ERROR: &str = "no microphone";

pub mod input_level;
pub mod voice_filter;
