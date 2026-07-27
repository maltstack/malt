//! Cross-platform isolation capability detection.
//!
//! Provides `IsolationCapabilities` — a struct that reports which isolation
//! primitives are available on the current platform, and *why* when they
//! aren't.
//!
//! **This answers a different question from `tier::session_tier_capabilities`,
//! and the difference matters.** This module reports *host primitives*: what
//! the operating system offers. `session_tier_capabilities` reports what
//! MALT's own session spawn path can actually establish, which is narrower —
//! an OS primitive that no MALT code path uses is not a session capability.
//!
//! The session surfaces (`malt isolation capabilities`, session creation)
//! read `session_tier_capabilities`. This module is consumed by
//! `tier_available()`, which currently has **no callers** — it is the
//! host-primitive entry point kept for the platform wiring in 007's T032/T033.
//! Stated explicitly because an uncalled mechanism that looks wired is this
//! repo's most repeated defect, and because an earlier version of this file
//! let job objects satisfy `Contained` — a false claim that survived precisely
//! because nothing called it.

use super::capability_report::{CapabilityReasonCode, CapabilityReport};

/// Reports which isolation primitives are available on this system.
///
/// Each field is a [`CapabilityReport`] rather than a plain boolean so
/// callers can distinguish "unavailable" from "available via a weaker
/// fallback" and see the specific reason in diagnostics.
#[derive(Debug, Clone)]
pub struct IsolationCapabilities {
    // Linux capabilities
    /// Linux namespaces (mount, PID, UTS, IPC) are available.
    pub linux_namespaces: CapabilityReport,
    /// Cgroup v2 is mounted and writable.
    pub linux_cgroups: CapabilityReport,
    /// OverlayFS is available for root filesystem isolation.
    pub linux_overlayfs: CapabilityReport,
    /// Seccomp BPF filtering is supported.
    pub linux_seccomp: CapabilityReport,

    // Windows capabilities
    /// Windows Job Objects are available for process grouping and limits.
    pub windows_job_objects: CapabilityReport,
    /// Windows restricted tokens are available for privilege reduction.
    pub windows_restricted_tokens: CapabilityReport,
    /// Windows HCS (Host Compute System) is available for full containment.
    /// Stronger than Job Objects alone — see `isolation::hcs`.
    pub windows_hcs: CapabilityReport,

    // macOS capabilities
    /// macOS sandbox-exec (Seatbelt) is available.
    pub macos_sandbox: CapabilityReport,
    /// setrlimit is available for per-process resource limits.
    pub macos_rlimit: CapabilityReport,
}

impl Default for IsolationCapabilities {
    fn default() -> Self {
        Self {
            linux_namespaces: linux_namespaces_report(),
            linux_cgroups: linux_cgroups_report(),
            linux_overlayfs: linux_overlayfs_report(),
            linux_seccomp: linux_seccomp_report(),
            windows_job_objects: windows_job_objects_report(),
            windows_restricted_tokens: windows_restricted_tokens_report(),
            windows_hcs: windows_hcs_report(),
            macos_sandbox: macos_sandbox_report(),
            macos_rlimit: macos_rlimit_report(),
        }
    }
}

impl IsolationCapabilities {
    /// Probe the current system and return available capabilities.
    ///
    /// This performs lightweight checks — it does not create namespaces,
    /// cgroups, or job objects. It verifies prerequisites only.
    pub fn probe() -> Self {
        Self::default()
    }

    /// Check if any Linux isolation primitives are available.
    pub fn has_linux_isolation(&self) -> bool {
        self.linux_namespaces.is_usable()
            || self.linux_cgroups.is_usable()
            || self.linux_overlayfs.is_usable()
            || self.linux_seccomp.is_usable()
    }

    /// Check if any Windows isolation primitives are available.
    pub fn has_windows_isolation(&self) -> bool {
        self.windows_job_objects.is_usable()
            || self.windows_restricted_tokens.is_usable()
            || self.windows_hcs.is_usable()
    }

    /// Check if any macOS isolation primitives are available.
    pub fn has_macos_isolation(&self) -> bool {
        self.macos_sandbox.is_usable() || self.macos_rlimit.is_usable()
    }

