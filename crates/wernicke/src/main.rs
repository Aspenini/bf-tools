//! The `wernicke` command: a Cranium language server speaking over stdio.

use std::io::{self, BufReader};
use std::process::ExitCode;
use wernicke::server::Server;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None | Some("--stdio") => {}
        Some("--version") => {
            println!("wernicke {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("-h" | "--help") => {
            print_help();
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("wernicke: unknown option `{other}`");
            eprintln!("run `wernicke --help` for usage");
            return ExitCode::from(2);
        }
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = BufReader::new(stdin.lock());
    let mut output = stdout.lock();

    match Server::new().serve(&mut input, &mut output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("wernicke: {err}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "wernicke {} - a language server for Cranium

USAGE:
    wernicke [--stdio]

It speaks the language server protocol on standard input and output, so it is
started by an editor rather than by hand. Point your editor's Cranium client at
this binary; `--stdio` is accepted and is the default.

OPTIONS:
    --stdio       Serve over standard input and output (the default)
    -h, --help    Show this message
        --version Show the version",
        env!("CARGO_PKG_VERSION")
    );
}
