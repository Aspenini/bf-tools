//! Compiler driver types and tool invocation helpers.
//!
//! This module contains the reusable policy that the command-line binary uses
//! after CLI parsing: source loading, LLVM generation, output path selection,
//! and optional invocation of external LLVM tools.

use crate::DEFAULT_TAPE_SIZE;
use crate::bf;
use crate::llvm::{self, LlvmOptions};
use crate::runner;
use crate::target::{TargetImageFormat, TargetProfile};
use crate::targets;
use crate::tool;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Output kind requested from the compiler driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitKind {
    /// Compile and link a hosted executable.
    Executable,

    /// Compile to a relocatable object file.
    Object,

    /// Compile to target assembly.
    Assembly,

    /// Emit textual LLVM IR.
    LlvmIr,

    /// Execute the program directly with Hypothalamus' built-in runner.
    Jit,

    /// Execute generated LLVM IR through `lli`.
    LlvmJit,

    /// Build a complete target image, such as a ROM.
    Image,
}

impl EmitKind {
    /// Parse a CLI/API emit kind.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "exe" | "executable" => Some(Self::Executable),
            "obj" | "object" => Some(Self::Object),
            "asm" | "assembly" => Some(Self::Assembly),
            "llvm-ir" | "ll" => Some(Self::LlvmIr),
            "jit" | "run" => Some(Self::Jit),
            "llvm-jit" | "lli" => Some(Self::LlvmJit),
            "image" | "rom" => Some(Self::Image),
            _ => None,
        }
    }

    /// Return the canonical CLI spelling for this emit kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Executable => "exe",
            Self::Object => "obj",
            Self::Assembly => "asm",
            Self::LlvmIr => "llvm-ir",
            Self::Jit => "jit",
            Self::LlvmJit => "llvm-jit",
            Self::Image => "image",
        }
    }
}

/// LLVM optimization level passed to the external LLVM driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptLevel {
    /// `-O0`
    Zero,

    /// `-O1`
    One,

    /// `-O2`
    Two,

    /// `-O3`
    Three,

    /// `-Os`
    Size,

    /// `-Oz`
    SizeMin,
}

