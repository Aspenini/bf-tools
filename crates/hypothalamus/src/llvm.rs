//! LLVM IR generation for normalized Brainfuck operations.
//!
//! The backend emits a complete textual LLVM module containing an internal
//! global byte tape and either a hosted `main` function using libc I/O or a
//! freestanding entry point using caller-provided I/O hook symbols. It does not
//! run LLVM tools itself; callers can write the returned string to a `.ll` file
//! or pass it to a compatible LLVM driver such as `clang`.
//!
//! Before emission, parsed Brainfuck operations are lowered through
//! [`crate::ir`], so the generated program can use offset arithmetic, direct
//! clear stores, scan loops, and multiply-transfer loops.
//!
//! The generated program models Brainfuck cells as wrapping `i8` values and the
//! data pointer as an `i64` offset into the global tape. Pointer bounds are not
//! checked, which follows the conventional Brainfuck behavior described by the
//! command-line README.

use crate::bf::Op;
use crate::ir::{self, Ir};
use std::fmt::Write;

/// Options controlling LLVM module generation.
///
/// These options describe the target-neutral pieces of the emitted module. They
/// do not configure the external LLVM driver; the command-line binary applies
/// related options again when it invokes `clang`.
#[derive(Debug, Clone)]
pub struct LlvmOptions {
    /// Number of byte cells allocated in the generated global tape.
    ///
    /// This must be greater than zero. Use [`crate::DEFAULT_TAPE_SIZE`] for the
    /// conventional Brainfuck tape size.
    pub tape_size: usize,

    /// Optional LLVM target triple embedded in the module.
    ///
    /// When present, this is emitted as `target triple = "..."`
    /// near the top of the LLVM IR. Leaving it as `None` allows downstream LLVM
    /// tools to infer their default target.
    pub target_triple: Option<String>,

    /// Optional source filename embedded in the module.
    ///
    /// When present, this is emitted as `source_filename = "..."`. The value is
    /// escaped for LLVM string syntax before it is written.
    pub source_filename: Option<String>,

    /// Emit runtime tape bounds checks before every cell access.
    ///
    /// When enabled, out-of-range data-pointer access calls LLVM's trap
    /// intrinsic. The default command-line behavior leaves this disabled to
    /// preserve traditional Brainfuck boundary behavior.
    pub bounds_check: bool,

    /// Runtime ABI used by the generated module.
    pub runtime: Runtime,
}

/// Runtime ABI used by generated LLVM modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Runtime {
    /// Emit a hosted C-style `main` and call libc `putchar` / `getchar`.
    Hosted,

    /// Emit a freestanding entry point and call externally supplied I/O hooks.
    Freestanding(FreestandingOptions),
}

/// Symbol names for freestanding LLVM module generation.
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

/// Error returned when LLVM IR cannot be generated from valid operations.
///
/// Most semantic choices are already encoded in [`Op`]. Generation currently
/// fails only for invalid backend configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodegenError {
    /// The configured tape has zero cells.
    ///
    /// A zero-length tape cannot support even an empty Brainfuck program because
    /// generated operations address cells through the global tape object.
    InvalidTapeSize(usize),

    /// A requested LLVM symbol name cannot be emitted as a simple identifier.
    InvalidSymbolName {
        /// Human-readable role for the invalid symbol.
        kind: &'static str,

        /// Invalid symbol name.
        name: String,
    },

    /// Two runtime symbols were configured with the same name.
    DuplicateSymbolName(String),
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
        }
    }
}

impl std::error::Error for CodegenError {}

