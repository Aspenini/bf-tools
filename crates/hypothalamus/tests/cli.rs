use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use std::fs;

fn hypothalamus() -> Command {
    Command::cargo_bin("hypothalamus").expect("binary should build")
}

#[test]
fn help_is_generic_about_targets() {
    hypothalamus()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("Hypothalamus - Brainfuck AOT compiler"))
        .stdout(contains("Cranelift backend"))
        .stdout(contains("--target <TARGET>"))
        .stdout(contains("--list-targets"))
        .stdout(contains("tools doctor"))
        .stdout(contains("need no external tools"))
        .stdout(contains("--cc").not())
        .stdout(contains("llvm").not());
}

#[test]
fn version_prints_package_version() {
    hypothalamus()
        .arg("--version")
        .assert()
        .success()
        .stdout(contains(format!(
            "hypothalamus {}",
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn list_targets_includes_current_presets() {
    hypothalamus()
        .arg("--list-targets")
        .assert()
        .success()
        .stdout(contains("native"))
        .stdout(contains("x86_64-none"))
        .stdout(contains("x86_64-unknown-none-elf"));
}

#[test]
fn tools_doctor_reports_local_capabilities() {
    hypothalamus()
        .args(["tools", "doctor", "--linker", "/definitely/missing/cc"])
        .assert()
        .success()
        .stdout(contains("Hypothalamus tool doctor"))
        .stdout(contains("Backend:"))
        .stdout(contains("cranelift: ok"))
        .stdout(contains("jit: ok"))
        .stdout(contains("Tools:"))
        .stdout(contains("linker: missing"))
        .stdout(contains("Targets:"))
        // Code generation does not depend on the missing linker.
        .stdout(contains("codegen: ok"));
}

#[test]
fn run_executes_in_process_without_any_tool() {
    hypothalamus()
        .args(["--run", "examples/hello.bf"])
        .assert()
        .success()
        .stdout(contains("Hello World!"));
}

#[test]
fn run_reads_stdin() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let source = temp_dir.path().join("input.bf");
    fs::write(&source, ",+.,+.").expect("write input program");

    hypothalamus()
        .arg("--run")
        .arg(source)
        .write_stdin("AB")
        .assert()
        .success()
        .stdout("BC");
}

#[test]
fn run_leaves_the_cell_alone_at_end_of_file() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let source = temp_dir.path().join("eof.bf");
    fs::write(&source, "+++++++++++++++++++++++++++++++++,.").expect("write program");

    hypothalamus()
        .arg("--run")
        .arg(source)
        .write_stdin("")
        .assert()
        .success()
        .stdout("!");
}

#[test]
fn run_rejects_an_output_path() {
    hypothalamus()
        .args(["--run", "-o", "hello", "examples/hello.bf"])
        .assert()
        .failure()
        .stderr(contains("--emit jit"));
}

#[test]
fn run_rejects_a_cross_target() {
    hypothalamus()
        .args([
            "--target",
            "aarch64-apple-darwin",
            "--run",
            "examples/hello.bf",
        ])
        .assert()
        .failure()
        .stderr(contains("in this process"));
}

#[test]
fn writes_an_object_file_without_any_tool() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let output = temp_dir.path().join("hello.o");

    hypothalamus()
        .args(["--emit", "obj", "examples/hello.bf", "-o"])
        .arg(&output)
        .assert()
        .success();

    assert!(!fs::read(output).expect("read object").is_empty());
}

#[test]
fn freestanding_targets_default_to_objects() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let output = temp_dir.path().join("hello_bf.o");

    hypothalamus()
        .args(["--target", "x86_64-none", "examples/hello.bf", "-o"])
        .arg(&output)
        .assert()
        .success();

    assert!(!fs::read(output).expect("read object").is_empty());
}

#[test]
fn freestanding_targets_reject_hosted_output() {
    hypothalamus()
        .args(["--target", "x86_64-none", "--emit", "exe", "kernel.bf"])
        .assert()
        .code(2)
        .stderr(contains("freestanding runtime"));
}

#[test]
fn no_output_kind_writes_to_stdout() {
    hypothalamus()
        .args(["--emit", "obj", "-o", "-", "examples/hello.bf"])
        .assert()
        .failure()
        .stderr(contains("--output -"));
}

#[test]
fn reports_targets_cranelift_cannot_reach() {
    for target in [
        "thumbv4t-none-eabi",
        "armv5te-none-eabi",
        "i386-unknown-none",
    ] {
        hypothalamus()
            .args(["--target", target, "--freestanding", "examples/hello.bf"])
            .assert()
            .failure()
            .stderr(contains("no backend"))
            .stderr(contains("x86_64, aarch64, riscv64, and s390x"));
    }
}

#[test]
fn asks_for_an_object_format_when_the_triple_omits_one() {
    hypothalamus()
        .args([
            "--target",
            "x86_64-unknown-none",
            "--freestanding",
            "examples/hello.bf",
        ])
        .assert()
        .failure()
        .stderr(contains("object file format"))
        .stderr(contains("x86_64-unknown-none-elf"));
}

#[test]
fn explains_options_that_belonged_to_the_llvm_backend() {
    for (option, expected) in [
        ("--cc", "--linker"),
        ("--lli", "--run uses the built-in JIT"),
        ("--keep-ll", "--keep-object"),
        ("--gba-gcc", "32-bit ARM"),
    ] {
        hypothalamus()
            .args([option, "clang", "examples/hello.bf"])
            .assert()
            .code(2)
            .stderr(contains("was removed"))
            .stderr(contains(expected));
    }
}

#[test]
fn explains_emit_kinds_that_belonged_to_the_llvm_backend() {
    for kind in ["llvm-ir", "llvm-jit", "asm", "image"] {
        hypothalamus()
            .args(["--emit", kind, "examples/hello.bf"])
            .assert()
            .code(2)
            .stderr(contains("expected exe, obj, or jit"));
    }
}

#[test]
fn bounds_checks_trap_instead_of_walking_off_the_tape() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let source = temp_dir.path().join("underflow.bf");
    fs::write(&source, "+<+.").expect("write underflow program");

    // Without the check the program is simply undefined, so only the checked
    // build is asserted on. A trap is not a clean exit, whatever the platform
    // turns it into.
    hypothalamus()
        .args(["--run", "--bounds-check"])
        .arg(source)
        .assert()
        .failure();
}
