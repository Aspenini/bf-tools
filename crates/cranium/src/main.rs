//! Command-line driver for the Cranium compiler.

use cranium::lexer;
use cranium::module;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Emit {
    Brainfuck,
    Tokens,
    Ast,
}

#[derive(Debug)]
struct Options {
    input: PathBuf,
    output: Option<PathBuf>,
    emit: Emit,
    run: bool,
    stats: bool,
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_help();
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--version") {
        println!("cranium {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let options = match parse_args(&args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("cranium: {message}");
            eprintln!("run `cranium --help` for usage");
            return ExitCode::from(2);
        }
    };

    match compile(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut input = None;
    let mut output = None;
    let mut emit = Emit::Brainfuck;
    let mut run = false;
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
                output = Some(PathBuf::from(value));
            }
            "--emit" => {
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--emit` needs a value".to_string())?;
                index += 1;
                emit = match value.as_str() {
                    "bf" | "brainfuck" => Emit::Brainfuck,
                    "tokens" => Emit::Tokens,
                    "ast" => Emit::Ast,
                    other => return Err(format!("unknown emit kind `{other}`")),
                };
            }
            "-r" | "--run" => run = true,
            "--stats" => stats = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown option `{other}`"));
            }
            path => {
                if input.is_some() {
                    return Err("only one input file is supported".to_string());
                }
                input = Some(PathBuf::from(path));
            }
        }
    }

    Ok(Options {
        input: input.ok_or_else(|| "no input file".to_string())?,
        output,
        emit,
        run,
        stats,
    })
}

fn compile(options: &Options) -> Result<(), String> {
    let name = options.input.display().to_string();

    if options.emit == Emit::Tokens {
        // Tokens are a view of one file, so imports are not followed here.
        let source = fs::read_to_string(&options.input)
            .map_err(|err| format!("cranium: cannot read {name}: {err}"))?;
        let tokens = lexer::tokenize(&source, 0).map_err(|err| format!("{name}:{err}"))?;
        for token in &tokens {
            println!("{}	{:?}", token.span, token.tok);
        }
        return Ok(());
    }

    if options.emit == Emit::Ast {
        // The whole program, with every import already spliced in.
        let entry = module::Disk::entry_name(&options.input);
        let loaded = module::gather(&entry, &mut module::Disk)
            .map_err(|failure| format!("{name}: {}", failure.error))?;
        for file in loaded.sources.names() {
            println!("// {file}");
        }
        println!("{:#?}", loaded.program);
        return Ok(());
    }

    let compiled = cranium::compile_file(&options.input).map_err(|err| err.to_string())?;

    if options.stats {
        eprintln!(
            "cranium: {} brainfuck commands, {} tape cells",
            compiled.code.len(),
            compiled.cells_used
        );
        // Reaching an array costs its distance from the working set, and the
        // order below is the declaration order, so this is actionable.
        for array in &compiled.arrays {
            eprintln!(
                "cranium:   {} cells away: {} ({} cells)",
                array.base, array.name, array.cells
            );
        }
        if compiled.cells_used > 30_000 {
            eprintln!(
                "cranium: warning: this needs more than the usual 30,000-cell tape;                  run it with `hypothalamus --tape-size {}` or `lobe --tape-size {}`",
                compiled.cells_used, compiled.cells_used
            );
        }
    }

    if options.run {
        return run_program(&compiled.code, compiled.cells_used);
    }

    let destination = options
        .output
        .clone()
        .unwrap_or_else(|| default_output(&options.input));
    fs::write(&destination, wrap(&compiled.code))
        .map_err(|err| format!("cranium: cannot write {}: {err}", destination.display()))?;
    Ok(())
}

/// Break the emitted program into lines so the file stays readable in an
/// editor. Brainfuck ignores everything that is not a command.
fn wrap(code: &str) -> String {
    const WIDTH: usize = 72;
    let mut out = String::with_capacity(code.len() + code.len() / WIDTH + 1);
    for (index, ch) in code.chars().enumerate() {
        if index > 0 && index % WIDTH == 0 {
            out.push('\n');
        }
        out.push(ch);
    }
    out.push('\n');
    out
}

fn default_output(input: &Path) -> PathBuf {
    input.with_extension("bf")
}

fn run_program(code: &str, cells_used: usize) -> Result<(), String> {
    let tape_size = cells_used.max(30_000);
    let mut runtime = lobe::create_runtime_with_tape(code, lobe::CellSize::Bits8, tape_size)
        .map_err(|err| format!("cranium: {err}"))?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    runtime
        .run_with_io(&mut input, &mut output)
        .map_err(|err| format!("cranium: {err}"))
}

fn print_help() {
    println!(
        "cranium {} - compile Cranium to Brainfuck

USAGE:
    cranium <input.cra> [OPTIONS]

OPTIONS:
    -o, --output <PATH>   Where to write the Brainfuck (default: input with a .bf extension)
        --emit <KIND>     bf (default), tokens, or ast
    -r, --run             Run the program instead of writing it out
        --stats           Report program size and tape usage
    -h, --help            Show this message
        --version         Show the version

EXAMPLES:
    cranium hello.cra -o hello.bf
    cranium fizzbuzz.cra --run
    cranium life.cra --stats -o life.bf && hypothalamus life.bf -o life",
        env!("CARGO_PKG_VERSION")
    );
}
