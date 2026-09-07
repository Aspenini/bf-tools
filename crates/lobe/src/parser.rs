use crate::types::{Bytecode, Instr};
use anyhow::{Result, anyhow};

/// Parse Brainfuck source code into bytecode
///
/// Strips all non-BF characters and matches brackets.
/// Returns an error if brackets are unmatched.
pub fn parse(src: &str) -> Result<Bytecode> {
    // Match brackets and build the instruction vector in one pass.
    let mut instrs = Vec::new();
    let mut bracket_stack: Vec<usize> = Vec::new(); // Stack of instruction indices for '['

    for ch in src
        .chars()
        .filter(|&c| matches!(c, '>' | '<' | '+' | '-' | '.' | ',' | '[' | ']'))
    {
        let instr_idx = instrs.len();

        match ch {
            '>' => instrs.push(Instr::IncrementPtr),
            '<' => instrs.push(Instr::DecrementPtr),
            '+' => instrs.push(Instr::Increment),
            '-' => instrs.push(Instr::Decrement),
            '.' => instrs.push(Instr::Output),
            ',' => instrs.push(Instr::Input),
            '[' => {
                bracket_stack.push(instr_idx);
                instrs.push(Instr::JumpIfZero(0)); // Placeholder, will be fixed
            }
            ']' => {
                let open_idx = bracket_stack
                    .pop()
                    .ok_or_else(|| anyhow!("Unmatched closing bracket"))?;
                instrs.push(Instr::JumpIfNonZero(open_idx));

                match &mut instrs[open_idx] {
                    Instr::JumpIfZero(target) => *target = instr_idx,
                    _ => unreachable!(),
                }
            }
            _ => {} // Should never happen after filtering, but satisfies exhaustiveness
        }
    }

    if !bracket_stack.is_empty() {
        return Err(anyhow!(
            "Unmatched opening brackets: {} remaining",
            bracket_stack.len()
        ));
    }

    Ok(Bytecode { instrs })
}
