use occipital::{Error, Options, Program, screen};
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            eprintln!("occipital: {message}");
            eprintln!("run `occipital --help` for usage");
            ExitCode::from(2)
        }
        Err(Failure::Program(error)) => {
            eprintln!("occipital: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Failure> {
    let mut input = None;
    let mut options = Options::default();
    let mut title = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            "--version" => {
                println!("occipital {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--scale" => options.scale = parse_scale(&next_value(&mut args, &arg)?)?,
            "--title" => title = Some(next_value(&mut args, &arg)?),
            "--no-wait" => options.wait_on_exit = false,
            _ if arg.starts_with("--scale=") => {
                options.scale = parse_scale(value_after_equals(&arg))?;
            }
            _ if arg.starts_with("--title=") => {
                title = Some(value_after_equals(&arg).to_string());
            }
            _ if arg.starts_with('-') => {
                return Err(Failure::Usage(format!("unknown option `{arg}`")));
            }
            _ if input.is_some() => {
                return Err(Failure::Usage("multiple input files provided".to_string()));
            }
            _ => input = Some(arg),
        }
    }

    let input = input.ok_or_else(|| Failure::Usage("missing input file".to_string()))?;
    let program = Program::load(&input)?;

    options.title = title.unwrap_or_else(|| format!("occipital - {}", program.name()));
    program.run(&options)?;
    Ok(())
}

fn next_value(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, Failure> {
    args.next()
        .ok_or_else(|| Failure::Usage(format!("missing value for {option}")))
}

fn value_after_equals(value: &str) -> &str {
    value.split_once('=').map(|(_, value)| value).unwrap_or("")
}

fn parse_scale(value: &str) -> Result<u8, Failure> {
    value
        .parse::<u8>()
        .ok()
        .filter(|scale| *scale > 0)
        .ok_or_else(|| Failure::Usage(format!("invalid --scale value `{value}`")))
}

fn print_help() {
    println!(
        r#"Occipital - a window for Brainfuck programs

Usage:
  occipital [OPTIONS] <PROGRAM>

<PROGRAM> is a .cra file, which is compiled to Brainfuck first, or a .bf file.

Options:
      --scale <N>       Magnify the {width}x{height} screen [default: 3]
      --title <TEXT>    Window title [default: from the program name]
      --no-wait         Close the window as soon as the program ends
  -h, --help            Print help
      --version         Print version

The program's own output drives the window. Plain bytes are text, drawn with
the built-in font, so a program written for a terminal works unchanged. A byte
of 0x10 escapes a drawing command; see the crate documentation for the list.

Reads return typed characters by default, and switch to non-blocking input
events once the program asks for them, which is what a game loop wants."#,
        width = screen::WIDTH,
        height = screen::HEIGHT,
    );
}

#[derive(Debug)]
enum Failure {
    Usage(String),
    Program(Error),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Program(error)
    }
}
