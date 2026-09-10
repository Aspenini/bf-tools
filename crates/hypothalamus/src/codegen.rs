//! Cranelift code generation for normalized Brainfuck operations.
//!
//! The backend builds a complete Cranelift module containing an internal byte
//! tape and either a hosted `main` function using libc I/O or a freestanding
//! entry point calling caller-supplied byte I/O hooks. It is generic over
//! [`Module`], so the same lowering serves object files
//! ([`cranelift_object::ObjectModule`]) and the in-process JIT
//! ([`cranelift_jit::JITModule`]).
//!
//! Before emission, parsed Brainfuck operations are lowered through
//! [`crate::ir`], so the generated program can use offset arithmetic, direct
//! clear stores, scan loops, and multiply-transfer loops.
//!
//! The generated program models Brainfuck cells as wrapping `i8` values and the
//! data pointer as a pointer-sized index into the tape. Pointer bounds are not
//! checked unless [`CodegenOptions::bounds_check`] is set, which follows the
//! conventional Brainfuck behavior described by the command-line README.

use crate::bf::Op;
use crate::ir::{self, Ir};
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    AbiParam, InstBuilder, MemFlagsData, Signature, TrapCode, Value, types,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, FuncId, Linkage, Module, ModuleError};
use target_lexicon::OperatingSystem;

/// Symbol name of the generated Brainfuck tape.
///
/// The tape has local linkage, so this name only ever appears in an object
/// file's symbol table as a private detail.
const TAPE_SYMBOL: &str = "__hypothalamus_tape";

/// `_O_BINARY`, the mode `_setmode` needs for byte-exact Windows streams.
const O_BINARY: i64 = 0x8000;

/// The trap Cranelift emits when a bounds-checked access leaves the tape.
const OUT_OF_BOUNDS: TrapCode = TrapCode::HEAP_OUT_OF_BOUNDS;

/// Options controlling Cranelift module generation.
///
/// These options describe the target-neutral pieces of the generated module.
/// The target itself, along with the optimization level, comes from the
/// [`Module`] the program is defined into.
#[derive(Debug, Clone)]
pub struct CodegenOptions {
    /// Number of byte cells allocated in the generated tape.
    ///
    /// This must be greater than zero. Use [`crate::DEFAULT_TAPE_SIZE`] for the
    /// conventional Brainfuck tape size.
    pub tape_size: usize,

    /// Emit runtime tape bounds checks before every cell access.
    ///
    /// When enabled, out-of-range data-pointer access traps. The default
    /// command-line behavior leaves this disabled to preserve traditional
    /// Brainfuck boundary behavior.
    pub bounds_check: bool,

    /// Put the hosted standard streams into binary mode on Windows.
    ///
    /// Windows opens the standard streams in text mode, which would turn every
    /// `.` of a newline into two bytes and make `,` stop at a `0x1a`.
    /// Brainfuck's streams are bytes, so an ahead-of-time hosted program calls
    /// `_setmode` on the way in. The JIT does its own byte-exact I/O and leaves
    /// this off.
    pub binary_stdio: bool,

    /// Runtime ABI used by the generated module.
    pub runtime: Runtime,
}

impl Default for CodegenOptions {
    fn default() -> Self {
        Self {
            tape_size: crate::DEFAULT_TAPE_SIZE,
            bounds_check: false,
            binary_stdio: true,
            runtime: Runtime::Hosted,
        }
    }
}

/// Runtime ABI used by generated modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Runtime {
    /// Emit a hosted C-style `main` and call libc `putchar` / `getchar`.
    Hosted,

    /// Emit a freestanding entry point and call externally supplied I/O hooks.
    Freestanding(FreestandingOptions),
}

/// Symbol names for freestanding module generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreestandingOptions {
    /// Name of the generated Brainfuck entry function.
    ///
    /// The function has signature `void ()`.
    pub entry_symbol: String,

    /// Name of the external byte-output hook.
    ///
    /// The hook must have signature `void (i8)`.
    pub putchar_symbol: String,

    /// Name of the external byte-input hook.
    ///
    /// The hook must have signature `i32 ()`, returning `-1` for EOF.
    pub getchar_symbol: String,
}

