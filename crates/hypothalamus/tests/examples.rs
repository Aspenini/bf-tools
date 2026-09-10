use hypothalamus::codegen::FreestandingOptions;
use hypothalamus::driver::{
    self, CompilerConfig, DriverError, EmitKind, compile, compile_to_object,
};
use hypothalamus::target::{RuntimeAbi, TargetProfile};
use object::{Object, ObjectSymbol};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

/// Compile `input` for `target` and return the object bytes.
fn object_for(input: &str, target: TargetProfile) -> Vec<u8> {
    let mut config = CompilerConfig::for_target(input, target);
    config.emit = EmitKind::Object;
    compile_to_object(&config).expect("program should compile to an object")
}

fn symbols(object: &[u8]) -> Vec<String> {
    object::File::parse(object)
        .expect("generated object should parse")
        .symbols()
        .filter_map(|symbol| symbol.name().ok().map(str::to_string))
        .collect()
}

/// Mach-O prefixes every symbol with an underscore, so match either spelling.
fn has_symbol(symbols: &[String], name: &str) -> bool {
    symbols
        .iter()
        .any(|symbol| symbol == name || symbol.strip_prefix('_') == Some(name))
}

#[test]
fn hello_example_compiles_for_the_host() {
    let object = object_for("examples/hello.bf", TargetProfile::native());
    let symbols = symbols(&object);

    assert!(has_symbol(&symbols, "main"), "{symbols:?}");
    assert!(has_symbol(&symbols, "putchar"), "{symbols:?}");
}

#[test]
fn freestanding_objects_use_the_documented_abi() {
    let object = object_for("examples/hello.bf", TargetProfile::resolve("x86_64-none"));
    let symbols = symbols(&object);

    assert!(
        symbols.iter().any(|symbol| symbol == "bf_main"),
        "{symbols:?}"
    );
    assert!(
        symbols.iter().any(|symbol| symbol == "bf_putchar"),
        "{symbols:?}"
    );
    assert!(
        symbols.iter().any(|symbol| symbol == "bf_getchar"),
        "{symbols:?}"
    );
    // A freestanding payload has no `main` and no libc.
    assert!(
        !symbols.iter().any(|symbol| symbol == "main"),
        "{symbols:?}"
    );
    assert!(
        !symbols.iter().any(|symbol| symbol == "putchar"),
        "{symbols:?}"
    );
}

#[test]
fn freestanding_symbol_names_are_configurable() {
    let target = TargetProfile::resolve("x86_64-none").with_runtime_abi(RuntimeAbi::Freestanding(
        FreestandingOptions {
            entry_symbol: "kernel_bf_main".to_string(),
            putchar_symbol: "serial_write_byte".to_string(),
            getchar_symbol: "serial_read_byte".to_string(),
        },
    ));

    let symbols = symbols(&object_for("examples/hello.bf", target));

    for expected in ["kernel_bf_main", "serial_write_byte", "serial_read_byte"] {
        assert!(
            symbols.iter().any(|symbol| symbol == expected),
            "{symbols:?}"
        );
    }
}

#[test]
fn cross_compiles_without_a_toolchain() {
    // Cranelift is linked in, so a target the host has no toolchain for still
    // produces a complete object file.
    for triple in [
        "x86_64-unknown-none-elf",
        "aarch64-unknown-none-elf",
        "riscv64-unknown-none-elf",
    ] {
        let target = TargetProfile::raw_triple(triple)
            .with_runtime_abi(RuntimeAbi::Freestanding(FreestandingOptions::default()));
        let object = object_for("examples/hello.bf", target);

        assert!(!object.is_empty(), "{triple} produced nothing");
        assert!(
            symbols(&object).iter().any(|symbol| symbol == "bf_main"),
            "{triple} is missing bf_main"
        );
    }
}

#[test]
fn every_optimized_operation_survives_lowering() {
    // Scan, multiply-transfer, clear, a general loop, and both I/O ops.
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let source = temp_dir.path().join("fixtures.bf");
    fs::write(&source, "+[>]+++[->+++<]>[-],.").expect("write fixture program");

    let object = object_for(
        source.to_str().expect("utf-8 path"),
        TargetProfile::native(),
    );

    assert!(!object.is_empty());
}

