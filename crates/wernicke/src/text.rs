//! Translating between what an editor says and what the compiler says.
//!
//! The protocol counts lines from zero and characters in UTF-16 code units;
//! Cranium counts lines from one and columns in bytes. Files arrive as `file:`
//! URIs rather than paths. Both conversions live here.

use std::fmt::Write;
use std::path::{Path, PathBuf};

/// Turn a `file:` URI into a path.
///
/// Returns `None` for any other scheme, since there is nothing to compile.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file:///C:/x` on Windows and `file:///home/x` elsewhere both leave a
    // leading slash here; only the drive-letter form should lose it.
    let decoded = percent_decode(rest);
    let trimmed = match decoded.strip_prefix('/') {
        Some(after) if looks_like_drive(after) => after,
        _ => &decoded,
    };
    Some(PathBuf::from(
        trimmed.replace('/', std::path::MAIN_SEPARATOR_STR),
    ))
}

/// Turn a path into a `file:` URI.
pub fn path_to_uri(path: &Path) -> String {
    let text = path.display().to_string().replace('\\', "/");
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for ch in text.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~' | '/' | ':' => out.push(ch),
            other => {
                let mut buffer = [0_u8; 4];
                for byte in other.encode_utf8(&mut buffer).as_bytes() {
                    let _ = write!(out, "%{byte:02X}");
                }
            }
        }
    }
    out
}

fn looks_like_drive(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic()
    )
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

/// A document's text, indexed by line so positions can be converted.
#[derive(Debug, Clone)]
pub struct Lines {
    text: String,
    /// Byte offset where each line starts.
    starts: Vec<usize>,
}

impl Lines {
    /// Index `text` by line.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let mut starts = vec![0];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(offset + 1);
            }
        }
        Self { text, starts }
    }

    /// The whole document.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Number of lines.
    pub fn count(&self) -> usize {
        self.starts.len()
    }

    /// One line, without its terminator. Lines count from zero.
    pub fn line(&self, index: usize) -> &str {
        let Some(&start) = self.starts.get(index) else {
            return "";
        };
        let end = self
            .starts
            .get(index + 1)
            .map_or(self.text.len(), |&next| next);
        self.text[start..end].trim_end_matches(['\r', '\n'])
    }

    /// Convert the compiler's one-based line and byte column into the
    /// protocol's zero-based line and UTF-16 character.
    pub fn to_position(&self, line: usize, column: usize) -> (usize, usize) {
        let line_index = line.saturating_sub(1);
        let text = self.line(line_index);
        let byte_offset = column.saturating_sub(1).min(text.len());
        let character = text[..byte_offset].encode_utf16().count();
        (line_index, character)
    }

    /// Convert the protocol's zero-based line and UTF-16 character into a byte
    /// offset within that line.
    pub fn to_byte_column(&self, line: usize, character: usize) -> usize {
        let text = self.line(line);
        let mut seen = 0;
        for (offset, ch) in text.char_indices() {
            if seen >= character {
                return offset;
            }
            seen += ch.len_utf16();
        }
        text.len()
    }

    /// The identifier surrounding a byte offset on a line, and where it starts.
    ///
    /// Used both to widen a diagnostic onto a whole word and to work out what
    /// the cursor is resting on.
    pub fn word_at(&self, line: usize, byte_column: usize) -> Option<(String, usize)> {
        let text = self.line(line);
        if byte_column > text.len() {
            return None;
        }
        let is_word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
        let bytes = text.as_bytes();

        let mut start = byte_column.min(bytes.len());
        while start > 0 && is_word(bytes[start - 1]) {
            start -= 1;
        }
        let mut end = byte_column.min(bytes.len());
        while end < bytes.len() && is_word(bytes[end]) {
            end += 1;
        }
        if start == end {
            return None;
        }
        Some((text[start..end].to_string(), start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_uris_and_paths_back_and_forth() {
        let path = uri_to_path("file:///C:/Projects/bf-tools/main.cra").expect("a path");
        assert!(path.to_string_lossy().contains("Projects"));
        assert!(path.to_string_lossy().starts_with("C:"));

        let unix = uri_to_path("file:///home/aspen/main.cra").expect("a path");
        assert!(unix.to_string_lossy().contains("home"));

        assert_eq!(uri_to_path("http://example.com/x.cra"), None);
    }

    #[test]
    fn decodes_escaped_characters_in_uris() {
        let path = uri_to_path("file:///tmp/my%20project/a%2Bb.cra").expect("a path");
        let text = path.to_string_lossy().replace('\\', "/");
        assert!(text.ends_with("my project/a+b.cra"), "{text}");
    }

    #[test]
    fn a_path_round_trips_through_a_uri() {
        let original = Path::new("/tmp/my project/main.cra");
        let uri = path_to_uri(original);
        assert!(uri.contains("%20"), "{uri}");
        let back = uri_to_path(&uri).expect("a path");
        assert_eq!(
            back.to_string_lossy().replace('\\', "/"),
            "/tmp/my project/main.cra"
        );
    }

    #[test]
    fn indexes_lines() {
        let lines = Lines::new("one\ntwo\r\nthree");
        assert_eq!(lines.count(), 3);
        assert_eq!(lines.line(0), "one");
        assert_eq!(lines.line(1), "two");
        assert_eq!(lines.line(2), "three");
        assert_eq!(lines.line(9), "");
    }

    #[test]
    fn converts_positions_across_the_one_based_boundary() {
        let lines = Lines::new("fn main() {\n    let x = 1;\n}");
        // The compiler's 2:9 is the `x`; the protocol counts from zero.
        assert_eq!(lines.to_position(2, 9), (1, 8));
        assert_eq!(lines.to_byte_column(1, 8), 8);
    }

    #[test]
    fn counts_characters_in_utf16_units() {
        // `😀` is one character but two UTF-16 units and four bytes.
        let lines = Lines::new("// 😀 x");
        let byte_column = "// 😀 ".len();
        let (_, character) = lines.to_position(1, byte_column + 1);
        assert_eq!(character, "// ".encode_utf16().count() + 2 + 1);
        assert_eq!(lines.to_byte_column(0, character), byte_column);
    }

    #[test]
    fn finds_the_word_under_a_position() {
        let lines = Lines::new("let total = square(7);");
        assert_eq!(lines.word_at(0, 13), Some(("square".to_string(), 12)));
        // Resting just past the end of a word still finds it.
        assert_eq!(lines.word_at(0, 3), Some(("let".to_string(), 0)));
        // Punctuation is not a word.
        assert_eq!(lines.word_at(0, 21), None);
    }
}
