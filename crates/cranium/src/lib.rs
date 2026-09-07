//! Cranium: a small structured language that compiles to Brainfuck.
//!
//! Cranium exists so that real programs can be *written* for the Brainfuck
//! runtimes in this workspace. It has variables, arrays, signed and unsigned
//! arithmetic, comparisons, structured control flow, and functions, and it
//! lowers all of that onto a flat byte tape.
//!
//! ```
//! let bf = cranium::compile_str("fn main() { print(6 * 7); }").expect("valid program");
//! assert!(bf.code.contains('.'));
//! ```
//!
//! The pipeline is [`lexer::tokenize`], [`parser::parse`], then
//! [`codegen::compile`]. Programs are lowered by inlining every call, so
//! recursion is rejected at compile time; everything else in the language works
//! the way it looks like it works.

#![warn(missing_docs)]

pub mod array;
pub mod ast;
pub mod bf;
pub mod codegen;
pub mod lexer;
pub mod num;
pub mod parser;

use std::fmt;

pub use codegen::{compile, ArrayPlacement, CompileError, Output};

/// Anything that can go wrong while compiling Cranium source.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// The source could not be tokenized.
    Lex(lexer::LexError),
    /// The tokens did not form a valid program.
    Parse(parser::ParseError),
    /// The program parsed but could not be lowered.
    Compile(CompileError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Lex(err) => write!(f, "{err}"),
            Error::Parse(err) => write!(f, "{err}"),
            Error::Compile(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<lexer::LexError> for Error {
    fn from(err: lexer::LexError) -> Self {
        Error::Lex(err)
    }
}

impl From<parser::ParseError> for Error {
    fn from(err: parser::ParseError) -> Self {
        Error::Parse(err)
    }
}

impl From<CompileError> for Error {
    fn from(err: CompileError) -> Self {
        Error::Compile(err)
    }
}

/// Compile Cranium source text to Brainfuck.
///
/// # Errors
///
/// Returns the first lexical, syntactic, or semantic problem found.
pub fn compile_str(source: &str) -> Result<Output, Error> {
    let tokens = lexer::tokenize(source)?;
    let program = parser::parse(tokens)?;
    Ok(codegen::compile(&program)?)
}