impl Default for FreestandingOptions {
    fn default() -> Self {
        Self {
            entry_symbol: "bf_main".to_string(),
            putchar_symbol: "bf_putchar".to_string(),
            getchar_symbol: "bf_getchar".to_string(),
        }
    }
}

/// Error returned when a module cannot be generated from valid operations.
///
/// Most semantic choices are already encoded in [`Op`]. Generation fails for
/// invalid backend configuration, or when Cranelift rejects the module.
#[derive(Debug)]
pub enum CodegenError {
    /// The configured tape has zero cells.
    ///
    /// A zero-length tape cannot support even an empty Brainfuck program because
    /// generated operations address cells through the tape object.
    InvalidTapeSize(usize),

    /// A requested runtime symbol name cannot be emitted as an object symbol.
    InvalidSymbolName {
        /// Human-readable role for the invalid symbol.
        kind: &'static str,

        /// Invalid symbol name.
        name: String,
    },

    /// Two runtime symbols were configured with the same name.
    DuplicateSymbolName(String),

    /// Cranelift rejected a declaration or definition.
    ///
    /// Boxed because `ModuleError` is far larger than every other variant.
    Module(Box<ModuleError>),
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTapeSize(size) => {
                write!(f, "tape size must be greater than zero, got {size}")
            }
            Self::InvalidSymbolName { kind, name } => {
                write!(f, "invalid {kind} symbol name `{name}`")
            }
            Self::DuplicateSymbolName(name) => {
                write!(f, "runtime symbol name `{name}` is used more than once")
            }
            Self::Module(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CodegenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Module(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ModuleError> for CodegenError {
    fn from(error: ModuleError) -> Self {
        Self::Module(Box::new(error))
    }
}

/// Define a complete Brainfuck program in `module` and return its entry point.
///
/// The tape and the entry function are declared and defined; the I/O hooks are
/// declared as imports for the linker or the JIT to resolve. The caller decides
/// what happens next: [`cranelift_object::ObjectModule::finish`] writes an
/// object file, [`cranelift_jit::JITModule::finalize_definitions`] makes the
/// code callable.
///
/// # Errors
///
/// Returns [`CodegenError::InvalidTapeSize`] if [`CodegenOptions::tape_size`]
/// is zero, [`CodegenError::InvalidSymbolName`] or
/// [`CodegenError::DuplicateSymbolName`] for unusable freestanding symbol
/// names, and [`CodegenError::Module`] if Cranelift rejects the module.
///
/// # Examples
///
/// ```
/// use cranelift_codegen::settings;
/// use cranelift_module::{default_libcall_names, Module};
/// use cranelift_object::{ObjectBuilder, ObjectModule};
/// use hypothalamus::bf;
/// use hypothalamus::codegen::{self, CodegenOptions};
///
/// let isa = cranelift_native::builder()
///     .expect("host is supported")
///     .finish(settings::Flags::new(settings::builder()))
///     .expect("valid flags");
/// let mut module = ObjectModule::new(
///     ObjectBuilder::new(isa, "hello", default_libcall_names()).expect("object builder"),
/// );
///
/// let ops = bf::parse(b"++.").expect("valid Brainfuck");
/// codegen::define_program(&mut module, &ops, &CodegenOptions::default())
///     .expect("Brainfuck lowers to Cranelift IR");
///
/// let object = module.finish().emit().expect("object bytes");
/// assert!(!object.is_empty());
/// ```
pub fn define_program<M: Module>(
    module: &mut M,
    ops: &[Op],
    options: &CodegenOptions,
) -> Result<FuncId, CodegenError> {
    if options.tape_size == 0 {
        return Err(CodegenError::InvalidTapeSize(options.tape_size));
    }
    validate_runtime(&options.runtime)?;

    let optimized = ir::optimize(ops);
    let symbols = Symbols::new(&options.runtime);

    let mut tape = DataDescription::new();
    tape.define_zeroinit(options.tape_size);
    let tape_id = module.declare_data(TAPE_SYMBOL, Linkage::Local, true, false)?;
    module.define_data(tape_id, &tape)?;

    let call_conv = module.target_config().default_call_conv;
    let putchar_id = module.declare_function(
        symbols.putchar,
        Linkage::Import,
        &putchar_signature(call_conv, &options.runtime),
    )?;
    let getchar_id = module.declare_function(
        symbols.getchar,
        Linkage::Import,
        &getchar_signature(call_conv),
    )?;

    let entry_signature = entry_signature(call_conv, &options.runtime);
    let entry_id = module.declare_function(symbols.entry, Linkage::Export, &entry_signature)?;

    // Windows text-mode streams only matter to a hosted program that talks to
    // the C runtime; a freestanding one has no C runtime to correct.
    let setmode = if options.binary_stdio
        && options.runtime == Runtime::Hosted
        && module.isa().triple().operating_system == OperatingSystem::Windows
    {
        Some(module.declare_function("_setmode", Linkage::Import, &setmode_signature(call_conv))?)
    } else {
        None
    };

    let pointer_type = module.target_config().pointer_type();
    let frontend_config = module.target_config();

    let mut context = module.make_context();
    context.func.signature = entry_signature;

    let mut function_context = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut context.func, &mut function_context);
        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        // The entry block dominates the whole function, so the tape address and
        // the hook references it sets up stay usable from every later block.
        let tape_value = module.declare_data_in_func(tape_id, builder.func);
        let tape_base = builder.ins().symbol_value(pointer_type, tape_value);
        let putchar = module.declare_func_in_func(putchar_id, builder.func);
        let getchar = module.declare_func_in_func(getchar_id, builder.func);

        let pointer = builder.declare_var(pointer_type);
        let zero = builder.ins().iconst(pointer_type, 0);
        builder.def_var(pointer, zero);

        if let Some(setmode) = setmode {
            let setmode = module.declare_func_in_func(setmode, builder.func);
            for descriptor in [0, 1] {
                let descriptor = builder.ins().iconst(types::I32, descriptor);
                let mode = builder.ins().iconst(types::I32, O_BINARY);
                builder.ins().call(setmode, &[descriptor, mode]);
            }
        }

        let mut translator = Translator {
            builder,
            pointer,
            tape_base,
            tape_size: options.tape_size,
            bounds_check: options.bounds_check,
            putchar,
            getchar,
            hosted: options.runtime == Runtime::Hosted,
        };
        translator.translate_block(&optimized);

        let mut builder = translator.builder;
        match options.runtime {
            Runtime::Hosted => {
                let status = builder.ins().iconst(types::I32, 0);
                builder.ins().return_(&[status]);
            }
            Runtime::Freestanding(_) => {
                builder.ins().return_(&[]);
            }
        }
        builder.finalize(frontend_config);
    }

    module.define_function(entry_id, &mut context)?;
    module.clear_context(&mut context);

    Ok(entry_id)
}

/// Lowers optimized Brainfuck IR into the body of one Cranelift function.
struct Translator<'a> {
    builder: FunctionBuilder<'a>,
    pointer: Variable,
    tape_base: Value,
    tape_size: usize,
    bounds_check: bool,
    putchar: cranelift_codegen::ir::FuncRef,
    getchar: cranelift_codegen::ir::FuncRef,
    hosted: bool,
}