/// Generate a complete textual LLVM module for `ops`.
///
/// The returned string is ready to be written as LLVM IR. For native code
/// generation, pass it to an LLVM-compatible toolchain such as `clang`.
///
/// # Errors
///
/// Returns [`CodegenError::InvalidTapeSize`] if
/// [`LlvmOptions::tape_size`] is zero.
///
/// # Examples
///
/// ```
/// use hypothalamus::bf;
/// use hypothalamus::llvm::{generate_module, LlvmOptions};
/// use hypothalamus::DEFAULT_TAPE_SIZE;
///
/// let ops = bf::parse(b"++.").expect("valid Brainfuck");
/// let ir = generate_module(
///     &ops,
///     &LlvmOptions {
///         tape_size: DEFAULT_TAPE_SIZE,
///         target_triple: Some("x86_64-unknown-linux-gnu".to_string()),
///         source_filename: Some("hello.bf".to_string()),
///         bounds_check: false,
///         runtime: hypothalamus::llvm::Runtime::Hosted,
///     },
/// )
/// .expect("LLVM IR");
///
/// assert!(ir.contains("; Generated by hypothalamus."));
/// assert!(ir.contains("define i32 @main()"));
/// ```
pub fn generate_module(ops: &[Op], options: &LlvmOptions) -> Result<String, CodegenError> {
    if options.tape_size == 0 {
        return Err(CodegenError::InvalidTapeSize(options.tape_size));
    }
    validate_runtime(&options.runtime)?;

    let optimized = ir::optimize(ops);
    let mut emitter = Emitter::new(options);
    emitter.emit_preamble();
    emitter.emit_main(&optimized);
    Ok(emitter.output)
}

struct Emitter<'a> {
    output: String,
    tape_size: usize,
    target_triple: Option<&'a str>,
    source_filename: Option<&'a str>,
    bounds_check: bool,
    runtime: &'a Runtime,
    temp_index: usize,
    label_index: usize,
}

impl<'a> Emitter<'a> {
    fn new(options: &'a LlvmOptions) -> Self {
        Self {
            output: String::new(),
            tape_size: options.tape_size,
            target_triple: options.target_triple.as_deref(),
            source_filename: options.source_filename.as_deref(),
            bounds_check: options.bounds_check,
            runtime: &options.runtime,
            temp_index: 0,
            label_index: 0,
        }
    }

    fn emit_preamble(&mut self) {
        self.line("; Generated by hypothalamus.");

        if let Some(source_filename) = self.source_filename {
            self.line(&format!(
                "source_filename = \"{}\"",
                escape_llvm_string(source_filename)
            ));
        }

        if let Some(target_triple) = self.target_triple {
            self.line(&format!(
                "target triple = \"{}\"",
                escape_llvm_string(target_triple)
            ));
        }

        self.blank_line();
        self.line(&format!(
            "@tape = internal global [{} x i8] zeroinitializer, align 16",
            self.tape_size
        ));
        self.blank_line();
        match self.runtime {
            Runtime::Hosted => {
                self.line("declare i32 @putchar(i32)");
                self.line("declare i32 @getchar()");
                if self.targets_windows() {
                    self.line("declare i32 @_setmode(i32, i32)");
                }
            }
            Runtime::Freestanding(options) => {
                self.line(&format!("declare void @{}(i8)", options.putchar_symbol));
                self.line(&format!("declare i32 @{}()", options.getchar_symbol));
            }
        }
        if self.bounds_check {
            self.line("declare void @llvm.trap()");
        }
        self.blank_line();
    }

    fn emit_main(&mut self, ops: &[Ir]) {
        match self.runtime {
            Runtime::Hosted => self.line("define i32 @main() {"),
            Runtime::Freestanding(options) => {
                self.line(&format!("define void @{}() {{", options.entry_symbol));
            }
        }
        self.label("entry");
        self.line("  %ptr = alloca i64, align 8");
        self.line("  store i64 0, ptr %ptr, align 8");
        if matches!(self.runtime, Runtime::Hosted) && self.targets_windows() {
            // Windows opens the standard streams in text mode, which would
            // turn every `.` of a newline into two bytes and make `,` stop at
            // a `0x1a`. Brainfuck's streams are bytes, so switch them back.
            // These registers are named so they do not disturb the numbering
            // of the unnamed ones the rest of the function relies on.
            self.line("  %stdin_mode = call i32 @_setmode(i32 0, i32 32768)");
            self.line("  %stdout_mode = call i32 @_setmode(i32 1, i32 32768)");
        }
        self.emit_ops(ops);
        match self.runtime {
            Runtime::Hosted => self.line("  ret i32 0"),
            Runtime::Freestanding(_) => self.line("  ret void"),
        }
        self.line("}");
    }

