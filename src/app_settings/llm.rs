//! The persisted `llm` settings (`crate::local_model`): the llama.cpp server
//! that backs the local rewrite.

use super::*;

/// The local model server. Empty values fall back to the built-in defaults, so
/// a settings file written before these keys existed still names a working
/// server.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct LlmSettings {
    /// Start the server when Hex opens, and stop it when Hex quits.
    pub enabled: bool,
    /// The `llama-server` program. `None` uses `~/tools/llama-cpp`.
    pub program: Option<std::path::PathBuf>,
    /// The GGUF model. `None` uses Hex's own copy in its models folder.
    pub model: Option<std::path::PathBuf>,
    /// The port the local rewrite transformation calls.
    pub port: u16,
    /// Extra generation flags. Empty uses the tuned defaults.
    pub args: Vec<String>,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            program: None,
            model: None,
            port: crate::local_model::DEFAULT_PORT,
            args: Vec::new(),
        }
    }
}
