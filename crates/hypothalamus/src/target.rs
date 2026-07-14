//! Target profiles for hosted and freestanding Brainfuck output.
//!
//! A target profile is deliberately small: it names an LLVM target triple,
//! runtime ABI, default output kind, optional target image format, and extra
//! LLVM-driver flags. Complete image construction stays in target-specific
//! builder modules.

use crate::driver::EmitKind;
use crate::llvm::{FreestandingOptions, Runtime};

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

    /// Convert this target ABI into LLVM backend runtime options.
    pub fn to_llvm_runtime(&self) -> Runtime {
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

/// Complete-image format produced by a target-specific builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetImageFormat {
    /// Game Boy Advance `.gba` ROM image.
    Gba,
}

/// A built-in target preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetPreset {
    /// Stable CLI/API target name.
    pub name: &'static str,

    /// Short human-readable description.
    pub description: &'static str,

    /// LLVM target triple embedded in IR and passed to `clang`.
    pub llvm_triple: Option<&'static str>,

    /// Runtime ABI used by this preset.
    pub runtime_abi: RuntimeAbiKind,

    /// Additional arguments passed to `clang`.
    pub clang_args: &'static [&'static str],

    /// Default output kind for this target.
    pub default_emit: EmitKind,

    /// Optional complete-image format for this target.
    pub image_format: Option<TargetImageFormat>,
}

impl TargetPreset {
    /// Convert this preset into an owned target profile.
    pub fn profile(self) -> TargetProfile {
        TargetProfile {
            name: self.name.to_string(),
            description: self.description.to_string(),
            llvm_triple: self.llvm_triple.map(ToString::to_string),
            runtime_abi: self.runtime_abi.into_abi(),
            clang_args: self.clang_args.iter().map(ToString::to_string).collect(),
            default_emit: self.default_emit,
            image_format: self.image_format,
        }
    }
}

/// Resolved target configuration used by the compiler driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetProfile {
    name: String,
    description: String,
    llvm_triple: Option<String>,
    runtime_abi: RuntimeAbi,
    clang_args: Vec<String>,
    default_emit: EmitKind,
    image_format: Option<TargetImageFormat>,
}

impl TargetProfile {
    /// Return the hosted native target profile.
    pub fn native() -> Self {
        known_targets()[0].profile()
    }

    /// Resolve a known target name or treat `value` as a raw LLVM triple.
    pub fn resolve(value: &str) -> Self {
        find_target(value)
            .map(TargetPreset::profile)
            .unwrap_or_else(|| Self::raw_llvm_triple(value))
    }

    /// Create a hosted profile for an arbitrary LLVM triple.
    pub fn raw_llvm_triple(triple: &str) -> Self {
        Self {
            name: triple.to_string(),
            description: "raw LLVM target triple".to_string(),
            llvm_triple: Some(triple.to_string()),
            runtime_abi: RuntimeAbi::Hosted,
            clang_args: Vec::new(),
            default_emit: EmitKind::Executable,
            image_format: None,
        }
    }

