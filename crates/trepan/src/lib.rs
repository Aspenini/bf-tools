//! Lift Brainfuck back into structured Cranium.
//!
//! Brainfuck has one anonymous tape and a pointer into it. Cranium has named
//! variables. The gap between the two closes exactly when the pointer's
//! position is known at every point in the program, because then every `+`,
//! `.` and `,` names one specific cell, and that cell can simply be a
//! variable.
//!
//! That holds as long as no loop leaves the pointer somewhere other than where
//! it found it, which rules out `[>]`-style scans and any other unbalanced
//! loop. A program that reaches a computed address genuinely needs the tape,
//! and this lifts none of those: emitting `tape[p]` for every operation would
//! be correct and useless, because Cranium walks to a runtime index one cell at
//! a time and the result would compile back into something far larger than it
//! started as. [`lift`] declines instead, and says which construct stopped it.
//!
//! What comes out is a decompilation, not the original source. Brainfuck keeps
//! no functions, no types, no `if` and no `for` — only cells and `while` — so
//! that is what comes back.

use hypothalamus::bf::{self, SyntaxError};
use hypothalamus::ir::{self, Ir};
use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;

/// Why a program could not be lifted to named variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiftError {
    /// The Brainfuck itself did not parse.
    Syntax(SyntaxError),

    /// A scan loop such as `[>]` walks until it finds a zero, so where it
    /// stops depends on the data.
    Scan(i64),

    /// A loop body left the pointer `delta` cells from where it started, so
    /// the position after the loop depends on how many times it ran.
    Unbalanced(i64),
}

impl fmt::Display for LiftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(error) => write!(f, "{error}"),
            Self::Scan(stride) => write!(
                f,
                "this program scans for a cell (a loop equivalent to `[{}]`), so \
                 where the pointer stops depends on the data and its cells cannot \
                 become variables",
                if *stride < 0 { "<" } else { ">" }
            ),
            Self::Unbalanced(delta) => write!(
                f,
                "a loop here leaves the pointer {} cell{} from where it started, so \
                 the position after it depends on how many times it ran and the \
                 cells after it cannot become variables",
                delta.abs(),
                if delta.abs() == 1 { "" } else { "s" }
            ),
        }
    }
}

impl std::error::Error for LiftError {}

impl From<SyntaxError> for LiftError {
    fn from(error: SyntaxError) -> Self {
        Self::Syntax(error)
    }
}

/// A Brainfuck program rewritten as Cranium.
#[derive(Debug, Clone)]
pub struct Lifted {
    /// The Cranium source.
    pub source: String,

    /// How many tape cells became variables.
    pub cells: usize,

    /// How many statements the body came to.
    pub statements: usize,
}

/// Rewrite `source` as Cranium, naming `label` in the generated header.
///
/// Fails when the program's pointer position is not static; see [`LiftError`].
pub fn lift(source: &[u8], label: &str) -> Result<Lifted, LiftError> {
    let code = ir::optimize(&bf::parse(source)?);
    check_static(&code)?;

    let mut cells = BTreeSet::new();
    Collect {
        cells: &mut cells,
        cursor: 0,
    }
    .block(&code);

    let mut emit = Emit {
        out: String::new(),
        cursor: 0,
        statements: 0,
        depth: 1,
    };
    emit.block(&code);

    let mut source = String::new();
    write_header(&mut source, label, cells.len(), emit.statements);
    for cell in &cells {
        let _ = writeln!(source, "let {} = 0;", name(*cell));
    }
    if !cells.is_empty() {
        source.push('\n');
    }
    source.push_str("fn main() {\n");
    source.push_str(&emit.out);
    source.push_str("}\n");

    Ok(Lifted {
        source,
        cells: cells.len(),
        statements: emit.statements,
    })
}

