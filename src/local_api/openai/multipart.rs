//! A bounded `multipart/form-data` reader for an OpenAI transcription
//! upload. The body is already in memory (capped by the caller); this finds
//! the `file` part and the few short text fields, and ignores the rest. No
//! crate is involved: the tree has none for multipart.

use std::ops::Range;

/// RFC 2046 allows at most 70 characters in a boundary.
const MAX_BOUNDARY: usize = 70;
const MAX_PARTS: usize = 32;
/// The longest a text field's value may be.
const MAX_FIELD_BYTES: usize = 1024;
const MAX_PART_HEADER_BYTES: usize = 4 * 1024;

#[derive(Debug, Eq, PartialEq)]
pub(super) enum FormError {
    MissingBoundary,
    Truncated,
    Malformed,
    TooManyParts,
    FieldTooLong,
    MissingFile,
}

impl FormError {
    pub(super) fn message(&self) -> &'static str {
        match self {
            Self::MissingBoundary => "The multipart boundary is missing from Content-Type.",
            Self::Truncated => "The multipart body is incomplete.",
            Self::Malformed => "The multipart body could not be read.",
            Self::TooManyParts => "The multipart body has too many parts.",
            Self::FieldTooLong => "A multipart text field is too long.",
            Self::MissingFile => "The file field is missing.",
        }
    }
}

/// The fields an OpenAI transcription request carries.
#[derive(Debug, Default)]
pub(super) struct Form {
    file: Range<usize>,
    pub(super) file_name: Option<String>,
    pub(super) file_type: Option<String>,
    pub(super) language: Option<String>,
    pub(super) response_format: Option<String>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) model: Option<String>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) prompt: Option<String>,
}

impl Form {
    /// The uploaded audio, borrowed from the body that was parsed.
    pub(super) fn file<'a>(&self, body: &'a [u8]) -> &'a [u8] {
        &body[self.file.clone()]
    }
}

/// The boundary a `Content-Type` names, quoted or bare.
pub(super) fn boundary(content_type: &str) -> Option<String> {
    let mut parts = content_type.split(';');
    if !parts
        .next()?
        .trim()
        .eq_ignore_ascii_case("multipart/form-data")
    {
        return None;
    }
    let rest = &content_type[content_type.find(';')? + 1..];
    let value = parameters(rest)
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("boundary"))?
        .1;
    (!value.is_empty()
        && value.len() <= MAX_BOUNDARY
        && value.bytes().all(|byte| (0x20..0x7f).contains(&byte)))
    .then_some(value)
}

/// `name=value` pairs separated by `;`, where a value may be a quoted string
/// with backslash escapes that holds `;`.
fn parameters(text: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut characters = text.chars().peekable();
    loop {
        while characters
            .next_if(|c| c.is_whitespace() || *c == ';')
            .is_some()
        {}
        let mut name = String::new();
        while let Some(c) = characters.next_if(|c| *c != '=' && *c != ';') {
            name.push(c);
        }
        if characters.next_if_eq(&'=').is_none() {
            if name.trim().is_empty() {
                return pairs;
            }
            continue;
        }
        let mut value = String::new();
        if characters.next_if_eq(&'"').is_some() {
            while let Some(c) = characters.next() {
                match c {
                    '"' => break,
                    '\\' => value.extend(characters.next()),
                    c => value.push(c),
                }
            }
        } else {
            while let Some(c) = characters.next_if(|c| *c != ';') {
                value.push(c);
            }
        }
        pairs.push((name.trim().to_owned(), value.trim().to_owned()));
    }
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    let first = *needle.first()?;
    let last = haystack.len().checked_sub(needle.len())?;
    let mut index = from;
    while index <= last {
        index += haystack[index..=last]
            .iter()
            .position(|byte| *byte == first)?;
        if &haystack[index..index + needle.len()] == needle {
            return Some(index);
        }
        index += 1;
    }
    None
}

struct PartHeaders {
    name: Option<String>,
    file_name: Option<String>,
    content_type: Option<String>,
}

fn part_headers(headers: &[u8]) -> Result<PartHeaders, FormError> {
    let headers = std::str::from_utf8(headers).map_err(|_| FormError::Malformed)?;
    let mut parsed = PartHeaders {
        name: None,
        file_name: None,
        content_type: None,
    };
    for line in headers.split("\r\n").filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or(FormError::Malformed)?;
        if name.eq_ignore_ascii_case("content-disposition") {
            let mut pieces = value.splitn(2, ';');
            if !pieces
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("form-data")
            {
                return Err(FormError::Malformed);
            }
            for (key, value) in parameters(pieces.next().unwrap_or("")) {
                if key.eq_ignore_ascii_case("name") {
                    parsed.name = Some(value);
                } else if key.eq_ignore_ascii_case("filename") {
                    parsed.file_name = Some(value);
                }
            }
        } else if name.eq_ignore_ascii_case("content-type") {
            parsed.content_type = Some(value.trim().to_owned());
        }
    }
    Ok(parsed)
}

