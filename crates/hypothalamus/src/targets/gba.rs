//! Game Boy Advance ROM image builder.

use crate::driver::{CompilerConfig, DriverError};
use crate::target::RuntimeAbi;
use crate::tool;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Size of the GBA cartridge header.
pub const HEADER_SIZE: usize = 0xC0;

/// Default GBA ROM title written into the header.
pub const ROM_TITLE: &[u8; 12] = b"HYPOTHALAM  ";

/// Default GBA game code written into the header.
pub const GAME_CODE: &[u8; 4] = b"HYBF";

/// Default GBA maker code written into the header.
pub const MAKER_CODE: &[u8; 2] = b"00";

const DEVKITARM_BIN: &str = "/opt/devkitpro/devkitARM/bin";
const GBA_LLVM_TOOL_HINT: &str = "Install an LLVM toolchain with clang and ld.lld, put bundled LLVM tools beside the configured clang, or install devkitARM for GCC fallback.";
const STARTUP_ASM: &str = include_str!("gba/startup.S");
const LINKER_SCRIPT: &str = include_str!("gba/gba.ld");
const RUNTIME_C: &str = include_str!("gba/runtime.c");
const ROM_ORIGIN: u32 = 0x0800_0000;
const MAX_ROM_SIZE: usize = 32 * 1024 * 1024;
const PT_LOAD: u32 = 1;

const NINTENDO_LOGO: [u8; 156] = [
    0x24, 0xFF, 0xAE, 0x51, 0x69, 0x9A, 0xA2, 0x21, 0x3D, 0x84, 0x82, 0x0A, 0x84, 0xE4, 0x09, 0xAD,
    0x11, 0x24, 0x8B, 0x98, 0xC0, 0x81, 0x7F, 0x21, 0xA3, 0x52, 0xBE, 0x19, 0x93, 0x09, 0xCE, 0x20,
    0x10, 0x46, 0x4A, 0x4A, 0xF8, 0x27, 0x31, 0xEC, 0x58, 0xC7, 0xE8, 0x33, 0x82, 0xE3, 0xCE, 0xBF,
    0x85, 0xF4, 0xDF, 0x94, 0xCE, 0x4B, 0x09, 0xC1, 0x94, 0x56, 0x8A, 0xC0, 0x13, 0x72, 0xA7, 0xFC,
    0x9F, 0x84, 0x4D, 0x73, 0xA3, 0xCA, 0x9A, 0x61, 0x58, 0x97, 0xA3, 0x27, 0xFC, 0x03, 0x98, 0x76,
    0x23, 0x1D, 0xC7, 0x61, 0x03, 0x04, 0xAE, 0x56, 0xBF, 0x38, 0x84, 0x00, 0x40, 0xA7, 0x0E, 0xFD,
    0xFF, 0x52, 0xFE, 0x03, 0x6F, 0x95, 0x30, 0xF1, 0x97, 0xFB, 0xC0, 0x85, 0x60, 0xD6, 0x80, 0x25,
    0xA9, 0x63, 0xBE, 0x03, 0x01, 0x4E, 0x38, 0xE2, 0xF9, 0xA2, 0x34, 0xFF, 0xBB, 0x3E, 0x03, 0x44,
    0x78, 0x00, 0x90, 0xCB, 0x88, 0x11, 0x3A, 0x94, 0x65, 0xC0, 0x7C, 0x63, 0x87, 0xF0, 0x3C, 0xAF,
    0xD6, 0x25, 0xE4, 0x8B, 0x38, 0x0A, 0xAC, 0x72, 0x21, 0xD4, 0xF8, 0x07,
];

/// Build a complete `.gba` ROM image from generated LLVM IR.
pub fn build_image(
    config: &CompilerConfig,
    module: &str,
    output: &Path,
) -> Result<(), DriverError> {
    let temp_dir = temporary_dir();
    fs::create_dir_all(&temp_dir).map_err(|source| DriverError::WriteFile {
        path: temp_dir.clone(),
        source,
    })?;

    let result = build_image_in_dir(config, module, output, &temp_dir);
    let _ = fs::remove_dir_all(&temp_dir);
    result
}

/// Patch the 192-byte GBA ROM header at the start of `rom`.
pub fn patch_header(rom: &mut Vec<u8>) {
    if rom.len() < HEADER_SIZE {
        rom.resize(HEADER_SIZE, 0);
    }

    rom[..HEADER_SIZE].fill(0);
    rom[0..4].copy_from_slice(&0xEA00002E_u32.to_le_bytes());
    rom[0x04..0xA0].copy_from_slice(&NINTENDO_LOGO);
    rom[0xA0..0xAC].copy_from_slice(ROM_TITLE);
    rom[0xAC..0xB0].copy_from_slice(GAME_CODE);
    rom[0xB0..0xB2].copy_from_slice(MAKER_CODE);
    rom[0xB2] = 0x96;
    rom[0xBD] = header_checksum(rom);
}

