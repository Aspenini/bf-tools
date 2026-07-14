use lobe::{create_runtime, CellSize};
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
