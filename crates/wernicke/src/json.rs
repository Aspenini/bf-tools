//! Just enough JSON for the language server protocol.
//!
//! The protocol is JSON over a pipe, and the messages involved are small and
//! well understood, so this is a hand-written parser and writer rather than a
//! dependency. Objects keep their insertion order, which makes the traffic
//! readable when someone is watching it.

use std::fmt;

/// A JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    /// `null`
    Null,
    /// `true` or `false`
    Bool(bool),
    /// Any number; the protocol only uses integers.
    Number(f64),
    /// A string, already unescaped.
    String(String),
    /// An array.
    Array(Vec<Json>),
    /// An object, in insertion order.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Build an object from key/value pairs.
    pub fn object<K: Into<String>>(fields: impl IntoIterator<Item = (K, Json)>) -> Self {
        Json::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
    }

    /// Build an array.
    pub fn array(items: impl IntoIterator<Item = Json>) -> Self {
        Json::Array(items.into_iter().collect())
    }

    /// Build a string.
    pub fn string(text: impl Into<String>) -> Self {
        Json::String(text.into())
    }

    /// Build a number from an integer.
    pub fn int(value: i64) -> Self {
        Json::Number(value as f64)
    }

    /// The value of `key`, for objects.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// Follow a chain of keys.
    pub fn path(&self, keys: &[&str]) -> Option<&Json> {
        keys.iter().try_fold(self, |value, key| value.get(key))
    }

    /// The text of a string value.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text),
            _ => None,
        }
    }

    /// A number rounded towards zero.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Number(value) => Some(*value as i64),
            _ => None,
        }
    }

    /// The elements of an array.
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    /// True when this is `null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Json::Null => f.write_str("null"),
            Json::Bool(true) => f.write_str("true"),
            Json::Bool(false) => f.write_str("false"),
            Json::Number(value) => {
                if value.fract() == 0.0 && value.is_finite() {
                    write!(f, "{}", *value as i64)
                } else {
                    write!(f, "{value}")
                }
            }
            Json::String(text) => write_string(f, text),
            Json::Array(items) => {
                f.write_str("[")?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
            Json::Object(fields) => {
                f.write_str("{")?;
                for (index, (key, value)) in fields.iter().enumerate() {
                    if index > 0 {
                        f.write_str(",")?;
                    }
                    write_string(f, key)?;
                    write!(f, ":{value}")?;
                }
                f.write_str("}")
            }
        }
    }
}

fn write_string(f: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    f.write_str("\"")?;
    for ch in text.chars() {
        match ch {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            // Everything below a space has to be escaped; the rest goes out as
            // UTF-8, which JSON allows.
            ch if (ch as u32) < 0x20 => write!(f, "\\u{:04x}", ch as u32)?,
            ch => write!(f, "{ch}")?,
        }
    }
    f.write_str("\"")
}

/// A malformed JSON document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// What went wrong.
    pub message: String,
    /// Byte offset where it was noticed.
    pub offset: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for ParseError {}

