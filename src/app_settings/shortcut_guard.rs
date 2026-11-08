//! `shortcuts.*`: the apps in which Hex's shortcuts stay out of the way.
//!
//! A game reads `Control+Space` as its own key and loses it to Hex's hook, and
//! a dictation started by accident inside one is never wanted. The exclusion
//! list names the executables; the fullscreen switch is the opt-in fallback
//! for a game whose window the list does not know.

use super::*;

/// Where the shortcuts are paused.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ShortcutGuard {
    /// Executable stems, lowercased and without `.exe`, in which no Hex
    /// shortcut fires: `["cs2", "eldenring"]`.
    pub excluded_processes: Vec<String>,
    /// Also stay quiet in a window that covers its whole monitor.
    pub disable_in_fullscreen: bool,
}

impl ShortcutGuard {
    /// Whether `stem`, an executable stem already lowercased, is excluded.
    pub fn excludes(&self, stem: &str) -> bool {
        let stem = stem.trim().to_ascii_lowercase();
        self.excluded_processes.iter().any(|entry| *entry == stem)
    }

    /// One entry as the list stores it: trimmed, lowercased, `.exe` gone.
    /// None when nothing is left, so an empty row is not stored.
    pub fn cleaned(entry: &str) -> Option<String> {
        let entry = entry.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
        let entry = entry.strip_suffix(".exe").unwrap_or(&entry).trim();
        (!entry.is_empty()).then(|| entry.to_string())
    }

    /// The list with every entry cleaned and the duplicates dropped, keeping
    /// the order a person typed it in.
    pub fn clean(entries: &[String]) -> Vec<String> {
        let mut kept: Vec<String> = Vec::new();
        for entry in entries {
            if let Some(entry) = Self::cleaned(entry)
                && !kept.contains(&entry)
            {
                kept.push(entry);
            }
        }
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_and_the_switch_round_trip() {
        for excluded in [Vec::new(), vec!["cs2".to_string(), "eldenring".to_string()]] {
            for disable_in_fullscreen in [false, true] {
                let guard = ShortcutGuard {
                    excluded_processes: excluded.clone(),
                    disable_in_fullscreen,
                };
                let decoded: ShortcutGuard =
                    serde_json::from_str(&serde_json::to_string(&guard).unwrap()).unwrap();
                assert_eq!(decoded, guard);
            }
        }
    }

    #[test]
    fn an_old_file_with_neither_key_loads_as_empty_and_off() {
        let decoded: ShortcutGuard = serde_json::from_str("{}").unwrap();
        assert!(decoded.excluded_processes.is_empty());
        assert!(!decoded.disable_in_fullscreen);
    }

    #[test]
    fn a_stem_is_matched_without_case_or_suffix() {
        let guard = ShortcutGuard {
            excluded_processes: vec!["cs2".to_string()],
            disable_in_fullscreen: false,
        };
        assert!(guard.excludes("cs2"));
        assert!(guard.excludes("CS2"));
        assert!(!guard.excludes("csgo"));
    }

    #[test]
    fn an_entry_loses_its_path_suffix_quotes_and_duplicates() {
        assert_eq!(ShortcutGuard::cleaned("  CS2.EXE "), Some("cs2".into()));
        assert_eq!(
            ShortcutGuard::cleaned("\"EldenRing.exe\""),
            Some("eldenring".into())
        );
        assert_eq!(ShortcutGuard::cleaned("   "), None);
        assert_eq!(
            ShortcutGuard::clean(&["cs2".into(), " CS2.exe ".into(), "eldenring".into(),]),
            vec!["cs2".to_string(), "eldenring".to_string()]
        );
    }
}
