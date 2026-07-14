//! Built-in Brainfuck runner for direct execution without LLVM tools.

use crate::bf::Op;
use crate::ir::{self, Ir};
use std::fmt;
use std::io::{self, Read, Write};

#[derive(Debug)]
pub(crate) enum RunError {
    EmptyTape,
    PointerOutOfBounds { pointer: i128 },
    Input(io::Error),
    Output(io::Error),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTape => write!(f, "tape size must be greater than zero"),
            Self::PointerOutOfBounds { pointer } => {
                write!(f, "data pointer moved out of bounds to cell {pointer}")
            }
            Self::Input(error) => write!(f, "failed to read stdin: {error}"),
            Self::Output(error) => write!(f, "failed to write stdout: {error}"),
        }
    }
}

impl std::error::Error for RunError {}

pub(crate) fn run_ops(
    ops: &[Op],
    tape_size: usize,
    input: &mut impl Read,
    output: &mut impl Write,
) -> Result<(), RunError> {
    if tape_size == 0 {
        return Err(RunError::EmptyTape);
    }

    let ir = ir::optimize(ops);
    let mut state = State {
        tape: vec![0; tape_size],
        pointer: 0,
    };
    execute_block(&ir, &mut state, input, output)?;
    output.flush().map_err(RunError::Output)
}

fn execute_block(
    ops: &[Ir],
    state: &mut State,
    input: &mut impl Read,
    output: &mut impl Write,
) -> Result<(), RunError> {
    for op in ops {
        match op {
            Ir::Add { offset, delta } => {
                let cell = state.cell_mut(*offset)?;
                *cell = cell.wrapping_add(*delta as u8);
            }
            Ir::Set { offset, value } => {
                *state.cell_mut(*offset)? = *value;
            }
            Ir::Move(delta) => {
                state.pointer = state.index(*delta)?;
            }
            Ir::Input { offset } => {
                if let Some(byte) = read_byte(input)? {
                    *state.cell_mut(*offset)? = byte;
                }
            }
            Ir::Output { offset } => {
                let byte = *state.cell_mut(*offset)?;
                output.write_all(&[byte]).map_err(RunError::Output)?;
            }
            Ir::Loop(body) => {
                while state.tape[state.pointer] != 0 {
                    execute_block(body, state, input, output)?;
                }
            }
            Ir::Scan(stride) => {
                while state.tape[state.pointer] != 0 {
                    state.pointer = state.index(*stride)?;
                }
            }
            Ir::AddMul { terms } => {
                let source = state.tape[state.pointer];
                for (offset, factor) in terms {
                    let cell = state.cell_mut(*offset)?;
                    *cell = cell.wrapping_add(source.wrapping_mul(*factor as u8));
                }
                state.tape[state.pointer] = 0;
            }
        }
    }

    Ok(())
}

fn read_byte(input: &mut impl Read) -> Result<Option<u8>, RunError> {
    let mut byte = [0];
    loop {
        match input.read(&mut byte) {
            Ok(0) => return Ok(None),
            Ok(_) => return Ok(Some(byte[0])),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(RunError::Input(error)),
        }
    }
}

#[derive(Debug)]
struct State {
    tape: Vec<u8>,
    pointer: usize,
}

impl State {
    fn cell_mut(&mut self, offset: i64) -> Result<&mut u8, RunError> {
        let index = self.index(offset)?;
        Ok(&mut self.tape[index])
    }

    fn index(&self, offset: i64) -> Result<usize, RunError> {
        let pointer = self.pointer as i128 + offset as i128;
        if pointer < 0 || pointer >= self.tape.len() as i128 {
            return Err(RunError::PointerOutOfBounds { pointer });
        }
        Ok(pointer as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bf;

    fn run(source: &[u8], input: &[u8]) -> Result<Vec<u8>, RunError> {
        let ops = bf::parse(source).expect("valid Brainfuck");
        let mut input = input;
        let mut output = Vec::new();
        run_ops(&ops, 30_000, &mut input, &mut output)?;
        Ok(output)
    }

    #[test]
    fn runs_output_program() {
        assert_eq!(run(b"+++++[>++++++++<-]>+.", b"").unwrap(), b")");
    }

    #[test]
    fn reads_stdin_and_leaves_cell_on_eof() {
        assert_eq!(run(b",+.,+.", b"A").unwrap(), b"BC");
    }

    #[test]
    fn reports_pointer_underflow() {
        let ops = bf::parse(b"<").expect("valid Brainfuck");
        let mut input = &b""[..];
        let mut output = Vec::new();

        assert!(matches!(
            run_ops(&ops, 30_000, &mut input, &mut output),
            Err(RunError::PointerOutOfBounds { pointer: -1 })
        ));
    }
}
