//! In-process execution of compiled Brainfuck through the Cranelift JIT.
//!
//! `--emit jit` compiles the program for the host exactly as `--emit exe` does
//! and then calls it, so JIT execution and a compiled executable run the same
//! machine code. The only difference is who provides the two I/O hooks: instead
//! of linking against libc, the module's `putchar` and `getchar` imports are
//! bound to Rust functions that call a [`Host`].
//!
//! [`Stdio`] is the [`Host`] the command line uses, and it moves raw bytes,
//! which is why the JIT does not need the Windows `_setmode` prologue that a
//! hosted executable does. Another [`Host`] can put those bytes anywhere — a
//! window, a socket, a test buffer — which is how a Brainfuck program gets an
//! interface wider than a terminal without Brainfuck gaining an instruction.

use crate::bf::Op;
use crate::codegen::{self, CodegenError, CodegenOptions};
use crate::isa::{self, IsaError, IsaOptions};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{ModuleError, default_libcall_names};
use std::cell::Cell;
use std::fmt;
use std::io::{self, BufWriter, Read, Stdout, Write};

/// The byte I/O a JIT-compiled Brainfuck program is given.
///
/// A Brainfuck program's entire interface is one byte out (`.`) and one byte in
/// (`,`). Implementing this trait is how a caller decides what those bytes
/// mean.
pub trait Host {
    /// Handle one byte written by `.`.
    fn write(&mut self, byte: u8) -> io::Result<()>;

    /// Supply one byte for `,`, or `None` for end of input.
    ///
    /// Brainfuck leaves the current cell unchanged at end of input, so `None`
    /// is not an error and a program can keep running after it.
    fn read(&mut self) -> io::Result<Option<u8>>;

    /// Finish any buffered work, once the program returns.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<T: Host + ?Sized> Host for &mut T {
    fn write(&mut self, byte: u8) -> io::Result<()> {
        (**self).write(byte)
    }

    fn read(&mut self) -> io::Result<Option<u8>> {
        (**self).read()
    }

    fn flush(&mut self) -> io::Result<()> {
        (**self).flush()
    }
}

/// A [`Host`] wired to this process' standard input and output.
///
/// Output is buffered and flushed before every read, so a program that prompts
/// and then waits has its prompt on screen while it waits.
#[derive(Debug)]
pub struct Stdio {
    output: BufWriter<Stdout>,
}

impl Stdio {
    /// Create a host over the process' standard streams.
    pub fn new() -> Self {
        Self {
            output: BufWriter::new(io::stdout()),
        }
    }
}

impl Default for Stdio {
    fn default() -> Self {
        Self::new()
    }
}

impl Host for Stdio {
    fn write(&mut self, byte: u8) -> io::Result<()> {
        self.output.write_all(&[byte])
    }

