//! Tokenizer for Cranium source text.

use std::fmt;

/// One-based line/column of a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    /// One-based line number.
    pub line: usize,
    /// One-based column number.
    pub column: usize,
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// A lexical token.
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// Identifier or keyword.
    Ident(String),
    /// Integer literal; character literals lex to this too.
    Int(u64),
    /// String literal with escapes already resolved.
    Str(Vec<u8>),
    /// Punctuation or operator, stored as its source spelling.
    Punct(&'static str),
    /// End of input.
    Eof,
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tok::Ident(name) => write!(f, "`{name}`"),
            Tok::Int(value) => write!(f, "`{value}`"),
            Tok::Str(_) => write!(f, "string literal"),
            Tok::Punct(text) => write!(f, "`{text}`"),
            Tok::Eof => write!(f, "end of file"),
        }
    }
}

/// A token together with the position where it started.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// The token itself.
    pub tok: Tok,
    /// Where the token starts in the source.
    pub span: Span,
}

/// A lexing failure.
#[derive(Debug, Clone, PartialEq)]
pub struct LexError {
    /// Human readable description.
    pub message: String,
    /// Where the problem was found.
    pub span: Span,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.span, self.message)
    }
}

/// Operators and punctuation, longest first so greedy matching works.
const PUNCTUATION: &[&str] = &[
    "&&", "||", "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=", "%=", "..", "->", "<<", ">>", "+",
    "-", "*", "/", "%", "=", "<", ">", "!", "(", ")", "{", "}", "[", "]", ",", ";", ":", "&", "|",
    "^",
];

struct Lexer<'a> {
    src: &'a [u8],
    index: usize,
    line: usize,
    column: usize,
}

/// Split Cranium source into tokens.
///
/// Line comments start with `//`, block comments are `/* ... */` and do not
/// nest. Every other byte must belong to a token.
pub fn tokenize(src: &str) -> Result<Vec<Token>, LexError> {
    let mut lexer = Lexer {
        src: src.as_bytes(),
        index: 0,
        line: 1,
        column: 1,
    };
    lexer.run()
}

