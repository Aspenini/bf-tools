//! Hypothalamus' reusable compiler library.
//!
//! The crate exposes the pipeline stages used by the command-line compiler:
//! parsing Brainfuck source into a compact operation tree, optimizing that tree
//! into a backend-oriented intermediate representation, lowering it into
//! Cranelift IR, and emitting an object file or running the result in process.
//!
//! Cranelift is compiled into the binary, so code generation needs no external
//! toolchain. The one thing that still does is linking an executable, which is
//! why [`driver::compile`] may look for a C compiler driver and
//! [`driver::compile_to_object`] never does.
//!
//! Typical library usage goes through [`driver`]:
//!
//! ```
//! use hypothalamus::{CompilerConfig, EmitKind, compile_to_object};
//!
//! let mut config = CompilerConfig::new("examples/hello.bf");
//! config.emit = EmitKind::Object;
//!
//! let object = compile_to_object(&config).expect("hello.bf compiles");
//! assert!(!object.is_empty());
//! ```
//!
//! Lower-level callers can use [`bf`], [`ir`], and [`codegen`] directly.
//! [`codegen::define_program`] is generic over [`cranelift_module::Module`], so
//! it can define a Brainfuck program into any Cranelift module — including one
//! that already holds other code.
//!
//! # Targets
//!
//! Cranelift's backends cover x86-64, aarch64, riscv64, and s390x, so those are
//! the architectures Hypothalamus can reach. There is no 32-bit x86 or ARM
//! backend; [`isa::build`] reports that as
//! [`IsaError::UnsupportedTarget`](isa::IsaError::UnsupportedTarget) rather
//! than failing later.

#![warn(missing_docs)]

/// Brainfuck parser and optimizer.
pub mod bf;

/// Cranelift backend for parsed Brainfuck operations.
pub mod codegen;

/// Toolchain and target diagnostics.
pub mod diagnostics;

/// Compiler-driver configuration, object emission, and linking.
pub mod driver;

/// Optimized intermediate representation and Brainfuck-specific optimization.
pub mod ir;

/// Target ISA construction.
pub mod isa;

/// In-process execution through the Cranelift JIT.
pub mod jit;

/// Named target profiles and runtime ABI defaults.
pub mod target;

mod tool;

pub use driver::{CompilerConfig, EmitKind, OptLevel, compile, compile_to_object};
pub use target::{RuntimeAbi, TargetProfile};

/// Default number of byte cells in the generated Brainfuck tape.
///
/// This matches the conventional 30,000-cell tape used by many Brainfuck
/// implementations. The parser itself is independent of the tape size; this
/// value is consumed by the backend when it declares the tape.
pub const DEFAULT_TAPE_SIZE: usize = 30_000;