    fn emit_ops(&mut self, ops: &[Ir]) {
        for op in ops {
            match op {
                Ir::Add { offset, delta } => self.emit_add(*offset, *delta),
                Ir::Set { offset, value } => self.emit_set(*offset, *value),
                Ir::Move(delta) => self.emit_move(*delta),
                Ir::Input { offset } => self.emit_input(*offset),
                Ir::Output { offset } => self.emit_output(*offset),
                Ir::Loop(body) => self.emit_loop(body),
                Ir::Scan(stride) => self.emit_scan(*stride),
                Ir::AddMul { terms } => self.emit_add_mul(terms),
            }
        }
    }

    fn emit_add(&mut self, offset: i64, delta: i32) {
        let delta = delta.rem_euclid(256);
        if delta == 0 {
            return;
        }

        let cell_ptr = self.emit_cell_ptr(offset);
        let current = self.temp();
        self.line(&format!("  {current} = load i8, ptr {cell_ptr}, align 1"));
        let next = self.temp();
        self.line(&format!("  {next} = add i8 {current}, {delta}"));
        self.line(&format!("  store i8 {next}, ptr {cell_ptr}, align 1"));
    }

    fn emit_set(&mut self, offset: i64, value: u8) {
        let cell_ptr = self.emit_cell_ptr(offset);
        self.line(&format!("  store i8 {value}, ptr {cell_ptr}, align 1"));
    }

    fn emit_move(&mut self, delta: i64) {
        if delta == 0 {
            return;
        }

        let current = self.temp();
        self.line(&format!("  {current} = load i64, ptr %ptr, align 8"));
        let next = self.temp();
        self.line(&format!("  {next} = add i64 {current}, {delta}"));
        self.line(&format!("  store i64 {next}, ptr %ptr, align 8"));
    }

    fn emit_input(&mut self, offset: i64) {
        let byte = self.temp();
        match self.runtime {
            Runtime::Hosted => self.line(&format!("  {byte} = call i32 @getchar()")),
            Runtime::Freestanding(options) => {
                self.line(&format!(
                    "  {byte} = call i32 @{}()",
                    options.getchar_symbol
                ));
            }
        }
        let is_eof = self.temp();
        self.line(&format!("  {is_eof} = icmp eq i32 {byte}, -1"));

        let store_label = self.fresh_label("input_store");
        let cont_label = self.fresh_label("input_cont");
        self.line(&format!(
            "  br i1 {is_eof}, label %{cont_label}, label %{store_label}"
        ));

        self.label(&store_label);
        let truncated = self.temp();
        self.line(&format!("  {truncated} = trunc i32 {byte} to i8"));
        let cell_ptr = self.emit_cell_ptr(offset);
        self.line(&format!("  store i8 {truncated}, ptr {cell_ptr}, align 1"));
        self.line(&format!("  br label %{cont_label}"));

        self.label(&cont_label);
    }

    fn emit_output(&mut self, offset: i64) {
        let cell_ptr = self.emit_cell_ptr(offset);
        let byte = self.temp();
        self.line(&format!("  {byte} = load i8, ptr {cell_ptr}, align 1"));
        match self.runtime {
            Runtime::Hosted => {
                let widened = self.temp();
                self.line(&format!("  {widened} = zext i8 {byte} to i32"));
                let result = self.temp();
                self.line(&format!("  {result} = call i32 @putchar(i32 {widened})"));
            }
            Runtime::Freestanding(options) => {
                self.line(&format!(
                    "  call void @{}(i8 {byte})",
                    options.putchar_symbol
                ));
            }
        }
    }

