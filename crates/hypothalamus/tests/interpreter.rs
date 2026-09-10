//! The owned Brainfuck interpreter, compiled and then run by the JIT.
//!
//! `examples/interpreter.bf` is 87 KB of Brainfuck, so this is the closest
//! thing the suite has to a real program: it exercises the optimizer, the
//! Cranelift lowering, and byte-exact I/O all at once, and it needs no
//! toolchain to do it.

use assert_cmd::Command;
use hypothalamus::driver::{CompilerConfig, EmitKind, compile_to_object};
use hypothalamus::target::TargetProfile;
use object::{Object, ObjectSymbol};

const INTERPRETER: &str = "examples/interpreter.bf";

fn interpreter_object() -> Vec<u8> {
    let mut config = CompilerConfig::for_target(INTERPRETER, TargetProfile::native());
    config.emit = EmitKind::Object;
    compile_to_object(&config).expect("owned interpreter should compile")
}

#[test]
fn owned_interpreter_compiles_for_the_host() {
    let object = interpreter_object();
    let file = object::File::parse(&*object).expect("object should parse");
    let symbols = file
        .symbols()
        .filter_map(|symbol| symbol.name().ok().map(str::to_string))
        .collect::<Vec<_>>();

    assert!(
        symbols
            .iter()
            .any(|symbol| symbol == "main" || symbol == "_main"),
        "{symbols:?}"
    );
}

#[test]
fn owned_interpreter_runs_smoke_programs_in_the_jit() {
    // The interpreter reads a Brainfuck program, then `!`, then its input.
    assert_eq!(run_interpreter(b",+.!A"), b"B");
    assert_eq!(run_interpreter(b"++[>++[>++<-]<-]>>.!"), &[8]);
}

fn run_interpreter(input: &[u8]) -> Vec<u8> {
    let assert = Command::cargo_bin("hypothalamus")
        .expect("binary should build")
        .args(["--run", INTERPRETER])
        .write_stdin(input.to_vec())
        .assert()
        .success();

    assert.get_output().stdout.clone()
}
