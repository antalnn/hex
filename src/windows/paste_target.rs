//! The window a dictation was started in.
//!
//! A dictation is usually started where its text belongs. When the person
//! switches windows before finishing (Alt-Tab to look something up, say), the
//! finishing keystroke lands in the other application, and today's paste
//! follows it. Keeping the handle captured at the start lets the transcript go
//! back to the window it was meant for.

use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
};

/// How long to wait for the window manager to hand the foreground back.
const FOREGROUND_WAIT: Duration = Duration::from_millis(250);
const FOREGROUND_POLL: Duration = Duration::from_millis(10);

/// The foreground window at the moment dictation started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PasteTarget(isize);

impl PasteTarget {
    /// The window in front now, or none when the desktop has none (a locked
    /// session, or the moment between two windows).
    pub(crate) fn capture() -> Option<Self> {
        // SAFETY: plain Win32 query.
        target_of(unsafe { GetForegroundWindow() } as isize)
    }

    /// Brings this window back to the front. `false` when the window is gone
    /// or the desktop refused the change, in which case the caller pastes
    /// into the current focus rather than losing the transcript.
    pub(crate) fn restore(self) -> bool {
        let target = self.0 as HWND;
        // SAFETY: plain Win32 query; `IsWindow` accepts any handle.
        if unsafe { IsWindow(target) } == 0 {
            return false;
        }
        if !should_restore(foreground(), self.0) {
            return true;
        }
        raise(target);
        if focus_is(target) {
            return true;
        }
        // Windows only lets the foreground process change the foreground
        // window, unless a thread that already has it shares its input queue
        // for a moment. Attaching is the documented way around that.
        attach_and_raise(target);
        wait_for_focus(target, FOREGROUND_WAIT)
    }
}

/// Whether a paste has to bring the target forward at all: only when the
/// foreground moved to another window. No foreground means a transition, and
/// the target is the best answer there.
fn should_restore(current: Option<isize>, target: isize) -> bool {
    current != Some(target)
}

/// A handle Windows reported, or none when it reported nothing. Every
/// `PasteTarget` comes through here, so a null handle is never carried to a
/// later paste.
fn target_of(window: isize) -> Option<PasteTarget> {
    (window != 0).then_some(PasteTarget(window))
}

/// The foreground window, or none when Windows reports no window.
fn foreground() -> Option<isize> {
    // SAFETY: plain Win32 query.
    let window = unsafe { GetForegroundWindow() };
    (!window.is_null()).then_some(window as isize)
}

fn focus_is(target: HWND) -> bool {
    foreground() == Some(target as isize)
}

fn raise(target: HWND) {
    // SAFETY: plain Win32 call on a handle `restore` checked with `IsWindow`.
    unsafe {
        SetForegroundWindow(target);
    }
}

/// One attempt to take the foreground by sharing the current owner's input
/// queue. Nothing is retried: a refusal falls back to the current focus.
fn attach_and_raise(target: HWND) {
    // SAFETY: plain Win32 calls on handles and thread ids Windows reports; the
    // detach mirrors a successful attach.
    unsafe {
        let current_window = GetForegroundWindow();
        if current_window.is_null() || current_window == target {
            return;
        }
        let owner = GetWindowThreadProcessId(current_window, std::ptr::null_mut());
        let this_thread = GetCurrentThreadId();
        if owner == 0 || owner == this_thread {
            return;
        }
        if AttachThreadInput(this_thread, owner, 1) == 0 {
            return;
        }
        SetForegroundWindow(target);
        AttachThreadInput(this_thread, owner, 0);
    }
}

fn wait_for_focus(target: HWND, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    loop {
        if focus_is(target) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(FOREGROUND_POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dictation_that_stayed_in_its_window_needs_no_restore() {
        assert!(!should_restore(Some(41), 41));
    }

    #[test]
    fn a_different_foreground_is_put_back() {
        assert!(should_restore(Some(72), 41));
    }

    #[test]
    fn no_foreground_at_all_is_put_back() {
        assert!(should_restore(None, 41));
    }

    #[test]
    fn a_captured_target_is_never_null() {
        assert_eq!(target_of(0), None);
        assert_eq!(target_of(41), Some(PasteTarget(41)));
    }

    #[test]
    fn capture_reports_a_target_or_nothing_and_never_moves_focus() {
        // The desktop decides what is in front; this proves the call
        // type-checks and stays quiet when there is no foreground window.
        if let Some(target) = PasteTarget::capture() {
            assert_ne!(target.0, 0);
        }
    }
}