    fn emit_loop(&mut self, body: &[Ir]) {
        let check_label = self.fresh_label("loop_check");
        let body_label = self.fresh_label("loop_body");
        let end_label = self.fresh_label("loop_end");

        self.line(&format!("  br label %{check_label}"));

        self.label(&check_label);
        let cell_ptr = self.emit_cell_ptr(0);
        let byte = self.temp();
        self.line(&format!("  {byte} = load i8, ptr {cell_ptr}, align 1"));
        let is_zero = self.temp();
        self.line(&format!("  {is_zero} = icmp eq i8 {byte}, 0"));
        self.line(&format!(
            "  br i1 {is_zero}, label %{end_label}, label %{body_label}"
        ));

        self.label(&body_label);
        self.emit_ops(body);
        self.line(&format!("  br label %{check_label}"));

        self.label(&end_label);
    }

    fn emit_scan(&mut self, stride: i64) {
        let check_label = self.fresh_label("scan_check");
        let body_label = self.fresh_label("scan_body");
        let end_label = self.fresh_label("scan_end");

        self.line(&format!("  br label %{check_label}"));

        self.label(&check_label);
        let cell_ptr = self.emit_cell_ptr(0);
        let byte = self.temp();
        self.line(&format!("  {byte} = load i8, ptr {cell_ptr}, align 1"));
        let is_zero = self.temp();
        self.line(&format!("  {is_zero} = icmp eq i8 {byte}, 0"));
        self.line(&format!(
            "  br i1 {is_zero}, label %{end_label}, label %{body_label}"
        ));

        self.label(&body_label);
        self.emit_move(stride);
        self.line(&format!("  br label %{check_label}"));

        self.label(&end_label);
    }

    fn emit_add_mul(&mut self, terms: &[(i64, i32)]) {
        let source_ptr = self.emit_cell_ptr(0);
        let source = self.temp();
        self.line(&format!("  {source} = load i8, ptr {source_ptr}, align 1"));

        for (offset, factor) in terms {
            let factor = factor.rem_euclid(256);
            if factor == 0 {
                continue;
            }

            let product = if factor == 1 {
                source.clone()
            } else {
                let product = self.temp();
                self.line(&format!("  {product} = mul i8 {source}, {factor}"));
                product
            };

            let cell_ptr = self.emit_cell_ptr(*offset);
            let current = self.temp();
            self.line(&format!("  {current} = load i8, ptr {cell_ptr}, align 1"));
            let next = self.temp();
            self.line(&format!("  {next} = add i8 {current}, {product}"));
            self.line(&format!("  store i8 {next}, ptr {cell_ptr}, align 1"));
        }

        self.line(&format!("  store i8 0, ptr {source_ptr}, align 1"));
    }

    fn emit_cell_ptr(&mut self, offset: i64) -> String {
        let pointer = self.temp();
        self.line(&format!("  {pointer} = load i64, ptr %ptr, align 8"));
        let pointer = if offset == 0 {
            pointer
        } else {
            let adjusted = self.temp();
            self.line(&format!("  {adjusted} = add i64 {pointer}, {offset}"));
            adjusted
        };
        if self.bounds_check {
            self.emit_bounds_check(&pointer);
        }
        let cell_ptr = self.temp();
        self.line(&format!(
            "  {cell_ptr} = getelementptr [{} x i8], ptr @tape, i64 0, i64 {pointer}",
            self.tape_size
        ));
        cell_ptr
    }

    fn emit_bounds_check(&mut self, pointer: &str) {
        let in_bounds = self.temp();
        self.line(&format!(
            "  {in_bounds} = icmp ult i64 {pointer}, {}",
            self.tape_size
        ));

        let trap_label = self.fresh_label("bounds_trap");
        let cont_label = self.fresh_label("bounds_cont");
        self.line(&format!(
            "  br i1 {in_bounds}, label %{cont_label}, label %{trap_label}"
        ));

        self.label(&trap_label);
        self.line("  call void @llvm.trap()");
        self.line("  unreachable");

        self.label(&cont_label);
    }

    /// Whether the program will run on Windows.
    ///
    /// A target with no explicit triple compiles for whatever host clang runs
    /// on, so the host decides.
    fn targets_windows(&self) -> bool {
        match self.target_triple {
            Some(triple) => triple.contains("windows"),
            None => cfg!(windows),
        }
    }