    /// Check if the Restricted tier is achievable on this platform.
    ///
    /// Requires at least one basic isolation primitive:
    /// - Linux: namespaces
    /// - Windows: job objects or HCS
    /// - macOS: sandbox
    pub fn supports_restricted(&self) -> bool {
        self.linux_namespaces.is_usable()
            || self.windows_job_objects.is_usable()
            || self.windows_hcs.is_usable()
            || self.macos_sandbox.is_usable()
    }

    /// Check if the Capped tier is achievable on this platform.
    ///
    /// Requires Restricted + resource limits:
    /// - Linux: namespaces + cgroups
    /// - Windows: job objects (includes CPU rate) or HCS
    /// - macOS: sandbox + rlimit
    pub fn supports_capped(&self) -> bool {
        (self.linux_namespaces.is_usable() && self.linux_cgroups.is_usable())
            || self.windows_job_objects.is_usable()
            || self.windows_hcs.is_usable()
            || (self.macos_sandbox.is_usable() && self.macos_rlimit.is_usable())
    }

    /// Check if the Contained tier is achievable on this platform.
    ///
    /// Requires Capped + a mechanism that establishes a container boundary,
    /// which is strictly more than the resource limits Capped already has:
    /// - Linux: namespaces + cgroups + overlayfs + seccomp
    /// - Windows: HCS only
    /// - macOS: never, on this build
    ///
    /// Two exclusions are deliberate, and both were previously wrong here
    /// (FR-009 — a tier must never be satisfied by a weaker mechanism):
    ///
    /// - **Windows job objects + restricted tokens is not a fallback.** It is
    ///   the same mechanism `supports_capped` already accepts, so admitting it
    ///   made `Capped` and `Contained` aliases: two names, one behaviour, and
    ///   the stronger name claiming a boundary it does not establish.
    /// - **macOS sandbox + rlimit is not sufficient**, for exactly the same
    ///   reason — it is character-for-character the `supports_capped` macOS
    ///   condition. MALT has no macOS mechanism that establishes a container
    ///   boundary, so the honest answer is that the tier is unavailable, not
    ///   that Capped is good enough.
    ///
    /// A macOS or Windows-without-HCS host therefore reports `Contained`
    /// unavailable. Under `required` that refuses the request; under
    /// `preferred` it downgrades visibly. Both are correct outcomes, and both
    /// are better than a session reporting containment it does not have.
    pub fn supports_contained(&self) -> bool {
        (self.linux_namespaces.is_usable()
            && self.linux_cgroups.is_usable()
            && self.linux_overlayfs.is_usable()
            && self.linux_seccomp.is_usable())
            || self.windows_hcs.is_usable()
    }

