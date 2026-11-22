//! An OpenAI-compatible speech-to-text endpoint, for apps on this PC such as
//! OpenChamber (Labs › Transcription endpoint). This module owns the routes
//! and the request and reply shapes; it never touches a model. The audio
//! goes to a [`Transcribe`] seam, so the route tests run without one.
//!
//! The listener (`server`) binds `127.0.0.1` only and needs no token: a
//! program that can reach loopback is already on this PC. It is not the
//! local API, which keeps its own listener, token and discovery file.

mod multipart;
mod reply;

use std::sync::Arc;

use serde_json::json;

use super::http::HttpResponse;
use reply::Format;
pub(crate) use reply::{Failure, Outcome};

/// One request, read in full: the local API's `read_request` and
/// `read_body_bounded` supply the parts.
pub(super) struct Request {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) content_type: Option<String>,
    pub(super) body: Vec<u8>,
}

/// The audio of one upload and the language the client asked for, if it
/// named one (`auto` and empty already mean none).
pub(crate) struct Upload {
    pub(crate) bytes: Vec<u8>,
    pub(crate) language: Option<String>,
}

/// What turns one upload into a transcript.
pub(crate) type Transcribe = Arc<dyn Fn(Upload) -> Result<Outcome, Failure> + Send + Sync>;

/// Routes one request. `/v1` is optional in front of every route.
pub(super) fn respond(request: Request, transcribe: &Transcribe) -> HttpResponse {
    let path = request.path.split('?').next().unwrap_or("/");
    let path = match path.strip_prefix("/v1") {
        Some(rest) if rest.starts_with('/') => rest,
        _ => path,
    };
    match (request.method.as_str(), path) {
        ("GET", "/health") => HttpResponse::json(200, &json!({ "ok": true, "service": "hex" })),
        ("GET", "/models") => HttpResponse::json(
            200,
            &json!({
                "object": "list",
                "data": [{ "id": "hex", "object": "model", "owned_by": "hex" }],
            }),
        ),
        ("POST", "/audio/transcriptions") => {
            transcription(&request, transcribe).unwrap_or_else(|failure| reply::error(&failure))
        }
        _ => reply::error(&Failure::new(
            404,
            "not_found",
            format!("No route for {} {}", request.method, path),
        )),
    }
}

