//! Structured capability reporting for isolation primitives.
//!
//! Replaces flat booleans with a status + reason model so callers (and
//! diagnostics) can tell *why* a capability is unavailable, not just that it
//! is. A capability can also be `Degraded` — usable, but with caveats worth
//! surfacing (e.g. a weaker fallback backend is active).

use std::fmt;

/// Whether a capability can be used, and how confidently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityStatus {
    /// Fully available and verified.
    Supported,
    /// Available, but via a weaker fallback or with caveats — see the report's
    /// `reason_detail` for what's different.
    Degraded,
    /// Not available on this system.
    Unsupported,
}

/// Coarse classification of why a capability is unsupported or degraded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityReasonCode {
    /// The backend exists but this build doesn't compile it in.
    NotImplemented,
    /// Required kernel/OS feature isn't present (e.g. cgroup v2 not mounted).
    MissingKernelFeature,
    /// Caller lacks the permission/privilege needed.
    MissingPermission,
    /// A required binary or library isn't present on this system.
    MissingBinary,
    /// This capability doesn't exist on the current platform at all.
    UnsupportedPlatform,
    /// The requested configuration isn't valid or isn't supported here.
    UnsupportedConfiguration,
    /// The probe itself failed unexpectedly.
    RuntimeError,
}

impl fmt::Display for CapabilityReasonCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            CapabilityReasonCode::NotImplemented => "not implemented",
            CapabilityReasonCode::MissingKernelFeature => "missing kernel feature",
            CapabilityReasonCode::MissingPermission => "missing permission",
            CapabilityReasonCode::MissingBinary => "missing binary",
            CapabilityReasonCode::UnsupportedPlatform => "unsupported platform",
            CapabilityReasonCode::UnsupportedConfiguration => "unsupported configuration",
            CapabilityReasonCode::RuntimeError => "runtime error",
        };
        write!(f, "{s}")
    }
}

/// The result of probing a single isolation capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityReport {
    pub status: CapabilityStatus,
    pub reason_code: CapabilityReasonCode,
    pub reason_detail: Option<String>,
}

impl CapabilityReport {
    /// A fully supported, verified capability.
    pub fn supported() -> Self {
        Self {
            status: CapabilityStatus::Supported,
            reason_code: CapabilityReasonCode::NotImplemented,
            reason_detail: None,
        }
    }

    /// Available, but via a fallback or with caveats. `detail` should say what.
    pub fn degraded(detail: impl Into<String>) -> Self {
        Self {
            status: CapabilityStatus::Degraded,
            reason_code: CapabilityReasonCode::UnsupportedConfiguration,
            reason_detail: Some(detail.into()),
        }
    }

    /// Not available. `detail` should say what's missing.
    pub fn unsupported(reason_code: CapabilityReasonCode, detail: impl Into<String>) -> Self {
        Self {
            status: CapabilityStatus::Unsupported,
            reason_code,
            reason_detail: Some(detail.into()),
        }
    }

    /// Usable for isolation purposes — `Supported` or `Degraded`, not `Unsupported`.
    pub fn is_usable(&self) -> bool {
        matches!(
            self.status,
            CapabilityStatus::Supported | CapabilityStatus::Degraded
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_is_usable() {
        assert!(CapabilityReport::supported().is_usable());
    }

    #[test]
    fn degraded_is_usable() {
        let report = CapabilityReport::degraded("fallback active");
        assert!(report.is_usable());
        assert_eq!(report.reason_detail.as_deref(), Some("fallback active"));
    }

    #[test]
    fn unsupported_is_not_usable() {
        let report =
            CapabilityReport::unsupported(CapabilityReasonCode::MissingBinary, "not found");
        assert!(!report.is_usable());
        assert_eq!(report.reason_code, CapabilityReasonCode::MissingBinary);
    }

    #[test]
    fn reason_code_display_is_human_readable() {
        assert_eq!(
            CapabilityReasonCode::MissingKernelFeature.to_string(),
            "missing kernel feature"
        );
    }
}
