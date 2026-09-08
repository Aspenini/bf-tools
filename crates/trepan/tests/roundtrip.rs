//! Round-trip tests: lift Brainfuck, compile the Cranium that comes out, and
//! check it does what the Brainfuck did.
//!
//! This is the property that matters. Anything else — how the statements are
//! spelled, which cells got which names — is presentation.

use cranium::compile_str;
use lobe::{CellSize, create_runtime_with_tape};
use trepan::LiftError;

const HELLO: &str = include_str!("../../hypothalamus/examples/hello.bf");
const INTERPRETER: &str = include_str!("../../hypothalamus/examples/interpreter.bf");
const SIERPINSKI: &str = include_str!("../../lobe/bf/sierpinski.bf");
const GOLDEN: &str = include_str!("../../lobe/bf/golden.bf");

/// Run Brainfuck and collect what it wrote.
fn run_bf(code: &str, input: &[u8]) -> Vec<u8> {
    let mut runtime =
        create_runtime_with_tape(code, CellSize::Bits8, 30_000).expect("valid brainfuck");
    let mut input = std::io::Cursor::new(input.to_vec());
    let mut output = Vec::new();
    runtime
        .run_with_io(&mut input, &mut output)
        .expect("program runs");
    output
}

/// Lift Brainfuck, compile the Cranium, and run that instead.
fn run_lifted(source: &str, input: &[u8]) -> Vec<u8> {
    let lifted = trepan::lift(source.as_bytes(), "test").expect("lifts");
    let compiled = match compile_str(&lifted.source) {
        Ok(compiled) => compiled,
        Err(error) => panic!(
            "lifted Cranium did not compile: {error}\n\n{}",
            lifted.source
        ),
    };
    let mut runtime = create_runtime_with_tape(
        &compiled.code,
        CellSize::Bits8,
        compiled.cells_used.max(30_000),
    )
    .expect("cranium emits valid brainfuck");
    let mut input = std::io::Cursor::new(input.to_vec());
    let mut output = Vec::new();
    runtime
        .run_with_io(&mut input, &mut output)
        .expect("program runs");
    output
}

#[test]
fn hello_world_survives_the_round_trip() {
    let original = run_bf(HELLO, b"");

    assert_eq!(original, b"Hello World!\n");
    assert_eq!(run_lifted(HELLO, b""), original);
}

#[test]
fn hello_world_lifts_to_readable_arithmetic() {
    let lifted = trepan::lift(HELLO.as_bytes(), "hello.bf").expect("lifts");

    assert_eq!(lifted.cells, 5);
    // The setup loop is a multiply-transfer, and reads as one.
    assert!(lifted.source.contains("c1 += c0 * 7;"), "{}", lifted.source);
    assert!(lifted.source.contains("putc(c1);"), "{}", lifted.source);
    // Nothing needed the tape itself.
    assert!(!lifted.source.contains('['), "{}", lifted.source);
}

#[test]
fn a_brainfuck_interpreter_survives_the_round_trip() {
    // `,+.` with `A` for input, so the interpreted program echoes `B`.
    let program = b",+.!A";
    let original = run_bf(INTERPRETER, program);

    assert_eq!(original, b"B");
    assert_eq!(run_lifted(INTERPRETER, program), original);
}

#[test]
fn the_interpreter_lifts_without_a_tape_array() {
    let lifted = trepan::lift(INTERPRETER.as_bytes(), "interpreter.bf").expect("lifts");

    assert_eq!(lifted.cells, 104);
    assert!(lifted.statements > 3_000, "{}", lifted.statements);
    assert!(!lifted.source.contains('['), "no array indexing");
}

#[test]
fn programs_that_scan_are_declined() {
    // Both of these walk the tape looking for a zero, so where they stop is
    // not something the lifter can know.
    assert!(matches!(
        trepan::lift(SIERPINSKI.as_bytes(), "sierpinski.bf"),
        Err(LiftError::Unbalanced(_) | LiftError::Scan(_))
    ));
    assert!(matches!(
        trepan::lift(GOLDEN.as_bytes(), "golden.bf"),
        Err(LiftError::Unbalanced(_) | LiftError::Scan(_))
    ));
}

#[test]
fn declining_says_why() {
    let error = trepan::lift(SIERPINSKI.as_bytes(), "sierpinski.bf").unwrap_err();

    let message = error.to_string();
    assert!(message.contains("pointer"), "{message}");
    assert!(message.contains("cannot become variables"), "{message}");
}
