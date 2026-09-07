//! Language-server-protocol framing: JSON messages behind a `Content-Length`
//! header, over a pipe.

use crate::json::{self, Json};
use std::io::{self, BufRead, Write};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Read one message, or `None` at end of input.
///
/// # Errors
///
/// Fails when the stream breaks, a header is malformed, or the body is not
/// valid JSON.
pub fn read_message(input: &mut impl BufRead) -> io::Result<Option<Json>> {
    let mut length: Option<usize> = None;

    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        // Header names are case-insensitive; only the length matters here.
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().ok();
            }
        }
    }

    let Some(length) = length else {
        return Err(invalid("message has no Content-Length"));
    };

    let mut body = vec![0_u8; length];
    input.read_exact(&mut body)?;
    let text = String::from_utf8(body).map_err(|_| invalid("message body is not UTF-8"))?;
    json::parse(&text)
        .map(Some)
        .map_err(|err| invalid(format!("malformed message: {err}")))
}

/// Write one message with its header.
///
/// # Errors
///
/// Fails when the stream breaks.
pub fn write_message(output: &mut impl Write, message: &Json) -> io::Result<()> {
    let body = message.to_string();
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(body.as_bytes())?;
    output.flush()
}

/// A response carrying a result.
pub fn response(id: Json, result: Json) -> Json {
    Json::object([
        ("jsonrpc", Json::string("2.0")),
        ("id", id),
        ("result", result),
    ])
}

/// A response carrying an error.
pub fn error_response(id: Json, code: i64, message: &str) -> Json {
    Json::object([
        ("jsonrpc", Json::string("2.0")),
        ("id", id),
        (
            "error",
            Json::object([
                ("code", Json::int(code)),
                ("message", Json::string(message)),
            ]),
        ),
    ])
}

/// A notification to the editor.
pub fn notification(method: &str, params: Json) -> Json {
    Json::object([
        ("jsonrpc", Json::string("2.0")),
        ("method", Json::string(method)),
        ("params", params),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_a_framed_message() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"shutdown"}"#;
        let raw = format!("Content-Length: {}\r\n\r\n{body}", body.len());
        let mut input = Cursor::new(raw.into_bytes());

        let message = read_message(&mut input).expect("reads").expect("a message");
        assert_eq!(
            message.get("method").and_then(Json::as_str),
            Some("shutdown")
        );
        assert!(read_message(&mut input).expect("reads").is_none());
    }

    #[test]
    fn ignores_other_headers_and_header_case() {
        let body = r#"{"id":2}"#;
        let raw = format!(
            "Content-Type: application/vscode-jsonrpc\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut input = Cursor::new(raw.into_bytes());

        let message = read_message(&mut input).expect("reads").expect("a message");
        assert_eq!(message.get("id").and_then(Json::as_i64), Some(2));
    }

    #[test]
    fn counts_length_in_bytes_not_characters() {
        let body = r#"{"s":"é😀"}"#;
        let raw = format!("Content-Length: {}\r\n\r\n{body}", body.len());
        let mut input = Cursor::new(raw.into_bytes());

        let message = read_message(&mut input).expect("reads").expect("a message");
        assert_eq!(message.get("s").and_then(Json::as_str), Some("é😀"));
    }

    #[test]
    fn writes_a_header_matching_the_body() {
        let mut out = Vec::new();
        write_message(&mut out, &Json::object([("a", Json::int(1))])).expect("writes");
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Content-Length: 7\r\n\r\n{\"a\":1}"
        );
    }

    #[test]
    fn round_trips_through_the_pipe() {
        let mut wire = Vec::new();
        let sent = notification("textDocument/publishDiagnostics", Json::array([]));
        write_message(&mut wire, &sent).expect("writes");

        let mut input = Cursor::new(wire);
        let received = read_message(&mut input).expect("reads").expect("a message");
        assert_eq!(received, sent);
    }

    #[test]
    fn rejects_a_message_with_no_length() {
        let mut input = Cursor::new(b"X: 1\r\n\r\n{}".to_vec());
        assert!(read_message(&mut input).is_err());
    }
}