    fn temp(&mut self) -> String {
        let temp = format!("%{}", self.temp_index);
        self.temp_index += 1;
        temp
    }

    fn fresh_label(&mut self, prefix: &str) -> String {
        let label = format!("{prefix}_{}", self.label_index);
        self.label_index += 1;
        label
    }

    fn label(&mut self, label: &str) {
        self.line(&format!("{label}:"));
    }

    fn line(&mut self, line: &str) {
        self.output.push_str(line);
        self.output.push('\n');
    }

    fn blank_line(&mut self) {
        self.output.push('\n');
    }
}

fn escape_llvm_string(value: &str) -> String {
    let mut escaped = String::new();

    for byte in value.bytes() {
        match byte {
            b'"' => escaped.push_str("\\22"),
            b'\\' => escaped.push_str("\\5C"),
            0x20..=0x7e => escaped.push(byte as char),
            _ => {
                write!(&mut escaped, "\\{byte:02X}").expect("write to string");
            }
        }
    }

    escaped
}

fn validate_runtime(runtime: &Runtime) -> Result<(), CodegenError> {
    let Runtime::Freestanding(options) = runtime else {
        return Ok(());
    };

    validate_symbol_name("entry", &options.entry_symbol)?;
    validate_symbol_name("putchar", &options.putchar_symbol)?;
    validate_symbol_name("getchar", &options.getchar_symbol)?;

    if options.entry_symbol == options.putchar_symbol
        || options.entry_symbol == options.getchar_symbol
    {
        return Err(CodegenError::DuplicateSymbolName(
            options.entry_symbol.clone(),
        ));
    }
    if options.putchar_symbol == options.getchar_symbol {
        return Err(CodegenError::DuplicateSymbolName(
            options.putchar_symbol.clone(),
        ));
    }

    Ok(())
}

fn validate_symbol_name(kind: &'static str, name: &str) -> Result<(), CodegenError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(CodegenError::InvalidSymbolName {
            kind,
            name: name.to_string(),
        });
    };

    if !is_symbol_start(first) || !chars.all(is_symbol_continue) {
        return Err(CodegenError::InvalidSymbolName {
            kind,
            name: name.to_string(),
        });
    }

    Ok(())
}

fn is_symbol_start(value: char) -> bool {
    value.is_ascii_alphabetic() || matches!(value, '_' | '$' | '.')
}

