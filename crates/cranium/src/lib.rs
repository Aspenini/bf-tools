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
//! A program can span several files: [`module`] resolves `import` by reading
//! each file once and splicing its items into one program, which is then
//! compiled exactly as a single file would be.
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
pub mod module;
pub mod num;
pub mod parser;

use std::fmt;
use std::path::Path;

pub use codegen::{compile, ArrayPlacement, CompileError, Output};
pub use module::{Loaded, Loader, SourceMap};

/// A problem in the source, located in whichever file it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// Name of the file the problem is in.
    pub file: String,
    /// Line and column, when the problem has a position.
    pub position: Option<(usize, usize)>,
    /// Human readable description.
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.position {
            Some((line, column)) => write!(f, "{}:{line}:{column}: {}", self.file, self.message),
            None => write!(f, "{}: {}", self.file, self.message),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    fn at(sources: &SourceMap, span: lexer::Span, message: impl Into<String>) -> Self {
        Self {
            file: sources.name(span.file).to_string(),
            position: Some((span.line, span.column)),
            message: message.into(),
        }
    }

    fn from_gather(sources: &SourceMap, entry: &str, err: module::GatherError) -> Self {
        match err {
            module::GatherError::Lex(err) => Self::at(sources, err.span, err.message),
            module::GatherError::Parse(err) => Self::at(sources, err.span, err.message),
            module::GatherError::Load(err) => match err.span {
                Some(span) => Self::at(sources, span, err.message),
                None => Self {
                    file: entry.to_string(),
                    position: None,
                    message: err.message,
                },
            },
        }
    }
}

/// Compile a program that starts at `entry`, following its imports.
///
/// # Errors
///
/// Returns the first problem found, naming the file it is in.
pub fn compile_with(entry: &str, loader: &mut dyn Loader) -> Result<Output, Error> {
    // A failed gather still hands back the files it read, so an error inside an
    // imported file names that file rather than the entry point.
    let loaded = match module::gather(entry, loader) {
        Ok(loaded) => loaded,
        Err(failure) => {
            return Err(Error::from_gather(&failure.sources, entry, failure.error));
        }
    };
    codegen::compile(&loaded.program)
        .map_err(|err| Error::at(&loaded.sources, err.span, err.message))
}

/// Compile the file at `path`, following its imports.
///
/// # Errors
///
/// Returns the first problem found, naming the file it is in.
pub fn compile_file(path: impl AsRef<Path>) -> Result<Output, Error> {
    let path = path.as_ref();
    let entry = module::Disk::entry_name(path);
    compile_with(&entry, &mut module::Disk)
}

/// Compile a single detached source string.
///
/// The source has no file to resolve imports against, so an `import` in it is
/// an error; use [`compile_file`] or [`compile_with`] for a project.
///
/// # Errors
///
/// Returns the first lexical, syntactic, or semantic problem found.
pub fn compile_str(source: &str) -> Result<Output, Error> {
    const NAME: &str = "<source>";
    let mut loader = module::Memory::new([(NAME, source)]);
    compile_with(NAME, &mut loader)
}