impl Translator<'_> {
    fn translate_block(&mut self, ops: &[Ir]) {
        for op in ops {
            match op {
                Ir::Add { offset, delta } => self.translate_add(*offset, *delta),
                Ir::Set { offset, value } => self.translate_set(*offset, *value),
                Ir::Move(delta) => self.translate_move(*delta),
                Ir::Input { offset } => self.translate_input(*offset),
                Ir::Output { offset } => self.translate_output(*offset),
                Ir::Loop(body) => self.translate_loop(body),
                Ir::Scan(stride) => self.translate_scan(*stride),
                Ir::AddMul { terms } => self.translate_add_mul(terms),
            }
        }
    }

    fn translate_add(&mut self, offset: i64, delta: i32) {
        let delta = i64::from(delta.rem_euclid(256));
        if delta == 0 {
            return;
        }

        let address = self.cell_address(offset);
        let current = self.load_cell(address);
        let next = self.builder.ins().iadd_imm_u(current, delta);
        self.store_cell(address, next);
    }

    fn translate_set(&mut self, offset: i64, value: u8) {
        let address = self.cell_address(offset);
        let value = self.builder.ins().iconst(types::I8, i64::from(value));
        self.store_cell(address, value);
    }

    fn translate_move(&mut self, delta: i64) {
        if delta == 0 {
            return;
        }

        let current = self.builder.use_var(self.pointer);
        let next = self.builder.ins().iadd_imm_s(current, delta);
        self.builder.def_var(self.pointer, next);
    }

    fn translate_input(&mut self, offset: i64) {
        let call = self.builder.ins().call(self.getchar, &[]);
        let byte = self.builder.inst_results(call)[0];
        let is_eof = self.builder.ins().icmp_imm_s(IntCC::Equal, byte, -1);

        let store_block = self.builder.create_block();
        let continue_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(is_eof, continue_block, &[], store_block, &[]);

        // End-of-file leaves the cell unchanged, so only the taken path stores.
        self.builder.switch_to_block(store_block);
        self.builder.seal_block(store_block);
        let truncated = self.builder.ins().ireduce(types::I8, byte);
        let address = self.cell_address(offset);
        self.store_cell(address, truncated);
        self.builder.ins().jump(continue_block, &[]);

        self.builder.switch_to_block(continue_block);
        self.builder.seal_block(continue_block);
    }

    fn translate_output(&mut self, offset: i64) {
        let address = self.cell_address(offset);
        let byte = self.load_cell(address);
        if self.hosted {
            let widened = self.builder.ins().uextend(types::I32, byte);
            self.builder.ins().call(self.putchar, &[widened]);
        } else {
            self.builder.ins().call(self.putchar, &[byte]);
        }
    }

    fn translate_loop(&mut self, body: &[Ir]) {
        let (header, body_block, exit) = self.open_loop();

        self.builder.switch_to_block(body_block);
        self.builder.seal_block(body_block);
        self.translate_block(body);

        self.close_loop(header, exit);
    }

    fn translate_scan(&mut self, stride: i64) {
        let (header, body_block, exit) = self.open_loop();

        self.builder.switch_to_block(body_block);
        self.builder.seal_block(body_block);
        self.translate_move(stride);

        self.close_loop(header, exit);
    }

    /// Emit a loop header that falls through to the body while the cell is set.
    ///
    /// The header stays unsealed until [`Self::close_loop`] adds the back edge.
    fn open_loop(
        &mut self,
    ) -> (
        cranelift_codegen::ir::Block,
        cranelift_codegen::ir::Block,
        cranelift_codegen::ir::Block,
    ) {
        let header = self.builder.create_block();
        let body_block = self.builder.create_block();
        let exit = self.builder.create_block();

        self.builder.ins().jump(header, &[]);
        self.builder.switch_to_block(header);

        let address = self.cell_address(0);
        let cell = self.load_cell(address);
        let is_zero = self.builder.ins().icmp_imm_s(IntCC::Equal, cell, 0);
        self.builder.ins().brif(is_zero, exit, &[], body_block, &[]);

        (header, body_block, exit)
    }

    fn close_loop(
        &mut self,
        header: cranelift_codegen::ir::Block,
        exit: cranelift_codegen::ir::Block,
    ) {
        self.builder.ins().jump(header, &[]);
        // Both the entry edge and the back edge exist now.
        self.builder.seal_block(header);

        self.builder.switch_to_block(exit);
        self.builder.seal_block(exit);
    }

    fn translate_add_mul(&mut self, terms: &[(i64, i32)]) {
        let source_address = self.cell_address(0);
        let source = self.load_cell(source_address);

        for (offset, factor) in terms {
            let factor = i64::from(factor.rem_euclid(256));
            if factor == 0 {
                continue;
            }

            let product = if factor == 1 {
                source
            } else {
                self.builder.ins().imul_imm_u(source, factor)
            };

            let address = self.cell_address(*offset);
            let current = self.load_cell(address);
            let next = self.builder.ins().iadd(current, product);
            self.store_cell(address, next);
        }

        let zero = self.builder.ins().iconst(types::I8, 0);
        self.store_cell(source_address, zero);
    }

    /// Compute the address of the cell `offset` cells from the data pointer.
    fn cell_address(&mut self, offset: i64) -> Value {
        let pointer = self.builder.use_var(self.pointer);
        let index = if offset == 0 {
            pointer
        } else {
            self.builder.ins().iadd_imm_s(pointer, offset)
        };

        if self.bounds_check {
            // An underflowed index wraps to a very large unsigned value, so one
            // unsigned comparison catches both ends of the tape.
            let out_of_range = self.builder.ins().icmp_imm_u(
                IntCC::UnsignedGreaterThanOrEqual,
                index,
                self.tape_size as i64,
            );
            self.builder.ins().trapnz(out_of_range, OUT_OF_BOUNDS);
        }

        self.builder.ins().iadd(self.tape_base, index)
    }

    fn load_cell(&mut self, address: Value) -> Value {
        self.builder
            .ins()
            .load(types::I8, MemFlagsData::trusted(), address, 0)
    }

    fn store_cell(&mut self, address: Value, value: Value) {
        self.builder
            .ins()
            .store(MemFlagsData::trusted(), value, address, 0);
    }
}

