use crate::types::{Bytecode, CellSize, Instr};
use anyhow::Result;
use std::io::{self, Read, Write};

/// Fixed tape size per original Brainfuck specification
const TAPE_SIZE: usize = 30000;

/// Runtime state for executing Brainfuck programs
pub struct Runtime {
    bytecode: Bytecode,
    tape: Vec<u64>,
    dp: usize,
    ip: usize, // instruction pointer
    cell_size: CellSize,
}

impl Runtime {
    /// Create a new runtime with the given bytecode and cell size
    pub fn new(bytecode: Bytecode, cell_size: CellSize) -> Self {
        Self {
            bytecode,
            tape: vec![0u64; TAPE_SIZE], // Fixed-size tape per original spec
            dp: 0,
            ip: 0,
            cell_size,
        }
    }

    /// Execute the program following original Brainfuck rules:
    /// - Fixed 30,000 cell tape
    /// - Pointer wraps around at both ends
    /// - Cell values with wrapping based on cell size
    pub fn run(&mut self) -> Result<()> {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut input = stdin.lock();
        let mut output = stdout.lock();

        self.run_with_io(&mut input, &mut output)
    }

    /// Execute the program with custom input and output streams.
    pub fn run_with_io<R: Read, W: Write>(&mut self, input: &mut R, output: &mut W) -> Result<()> {
        while self.ip < self.bytecode.instrs.len() {
            match &self.bytecode.instrs[self.ip] {
                Instr::IncrementPtr => {
                    // Wrap around: if at end, go to beginning
                    self.dp = (self.dp + 1) % TAPE_SIZE;
                    self.ip += 1;
                }
                Instr::DecrementPtr => {
                    // Wrap around: if at beginning, go to end
                    self.dp = if self.dp == 0 {
                        TAPE_SIZE - 1
                    } else {
                        self.dp - 1
                    };
                    self.ip += 1;
                }
                Instr::Increment => {
                    let value = self.tape[self.dp].wrapping_add(1);
                    self.tape[self.dp] = self.cell_size.mask(value);
                    self.ip += 1;
                }
                Instr::Decrement => {
                    let value = self.tape[self.dp].wrapping_sub(1);
                    self.tape[self.dp] = self.cell_size.mask(value);
                    self.ip += 1;
                }
                Instr::Output => {
                    let value = self.tape[self.dp];
                    if let Some(ch) = self.cell_size.to_char(value) {
                        write!(output, "{}", ch)?;
                    } else {
                        // For non-8-bit, output as number
                        write!(output, "{}", value)?;
                    }
                    output.flush()?;
                    self.ip += 1;
                }
                Instr::Input => {
                    let mut buf = [0u8; 1];
                    match input.read_exact(&mut buf) {
                        Ok(_) => {
                            let input_value = buf[0] as u64;
                            self.tape[self.dp] = self.cell_size.mask(input_value);
                        }
                        Err(_) => self.tape[self.dp] = 0, // EOF behavior: set to 0
                    }
                    self.ip += 1;
                }
                Instr::JumpIfZero(target) => {
                    if self.tape[self.dp] == 0 {
                        self.ip = *target + 1;
                    } else {
                        self.ip += 1;
                    }
                }
                Instr::JumpIfNonZero(target) => {
                    if self.tape[self.dp] != 0 {
                        self.ip = *target;
                    } else {
                        self.ip += 1;
                    }
                }
            }
        }

        Ok(())
    }
}
