//! The focused window's selected text, for the rewrite hotkey.
//!
//! Two sources, in order. UI Automation answers for the apps that expose a text
//! pattern (browsers, editors, WinUI); it needs no keyboard input and leaves the
//! clipboard alone. Everything else gets a clipboard round trip: snapshot,
//! Ctrl+C, read, put the previous clipboard back.
//!
//! The selection is refused before any model call when it is empty or longer
//! than [`MAX_SELECTION_CHARS`]: a 4096-token rewrite is not a document editor,
//! and a whole-file selection is almost certainly a mistaken key.

use std::time::Duration;

use color_eyre::eyre::{Result, eyre};

/// The longest selection the rewrite accepts. The local model's context is
/// 4096 tokens, so anything past this could not be rewritten whole anyway.
pub(crate) const MAX_SELECTION_CHARS: usize = 20_000;
/// How long the clipboard fallback waits for Ctrl+C to land.
const CLIPBOARD_WAIT: Duration = Duration::from_millis(300);
const CLIPBOARD_POLL: Duration = Duration::from_millis(10);

/// The selected text of the focused window.
pub(crate) fn read_selection() -> Result<String> {
    match uia_selection() {
        // The app answered with a text pattern, so an empty list is an empty
        // selection: a clipboard round trip would only add a keystroke.
        Some(ranges) => accept(assemble(&ranges)),
        None => accept(crate::windows::paste::read_copied_selection(
            CLIPBOARD_WAIT,
            CLIPBOARD_POLL,
        )?),
    }
}

/// Trims the selection and enforces the limits. Both sources end here, so the
/// errors say the same thing whichever one answered.
fn accept(text: String) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(eyre!("there is no selected text to rewrite"));
    }
    let characters = text.chars().count();
    if characters > MAX_SELECTION_CHARS {
        return Err(eyre!(
            "the selection is {characters} characters; select less than {MAX_SELECTION_CHARS}"
        ));
    }
    Ok(text.to_string())
}

/// One string for the ranges UIA reported, in order, separated by one newline.
/// Empty ranges are skipped so a caret the app also reports leaves no blank line.
fn assemble(ranges: &[String]) -> String {
    let mut text = String::new();
    for range in ranges {
        if range.trim().is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(range);
    }
    text
}

/// The focused element's text selection, when the app exposes the text
/// pattern. `None` means the app has no such pattern and the caller falls back
/// to the clipboard; `Some` with no ranges means nothing is selected.
fn uia_selection() -> Option<Vec<String>> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationTextPattern, UIA_TextPatternId,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    unsafe {
        if GetForegroundWindow().is_null() {
            return None;
        }
        let com = CoInitializeEx(None, COINIT_MULTITHREADED);
        let ranges = (|| -> windows::core::Result<Option<Vec<String>>> {
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            let focused = automation.GetFocusedElement()?;
            let Ok(pattern) =
                focused.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            else {
                return Ok(None);
            };
            let selection = pattern.GetSelection()?;
            let mut ranges = Vec::new();
            for index in 0..selection.Length()? {
                let Ok(range) = selection.GetElement(index) else {
                    continue;
                };
                let Ok(text) = range.GetText(-1) else {
                    continue;
                };
                ranges.push(text.to_string());
            }
            Ok(Some(ranges))
        })();
        if com.is_ok() {
            CoUninitialize();
        }
        ranges.ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_limit(characters: usize) -> String {
        "a".repeat(characters)
    }

    #[test]
    fn ranges_join_with_one_newline_and_empty_ones_are_skipped() {
        let ranges = vec![
            "first line".to_string(),
            "  ".to_string(),
            "second".to_string(),
        ];
        assert_eq!(assemble(&ranges), "first line\nsecond");
    }

    #[test]
    fn a_single_range_is_returned_as_it_is() {
        assert_eq!(assemble(&["only".to_string()]), "only");
    }

    #[test]
    fn an_empty_selection_is_an_error() {
        let error = accept(String::new()).unwrap_err();
        assert_eq!(error.to_string(), "there is no selected text to rewrite");
        assert!(accept(assemble(&[])).is_err());
    }

    #[test]
    fn a_whitespace_only_selection_is_an_error() {
        assert!(accept("  \r\n\t ".to_string()).is_err());
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(accept("  hello\n".to_string()).unwrap(), "hello");
    }

    #[test]
    fn the_cap_is_exact_and_one_character_past_it_is_refused() {
        assert_eq!(
            accept(at_limit(MAX_SELECTION_CHARS)).unwrap().len(),
            MAX_SELECTION_CHARS
        );
        let error = accept(at_limit(MAX_SELECTION_CHARS + 1)).unwrap_err();
        assert!(error.to_string().contains("select less than"), "{error}");
    }

    #[test]
    fn the_cap_counts_characters_not_bytes() {
        // One character over in characters is one over, however many bytes it
        // takes; the error must not report bytes.
        let text = "é".repeat(MAX_SELECTION_CHARS + 1);
        let error = accept(text).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with(&format!("the selection is {}", MAX_SELECTION_CHARS + 1)),
            "{error}"
        );
    }
}