/// The three symbol names a generated module refers to.
struct Symbols<'a> {
    entry: &'a str,
    putchar: &'a str,
    getchar: &'a str,
}

impl<'a> Symbols<'a> {
    fn new(runtime: &'a Runtime) -> Self {
        match runtime {
            Runtime::Hosted => Self {
                entry: "main",
                putchar: "putchar",
                getchar: "getchar",
            },
            Runtime::Freestanding(options) => Self {
                entry: &options.entry_symbol,
                putchar: &options.putchar_symbol,
                getchar: &options.getchar_symbol,
            },
        }
    }
}

fn entry_signature(call_conv: cranelift_codegen::isa::CallConv, runtime: &Runtime) -> Signature {
    let mut signature = Signature::new(call_conv);
    if matches!(runtime, Runtime::Hosted) {
        signature.returns.push(AbiParam::new(types::I32));
    }
    signature
}

fn putchar_signature(call_conv: cranelift_codegen::isa::CallConv, runtime: &Runtime) -> Signature {
    let mut signature = Signature::new(call_conv);
    match runtime {
        // libc: `int putchar(int)`.
        Runtime::Hosted => {
            signature.params.push(AbiParam::new(types::I32));
            signature.returns.push(AbiParam::new(types::I32));
        }
        // Freestanding hook: `void bf_putchar(unsigned char)`.
        Runtime::Freestanding(_) => signature.params.push(AbiParam::new(types::I8)),
    }
    signature
}