    fn read(&mut self) -> io::Result<Option<u8>> {
        // Anything already printed should be visible before the program blocks
        // waiting for a reply.
        self.output.flush()?;

        let mut byte = [0];
        loop {
            match io::stdin().read(&mut byte) {
                Ok(0) => return Ok(None),
                Ok(_) => return Ok(Some(byte[0])),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

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

    /// The [`Host`] failed while the program was running.
    Host(io::Error),
}

impl fmt::Display for JitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Isa(error) => write!(f, "{error}"),
            Self::Codegen(error) => write!(f, "{error}"),
            Self::Module(error) => write!(f, "{error}"),
            Self::Host(error) => write!(f, "{error}"),
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

/// Compile `ops` for the host machine and run it against `host`.
///
/// Returns the program's exit status, which is `0` for every Brainfuck program
/// that runs to completion.
///
/// `options` must use the hosted runtime; the freestanding ABI names I/O hooks
/// for someone else to provide, so there is nothing for a [`Host`] to attach
/// to. [`crate::driver`] rejects that combination before it gets here.
///
/// # Errors
///
/// Returns [`JitError::Host`] if the [`Host`] failed, and the code generation
/// variants if the program could not be compiled for this machine.
///
/// # Examples
///
/// ```
/// use hypothalamus::bf;
/// use hypothalamus::codegen::CodegenOptions;
/// use hypothalamus::driver::OptLevel;
/// use hypothalamus::isa::IsaOptions;
/// use hypothalamus::jit::{self, Host};
/// use std::io;
///
/// /// Collects output and answers every read with the same byte.
/// struct Echo {
///     written: Vec<u8>,
/// }
///
/// impl Host for Echo {
///     fn write(&mut self, byte: u8) -> io::Result<()> {
///         self.written.push(byte);
///         Ok(())
///     }
///
///     fn read(&mut self) -> io::Result<Option<u8>> {
///         Ok(Some(b'A'))
///     }
/// }
///
/// let ops = bf::parse(b",+.").expect("valid Brainfuck");
/// let mut host = Echo { written: Vec::new() };
/// jit::run(
///     &ops,
///     &CodegenOptions::for_jit(),
///     IsaOptions { opt_level: OptLevel::Speed, position_independent: false },
///     &mut host,
/// )
/// .expect("program runs");
///
/// assert_eq!(host.written, b"B");
/// ```
pub fn run<H: Host>(
    ops: &[Op],
    options: &CodegenOptions,
    isa_options: IsaOptions,
    host: &mut H,
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

    let status = {
        let _installed = Installed::new(host);
        program()
    };

    // The host's last write may still be buffered, and an error there is the
    // program's error, not a clean exit.
    host.flush().map_err(JitError::Host)?;

    match take_error() {
        Some(error) => Err(JitError::Host(error)),
        None => Ok(status),
    }
}

/// The current thread's active host, as an untyped pointer plus the two shims
/// that know its real type.
///
/// The generated code calls `putchar` and `getchar` with C signatures and no
/// context argument, so the host has to be reachable from a thread-local. Only
/// the pointer is erased; the shims are monomorphized for the concrete host
/// type, so nothing here transmutes a lifetime or a trait object.
#[derive(Clone, Copy)]
struct Slot {
    host: *mut (),
    write: unsafe fn(*mut (), u8) -> io::Result<()>,
    read: unsafe fn(*mut ()) -> io::Result<Option<u8>>,
}

thread_local! {
    static SLOT: Cell<Option<Slot>> = const { Cell::new(None) };

    /// The first host error the running program hit.
    ///
    /// The hooks match C's `putchar` and `getchar` signatures, which have no
    /// way to report why they failed, so the error is parked here and raised
    /// once the program returns.
    static ERROR: Cell<Option<io::Error>> = const { Cell::new(None) };
}

/// Installs a host for the current thread and takes it back out on drop.
///
/// This is a guard rather than a pair of calls so that the pointer cannot
/// outlive the borrow even if the program panics on the way out.
struct Installed {
    previous: Option<Slot>,
}

impl Installed {
    fn new<H: Host>(host: &mut H) -> Self {
        unsafe fn write<H: Host>(host: *mut (), byte: u8) -> io::Result<()> {
            // SAFETY: `host` is the pointer `Installed::new` stored for this
            // same `H`, and the borrow it came from outlives the guard.
            unsafe { (*host.cast::<H>()).write(byte) }
        }

        unsafe fn read<H: Host>(host: *mut ()) -> io::Result<Option<u8>> {
            // SAFETY: as above.
            unsafe { (*host.cast::<H>()).read() }
        }

        let slot = Slot {
            host: (host as *mut H).cast::<()>(),
            write: write::<H>,
            read: read::<H>,
        };

        Self {
            previous: SLOT.replace(Some(slot)),
        }
    }
}

impl Drop for Installed {
    fn drop(&mut self) {
        SLOT.set(self.previous);
    }
}

/// `int putchar(int)`, handing the low byte to the installed host.
extern "C" fn jit_putchar(byte: i32) -> i32 {
    let Some(slot) = SLOT.get() else {
        return -1;
    };

    // SAFETY: the slot is live for exactly as long as the guard in `run`.
    match unsafe { (slot.write)(slot.host, byte as u8) } {
        Ok(()) => byte,
        Err(error) => {
            record(error);
            -1
        }
    }
}

/// `int getchar(void)`, returning `-1` at end of input.
extern "C" fn jit_getchar() -> i32 {
    let Some(slot) = SLOT.get() else {
        return -1;
    };

    // SAFETY: as above.
    match unsafe { (slot.read)(slot.host) } {
        Ok(Some(byte)) => i32::from(byte),
        Ok(None) => -1,
        Err(error) => {
            record(error);
            -1
        }
    }
}

fn record(error: io::Error) {
    if let Some(first) = ERROR.replace(Some(error)) {
        // Keep the first failure; it is the one that explains the rest.
        ERROR.set(Some(first));
    }
}

fn take_error() -> Option<io::Error> {
    ERROR.take()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bf;
    use crate::driver::OptLevel;

    /// A host over fixed input that records everything written.
    struct Buffer {
        input: Vec<u8>,
        cursor: usize,
        output: Vec<u8>,
        flushed: bool,
    }

    impl Buffer {
        fn new(input: &[u8]) -> Self {
            Self {
                input: input.to_vec(),
                cursor: 0,
                output: Vec::new(),
                flushed: false,
            }
        }
    }

    impl Host for Buffer {
        fn write(&mut self, byte: u8) -> io::Result<()> {
            self.output.push(byte);
            Ok(())
        }

        fn read(&mut self) -> io::Result<Option<u8>> {
            let byte = self.input.get(self.cursor).copied();
            if byte.is_some() {
                self.cursor += 1;
            }
            Ok(byte)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushed = true;
            Ok(())
        }
    }

    fn isa_options() -> IsaOptions {
        IsaOptions {
            opt_level: OptLevel::Speed,
            position_independent: false,
        }
    }

    fn run_buffered(source: &[u8], input: &[u8]) -> Buffer {
        let ops = bf::parse(source).expect("valid Brainfuck");
        let mut host = Buffer::new(input);

        let status = run(&ops, &CodegenOptions::for_jit(), isa_options(), &mut host)
            .expect("program should run");

        assert_eq!(status, 0);
        host
    }

    #[test]
    fn routes_output_to_the_host() {
        let host = run_buffered(b"++++++++[>++++++++<-]>+.", b"");

        assert_eq!(host.output, b"A");
        assert!(host.flushed);
    }

    #[test]
    fn routes_input_from_the_host() {
        assert_eq!(run_buffered(b",+.,+.", b"AB").output, b"BC");
    }

    #[test]
    fn end_of_input_leaves_the_cell_alone() {
        // `,` at end of input is not an error, and the cell keeps its value.
        assert_eq!(
            run_buffered(b"+++++++++++++++++++++++++++++++++,.", b"").output,
            b"!"
        );
    }

    #[test]
    fn reports_a_host_failure_once_the_program_returns() {
        struct Broken;

        impl Host for Broken {
            fn write(&mut self, _byte: u8) -> io::Result<()> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "no reader"))
            }

            fn read(&mut self) -> io::Result<Option<u8>> {
                Ok(None)
            }
        }

        let ops = bf::parse(b"+.").expect("valid Brainfuck");
        let error = run(&ops, &CodegenOptions::for_jit(), isa_options(), &mut Broken)
            .expect_err("a failing host should surface");

        assert!(
            matches!(&error, JitError::Host(error) if error.kind() == io::ErrorKind::BrokenPipe)
        );
    }

    #[test]
    fn a_host_can_be_used_behind_a_trait_object() {
        let ops = bf::parse(b"+.").expect("valid Brainfuck");
        let mut buffer = Buffer::new(b"");
        let mut host: &mut dyn Host = &mut buffer;

        run(&ops, &CodegenOptions::for_jit(), isa_options(), &mut host)
            .expect("program should run");

        assert_eq!(buffer.output, [1]);
    }

    #[test]
    fn the_thread_local_slot_is_cleared_afterwards() {
        run_buffered(b"+.", b"");

        assert!(SLOT.get().is_none());
        assert!(ERROR.take().is_none());
    }
}