fn write_header(out: &mut String, label: &str, cells: usize, statements: usize) {
    let _ = writeln!(out, "// Lifted from {label} by trepan.");
    let _ = writeln!(out, "//");
    let _ = writeln!(
        out,
        "// The data pointer was known at every point in this program, so its {cells} tape"
    );
    let _ = writeln!(
        out,
        "// cell{} became variable{} and there is no tape array. Brainfuck's wrapping",
        if cells == 1 { "" } else { "s" },
        if cells == 1 { "" } else { "s" }
    );
    let _ = writeln!(
        out,
        "// byte arithmetic is exactly Cranium's `byte`, so that much carries over"
    );
    let _ = writeln!(
        out,
        "// unchanged. {statements} statements; the names are tape offsets, because"
    );
    let _ = writeln!(out, "// Brainfuck kept none of its own.");
    let _ = writeln!(out, "//");
    let _ = writeln!(
        out,
        "// `,` became `getc()`, which reads 0 at end of input. Brainfuck runtimes"
    );
    let _ = writeln!(
        out,
        "// disagree here — lobe stores 0, hypothalamus leaves the cell alone — so a"
    );
    let _ = writeln!(
        out,
        "// program that reads past the end of its input may not agree with the"
    );
    let _ = writeln!(out, "// original.");
    let _ = writeln!(out);
}

/// The name a cell at `offset` gets. Cells left of the start are `m` for minus,
/// since reaching them is undefined in Brainfuck to begin with.
fn name(offset: i64) -> String {
    if offset < 0 {
        format!("cm{}", -offset)
    } else {
        format!("c{offset}")
    }
}

/// Net pointer movement of a block, or `None` when it is not static.
fn net(block: &[Ir]) -> Option<i64> {
    let mut delta = 0;
    for op in block {
        match op {
            Ir::Move(step) => delta += step,
            Ir::Loop(body) => match net(body) {
                Some(0) => {}
                _ => return None,
            },
            Ir::Scan(_) => return None,
            _ => {}
        }
    }
    Some(delta)
}