    /// Explain which required capabilities are missing for a tier, or `None`
    /// if the tier is fully supported. Aggregates every unmet requirement
    /// (not just the first) so a single error message covers everything
    /// that needs fixing.
    pub fn unsupported_detail_for_tier(&self, tier: super::IsolationTier) -> Option<String> {
        use super::IsolationTier;

        // A tier that is available has nothing unsupported to report, even if
        // some facet that *could* provide it is missing. Several tiers are
        // satisfied by alternatives -- Restricted on Windows is job objects
        // OR HCS -- so listing every unusable candidate describes a problem
        // the caller does not have, and contradicts the availability answer
        // given by `supports_*`. Two surfaces disagreeing about the same tier
        // is the defect feature 007 exists to remove (FR-007).
        //
        // This generalises a special case that already existed below for
        // Contained/HCS; that case is now redundant but harmless.
        let available = match tier {
            IsolationTier::Bare => true,
            IsolationTier::Restricted => self.supports_restricted(),
            IsolationTier::Capped => self.supports_capped(),
            IsolationTier::Contained => self.supports_contained(),
        };
        if available {
            return None;
        }

        let candidates: Vec<(&'static str, &CapabilityReport)> = match tier {
            IsolationTier::Bare => return None,
            IsolationTier::Restricted => vec![
                #[cfg(target_os = "linux")]
                ("linux_namespaces", &self.linux_namespaces),
                #[cfg(target_os = "windows")]
                ("windows_job_objects", &self.windows_job_objects),
                #[cfg(target_os = "windows")]
                ("windows_hcs", &self.windows_hcs),
                #[cfg(target_os = "macos")]
                ("macos_sandbox", &self.macos_sandbox),
            ],
            IsolationTier::Capped => vec![
                #[cfg(target_os = "linux")]
                ("linux_namespaces", &self.linux_namespaces),
                #[cfg(target_os = "linux")]
                ("linux_cgroups", &self.linux_cgroups),
                #[cfg(target_os = "windows")]
                ("windows_job_objects", &self.windows_job_objects),
                #[cfg(target_os = "windows")]
                ("windows_hcs", &self.windows_hcs),
                #[cfg(target_os = "macos")]
                ("macos_sandbox", &self.macos_sandbox),
                #[cfg(target_os = "macos")]
                ("macos_rlimit", &self.macos_rlimit),
            ],
            IsolationTier::Contained => vec![
                #[cfg(target_os = "linux")]
                ("linux_namespaces", &self.linux_namespaces),
                #[cfg(target_os = "linux")]
                ("linux_cgroups", &self.linux_cgroups),
                #[cfg(target_os = "linux")]
                ("linux_overlayfs", &self.linux_overlayfs),
                #[cfg(target_os = "linux")]
                ("linux_seccomp", &self.linux_seccomp),
                #[cfg(target_os = "windows")]
                ("windows_hcs", &self.windows_hcs),
                #[cfg(target_os = "macos")]
                ("macos_sandbox", &self.macos_sandbox),
                #[cfg(target_os = "macos")]
                ("macos_rlimit", &self.macos_rlimit),
            ],
        };

        // On Windows, Contained can be met by windows_hcs alone OR by
        // job_objects+restricted_tokens together; don't report job_objects
        // as "missing" if HCS already satisfies the tier.
        #[cfg(target_os = "windows")]
        if matches!(tier, IsolationTier::Contained) && self.windows_hcs.is_usable() {
            return None;
        }

        let failed: Vec<String> = candidates
            .into_iter()
            .filter(|(_, report)| !report.is_usable())
            .map(|(name, report)| {
                let detail = report
                    .reason_detail
                    .as_deref()
                    .unwrap_or("no reason detail provided");
                format!("{name} ({}): {detail}", report.reason_code)
            })
            .collect();

        if !failed.is_empty() {
            return Some(format!(
                "{tier:?} requirements not met: {}",
                failed.join("; ")
            ));
        }

        // Reaching here means the tier is unavailable (checked at the top of
        // this function) and yet every facet that could contribute to it is
        // usable. That is not a contradiction -- it is a tier with no
        // mechanism on this platform at all, which is macOS `Contained`:
        // sandbox and rlimit are both present and both satisfy `Capped`, and
        // nothing here establishes a container boundary.
        //
        // Returning `None` for it would report a tier as unavailable while
        // offering no reason, which is the two-surfaces-disagree defect
        // FR-007 forbids -- and is exactly what
        // `tier_availability_and_unsupported_detail_never_disagree` caught on
        // macOS after `supports_contained` stopped accepting the `Capped`
        // facets under a stronger name.
        Some(format!(
            "{tier:?} has no supporting mechanism on {}: every facet that could \
             contribute to it is present, but none of them establishes what this \
             tier requires beyond the tier below it",
            std::env::consts::OS
        ))
    }
}

// Linux report builders

#[cfg(target_os = "linux")]
fn linux_namespaces_report() -> CapabilityReport {
    CapabilityReport::assumed("namespace availability is inferred; no namespace was created")
}
#[cfg(not(target_os = "linux"))]
fn linux_namespaces_report() -> CapabilityReport {
    CapabilityReport::unsupported(
        CapabilityReasonCode::UnsupportedPlatform,
        "not running on Linux",
    )
}

#[cfg(target_os = "linux")]
fn linux_cgroups_report() -> CapabilityReport {
    if std::path::Path::new("/sys/fs/cgroup/cgroup.controllers").exists() {
        CapabilityReport::verified()
    } else {
        CapabilityReport::unsupported(
            CapabilityReasonCode::MissingKernelFeature,
            "cgroup v2 not mounted",
        )
    }
}
#[cfg(not(target_os = "linux"))]
fn linux_cgroups_report() -> CapabilityReport {
    CapabilityReport::unsupported(
        CapabilityReasonCode::UnsupportedPlatform,
        "not running on Linux",
    )
}