/// `int getchar(void)`, and the freestanding hook that mirrors it.
fn getchar_signature(call_conv: cranelift_codegen::isa::CallConv) -> Signature {
    let mut signature = Signature::new(call_conv);
    signature.returns.push(AbiParam::new(types::I32));
    signature
}

/// `int _setmode(int fd, int mode)`.
fn setmode_signature(call_conv: cranelift_codegen::isa::CallConv) -> Signature {
    let mut signature = Signature::new(call_conv);
    signature.params.push(AbiParam::new(types::I32));
    signature.params.push(AbiParam::new(types::I32));
    signature.returns.push(AbiParam::new(types::I32));
    signature
}

fn validate_runtime(runtime: &Runtime) -> Result<(), CodegenError> {
    let Runtime::Freestanding(options) = runtime else {
        return Ok(());
    };

    for (kind, name) in [
        ("entry", &options.entry_symbol),
        ("putchar", &options.putchar_symbol),
        ("getchar", &options.getchar_symbol),
    ] {
        if !is_valid_symbol_name(name) {
            return Err(CodegenError::InvalidSymbolName {
                kind,
                name: name.clone(),
            });
        }
    }

    for (first, second) in [
        (&options.entry_symbol, &options.putchar_symbol),
        (&options.entry_symbol, &options.getchar_symbol),
        (&options.putchar_symbol, &options.getchar_symbol),
    ] {
        if first == second {
            return Err(CodegenError::DuplicateSymbolName(first.clone()));
        }
    }

    Ok(())
}

