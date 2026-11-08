//! Where a model call goes: Hex's own llama.cpp server, or an
//! OpenAI-compatible endpoint elsewhere.
//!
//! Dictation cleanup and Rewrite selection each carry one of these, so one can
//! stay on the local server while the other uses a larger model. "Remote"
//! covers a hosted provider and a server on the network alike: both are an
//! address, a model name, and optionally a bearer token.

use super::*;

/// Which endpoint a model call uses.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AiSource {
    /// Hex's own llama.cpp server, on `llm.port`.
    #[default]
    Local,
    /// An OpenAI-compatible endpoint elsewhere. Its address, model and key are
    /// on the same struct.
    Remote,
}

impl AiSource {
    /// Every source, in the order the Models pane shows them.
    pub const ALL: [Self; 2] = [Self::Local, Self::Remote];

    /// The name the tab and `hex settings` show.
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "Local",
            Self::Remote => "Remote",
        }
    }

    /// The value `hex settings set` takes.
    pub fn key(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

/// One feature's model call: where it goes, and what a remote call needs.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ModelSource {
    /// Local or remote. Defaults to local, so a settings file written before
    /// this key existed keeps the behaviour it had.
    pub source: AiSource,
    /// The remote base address, such as `https://api.example.com/v1`. A bare
    /// host works too: `/v1/chat/completions` is appended.
    pub endpoint: String,
    /// The model name the remote endpoint expects.
    pub model: String,
    /// A bearer token, when the endpoint wants one. Empty sends no header.
    pub api_key: String,
}

impl ModelSource {
    pub fn is_remote(&self) -> bool {
        self.source == AiSource::Remote
    }

    /// The `/chat/completions` URL this source calls. `port` is the local
    /// llama.cpp server's, which a local source ignores.
    pub fn chat_url(&self, port: u16) -> String {
        let base = self.endpoint.trim().trim_end_matches('/');
        if !self.is_remote() || base.is_empty() {
            return format!("http://127.0.0.1:{port}/v1/chat/completions");
        }
        if base.ends_with("/chat/completions") {
            return base.to_string();
        }
        if base.ends_with("/v1") {
            return format!("{base}/chat/completions");
        }
        format!("{base}/v1/chat/completions")
    }

    /// What a remote call still needs, named for the person. None when the
    /// source can be called, which includes every local source.
    pub fn missing(&self) -> Option<&'static str> {
        if !self.is_remote() {
            return None;
        }
        let endpoint = self.endpoint.trim();
        if endpoint.is_empty() {
            return Some("the remote endpoint address is empty");
        }
        if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
            return Some("the remote endpoint address must start with http:// or https://");
        }
        if self.model.trim().is_empty() {
            return Some("the remote model name is empty");
        }
        None
    }

    /// The model name a caller reports, for a log line or a history entry.
    pub fn describe(&self) -> String {
        if self.is_remote() {
            let endpoint = self.endpoint.trim();
            if self.model.trim().is_empty() {
                return format!("remote · {endpoint}");
            }
            return format!("remote · {}", self.model.trim());
        }
        "local".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_file_is_local_with_nothing_remote() {
        let decoded: ModelSource = serde_json::from_str("{}").unwrap();
        assert_eq!(decoded.source, AiSource::Local);
        assert!(decoded.endpoint.is_empty());
        assert!(decoded.model.is_empty());
        assert!(decoded.api_key.is_empty());
        assert!(decoded.missing().is_none());
    }

    #[test]
    fn a_source_round_trips_through_json() {
        let source = ModelSource {
            source: AiSource::Remote,
            endpoint: "https://api.example.com/v1".into(),
            model: "big-model".into(),
            api_key: "secret".into(),
        };
        let decoded: ModelSource =
            serde_json::from_str(&serde_json::to_string(&source).unwrap()).unwrap();
        assert_eq!(decoded, source);
        assert!(decoded.is_remote());
    }

    #[test]
    fn a_local_source_ignores_the_remote_fields() {
        let source = ModelSource {
            endpoint: "https://api.example.com/v1".into(),
            ..ModelSource::default()
        };
        assert_eq!(
            source.chat_url(8081),
            "http://127.0.0.1:8081/v1/chat/completions"
        );
        assert!(source.missing().is_none());
        assert_eq!(source.describe(), "local");
    }

    #[test]
    fn a_remote_address_gains_the_chat_completions_path() {
        let base = |endpoint: &str| {
            ModelSource {
                source: AiSource::Remote,
                endpoint: endpoint.into(),
                model: "m".into(),
                ..ModelSource::default()
            }
            .chat_url(8081)
        };
        assert_eq!(
            base("https://api.example.com"),
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(
            base("https://api.example.com/"),
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(
            base("https://api.example.com/v1"),
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(
            base("https://api.example.com/v1/chat/completions"),
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(
            base("http://192.168.1.9:8000/v1"),
            "http://192.168.1.9:8000/v1/chat/completions"
        );
    }

    #[test]
    fn a_remote_source_names_what_it_still_needs() {
        let remote = |endpoint: &str, model: &str| ModelSource {
            source: AiSource::Remote,
            endpoint: endpoint.into(),
            model: model.into(),
            ..ModelSource::default()
        };
        assert_eq!(
            remote("", "m").missing(),
            Some("the remote endpoint address is empty")
        );
        assert_eq!(
            remote("api.example.com", "m").missing(),
            Some("the remote endpoint address must start with http:// or https://")
        );
        assert_eq!(
            remote("https://api.example.com", "").missing(),
            Some("the remote model name is empty")
        );
        assert!(remote("https://api.example.com", "m").missing().is_none());
    }

    #[test]
    fn a_remote_source_describes_its_model() {
        let source = ModelSource {
            source: AiSource::Remote,
            endpoint: "https://api.example.com/v1".into(),
            model: "big-model".into(),
            ..ModelSource::default()
        };
        assert_eq!(source.describe(), "remote · big-model");
    }
}
