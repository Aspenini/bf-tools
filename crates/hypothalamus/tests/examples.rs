use hypothalamus::DEFAULT_TAPE_SIZE;
use hypothalamus::bf;
use hypothalamus::driver::{
    CompilerConfig, DriverError, EmitKind, compile_to_llvm, compile_with_tools,
};
use hypothalamus::llvm::{self, LlvmOptions};
use hypothalamus::target::TargetProfile;
use hypothalamus::targets::gba;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn compile_example(source: &[u8], name: &str) -> String {
    let ops = bf::parse(source).expect("example should parse");
    llvm::generate_module(
        &ops,
        &LlvmOptions {
            tape_size: DEFAULT_TAPE_SIZE,
            target_triple: None,
            source_filename: Some(name.to_string()),
            bounds_check: false,
            runtime: llvm::Runtime::Hosted,
        },
    )
    .expect("example should lower to LLVM IR")
}

#[test]
fn hello_example_lowers_to_llvm() {
    let ir = compile_example(include_bytes!("../examples/hello.bf"), "examples/hello.bf");
    assert!(ir.contains("define i32 @main()"));
}

#[test]
fn scan_loop_fixture_lowers_to_explicit_scan() {
    let ir = compile_example(b"+[>]", "scan-loop.bf");
    assert!(ir.contains("define i32 @main()"));
    assert!(ir.contains("scan_check_"));
}

#[test]
fn multiply_transfer_fixture_lowers_to_straight_line_ir() {
    let ir = compile_example(b"+++++[->+++>++<<]>>.", "multiply-transfer.bf");
    assert!(ir.contains("define i32 @main()"));
    assert!(ir.contains("mul i8"));
    assert!(!ir.contains("loop_check_"));
}

#[test]
fn target_presets_emit_expected_llvm_runtime() {
    let mut config = CompilerConfig::for_target("examples/hello.bf", TargetProfile::resolve("gba"));
    config.emit = EmitKind::LlvmIr;

    let ir = compile_to_llvm(&config).expect("GBA target should lower to LLVM IR");

    assert!(ir.contains("target triple = \"thumbv4t-none-eabi\""));
    assert!(ir.contains("define void @bf_main()"));
    assert!(ir.contains("declare void @bf_putchar(i8)"));
    assert!(!ir.contains("define i32 @main()"));

    let mut config =
        CompilerConfig::for_target("examples/hello.bf", TargetProfile::resolve("nds-arm9"));
    config.emit = EmitKind::LlvmIr;

    let ir = compile_to_llvm(&config).expect("DS ARM9 target should lower to LLVM IR");

    assert!(ir.contains("target triple = \"armv5te-none-eabi\""));
    assert!(ir.contains("define void @bf_main()"));
    assert!(ir.contains("declare void @bf_putchar(i8)"));
    assert!(!ir.contains("define i32 @main()"));
}

#[test]
fn freestanding_targets_emit_objects_when_clang_supports_them() {
    for target_name in ["i386-none", "nds-arm9", "gba"] {
        let target = TargetProfile::resolve(target_name);
        let Some(triple) = target.llvm_triple() else {
            continue;
        };
        if !clang_supports_target(triple, target.clang_args()) {
            eprintln!("skipping {target_name} object smoke test: clang does not support {triple}");
            continue;
        }

        let temp_dir = TestTempDir::new(&format!("target-smoke-{target_name}"));
        let output = temp_dir.path().join("hello.o");

        let mut config = CompilerConfig::for_target("examples/hello.bf", target);
        config.emit = EmitKind::Object;
        config.output = Some(output.clone());

        compile_with_tools(&config).expect("target object smoke should compile");
        assert!(output.exists());
    }
}

#[test]
fn gba_target_builds_rom_when_tools_are_available() {
    let target = TargetProfile::resolve("gba");
    if !clang_supports_target(
        target.llvm_triple().expect("GBA target triple"),
        target.clang_args(),
    ) {
        eprintln!("skipping GBA ROM smoke test: clang does not support GBA target");
        return;
    }

    let temp_dir = TestTempDir::new("gba-rom-smoke");
    let output = temp_dir.path().join("hello.gba");

    let mut config = CompilerConfig::for_target("examples/hello.bf", target);
    config.output = Some(output.clone());
    config.gba_objcopy = Some(temp_dir.path().join("missing-objcopy"));

    match compile_with_tools(&config) {
        Ok(()) => {}
        Err(DriverError::ToolNotFound {
            tool: "ld.lld" | "arm-none-eabi-gcc",
            ..
        }) => {
            eprintln!(
                "skipping GBA ROM smoke test: neither LLVM LLD nor devkitARM GCC is available"
            );
            return;
        }
        Err(error) => panic!("GBA ROM smoke should compile: {error}"),
    }

    let rom = fs::read(output).expect("read generated GBA ROM");
    assert!(gba::has_valid_header(&rom));
    assert!(rom.len() > gba::HEADER_SIZE);
    assert_eq!(&rom[0xA0..0xAC], gba::ROM_TITLE);
    assert_eq!(&rom[0xAC..0xB0], gba::GAME_CODE);
}

