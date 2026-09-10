//! A window for Brainfuck programs.
//!
//! Brainfuck's whole interface is one byte out and one byte in, with no
//! syscall to reach for, which is why a Cranium program can emulate a terminal
//! but never open one. Occipital does not widen that interface — it reads it
//! differently. Bytes a program writes are text by default, drawn with a
//! built-in font, and a command when escaped with
//! [`protocol::DLE`]; bytes it reads are keystrokes or input events. Nothing
//! about the language or the compiler changes, so a program that draws a
//! window is still a `.bf` file that `lobe` can interpret.
//!
//! ```no_run
//! use occipital::{Options, Program};
//!
//! let program = Program::load("game.cra")?;
//! program.run(&Options {
//!     title: "game".to_string(),
//!     ..Options::default()
//! })?;
//! # Ok::<(), occipital::Error>(())
//! ```
//!
//! # The screen
//!
//! 256x192 pixels, 256 palette entries, every coordinate one byte. That is
//! sized for the language rather than for nostalgia: a Brainfuck program adds
//! bytes cheaply and anything wider expensively, so a protocol it can drive at
//! speed is one whose arguments are all single bytes.
//!
//! See [`protocol`] for the commands and [`screen`] for what they draw.

#![warn(missing_docs)]

pub mod console;
pub mod font;
pub mod headless;
pub mod protocol;
pub mod screen;

pub use console::{Console, Options};
pub use headless::Headless;
pub use protocol::{Command, Decoder, InputMode};
pub use screen::Screen;

use hypothalamus::bf;
use hypothalamus::codegen::CodegenOptions;
use hypothalamus::driver::OptLevel;
use hypothalamus::isa::IsaOptions;
use hypothalamus::jit;
use std::fmt;
use std::path::{Path, PathBuf};

/// Something that stopped a program from running in a window.
#[derive(Debug)]
pub enum Error {
    /// The program file could not be read.
    Read {
        /// Path that could not be read.
        path: PathBuf,

        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// A Cranium source file did not compile.
    Cranium(cranium::Error),

    /// A Brainfuck source file did not parse.
    Syntax(bf::SyntaxError),

    /// The window could not be opened.
    Window(minifb::Error),

    /// The program could not be compiled for this machine, or failed while
    /// running.
    Run(jit::JitError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::Cranium(error) => write!(f, "{error}"),
            Self::Syntax(error) => write!(f, "syntax error: {error}"),
            Self::Window(error) => write!(f, "failed to open a window: {error}"),
            Self::Run(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<cranium::Error> for Error {
    fn from(error: cranium::Error) -> Self {
        Self::Cranium(error)
    }
}

impl From<minifb::Error> for Error {
    fn from(error: minifb::Error) -> Self {
        Self::Window(error)
    }
}

impl From<jit::JitError> for Error {
    fn from(error: jit::JitError) -> Self {
        Self::Run(error)
    }
}

/// A program ready to run, parsed and optimized but not yet compiled.
#[derive(Debug, Clone)]
pub struct Program {
    ops: Vec<bf::Op>,
    name: String,
}

impl Program {
    /// Load a program from disk.
    ///
    /// A `.cra` file is compiled to Brainfuck first, so a Cranium program runs
    /// in one step; anything else is read as Brainfuck.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Read`] if the file cannot be read, [`Error::Cranium`]
    /// if a Cranium source does not compile, and [`Error::Syntax`] if the
    /// Brainfuck has unbalanced brackets.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "program".to_string());

        let source = if path.extension().is_some_and(|extension| extension == "cra") {
            cranium::compile_file(path)?.code.into_bytes()
        } else {
            std::fs::read(path).map_err(|source| Error::Read {
                path: path.to_path_buf(),
                source,
            })?
        };

        Ok(Self {
            ops: bf::parse(&source).map_err(Error::Syntax)?,
            name,
        })
    }

    /// Build a program from Brainfuck source already in memory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Syntax`] if the brackets do not balance.
    pub fn from_brainfuck(source: &[u8], name: impl Into<String>) -> Result<Self, Error> {
        Ok(Self {
            ops: bf::parse(source).map_err(Error::Syntax)?,
            name: name.into(),
        })
    }

    /// The program's name, taken from its file.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The parsed operations, for a caller that wants to run them itself.
    pub fn ops(&self) -> &[bf::Op] {
        &self.ops
    }

    /// Open a window, compile the program for this machine, and run it.
    ///
    /// Returns once the program ends and — unless
    /// [`Options::wait_on_exit`] is off — the window has been closed.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Window`] if no window could be opened, and
    /// [`Error::Run`] if the program could not be compiled for this machine or
    /// the window failed while it ran.
    pub fn run(&self, options: &Options) -> Result<(), Error> {
        let mut console = Console::open(options)?;

        let result = jit::run(
            &self.ops,
            &CodegenOptions::for_jit(),
            IsaOptions {
                opt_level: OptLevel::Speed,
                position_independent: false,
            },
            &mut console,
        );

        // The final frame is usually the interesting one, so hold the window
        // open even when the program stopped badly.
        console.finish();
        result?;
        Ok(())
    }
}
