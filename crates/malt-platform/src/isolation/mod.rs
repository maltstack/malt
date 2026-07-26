//! Cross-platform isolation primitives for process spawning.
//!
//! Provides namespace, cgroup, overlayfs, seccomp, and network isolation
//! for Linux; Job Objects and restricted tokens for Windows; sandbox-exec
//! and setrlimit for macOS.
//!
//! All platform-specific code is gated behind `#[cfg(target_os = "...")]`.
//! On non-applicable platforms, stub implementations return `Unsupported` errors.

// Linux isolation primitives
#[cfg(target_os = "linux")]
pub mod cgroups;
#[cfg(target_os = "linux")]
pub mod namespaces;
#[cfg(target_os = "linux")]
pub mod network;
#[cfg(target_os = "linux")]
pub mod overlayfs;
#[cfg(target_os = "linux")]
pub mod seccomp;

// Windows isolation primitives
#[cfg(target_os = "windows")]
pub mod hcs;
#[cfg(target_os = "windows")]
pub mod job_objects;
#[cfg(target_os = "windows")]
pub mod tokens;

// macOS isolation primitives
#[cfg(target_os = "macos")]
pub mod rlimit;
#[cfg(target_os = "macos")]
pub mod sandbox;

// Cross-platform capability detection
pub mod probe;

mod capability_report;
mod tier;

pub use capability_report::{
    CapabilityBasis, CapabilityReasonCode, CapabilityReport, CapabilityStatus,
};
pub use probe::IsolationCapabilities;
pub use tier::{
    tier_requirements, IsolationContext, IsolationError, IsolationMechanism, IsolationTier,
    TierConstraint, TierRequirements,
};

/// Check whether the requested isolation tier is available on this platform.
///
/// Returns `Ok(())` if the tier can be applied, or an `IsolationError`
/// describing why it cannot (missing capabilities, kernel support, etc.).
///
/// This function performs lightweight runtime probes — it does not actually
/// create namespaces, job objects, or sandboxes, only checks prerequisites.
pub fn tier_available(tier: IsolationTier) -> Result<(), IsolationError> {
    if tier == IsolationTier::Bare {
        return Ok(());
    }

    let caps = IsolationCapabilities::probe();

    match tier {
        IsolationTier::Bare => Ok(()),

        IsolationTier::Restricted => {
            if !caps.supports_restricted() {
                return Err(IsolationError::UnsupportedPlatform(
                    "no basic isolation primitives available on this platform".to_string(),
                ));
            }

            #[cfg(target_os = "linux")]
            {
                // Linux: check for CAP_SYS_ADMIN (required for namespaces)
                if !nix::unistd::geteuid().is_root() {
                    return Err(IsolationError::InsufficientCapabilities(
                        "CAP_SYS_ADMIN (root required)".to_string(),
                    ));
                }
                namespaces::probe_namespaces()?;
            }

            #[cfg(target_os = "windows")]
            {
                // Windows: Job Objects are always available on modern Windows
                // No special privileges needed for basic job object creation
            }

            #[cfg(target_os = "macos")]
            {
                // macOS: sandbox_init is available to all processes
                // No special privileges needed
            }

            Ok(())
        }

        IsolationTier::Capped => {
            if !caps.supports_capped() {
                return Err(IsolationError::UnsupportedPlatform(
                    "resource limit primitives not available on this platform".to_string(),
                ));
            }

            #[cfg(target_os = "linux")]
            {
                if !nix::unistd::geteuid().is_root() {
                    return Err(IsolationError::InsufficientCapabilities(
                        "CAP_SYS_ADMIN (root required)".to_string(),
                    ));
                }
                namespaces::probe_namespaces()?;
                cgroups::probe_cgroup_v2()?;
            }

            #[cfg(target_os = "windows")]
            {
                // Windows: Job Objects support CPU rate and memory limits
                // No additional checks needed beyond Restricted tier
            }

            #[cfg(target_os = "macos")]
            {
                // macOS: setrlimit is always available
                // sandbox_init is available to all processes
            }

            Ok(())
        }

        IsolationTier::Contained => {
            if !caps.supports_contained() {
                return Err(IsolationError::UnsupportedPlatform(
                    "advanced isolation primitives not available on this platform".to_string(),
                ));
            }

            #[cfg(target_os = "linux")]
            {
                if !nix::unistd::geteuid().is_root() {
                    return Err(IsolationError::InsufficientCapabilities(
                        "CAP_SYS_ADMIN (root required)".to_string(),
                    ));
                }
                namespaces::probe_namespaces()?;
                cgroups::probe_cgroup_v2()?;
                overlayfs::probe_overlayfs()?;
                seccomp::probe_seccomp()?;
                network::probe_network_isolation()?;
            }

            #[cfg(target_os = "windows")]
            {
                // Windows: Job Objects + restricted tokens provide strong isolation
                // No additional runtime checks needed
            }

            #[cfg(target_os = "macos")]
            {
                // macOS: sandbox + rlimit provide strong isolation
                // No additional runtime checks needed
            }

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_tier_always_available() {
        assert!(tier_available(IsolationTier::Bare).is_ok());
    }

    #[test]
    fn isolation_error_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<IsolationError>();
    }

    #[test]
    fn isolation_error_is_debug() {
        fn assert_debug<T: std::fmt::Debug>() {}
        assert_debug::<IsolationError>();
    }

    #[test]
    fn capabilities_re_exported() {
        // Verify IsolationCapabilities is accessible from the isolation module
        let _caps = IsolationCapabilities::probe();
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_tiers_availability() {
        // On Windows, Restricted and Capped should always be available
        assert!(tier_available(IsolationTier::Restricted).is_ok());
        assert!(tier_available(IsolationTier::Capped).is_ok());
        // Contained requires job objects + restricted tokens
        assert!(tier_available(IsolationTier::Contained).is_ok());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_tiers_availability() {
        // On macOS, all tiers should be available (sandbox + rlimit)
        assert!(tier_available(IsolationTier::Restricted).is_ok());
        assert!(tier_available(IsolationTier::Capped).is_ok());
        assert!(tier_available(IsolationTier::Contained).is_ok());
    }
}