fn is_symbol_continue(value: char) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, '_' | '$' | '.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_TAPE_SIZE;

    fn options() -> LlvmOptions {
        LlvmOptions {
            tape_size: DEFAULT_TAPE_SIZE,
            target_triple: Some("x86_64-unknown-linux-gnu".to_string()),
            source_filename: Some("test.bf".to_string()),
            bounds_check: false,
            runtime: Runtime::Hosted,
        }
    }

    #[test]
    fn emits_module_header_and_main() {
        let ir = generate_module(&[], &options()).expect("codegen");
        assert!(ir.contains("target triple = \"x86_64-unknown-linux-gnu\""));
        assert!(ir.contains("@tape = internal global [30000 x i8] zeroinitializer"));
        assert!(ir.contains("define i32 @main()"));
        assert!(ir.contains("ret i32 0"));
    }

    #[test]
    fn input_leaves_cell_unchanged_on_eof() {
        let ir = generate_module(&[Op::Input], &options()).expect("codegen");
        assert!(ir.contains("call i32 @getchar()"));
        assert!(ir.contains("icmp eq i32"));
        assert!(ir.contains("input_store_"));
        assert!(ir.contains("input_cont_"));
    }

    #[test]
    fn output_names_putchar_result() {
        let ir = generate_module(&[Op::Output], &options()).expect("codegen");
        assert!(ir.contains(" = call i32 @putchar(i32 "));
    }

    #[test]
    fn emits_loop_blocks() {
        let ir = generate_module(&[Op::Loop(vec![Op::Add(254)])], &options()).expect("codegen");
        assert!(ir.contains("loop_check_"));
        assert!(ir.contains("loop_body_"));
        assert!(ir.contains("loop_end_"));
    }

    #[test]
    fn emits_scan_blocks() {
        let ir = generate_module(&[Op::Loop(vec![Op::Move(1)])], &options()).expect("codegen");
        assert!(ir.contains("scan_check_"));
        assert!(ir.contains("scan_body_"));
        assert!(ir.contains("scan_end_"));
    }

    #[test]
    fn emits_multiply_transfer_loop_without_runtime_loop() {
        let ir = generate_module(
            &[
                Op::Loop(vec![
                    Op::Add(255),
                    Op::Move(1),
                    Op::Add(3),
                    Op::Move(1),
                    Op::Add(2),
                    Op::Move(-2),
                ]),
                Op::Move(1),
                Op::Output,
            ],
            &options(),
        )
        .expect("codegen");

        assert!(ir.contains("mul i8"));
        assert!(ir.contains("store i8 0"));
        assert!(!ir.contains("loop_check_"));
    }

    #[test]
    fn rejects_empty_tape() {
        let mut options = options();
        options.tape_size = 0;
        assert_eq!(
            generate_module(&[], &options).expect_err("invalid tape"),
            CodegenError::InvalidTapeSize(0)
        );
    }

    #[test]
    fn emits_optional_bounds_checks() {
        let mut options = options();
        options.bounds_check = true;
        let ir = generate_module(&[Op::Add(1)], &options).expect("codegen");

        assert!(ir.contains("declare void @llvm.trap()"));
        assert!(ir.contains("bounds_trap_"));
        assert!(ir.contains("icmp ult i64"));
    }

    #[test]
    fn emits_freestanding_entry_and_io_hooks() {
        let mut options = options();
        options.runtime = Runtime::Freestanding(FreestandingOptions::default());

        let ir = generate_module(&[Op::Input, Op::Output], &options).expect("codegen");

        assert!(ir.contains("define void @bf_main()"));
        assert!(ir.contains("declare void @bf_putchar(i8)"));
        assert!(ir.contains("declare i32 @bf_getchar()"));
        assert!(ir.contains("call void @bf_putchar(i8 "));
        assert!(!ir.contains("@putchar"));
        assert!(!ir.contains("define i32 @main()"));
    }

    #[test]
    fn rejects_invalid_freestanding_symbols() {
        let mut options = options();
        options.runtime = Runtime::Freestanding(FreestandingOptions {
            entry_symbol: "bf-main".to_string(),
            ..FreestandingOptions::default()
        });

        assert_eq!(
            generate_module(&[], &options).expect_err("invalid symbol"),
            CodegenError::InvalidSymbolName {
                kind: "entry",
                name: "bf-main".to_string()
            }
        );
    }

    #[test]
    fn windows_targets_switch_the_streams_to_binary() {
        let ir = generate_module(
            &[Op::Output],
            &LlvmOptions {
                target_triple: Some("x86_64-pc-windows-msvc".to_string()),
                ..options()
            },
        )
        .expect("codegen");

        assert!(ir.contains("declare i32 @_setmode(i32, i32)"));
        assert!(ir.contains("%stdin_mode = call i32 @_setmode(i32 0, i32 32768)"));
        assert!(ir.contains("%stdout_mode = call i32 @_setmode(i32 1, i32 32768)"));
    }

    #[test]
    fn other_targets_leave_the_streams_alone() {
        let ir = generate_module(&[Op::Output], &options()).expect("codegen");
        assert!(!ir.contains("_setmode"));
    }

    #[test]
    fn freestanding_targets_have_no_streams_to_switch() {
        let ir = generate_module(
            &[Op::Output],
            &LlvmOptions {
                target_triple: Some("x86_64-pc-windows-msvc".to_string()),
                runtime: Runtime::Freestanding(FreestandingOptions::default()),
                ..options()
            },
        )
        .expect("codegen");

        assert!(!ir.contains("_setmode"));
    }
}
