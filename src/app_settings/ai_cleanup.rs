//! The persisted `ai_cleanup` settings: whether the native `ai-rewrite` step
//! runs, and the instruction it gives the model.
//!
//! The step id, pipeline order and trust behaviour are unchanged; these are
//! the user's on/off and prompt. A settings file written before this group
//! existed loads with cleanup on and the shipped wording, so the behavior it
//! had is kept.

use super::*;

/// AI cleanup's own settings.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct AiCleanupSettings {
    /// Run the native `ai-rewrite` step on each dictation.
    pub enabled: bool,
    /// The instruction the model receives. Empty uses the shipped wording, so
    /// clearing the box never degrades silently.
    pub prompt: String,
    /// How long one model call may wait before the step gives up and keeps
    /// the text. One second is the floor.
    pub deadline_seconds: u64,
}

impl Default for AiCleanupSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            prompt: crate::dictation::ai_rewrite::SYSTEM_PROMPT.to_string(),
            deadline_seconds: crate::dictation::ai_rewrite::DEFAULT_TIMEOUT.as_secs(),
        }
    }
}

impl AiCleanupSettings {
    /// The instruction the model receives: the saved prompt, or the shipped
    /// wording when the saved one is empty.
    pub fn instruction(&self) -> &str {
        let saved = self.prompt.trim();
        if saved.is_empty() {
            crate::dictation::ai_rewrite::SYSTEM_PROMPT
        } else {
            saved
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_cleanup_defaults_to_on_with_the_shipped_prompt() {
        let settings = AiCleanupSettings::default();
        assert!(settings.enabled);
        assert_eq!(
            settings.instruction(),
            crate::dictation::ai_rewrite::SYSTEM_PROMPT
        );
    }

    #[test]
    fn a_saved_prompt_replaces_the_shipped_one() {
        let settings = AiCleanupSettings {
            prompt: "  Fix the words.  ".into(),
            ..AiCleanupSettings::default()
        };
        assert_eq!(settings.instruction(), "Fix the words.");
    }

    #[test]
    fn an_empty_prompt_falls_back_to_the_shipped_wording() {
        let settings = AiCleanupSettings {
            prompt: "   ".into(),
            ..AiCleanupSettings::default()
        };
        assert_eq!(
            settings.instruction(),
            crate::dictation::ai_rewrite::SYSTEM_PROMPT
        );
    }

    #[test]
    fn an_old_file_loads_with_cleanup_on_and_the_shipped_prompt() {
        let decoded: AiCleanupSettings = serde_json::from_str("{}").unwrap();
        assert!(decoded.enabled);
        assert_eq!(
            decoded.instruction(),
            crate::dictation::ai_rewrite::SYSTEM_PROMPT
        );
        assert_eq!(
            decoded.deadline_seconds,
            crate::dictation::ai_rewrite::DEFAULT_TIMEOUT.as_secs()
        );
    }

    #[test]
    fn a_saved_deadline_replaces_the_default() {
        let decoded: AiCleanupSettings =
            serde_json::from_str(r#"{"deadline_seconds":45}"#).unwrap();
        assert_eq!(decoded.deadline_seconds, 45);
    }
}
