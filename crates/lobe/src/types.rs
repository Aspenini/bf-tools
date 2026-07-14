//! Core data structures for the Brainfuck interpreter

/// Brainfuck instruction representation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instr {
    /// `>` - Increment data pointer
    IncrementPtr,
    /// `<` - Decrement data pointer
    DecrementPtr,
    /// `+` - Increment value at data pointer
    Increment,
    /// `-` - Decrement value at data pointer
    Decrement,
    /// `.` - Output character at data pointer
    Output,
    /// `,` - Input character and store at data pointer
    Input,
    /// `[` - Jump if zero to matching `]` (index is the matching bracket position)
    JumpIfZero(usize),
    /// `]` - Jump if non-zero to matching `[` (index is the matching bracket position)
    JumpIfNonZero(usize),
}

/// Compiled bytecode representation
#[derive(Debug, Clone)]
pub struct Bytecode {
    pub instrs: Vec<Instr>,
}

/// Cell size for Brainfuck interpreter
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellSize {
    /// 8-bit cells (0-255)
    Bits8,
    /// 16-bit cells (0-65535)
    Bits16,
    /// 32-bit cells (0-4294967295)
    Bits32,
    /// 64-bit cells (0-18446744073709551615)
    Bits64,
}

impl CellSize {
    /// Get the maximum value for this cell size
    pub fn max_value(self) -> u64 {
        match self {
            CellSize::Bits8 => u8::MAX as u64,
            CellSize::Bits16 => u16::MAX as u64,
            CellSize::Bits32 => u32::MAX as u64,
            CellSize::Bits64 => u64::MAX,
        }
    }

    /// Mask a value to fit within this cell size
    pub fn mask(self, value: u64) -> u64 {
        value & self.max_value()
    }

    /// Convert a value to a character for output (only valid for 8-bit)
    pub fn to_char(self, value: u64) -> Option<char> {
        match self {
            CellSize::Bits8 => {
                let masked = self.mask(value) as u8;
                Some(masked as char)
            }
            _ => None,
        }
    }
}
