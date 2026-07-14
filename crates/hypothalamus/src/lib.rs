//! Hypothalamus' reusable compiler library.
//!
//! The crate exposes the pipeline stages used by the command-line compiler:
//! parsing Brainfuck source into a compact operation tree, optimizing that tree
//! into a backend-oriented intermediate representation, lowering it into LLVM
//! IR, and optionally driving external LLVM tools through reusable target
//! profiles.
//!
//! Typical library usage starts with [`bf::parse`] and passes the resulting
//! operations to [`llvm::generate_module`]:
//!
//! ```
//! use hypothalamus::bf;
//! use hypothalamus::llvm::{self, LlvmOptions};
//! use hypothalamus::DEFAULT_TAPE_SIZE;
//!
//! let ops = bf::parse(b"++.").expect("valid Brainfuck");
//! let ir = llvm::generate_module(
//!     &ops,
//!     &LlvmOptions {
//!         tape_size: DEFAULT_TAPE_SIZE,
//!         target_triple: None,
//!         source_filename: Some("example.bf".to_string()),
//!         bounds_check: false,
//!         runtime: hypothalamus::llvm::Runtime::Hosted,
//!     },
//! )
//! .expect("LLVM IR");
//!
//! assert!(ir.contains("define i32 @main()"));
//! ```
//!
//! Lower-level callers can use [`bf`], [`ir`], and [`llvm`] directly. Tooling
//! that wants Hypothalamus' command-line behavior without shelling out can use
//! [`driver`] with a [`target::TargetProfile`].

#![warn(missing_docs)]

/// Brainfuck parser and optimizer.
pub mod bf;

/// Compiler-driver configuration and LLVM tool invocation.
pub mod driver;

/// Toolchain and target diagnostics.
pub mod diagnostics;

/// Optimized intermediate representation and Brainfuck-specific optimization.
pub mod ir;

/// LLVM IR backend for parsed Brainfuck operations.
pub mod llvm;

mod runner;
mod tool;

/// Named target profiles and runtime ABI defaults.
pub mod target;

/// Target-specific complete-image builders.
pub mod targets;

pub use driver::{CompilerConfig, EmitKind, OptLevel, compile_to_llvm, compile_with_tools};
pub use target::{RuntimeAbi, TargetImageFormat, TargetProfile};

/// Default number of byte cells in the generated Brainfuck tape.
///
/// This matches the conventional 30,000-cell tape used by many Brainfuck
/// implementations. The parser itself is independent of the tape size; this
/// value is consumed by the LLVM backend when it declares the global tape.
pub const DEFAULT_TAPE_SIZE: usize = 30_000;