#[test]
fn nds_arm9_runtime_example_links_when_tools_are_available() {
    let target = TargetProfile::resolve("nds-arm9");
    if !clang_supports_target(
        target.llvm_triple().expect("DS ARM9 target triple"),
        target.clang_args(),
    ) {
        eprintln!("skipping DS ARM9 runtime link smoke test: clang does not support ARM9 target");
        return;
    }
    if !tool_runs("ld.lld", &["--version"]) {
        eprintln!("skipping DS ARM9 runtime link smoke test: ld.lld is not available");
        return;
    }

    let temp_dir = TestTempDir::new("nds-arm9-runtime-link");
    let bf_object = temp_dir.path().join("hello_arm9.o");
    let startup_object = temp_dir.path().join("nds_start.o");
    let runtime_object = temp_dir.path().join("nds_runtime.o");
    let elf = temp_dir.path().join("hello_arm9.elf");

    let mut config = CompilerConfig::for_target("examples/hello.bf", target);
    config.output = Some(bf_object.clone());
    compile_with_tools(&config).expect("DS ARM9 payload object should compile");

    run_checked(
        Command::new("clang")
            .args([
                "--target=armv5te-none-eabi",
                "-mcpu=arm946e-s",
                "-marm",
                "-x",
                "assembler-with-cpp",
                "-c",
                "examples/runtimes/nds-arm9/start.S",
                "-o",
            ])
            .arg(&startup_object),
        "compile DS ARM9 startup",
    );
    run_checked(
        Command::new("clang")
            .args([
                "--target=armv5te-none-eabi",
                "-mcpu=arm946e-s",
                "-marm",
                "-ffreestanding",
                "-fno-builtin",
                "-fno-unwind-tables",
                "-fno-asynchronous-unwind-tables",
                "-Os",
                "-std=c99",
                "-c",
                "examples/runtimes/nds-arm9/runtime.c",
                "-o",
            ])
            .arg(&runtime_object),
        "compile DS ARM9 runtime",
    );
    run_checked(
        Command::new("ld.lld")
            .args(["-m", "armelf", "-T", "examples/runtimes/nds-arm9/arm9.ld"])
            .arg(&startup_object)
            .arg(&runtime_object)
            .arg(&bf_object)
            .arg("-o")
            .arg(&elf),
        "link DS ARM9 ELF",
    );

    assert!(elf.exists());
    assert!(fs::metadata(elf).expect("read linked ELF metadata").len() > 0);
}

fn clang_supports_target(triple: &str, clang_args: &[String]) -> bool {
    let Some(temp_dir) = TestTempDir::try_new("clang-target-check") else {
        return false;
    };
    let output = temp_dir.path().join("check.o");

    let mut child = match Command::new("clang")
        .arg(format!("--target={triple}"))
        .args(clang_args)
        .arg("-x")
        .arg("c")
        .arg("-c")
        .arg("-")
        .arg("-o")
        .arg(output)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };

    {
        let Some(stdin) = child.stdin.as_mut() else {
            return false;
        };
        use std::io::Write;
        if stdin.write_all(b"void f(void) {}\n").is_err() {
            return false;
        }
    }

    child.wait().map(|status| status.success()).unwrap_or(false)
}

fn tool_runs(tool: &str, args: &[&str]) -> bool {
    Command::new(tool)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn run_checked(command: &mut Command, label: &str) {
    let status = command.status().unwrap_or_else(|error| {
        panic!("{label} failed to launch: {error}");
    });
    assert!(status.success(), "{label} failed with status {status}");
}

struct TestTempDir {
    path: PathBuf,
}

impl TestTempDir {
    fn new(label: &str) -> Self {
        Self::try_new(label).expect("create test temp dir")
    }

    fn try_new(label: &str) -> Option<Self> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "hypothalamus-{label}-{}-{timestamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).ok()?;
        Some(Self { path })
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