#[cfg(target_os = "linux")]
fn linux_overlayfs_report() -> CapabilityReport {
    let available = std::fs::read_to_string("/proc/filesystems")
        .map(|content| content.lines().any(|line| line.contains("overlay")))
        .unwrap_or(false);
    if available {
        CapabilityReport::verified()
    } else {
        CapabilityReport::unsupported(
            CapabilityReasonCode::MissingKernelFeature,
            "overlay not listed in /proc/filesystems",
        )
    }
}
#[cfg(not(target_os = "linux"))]
fn linux_overlayfs_report() -> CapabilityReport {
    CapabilityReport::unsupported(
        CapabilityReasonCode::UnsupportedPlatform,
        "not running on Linux",
    )
}

#[cfg(target_os = "linux")]
fn linux_seccomp_report() -> CapabilityReport {
    let available = std::fs::read_to_string("/proc/self/status")
        .map(|content| content.lines().any(|line| line.starts_with("Seccomp:")))
        .unwrap_or(false);
    if available {
        CapabilityReport::verified()
    } else {
        CapabilityReport::unsupported(
            CapabilityReasonCode::MissingKernelFeature,
            "Seccomp field not available in /proc/self/status",
        )
    }
}
#[cfg(not(target_os = "linux"))]
fn linux_seccomp_report() -> CapabilityReport {
    CapabilityReport::unsupported(
        CapabilityReasonCode::UnsupportedPlatform,
        "not running on Linux",
    )
}

// Windows report builders

#[cfg(target_os = "windows")]
fn windows_job_objects_report() -> CapabilityReport {
    // Job Objects have been available on every supported Windows version;
    // unlike HCS there's no optional feature to check for.
    CapabilityReport::assumed(
        "Job Objects are expected on supported Windows versions; not created during probe",
    )
}
#[cfg(not(target_os = "windows"))]
fn windows_job_objects_report() -> CapabilityReport {
    CapabilityReport::unsupported(
        CapabilityReasonCode::UnsupportedPlatform,
        "not running on Windows",
    )
}

#[cfg(target_os = "windows")]
fn windows_restricted_tokens_report() -> CapabilityReport {
    CapabilityReport::assumed("restricted-token support is assumed; token not created during probe")
}
#[cfg(not(target_os = "windows"))]
fn windows_restricted_tokens_report() -> CapabilityReport {
    CapabilityReport::unsupported(
        CapabilityReasonCode::UnsupportedPlatform,
        "not running on Windows",
    )
}

#[cfg(target_os = "windows")]
fn windows_hcs_report() -> CapabilityReport {
    if !super::hcs::hcs_available() {
        return CapabilityReport::unsupported(
            CapabilityReasonCode::MissingBinary,
            "computecore.dll not found (Windows Containers feature may not be installed)",
        );
    }
    // `computecore.dll` ships with Windows whether or not the Containers
    // feature is enabled and whether or not this build compiled the HCS
    // backend in, so its presence is not evidence that HCS can be used.
    // Reporting `verified()` on the strength of it advertised `Contained` as
    // available on hosts where every required request for it then failed --
    // the capabilities call disagreeing with reality, which SC-007 exists to
    // forbid.
    match super::hcs::ensure_hcs_runtime() {
        // Still `assumed`, not `verified`: the runtime prerequisites resolve,
        // but no compute system has been created, and creating one is the
        // only thing that would establish the tier is actually deliverable.
        Ok(()) => CapabilityReport::assumed(
            "HCS runtime prerequisites resolve; creating a compute system is not attempted during probing",
        ),
        Err(error) => CapabilityReport::unsupported(
            CapabilityReasonCode::MissingKernelFeature,
            format!("HCS runtime unavailable: {error}"),
        ),
    }
}
#[cfg(not(target_os = "windows"))]
fn windows_hcs_report() -> CapabilityReport {
    CapabilityReport::unsupported(
        CapabilityReasonCode::UnsupportedPlatform,
        "not running on Windows",
    )
}

// macOS report builders

#[cfg(target_os = "macos")]
fn macos_sandbox_report() -> CapabilityReport {
    CapabilityReport::assumed(
        "sandbox_init availability is assumed; sandbox not initialized during probe",
    )
}
#[cfg(not(target_os = "macos"))]
fn macos_sandbox_report() -> CapabilityReport {
    CapabilityReport::unsupported(
        CapabilityReasonCode::UnsupportedPlatform,
        "not running on macOS",
    )
}

