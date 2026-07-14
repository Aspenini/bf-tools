//! Brainfuck parsing and local normalization.
//!
//! This module turns source bytes into a tree of [`Op`](crate::bf::Op) values.
//! It ignores all non-command bytes, validates bracket nesting, and performs
//! small local optimizations while preserving Brainfuck semantics:
//!
//! - consecutive additions are folded modulo 256;
//! - consecutive pointer moves are folded;
//! - no-op folded runs are removed;
//! - simple clear loops such as `[-]` and `[+]` become [`Op::Clear`](crate::bf::Op::Clear).
//!
//! The output is deliberately close to Brainfuck's instruction model, which
//! keeps backend code generation straightforward while avoiding work for common
//! instruction runs.

use std::fmt;

/// A normalized Brainfuck operation.
///
/// Operations are produced by [`parse`] and consumed by code-generation
/// backends such as [`crate::llvm`]. The parser folds adjacent operations where
/// possible, so the values here may represent more than one source command.
///
/// # Examples
///
/// ```
/// use hypothalamus::bf::{self, Op};
///
/// let ops = bf::parse(b"+++>>.").expect("valid Brainfuck");
/// assert_eq!(ops, vec![Op::Add(3), Op::Move(2), Op::Output]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Add `delta` to the current cell, wrapping modulo 256.
    ///
    /// The parser stores deltas in the range `1..=255`; subtract operations are
    /// represented by their wrapped equivalent. For example, `-` becomes
    /// `Add(255)`.
    Add(i32),

    /// Move the data pointer by `delta` cells.
    ///
    /// Positive values move right and negative values move left. Hypothalamus
    /// does not insert bounds checks; moving outside the generated tape remains
    /// undefined behavior, matching the traditional Brainfuck model.
    Move(i64),

    /// Read one byte from standard input into the current cell.
    ///
    /// The LLVM backend implements this with `getchar`. End-of-file leaves the
    /// current cell unchanged.
    Input,

    /// Write the current cell to standard output as one byte.
    ///
    /// The LLVM backend implements this with `putchar`.
    Output,

    /// Execute `body` repeatedly while the current cell is nonzero.
    ///
    /// Loops may contain nested [`Op::Loop`] values. The parser validates that
    /// all source brackets match before returning operations.
    Loop(Vec<Op>),

    /// Set the current cell to zero.
    ///
    /// This operation is emitted for canonical clear loops such as `[-]` and
    /// `[+]`. It gives backends a direct way to generate a store instead of a
    /// runtime loop.
    Clear,
}

/// Byte and line/column location of a source character.
///
/// Positions are reported in [`SyntaxError`] values. `byte` is a zero-based
/// byte offset into the original input, while `line` and `column` are one-based
/// counters intended for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePosition {
    /// Zero-based byte offset into the original source buffer.
    pub byte: usize,

    /// One-based line number.
    pub line: usize,

    /// One-based column number within [`Self::line`].
    pub column: usize,
}

impl SourcePosition {
    fn new(byte: usize, line: usize, column: usize) -> Self {
        Self { byte, line, column }
    }
}

/// Syntax error returned when Brainfuck brackets do not match.
///
/// Brainfuck has only one syntactic structure: matching `[` and `]` loops. All
/// other bytes outside the eight command characters are comments and cannot
/// produce syntax errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntaxError {
    /// A `[` was opened but no matching `]` was found.
    UnmatchedOpen(SourcePosition),

    /// A `]` appeared without a matching earlier `[`.
    UnmatchedClose(SourcePosition),
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnmatchedOpen(position) => write!(
                f,
                "unmatched '[' at line {}, column {}",
                position.line, position.column
            ),
            Self::UnmatchedClose(position) => write!(
                f,
                "unmatched ']' at line {}, column {}",
                position.line, position.column
            ),
        }
    }
}

impl std::error::Error for SyntaxError {}

#[derive(Debug)]
struct Frame {
    opening: Option<SourcePosition>,
    ops: Vec<Op>,
}

