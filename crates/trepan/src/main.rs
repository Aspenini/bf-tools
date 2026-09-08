//! Command-line driver for trepan.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug)]
struct Options {
    input: PathBuf,
    output: Option<PathBuf>,
    stdout: bool,
    stats: bool,
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_help();
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--version") {
        println!("trepan {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let options = match parse_args(&args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("trepan: {message}");
            eprintln!("run `trepan --help` for usage");
            return ExitCode::from(2);
        }
    };

    match run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("trepan: {message}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut input = None;
    let mut output = None;
    let mut stdout = false;
    let mut stats = false;

    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        index += 1;
        match arg {
            "-o" | "--output" => {
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{arg}` needs a path"))?;
                index += 1;
                if value == "-" {
                    stdout = true;
                } else {
                    output = Some(PathBuf::from(value));
                }
            }
            "--stats" => stats = true,
            _ if arg.starts_with('-') && arg != "-" => {
                return Err(format!("unknown option `{arg}`"));
            }
            _ => {
                if input.is_some() {
                    return Err("more than one input file".to_string());
                }
                input = Some(PathBuf::from(arg));
            }
        }
    }

    Ok(Options {
        input: input.ok_or("no input file")?,
        output,
        stdout,
        stats,
    })
}

fn run(options: &Options) -> Result<(), String> {
    let source = fs::read(&options.input)
        .map_err(|error| format!("cannot read {}: {error}", options.input.display()))?;

    let label = options
        .input
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| options.input.display().to_string());

    let lifted = trepan::lift(&source, &label)
        .map_err(|error| format!("{}: {error}", options.input.display()))?;

    if options.stdout {
        print!("{}", lifted.source);
    } else {
        let path = options
            .output
            .clone()
            .unwrap_or_else(|| default_output(&options.input));
        fs::write(&path, &lifted.source)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }

    if options.stats {
        eprintln!(
            "trepan: {} cells lifted to variables, {} statements",
            lifted.cells, lifted.statements
        );
    }

    Ok(())
}

fn default_output(input: &Path) -> PathBuf {
    input.with_extension("cra")
}

fn print_help() {
    println!("Lift Brainfuck back into structured Cranium.");
    println!();
    println!("Usage: trepan <input.bf> [OPTIONS]");
    println!();
    println!("Options:");
    println!("  -o, --output <PATH>   Where to write the Cranium (default: input with a .cra");
    println!("                        extension; `-` writes to standard output)");
    println!("      --stats           Report how many cells and statements came out");
    println!("  -h, --help            Show this message");
    println!("      --version         Show the version");
    println!();
    println!("Only programs whose data pointer is static can be lifted: no `[>]`-style");
    println!("scans, and every loop has to leave the pointer where it found it. Anything");
    println!("else needs the tape itself, and is declined rather than turned into a much");
    println!("larger program than it started as.");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn takes_an_input_and_an_output() {
        let options = parse_args(&args(&["in.bf", "-o", "out.cra"])).expect("parses");

        assert_eq!(options.input, PathBuf::from("in.bf"));
        assert_eq!(options.output, Some(PathBuf::from("out.cra")));
        assert!(!options.stdout);
    }

    #[test]
    fn a_dash_means_standard_output() {
        let options = parse_args(&args(&["in.bf", "-o", "-"])).expect("parses");

        assert!(options.stdout);
        assert_eq!(options.output, None);
    }

    #[test]
    fn defaults_the_output_to_a_cra_beside_the_input() {
        assert_eq!(
            default_output(Path::new("bf/hello.bf")),
            PathBuf::from("bf/hello.cra")
        );
    }

    #[test]
    fn rejects_a_second_input() {
        assert!(parse_args(&args(&["one.bf", "two.bf"])).is_err());
    }

    #[test]
    fn rejects_an_unknown_option() {
        assert!(parse_args(&args(&["in.bf", "--lift-everything"])).is_err());
    }

    #[test]
    fn needs_an_input() {
        assert!(parse_args(&args(&["--stats"])).is_err());
    }
}