fn transcription(request: &Request, transcribe: &Transcribe) -> Result<HttpResponse, Failure> {
    let content_type = request.content_type.as_deref().unwrap_or("");
    if !content_type
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        return Err(Failure::invalid(
            "Send multipart/form-data with a file field.",
        ));
    }
    let form = multipart::parse(content_type, &request.body)
        .map_err(|error| Failure::invalid(error.message()))?;
    let format = Format::parse(form.response_format.as_deref())?;
    let language = form
        .language
        .as_deref()
        .map(|language| language.trim().to_ascii_lowercase())
        .filter(|language| !language.is_empty() && language != "auto");
    let outcome = transcribe(Upload {
        bytes: form.file(&request.body).to_vec(),
        language,
    })?;
    Ok(reply::success(format, &outcome))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::multipart::tests::body;
    use super::*;

    fn request(method: &str, path: &str, content_type: Option<&str>, body: Vec<u8>) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            content_type: content_type.map(Into::into),
            body,
        }
    }

    fn outcome(text: &str) -> Outcome {
        Outcome {
            text: text.into(),
            duration_ms: 1_500,
            language: "en".into(),
        }
    }

    /// Records every upload it gets and answers `text`, or fails.
    fn fake(
        seen: &Arc<Mutex<Vec<(usize, Option<String>)>>>,
        answer: Result<&'static str, u16>,
    ) -> Transcribe {
        let seen = seen.clone();
        Arc::new(move |upload: Upload| {
            seen.lock()
                .unwrap()
                .push((upload.bytes.len(), upload.language.clone()));
            match answer {
                Ok(text) => Ok(outcome(text)),
                Err(503) => Err(Failure::unavailable("no model")),
                Err(_) => Err(Failure::busy()),
            }
        })
    }

    fn post(path: &str, fields: &[(&str, &str)], transcribe: &Transcribe) -> HttpResponse {
        let bytes = body("xyz", fields, b"audio-bytes");
        respond(
            request(
                "POST",
                path,
                Some("multipart/form-data; boundary=xyz"),
                bytes,
            ),
            transcribe,
        )
    }

    fn json_of(response: &HttpResponse) -> serde_json::Value {
        serde_json::from_slice(&response.body).unwrap()
    }

    #[test]
    fn health_and_models_answer_with_and_without_the_v1_prefix() {
        let seen = Arc::default();
        let transcribe = fake(&seen, Ok("x"));
        for path in ["/v1/health", "/health", "/v1/health?probe=1"] {
            let response = respond(request("GET", path, None, Vec::new()), &transcribe);
            assert_eq!(response.status, 200, "{path}");
            assert_eq!(json_of(&response)["ok"], true);
        }
        let response = respond(request("GET", "/v1/models", None, Vec::new()), &transcribe);
        assert_eq!(json_of(&response)["data"][0]["id"], "hex");
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn a_transcription_answers_the_text_the_seam_returned() {
        let seen = Arc::default();
        let transcribe = fake(&seen, Ok("Finished text."));
        for path in ["/v1/audio/transcriptions", "/audio/transcriptions"] {
            let response = post(path, &[("model", "hex")], &transcribe);
            assert_eq!(response.status, 200, "{path}");
            assert_eq!(json_of(&response), json!({ "text": "Finished text." }));
        }
        assert_eq!(seen.lock().unwrap()[0], (11, None));
    }

    #[test]
    fn each_response_format_is_shaped() {
        let seen = Arc::default();
        let transcribe = fake(&seen, Ok("Hi."));
        let response = post(
            "/v1/audio/transcriptions",
            &[("response_format", "text")],
            &transcribe,
        );
        assert_eq!(response.content_type, Some("text/plain; charset=utf-8"));
        assert_eq!(response.body, b"Hi.");
        let response = post(
            "/v1/audio/transcriptions",
            &[("response_format", "verbose_json")],
            &transcribe,
        );
        assert_eq!(
            json_of(&response),
            json!({ "task": "transcribe", "language": "en", "duration": 1.5, "text": "Hi." })
        );
        let response = post(
            "/v1/audio/transcriptions",
            &[("response_format", "srt")],
            &transcribe,
        );
        assert_eq!(response.status, 400);
        assert_eq!(json_of(&response)["error"]["type"], "invalid_request_error");
        // An unsupported format is refused before any audio is transcribed.
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn the_requested_language_reaches_the_seam_unless_it_is_auto() {
        let seen = Arc::default();
        let transcribe = fake(&seen, Ok("x"));
        for language in [" EN ", "auto", "", "Sv"] {
            post(
                "/v1/audio/transcriptions",
                &[("language", language)],
                &transcribe,
            );
        }
        let languages: Vec<_> = seen.lock().unwrap().iter().map(|s| s.1.clone()).collect();
        assert_eq!(
            languages,
            vec![Some("en".to_owned()), None, None, Some("sv".to_owned())]
        );
    }

    #[test]
    fn the_seam_s_failures_keep_their_status_and_shape() {
        let seen = Arc::default();
        let response = post("/v1/audio/transcriptions", &[], &fake(&seen, Err(503)));
        assert_eq!(response.status, 503);
        assert_eq!(
            json_of(&response),
            json!({ "error": { "message": "no model", "type": "hex_unavailable" } })
        );
        let response = post("/v1/audio/transcriptions", &[], &fake(&seen, Err(429)));
        assert_eq!(response.status, 429);
    }

    #[test]
    fn bad_requests_are_refused_before_the_seam_runs() {
        let seen = Arc::default();
        let transcribe = fake(&seen, Ok("x"));
        let not_multipart = respond(
            request(
                "POST",
                "/v1/audio/transcriptions",
                Some("application/json"),
                b"{}".to_vec(),
            ),
            &transcribe,
        );
        assert_eq!(not_multipart.status, 400);
        let no_type = respond(
            request("POST", "/v1/audio/transcriptions", None, Vec::new()),
            &transcribe,
        );
        assert_eq!(no_type.status, 400);
        let no_file = respond(
            request(
                "POST",
                "/v1/audio/transcriptions",
                Some("multipart/form-data; boundary=b"),
                b"--b\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nhex\r\n--b--\r\n"
                    .to_vec(),
            ),
            &transcribe,
        );
        assert_eq!(no_file.status, 400);
        assert_eq!(
            json_of(&no_file)["error"]["message"],
            "The file field is missing."
        );
        let truncated = respond(
            request(
                "POST",
                "/v1/audio/transcriptions",
                Some("multipart/form-data; boundary=b"),
                b"--b\r\nContent-Disposition: form-data; name=\"file\"\r\n\r\nabc".to_vec(),
            ),
            &transcribe,
        );
        assert_eq!(truncated.status, 400);
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn every_other_route_is_not_found_in_the_openai_shape() {
        let transcribe = fake(&Arc::default(), Ok("x"));
        for (method, path) in [
            ("GET", "/"),
            ("GET", "/v1/audio/transcriptions"),
            ("POST", "/v1/health"),
            ("GET", "/v1/v1/health"),
            ("DELETE", "/v1/models"),
            ("GET", "/v1health"),
        ] {
            let response = respond(request(method, path, None, Vec::new()), &transcribe);
            assert_eq!(response.status, 404, "{method} {path}");
            assert_eq!(json_of(&response)["error"]["type"], "not_found");
        }
    }
}
