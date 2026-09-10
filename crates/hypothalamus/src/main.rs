use hypothalamus::DEFAULT_TAPE_SIZE;
use hypothalamus::codegen::FreestandingOptions;
use hypothalamus::diagnostics::{ToolDoctorConfig, tools_doctor_report};
use hypothalamus::driver::{CompilerConfig, DriverError, EmitKind, OptLevel, compile};
use hypothalamus::target::{RuntimeAbi, TargetProfile, known_targets};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug)]
enum Action {
    Run(Box<CompilerConfig>),
    Help,
    Version,
    ListTargets,
    ToolsDoctor(ToolDoctorConfig),
    ToolsHelp,
}

/// Options that went away with the LLVM backend, and what replaced them.
const REMOVED_OPTIONS: &[(&str, &str)] = &[
    (
        "--cc",
        "use --linker <PATH>; Cranelift only needs a tool to link",
    ),
    (
        "--lli",
        "removed with the LLVM backend; --run uses the built-in JIT",
    ),
    (
        "--keep-ll",
        "use --keep-object to keep the generated object file",
    ),
    (
        "--gba-gcc",
        "the gba target was removed; Cranelift has no 32-bit ARM backend",
    ),
    (
        "--gba-objcopy",
        "the gba target was removed; Cranelift has no 32-bit ARM backend",
    ),
];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::Usage(message)) => {
            eprintln!("hypothalamus: {message}");
            eprintln!("run `hypothalamus --help` for usage");
            ExitCode::from(2)
        }
        Err(Error::Failure(message)) => {
            eprintln!("hypothalamus: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Error> {
    match parse_args(env::args().skip(1))? {
        Action::Help => {
            print_help();
            Ok(())
        }
        Action::Version => {
            println!("hypothalamus {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Action::ListTargets => {
            print_targets();
            Ok(())
        }
        Action::ToolsDoctor(config) => {
            print!("{}", tools_doctor_report(&config));
            Ok(())
        }
        Action::ToolsHelp => {
            print_tools_help();
            Ok(())
        }
        Action::Run(config) => compile(&config).map_err(Error::from),
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Action, Error> {
    let args = args.into_iter().collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("tools") {
        return parse_tools_args(args.into_iter().skip(1));
    }

    let mut input = None;
    let mut output = None;
    let mut emit = None;
    let mut target = None;
    let mut tape_size = DEFAULT_TAPE_SIZE;
    let mut bounds_check = false;
    let mut freestanding = false;
    let mut freestanding_symbols_configured = false;
    let mut entry_symbol = "bf_main".to_string();
    let mut putchar_symbol = "bf_putchar".to_string();
    let mut getchar_symbol = "bf_getchar".to_string();
    let mut opt_level = OptLevel::Speed;
    let mut linker = None;
    let mut keep_object = false;

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Action::Help),
            "--version" => return Ok(Action::Version),
            "--list-targets" => return Ok(Action::ListTargets),
            "-o" | "--output" => {
                output = Some(PathBuf::from(next_value(&mut args, &arg)?));
            }
            "--emit" => {
                emit = Some(parse_emit_value(&next_value(&mut args, &arg)?)?);
            }
            "--jit" | "--run" => {
                emit = Some(EmitKind::Jit);
            }
            "--target" => {
                target = Some(next_value(&mut args, &arg)?);
            }
            "--tape-size" => {
                let value = next_value(&mut args, &arg)?;
                tape_size = parse_tape_size(&value)?;
            }
            "--bounds-check" => {
                bounds_check = true;
            }
            "--freestanding" => {
                freestanding = true;
            }
            "--entry" => {
                entry_symbol = next_value(&mut args, &arg)?;
                freestanding_symbols_configured = true;
            }
            "--putchar-symbol" => {
                putchar_symbol = next_value(&mut args, &arg)?;
                freestanding_symbols_configured = true;
            }
            "--getchar-symbol" => {
                getchar_symbol = next_value(&mut args, &arg)?;
                freestanding_symbols_configured = true;
            }
            "--opt-level" => {
                opt_level = parse_opt_level_value(&next_value(&mut args, &arg)?)?;
            }
            "--linker" => {
                linker = Some(next_value(&mut args, &arg)?);
            }
            "--keep-object" => {
                keep_object = true;
            }
            "--" => {
                for value in args {
                    set_input(&mut input, value)?;
                }
                break;
            }
            _ if arg.starts_with("--output=") => {
                output = Some(PathBuf::from(value_after_equals(&arg)));
            }
            _ if arg.starts_with("--emit=") => {
                emit = Some(parse_emit_value(value_after_equals(&arg))?);
            }
            _ if arg.starts_with("--entry=") => {
                entry_symbol = value_after_equals(&arg).to_string();
                freestanding_symbols_configured = true;
            }
            _ if arg.starts_with("--putchar-symbol=") => {
                putchar_symbol = value_after_equals(&arg).to_string();
                freestanding_symbols_configured = true;
            }
            _ if arg.starts_with("--getchar-symbol=") => {
                getchar_symbol = value_after_equals(&arg).to_string();
                freestanding_symbols_configured = true;
            }
            _ if arg.starts_with("--opt-level=") => {
                opt_level = parse_opt_level_value(value_after_equals(&arg))?;
            }
            _ if arg.starts_with("--target=") => {
                target = Some(value_after_equals(&arg).to_string());
            }
            _ if arg.starts_with("--tape-size=") => {
                tape_size = parse_tape_size(value_after_equals(&arg))?;
            }
            _ if arg.starts_with("--linker=") => {
                linker = Some(value_after_equals(&arg).to_string());
            }
            _ if arg.starts_with("--bounds-check=") => {
                return Err(Error::Usage(
                    "--bounds-check does not take a value".to_string(),
                ));
            }
            _ if arg.starts_with("--freestanding=") => {
                return Err(Error::Usage(
                    "--freestanding does not take a value".to_string(),
                ));
            }
            _ if arg.starts_with("--keep-object=") => {
                return Err(Error::Usage(
                    "--keep-object does not take a value".to_string(),
                ));
            }
            _ if arg.starts_with("--list-targets=") => {
                return Err(Error::Usage(
                    "--list-targets does not take a value".to_string(),
                ));
            }
            "-O0" => {
                opt_level = OptLevel::None;
            }
            "-O1" | "-O2" | "-O3" => {
                opt_level = OptLevel::Speed;
            }
            "-Os" | "-Oz" => {
                opt_level = OptLevel::SpeedAndSize;
            }
            _ if arg.starts_with('-') && arg != "-" => {
                return Err(removed_option(&arg)
                    .unwrap_or_else(|| Error::Usage(format!("unknown option `{arg}`"))));
            }
            _ => set_input(&mut input, arg)?,
        }
    }

    let input = input.ok_or_else(|| Error::Usage("missing input file".to_string()))?;
    let mut target = target
        .as_deref()
        .map(TargetProfile::resolve)
        .unwrap_or_else(TargetProfile::native);

    if freestanding || (target.is_freestanding() && freestanding_symbols_configured) {
        target.set_runtime_abi(RuntimeAbi::Freestanding(FreestandingOptions {
            entry_symbol,
            putchar_symbol,
            getchar_symbol,
        }));
    } else if freestanding_symbols_configured {
        return Err(Error::Usage(
            "--entry, --putchar-symbol, and --getchar-symbol require a freestanding target or --freestanding".to_string(),
        ));
    }

    let emit = emit.unwrap_or_else(|| target.default_emit());
    if target.is_freestanding() && matches!(emit, EmitKind::Executable | EmitKind::Jit) {
        return Err(Error::Usage(format!(
            "target `{}` uses a freestanding runtime and supports --emit obj",
            target.name()
        )));
    }

    Ok(Action::Run(Box::new(CompilerConfig {
        input,
        output,
        emit,
        target,
        tape_size,
        bounds_check,
        opt_level,
        linker,
        keep_object,
    })))
}

fn parse_tools_args(args: impl IntoIterator<Item = String>) -> Result<Action, Error> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Err(Error::Usage(
            "missing tools command; expected `doctor`".to_string(),
        ));
    };

    match command.as_str() {
        "-h" | "--help" => return Ok(Action::ToolsHelp),
        "doctor" => {}
        _ => {
            return Err(Error::Usage(format!(
                "unknown tools command `{command}`; expected `doctor`"
            )));
        }
    }

    let mut config = ToolDoctorConfig::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Action::ToolsHelp),
            "--linker" => {
                config.linker = Some(next_value(&mut args, &arg)?);
            }
            _ if arg.starts_with("--linker=") => {
                config.linker = Some(value_after_equals(&arg).to_string());
            }
            _ if arg.starts_with('-') => {
                return Err(removed_option(&arg).unwrap_or_else(|| {
                    Error::Usage(format!("unknown tools doctor option `{arg}`"))
                }));
            }
            _ => {
                return Err(Error::Usage(format!(
                    "unexpected tools doctor argument `{arg}`"
                )));
            }
        }
    }

    Ok(Action::ToolsDoctor(config))
}

