use clap::{CommandFactory, Parser, ValueEnum};
use lobe::{create_runtime, CellSize};
use std::fs;
use std::process;

#[derive(Parser)]
#[command(name = "lobe")]
#[command(about = "A fast Brainfuck interpreter")]
struct Cli {
    /// Brainfuck source file to run
    file: Option<String>,

    /// Cell size in bits (8, 16, 32, or 64)
    #[arg(short, long, default_value = "8", value_enum)]
    bits: CellSizeArg,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CellSizeArg {
    /// 8-bit cells (0-255) - standard Brainfuck
    #[value(name = "8")]
    Bits8,
    /// 16-bit cells (0-65535)
    #[value(name = "16")]
    Bits16,
    /// 32-bit cells (0-4294967295)
    #[value(name = "32")]
    Bits32,
    /// 64-bit cells (0-18446744073709551615)
    #[value(name = "64")]
    Bits64,
}

impl From<CellSizeArg> for CellSize {
    fn from(arg: CellSizeArg) -> Self {
        match arg {
            CellSizeArg::Bits8 => CellSize::Bits8,
            CellSizeArg::Bits16 => CellSize::Bits16,
            CellSizeArg::Bits32 => CellSize::Bits32,
            CellSizeArg::Bits64 => CellSize::Bits64,
        }
    }
}

fn main() {
    let cli = Cli::parse();

    // If no file is provided, print the ASCII logo and help info
    let file = match cli.file {
        Some(f) => f,
        None => {
            println!("██╗      ██████╗ ██████╗ ███████╗");
            println!("██║     ██╔═══██╗██╔══██╗██╔════╝");
            println!("██║     ██║   ██║██████╔╝█████╗  ");
            println!("██║     ██║   ██║██╔══██╗██╔══╝  ");
            println!("███████╗╚██████╔╝██████╔╝███████╗");
            println!("╚══════╝ ╚═════╝ ╚═════╝ ╚══════╝");
            println!();
            Cli::command().print_help().unwrap();
            return;
        }
    };

    // Read the source file
    let src = match fs::read_to_string(&file) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Error reading file '{}': {}", file, e);
            process::exit(1);
        }
    };

    // Create runtime and run
    let cell_size = cli.bits.into();
    let mut runtime = match create_runtime(&src, cell_size) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    };

    if let Err(e) = runtime.run() {
        eprintln!("Runtime error: {}", e);
        process::exit(1);
    }
}