    /// Create a fully custom target profile.
    pub fn custom(
        name: impl Into<String>,
        description: impl Into<String>,
        llvm_triple: Option<String>,
        runtime_abi: RuntimeAbi,
        clang_args: Vec<String>,
        default_emit: EmitKind,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            llvm_triple,
            runtime_abi,
            clang_args,
            default_emit,
            image_format: None,
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

    /// Return the LLVM target triple, if this profile forces one.
    pub fn llvm_triple(&self) -> Option<&str> {
        self.llvm_triple.as_deref()
    }

    /// Return the target runtime ABI.
    pub fn runtime_abi(&self) -> &RuntimeAbi {
        &self.runtime_abi
    }

    /// Return true when this target uses the freestanding ABI.
    pub fn is_freestanding(&self) -> bool {
        self.runtime_abi.is_freestanding()
    }

    /// Return additional arguments passed to `clang` for this target.
    pub fn clang_args(&self) -> &[String] {
        &self.clang_args
    }

    /// Return the default output kind for this target.
    pub fn default_emit(&self) -> EmitKind {
        self.default_emit
    }

    /// Return the complete-image format supported by this profile, if any.
    pub fn image_format(&self) -> Option<TargetImageFormat> {
        self.image_format
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

    /// Return a copy of this profile with a complete-image format.
    pub fn with_image_format(mut self, image_format: Option<TargetImageFormat>) -> Self {
        self.image_format = image_format;
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

const TARGETS: [TargetPreset; 5] = [
    TargetPreset {
        name: "native",
        description: "hosted executable/JIT on the host LLVM default target",
        llvm_triple: None,
        runtime_abi: RuntimeAbiKind::Hosted,
        clang_args: &[],
        default_emit: EmitKind::Executable,
        image_format: None,
    },
    TargetPreset {
        name: "x86_64-none",
        description: "x86_64 freestanding object for a caller-provided runtime",
        llvm_triple: Some("x86_64-unknown-none"),
        runtime_abi: RuntimeAbiKind::Freestanding,
        clang_args: &[],
        default_emit: EmitKind::Object,
        image_format: None,
    },
    TargetPreset {
        name: "i386-none",
        description: "32-bit x86 freestanding object for tiny boot/runtime layers",
        llvm_triple: Some("i386-unknown-none"),
        runtime_abi: RuntimeAbiKind::Freestanding,
        clang_args: &[],
        default_emit: EmitKind::Object,
        image_format: None,
    },
    TargetPreset {
        name: "nds-arm9",
        description: "Nintendo DS ARM9 freestanding payload object",
        llvm_triple: Some("armv5te-none-eabi"),
        runtime_abi: RuntimeAbiKind::Freestanding,
        clang_args: &["-mcpu=arm946e-s"],
        default_emit: EmitKind::Object,
        image_format: None,
    },
    TargetPreset {
        name: "gba",
        description: "Game Boy Advance ARM7TDMI/Thumb ROM image",
        llvm_triple: Some("thumbv4t-none-eabi"),
        runtime_abi: RuntimeAbiKind::Freestanding,
        clang_args: &["-mcpu=arm7tdmi", "-mthumb"],
        default_emit: EmitKind::Image,
        image_format: Some(TargetImageFormat::Gba),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_known_native_target() {
        let target = TargetProfile::resolve("native");

        assert_eq!(target.name(), "native");
        assert_eq!(target.llvm_triple(), None);
        assert!(!target.is_freestanding());
        assert_eq!(target.default_emit(), EmitKind::Executable);
    }

    #[test]
    fn resolves_raw_triples_as_hosted_targets() {
        let target = TargetProfile::resolve("x86_64-unknown-linux-gnu");

        assert_eq!(target.name(), "x86_64-unknown-linux-gnu");
        assert_eq!(target.llvm_triple(), Some("x86_64-unknown-linux-gnu"));
        assert!(!target.is_freestanding());
        assert_eq!(target.default_emit(), EmitKind::Executable);
    }

    #[test]
    fn gba_is_freestanding_thumb_target() {
        let target = TargetProfile::resolve("gba");

        assert_eq!(target.llvm_triple(), Some("thumbv4t-none-eabi"));
        assert!(target.is_freestanding());
        assert_eq!(target.default_emit(), EmitKind::Image);
        assert_eq!(target.image_format(), Some(TargetImageFormat::Gba));
        assert!(target.clang_args().iter().any(|arg| arg == "-mthumb"));
    }

    #[test]
    fn nds_arm9_is_freestanding_object_target() {
        let target = TargetProfile::resolve("nds-arm9");

        assert_eq!(target.llvm_triple(), Some("armv5te-none-eabi"));
        assert!(target.is_freestanding());
        assert_eq!(target.default_emit(), EmitKind::Object);
        assert_eq!(target.image_format(), None);
        assert!(
            target
                .clang_args()
                .iter()
                .any(|arg| arg == "-mcpu=arm946e-s")
        );
    }

    #[test]
    fn builds_custom_freestanding_target() {
        let target = TargetProfile::custom(
            "weird-board",
            "custom board",
            Some("thumbv7em-none-eabi".to_string()),
            RuntimeAbi::Freestanding(FreestandingOptions::default()),
            vec!["-mcpu=cortex-m4".to_string()],
            EmitKind::Object,
        );

        assert_eq!(target.name(), "weird-board");
        assert_eq!(target.llvm_triple(), Some("thumbv7em-none-eabi"));
        assert!(target.is_freestanding());
        assert_eq!(target.clang_args(), ["-mcpu=cortex-m4"]);
    }
}
