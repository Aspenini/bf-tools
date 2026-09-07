use crate::types::{Bytecode, CellSize, Instr};
use anyhow::Result;
use std::io::{self, Read, Write};

/// Tape size used when a program does not ask for a different one.
pub const DEFAULT_TAPE_SIZE: usize = 30000;

/// Runtime state for executing Brainfuck programs
pub struct Runtime {
    bytecode: Bytecode,
    tape: Vec<u64>,
    dp: usize,
    ip: usize, // instruction pointer
    cell_size: CellSize,
    tape_size: usize,
}

impl Runtime {
    /// Create a new runtime with the given bytecode and cell size
    pub fn new(bytecode: Bytecode, cell_size: CellSize) -> Self {
        Self::with_tape_size(bytecode, cell_size, DEFAULT_TAPE_SIZE)
    }

    /// Create a runtime with a tape of `tape_size` cells.
    ///
    /// Compilers that target Brainfuck often lay out arrays and scratch space
    /// well past the traditional 30,000 cells, so the size is configurable.
    /// A size of zero is treated as [`DEFAULT_TAPE_SIZE`].
    pub fn with_tape_size(bytecode: Bytecode, cell_size: CellSize, tape_size: usize) -> Self {
        let tape_size = if tape_size == 0 {
            DEFAULT_TAPE_SIZE
        } else {
            tape_size
        };
        Self {
            bytecode,
            tape: vec![0u64; tape_size],
            dp: 0,
            ip: 0,
            cell_size,
            tape_size,
        }
    }

    /// Number of cells on this runtime's tape.
    pub fn tape_size(&self) -> usize {
        self.tape_size
    }

    /// Execute the program following original Brainfuck rules:
    /// - Fixed-size tape (30,000 cells unless configured otherwise)
    /// - Pointer wraps around at both ends
    /// - Cell values with wrapping based on cell size
    pub fn run(&mut self) -> Result<()> {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut input = stdin.lock();
        let mut output = stdout.lock();

        let result = self.run_with_io(&mut input, &mut output);
        output.flush()?;
        result
    }

    /// Execute the program with custom input and output streams.
    ///
    /// Output is not flushed after every byte; it is flushed before each read
    /// so prompts appear, and callers should flush once when `run_with_io`
    /// returns. [`Runtime::run`] does that for standard output.
    pub fn run_with_io<R: Read, W: Write>(&mut self, input: &mut R, output: &mut W) -> Result<()> {
        // Hoisting these out of the loop keeps the hot path free of an enum
        // match and a division per instruction.
        let mask = self.cell_size.max_value();
        let bytes_out = matches!(self.cell_size, CellSize::Bits8);
        let last_cell = self.tape_size - 1;

        while self.ip < self.bytecode.instrs.len() {
            match &self.bytecode.instrs[self.ip] {
                Instr::IncrementPtr => {
                    // Wrap around: if at end, go to beginning
                    self.dp = if self.dp == last_cell { 0 } else { self.dp + 1 };
                    self.ip += 1;
                }
                Instr::DecrementPtr => {
                    // Wrap around: if at beginning, go to end
                    self.dp = if self.dp == 0 { last_cell } else { self.dp - 1 };
                    self.ip += 1;
                }
                Instr::Increment => {
                    self.tape[self.dp] = self.tape[self.dp].wrapping_add(1) & mask;
                    self.ip += 1;
                }
                Instr::Decrement => {
                    self.tape[self.dp] = self.tape[self.dp].wrapping_sub(1) & mask;
                    self.ip += 1;
                }
                Instr::Output => {
                    let value = self.tape[self.dp];
                    if bytes_out {
                        // `.` writes the cell as one raw byte. Encoding it as
                        // text instead would turn every value above 127 into
                        // two bytes and disagree with compiled Brainfuck.
                        output.write_all(&[value as u8])?;
                    } else {
                        // Wider cells have no single-byte meaning, so they are
                        // written as decimal numbers.
                        write!(output, "{value}")?;
                    }
                    self.ip += 1;
                }
                Instr::Input => {
                    output.flush()?;
                    let mut buf = [0u8; 1];
                    match input.read_exact(&mut buf) {
                        Ok(_) => self.tape[self.dp] = u64::from(buf[0]) & mask,
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
