//! Target ISA construction for the Cranelift backend.
//!
//! Everything Cranelift needs to know about a target comes from its triple and
//! a small set of shared settings, so this module is the one place that turns a
//! [`TargetProfile`](crate::target::TargetProfile) and an
//! [`OptLevel`](crate::driver::OptLevel) into a configured
//! [`OwnedTargetIsa`].

use crate::driver::OptLevel;
use cranelift_codegen::isa::{self, OwnedTargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use std::fmt;
use std::str::FromStr;
use target_lexicon::Triple;

/// Error returned when a target ISA cannot be configured.
#[derive(Debug)]
pub enum IsaError {
    /// The requested target is not a well-formed target triple.
    InvalidTriple {
        /// The value that was requested.
        target: String,

        /// Why it could not be parsed.
        reason: String,
    },

    /// Cranelift has no backend for the requested triple.
    UnsupportedTarget {
        /// The triple that was requested, or a description of the host.
        target: String,

        /// Cranelift's own explanation.
        reason: String,
    },

    /// The host architecture could not be detected.
    UnsupportedHost(String),

    /// Cranelift rejected the combination of settings.
    InvalidSettings(String),
}

impl fmt::Display for IsaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTriple { target, reason } => {
                write!(f, "`{target}` is not a target triple: {reason}")
            }
            Self::UnsupportedTarget { target, reason } => write!(
                f,
                "Cranelift has no backend for target `{target}`: {reason}. \
                 Supported architectures are x86_64, aarch64, riscv64, and s390x"
            ),
            Self::UnsupportedHost(reason) => {
                write!(f, "failed to detect the host architecture: {reason}")
            }
            Self::InvalidSettings(reason) => write!(f, "invalid code generator settings: {reason}"),
        }
    }
}

impl std::error::Error for IsaError {}

/// Settings that are not implied by the target triple.
#[derive(Debug, Clone, Copy)]
pub struct IsaOptions {
    /// Optimization level Cranelift compiles at.
    pub opt_level: OptLevel,

    /// Generate position-independent code.
    ///
    /// Hosted output wants this, because the usual system linkers build
    /// position-independent executables by default. Freestanding output does
    /// not: a runtime that owns its own load address wants plain absolute
    /// references and no global offset table.
    pub position_independent: bool,
}

/// Build a target ISA for `triple`, or for the host when it is `None`.
pub fn build(triple: Option<&str>, options: IsaOptions) -> Result<OwnedTargetIsa, IsaError> {
    let mut flags = settings::builder();
    flags
        .set("opt_level", options.opt_level.cranelift_setting())
        .map_err(|error| IsaError::InvalidSettings(error.to_string()))?;
    flags
        .set(
            "is_pic",
            if options.position_independent {
                "true"
            } else {
                "false"
            },
        )
        .map_err(|error| IsaError::InvalidSettings(error.to_string()))?;

    let builder = match triple {
        Some(name) => {
            // `isa::lookup_by_name` panics on a triple it cannot parse, and
            // this one comes straight from `--target`, so parse it here where
            // the failure can be reported.
            let triple = Triple::from_str(name).map_err(|error| IsaError::InvalidTriple {
                target: name.to_string(),
                reason: error.to_string(),
            })?;

            isa::lookup(triple).map_err(|error| IsaError::UnsupportedTarget {
                target: name.to_string(),
                reason: error.to_string(),
            })?
        }
        None => cranelift_native::builder()
            .map_err(|error| IsaError::UnsupportedHost(error.to_string()))?,
    };

    builder
        .finish(settings::Flags::new(flags))
        .map_err(|error| IsaError::InvalidSettings(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> IsaOptions {
        IsaOptions {
            opt_level: OptLevel::Speed,
            position_independent: true,
        }
    }

    #[test]
    fn builds_an_isa_for_the_host() {
        let isa = build(None, options()).expect("host should be supported");

        assert!(isa.pointer_bits() >= 32);
    }

    #[test]
    fn builds_an_isa_for_a_named_triple() {
        let isa = build(Some("x86_64-unknown-none"), options()).expect("x86_64 is supported");

        assert_eq!(isa.pointer_bits(), 64);
    }

    #[test]
    fn reports_targets_cranelift_cannot_reach() {
        // Cranelift has no 32-bit ARM backend, which is exactly the kind of
        // target a user coming from an LLVM toolchain is likely to try.
        // `OwnedTargetIsa` is not `Debug`, so this cannot use `expect_err`.
        let Err(error) = build(Some("thumbv4t-none-eabi"), options()) else {
            panic!("32-bit ARM should have no Cranelift backend");
        };

        assert!(matches!(error, IsaError::UnsupportedTarget { .. }));
        assert!(error.to_string().contains("no backend"));
    }

    #[test]
    fn reports_triples_that_are_not_triples() {
        // Cranelift's own `lookup_by_name` panics on these, so the parse has
        // to happen before it does.
        let Err(error) = build(Some("not a triple"), options()) else {
            panic!("`not a triple` should not resolve to an ISA");
        };

        assert!(matches!(error, IsaError::InvalidTriple { .. }), "{error}");
        assert!(error.to_string().contains("is not a target triple"));
    }
}
