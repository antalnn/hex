//! The window questions the shortcut guard asks: whether the app in front
//! fills its monitor. Which app it is comes from
//! [`crate::windows::input::foreground_process_stem`].

/// Whether the foreground window covers its monitor. A borderless game covers
/// the screen without being maximized, so the maximized flag alone is not
/// enough.
pub(crate) fn is_foreground_fullscreen() -> bool {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect};

    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return false;
        }
        let mut rect = RECT::default();
        if GetWindowRect(window, &mut rect) == 0 {
            return false;
        }
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..MONITORINFO::default()
        };
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return false;
        }
        covers_monitor(
            (rect.left, rect.top, rect.right, rect.bottom),
            (
                info.rcMonitor.left,
                info.rcMonitor.top,
                info.rcMonitor.right,
                info.rcMonitor.bottom,
            ),
        )
    }
}

/// Whether `window` covers `monitor`, two pixels of shadow allowed.
pub(crate) fn covers_monitor(window: (i32, i32, i32, i32), monitor: (i32, i32, i32, i32)) -> bool {
    const SLOP: i32 = 2;
    window.0 <= monitor.0 + SLOP
        && window.1 <= monitor.1 + SLOP
        && window.2 >= monitor.2 - SLOP
        && window.3 >= monitor.3 - SLOP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_that_covers_its_monitor_is_fullscreen() {
        assert!(covers_monitor((0, 0, 1920, 1080), (0, 0, 1920, 1080)));
        // A shadow may hang a pixel or two over the edge.
        assert!(covers_monitor((-1, -1, 1921, 1081), (0, 0, 1920, 1080)));
        assert!(!covers_monitor((100, 100, 1000, 700), (0, 0, 1920, 1080)));
        // Ten pixels short of the bottom is a window, not fullscreen.
        assert!(!covers_monitor((0, 0, 1920, 1070), (0, 0, 1920, 1080)));
    }
}