/// Explain an option the LLVM backend used to accept, rather than just
/// reporting it as unknown.
fn removed_option(arg: &str) -> Option<Error> {
    let name = arg.split_once('=').map(|(name, _)| name).unwrap_or(arg);

    REMOVED_OPTIONS
        .iter()
        .find(|(removed, _)| *removed == name)
        .map(|(removed, hint)| Error::Usage(format!("`{removed}` was removed: {hint}")))
}

fn next_value(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, Error> {
    args.next()
        .ok_or_else(|| Error::Usage(format!("missing value for {option}")))
}

fn set_input(input: &mut Option<PathBuf>, value: String) -> Result<(), Error> {
    if input.is_some() {
        return Err(Error::Usage("multiple input files provided".to_string()));
    }

    *input = Some(PathBuf::from(value));
    Ok(())
}

fn parse_emit_value(value: &str) -> Result<EmitKind, Error> {
    EmitKind::parse(value).ok_or_else(|| {
        Error::Usage(format!(
            "invalid --emit value `{value}`; expected exe, obj, or jit"
        ))
    })
}

fn parse_opt_level_value(value: &str) -> Result<OptLevel, Error> {
    OptLevel::parse(value).ok_or_else(|| {
        Error::Usage(format!(
            "invalid --opt-level value `{value}`; expected 0, 1, 2, 3, s, or z"
        ))
    })
}

fn parse_tape_size(value: &str) -> Result<usize, Error> {
    value
        .parse::<usize>()
        .map_err(|_| Error::Usage(format!("invalid --tape-size value `{value}`")))
}

fn value_after_equals(value: &str) -> &str {
    value.split_once('=').map(|(_, value)| value).unwrap_or("")
}

fn print_targets() {
    println!("Known target presets:");
    for target in known_targets() {
        let triple = target.triple.unwrap_or("host default");
        let runtime = match target.runtime_abi {
            hypothalamus::target::RuntimeAbiKind::Hosted => "hosted",
            hypothalamus::target::RuntimeAbiKind::Freestanding => "freestanding",
        };
        println!(
            "  {:12} {:24} {:13} {}",
            target.name, triple, runtime, target.description
        );
    }
    println!();
    println!("Any Cranelift target triple also works: x86_64, aarch64, riscv64, and s390x.");
}

fn print_help() {
    println!(
        r#"Hypothalamus - Brainfuck AOT compiler with a Cranelift backend

Usage:
  hypothalamus [OPTIONS] <INPUT>
  hypothalamus tools doctor [OPTIONS]

Options:
  -o, --output <PATH>       Output path [default: from the input and emit kind]
      --emit <KIND>         exe, obj, or jit [default: target-specific]
      --jit, --run          Compile for the host and run it in this process
      --target <TARGET>     Target preset or raw target triple [default: native]
      --list-targets        Print built-in target presets
      --tape-size <CELLS>   Tape cell count [default: 30000]
      --bounds-check        Trap on out-of-range tape access
      --freestanding        Emit a callable Brainfuck payload for freestanding runtimes
      --entry <SYMBOL>      Freestanding entry function [default: bf_main]
      --putchar-symbol <S>  Freestanding output hook: void (u8) [default: bf_putchar]
      --getchar-symbol <S>  Freestanding input hook: int () [default: bf_getchar]
      --opt-level <LEVEL>   0, 1, 2, 3, s, or z [default: 2]
      --linker <PATH>       C compiler driver used to link executables
      --keep-object         Keep the generated object file beside the output
  -h, --help                Print help
      --version             Print version

Code generation is built in, so --emit obj and --run need no external tools.
Only --emit exe does, to link against the C runtime.

Commands:
  tools doctor              Inspect the backend, the linker, and target support"#
    );
}

fn print_tools_help() {
    println!(
        r#"Hypothalamus tool diagnostics

Usage:
  hypothalamus tools doctor [OPTIONS]

Options:
      --linker <PATH>       C compiler driver used to link executables
  -h, --help                Print help"#
    );
}

#[derive(Debug)]
enum Error {
    Usage(String),
    Failure(String),
}

impl From<DriverError> for Error {
    fn from(error: DriverError) -> Self {
        Self::Failure(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_config(args: &[&str]) -> CompilerConfig {
        let action = parse_args(args.iter().copied().map(String::from)).expect("valid args");

        let Action::Run(config) = action else {
            panic!("expected run action");
        };

        *config
    }

    #[test]
    fn parses_list_targets_action() {
        assert!(matches!(
            parse_args(["--list-targets"].map(String::from)),
            Ok(Action::ListTargets)
        ));
    }

    #[test]
    fn parses_tools_doctor_action() {
        let action = parse_args(["tools", "doctor", "--linker", "/tools/cc"].map(String::from))
            .expect("valid tools doctor args");

        let Action::ToolsDoctor(config) = action else {
            panic!("expected tools doctor action");
        };

        assert_eq!(config.linker.as_deref(), Some("/tools/cc"));
    }

    #[test]
    fn defaults_to_a_hosted_executable() {
        let config = run_config(&["hello.bf"]);

        assert_eq!(config.emit, EmitKind::Executable);
        assert_eq!(config.target.name(), "native");
        assert_eq!(config.opt_level, OptLevel::Speed);
        assert!(!config.target.is_freestanding());
    }

    #[test]
    fn freestanding_defaults_to_object_output() {
        let config = run_config(&["--freestanding", "kernel.bf"]);

        assert_eq!(config.emit, EmitKind::Object);
        assert!(config.target.is_freestanding());
        let RuntimeAbi::Freestanding(options) = config.target.runtime_abi() else {
            panic!("expected freestanding runtime");
        };
        assert_eq!(options.entry_symbol, "bf_main");
        assert_eq!(options.putchar_symbol, "bf_putchar");
        assert_eq!(options.getchar_symbol, "bf_getchar");
    }

    #[test]
    fn x86_64_none_target_defaults_to_object_output() {
        let config = run_config(&["--target", "x86_64-none", "examples/hello.bf"]);

        assert_eq!(config.emit, EmitKind::Object);
        assert_eq!(config.target.triple(), Some("x86_64-unknown-none-elf"));
        assert!(config.target.is_freestanding());
    }

    #[test]
    fn raw_target_triples_stay_hosted() {
        let config = run_config(&["--target", "x86_64-unknown-linux-gnu", "hello.bf"]);

        assert_eq!(config.emit, EmitKind::Executable);
        assert_eq!(config.target.triple(), Some("x86_64-unknown-linux-gnu"));
        assert!(!config.target.is_freestanding());
    }

    #[test]
    fn parses_the_linker_override_and_object_retention() {
        let config = run_config(&["--linker=/tools/cc", "--keep-object", "hello.bf"]);

        assert_eq!(config.linker.as_deref(), Some("/tools/cc"));
        assert!(config.keep_object);
    }

    #[test]
    fn folds_short_optimization_flags_onto_cranelift_levels() {
        assert_eq!(run_config(&["-O0", "hello.bf"]).opt_level, OptLevel::None);
        assert_eq!(run_config(&["-O3", "hello.bf"]).opt_level, OptLevel::Speed);
        assert_eq!(
            run_config(&["-Oz", "hello.bf"]).opt_level,
            OptLevel::SpeedAndSize
        );
    }

    #[test]
    fn freestanding_rejects_hosted_executable_output() {
        let err =
            parse_args(["--target", "x86_64-none", "--emit", "exe", "kernel.bf"].map(String::from))
                .expect_err("freestanding executable should be rejected");

        assert!(matches!(err, Error::Usage(message) if message.contains("freestanding runtime")));
    }

    #[test]
    fn freestanding_symbol_options_require_freestanding_runtime() {
        let err = parse_args(["--entry", "kernel_main", "hello.bf"].map(String::from))
            .expect_err("freestanding symbol should be rejected in hosted mode");

        assert!(matches!(err, Error::Usage(message) if message.contains("freestanding target")));
    }

    #[test]
    fn explains_options_the_llvm_backend_used_to_take() {
        for (arg, expected) in [
            ("--cc", "--linker"),
            ("--keep-ll", "--keep-object"),
            ("--gba-gcc", "32-bit ARM"),
        ] {
            let err = parse_args([arg, "clang", "hello.bf"].map(String::from))
                .expect_err("removed option should be rejected");

            assert!(
                matches!(&err, Error::Usage(message)
                    if message.contains("was removed") && message.contains(expected)),
                "{arg} produced {err:?}"
            );
        }
    }

    #[test]
    fn rejects_emit_kinds_the_llvm_backend_used_to_take() {
        for value in ["llvm-ir", "llvm-jit", "asm", "image"] {
            let err = parse_args(["--emit", value, "hello.bf"].map(String::from))
                .expect_err("removed emit kind should be rejected");

            assert!(
                matches!(err, Error::Usage(message) if message.contains("expected exe, obj, or jit"))
            );
        }
    }
}
