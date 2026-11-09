//! Speech models, fixture only: enough of the catalog for the General pane's
//! language row and the Models pane stub.

use serde::Deserialize;

/// A speech model's id.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TranscriptionModelId(pub &'static str);

impl serde::Serialize for TranscriptionModelId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0)
    }
}

impl<'de> serde::Deserialize<'de> for TranscriptionModelId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Ok(Self(Box::leak(id.into_boxed_str())))
    }
}

impl TranscriptionModelId {
    pub fn is_labs_gated(self) -> bool {
        false
    }
}

impl std::fmt::Display for TranscriptionModelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// One model's catalog entry.
pub struct ModelDefinition {
    pub id: TranscriptionModelId,
    pub name: &'static str,
    languages: &'static [&'static str],
}

impl ModelDefinition {
    pub fn language_choices(&self) -> &'static [&'static str] {
        self.languages
    }
}

const WHISPER: ModelDefinition = ModelDefinition {
    id: TranscriptionModelId("whisper-large-v3-turbo"),
    name: "Whisper Large v3 Turbo",
    languages: &[
        "en", "de", "fr", "es", "it", "pt", "ru", "pl", "zh", "ja", "uk",
    ],
};

const PARAREKET: ModelDefinition = ModelDefinition {
    id: TranscriptionModelId("parakeet-v3"),
    name: "Parakeet v3",
    languages: &["en"],
};

pub(crate) fn definition(id: TranscriptionModelId) -> &'static ModelDefinition {
    match id.0 {
        "parakeet-v3" => &PARAREKET,
        _ => &WHISPER,
    }
}

/// English name for a language code, for the select's labels.
pub(crate) fn language_name(code: &str) -> &'static str {
    match code {
        "de" => "German",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "pl" => "Polish",
        "zh" => "Chinese",
        "ja" => "Japanese",
        "uk" => "Ukrainian",
        _ => "English",
    }
}

/// The saved selection: one model and the language it listens for.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TranscriptionSelection {
    pub model: TranscriptionModelId,
    pub language: String,
}

impl TranscriptionSelection {
    pub fn new() -> Self {
        Self {
            model: TranscriptionModelId::default(),
            language: "en".into(),
        }
    }
}

/// The stage a preparation is at, for the Models pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ModelPreparationStage {
    Downloading,
    Loading,
}

/// Which runtime carries a model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelRuntime {
    Gguf,
    AppleSpeech,
}