/// Return true when `rom` has the header fields Hypothalamus writes.
pub fn has_valid_header(rom: &[u8]) -> bool {
    rom.len() >= HEADER_SIZE
        && rom[0..4] == 0xEA00002E_u32.to_le_bytes()
        && rom[0x04..0xA0] == NINTENDO_LOGO
        && rom[0xA0..0xAC] == ROM_TITLE[..]
        && rom[0xAC..0xB0] == GAME_CODE[..]
        && rom[0xB0..0xB2] == MAKER_CODE[..]
        && rom[0xB2] == 0x96
        && rom[0xBD] == header_checksum(rom)
}

/// Compute the GBA header complement checksum byte.
pub fn header_checksum(rom: &[u8]) -> u8 {
    let sum = rom[0xA0..=0xBC]
        .iter()
        .fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
    0_u8.wrapping_sub(sum).wrapping_sub(0x19)
}

/// Locate a GBA tool using an explicit override, `PATH`, then devkitPro.
pub fn find_gba_tool(override_path: Option<&Path>, name: &'static str) -> Option<PathBuf> {
    tool::find_tool(override_path, name, Some(Path::new(DEVKITARM_BIN)))
}

fn build_image_in_dir(
    config: &CompilerConfig,
    module: &str,
    output: &Path,
    temp_dir: &Path,
) -> Result<(), DriverError> {
    let ll_path = if config.keep_ll {
        output.with_extension("ll")
    } else {
        temp_dir.join("program.ll")
    };
    write_file(&ll_path, module.as_bytes())?;

    let bf_object = temp_dir.join("program.o");
    compile_bf_object(config, &ll_path, &bf_object)?;

    let startup_source = temp_dir.join("gba_startup.S");
    let runtime_source = temp_dir.join("gba_runtime.c");
    let linker_script = temp_dir.join("gba.ld");
    write_file(&startup_source, STARTUP_ASM.as_bytes())?;
    write_file(&runtime_source, runtime_c_source(config).as_bytes())?;
    write_file(&linker_script, LINKER_SCRIPT.as_bytes())?;

    let startup_object = temp_dir.join("gba_startup.o");
    let runtime_object = temp_dir.join("gba_runtime.o");
    let elf_path = temp_dir.join("program.elf");

    match build_image_with_llvm(
        config,
        &startup_source,
        &runtime_source,
        &linker_script,
        &startup_object,
        &runtime_object,
        &bf_object,
        &elf_path,
        output,
    ) {
        Ok(()) => Ok(()),
        Err(llvm_error) => {
            let Some(gcc) = find_gba_tool(config.gba_gcc.as_deref(), "arm-none-eabi-gcc") else {
                return Err(llvm_error);
            };
            build_image_with_gcc(
                &gcc,
                &startup_source,
                &runtime_source,
                &linker_script,
                &startup_object,
                &runtime_object,
                &bf_object,
                &elf_path,
                output,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_image_with_llvm(
    config: &CompilerConfig,
    startup_source: &Path,
    runtime_source: &Path,
    linker_script: &Path,
    startup_object: &Path,
    runtime_object: &Path,
    bf_object: &Path,
    elf_path: &Path,
    output: &Path,
) -> Result<(), DriverError> {
    let lld = find_lld(config).ok_or(DriverError::ToolNotFound {
        tool: "ld.lld",
        hint: GBA_LLVM_TOOL_HINT,
    })?;

    compile_startup_with_clang(config, startup_source, startup_object)?;
    compile_runtime_with_clang(config, runtime_source, runtime_object)?;
    link_elf_with_lld(
        &lld,
        linker_script,
        startup_object,
        runtime_object,
        bf_object,
        elf_path,
    )?;
    write_rom_from_elf(elf_path, output)
}

#[allow(clippy::too_many_arguments)]
fn build_image_with_gcc(
    gcc: &Path,
    startup_source: &Path,
    runtime_source: &Path,
    linker_script: &Path,
    startup_object: &Path,
    runtime_object: &Path,
    bf_object: &Path,
    elf_path: &Path,
    output: &Path,
) -> Result<(), DriverError> {
    compile_startup_with_gcc(gcc, startup_source, startup_object)?;
    compile_runtime_with_gcc(gcc, runtime_source, runtime_object)?;
    link_elf_with_gcc(
        gcc,
        linker_script,
        startup_object,
        runtime_object,
        bf_object,
        elf_path,
    )?;
    write_rom_from_elf(elf_path, output)
}

fn find_lld(config: &CompilerConfig) -> Option<PathBuf> {
    tool::find_on_path("ld.lld")
        .or_else(|| tool::find_sibling_tool(&config.clang, &["ld.lld", "lld"]))
}

fn compile_bf_object(
    config: &CompilerConfig,
    ll_path: &Path,
    output: &Path,
) -> Result<(), DriverError> {
    let mut command = Command::new(&config.clang);
    command.arg("-Wno-override-module");
    command.arg(config.opt_level.clang_arg());
    command.arg("-ffreestanding");
    command.arg("-fno-builtin");
    command.arg("-fno-unwind-tables");
    command.arg("-fno-asynchronous-unwind-tables");
    if let Some(target_triple) = config.target.llvm_triple() {
        command.arg(format!("--target={target_triple}"));
    }
    command.args(config.target.clang_args());
    command.arg("-c");
    command.arg(ll_path);
    command.arg("-o");
    command.arg(output);
    run_command(command, &config.clang)
}

fn compile_startup_with_clang(
    config: &CompilerConfig,
    source: &Path,
    output: &Path,
) -> Result<(), DriverError> {
    let mut command = Command::new(&config.clang);
    command.args([
        "--target=thumbv4t-none-eabi",
        "-mcpu=arm7tdmi",
        "-marm",
        "-x",
        "assembler-with-cpp",
        "-c",
    ]);
    command.arg(source);
    command.arg("-o");
    command.arg(output);
    run_command(command, &config.clang)
}

fn compile_runtime_with_clang(
    config: &CompilerConfig,
    source: &Path,
    output: &Path,
) -> Result<(), DriverError> {
    let mut command = Command::new(&config.clang);
    command.args([
        "--target=thumbv4t-none-eabi",
        "-mcpu=arm7tdmi",
        "-mthumb",
        "-ffreestanding",
        "-fno-builtin",
        "-fno-common",
        "-fno-jump-tables",
        "-fno-unwind-tables",
        "-fno-asynchronous-unwind-tables",
        "-Os",
        "-std=c99",
        "-c",
    ]);
    command.arg(source);
    command.arg("-o");
    command.arg(output);
    run_command(command, &config.clang)
}

fn compile_startup_with_gcc(gcc: &Path, source: &Path, output: &Path) -> Result<(), DriverError> {
    let mut command = Command::new(gcc);
    command.args([
        "-mcpu=arm7tdmi",
        "-marm",
        "-mthumb-interwork",
        "-x",
        "assembler-with-cpp",
        "-c",
    ]);
    command.arg(source);
    command.arg("-o");
    command.arg(output);
    run_command(command, &gcc.display().to_string())
}

fn compile_runtime_with_gcc(gcc: &Path, source: &Path, output: &Path) -> Result<(), DriverError> {
    let mut command = Command::new(gcc);
    command.args([
        "-mcpu=arm7tdmi",
        "-mthumb",
        "-mthumb-interwork",
        "-ffreestanding",
        "-fno-builtin",
        "-fno-common",
        "-fno-jump-tables",
        "-fno-unwind-tables",
        "-fno-asynchronous-unwind-tables",
        "-Os",
        "-std=c99",
        "-c",
    ]);
    command.arg(source);
    command.arg("-o");
    command.arg(output);
    run_command(command, &gcc.display().to_string())
}

fn link_elf_with_lld(
    lld: &Path,
    linker_script: &Path,
    startup_object: &Path,
    runtime_object: &Path,
    bf_object: &Path,
    output: &Path,
) -> Result<(), DriverError> {
    let mut command = Command::new(lld);
    command.args(["-m", "armelf", "-T"]);
    command.arg(linker_script);
    // No `--no-warn-execstack`: LLD 22 dropped it, and the executable-stack
    // warning it silenced only ever reached output that a successful link
    // throws away.
    command.args(["--gc-sections"]);
    command.arg(startup_object);
    command.arg(runtime_object);
    command.arg(bf_object);
    command.arg("-o");
    command.arg(output);
    run_command(command, &lld.display().to_string())
}

fn link_elf_with_gcc(
    gcc: &Path,
    linker_script: &Path,
    startup_object: &Path,
    runtime_object: &Path,
    bf_object: &Path,
    output: &Path,
) -> Result<(), DriverError> {
    let mut command = Command::new(gcc);
    command.args([
        "-mcpu=arm7tdmi",
        "-mthumb",
        "-mthumb-interwork",
        "-nostdlib",
        "-Wl,--gc-sections",
    ]);
    command.arg(format!("-Wl,-T,{}", linker_script.display()));
    command.arg(startup_object);
    command.arg(runtime_object);
    command.arg(bf_object);
    command.arg("-o");
    command.arg(output);
    run_command(command, &gcc.display().to_string())
}

fn write_rom_from_elf(elf_path: &Path, output: &Path) -> Result<(), DriverError> {
    let elf = fs::read(elf_path).map_err(|source| DriverError::ReadSource {
        path: elf_path.to_path_buf(),
        source,
    })?;
    let mut rom = extract_rom_from_elf(&elf).map_err(|message| DriverError::InvalidImage {
        format: "GBA",
        message,
    })?;
    patch_header(&mut rom);
    write_file(output, &rom)
}

fn extract_rom_from_elf(elf: &[u8]) -> Result<Vec<u8>, String> {
    if elf.len() < 52 {
        return Err("ELF file is shorter than the ELF32 header".to_string());
    }
    if elf.get(0..4) != Some(b"\x7FELF") {
        return Err("file is not an ELF object".to_string());
    }
    if elf[4] != 1 {
        return Err("expected an ELF32 file".to_string());
    }
    if elf[5] != 1 {
        return Err("expected a little-endian ELF file".to_string());
    }

    let phoff = read_u32(elf, 28)? as usize;
    let phentsize = read_u16(elf, 42)? as usize;
    let phnum = read_u16(elf, 44)? as usize;
    if phentsize < 32 {
        return Err("ELF program headers are too small".to_string());
    }

    let mut rom = Vec::new();
    let mut copied_segment = false;
    for index in 0..phnum {
        let header_offset = phoff
            .checked_add(
                index
                    .checked_mul(phentsize)
                    .ok_or("ELF program header overflow")?,
            )
            .ok_or("ELF program header overflow")?;
        let header_end = header_offset
            .checked_add(32)
            .ok_or("ELF program header overflow")?;
        if header_end > elf.len() {
            return Err("ELF program header extends past end of file".to_string());
        }

        let p_type = read_u32(elf, header_offset)?;
        if p_type != PT_LOAD {
            continue;
        }

        let p_offset = read_u32(elf, header_offset + 4)? as usize;
        let p_vaddr = read_u32(elf, header_offset + 8)?;
        let p_paddr = read_u32(elf, header_offset + 12)?;
        let p_filesz = read_u32(elf, header_offset + 16)? as usize;
        if p_filesz == 0 {
            continue;
        }

        let load_addr = if p_paddr >= ROM_ORIGIN {
            p_paddr
        } else {
            p_vaddr
        };
        if load_addr < ROM_ORIGIN {
            continue;
        }

        let rom_offset = (load_addr - ROM_ORIGIN) as usize;
        let file_end = p_offset
            .checked_add(p_filesz)
            .ok_or("ELF segment file range overflow")?;
        if file_end > elf.len() {
            return Err("ELF load segment extends past end of file".to_string());
        }

        let rom_end = rom_offset
            .checked_add(p_filesz)
            .ok_or("GBA ROM segment range overflow")?;
        if rom_end > MAX_ROM_SIZE {
            return Err("GBA ROM exceeds 32 MiB".to_string());
        }

        if rom.len() < rom_end {
            rom.resize(rom_end, 0);
        }
        rom[rom_offset..rom_end].copy_from_slice(&elf[p_offset..file_end]);
        copied_segment = true;
    }

    if copied_segment {
        Ok(rom)
    } else {
        Err("ELF did not contain loadable GBA ROM segments".to_string())
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or("ELF read extends past end of file")?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or("ELF read extends past end of file")?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn run_command(command: Command, tool_name: &str) -> Result<(), DriverError> {
    if let Some(failure) = tool::run_captured(command).map_err(|source| DriverError::RunTool {
        tool: tool_name.to_string(),
        source,
    })? {
        return Err(DriverError::tool_failed(tool_name, failure));
    }

    Ok(())
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), DriverError> {
    fs::write(path, bytes).map_err(|source| DriverError::WriteFile {
        path: path.to_path_buf(),
        source,
    })
}

fn temporary_dir() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    env::temp_dir().join(format!(
        "hypothalamus-gba-{}-{timestamp}",
        std::process::id()
    ))
}

fn runtime_c_source(config: &CompilerConfig) -> String {
    let (entry_symbol, putchar_symbol, getchar_symbol) = runtime_symbols(config);

    RUNTIME_C
        .replace(
            "__HYPOTHALAMUS_ENTRY_SYMBOL__",
            &escape_c_string(entry_symbol),
        )
        .replace(
            "__HYPOTHALAMUS_PUTCHAR_SYMBOL__",
            &escape_c_string(putchar_symbol),
        )
        .replace(
            "__HYPOTHALAMUS_GETCHAR_SYMBOL__",
            &escape_c_string(getchar_symbol),
        )
}

fn runtime_symbols(config: &CompilerConfig) -> (&str, &str, &str) {
    match config.target.runtime_abi() {
        RuntimeAbi::Freestanding(options) => (
            &options.entry_symbol,
            &options.putchar_symbol,
            &options.getchar_symbol,
        ),
        RuntimeAbi::Hosted => ("bf_main", "bf_putchar", "bf_getchar"),
    }
}

fn escape_c_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_valid_gba_header() {
        let mut rom = vec![0xAA; 512];

        patch_header(&mut rom);

        assert!(has_valid_header(&rom));
        assert_eq!(&rom[0xA0..0xAC], ROM_TITLE);
        assert_eq!(&rom[0xAC..0xB0], GAME_CODE);
        assert_eq!(&rom[0xB0..0xB2], MAKER_CODE);
        assert_eq!(rom[0xB2], 0x96);
    }

    #[test]
    fn patch_extends_short_roms() {
        let mut rom = Vec::new();

        patch_header(&mut rom);

        assert_eq!(rom.len(), HEADER_SIZE);
        assert!(has_valid_header(&rom));
    }

    #[test]
    fn extracts_loadable_rom_segments_from_elf() {
        let mut elf = vec![0; 0x120];
        elf[0..4].copy_from_slice(b"\x7FELF");
        elf[4] = 1;
        elf[5] = 1;
        elf[6] = 1;
        write_u32(&mut elf, 28, 52);
        write_u16(&mut elf, 42, 32);
        write_u16(&mut elf, 44, 2);

        write_program_header(&mut elf, 52, 0x100, ROM_ORIGIN + 4, 3);
        elf[0x100..0x103].copy_from_slice(b"abc");
        write_program_header(&mut elf, 84, 0x110, ROM_ORIGIN + 0x10, 2);
        elf[0x110..0x112].copy_from_slice(b"xy");

        let rom = extract_rom_from_elf(&elf).expect("extract ROM bytes");

        assert_eq!(rom.len(), 0x12);
        assert_eq!(&rom[0..4], &[0, 0, 0, 0]);
        assert_eq!(&rom[4..7], b"abc");
        assert_eq!(&rom[0x10..0x12], b"xy");
    }

    #[test]
    fn runtime_c_uses_configured_freestanding_symbols() {
        let target = crate::target::TargetProfile::resolve("gba").with_runtime_abi(
            RuntimeAbi::Freestanding(crate::llvm::FreestandingOptions {
                entry_symbol: "custom_entry".to_string(),
                putchar_symbol: "custom_putchar".to_string(),
                getchar_symbol: "custom_getchar".to_string(),
            }),
        );
        let config = CompilerConfig::for_target("examples/hello.bf", target);

        let runtime = runtime_c_source(&config);

        assert!(runtime.contains("__asm__(\"custom_entry\")"));
        assert!(runtime.contains("__asm__(\"custom_putchar\")"));
        assert!(runtime.contains("__asm__(\"custom_getchar\")"));
        assert!(runtime.contains("hypothalamus_bf_entry();"));
        assert!(!runtime.contains("bf_main();"));
    }

    #[test]
    fn runtime_c_provides_the_freestanding_memory_helpers() {
        let target = crate::target::TargetProfile::resolve("gba");
        let config = CompilerConfig::for_target("examples/hello.bf", target);

        let runtime = runtime_c_source(&config);

        // Optimized builds lower the tape clear into these, and a ROM links
        // against no libc to find them in.
        for symbol in [
            "memset",
            "memcpy",
            "memmove",
            "__aeabi_memclr",
            "__aeabi_memset",
            "__aeabi_memcpy",
            "__aeabi_memmove",
        ] {
            assert!(runtime.contains(symbol), "runtime is missing {symbol}");
        }
    }

    fn write_program_header(
        elf: &mut [u8],
        offset: usize,
        segment_offset: u32,
        load_addr: u32,
        filesz: u32,
    ) {
        write_u32(elf, offset, PT_LOAD);
        write_u32(elf, offset + 4, segment_offset);
        write_u32(elf, offset + 8, load_addr);
        write_u32(elf, offset + 12, load_addr);
        write_u32(elf, offset + 16, filesz);
        write_u32(elf, offset + 20, filesz);
    }

    fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}
