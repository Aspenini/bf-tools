//! Optimized intermediate representation for Brainfuck programs.
//!
//! The parser's [`crate::bf::Op`] values stay intentionally close to the source
//! language. This module lowers those operations into a representation that is
//! easier for backends to emit efficiently:
//!
//! - arithmetic can target cells at a fixed offset from the current pointer;
//! - clear operations become direct stores;
//! - scan loops such as `[>]` become explicit pointer searches;
//! - canonical transfer and multiply-transfer loops become a single operation.

use crate::bf::Op;
use std::collections::BTreeMap;

/// An optimized Brainfuck operation.
///
/// Offsets are relative to the current data pointer at the point where the
/// operation executes. [`Ir::Move`] is the only operation that changes the
/// current pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ir {
    /// Add `delta` to the cell at `offset`, wrapping modulo 256.
    Add {
        /// Cell offset from the current data pointer.
        offset: i64,

        /// Wrapping byte delta in the range `1..=255`.
        delta: i32,
    },

    /// Set the cell at `offset` to `value`.
    Set {
        /// Cell offset from the current data pointer.
        offset: i64,

        /// New byte value for the target cell.
        value: u8,
    },

    /// Move the current data pointer by `delta` cells.
    Move(i64),

    /// Read one byte into the cell at `offset`.
    ///
    /// End-of-file leaves the cell unchanged.
    Input {
        /// Cell offset from the current data pointer.
        offset: i64,
    },

    /// Write the cell at `offset` to standard output.
    Output {
        /// Cell offset from the current data pointer.
        offset: i64,
    },

    /// Execute `body` while the current cell is nonzero.
    Loop(Vec<Ir>),

    /// Move by `stride` until the current cell is zero.
    ///
    /// This is emitted for loops such as `[>]`, `[<]`, `[>>]`, and `[<<]`.
    Scan(i64),

    /// Add multiples of the current cell to other cells, then clear it.
    ///
    /// For each `(offset, factor)`, the target cell receives
    /// `current_cell * factor` modulo 256. The current cell is then set to zero.
    AddMul {
        /// Target offsets and wrapping multiplication factors.
        terms: Vec<(i64, i32)>,
    },
}

/// Lower parsed Brainfuck operations into optimized IR.
///
/// This function preserves Brainfuck's wrapping byte-cell semantics and keeps
/// pointer-boundary behavior undefined, matching the parser and LLVM backend.
pub fn optimize(ops: &[Op]) -> Vec<Ir> {
    lower_block(ops)
}

fn lower_block(ops: &[Op]) -> Vec<Ir> {
    let mut output = Vec::new();
    let mut pending_adds = BTreeMap::new();
    let mut cursor = 0_i64;

    for op in ops {
        match op {
            Op::Add(delta) => {
                add_pending(&mut pending_adds, cursor, *delta);
            }
            Op::Move(delta) => {
                cursor += *delta;
            }
            Op::Clear => {
                pending_adds.remove(&cursor);
                push_ir(
                    &mut output,
                    Ir::Set {
                        offset: cursor,
                        value: 0,
                    },
                );
            }
            Op::Input => {
                flush_pending_adds(&mut output, &mut pending_adds);
                push_ir(&mut output, Ir::Input { offset: cursor });
            }
            Op::Output => {
                flush_pending_adds(&mut output, &mut pending_adds);
                push_ir(&mut output, Ir::Output { offset: cursor });
            }
            Op::Loop(body) => {
                flush_pending_adds(&mut output, &mut pending_adds);
                if cursor != 0 {
                    push_ir(&mut output, Ir::Move(cursor));
                    cursor = 0;
                }

                let body = lower_block(body);
                push_ir(&mut output, optimize_loop(body));
            }
        }
    }

    flush_pending_adds(&mut output, &mut pending_adds);
    if cursor != 0 {
        push_ir(&mut output, Ir::Move(cursor));
    }

    output
}

fn add_pending(pending_adds: &mut BTreeMap<i64, i32>, offset: i64, delta: i32) {
    let next = (pending_adds.get(&offset).copied().unwrap_or_default() + delta).rem_euclid(256);
    if next == 0 {
        pending_adds.remove(&offset);
    } else {
        pending_adds.insert(offset, next);
    }
}

fn flush_pending_adds(output: &mut Vec<Ir>, pending_adds: &mut BTreeMap<i64, i32>) {
    for (offset, delta) in std::mem::take(pending_adds) {
        push_ir(output, Ir::Add { offset, delta });
    }
}

