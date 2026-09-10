//! In-process execution of compiled Brainfuck through the Cranelift JIT.
//!
//! `--emit jit` compiles the program for the host exactly as `--emit exe` does
//! and then calls it, so JIT execution and a compiled executable run the same
//! machine code. The only difference is who provides the two I/O hooks: instead
//! of linking against libc, the module's `putchar` and `getchar` imports are
//! bound to the Rust functions below.
//!
//! Those functions read and write bytes directly, which is why the JIT does not
//! need the Windows `_setmode` prologue that a hosted executable does.

use crate::bf::Op;
use crate::codegen::{self, CodegenError, CodegenOptions};
use crate::isa::{self, IsaError, IsaOptions};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{ModuleError, default_libcall_names};
use std::cell::RefCell;
use std::fmt;
use std::io::{self, BufWriter, Read, Stdin, Stdout, Write};

/// Error returned when a program cannot be JIT-compiled or fails during I/O.
#[derive(Debug)]
pub enum JitError {
    /// The host ISA could not be configured.
    Isa(IsaError),

    /// The program could not be lowered.
    Codegen(CodegenError),

    /// Cranelift could not finalize the compiled code.
    ///
    /// Boxed because `ModuleError` is far larger than every other variant.
    Module(Box<ModuleError>),

    /// Reading standard input failed.
    Input(io::Error),

    /// Writing standard output failed.
    Output(io::Error),
}

impl fmt::Display for JitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Isa(error) => write!(f, "{error}"),
            Self::Codegen(error) => write!(f, "{error}"),
            Self::Module(error) => write!(f, "{error}"),
            Self::Input(error) => write!(f, "failed to read stdin: {error}"),
            Self::Output(error) => write!(f, "failed to write stdout: {error}"),
        }
    }
}

impl std::error::Error for JitError {}

impl From<IsaError> for JitError {
    fn from(error: IsaError) -> Self {
        Self::Isa(error)
    }
}

impl From<CodegenError> for JitError {
    fn from(error: CodegenError) -> Self {
        Self::Codegen(error)
    }
}

impl From<ModuleError> for JitError {
    fn from(error: ModuleError) -> Self {
        Self::Module(Box::new(error))
    }
}

/// Compile `ops` for the host and run it, returning the program's exit status.
///
/// `options` must use the hosted runtime; the freestanding ABI has no I/O hooks
/// for the JIT to supply. The driver rejects that combination before it gets
/// here.
pub(crate) fn run(
    ops: &[Op],
    options: &CodegenOptions,
    isa_options: IsaOptions,
) -> Result<i32, JitError> {
    debug_assert_eq!(options.runtime, codegen::Runtime::Hosted);

    let isa = isa::build(
        None,
        IsaOptions {
            // Code compiled straight into this process is loaded at a known
            // address and resolves its imports through the JIT's own symbol
            // table, so cranelift-jit requires non-relocatable code.
            position_independent: false,
            ..isa_options
        },
    )?;
    let mut builder = JITBuilder::with_isa(isa, default_libcall_names());
    builder.symbol("putchar", jit_putchar as *const u8);
    builder.symbol("getchar", jit_getchar as *const u8);

    let mut module = JITModule::new(builder);
    let entry = codegen::define_program(&mut module, ops, options)?;
    module.finalize_definitions()?;

    let code = module.get_finalized_function(entry);
    // SAFETY: `entry` was defined above with the hosted signature, `() -> i32`,
    // using the module's own default (C) calling convention, and the code it
    // points at stays mapped for as long as `module` lives.
    let program: extern "C" fn() -> i32 = unsafe { std::mem::transmute(code) };

    let status = program();

    // The program's last write may still be sitting in the buffer, and an
    // error there is the program's error, not a clean exit.
    flush_output().map_err(JitError::Output)?;
    take_io_error(status)
}

thread_local! {
    static OUTPUT: RefCell<BufWriter<Stdout>> = RefCell::new(BufWriter::new(io::stdout()));
    static INPUT: RefCell<Stdin> = RefCell::new(io::stdin());

    /// The first I/O error the running program hit.
    ///
    /// The hooks match C's `putchar` and `getchar` signatures, which have no
    /// way to report why they failed, so the error is parked here and raised
    /// once the program returns.
    static IO_ERROR: RefCell<Option<JitError>> = const { RefCell::new(None) };
}

/// `int putchar(int)`, writing the low byte and never touching the C runtime.
extern "C" fn jit_putchar(byte: i32) -> i32 {
    let result = OUTPUT.with(|output| output.borrow_mut().write_all(&[byte as u8]));
    match result {
        Ok(()) => byte,
        Err(error) => {
            record_error(JitError::Output(error));
            -1
        }
    }
}

/// `int getchar(void)`, returning `-1` at end of file.
extern "C" fn jit_getchar() -> i32 {
    // Anything the program has already printed should be visible before it
    // blocks waiting for a reply.
    if let Err(error) = flush_output() {
        record_error(JitError::Output(error));
        return -1;
    }

    let mut byte = [0];
    loop {
        let result = INPUT.with(|input| input.borrow_mut().read(&mut byte));
        match result {
            Ok(0) => return -1,
            Ok(_) => return i32::from(byte[0]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                record_error(JitError::Input(error));
                return -1;
            }
        }
    }
}

fn flush_output() -> io::Result<()> {
    OUTPUT.with(|output| output.borrow_mut().flush())
}

fn record_error(error: JitError) {
    IO_ERROR.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(error);
        }
    });
}

/// Return the exit status, unless an I/O hook parked an error along the way.
fn take_io_error(status: i32) -> Result<i32, JitError> {
    match IO_ERROR.with(|slot| slot.borrow_mut().take()) {
        Some(error) => Err(error),
        None => Ok(status),
    }
}
