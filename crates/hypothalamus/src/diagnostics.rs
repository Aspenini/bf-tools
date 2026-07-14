//! Toolchain diagnostics for the command-line `tools doctor` command.

use crate::target::{RuntimeAbiKind, TargetImageFormat, TargetPreset, known_targets};
use crate::targets;
use crate::tool;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// Configuration for toolchain diagnostics.
#[derive(Debug, Clone)]
pub struct ToolDoctorConfig {
    /// `clang`-compatible compiler driver to inspect.
    pub clang: String,

    /// LLVM `lli` command to inspect for explicit LLVM JIT execution.
    pub lli: String,

    /// Optional devkitARM GCC override for GBA fallback image links.
    pub gba_gcc: Option<PathBuf>,
}

impl Default for ToolDoctorConfig {
    fn default() -> Self {
        Self {
            clang: "clang".to_string(),
            lli: "lli".to_string(),
            gba_gcc: None,
        }
    }
}

/// Build a human-readable toolchain diagnostics report.
pub fn tools_doctor_report(config: &ToolDoctorConfig) -> String {
    let clang_path = tool::resolve_command_path(&config.clang);
    let lld_path = find_lld(config);
    let lli_path = tool::resolve_command_path(&config.lli);
    let gba_gcc_path = find_gba_gcc(config);

    let mut output = String::new();
    output.push_str("Hypothalamus tool doctor\n\n");
    output.push_str("Execution:\n");
    output.push_str("  run: ok (built-in runner; no external tool required)\n\n");

    output.push_str("Tools:\n");
    push_tool_status(
        &mut output,
        "clang",
        clang_path.as_deref(),
        clang_version(&config.clang).as_deref(),
        "required for native executables, objects, assembly, and target images",
    );
    push_tool_status(
        &mut output,
        "ld.lld",
        lld_path.as_deref(),
        None,
        "required for LLVM-first GBA images and the DS ARM9 link example",
    );
    push_tool_status(
        &mut output,
        "lli",
        lli_path.as_deref(),
        None,
        "only required for --emit llvm-jit",
    );
    push_tool_status(
        &mut output,
        "arm-none-eabi-gcc",
        gba_gcc_path.as_deref(),
        None,
        "optional GBA image fallback",
    );

    output.push_str("\nTargets:\n");
    for target in known_targets() {
        push_target_status(&mut output, target, &config.clang, &lld_path, &gba_gcc_path);
    }

    output
}

fn push_tool_status(
    output: &mut String,
    name: &str,
    path: Option<&Path>,
    version: Option<&str>,
    note: &str,
) {
    if let Some(path) = path {
        output.push_str(&format!("  {name}: ok ({})", path.display()));
        if let Some(version) = version {
            output.push_str(&format!(" - {version}"));
        }
        output.push('\n');
    } else {
        output.push_str(&format!("  {name}: missing ({note})\n"));
    }
}

fn push_target_status(
    output: &mut String,
    target: &TargetPreset,
    clang: &str,
    lld_path: &Option<PathBuf>,
    gba_gcc_path: &Option<PathBuf>,
) {
    let object_status = object_status(target, clang);
    let image_status = image_status(target, &object_status, lld_path, gba_gcc_path);
    let runtime = match target.runtime_abi {
        RuntimeAbiKind::Hosted => "hosted",
        RuntimeAbiKind::Freestanding => "freestanding",
    };

    output.push_str(&format!(
        "  {:12} runtime: {:12} default: {:8} obj: {:28} image: {}",
        target.name,
        runtime,
        target.default_emit.as_str(),
        object_status.label,
        image_status
    ));
    if target.name == "nds-arm9" {
        let link_status = if object_status.available && lld_path.is_some() {
            "link example: ok"
        } else if object_status.available {
            "link example: missing ld.lld"
        } else {
            "link example: missing clang target"
        };
        output.push_str(&format!("; {link_status}"));
    }
    output.push('\n');
}

fn object_status(target: &TargetPreset, clang: &str) -> CapabilityStatus {
    if clang_supports_target(clang, target) {
        CapabilityStatus {
            available: true,
            label: "ok".to_string(),
        }
    } else {
        CapabilityStatus {
            available: false,
            label: "missing clang target".to_string(),
        }
    }
}

fn image_status(
    target: &TargetPreset,
    object_status: &CapabilityStatus,
    lld_path: &Option<PathBuf>,
    gba_gcc_path: &Option<PathBuf>,
) -> String {
    match target.image_format {
        Some(TargetImageFormat::Gba) if object_status.available && lld_path.is_some() => {
            "ok (LLVM-first)".to_string()
        }
        Some(TargetImageFormat::Gba) if object_status.available && gba_gcc_path.is_some() => {
            "ok (devkitARM fallback)".to_string()
        }
        Some(TargetImageFormat::Gba) if object_status.available => {
            "missing ld.lld or arm-none-eabi-gcc".to_string()
        }
        Some(TargetImageFormat::Gba) => "missing clang target".to_string(),
        None => "n/a".to_string(),
    }
}

fn clang_supports_target(clang: &str, target: &TargetPreset) -> bool {
    let Some(temp_dir) = temporary_dir("doctor-clang-target") else {
        return false;
    };
    let output = temp_dir.join("check.o");

    let mut command = Command::new(clang);
    if let Some(triple) = target.llvm_triple {
        command.arg(format!("--target={triple}"));
    }
    command
        .args(target.clang_args)
        .args(["-x", "c", "-c", "-", "-o"])
        .arg(&output)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let Ok(mut child) = command.spawn() else {
        let _ = fs::remove_dir_all(&temp_dir);
        return false;
    };

    let write_ok = child
        .stdin
        .as_mut()
        .map(|stdin| stdin.write_all(b"void f(void) {}\n").is_ok())
        .unwrap_or(false);
    let status_ok = child.wait().map(|status| status.success()).unwrap_or(false);
    let _ = fs::remove_dir_all(&temp_dir);

    write_ok && status_ok
}

fn clang_version(clang: &str) -> Option<String> {
    let output = Command::new(clang).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().next().map(ToString::to_string)
}

fn find_lld(config: &ToolDoctorConfig) -> Option<PathBuf> {
    tool::find_on_path("ld.lld")
        .or_else(|| tool::find_sibling_tool(&config.clang, &["ld.lld", "lld"]))
}

fn find_gba_gcc(config: &ToolDoctorConfig) -> Option<PathBuf> {
    if let Some(path) = &config.gba_gcc {
        return tool::resolve_command_path(&path.to_string_lossy());
    }

    targets::gba::find_gba_tool(None, "arm-none-eabi-gcc")
}

fn temporary_dir(label: &str) -> Option<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let path = env::temp_dir().join(format!(
        "hypothalamus-{label}-{}-{timestamp}",
        std::process::id()
    ));
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

#[derive(Debug)]
struct CapabilityStatus {
    available: bool,
    label: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_includes_core_sections() {
        let report = tools_doctor_report(&ToolDoctorConfig {
            clang: "/definitely/missing/clang".to_string(),
            lli: "/definitely/missing/lli".to_string(),
            gba_gcc: Some(PathBuf::from("/definitely/missing/gcc")),
        });

        assert!(report.contains("Hypothalamus tool doctor"));
        assert!(report.contains("Execution:"));
        assert!(report.contains("Tools:"));
        assert!(report.contains("Targets:"));
        assert!(report.contains("run: ok"));
        assert!(report.contains("nds-arm9"));
        assert!(report.contains("gba"));
    }
}