/// Parse a JSON document.
///
/// # Errors
///
/// Returns the first position that did not make sense.
pub fn parse(text: &str) -> Result<Json, ParseError> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        at: 0,
    };
    parser.skip_space();
    let value = parser.value()?;
    parser.skip_space();
    if parser.at != parser.bytes.len() {
        return Err(parser.error("trailing input"));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            message: message.into(),
            offset: self.at,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), ParseError> {
        if self.peek() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.error(format!("expected `{}`", byte as char)))
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, ParseError> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.error("unknown literal"))
        }
    }

    fn value(&mut self) -> Result<Json, ParseError> {
        match self.peek() {
            None => Err(self.error("unexpected end of input")),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(byte) => Err(self.error(format!("unexpected `{}`", byte as char))),
        }
    }

    fn array(&mut self) -> Result<Json, ParseError> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_space();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_space();
            items.push(self.value()?);
            self.skip_space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.error("expected `,` or `]`")),
            }
        }
    }

    fn object(&mut self) -> Result<Json, ParseError> {
        self.expect(b'{')?;
        let mut fields = Vec::new();
        self.skip_space();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_space();
            let key = self.string()?;
            self.skip_space();
            self.expect(b':')?;
            self.skip_space();
            let value = self.value()?;
            fields.push((key, value));
            self.skip_space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(fields));
                }
                _ => return Err(self.error("expected `,` or `}`")),
            }
        }
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        ) {
            self.at += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.at])
            .ok()
            .and_then(|text| text.parse().ok())
            .map(Json::Number)
            .ok_or_else(|| self.error("malformed number"))
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error("unterminated string"));
            };
            self.at += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(escape) = self.peek() else {
                        return Err(self.error("unterminated escape"));
                    };
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return Err(self.error("unknown escape")),
                    }
                }
                _ => {
                    // Multi-byte UTF-8 arrives a byte at a time; gather the
                    // whole sequence before pushing it.
                    let start = self.at - 1;
                    while self.peek().is_some_and(|next| next & 0xc0 == 0x80) {
                        self.at += 1;
                    }
                    match std::str::from_utf8(&self.bytes[start..self.at]) {
                        Ok(text) => out.push_str(text),
                        Err(_) => return Err(self.error("invalid UTF-8 in string")),
                    }
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let end = self.at + 4;
        if end > self.bytes.len() {
            return Err(self.error("`\\u` needs four hex digits"));
        }
        let text = std::str::from_utf8(&self.bytes[self.at..end])
            .map_err(|_| self.error("`\\u` needs four hex digits"))?;
        let value =
            u32::from_str_radix(text, 16).map_err(|_| self.error("`\\u` needs four hex digits"))?;
        self.at = end;
        Ok(value)
    }

    fn unicode_escape(&mut self) -> Result<char, ParseError> {
        let first = self.hex4()?;
        // A character outside the basic plane arrives as a surrogate pair.
        if (0xd800..0xdc00).contains(&first) {
            if self.peek() == Some(b'\\') && self.bytes.get(self.at + 1) == Some(&b'u') {
                self.at += 2;
                let second = self.hex4()?;
                if (0xdc00..0xe000).contains(&second) {
                    let combined = 0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00);
                    return char::from_u32(combined)
                        .ok_or_else(|| self.error("invalid surrogate pair"));
                }
            }
            return Err(self.error("unpaired surrogate"));
        }
        char::from_u32(first).ok_or_else(|| self.error("invalid escape"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_message() {
        let text =
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"a":[1,2,null,true]}}"#;
        let value = parse(text).expect("valid json");
        assert_eq!(value.to_string(), text);
    }

    #[test]
    fn reads_the_pieces_out() {
        let value = parse(r#"{"a":{"b":"c"},"n":-12,"list":[1,2]}"#).expect("valid json");
        assert_eq!(value.path(&["a", "b"]).and_then(Json::as_str), Some("c"));
        assert_eq!(value.get("n").and_then(Json::as_i64), Some(-12));
        assert_eq!(
            value.get("list").and_then(Json::as_array).map(<[_]>::len),
            Some(2)
        );
        assert_eq!(value.get("missing"), None);
    }

    #[test]
    fn handles_escapes_in_both_directions() {
        let value = parse(r#""a\"b\\c\ndAé😀""#).expect("valid json");
        assert_eq!(value.as_str(), Some("a\"b\\c\ndAé😀"));
        // Round-tripping keeps the meaning, escaping only what it must.
        let again = parse(&value.to_string()).expect("valid json");
        assert_eq!(again, value);
    }

    #[test]
    fn escapes_control_characters() {
        let value = Json::string("tab\there\u{1}");
        assert_eq!(value.to_string(), r#""tab\there\u0001""#);
    }

    #[test]
    fn keeps_object_order() {
        let value = Json::object([("z", Json::int(1)), ("a", Json::int(2))]);
        assert_eq!(value.to_string(), r#"{"z":1,"a":2}"#);
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in ["", "{", "[1,]", "{\"a\"}", "nul", "\"unterminated", "1 2"] {
            assert!(parse(bad).is_err(), "should reject {bad:?}");
        }
    }
}
