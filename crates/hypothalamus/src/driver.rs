//! Compiler-driver configuration and output emission.
//!
//! This module contains the reusable policy that the command-line binary uses
//! after CLI parsing: source loading, Cranelift code generation, output path
//! selection, and the one external tool that is still unavoidable — a linker,
//! and only when the requested output is an executable.

use crate::DEFAULT_TAPE_SIZE;
use crate::bf;
use crate::codegen::{self, CodegenError, CodegenOptions};
use crate::isa::{self, IsaError, IsaOptions};
use crate::jit::{self, JitError};
use crate::target::TargetProfile;
use crate::tool;
use cranelift_module::{ModuleError, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use target_lexicon::Triple;

/// Commands tried, in order, when no linker is configured.
///
/// These are all C compiler drivers rather than bare linkers: a hosted
/// Brainfuck program calls `putchar` and `getchar`, so the link needs the C
/// runtime and its startup files, and a cc driver is what knows where those
/// live on any given system.
pub const LINKER_CANDIDATES: &[&str] = &["cc", "clang", "gcc"];

/// Output kind requested from the compiler driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitKind {
    /// Compile and link a hosted executable.
    Executable,

    /// Compile to a relocatable object file.
    Object,

    /// Compile for the host and run the result in this process.
    Jit,
}

impl EmitKind {
    /// Parse a CLI/API emit kind.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "exe" | "executable" => Some(Self::Executable),
            "obj" | "object" => Some(Self::Object),
            "jit" | "run" => Some(Self::Jit),
            _ => None,
        }
    }

    /// Return the canonical CLI spelling for this emit kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Executable => "exe",
            Self::Object => "obj",
            Self::Jit => "jit",
        }
    }
}

/// Optimization level Cranelift compiles at.
///
/// Cranelift offers three levels rather than the six an LLVM driver does, so
/// the familiar `-O` spellings are accepted and folded onto the nearest one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptLevel {
    /// No optimization; the fastest possible compile.
    None,

    /// Optimize for speed.
    Speed,

    /// Optimize for speed, preferring smaller code when it is a close call.
    SpeedAndSize,
}

impl OptLevel {
    /// Parse a CLI/API optimization level.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "0" | "none" => Some(Self::None),
            "1" | "2" | "3" | "speed" => Some(Self::Speed),
            "s" | "S" | "z" | "Z" | "size" | "speed-and-size" => Some(Self::SpeedAndSize),
            _ => None,
        }
    }

    /// Return the Cranelift `opt_level` setting for this level.
    pub fn cranelift_setting(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Speed => "speed",
            Self::SpeedAndSize => "speed_and_size",
        }
    }
}

/// Complete compiler-driver configuration.
#[derive(Debug, Clone)]
pub struct CompilerConfig {
    /// Brainfuck source file to compile.
    pub input: PathBuf,

    /// Optional output path. When omitted, Hypothalamus chooses one from the
    /// input file, emit kind, and target.
    pub output: Option<PathBuf>,

    /// Requested output kind.
    pub emit: EmitKind,

    /// Target profile controlling the target triple and runtime ABI.
    pub target: TargetProfile,

    /// Number of byte cells allocated in the generated tape.
    pub tape_size: usize,

    /// Emit runtime traps before out-of-range tape access.
    pub bounds_check: bool,

    /// Optimization level Cranelift compiles at.
    pub opt_level: OptLevel,

    /// Linker command used for executables.
    ///
    /// When this is `None`, [`LINKER_CANDIDATES`] are tried in order.
    pub linker: Option<String>,

    /// Keep the generated object file beside the linked executable.
    pub keep_object: bool,
}

impl CompilerConfig {
    /// Build a default hosted-native configuration for `input`.
    pub fn new(input: impl Into<PathBuf>) -> Self {
        Self::for_target(input, TargetProfile::native())
    }

    /// Build a default configuration for `input` and `target`.
    pub fn for_target(input: impl Into<PathBuf>, target: TargetProfile) -> Self {
        let emit = target.default_emit();
        Self {
            input: input.into(),
            output: None,
            emit,
            target,
            tape_size: DEFAULT_TAPE_SIZE,
            bounds_check: false,
            opt_level: OptLevel::Speed,
            linker: None,
            keep_object: false,
        }
    }

    fn codegen_options(&self) -> CodegenOptions {
        CodegenOptions {
            tape_size: self.tape_size,
            bounds_check: self.bounds_check,
            binary_stdio: true,
            runtime: self.target.runtime_abi().to_codegen_runtime(),
        }
    }

