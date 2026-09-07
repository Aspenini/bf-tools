use lobe::{CellSize, create_runtime};
use std::io;

fn run_bytes(src: &str, cell_size: CellSize, input: &[u8]) -> Vec<u8> {
    let mut runtime = create_runtime(src, cell_size).unwrap();
    let mut input = io::Cursor::new(input);
    let mut output = Vec::new();

    runtime.run_with_io(&mut input, &mut output).unwrap();

    output
}

fn run_text(src: &str) -> String {
    String::from_utf8(run_bytes(src, CellSize::Bits8, &[])).unwrap()
}

#[test]
fn test_parser_bracket_matching() {
    assert!(create_runtime("++[>+<-]", CellSize::Bits8).is_ok());
    assert!(create_runtime("++[>+<-", CellSize::Bits8).is_err());
    assert!(create_runtime("++>+<-]", CellSize::Bits8).is_err());
    assert!(create_runtime("++[>+[<-]]", CellSize::Bits8).is_ok());
}

#[test]
fn test_parser_comment_stripping() {
    let output = run_bytes(
        "This is a comment +>++.< and more comment",
        CellSize::Bits8,
        &[],
    );

    assert_eq!(output, vec![2]);
}

#[test]
fn test_hello_world_output() {
    let hello_world = "++++++++[>++++[>++>+++>+++>+<<<<-]>+>+>->>+[<]<-]>>.>---.+++++++..+++.>>.<-.<.+++.------.--------.>>+.>++.";

    assert_eq!(run_text(hello_world), "Hello World!\n");
}

#[test]
fn test_simple_increment_output() {
    assert_eq!(run_bytes("+++++.", CellSize::Bits8, &[]), vec![5]);
}

#[test]
fn test_data_pointer_movement() {
    assert_eq!(run_bytes(">++<+>.", CellSize::Bits8, &[]), vec![2]);
}

#[test]
fn test_empty_program() {
    assert_eq!(run_bytes("", CellSize::Bits8, &[]), Vec::<u8>::new());
}

#[test]
fn test_loop_execution() {
    assert_eq!(run_bytes("+++++[-].", CellSize::Bits8, &[]), vec![0]);
}

#[test]
fn test_nested_loops() {
    assert_eq!(
        run_bytes("++[>++[>++<-]<-]>>.", CellSize::Bits8, &[]),
        vec![8]
    );
}

#[test]
fn test_pointer_wrapping() {
    assert_eq!(run_bytes("<+.", CellSize::Bits8, &[]), vec![1]);
}

#[test]
fn test_input_echo() {
    assert_eq!(run_bytes(",.", CellSize::Bits8, b"A"), b"A");
}

#[test]
fn test_eof_input_sets_cell_to_zero() {
    assert_eq!(run_bytes(",.", CellSize::Bits8, &[]), vec![0]);
}

#[test]
fn test_non_8_bit_cell_wrapping_outputs_number() {
    let output = String::from_utf8(run_bytes("-.", CellSize::Bits16, &[])).unwrap();

    assert_eq!(output, "65535");
}

#[test]
fn test_output_writes_raw_bytes() {
    // Cell 200 must leave the interpreter as one byte, the same as a compiled
    // Brainfuck program's putchar would write, not as two UTF-8 bytes.
    let program = "+".repeat(200) + ".";
    assert_eq!(run_bytes(&program, CellSize::Bits8, &[]), vec![200]);
}

#[test]
fn test_tape_size_is_configurable() {
    use lobe::{DEFAULT_TAPE_SIZE, create_runtime_with_tape};

    let runtime = create_runtime_with_tape("+", CellSize::Bits8, 100_000).unwrap();
    assert_eq!(runtime.tape_size(), 100_000);

    let runtime = create_runtime_with_tape("+", CellSize::Bits8, 0).unwrap();
    assert_eq!(runtime.tape_size(), DEFAULT_TAPE_SIZE);
}

#[test]
fn test_pointer_wraps_at_the_configured_tape_size() {
    // Step left off cell zero, write there, then step right back onto it: with
    // a four-cell tape that lands on the last cell.
    let mut runtime = lobe::create_runtime_with_tape(
        "<+++++++++++++++++++++++++++++++++++++++++++++++++.",
        CellSize::Bits8,
        4,
    )
    .unwrap();
    let mut input = io::Cursor::new(Vec::new());
    let mut output = Vec::new();
    runtime.run_with_io(&mut input, &mut output).unwrap();
    assert_eq!(output, vec![49]);
    assert_eq!(runtime.tape_size(), 4);
}