impl OptLevel {
    /// Parse a CLI/API optimization level.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "0" => Some(Self::Zero),
            "1" => Some(Self::One),
            "2" => Some(Self::Two),
            "3" => Some(Self::Three),
            "s" | "S" | "size" => Some(Self::Size),
            "z" | "Z" | "size-min" => Some(Self::SizeMin),
            _ => None,
        }
    }

    /// Return the `clang` optimization flag for this level.
    pub fn clang_arg(self) -> &'static str {
        match self {
            Self::Zero => "-O0",
            Self::One => "-O1",
            Self::Two => "-O2",
            Self::Three => "-O3",
            Self::Size => "-Os",
            Self::SizeMin => "-Oz",
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

    /// Target profile controlling LLVM target, runtime ABI, and target flags.
    pub target: TargetProfile,

    /// Number of byte cells allocated in the generated tape.
    pub tape_size: usize,

    /// Emit runtime traps before out-of-range tape access.
    pub bounds_check: bool,

    /// LLVM optimization level passed to `clang`.
    pub opt_level: OptLevel,

    /// `clang`-compatible LLVM driver command.
    pub clang: String,

    /// LLVM `lli` command for explicit LLVM JIT execution.
    pub lli: String,

    /// Keep generated LLVM IR beside the final output.
    pub keep_ll: bool,

    /// Optional `arm-none-eabi-gcc` command for GBA ROM images.
    pub gba_gcc: Option<PathBuf>,

    /// Optional legacy `arm-none-eabi-objcopy` command for GBA ROM images.
    ///
    /// Current ROM builds extract linked ELF segments internally, so this is
    /// accepted for compatibility but is not required.
    pub gba_objcopy: Option<PathBuf>,
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
            opt_level: OptLevel::Two,
            clang: "clang".to_string(),
            lli: "lli".to_string(),
            keep_ll: false,
            gba_gcc: None,
            gba_objcopy: None,
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

    /// LLVM IR generation failed.
    Codegen(llvm::CodegenError),

    /// Writing an output or temporary file failed.
    WriteFile {
        /// Path that could not be written.
        path: PathBuf,

        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// Launching an external tool failed.
    RunTool {
        /// Tool command that failed to launch.
        tool: String,

        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// An external tool exited unsuccessfully.
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

    /// A target-specific tool could not be found.
    ToolNotFound {
        /// Tool command that could not be found.
        tool: &'static str,

        /// Human-readable install or override hint.
        hint: &'static str,
    },

    /// A target image builder could not read or construct its image format.
    InvalidImage {
        /// Human-readable image format name.
        format: &'static str,

        /// Specific validation or construction failure.
        message: String,
    },

    /// Direct execution failed in Hypothalamus' built-in runner.
    Runtime(String),
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => write!(f, "{message}"),
            Self::ReadSource { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::Syntax(error) => write!(f, "syntax error: {error}"),
            Self::Codegen(error) => write!(f, "LLVM code generation failed: {error}"),
            Self::WriteFile { path, source } => {
                write!(f, "failed to write {}: {source}", path.display())
            }
            Self::RunTool { tool, source } if tool == "clang" => write!(
                f,
                "failed to run `clang`. Install clang or pass --cc <path>: {source}"
            ),
            Self::RunTool { tool, source } if tool == "lli" => write!(
                f,
                "failed to run `lli`. Install lli or pass --lli <path>: {source}"
            ),
            Self::RunTool { tool, source } => write!(f, "failed to run `{tool}`: {source}"),
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
            Self::ToolNotFound { tool, hint } => {
                write!(f, "failed to find `{tool}`. {hint}")
            }
            Self::InvalidImage { format, message } => {
                write!(f, "failed to build {format} image: {message}")
            }
            Self::Runtime(message) => write!(f, "runtime error: {message}"),
        }
    }
}

impl std::error::Error for DriverError {}

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

/// Read and compile the configured input to textual LLVM IR.
pub fn compile_to_llvm(config: &CompilerConfig) -> Result<String, DriverError> {
    let source = fs::read(&config.input).map_err(|source| DriverError::ReadSource {
        path: config.input.clone(),
        source,
    })?;
    let ops = bf::parse(&source).map_err(DriverError::Syntax)?;

    llvm::generate_module(
        &ops,
        &LlvmOptions {
            tape_size: config.tape_size,
            target_triple: config.target.llvm_triple().map(ToString::to_string),
            source_filename: Some(config.input.display().to_string()),
            bounds_check: config.bounds_check,
            runtime: config.target.runtime_abi().to_llvm_runtime(),
        },
    )
    .map_err(DriverError::Codegen)
}

/// Compile with the configured output mode and external tools.
pub fn compile_with_tools(config: &CompilerConfig) -> Result<(), DriverError> {
    validate_tool_config(config)?;

    if config.emit == EmitKind::Jit {
        return run_with_builtin_runner(config);
    }

    let module = compile_to_llvm(config)?;
    let output = match config.output.clone() {
        Some(path) => with_executable_extension(path, config.emit, &config.target),
        None => default_output_path(&config.input, config.emit, &config.target),
    };

    match config.emit {
        EmitKind::LlvmIr => write_llvm_ir(&output, &module),
        EmitKind::LlvmJit => run_with_lli(config, &module),
        EmitKind::Image => match config.target.image_format() {
            Some(TargetImageFormat::Gba) => targets::gba::build_image(config, &module, &output),
            None => unreachable!("image target validation should reject missing image builders"),
        },
        _ => compile_with_clang(config, &output, &module),
    }
}

/// Whether executables for `target` are expected to end in `.exe`.
///
/// A target with no explicit triple compiles for whatever host clang runs on,
/// so the host decides.
fn wants_exe_extension(target: &TargetProfile) -> bool {
    match target.llvm_triple() {
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
        EmitKind::Object => {
            output.set_extension("o");
        }
        EmitKind::Assembly => {
            output.set_extension("s");
        }
        EmitKind::LlvmIr | EmitKind::Jit | EmitKind::LlvmJit => {
            output.set_extension("ll");
        }
        EmitKind::Image => match target.image_format() {
            Some(TargetImageFormat::Gba) => {
                output.set_extension("gba");
            }
            None => {
                output.set_extension("img");
            }
        },
    }

    output
}

fn validate_tool_config(config: &CompilerConfig) -> Result<(), DriverError> {
    if matches!(config.emit, EmitKind::Jit | EmitKind::LlvmJit) && config.output.is_some() {
        return Err(DriverError::InvalidConfig(
            "--emit jit runs the program directly and does not accept --output".to_string(),
        ));
    }

    if config.output.as_deref() == Some(Path::new("-")) && config.emit != EmitKind::LlvmIr {
        return Err(DriverError::InvalidConfig(
            "--output - writes to stdout and is only supported with --emit llvm-ir".to_string(),
        ));
    }

    if config.emit == EmitKind::Image && config.target.image_format().is_none() {
        return Err(DriverError::InvalidConfig(format!(
            "target `{}` does not have a complete-image builder",
            config.target.name()
        )));
    }

    if config.target.is_freestanding()
        && matches!(
            config.emit,
            EmitKind::Executable | EmitKind::Jit | EmitKind::LlvmJit
        )
    {
        return Err(DriverError::InvalidConfig(format!(
            "target `{}` uses a freestanding runtime and supports --emit obj, --emit asm, --emit llvm-ir, or target images",
            config.target.name()
        )));
    }

    Ok(())
}

fn run_with_builtin_runner(config: &CompilerConfig) -> Result<(), DriverError> {
    let source = fs::read(&config.input).map_err(|source| DriverError::ReadSource {
        path: config.input.clone(),
        source,
    })?;
    let ops = bf::parse(&source).map_err(DriverError::Syntax)?;

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();

    runner::run_ops(&ops, config.tape_size, &mut input, &mut output)
        .map_err(|error| DriverError::Runtime(error.to_string()))
}

fn write_llvm_ir(output: &Path, module: &str) -> Result<(), DriverError> {
    if output == Path::new("-") {
        print!("{module}");
        return Ok(());
    }

    fs::write(output, module).map_err(|source| DriverError::WriteFile {
        path: output.to_path_buf(),
        source,
    })
}

fn compile_with_clang(
    config: &CompilerConfig,
    output: &Path,
    module: &str,
) -> Result<(), DriverError> {
    let ll_path = if config.keep_ll {
        output.with_extension("ll")
    } else {
        temporary_llvm_path()
    };

    fs::write(&ll_path, module).map_err(|source| DriverError::WriteFile {
        path: ll_path.clone(),
        source,
    })?;

    let result = invoke_clang(config, output, &ll_path);

    if !config.keep_ll {
        let _ = fs::remove_file(&ll_path);
    }

    result
}

fn run_with_lli(config: &CompilerConfig, module: &str) -> Result<(), DriverError> {
    let ll_path = if config.keep_ll {
        default_output_path(&config.input, EmitKind::LlvmIr, &config.target)
    } else {
        temporary_llvm_path()
    };

    fs::write(&ll_path, module).map_err(|source| DriverError::WriteFile {
        path: ll_path.clone(),
        source,
    })?;

    let result = invoke_lli(config, &ll_path);

    if !config.keep_ll {
        let _ = fs::remove_file(&ll_path);
    }

    result
}

fn invoke_clang(config: &CompilerConfig, output: &Path, ll_path: &Path) -> Result<(), DriverError> {
    let mut command = Command::new(&config.clang);
    command.arg("-Wno-override-module");
    command.arg(config.opt_level.clang_arg());

    if config.target.is_freestanding() {
        command.arg("-ffreestanding");
        command.arg("-fno-builtin");
    }

    if let Some(target_triple) = config.target.llvm_triple() {
        command.arg(format!("--target={target_triple}"));
    }

    command.args(config.target.clang_args());

    match config.emit {
        EmitKind::Executable => {}
        EmitKind::Object => {
            command.arg("-c");
        }
        EmitKind::Assembly => {
            command.arg("-S");
        }
        EmitKind::Image => unreachable!("target image emission does not invoke generic clang"),
        EmitKind::LlvmIr => unreachable!("LLVM IR emission does not invoke clang"),
        EmitKind::Jit => unreachable!("built-in execution does not invoke clang"),
        EmitKind::LlvmJit => unreachable!("LLVM JIT execution does not invoke clang"),
    }

    command.arg(ll_path);
    command.arg("-o");
    command.arg(output);

    if let Some(failure) = tool::run_captured(command).map_err(|source| DriverError::RunTool {
        tool: config.clang.clone(),
        source,
    })? {
        return Err(DriverError::tool_failed(config.clang.clone(), failure));
    }

    Ok(())
}

fn invoke_lli(config: &CompilerConfig, ll_path: &Path) -> Result<(), DriverError> {
    let status = Command::new(&config.lli)
        .arg(ll_path)
        .status()
        .map_err(|source| DriverError::RunTool {
            tool: config.lli.clone(),
            source,
        })?;

    if !status.success() {
        return Err(DriverError::ToolFailed {
            tool: config.lli.clone(),
            status: status.to_string(),
            stdout: String::new(),
            stderr: String::new(),
        });
    }

    Ok(())
}

fn temporary_llvm_path() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    env::temp_dir().join(format!(
        "hypothalamus-{}-{timestamp}.ll",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_emit_kinds() {
        assert_eq!(EmitKind::parse("jit"), Some(EmitKind::Jit));
        assert_eq!(EmitKind::parse("run"), Some(EmitKind::Jit));
        assert_eq!(EmitKind::parse("llvm-jit"), Some(EmitKind::LlvmJit));
        assert_eq!(EmitKind::parse("lli"), Some(EmitKind::LlvmJit));
        assert_eq!(EmitKind::parse("ll"), Some(EmitKind::LlvmIr));
        assert_eq!(EmitKind::parse("rom"), Some(EmitKind::Image));
        assert_eq!(EmitKind::parse("image"), Some(EmitKind::Image));
        assert_eq!(EmitKind::parse("bad"), None);
    }

    #[test]
    fn parses_optimization_levels() {
        assert_eq!(OptLevel::parse("0"), Some(OptLevel::Zero));
        assert_eq!(OptLevel::parse("3"), Some(OptLevel::Three));
        assert_eq!(OptLevel::parse("s"), Some(OptLevel::Size));
        assert_eq!(OptLevel::parse("z"), Some(OptLevel::SizeMin));
        assert_eq!(OptLevel::parse("fast"), None);
    }

    #[test]
    fn rejects_jit_output_path() {
        let mut config = CompilerConfig::new("examples/hello.bf");
        config.emit = EmitKind::Jit;
        config.output = Some(PathBuf::from("hello"));

        assert!(matches!(
            compile_with_tools(&config),
            Err(DriverError::InvalidConfig(message)) if message.contains("--emit jit")
        ));
    }

    #[test]
    fn rejects_image_for_targets_without_image_builders() {
        let mut config = CompilerConfig::new("examples/hello.bf");
        config.emit = EmitKind::Image;

        assert!(matches!(
            compile_with_tools(&config),
            Err(DriverError::InvalidConfig(message)) if message.contains("complete-image builder")
        ));
    }

    #[test]
    fn rejects_image_output_to_stdout() {
        let mut config =
            CompilerConfig::for_target("examples/hello.bf", TargetProfile::resolve("gba"));
        config.output = Some(PathBuf::from("-"));

        assert!(matches!(
            compile_with_tools(&config),
            Err(DriverError::InvalidConfig(message)) if message.contains("--emit llvm-ir")
        ));
    }

    #[test]
    fn rejects_stdout_output_for_non_llvm_ir_modes() {
        for emit in [
            EmitKind::Executable,
            EmitKind::Object,
            EmitKind::Assembly,
            EmitKind::Image,
        ] {
            let target = if emit == EmitKind::Image {
                TargetProfile::resolve("gba")
            } else {
                TargetProfile::native()
            };
            let mut config = CompilerConfig::for_target("examples/hello.bf", target);
            config.emit = emit;
            config.output = Some(PathBuf::from("-"));

            assert!(matches!(
                validate_tool_config(&config),
                Err(DriverError::InvalidConfig(message)) if message.contains("--emit llvm-ir")
            ));
        }
    }

    #[test]
    fn gba_images_default_to_gba_extension() {
        let target = TargetProfile::resolve("gba");
        let output = default_output_path(Path::new("hello.bf"), EmitKind::Image, &target);

        assert_eq!(output, PathBuf::from("hello.gba"));
    }

    #[test]
    fn windows_executables_end_in_exe() {
        let target = TargetProfile::resolve("x86_64-pc-windows-msvc");
        let output = default_output_path(Path::new("hello.bf"), EmitKind::Executable, &target);

        assert_eq!(output, PathBuf::from("hello.exe"));
    }

    #[test]
    fn native_executables_follow_the_host() {
        // The native target has no triple of its own, so clang builds for the
        // host and the host decides whether `.exe` is needed.
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
}
