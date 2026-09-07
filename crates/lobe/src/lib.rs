//! Lobe - A fast Brainfuck interpreter

mod interpreter;
mod parser;
mod types;

pub use interpreter::{Runtime, DEFAULT_TAPE_SIZE};
pub use types::{Bytecode, CellSize};

use anyhow::Result;

/// Create a new Runtime from Brainfuck source code
///
/// # Arguments
///
/// * `src` - Brainfuck source code
/// * `cell_size` - Cell size (8, 16, 32, or 64 bits)
///
/// # Errors
///
/// Returns an error if parsing fails (e.g., unmatched brackets)
pub fn create_runtime(src: &str, cell_size: CellSize) -> Result<Runtime> {
    create_runtime_with_tape(src, cell_size, DEFAULT_TAPE_SIZE)
}

/// Create a new Runtime with an explicit tape size
///
/// # Arguments
///
/// * `src` - Brainfuck source code
/// * `cell_size` - Cell size (8, 16, 32, or 64 bits)
/// * `tape_size` - Number of cells; zero selects [`DEFAULT_TAPE_SIZE`]
///
/// # Errors
///
/// Returns an error if parsing fails (e.g., unmatched brackets)
pub fn create_runtime_with_tape(
    src: &str,
    cell_size: CellSize,
    tape_size: usize,
) -> Result<Runtime> {
    let bytecode = parser::parse(src)?;
    Ok(Runtime::with_tape_size(bytecode, cell_size, tape_size))
}

/// Convenience function: parse and run a Brainfuck program
///
/// # Arguments
///
/// * `src` - Brainfuck source code
/// * `cell_size` - Cell size (8, 16, 32, or 64 bits)
///
/// # Errors
///
/// Returns an error if parsing or execution fails
pub fn run(src: &str, cell_size: CellSize) -> Result<()> {
    let mut runtime = create_runtime(src, cell_size)?;
    runtime.run()
}