/// Reject anything whose pointer position cannot be followed statically.
fn check_static(block: &[Ir]) -> Result<(), LiftError> {
    for op in block {
        match op {
            Ir::Scan(stride) => return Err(LiftError::Scan(*stride)),
            Ir::Loop(body) => {
                check_static(body)?;
                match net(body) {
                    Some(0) => {}
                    Some(delta) => return Err(LiftError::Unbalanced(delta)),
                    // A nested scan or unbalanced loop was reported above.
                    None => unreachable!("check_static already rejected the body"),
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Walks the program to find which cells it actually touches.
struct Collect<'a> {
    cells: &'a mut BTreeSet<i64>,
    cursor: i64,
}

impl Collect<'_> {
    fn block(&mut self, block: &[Ir]) {
        for op in block {
            match op {
                Ir::Add { offset, .. } | Ir::Set { offset, .. } => {
                    self.cells.insert(self.cursor + offset);
                }
                Ir::Input { offset } | Ir::Output { offset } => {
                    self.cells.insert(self.cursor + offset);
                }
                Ir::Move(step) => self.cursor += step,
                Ir::AddMul { terms } => {
                    self.cells.insert(self.cursor);
                    for (offset, _) in terms {
                        self.cells.insert(self.cursor + offset);
                    }
                }
                Ir::Loop(body) => {
                    // The loop tests the cell it starts on.
                    self.cells.insert(self.cursor);
                    let mut inner = Collect {
                        cells: self.cells,
                        cursor: self.cursor,
                    };
                    inner.block(body);
                }
                Ir::Scan(_) => unreachable!("rejected by check_static"),
            }
        }
    }
}

/// Writes the body of `main`.
struct Emit {
    out: String,
    cursor: i64,
    statements: usize,
    depth: usize,
}

impl Emit {
    fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
        self.statements += 1;
    }

    fn block(&mut self, block: &[Ir]) {
        for op in block {
            match op {
                Ir::Add { offset, delta } => {
                    let cell = name(self.cursor + offset);
                    // Deltas arrive as 1..=255; the far half reads better as a
                    // subtraction, and wraps to the same byte either way.
                    let delta = delta.rem_euclid(256);
                    if delta <= 128 {
                        self.line(&format!("{cell} += {delta};"));
                    } else {
                        self.line(&format!("{cell} -= {};", 256 - delta));
                    }
                }
                Ir::Set { offset, value } => {
                    let cell = name(self.cursor + offset);
                    self.line(&format!("{cell} = {value};"));
                }
                Ir::Input { offset } => {
                    let cell = name(self.cursor + offset);
                    self.line(&format!("{cell} = getc();"));
                }
                Ir::Output { offset } => {
                    let cell = name(self.cursor + offset);
                    self.line(&format!("putc({cell});"));
                }
                Ir::Move(step) => self.cursor += step,
                Ir::AddMul { terms } => {
                    let source = name(self.cursor);
                    // The lowering removes offset 0 from the terms, so no
                    // target is the source and the clear can come last.
                    for (offset, factor) in terms {
                        let target = name(self.cursor + offset);
                        let factor = factor.rem_euclid(256);
                        if factor == 1 {
                            self.line(&format!("{target} += {source};"));
                        } else if factor == 255 {
                            self.line(&format!("{target} -= {source};"));
                        } else if factor <= 128 {
                            self.line(&format!("{target} += {source} * {factor};"));
                        } else {
                            self.line(&format!("{target} -= {source} * {};", 256 - factor));
                        }
                    }
                    self.line(&format!("{source} = 0;"));
                }
                Ir::Loop(body) => {
                    let cell = name(self.cursor);
                    self.line(&format!("while {cell} != 0 {{"));
                    let start = self.cursor;
                    self.depth += 1;
                    self.block(body);
                    self.depth -= 1;
                    // Balanced by construction, checked before emitting.
                    self.cursor = start;
                    self.line("}");
                }
                Ir::Scan(_) => unreachable!("rejected by check_static"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(source: &str) -> String {
        let lifted = lift(source.as_bytes(), "test").expect("lifts");
        let start = lifted.source.find("fn main() {\n").expect("has a main");
        lifted.source[start + "fn main() {\n".len()..]
            .strip_suffix("}\n")
            .expect("main is closed")
            .to_string()
    }

    #[test]
    fn adds_become_assignments_to_named_cells() {
        assert_eq!(body("+++>++"), "    c0 += 3;\n    c1 += 2;\n");
    }

    #[test]
    fn a_large_delta_reads_as_a_subtraction() {
        // 255 `+` is one `-` on a wrapping byte.
        let source = "+".repeat(255);
        assert_eq!(body(&source), "    c0 -= 1;\n");
    }

    #[test]
    fn clear_loops_become_stores() {
        // The adds are dead once the cell is cleared, and the IR folds them
        // away before anything reaches the emitter.
        assert_eq!(body("+++[-]"), "    c0 = 0;\n");
    }

    #[test]
    fn a_clear_after_a_read_keeps_the_read() {
        assert_eq!(body(",[-]"), "    c0 = getc();\n    c0 = 0;\n");
    }

    #[test]
    fn transfer_loops_become_addition() {
        assert_eq!(
            body("+++[->+<]"),
            "    c0 += 3;\n    c1 += c0;\n    c0 = 0;\n"
        );
    }

    #[test]
    fn multiply_loops_keep_their_factor() {
        assert_eq!(
            body("++[->+++<]"),
            "    c0 += 2;\n    c1 += c0 * 3;\n    c0 = 0;\n"
        );
    }

    #[test]
    fn io_becomes_getc_and_putc() {
        assert_eq!(body(",."), "    c0 = getc();\n    putc(c0);\n");
    }

    #[test]
    fn loops_nest_and_the_pointer_comes_back() {
        assert_eq!(
            body("+>+[<[->+<]>-]"),
            "    c0 += 1;\n    c1 += 1;\n    while c1 != 0 {\n        c1 += c0;\n        \
             c0 = 0;\n        c1 -= 1;\n    }\n"
        );
    }

    #[test]
    fn declares_every_cell_it_touches() {
        let lifted = lift(b">>+<<+", "test").expect("lifts");
        assert!(lifted.source.contains("let c0 = 0;"), "{}", lifted.source);
        assert!(lifted.source.contains("let c2 = 0;"), "{}", lifted.source);
        // Cell 1 is only passed over, never touched.
        assert!(!lifted.source.contains("let c1 = 0;"), "{}", lifted.source);
        assert_eq!(lifted.cells, 2);
    }

    #[test]
    fn declines_a_scan() {
        assert_eq!(lift(b"+>+[>]", "test").unwrap_err(), LiftError::Scan(1));
    }

    #[test]
    fn declines_an_unbalanced_loop() {
        assert_eq!(
            lift(b"+[>+]", "test").unwrap_err(),
            LiftError::Unbalanced(1)
        );
    }

    #[test]
    fn reports_where_the_brainfuck_does_not_parse() {
        assert!(matches!(lift(b"+[+", "test"), Err(LiftError::Syntax(_))));
    }

    #[test]
    fn an_empty_program_still_lifts() {
        let lifted = lift(b"", "test").expect("lifts");
        assert_eq!(lifted.cells, 0);
        assert!(
            lifted.source.ends_with("fn main() {\n}\n"),
            "{}",
            lifted.source
        );
    }
}