fn optimize_loop(body: Vec<Ir>) -> Ir {
    if let [Ir::Move(stride)] = body.as_slice()
        && *stride != 0
    {
        return Ir::Scan(*stride);
    }

    if let Some(terms) = add_mul_terms(&body) {
        if terms.is_empty() {
            Ir::Set {
                offset: 0,
                value: 0,
            }
        } else {
            Ir::AddMul { terms }
        }
    } else {
        Ir::Loop(body)
    }
}

fn add_mul_terms(body: &[Ir]) -> Option<Vec<(i64, i32)>> {
    let mut effects = BTreeMap::new();

    for op in body {
        match op {
            Ir::Add { offset, delta } => add_pending(&mut effects, *offset, *delta),
            _ => return None,
        }
    }

    let source_delta = effects.remove(&0)?;
    let inverse = modular_inverse_256(source_delta)?;

    let mut terms = Vec::new();
    for (offset, delta) in effects {
        let factor = (-delta * inverse).rem_euclid(256);
        if factor != 0 {
            terms.push((offset, factor));
        }
    }

    Some(terms)
}

fn modular_inverse_256(value: i32) -> Option<i32> {
    let value = value.rem_euclid(256);
    if value % 2 == 0 {
        return None;
    }

    (1..256).find(|candidate| (value * candidate).rem_euclid(256) == 1)
}

fn push_ir(output: &mut Vec<Ir>, op: Ir) {
    match op {
        Ir::Add { offset, delta } => {
            let delta = delta.rem_euclid(256);
            if delta == 0 {
                return;
            }

            match output.last_mut() {
                Some(Ir::Add {
                    offset: previous_offset,
                    delta: previous_delta,
                }) if *previous_offset == offset => {
                    *previous_delta = (*previous_delta + delta).rem_euclid(256);
                    if *previous_delta == 0 {
                        output.pop();
                    }
                }
                Some(Ir::Set {
                    offset: previous_offset,
                    value,
                }) if *previous_offset == offset => {
                    *value = value.wrapping_add(delta as u8);
                }
                _ => output.push(Ir::Add { offset, delta }),
            }
        }
        Ir::Set { offset, value } => {
            if let Some(last) = output.last_mut() {
                match last {
                    Ir::Add {
                        offset: previous_offset,
                        ..
                    }
                    | Ir::Set {
                        offset: previous_offset,
                        ..
                    } if *previous_offset == offset => {
                        *last = Ir::Set { offset, value };
                        return;
                    }
                    _ => {}
                }
            }

            output.push(Ir::Set { offset, value });
        }
        Ir::Move(delta) => {
            if delta == 0 {
                return;
            }

            if let Some(Ir::Move(previous)) = output.last_mut() {
                *previous += delta;
                if *previous == 0 {
                    output.pop();
                }
            } else {
                output.push(Ir::Move(delta));
            }
        }
        Ir::Scan(0) => {}
        other => output.push(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combines_offset_arithmetic() {
        let ops = crate::bf::parse(b">+<->").expect("valid program");
        assert_eq!(
            optimize(&ops),
            vec![
                Ir::Add {
                    offset: 0,
                    delta: 255
                },
                Ir::Add {
                    offset: 1,
                    delta: 1
                },
                Ir::Move(1)
            ]
        );
    }

    #[test]
    fn removes_dead_add_before_clear() {
        let ops = crate::bf::parse(b"+++[-]").expect("valid program");
        assert_eq!(
            optimize(&ops),
            vec![Ir::Set {
                offset: 0,
                value: 0
            }]
        );
    }

    #[test]
    fn recognizes_scan_loops() {
        let ops = crate::bf::parse(b"[>]").expect("valid program");
        assert_eq!(optimize(&ops), vec![Ir::Scan(1)]);
    }

    #[test]
    fn recognizes_multiply_transfer_loops() {
        let ops = crate::bf::parse(b"[->+++>++<<]").expect("valid program");
        assert_eq!(
            optimize(&ops),
            vec![Ir::AddMul {
                terms: vec![(1, 3), (2, 2)]
            }]
        );
    }

    #[test]
    fn recognizes_incrementing_transfer_loops() {
        let ops = crate::bf::parse(b"[+>+<]").expect("valid program");
        assert_eq!(
            optimize(&ops),
            vec![Ir::AddMul {
                terms: vec![(1, 255)]
            }]
        );
    }

    #[test]
    fn leaves_non_terminating_linear_loops_alone() {
        let ops = crate::bf::parse(b"[--]").expect("valid program");
        assert_eq!(
            optimize(&ops),
            vec![Ir::Loop(vec![Ir::Add {
                offset: 0,
                delta: 254
            }])]
        );
    }
}