/// Reads `body` as the `multipart/form-data` that `content_type` describes.
pub(super) fn parse(content_type: &str, body: &[u8]) -> Result<Form, FormError> {
    let boundary = boundary(content_type).ok_or(FormError::MissingBoundary)?;
    let delimiter = format!("--{boundary}").into_bytes();
    let separator = format!("\r\n--{boundary}").into_bytes();
    // Anything before the first delimiter is a preamble.
    let mut position = find(body, &delimiter, 0).ok_or(FormError::Truncated)? + delimiter.len();
    let mut form = Form::default();
    let mut has_file = false;
    let mut parts = 0;
    loop {
        let rest = body.get(position..).ok_or(FormError::Truncated)?;
        if rest.starts_with(b"--") {
            break;
        }
        // Transport padding may follow a delimiter before its line ends.
        let padding = rest
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        let rest = &rest[padding..];
        if rest.len() < 2 {
            return Err(FormError::Truncated);
        }
        if !rest.starts_with(b"\r\n") {
            return Err(FormError::Malformed);
        }
        position += padding + 2;
        parts += 1;
        if parts > MAX_PARTS {
            return Err(FormError::TooManyParts);
        }
        let (headers, content_start) = if body[position..].starts_with(b"\r\n") {
            (&body[position..position], position + 2)
        } else {
            let end = find(body, b"\r\n\r\n", position).ok_or(FormError::Truncated)?;
            if end - position > MAX_PART_HEADER_BYTES {
                return Err(FormError::Malformed);
            }
            (&body[position..end], end + 4)
        };
        let content_end = find(body, &separator, content_start).ok_or(FormError::Truncated)?;
        position = content_end + separator.len();
        let headers = part_headers(headers)?;
        let content = content_start..content_end;
        let Some(name) = headers.name.as_deref() else {
            return Err(FormError::Malformed);
        };
        match name {
            "file" if !has_file => {
                has_file = true;
                form.file = content;
                form.file_name = headers.file_name;
                form.file_type = headers.content_type;
            }
            "language" | "response_format" | "model" | "prompt" => {
                if content.len() > MAX_FIELD_BYTES {
                    return Err(FormError::FieldTooLong);
                }
                let value = std::str::from_utf8(&body[content])
                    .map_err(|_| FormError::Malformed)?
                    .to_owned();
                let slot = match name {
                    "language" => &mut form.language,
                    "response_format" => &mut form.response_format,
                    "model" => &mut form.model,
                    _ => &mut form.prompt,
                };
                slot.get_or_insert(value);
            }
            _ => {}
        }
    }
    if !has_file {
        return Err(FormError::MissingFile);
    }
    Ok(form)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A body the way curl builds it: `--` plus an alphanumeric boundary.
    pub(crate) fn body(boundary: &str, fields: &[(&str, &str)], file: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        for (name, value) in fields {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
                )
                .as_bytes(),
            );
        }
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.webm\"\r\nContent-Type: audio/webm\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(file);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        body
    }

    fn content_type(boundary: &str) -> String {
        format!("multipart/form-data; boundary={boundary}")
    }

    #[test]
    fn a_webm_file_and_text_fields_are_read_as_curl_builds_them() {
        let audio = b"\x1a\x45\xdf\xa3 not really webm \r\n--almost\r\n\x00\xff";
        let bytes = body(
            "------------------------d74496d66958873e",
            &[
                ("model", "hex"),
                ("language", "en"),
                ("response_format", "verbose_json"),
                ("prompt", "names"),
            ],
            audio,
        );
        let form = parse(
            &content_type("------------------------d74496d66958873e"),
            &bytes,
        )
        .unwrap();
        assert_eq!(form.file(&bytes), audio);
        assert_eq!(form.file_name.as_deref(), Some("audio.webm"));
        assert_eq!(form.file_type.as_deref(), Some("audio/webm"));
        assert_eq!(form.model.as_deref(), Some("hex"));
        assert_eq!(form.language.as_deref(), Some("en"));
        assert_eq!(form.response_format.as_deref(), Some("verbose_json"));
        assert_eq!(form.prompt.as_deref(), Some("names"));
    }

    #[test]
    fn a_browser_form_data_body_with_a_quoted_boundary_is_read() {
        let boundary = "----WebKitFormBoundaryAbCd1234";
        let bytes = body(boundary, &[("model", "whisper-1")], b"RIFFdata");
        let header = format!("multipart/form-data; boundary=\"{boundary}\"; charset=utf-8");
        let form = parse(&header, &bytes).unwrap();
        assert_eq!(form.file(&bytes), b"RIFFdata");
        assert_eq!(form.model.as_deref(), Some("whisper-1"));
    }

    #[test]
    fn the_boundary_comes_from_a_quoted_or_bare_parameter() {
        assert_eq!(
            boundary("multipart/form-data; boundary=abc").as_deref(),
            Some("abc")
        );
        assert_eq!(
            boundary("Multipart/Form-Data;Boundary=\"a;b\"").as_deref(),
            Some("a;b")
        );
        assert_eq!(boundary("multipart/form-data"), None);
        assert_eq!(boundary("multipart/form-data; boundary="), None);
        assert_eq!(boundary("application/json; boundary=abc"), None);
        let long = "a".repeat(71);
        assert_eq!(
            boundary(&format!("multipart/form-data; boundary={long}")),
            None
        );
        let longest = "a".repeat(70);
        assert!(boundary(&format!("multipart/form-data; boundary={longest}")).is_some());
    }

    #[test]
    fn a_missing_boundary_is_refused() {
        assert_eq!(
            parse("multipart/form-data", b"x").unwrap_err(),
            FormError::MissingBoundary
        );
    }

    #[test]
    fn a_missing_file_is_refused() {
        let bytes =
            b"--b\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nhex\r\n--b--\r\n";
        assert_eq!(
            parse(&content_type("b"), bytes).unwrap_err(),
            FormError::MissingFile
        );
    }

    #[test]
    fn a_truncated_body_is_refused() {
        let bytes = body("b", &[("model", "hex")], b"audio-bytes");
        for cut in [5, 40, bytes.len() - 8, bytes.len() - 4, bytes.len() - 3] {
            let error = parse(&content_type("b"), &bytes[..cut]).unwrap_err();
            assert_eq!(error, FormError::Truncated, "cut at {cut}");
        }
        assert_eq!(
            parse(&content_type("b"), b"no delimiter at all").unwrap_err(),
            FormError::Truncated
        );
    }

    #[test]
    fn an_oversized_text_field_is_refused() {
        let long = "x".repeat(MAX_FIELD_BYTES + 1);
        let bytes = body("b", &[("prompt", &long)], b"a");
        assert_eq!(
            parse(&content_type("b"), &bytes).unwrap_err(),
            FormError::FieldTooLong
        );
        let fits = "x".repeat(MAX_FIELD_BYTES);
        let bytes = body("b", &[("prompt", &fits)], b"a");
        assert!(parse(&content_type("b"), &bytes).is_ok());
    }

    #[test]
    fn too_many_parts_are_refused() {
        let fields: Vec<(&str, &str)> = (0..MAX_PARTS).map(|_| ("extra", "1")).collect();
        let bytes = body("b", &fields, b"a");
        assert_eq!(
            parse(&content_type("b"), &bytes).unwrap_err(),
            FormError::TooManyParts
        );
        let fields: Vec<(&str, &str)> = (0..MAX_PARTS - 1).map(|_| ("extra", "1")).collect();
        let bytes = body("b", &fields, b"a");
        assert!(parse(&content_type("b"), &bytes).is_ok());
    }

    #[test]
    fn unknown_fields_and_a_preamble_are_ignored() {
        let mut bytes = b"this is a preamble\r\n".to_vec();
        bytes.extend(body(
            "b",
            &[("temperature", "0"), ("timestamp_granularities[]", "word")],
            b"abc",
        ));
        bytes.extend_from_slice(b"an epilogue");
        let form = parse(&content_type("b"), &bytes).unwrap();
        assert_eq!(form.file(&bytes), b"abc");
        assert_eq!(form.model, None);
    }

    #[test]
    fn a_part_without_a_name_or_with_bad_text_is_malformed() {
        let bytes = b"--b\r\nContent-Type: text/plain\r\n\r\nhex\r\n--b--\r\n";
        assert_eq!(
            parse(&content_type("b"), bytes).unwrap_err(),
            FormError::Malformed
        );
        let bytes =
            b"--b\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\n\xff\xfe\r\n--b--\r\n";
        assert_eq!(
            parse(&content_type("b"), bytes).unwrap_err(),
            FormError::Malformed
        );
        assert_eq!(
            parse(&content_type("b"), b"--bjunk").unwrap_err(),
            FormError::Malformed
        );
    }

    #[test]
    fn the_first_file_part_wins_and_an_empty_file_is_kept() {
        let mut bytes = body("b", &[], b"");
        let second = body("b", &[], b"second");
        let tail = b"--b--\r\n";
        bytes.truncate(bytes.len() - tail.len());
        bytes.extend_from_slice(&second);
        let form = parse(&content_type("b"), &bytes).unwrap();
        assert_eq!(form.file(&bytes), b"");
    }

    #[test]
    fn parameters_handle_quotes_escapes_and_semicolons() {
        let pairs = parameters(r#" name="a;b"; filename="x\"y.wav" ; flag=1; bare"#);
        assert_eq!(
            pairs,
            vec![
                ("name".to_owned(), "a;b".to_owned()),
                ("filename".to_owned(), "x\"y.wav".to_owned()),
                ("flag".to_owned(), "1".to_owned()),
            ]
        );
    }
}
