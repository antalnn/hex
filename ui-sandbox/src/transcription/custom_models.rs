//! The custom-model fallback the General pane asks for when a saved import is
//! missing; the sandbox has none.

#[derive(Clone, Debug)]
pub struct Fallback {
    pub model: super::models::TranscriptionModelId,
    pub language: String,
}

pub(crate) fn fallback() -> Fallback {
    Fallback {
        model: super::models::TranscriptionModelId::default(),
        language: "en".into(),
    }
}