/// Parse Brainfuck source into normalized operations.
///
/// The parser accepts arbitrary bytes. Only the eight Brainfuck commands
/// `+`, `-`, `<`, `>`, `.`, `,`, `[`, and `]` are meaningful; every other byte is
/// ignored. This makes comments and formatting free at the language level.
///
/// Returned operations are locally optimized, so the output is not necessarily
/// one operation per source byte. For example, `+++` becomes one [`Op::Add`],
/// `><` disappears, and `[-]` becomes [`Op::Clear`].
///
/// # Errors
///
/// Returns [`SyntaxError::UnmatchedOpen`] or [`SyntaxError::UnmatchedClose`]
/// when brackets do not match.
///
/// # Examples
///
/// ```
/// use hypothalamus::bf::{parse, Op};
///
/// let ops = parse(b"++ ignored -- >><< .").expect("valid program");
/// assert_eq!(ops, vec![Op::Output]);
/// ```
pub fn parse(source: &[u8]) -> Result<Vec<Op>, SyntaxError> {
    let mut stack = vec![Frame {
        opening: None,
        ops: Vec::new(),
    }];
    let mut line = 1;
    let mut column = 1;

    for (byte_index, byte) in source.iter().copied().enumerate() {
        let position = SourcePosition::new(byte_index, line, column);

        match byte {
            b'+' => push_op(&mut stack.last_mut().expect("root frame").ops, Op::Add(1)),
            b'-' => push_op(&mut stack.last_mut().expect("root frame").ops, Op::Add(-1)),
            b'>' => push_op(&mut stack.last_mut().expect("root frame").ops, Op::Move(1)),
            b'<' => push_op(&mut stack.last_mut().expect("root frame").ops, Op::Move(-1)),
            b'.' => stack.last_mut().expect("root frame").ops.push(Op::Output),
            b',' => stack.last_mut().expect("root frame").ops.push(Op::Input),
            b'[' => stack.push(Frame {
                opening: Some(position),
                ops: Vec::new(),
            }),
            b']' => {
                if stack.len() == 1 {
                    return Err(SyntaxError::UnmatchedClose(position));
                }

                let frame = stack.pop().expect("loop frame");
                let loop_op = build_loop(frame.ops);
                push_op(&mut stack.last_mut().expect("parent frame").ops, loop_op);
            }
            _ => {}
        }

        if byte == b'\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }

    if stack.len() != 1 {
        let frame = stack.pop().expect("unmatched loop frame");
        return Err(SyntaxError::UnmatchedOpen(
            frame.opening.expect("loop opening position"),
        ));
    }

    Ok(stack.pop().expect("root frame").ops)
}

fn build_loop(ops: Vec<Op>) -> Op {
    let optimized = optimize(ops);
    if is_clear_loop(&optimized) {
        Op::Clear
    } else {
        Op::Loop(optimized)
    }
}

fn optimize(ops: Vec<Op>) -> Vec<Op> {
    let mut output = Vec::new();

    for op in ops {
        let op = match op {
            Op::Loop(body) => build_loop(body),
            other => other,
        };
        push_op(&mut output, op);
    }

    output
}

fn push_op(output: &mut Vec<Op>, op: Op) {
    match op {
        Op::Add(delta) => {
            let delta = delta.rem_euclid(256);
            if delta == 0 {
                return;
            }

            if let Some(Op::Add(previous)) = output.last_mut() {
                *previous = (*previous + delta).rem_euclid(256);
                if *previous == 0 {
                    output.pop();
                }
                return;
            }

            output.push(Op::Add(delta));
        }
        Op::Move(delta) => {
            if delta == 0 {
                return;
            }

            if let Some(Op::Move(previous)) = output.last_mut() {
                *previous += delta;
                if *previous == 0 {
                    output.pop();
                }
                return;
            }

            output.push(Op::Move(delta));
        }
        Op::Clear => {
            if matches!(output.last(), Some(Op::Clear)) {
                return;
            }

            if matches!(output.last(), Some(Op::Add(_))) {
                output.pop();
            }

            output.push(Op::Clear);
        }
        other => output.push(other),
    }
}

fn is_clear_loop(ops: &[Op]) -> bool {
    matches!(ops, [Op::Add(1)] | [Op::Add(255)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_non_commands_and_combines_runs() {
        let ops = parse(b"++ ignored -- >><< .").expect("valid program");
        assert_eq!(ops, vec![Op::Output]);
    }

    #[test]
    fn builds_nested_loops() {
        let ops = parse(b"+[>+[<-]>]").expect("valid program");
        assert_eq!(
            ops,
            vec![
                Op::Add(1),
                Op::Loop(vec![
                    Op::Move(1),
                    Op::Add(1),
                    Op::Loop(vec![Op::Move(-1), Op::Add(255)]),
                    Op::Move(1)
                ])
            ]
        );
    }

    #[test]
    fn recognizes_clear_loops() {
        assert_eq!(parse(b"[-][+]").expect("valid program"), vec![Op::Clear]);
    }

    #[test]
    fn reports_unmatched_close() {
        let err = parse(b"]").expect_err("program should be invalid");
        assert_eq!(
            err,
            SyntaxError::UnmatchedClose(SourcePosition {
                byte: 0,
                line: 1,
                column: 1
            })
        );
    }

    #[test]
    fn reports_unmatched_open() {
        let err = parse(b"\n[").expect_err("program should be invalid");
        assert_eq!(
            err,
            SyntaxError::UnmatchedOpen(SourcePosition {
                byte: 1,
                line: 2,
                column: 1
            })
        );
    }
}
