//! Target profiles for hosted and freestanding Brainfuck output.
//!
//! A target profile is deliberately small: it names a target triple, a runtime
//! ABI, and a default output kind. Cranelift decides everything else from the
//! triple, so there are no per-target toolchain flags to carry around.
//!
//! Cranelift's backends cover x86-64, aarch64, riscv64, and s390x. There is no
//! 32-bit x86 or ARM backend, so 32-bit presets are not offered; a raw triple
//! naming one is rejected when the target is resolved to an ISA.

use crate::codegen::{FreestandingOptions, Runtime};
use crate::driver::EmitKind;

/// Runtime ABI used by a target profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeAbi {
    /// Hosted C ABI with `main`, `putchar`, and `getchar`.
    Hosted,

    /// Freestanding ABI with caller-supplied byte I/O hooks.
    Freestanding(FreestandingOptions),
}

impl RuntimeAbi {
    /// Return true when this ABI is freestanding.
    pub fn is_freestanding(&self) -> bool {
        matches!(self, Self::Freestanding(_))
    }

    /// Convert this target ABI into backend runtime options.
    pub fn to_codegen_runtime(&self) -> Runtime {
        match self {
            Self::Hosted => Runtime::Hosted,
            Self::Freestanding(options) => Runtime::Freestanding(options.clone()),
        }
    }
}

/// Runtime ABI kind used by static target presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeAbiKind {
    /// Hosted C ABI.
    Hosted,

    /// Freestanding byte I/O hook ABI.
    Freestanding,
}

impl RuntimeAbiKind {
    fn into_abi(self) -> RuntimeAbi {
        match self {
            Self::Hosted => RuntimeAbi::Hosted,
            Self::Freestanding => RuntimeAbi::Freestanding(FreestandingOptions::default()),
        }
    }
}

/// A built-in target preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetPreset {
    /// Stable CLI/API target name.
    pub name: &'static str,

    /// Short human-readable description.
    pub description: &'static str,

    /// Target triple Cranelift compiles for, or `None` for the host.
    pub triple: Option<&'static str>,

    /// Runtime ABI used by this preset.
    pub runtime_abi: RuntimeAbiKind,

    /// Default output kind for this target.
    pub default_emit: EmitKind,
}

impl TargetPreset {
    /// Convert this preset into an owned target profile.
    pub fn profile(self) -> TargetProfile {
        TargetProfile {
            name: self.name.to_string(),
            description: self.description.to_string(),
            triple: self.triple.map(ToString::to_string),
            runtime_abi: self.runtime_abi.into_abi(),
            default_emit: self.default_emit,
        }
    }
}

/// Resolved target configuration used by the compiler driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetProfile {
    name: String,
    description: String,
    triple: Option<String>,
    runtime_abi: RuntimeAbi,
    default_emit: EmitKind,
}

impl TargetProfile {
    /// Return the hosted native target profile.
    pub fn native() -> Self {
        known_targets()[0].profile()
    }

    /// Resolve a known target name or treat `value` as a raw target triple.
    pub fn resolve(value: &str) -> Self {
        find_target(value)
            .map(TargetPreset::profile)
            .unwrap_or_else(|| Self::raw_triple(value))
    }

    /// Create a hosted profile for an arbitrary target triple.
    pub fn raw_triple(triple: &str) -> Self {
        Self {
            name: triple.to_string(),
            description: "raw target triple".to_string(),
            triple: Some(triple.to_string()),
            runtime_abi: RuntimeAbi::Hosted,
            default_emit: EmitKind::Executable,
        }
    }

    /// Create a fully custom target profile.
    pub fn custom(
        name: impl Into<String>,
        description: impl Into<String>,
        triple: Option<String>,
        runtime_abi: RuntimeAbi,
        default_emit: EmitKind,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            triple,
            runtime_abi,
            default_emit,
        }
    }

    /// Return the profile name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the profile description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Return the target triple, if this profile forces one.
    pub fn triple(&self) -> Option<&str> {
        self.triple.as_deref()
    }

    /// Return the target runtime ABI.
    pub fn runtime_abi(&self) -> &RuntimeAbi {
        &self.runtime_abi
    }

    /// Return true when this target uses the freestanding ABI.
    pub fn is_freestanding(&self) -> bool {
        self.runtime_abi.is_freestanding()
    }

    /// Return the default output kind for this target.
    pub fn default_emit(&self) -> EmitKind {
        self.default_emit
    }

    /// Replace the runtime ABI for this profile.
    pub fn set_runtime_abi(&mut self, runtime_abi: RuntimeAbi) {
        self.runtime_abi = runtime_abi;
        if self.runtime_abi.is_freestanding() && self.default_emit == EmitKind::Executable {
            self.default_emit = EmitKind::Object;
        }
    }

    /// Return a copy of this profile with a different runtime ABI.
    pub fn with_runtime_abi(mut self, runtime_abi: RuntimeAbi) -> Self {
        self.set_runtime_abi(runtime_abi);
        self
    }
}

/// Return all built-in target presets.
pub fn known_targets() -> &'static [TargetPreset] {
    &TARGETS
}

/// Find a built-in target preset by name.
pub fn find_target(name: &str) -> Option<TargetPreset> {
    known_targets()
        .iter()
        .copied()
        .find(|target| target.name == name)
}

const TARGETS: [TargetPreset; 2] = [
    TargetPreset {
        name: "native",
        description: "hosted executable or JIT on the host architecture",
        triple: None,
        runtime_abi: RuntimeAbiKind::Hosted,
        default_emit: EmitKind::Executable,
    },
    TargetPreset {
        name: "x86_64-none",
        description: "x86_64 freestanding object for a caller-provided runtime",
        triple: Some("x86_64-unknown-none-elf"),
        runtime_abi: RuntimeAbiKind::Freestanding,
        default_emit: EmitKind::Object,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_known_native_target() {
        let target = TargetProfile::resolve("native");

        assert_eq!(target.name(), "native");
        assert_eq!(target.triple(), None);
        assert!(!target.is_freestanding());
        assert_eq!(target.default_emit(), EmitKind::Executable);
    }

    #[test]
    fn resolves_raw_triples_as_hosted_targets() {
        let target = TargetProfile::resolve("x86_64-unknown-linux-gnu");

        assert_eq!(target.name(), "x86_64-unknown-linux-gnu");
        assert_eq!(target.triple(), Some("x86_64-unknown-linux-gnu"));
        assert!(!target.is_freestanding());
        assert_eq!(target.default_emit(), EmitKind::Executable);
    }

    #[test]
    fn x86_64_none_is_a_freestanding_object_target() {
        let target = TargetProfile::resolve("x86_64-none");

        assert_eq!(target.triple(), Some("x86_64-unknown-none-elf"));
        assert!(target.is_freestanding());
        assert_eq!(target.default_emit(), EmitKind::Object);
    }

    #[test]
    fn builds_custom_freestanding_targets() {
        let target = TargetProfile::custom(
            "weird-board",
            "custom board",
            Some("aarch64-unknown-none-elf".to_string()),
            RuntimeAbi::Freestanding(FreestandingOptions::default()),
            EmitKind::Object,
        );

        assert_eq!(target.name(), "weird-board");
        assert_eq!(target.triple(), Some("aarch64-unknown-none-elf"));
        assert!(target.is_freestanding());
    }

    #[test]
    fn adopting_a_freestanding_abi_switches_the_default_output() {
        let target = TargetProfile::native()
            .with_runtime_abi(RuntimeAbi::Freestanding(FreestandingOptions::default()));

        assert_eq!(target.default_emit(), EmitKind::Object);
    }
}
