//! Toolchain diagnostics for the command-line `tools doctor` command.
//!
//! With Cranelift compiled into the binary, most of what this used to probe is
//! now answerable without running anything: whether a target has a backend is a
//! question for [`crate::isa`], not for an external driver. The only thing left
//! to look for on disk is a linker, and only executables need one.

use crate::driver::{self, LINKER_CANDIDATES, OptLevel};
use crate::isa::{self, IsaOptions};
use crate::target::{RuntimeAbiKind, TargetPreset, known_targets};
use std::fmt::Write;
use std::path::Path;

/// Configuration for toolchain diagnostics.
#[derive(Debug, Clone, Default)]
pub struct ToolDoctorConfig {
    /// Linker command to inspect, or `None` to report discovery.
    pub linker: Option<String>,
}

/// Build a human-readable toolchain diagnostics report.
pub fn tools_doctor_report(config: &ToolDoctorConfig) -> String {
    let linker = driver::find_linker(config.linker.as_deref());

    let mut output = String::new();
    output.push_str("Hypothalamus tool doctor\n\n");

    output.push_str("Backend:\n");
    match isa::build(None, probe_options(false)) {
        Ok(host) => {
            let _ = writeln!(output, "  cranelift: ok (host {})", host.triple());
            output.push_str("  jit: ok (in-process; no external tool required)\n");
            output.push_str("  obj: ok (no external tool required)\n");
        }
        Err(error) => {
            let _ = writeln!(output, "  cranelift: unsupported ({error})");
        }
    }

    output.push_str("\nTools:\n");
    push_linker_status(&mut output, linker.as_deref(), config.linker.as_deref());

    output.push_str("\nTargets:\n");
    for target in known_targets() {
        push_target_status(&mut output, target, linker.is_some());
    }

    output
}

/// Probe an ISA the way the driver would, at the default optimization level.
fn probe_options(freestanding: bool) -> IsaOptions {
    IsaOptions {
        opt_level: OptLevel::Speed,
        position_independent: !freestanding,
    }
}

fn push_linker_status(output: &mut String, found: Option<&Path>, configured: Option<&str>) {
    match found {
        Some(path) => {
            let _ = writeln!(output, "  linker: ok ({})", path.display());
        }
        None => match configured {
            Some(command) => {
                let _ = writeln!(
                    output,
                    "  linker: missing (`{command}` was not found; only --emit exe needs it)"
                );
            }
            None => {
                let _ = writeln!(
                    output,
                    "  linker: missing (tried {}; only --emit exe needs it)",
                    LINKER_CANDIDATES.join(", ")
                );
            }
        },
    }
}

fn push_target_status(output: &mut String, target: &TargetPreset, has_linker: bool) {
    let freestanding = target.runtime_abi == RuntimeAbiKind::Freestanding;
    let runtime = if freestanding {
        "freestanding"
    } else {
        "hosted"
    };

    let codegen = match isa::build(target.triple, probe_options(freestanding)) {
        Ok(_) => "ok".to_string(),
        Err(error) => format!("unsupported ({error})"),
    };

    let executable = if freestanding {
        "n/a (freestanding)"
    } else if codegen != "ok" {
        "n/a"
    } else if has_linker {
        "ok"
    } else {
        "needs a linker"
    };

    let _ = writeln!(
        output,
        "  {:12} {:24} {:13} default: {:4} codegen: {:8} exe: {}",
        target.name,
        target.triple.unwrap_or("host default"),
        runtime,
        target.default_emit.as_str(),
        codegen,
        executable
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_backend_and_every_target() {
        let report = tools_doctor_report(&ToolDoctorConfig::default());

        assert!(report.contains("Hypothalamus tool doctor"));
        assert!(report.contains("Backend:"));
        assert!(report.contains("cranelift: ok"));
        assert!(report.contains("jit: ok"));
        assert!(report.contains("Targets:"));
        for target in known_targets() {
            assert!(report.contains(target.name), "{} missing", target.name);
        }
    }

    #[test]
    fn reports_a_configured_linker_that_is_not_installed() {
        let report = tools_doctor_report(&ToolDoctorConfig {
            linker: Some("hypothalamus-absent-linker".to_string()),
        });

        assert!(report.contains("linker: missing"));
        assert!(report.contains("hypothalamus-absent-linker"));
        // Object output still works, so the target is not written off.
        assert!(report.contains("codegen: ok"));
    }
}
