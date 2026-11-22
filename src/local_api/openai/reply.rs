//! The replies the endpoint sends, in the shapes OpenAI clients read: the
//! transcript as `json`, `text` or `verbose_json`, and errors as
//! `{"error":{"message":...,"type":...}}`. Nothing here carries audio, and
//! only a success carries the transcript.

use serde_json::json;

use crate::local_api::http::HttpResponse;

/// What a transcription produced, ready to be shaped into a reply.
#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) text: String,
    pub(crate) duration_ms: u64,
    /// The language the transcript was made in; `auto` when Hex chose.
    pub(crate) language: String,
}

/// Why a request got no transcript: the status and OpenAI error `type` the
/// client sees. Only statuses [`status_known`] lists are sent as they are.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Failure {
    pub(crate) status: u16,
    pub(crate) kind: &'static str,
    pub(crate) message: String,
}

impl Failure {
    pub(crate) fn new(status: u16, kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new(400, "invalid_request_error", message)
    }

    pub(crate) fn unavailable(message: impl Into<String>) -> Self {
        Self::new(503, "hex_unavailable", message)
    }

    pub(crate) fn busy() -> Self {
        Self::new(429, "hex_busy", "Hex is busy with other audio. Try again.")
    }

    pub(crate) fn too_large() -> Self {
        Self::new(413, "invalid_request_error", "The upload is too large.")
    }

    pub(crate) fn timeout() -> Self {
        Self::new(
            504,
            "hex_timeout",
            "Hex took too long to transcribe the audio.",
        )
    }

    pub(crate) fn server(message: impl Into<String>) -> Self {
        Self::new(500, "server_error", message)
    }
}

/// The statuses `HttpResponse`'s reason table knows; any other would panic
/// the worker that built it.
pub(super) fn status_known(status: u16) -> bool {
    matches!(
        status,
        200 | 201
            | 204
            | 400
            | 401
            | 403
            | 404
            | 408
            | 409
            | 411
            | 413
            | 415
            | 422
            | 429
            | 431
            | 500
            | 503
            | 504
    )
}

pub(super) fn error(failure: &Failure) -> HttpResponse {
    let status = if status_known(failure.status) {
        failure.status
    } else {
        500
    };
    HttpResponse::json(
        status,
        &json!({ "error": { "message": failure.message, "type": failure.kind } }),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Format {
    Json,
    Text,
    VerboseJson,
}

impl Format {
    /// The requested `response_format`; absent or empty means `json`.
    pub(super) fn parse(requested: Option<&str>) -> Result<Self, Failure> {
        match requested.map(str::trim).unwrap_or_default() {
            "" | "json" => Ok(Self::Json),
            "text" => Ok(Self::Text),
            "verbose_json" => Ok(Self::VerboseJson),
            other => Err(Failure::invalid(format!(
                "response_format {other} is not supported (json, text, verbose_json)."
            ))),
        }
    }
}

pub(super) fn success(format: Format, outcome: &Outcome) -> HttpResponse {
    match format {
        Format::Json => HttpResponse::json(200, &json!({ "text": outcome.text })),
        Format::Text => HttpResponse {
            status: 200,
            reason: "OK",
            content_type: Some("text/plain; charset=utf-8"),
            body: outcome.text.clone().into_bytes(),
        },
        Format::VerboseJson => HttpResponse::json(
            200,
            &json!({
                "task": "transcribe",
                "language": outcome.language,
                "duration": outcome.duration_ms as f64 / 1000.0,
                "text": outcome.text,
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome() -> Outcome {
        Outcome {
            text: "Hello, world.".into(),
            duration_ms: 2_500,
            language: "en".into(),
        }
    }

    fn json_body(response: &HttpResponse) -> serde_json::Value {
        serde_json::from_slice(&response.body).unwrap()
    }

    #[test]
    fn json_answers_text_only() {
        let response = success(Format::Json, &outcome());
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, Some("application/json"));
        assert_eq!(json_body(&response), json!({ "text": "Hello, world." }));
    }

    #[test]
    fn text_answers_the_bare_transcript() {
        let response = success(Format::Text, &outcome());
        assert_eq!(response.content_type, Some("text/plain; charset=utf-8"));
        assert_eq!(response.body, b"Hello, world.");
    }

    #[test]
    fn verbose_json_adds_task_language_and_seconds() {
        let response = success(Format::VerboseJson, &outcome());
        assert_eq!(
            json_body(&response),
            json!({
                "task": "transcribe",
                "language": "en",
                "duration": 2.5,
                "text": "Hello, world.",
            })
        );
    }

    #[test]
    fn formats_parse_and_other_values_are_refused() {
        assert_eq!(Format::parse(None).unwrap(), Format::Json);
        assert_eq!(Format::parse(Some("")).unwrap(), Format::Json);
        assert_eq!(Format::parse(Some(" json ")).unwrap(), Format::Json);
        assert_eq!(Format::parse(Some("text")).unwrap(), Format::Text);
        assert_eq!(
            Format::parse(Some("verbose_json")).unwrap(),
            Format::VerboseJson
        );
        for other in ["srt", "vtt", "JSON"] {
            let failure = Format::parse(Some(other)).unwrap_err();
            assert_eq!(failure.status, 400);
            assert_eq!(failure.kind, "invalid_request_error");
            assert!(failure.message.contains(other));
        }
    }

    #[test]
    fn errors_use_the_openai_shape_for_every_status_the_endpoint_sends() {
        let failures = [
            Failure::invalid("bad"),
            Failure::unavailable("no model"),
            Failure::busy(),
            Failure::too_large(),
            Failure::timeout(),
            Failure::server("broke"),
            Failure::new(404, "not_found", "no route"),
        ];
        for failure in failures {
            assert!(status_known(failure.status), "{}", failure.status);
            let response = error(&failure);
            assert_eq!(response.status, failure.status);
            assert_eq!(
                json_body(&response),
                json!({ "error": { "message": failure.message, "type": failure.kind } })
            );
        }
    }

    #[test]
    fn an_unknown_status_becomes_a_server_error_not_a_panic() {
        let response = error(&Failure::new(502, "hex_error", "bad gateway"));
        assert_eq!(response.status, 500);
    }

    #[test]
    fn every_known_status_has_a_reason() {
        for status in 0..=599 {
            if status_known(status) {
                assert!(!crate::local_api::http::reason(status).is_empty());
            }
        }
    }
}
