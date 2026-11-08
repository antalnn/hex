//! The persisted `rewrite` settings: what Rewrite selection does when the
//! recording is a name rather than an instruction.

use super::*;

/// One name and the instruction it stands for. Saying the name alone while
/// text is selected uses `prompt`, so a phrase used every day costs two words.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct RewriteSnippet {
    /// What the person says, such as "structured prompt".
    pub name: String,
    /// What the model is told to do, such as "Turn it into a numbered prompt.".
    pub prompt: String,
}

/// Rewrite selection's own settings.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct RewriteSettings {
    /// Where this feature's model call goes: its own choice, independent of
    /// the dictation cleanup's.
    #[serde(flatten)]
    pub model: ModelSource,
    /// Names that stand for a prompt, matched against the whole recording.
    pub snippets: Vec<RewriteSnippet>,
    /// The prompt for a recording that said nothing (silence, or a release
    /// with no speech).
    pub default_prompt: String,
}

impl RewriteSettings {
    /// The snippets as (name, prompt) pairs, for
    /// [`crate::dictation::rewrite::resolve`].
    pub fn pairs(&self) -> Vec<(String, String)> {
        self.snippets
            .iter()
            .map(|snippet| (snippet.name.clone(), snippet.prompt.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snippet_round_trips() {
        let settings = RewriteSettings {
            model: ModelSource::default(),
            snippets: vec![RewriteSnippet {
                name: "structured prompt".into(),
                prompt: "Turn it into a prompt.".into(),
            }],
            default_prompt: "Tighten it.".into(),
        };
        let decoded: RewriteSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(decoded, settings);
    }

    #[test]
    fn an_old_file_loads_with_no_snippets_and_no_default() {
        let decoded: RewriteSettings = serde_json::from_str("{}").unwrap();
        assert!(decoded.snippets.is_empty());
        assert!(decoded.default_prompt.is_empty());
        assert_eq!(decoded.model.source, super::AiSource::Local);
    }

    #[test]
    fn the_model_source_shares_the_rewrite_prefix() {
        let settings = RewriteSettings {
            model: ModelSource {
                source: super::AiSource::Remote,
                endpoint: "https://api.example.com/v1".into(),
                model: "big-model".into(),
                api_key: "secret".into(),
            },
            snippets: Vec::new(),
            default_prompt: String::new(),
        };
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json["source"], "remote");
        assert_eq!(json["endpoint"], "https://api.example.com/v1");
        assert_eq!(json["model"], "big-model");
        assert_eq!(json["api_key"], "secret");
        let decoded: RewriteSettings = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, settings);
    }

    #[test]
    fn pairs_expose_the_name_and_the_prompt() {
        let settings = RewriteSettings {
            model: ModelSource::default(),
            snippets: vec![RewriteSnippet {
                name: "bullets".into(),
                prompt: "Make it a list.".into(),
            }],
            default_prompt: String::new(),
        };
        assert_eq!(
            settings.pairs(),
            vec![("bullets".to_string(), "Make it a list.".to_string())]
        );
    }
}