/// Accept the symbol names an assembler and a linker will both take verbatim.
fn is_valid_symbol_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|character: char| character.is_ascii_digit())
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_.$".contains(character))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bf;
    use cranelift_codegen::settings;
    use cranelift_module::default_libcall_names;
    use cranelift_object::{ObjectBuilder, ObjectModule};

    fn object_module(name: &str) -> ObjectModule {
        let isa = cranelift_native::builder()
            .expect("host should be a supported Cranelift target")
            .finish(settings::Flags::new(settings::builder()))
            .expect("default flags should be valid");
        ObjectModule::new(
            ObjectBuilder::new(isa, name, default_libcall_names()).expect("object builder"),
        )
    }

    fn compile(source: &[u8], options: &CodegenOptions) -> Result<Vec<u8>, CodegenError> {
        let ops = bf::parse(source).expect("valid Brainfuck");
        let mut module = object_module("test");
        define_program(&mut module, &ops, options)?;
        Ok(module.finish().emit().expect("object should emit"))
    }

    #[test]
    fn lowers_hello_world_to_an_object() {
        let object = compile(
            b"++++++++[>++++[>++>+++>+++>+<<<<-]>+>+>->>+[<]<-]>>.",
            &CodegenOptions::default(),
        )
        .expect("program should lower");

        assert!(!object.is_empty());
    }

    #[test]
    fn lowers_every_optimized_operation() {
        // Scan, multiply-transfer, clear, a general loop, and both I/O ops.
        let object = compile(b"+[>]+++[->+++<]>[-],.", &CodegenOptions::default())
            .expect("program should lower");

        assert!(!object.is_empty());
    }

    #[test]
    fn lowers_bounds_checked_programs() {
        let options = CodegenOptions {
            bounds_check: true,
            ..CodegenOptions::default()
        };

        assert!(
            !compile(b"+[>]<-.", &options)
                .expect("program should lower")
                .is_empty()
        );
    }

    #[test]
    fn lowers_freestanding_programs() {
        let options = CodegenOptions {
            runtime: Runtime::Freestanding(FreestandingOptions::default()),
            ..CodegenOptions::default()
        };

        assert!(
            !compile(b",+.", &options)
                .expect("program should lower")
                .is_empty()
        );
    }

    #[test]
    fn rejects_an_empty_tape() {
        let options = CodegenOptions {
            tape_size: 0,
            ..CodegenOptions::default()
        };

        assert!(matches!(
            compile(b"+", &options),
            Err(CodegenError::InvalidTapeSize(0))
        ));
    }

    #[test]
    fn rejects_unusable_symbol_names() {
        let options = CodegenOptions {
            runtime: Runtime::Freestanding(FreestandingOptions {
                entry_symbol: "kernel main".to_string(),
                ..FreestandingOptions::default()
            }),
            ..CodegenOptions::default()
        };

        assert!(matches!(
            compile(b"+", &options),
            Err(CodegenError::InvalidSymbolName { kind: "entry", .. })
        ));
    }

    #[test]
    fn rejects_repeated_symbol_names() {
        let options = CodegenOptions {
            runtime: Runtime::Freestanding(FreestandingOptions {
                entry_symbol: "bf_io".to_string(),
                putchar_symbol: "bf_io".to_string(),
                ..FreestandingOptions::default()
            }),
            ..CodegenOptions::default()
        };

        assert!(matches!(
            compile(b"+", &options),
            Err(CodegenError::DuplicateSymbolName(name)) if name == "bf_io"
        ));
    }

    #[test]
    fn accepts_the_symbol_names_linkers_accept() {
        assert!(is_valid_symbol_name("bf_main"));
        assert!(is_valid_symbol_name("_start"));
        assert!(is_valid_symbol_name("kernel.io$write"));
        assert!(!is_valid_symbol_name(""));
        assert!(!is_valid_symbol_name("2fast"));
        assert!(!is_valid_symbol_name("has space"));
    }
}