impl Lexer<'_> {
    fn span(&self) -> Span {
        Span {
            line: self.line,
            column: self.column,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.index).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.index + offset).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.index += 1;
        if byte == b'\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(byte)
    }

    fn error<T>(&self, message: impl Into<String>) -> Result<T, LexError> {
        Err(LexError {
            message: message.into(),
            span: self.span(),
        })
    }

    fn run(&mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();

        loop {
            self.skip_trivia()?;
            let span = self.span();
            let Some(byte) = self.peek() else {
                tokens.push(Token {
                    tok: Tok::Eof,
                    span,
                });
                return Ok(tokens);
            };

            let tok = match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'_' => self.lex_ident(),
                b'0'..=b'9' => self.lex_number()?,
                b'"' => Tok::Str(self.lex_string()?),
                b'\'' => Tok::Int(u64::from(self.lex_char()?)),
                _ => self.lex_punct()?,
            };

            tokens.push(Token { tok, span });
        }
    }

    fn skip_trivia(&mut self) -> Result<(), LexError> {
        loop {
            match self.peek() {
                Some(byte) if byte.is_ascii_whitespace() => {
                    self.bump();
                }
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    while let Some(byte) = self.peek() {
                        if byte == b'\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
                    let start = self.span();
                    self.bump();
                    self.bump();
                    loop {
                        match self.peek() {
                            None => {
                                return Err(LexError {
                                    message: "unterminated block comment".into(),
                                    span: start,
                                })
                            }
                            Some(b'*') if self.peek_at(1) == Some(b'/') => {
                                self.bump();
                                self.bump();
                                break;
                            }
                            Some(_) => {
                                self.bump();
                            }
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn lex_ident(&mut self) -> Tok {
        let start = self.index;
        while matches!(
            self.peek(),
            Some(b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_')
        ) {
            self.bump();
        }
        let text = std::str::from_utf8(&self.src[start..self.index]).expect("ascii identifier");
        Tok::Ident(text.to_string())
    }

    fn lex_number(&mut self) -> Result<Tok, LexError> {
        let span = self.span();
        let (radix, prefixed) = match (self.peek(), self.peek_at(1)) {
            (Some(b'0'), Some(b'x' | b'X')) => (16, true),
            (Some(b'0'), Some(b'b' | b'B')) => (2, true),
            _ => (10, false),
        };
        if prefixed {
            self.bump();
            self.bump();
        }

        let mut digits = String::new();
        while let Some(byte) = self.peek() {
            if byte == b'_' {
                self.bump();
                continue;
            }
            if !(byte as char).is_digit(radix) {
                break;
            }
            digits.push(byte as char);
            self.bump();
        }

        if digits.is_empty() {
            return Err(LexError {
                message: "integer literal has no digits".into(),
                span,
            });
        }

        u64::from_str_radix(&digits, radix)
            .map(Tok::Int)
            .map_err(|_| LexError {
                message: format!("integer literal `{digits}` does not fit in 64 bits"),
                span,
            })
    }

    fn lex_escape(&mut self) -> Result<u8, LexError> {
        let Some(byte) = self.bump() else {
            return self.error("unterminated escape sequence");
        };
        Ok(match byte {
            b'n' => 10,
            b'r' => 13,
            b't' => 9,
            b'0' => 0,
            b'e' => 27,
            b'\\' => 92,
            b'\'' => 39,
            b'"' => 34,
            b'x' => {
                let mut value = 0_u8;
                for _ in 0..2 {
                    let Some(digit) = self.peek().and_then(|b| (b as char).to_digit(16)) else {
                        return self.error("`\\x` needs two hex digits");
                    };
                    self.bump();
                    value = value * 16 + digit as u8;
                }
                value
            }
            other => return self.error(format!("unknown escape `\\{}`", other as char)),
        })
    }

    fn lex_string(&mut self) -> Result<Vec<u8>, LexError> {
        let start = self.span();
        self.bump();
        let mut bytes = Vec::new();
        loop {
            match self.peek() {
                None | Some(b'\n') => {
                    return Err(LexError {
                        message: "unterminated string literal".into(),
                        span: start,
                    })
                }
                Some(b'"') => {
                    self.bump();
                    return Ok(bytes);
                }
                Some(b'\\') => {
                    self.bump();
                    bytes.push(self.lex_escape()?);
                }
                Some(byte) => {
                    self.bump();
                    bytes.push(byte);
                }
            }
        }
    }

    fn lex_char(&mut self) -> Result<u8, LexError> {
        self.bump();
        let value = match self.peek() {
            None | Some(b'\n') => return self.error("unterminated character literal"),
            Some(b'\\') => {
                self.bump();
                self.lex_escape()?
            }
            Some(byte) => {
                self.bump();
                byte
            }
        };
        if self.peek() != Some(b'\'') {
            return self.error("character literal must contain exactly one byte");
        }
        self.bump();
        Ok(value)
    }

    fn lex_punct(&mut self) -> Result<Tok, LexError> {
        let rest = &self.src[self.index..];
        for candidate in PUNCTUATION {
            if rest.starts_with(candidate.as_bytes()) {
                for _ in 0..candidate.len() {
                    self.bump();
                }
                return Ok(Tok::Punct(candidate));
            }
        }
        let byte = self.peek().expect("caller checked for a byte");
        self.error(format!("unexpected character `{}`", byte as char))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        tokenize(src)
            .expect("valid source")
            .into_iter()
            .map(|token| token.tok)
            .collect()
    }

    #[test]
    fn lexes_declarations() {
        assert_eq!(
            kinds("let x = 5;"),
            vec![
                Tok::Ident("let".into()),
                Tok::Ident("x".into()),
                Tok::Punct("="),
                Tok::Int(5),
                Tok::Punct(";"),
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn prefers_longest_operator() {
        assert_eq!(
            kinds("a <= b"),
            vec![
                Tok::Ident("a".into()),
                Tok::Punct("<="),
                Tok::Ident("b".into()),
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn resolves_escapes_and_radixes() {
        assert_eq!(
            kinds("\"a\\n\" 0xff 0b1010 '\\t'"),
            vec![
                Tok::Str(vec![97, 10]),
                Tok::Int(255),
                Tok::Int(10),
                Tok::Int(9),
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn skips_comments() {
        assert_eq!(
            kinds("// gone\n/* also gone */ 1"),
            vec![Tok::Int(1), Tok::Eof]
        );
    }

    #[test]
    fn reports_unterminated_string() {
        let err = tokenize("\"oops").expect_err("string is unterminated");
        assert!(err.message.contains("unterminated string"));
    }
}