#[cfg(target_os = "macos")]
fn macos_rlimit_report() -> CapabilityReport {
    CapabilityReport::assumed("setrlimit availability is assumed; limits not applied during probe")
}
#[cfg(not(target_os = "macos"))]
fn macos_rlimit_report() -> CapabilityReport {
    CapabilityReport::unsupported(
        CapabilityReasonCode::UnsupportedPlatform,
        "not running on macOS",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_probe_succeeds() {
        let caps = IsolationCapabilities::probe();
        let _ = caps;
    }

    #[test]
    fn capabilities_is_debug() {
        let caps = IsolationCapabilities::probe();
        let debug_str = format!("{caps:?}");
        assert!(!debug_str.is_empty());
    }

    #[test]
    fn capabilities_is_cloneable() {
        let caps = IsolationCapabilities::probe();
        let cloned = caps.clone();
        assert_eq!(caps.has_linux_isolation(), cloned.has_linux_isolation());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_capabilities_reflect_platform() {
        let caps = IsolationCapabilities::probe();
        assert!(caps.linux_namespaces.is_usable());
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_capabilities_reflect_platform() {
        let caps = IsolationCapabilities::probe();
        assert!(caps.windows_job_objects.is_usable());
        assert!(caps.windows_restricted_tokens.is_usable());
        assert!(!caps.linux_namespaces.is_usable());
        assert!(!caps.macos_sandbox.is_usable());
        // windows_hcs depends on whether Windows Containers is installed on
        // this machine — just verify the probe runs and reports a reason
        // when unsupported.
        if !caps.windows_hcs.is_usable() {
            assert!(caps.windows_hcs.reason_detail.is_some());
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_capabilities_reflect_platform() {
        let caps = IsolationCapabilities::probe();
        assert!(caps.macos_sandbox.is_usable());
        assert!(caps.macos_rlimit.is_usable());
        assert!(!caps.linux_namespaces.is_usable());
        assert!(!caps.windows_job_objects.is_usable());
    }

    #[test]
    fn has_platform_isolation_methods() {
        let caps = IsolationCapabilities::probe();

        #[cfg(target_os = "linux")]
        {
            assert!(caps.has_linux_isolation());
            assert!(!caps.has_windows_isolation());
            assert!(!caps.has_macos_isolation());
        }

        #[cfg(target_os = "windows")]
        {
            assert!(!caps.has_linux_isolation());
            assert!(caps.has_windows_isolation());
            assert!(!caps.has_macos_isolation());
        }

        #[cfg(target_os = "macos")]
        {
            assert!(!caps.has_linux_isolation());
            assert!(!caps.has_windows_isolation());
            assert!(caps.has_macos_isolation());
        }
    }

    #[test]
    fn supports_restricted_on_current_platform() {
        let caps = IsolationCapabilities::probe();
        assert!(caps.supports_restricted());
    }

    #[test]
    fn supports_capped_on_current_platform() {
        let caps = IsolationCapabilities::probe();

        #[cfg(target_os = "windows")]
        {
            assert!(caps.supports_capped());
        }
        #[cfg(target_os = "macos")]
        {
            assert!(caps.supports_capped());
        }
        #[cfg(target_os = "linux")]
        {
            let _ = caps.supports_capped();
        }
    }

    #[test]
    fn supports_contained_on_current_platform() {
        let caps = IsolationCapabilities::probe();

        #[cfg(target_os = "windows")]
        {
            // Contained tracks HCS and nothing else. Job objects must not
            // stand in for it: they are what Capped already uses, so allowing
            // them here would make the two tiers aliases (FR-009).
            assert_eq!(caps.supports_contained(), caps.windows_hcs.is_usable());
        }
        #[cfg(target_os = "macos")]
        {
            // No macOS mechanism establishes a container boundary on this
            // build, so this is unconditionally false rather than inheriting
            // the sandbox+rlimit pair that already satisfies Capped.
            assert!(!caps.supports_contained());
        }
        #[cfg(target_os = "linux")]
        {
            let _ = caps.supports_contained();
        }
    }

    /// The defect this guards is not that a tier is unavailable -- it is two
    /// tiers being satisfied by the same mechanism, so a caller asking for the
    /// stronger one gets the weaker one under its name. Asserting the
    /// conditions differ catches that regardless of what this host supports.
    #[test]
    fn capped_and_contained_are_not_satisfied_by_the_same_capabilities() {
        let supported = CapabilityReport::supported();
        let unsupported =
            || CapabilityReport::unsupported(CapabilityReasonCode::UnsupportedPlatform, "test");

        // Everything Capped needs on Windows and macOS, and nothing more.
        let capped_only = IsolationCapabilities {
            linux_namespaces: unsupported(),
            linux_cgroups: unsupported(),
            linux_overlayfs: unsupported(),
            linux_seccomp: unsupported(),
            windows_job_objects: supported.clone(),
            windows_restricted_tokens: supported.clone(),
            windows_hcs: unsupported(),
            macos_sandbox: supported.clone(),
            macos_rlimit: supported.clone(),
        };
        assert!(capped_only.supports_capped());
        assert!(
            !capped_only.supports_contained(),
            "Contained must require a mechanism Capped does not already have"
        );

        // Same for Linux: cgroups+namespaces satisfy Capped, and must not
        // satisfy Contained without overlayfs and seccomp.
        let linux_capped_only = IsolationCapabilities {
            linux_namespaces: supported.clone(),
            linux_cgroups: supported.clone(),
            linux_overlayfs: unsupported(),
            linux_seccomp: unsupported(),
            windows_job_objects: unsupported(),
            windows_restricted_tokens: unsupported(),
            windows_hcs: unsupported(),
            macos_sandbox: unsupported(),
            macos_rlimit: unsupported(),
        };
        assert!(linux_capped_only.supports_capped());
        assert!(!linux_capped_only.supports_contained());
    }

    #[test]
    fn empty_capabilities_supports_nothing() {
        let unsupported =
            || CapabilityReport::unsupported(CapabilityReasonCode::UnsupportedPlatform, "test");
        let caps = IsolationCapabilities {
            linux_namespaces: unsupported(),
            linux_cgroups: unsupported(),
            linux_overlayfs: unsupported(),
            linux_seccomp: unsupported(),
            windows_job_objects: unsupported(),
            windows_restricted_tokens: unsupported(),
            windows_hcs: unsupported(),
            macos_sandbox: unsupported(),
            macos_rlimit: unsupported(),
        };
        assert!(!caps.has_linux_isolation());
        assert!(!caps.has_windows_isolation());
        assert!(!caps.has_macos_isolation());
        assert!(!caps.supports_restricted());
        assert!(!caps.supports_capped());
        assert!(!caps.supports_contained());
    }

    #[test]
    fn unsupported_detail_mentions_failed_requirements() {
        let caps = IsolationCapabilities {
            linux_namespaces: CapabilityReport::unsupported(
                CapabilityReasonCode::MissingKernelFeature,
                "namespace unavailable",
            ),
            linux_cgroups: CapabilityReport::supported(),
            linux_overlayfs: CapabilityReport::supported(),
            linux_seccomp: CapabilityReport::supported(),
            windows_job_objects: CapabilityReport::unsupported(
                CapabilityReasonCode::UnsupportedPlatform,
                "not windows",
            ),
            windows_restricted_tokens: CapabilityReport::unsupported(
                CapabilityReasonCode::UnsupportedPlatform,
                "not windows",
            ),
            windows_hcs: CapabilityReport::unsupported(
                CapabilityReasonCode::UnsupportedPlatform,
                "not windows",
            ),
            macos_sandbox: CapabilityReport::unsupported(
                CapabilityReasonCode::UnsupportedPlatform,
                "not macos",
            ),
            macos_rlimit: CapabilityReport::unsupported(
                CapabilityReasonCode::UnsupportedPlatform,
                "not macos",
            ),
        };
        let detail = caps.unsupported_detail_for_tier(super::super::IsolationTier::Restricted);
        #[cfg(target_os = "linux")]
        {
            let detail = detail.expect("must provide detail");
            assert!(detail.contains("linux_namespaces"));
            assert!(detail.contains("namespace unavailable"));
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = detail;
        }
    }

    #[test]
    fn unsupported_detail_is_none_when_tier_is_supported() {
        let caps = IsolationCapabilities::probe();
        // Whatever tier this platform genuinely supports should have no detail.
        if caps.supports_restricted() {
            assert!(caps
                .unsupported_detail_for_tier(super::super::IsolationTier::Restricted)
                .is_none());
        }
    }
}