    fn isa_options(&self) -> IsaOptions {
        IsaOptions {
            opt_level: self.opt_level,
            // A freestanding runtime owns its own load address and has no
            // dynamic loader to fill in a global offset table.
            position_independent: !self.target.is_freestanding(),
        }
    }
}

/// Error returned by the compiler driver.
#[derive(Debug)]
pub enum DriverError {
    /// The supplied configuration is internally inconsistent.
    InvalidConfig(String),

    /// Source file reading failed.
    ReadSource {
        /// Path that could not be read.
        path: PathBuf,

        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// Brainfuck syntax validation failed.
    Syntax(bf::SyntaxError),

    /// The target ISA could not be configured.
    Isa(IsaError),

    /// Cranelift code generation failed.
    Codegen(CodegenError),

    /// Cranelift rejected the module.
    ///
    /// Boxed because `ModuleError` is far larger than every other variant.
    Module(Box<ModuleError>),

    /// The target triple does not say which object file format to use.
    UnknownBinaryFormat {
        /// The triple that was requested.
        target: String,
    },

    /// The generated module could not be written as an object file.
    EmitObject(String),

    /// Writing an output or temporary file failed.
    WriteFile {
        /// Path that could not be written.
        path: PathBuf,

        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// No linker could be found for an executable link.
    LinkerNotFound,

    /// Launching the linker failed.
    RunTool {
        /// Tool command that failed to launch.
        tool: String,

        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// The linker exited unsuccessfully.
    ToolFailed {
        /// Tool command that exited unsuccessfully.
        tool: String,

        /// Process exit status formatted for diagnostics.
        status: String,

        /// Captured standard output tail from the failed tool.
        stdout: String,

        /// Captured standard error tail from the failed tool.
        stderr: String,
    },

    /// Direct execution failed in the Cranelift JIT.
    Runtime(JitError),
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => write!(f, "{message}"),
            Self::ReadSource { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::Syntax(error) => write!(f, "syntax error: {error}"),
            Self::Isa(error) => write!(f, "{error}"),
            Self::Codegen(error) => write!(f, "code generation failed: {error}"),
            Self::Module(error) => write!(f, "code generation failed: {error}"),
            Self::UnknownBinaryFormat { target } => write!(
                f,
                "target `{target}` does not say which object file format to use. \
                 Name one in the triple, such as `{target}-elf`"
            ),
            Self::EmitObject(message) => write!(f, "failed to write object file: {message}"),
            Self::WriteFile { path, source } => {
                write!(f, "failed to write {}: {source}", path.display())
            }
            Self::LinkerNotFound => write!(
                f,
                "failed to find a linker. Tried {}. Install a C toolchain or pass --linker <path>. \
                 Object output (--emit obj) and direct execution (--run) need no linker",
                LINKER_CANDIDATES.join(", ")
            ),
            Self::RunTool { tool, source } => write!(
                f,
                "failed to run `{tool}`. Install it or pass --linker <path>: {source}"
            ),
            Self::ToolFailed {
                tool,
                status,
                stdout,
                stderr,
            } => {
                write!(f, "{tool} failed with status {status}")?;
                if !stderr.trim().is_empty() {
                    write!(f, "\nstderr:\n{}", stderr.trim_end())?;
                }
                if !stdout.trim().is_empty() {
                    write!(f, "\nstdout:\n{}", stdout.trim_end())?;
                }
                Ok(())
            }
            Self::Runtime(error) => write!(f, "runtime error: {error}"),
        }
    }
}

impl std::error::Error for DriverError {}

impl From<IsaError> for DriverError {
    fn from(error: IsaError) -> Self {
        Self::Isa(error)
    }
}

impl From<CodegenError> for DriverError {
    fn from(error: CodegenError) -> Self {
        Self::Codegen(error)
    }
}

impl From<ModuleError> for DriverError {
    fn from(error: ModuleError) -> Self {
        Self::Module(Box::new(error))
    }
}

impl DriverError {
    pub(crate) fn tool_failed(tool: impl Into<String>, failure: tool::CapturedToolFailure) -> Self {
        Self::ToolFailed {
            tool: tool.into(),
            status: failure.status,
            stdout: failure.stdout,
            stderr: failure.stderr,
        }
    }
}

/// Read and compile the configured input to a relocatable object file.
///
/// This is the whole compiler: no external tool is involved, whatever the
/// target. Only linking the result into an executable needs one.
pub fn compile_to_object(config: &CompilerConfig) -> Result<Vec<u8>, DriverError> {
    let ops = parse_input(config)?;
    let isa = isa::build(config.target.triple(), config.isa_options())?;

    // The object writer needs a container format, and a bare `-none` triple
    // does not imply one. Saying so here beats Cranelift's "binary format is
    // unknown" from three layers down.
    if isa.triple().binary_format == target_lexicon::BinaryFormat::Unknown {
        return Err(DriverError::UnknownBinaryFormat {
            target: isa.triple().to_string(),
        });
    }

    let name = config
        .input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "hypothalamus".to_string());
    let mut module = ObjectModule::new(ObjectBuilder::new(isa, name, default_libcall_names())?);

    codegen::define_program(&mut module, &ops, &config.codegen_options())?;

    module
        .finish()
        .emit()
        .map_err(|error| DriverError::EmitObject(error.to_string()))
}

/// Compile with the configured output mode, linking or running as requested.
pub fn compile(config: &CompilerConfig) -> Result<(), DriverError> {
    validate_config(config)?;

    if config.emit == EmitKind::Jit {
        return run_in_process(config);
    }

    let object = compile_to_object(config)?;
    let output = match config.output.clone() {
        Some(path) => with_executable_extension(path, config.emit, &config.target),
        None => default_output_path(&config.input, config.emit, &config.target),
    };

    match config.emit {
        EmitKind::Object => write_file(&output, &object),
        EmitKind::Executable => link_executable(config, &output, &object),
        EmitKind::Jit => unreachable!("direct execution returns before object emission"),
    }
}

/// Whether executables for `target` are expected to end in `.exe`.
///
/// A target with no explicit triple compiles for the host, so the host decides.
fn wants_exe_extension(target: &TargetProfile) -> bool {
    match target.triple() {
        Some(triple) => triple.contains("windows"),
        None => cfg!(windows),
    }
}

/// Add `.exe` to an explicitly requested executable path that has no extension.
///
/// Windows will not run a file without it, and asking for `-o hello` on Windows
/// plainly means `hello.exe`. A path that already carries an extension is left
/// exactly as written.
fn with_executable_extension(output: PathBuf, emit: EmitKind, target: &TargetProfile) -> PathBuf {
    if emit != EmitKind::Executable || output.extension().is_some() || !wants_exe_extension(target)
    {
        return output;
    }
    let mut output = output;
    output.set_extension("exe");
    output
}

/// Choose the default output path for a compile request.
pub fn default_output_path(input: &Path, emit: EmitKind, target: &TargetProfile) -> PathBuf {
    let mut output = input.to_path_buf();

    match emit {
        EmitKind::Executable => {
            output.set_extension("");
            if wants_exe_extension(target) {
                output.set_extension("exe");
            } else if output == input {
                output.set_extension("out");
            }
        }
        EmitKind::Object | EmitKind::Jit => {
            output.set_extension("o");
        }
    }

    output
}

fn validate_config(config: &CompilerConfig) -> Result<(), DriverError> {
    if config.emit == EmitKind::Jit && config.output.is_some() {
        return Err(DriverError::InvalidConfig(
            "--emit jit runs the program directly and does not accept --output".to_string(),
        ));
    }

    if config.output.as_deref() == Some(Path::new("-")) {
        return Err(DriverError::InvalidConfig(
            "--output - is not supported; every output kind is binary".to_string(),
        ));
    }

    // The runtime ABI is the more fundamental reason of the two, so it is
    // reported first: a freestanding target has no I/O for the JIT to supply,
    // whatever architecture it names.
    if config.target.is_freestanding()
        && matches!(config.emit, EmitKind::Executable | EmitKind::Jit)
    {
        return Err(DriverError::InvalidConfig(format!(
            "target `{}` uses a freestanding runtime and supports --emit obj",
            config.target.name()
        )));
    }

    if config.emit == EmitKind::Jit && config.target.triple().is_some() {
        return Err(DriverError::InvalidConfig(format!(
            "--emit jit runs the program in this process and cannot target `{}`",
            config.target.name()
        )));
    }

    // Cranelift will happily emit the object, but the link afterwards runs the
    // host's own linker against the host's C runtime. Saying so beats letting
    // the linker report it as an unknown file type.
    if config.emit == EmitKind::Executable
        && let Some(triple) = config.target.triple()
        && !links_on_this_host(triple)
    {
        return Err(DriverError::InvalidConfig(format!(
            "target `{}` cannot be linked on this host ({}). \
             Use --emit obj and link the object with a toolchain for that target",
            config.target.name(),
            Triple::host()
        )));
    }

    Ok(())
}

/// Whether an executable for `triple` could be linked by the host's toolchain.
///
/// Only the architecture and operating system matter: those decide the object
/// format, the C runtime, and the startup files. A triple that does not parse
/// is left alone, so that [`isa::build`] can report it properly.
fn links_on_this_host(triple: &str) -> bool {
    let Ok(triple) = Triple::from_str(triple) else {
        return true;
    };
    let host = Triple::host();

    triple.architecture == host.architecture && triple.operating_system == host.operating_system
}

fn parse_input(config: &CompilerConfig) -> Result<Vec<bf::Op>, DriverError> {
    let source = fs::read(&config.input).map_err(|source| DriverError::ReadSource {
        path: config.input.clone(),
        source,
    })?;

    bf::parse(&source).map_err(DriverError::Syntax)
}

fn run_in_process(config: &CompilerConfig) -> Result<(), DriverError> {
    let ops = parse_input(config)?;
    let options = CodegenOptions {
        // The JIT's own I/O hooks move raw bytes, so there are no text-mode
        // streams to correct.
        binary_stdio: false,
        ..config.codegen_options()
    };

    jit::run(&ops, &options, config.isa_options())
        .map(|_status| ())
        .map_err(DriverError::Runtime)
}

fn write_file(output: &Path, bytes: &[u8]) -> Result<(), DriverError> {
    fs::write(output, bytes).map_err(|source| DriverError::WriteFile {
        path: output.to_path_buf(),
        source,
    })
}

fn link_executable(
    config: &CompilerConfig,
    output: &Path,
    object: &[u8],
) -> Result<(), DriverError> {
    let linker = find_linker(config.linker.as_deref()).ok_or(DriverError::LinkerNotFound)?;

    let object_path = if config.keep_object {
        output.with_extension("o")
    } else {
        temporary_object_path()
    };
    write_file(&object_path, object)?;

    let mut command = Command::new(&linker);
    command.arg(&object_path);
    command.arg("-o");
    command.arg(output);

    let result = match tool::run_captured(command) {
        Ok(None) => Ok(()),
        Ok(Some(failure)) => Err(DriverError::tool_failed(
            linker.display().to_string(),
            failure,
        )),
        Err(source) => Err(DriverError::RunTool {
            tool: linker.display().to_string(),
            source,
        }),
    };

    if !config.keep_object {
        let _ = fs::remove_file(&object_path);
    }

    result
}

/// Find the linker to use, honouring an explicit command over discovery.
pub fn find_linker(configured: Option<&str>) -> Option<PathBuf> {
    match configured {
        Some(command) => tool::resolve_command_path(command),
        None => LINKER_CANDIDATES
            .iter()
            .find_map(|candidate| tool::find_on_path(candidate)),
    }
}

fn temporary_object_path() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    env::temp_dir().join(format!("hypothalamus-{}-{timestamp}.o", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::FreestandingOptions;
    use crate::target::{RuntimeAbi, TargetProfile};

    #[test]
    fn parses_emit_kinds() {
        assert_eq!(EmitKind::parse("jit"), Some(EmitKind::Jit));
        assert_eq!(EmitKind::parse("run"), Some(EmitKind::Jit));
        assert_eq!(EmitKind::parse("exe"), Some(EmitKind::Executable));
        assert_eq!(EmitKind::parse("obj"), Some(EmitKind::Object));
        assert_eq!(EmitKind::parse("llvm-ir"), None);
        assert_eq!(EmitKind::parse("bad"), None);
    }

    #[test]
    fn folds_familiar_optimization_levels_onto_cranelift_levels() {
        assert_eq!(OptLevel::parse("0"), Some(OptLevel::None));
        assert_eq!(OptLevel::parse("2"), Some(OptLevel::Speed));
        assert_eq!(OptLevel::parse("3"), Some(OptLevel::Speed));
        assert_eq!(OptLevel::parse("z"), Some(OptLevel::SpeedAndSize));
        assert_eq!(OptLevel::parse("fast"), None);

        assert_eq!(OptLevel::None.cranelift_setting(), "none");
        assert_eq!(OptLevel::SpeedAndSize.cranelift_setting(), "speed_and_size");
    }

    #[test]
    fn rejects_jit_output_path() {
        let mut config = CompilerConfig::new("examples/hello.bf");
        config.emit = EmitKind::Jit;
        config.output = Some(PathBuf::from("hello"));

        assert!(matches!(
            compile(&config),
            Err(DriverError::InvalidConfig(message)) if message.contains("--emit jit")
        ));
    }

    #[test]
    fn rejects_cross_target_jit() {
        let mut config = CompilerConfig::for_target(
            "examples/hello.bf",
            TargetProfile::resolve("aarch64-apple-darwin"),
        );
        config.emit = EmitKind::Jit;

        assert!(matches!(
            compile(&config),
            Err(DriverError::InvalidConfig(message)) if message.contains("in this process")
        ));
    }

    #[test]
    fn rejects_hosted_output_for_freestanding_targets() {
        for emit in [EmitKind::Executable, EmitKind::Jit] {
            let mut config = CompilerConfig::for_target(
                "examples/hello.bf",
                TargetProfile::resolve("x86_64-none"),
            );
            config.emit = emit;

            assert!(matches!(
                validate_config(&config),
                Err(DriverError::InvalidConfig(message)) if message.contains("freestanding runtime")
            ));
        }
    }

    #[test]
    fn rejects_stdout_output() {
        let mut config = CompilerConfig::new("examples/hello.bf");
        config.emit = EmitKind::Object;
        config.output = Some(PathBuf::from("-"));

        assert!(matches!(
            validate_config(&config),
            Err(DriverError::InvalidConfig(message)) if message.contains("--output -")
        ));
    }

    #[test]
    fn compiles_a_freestanding_object_without_any_tool() {
        let target = TargetProfile::resolve("x86_64-none");
        let mut config = CompilerConfig::for_target("examples/hello.bf", target);
        config.emit = EmitKind::Object;

        let object = compile_to_object(&config).expect("freestanding object should compile");

        assert!(!object.is_empty());
    }

    #[test]
    fn honours_custom_freestanding_symbols() {
        let target = TargetProfile::resolve("x86_64-none").with_runtime_abi(
            RuntimeAbi::Freestanding(FreestandingOptions {
                entry_symbol: "kernel bf".to_string(),
                ..FreestandingOptions::default()
            }),
        );
        let config = CompilerConfig::for_target("examples/hello.bf", target);

        assert!(matches!(
            compile_to_object(&config),
            Err(DriverError::Codegen(CodegenError::InvalidSymbolName { .. }))
        ));
    }

    #[test]
    fn object_output_defaults_to_a_dot_o_path() {
        let target = TargetProfile::resolve("x86_64-none");
        let output = default_output_path(Path::new("hello.bf"), EmitKind::Object, &target);

        assert_eq!(output, PathBuf::from("hello.o"));
    }

    #[test]
    fn windows_executables_end_in_exe() {
        let target = TargetProfile::resolve("x86_64-pc-windows-msvc");
        let output = default_output_path(Path::new("hello.bf"), EmitKind::Executable, &target);

        assert_eq!(output, PathBuf::from("hello.exe"));
    }

    #[test]
    fn native_executables_follow_the_host() {
        // The native target has no triple of its own, so the host decides
        // whether `.exe` is needed.
        let target = TargetProfile::native();
        let output = default_output_path(Path::new("hello.bf"), EmitKind::Executable, &target);

        if cfg!(windows) {
            assert_eq!(output, PathBuf::from("hello.exe"));
        } else {
            assert_eq!(output, PathBuf::from("hello"));
        }
    }

    #[test]
    fn requested_executable_paths_gain_the_host_extension() {
        let target = TargetProfile::native();
        let output =
            with_executable_extension(PathBuf::from("build/hello"), EmitKind::Executable, &target);

        if cfg!(windows) {
            assert_eq!(output, PathBuf::from("build/hello.exe"));
        } else {
            assert_eq!(output, PathBuf::from("build/hello"));
        }
    }

    #[test]
    fn requested_paths_keep_an_extension_the_user_chose() {
        let target = TargetProfile::resolve("x86_64-pc-windows-msvc");

        // An explicit extension is honoured, whatever it is.
        assert_eq!(
            with_executable_extension(PathBuf::from("hello.bin"), EmitKind::Executable, &target),
            PathBuf::from("hello.bin")
        );
        // Only executables get the treatment.
        assert_eq!(
            with_executable_extension(PathBuf::from("hello"), EmitKind::Object, &target),
            PathBuf::from("hello")
        );
    }

    #[test]
    fn an_explicit_linker_is_used_verbatim_when_it_exists() {
        assert_eq!(find_linker(Some("hypothalamus-absent-linker")), None);
    }
}