#[test]
fn rejects_architectures_cranelift_has_no_backend_for() {
    let target = TargetProfile::raw_triple("thumbv4t-none-eabi");
    let mut config = CompilerConfig::for_target("examples/hello.bf", target);
    config.emit = EmitKind::Object;

    let error = compile_to_object(&config).expect_err("32-bit ARM has no backend");

    assert!(matches!(error, DriverError::Isa(_)), "{error}");
    assert!(error.to_string().contains("no backend"));
}

#[test]
fn hello_example_links_and_runs_when_a_linker_is_available() {
    let Some(linker) = driver::find_linker(None) else {
        eprintln!("skipping executable smoke test: no linker on PATH");
        return;
    };

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let output = temp_dir.path().join("hello");

    let mut config = CompilerConfig::new("examples/hello.bf");
    config.output = Some(output.clone());

    if let Err(error) = compile(&config) {
        panic!("linking with {} failed: {error}", linker.display());
    }

    // The driver adds the host's executable extension, so ask it where it put
    // the file rather than guessing.
    let executable = if cfg!(windows) {
        output.with_extension("exe")
    } else {
        output
    };
    assert!(
        executable.exists(),
        "{} was not created",
        executable.display()
    );
    assert_eq!(run(&executable, b""), b"Hello World!\n");
}

#[test]
fn compiled_programs_and_the_jit_agree_on_bytes() {
    let Some(_) = driver::find_linker(None) else {
        eprintln!("skipping byte-agreement smoke test: no linker on PATH");
        return;
    };

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    // Echo two bytes back incremented, then a newline from a fresh cell: enough
    // to catch a platform rewriting the newline or stopping input early.
    let source = temp_dir.path().join("echo.bf");
    fs::write(&source, ",+.,+.>++++++++++.").expect("write echo program");
    let output = temp_dir.path().join("echo");

    let mut config = CompilerConfig::new(&source);
    config.output = Some(output.clone());
    compile(&config).expect("echo program should link");

    let executable = if cfg!(windows) {
        output.with_extension("exe")
    } else {
        output
    };
    assert_eq!(run(&executable, b"AB"), b"BC\n");
}

#[test]
fn keep_object_decides_whether_the_object_survives_the_link() {
    if driver::find_linker(None).is_none() {
        eprintln!("skipping --keep-object smoke test: no linker on PATH");
        return;
    }

    for keep_object in [false, true] {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let output = temp_dir.path().join("hello");

        let mut config = CompilerConfig::new("examples/hello.bf");
        config.output = Some(output.clone());
        config.keep_object = keep_object;
        compile(&config).expect("hello.bf should link");

        // The object lands beside the output, whatever extension the
        // executable ended up with.
        assert_eq!(
            output.with_extension("o").exists(),
            keep_object,
            "keep_object = {keep_object}"
        );
    }
}

#[test]
fn refuses_to_link_an_executable_for_another_host() {
    // Cranelift can emit the object for any target, but the link would run
    // this host's linker against this host's C runtime.
    let elsewhere = if cfg!(windows) {
        "x86_64-unknown-linux-gnu"
    } else {
        "x86_64-pc-windows-msvc"
    };

    let mut config =
        CompilerConfig::for_target("examples/hello.bf", TargetProfile::resolve(elsewhere));
    config.emit = EmitKind::Executable;

    let error = compile(&config).expect_err("a cross-host link should be refused");

    assert!(
        matches!(&error, DriverError::InvalidConfig(message) if message.contains("--emit obj")),
        "{error}"
    );

    // The same target is fine as an object.
    config.emit = EmitKind::Object;
    assert!(
        !compile_to_object(&config)
            .expect("object should compile")
            .is_empty()
    );
}

fn run(executable: &Path, input: &[u8]) -> Vec<u8> {
    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("run {}: {error}", executable.display()));

    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().expect("child stdin");
        stdin.write_all(input).expect("write program input");
    }

    let output = child.wait_with_output().expect("read program output");
    assert!(
        output.status.success(),
        "program exited with {}",
        output.status
    );
    output.stdout
}
