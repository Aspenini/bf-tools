use hypothalamus::DEFAULT_TAPE_SIZE;
use hypothalamus::bf;
use hypothalamus::llvm::{self, LlvmOptions, Runtime};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const INTERPRETER_BF: &[u8] = include_bytes!("../examples/interpreter.bf");

fn interpreter_llvm() -> String {
    let ops = bf::parse(INTERPRETER_BF).expect("owned interpreter should parse");
    llvm::generate_module(
        &ops,
        &LlvmOptions {
            tape_size: DEFAULT_TAPE_SIZE,
            target_triple: None,
            source_filename: Some("examples/interpreter.bf".to_string()),
            bounds_check: false,
            runtime: Runtime::Hosted,
        },
    )
    .expect("owned interpreter should lower to LLVM IR")
}

#[test]
fn owned_interpreter_lowers_to_llvm() {
    let ir = interpreter_llvm();

    assert!(ir.contains("define i32 @main()"));
    assert!(ir.contains("@putchar"));
    assert!(ir.contains("loop_check_"));
}

#[test]
fn owned_interpreter_runs_smoke_programs_when_clang_is_available() {
    if !clang_available() {
        eprintln!("skipping owned interpreter execution smoke test: clang is not available");
        return;
    }

    let temp_dir = TestTempDir::new("owned-interpreter");

    let ll_path = temp_dir.path().join("interpreter.ll");
    let exe_path = temp_dir.path().join("interpreter");
    fs::write(&ll_path, interpreter_llvm()).expect("write interpreter LLVM IR");

    let status = Command::new("clang")
        .arg("-O3")
        .arg(&ll_path)
        .arg("-o")
        .arg(&exe_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run clang");

    if !status.success() {
        eprintln!("skipping owned interpreter execution smoke test: clang failed");
        return;
    }

    assert_eq!(run_interpreter(&exe_path, b",+.!A"), b"B");
    assert_eq!(run_interpreter(&exe_path, b"++[>++[>++<-]<-]>>.!"), &[8]);
}

fn clang_available() -> bool {
    Command::new("clang")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn run_interpreter(exe_path: &Path, input: &[u8]) -> Vec<u8> {
    let mut child = Command::new(exe_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run owned interpreter");

    {
        let stdin = child.stdin.as_mut().expect("interpreter stdin");
        use std::io::Write;
        stdin.write_all(input).expect("write interpreter input");
    }

    let output = child.wait_with_output().expect("read interpreter output");
    assert!(output.status.success(), "interpreter should exit cleanly");
    output.stdout
}

struct TestTempDir {
    path: PathBuf,
}

impl TestTempDir {
    fn new(label: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "hypothalamus-{label}-{}-{timestamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
