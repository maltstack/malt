//! Isolation tier definitions and context token.
//!
//! The `IsolationTier` enum defines four levels of process isolation.
//! `IsolationContext` is an opaque token carrying tier configuration from
//! the daemon through MASH to the platform spawn traits.

use std::fmt;

/// Isolation tiers for session processes.
///
/// Fixed set — platform capabilities are finite. Higher tiers provide
/// stronger isolation guarantees.
///
/// | Tier        | Primitives                                          |
/// |-------------|-----------------------------------------------------|
/// | Bare        | No isolation                                        |
/// | Restricted  | PID/mount namespaces, PR_SET_NO_NEW_PRIVS           |
/// | Capped      | Restricted + cgroup v2 memory/CPU limits             |
/// | Contained   | Capped + overlayfs + seccomp + network isolation     |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IsolationTier {
    /// No isolation — processes run with normal OS permissions.
    Bare,
    /// Basic restrictions — PID/mount namespaces, PR_SET_NO_NEW_PRIVS.
    Restricted,
    /// Resource-capped — Restricted + cgroup v2 memory/CPU limits.
    Capped,
    /// Full containerization — Capped + overlayfs + seccomp + network.
    Contained,
}

/// The operating-system mechanism a session tier is allowed to name.
///
/// Grows as backends land (ADR-0005), so it is `#[non_exhaustive]`: adding a
/// mechanism must not be a breaking change for anything matching on it.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationMechanism {
    None,
    JobObject,
    Hcs,
    LinuxNamespaces,
    MacosSandbox,
}

/// A concrete promise used to distinguish adjacent tiers in tests and status
/// reporting. A tier must never be represented by a weaker mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierConstraint {
    ProcessGroup,
    MemoryLimit,
    CpuLimit,
    ContainerBoundary,
}

/// Named mechanism and observable promises for one tier on this build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierRequirements {
    pub tier: IsolationTier,
    pub mechanism: IsolationMechanism,
    pub constraints: &'static [TierConstraint],
}

/// Capability as it applies to MALT's actual session spawn path. This is
/// intentionally narrower than a host primitive probe: an unused OS module
/// is not a session capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTierCapability {
    pub tier: IsolationTier,
    pub available: bool,
    pub basis: super::CapabilityBasis,
    pub mechanism: Option<IsolationMechanism>,
    pub detail: Option<String>,
}

pub fn session_tier_capabilities() -> Vec<SessionTierCapability> {
    let unavailable = |tier, detail: &str| SessionTierCapability {
        tier,
        available: false,
        basis: super::CapabilityBasis::None,
        mechanism: None,
        detail: Some(detail.to_string()),
    };
    let mut tiers = vec![SessionTierCapability {
        tier: IsolationTier::Bare,
        available: true,
        basis: super::CapabilityBasis::None,
        mechanism: Some(IsolationMechanism::None),
        detail: None,
    }];
    #[cfg(windows)]
    {
        tiers.push(SessionTierCapability {
            tier: IsolationTier::Restricted,
            available: true,
            basis: super::CapabilityBasis::Assumed,
            mechanism: Some(IsolationMechanism::JobObject),
            detail: Some("Job Object establishment is checked during session creation".to_string()),
        });
        tiers.push(SessionTierCapability {
            tier: IsolationTier::Capped,
            available: true,
            basis: super::CapabilityBasis::Assumed,
            mechanism: Some(IsolationMechanism::JobObject),
            detail: Some("Job Object establishment is checked during session creation".to_string()),
        });
        tiers.push(unavailable(IsolationTier::Contained, "HCS is not wired to MASH's process spawn path; Job Objects are not reported as containers"));
    }
    #[cfg(not(windows))]
    {
        tiers.push(unavailable(
            IsolationTier::Restricted,
            "no session isolation backend is wired on this platform",
        ));
        tiers.push(unavailable(
            IsolationTier::Capped,
            "no session isolation backend is wired on this platform",
        ));
        tiers.push(unavailable(
            IsolationTier::Contained,
            "no session containment backend is wired on this platform",
        ));
    }
    tiers
}

pub fn tier_requirements(tier: IsolationTier) -> TierRequirements {
    match tier {
        IsolationTier::Bare => TierRequirements {
            tier,
            mechanism: IsolationMechanism::None,
            constraints: &[],
        },
        IsolationTier::Restricted => TierRequirements {
            tier,
            mechanism: IsolationMechanism::JobObject,
            constraints: &[TierConstraint::ProcessGroup],
        },
        IsolationTier::Capped => TierRequirements {
            tier,
            mechanism: IsolationMechanism::JobObject,
            constraints: &[
                TierConstraint::ProcessGroup,
                TierConstraint::MemoryLimit,
                TierConstraint::CpuLimit,
            ],
        },
        IsolationTier::Contained => TierRequirements {
            tier,
            mechanism: IsolationMechanism::Hcs,
            constraints: &[TierConstraint::ContainerBoundary],
        },
    }
}

/// Error type for isolation operations.
#[derive(Debug)]
pub enum IsolationError {
    /// The requested operation is not supported on this platform.
    UnsupportedPlatform(String),
    /// Insufficient capabilities (e.g., not root, missing CAP_SYS_ADMIN).
    InsufficientCapabilities(String),
    /// Namespace operation failed.
    NamespaceError(String),
    /// Cgroup operation failed.
    CgroupError(String),
    /// Overlayfs operation failed.
    OverlayfsError(String),
    /// Seccomp operation failed.
    SeccompError(String),
    /// Network isolation operation failed.
    NetworkError(String),
    /// Windows Job Object operation failed.
    JobObjectError(String),
    /// Windows token operation failed.
    TokenError(String),
    /// macOS sandbox operation failed.
    SandboxError(String),
    /// macOS rlimit operation failed.
    RlimitError(String),
    /// Windows HCS (Host Compute System) operation failed.
    HcsError(String),
    /// IO error during isolation setup.
    IoError(std::io::Error),
    /// A live enforcement mechanism cannot be replaced after session creation.
    EstablishmentLocked(String),
}

impl fmt::Display for IsolationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IsolationError::UnsupportedPlatform(msg) => write!(f, "unsupported platform: {msg}"),
            IsolationError::InsufficientCapabilities(msg) => {
                write!(f, "insufficient capabilities: {msg}")
            }
            IsolationError::NamespaceError(msg) => write!(f, "namespace error: {msg}"),
            IsolationError::CgroupError(msg) => write!(f, "cgroup error: {msg}"),
            IsolationError::OverlayfsError(msg) => write!(f, "overlayfs error: {msg}"),
            IsolationError::SeccompError(msg) => write!(f, "seccomp error: {msg}"),
            IsolationError::NetworkError(msg) => write!(f, "network error: {msg}"),
            IsolationError::JobObjectError(msg) => write!(f, "job object error: {msg}"),
            IsolationError::TokenError(msg) => write!(f, "token error: {msg}"),
            IsolationError::SandboxError(msg) => write!(f, "sandbox error: {msg}"),
            IsolationError::RlimitError(msg) => write!(f, "rlimit error: {msg}"),
            IsolationError::HcsError(msg) => write!(f, "hcs error: {msg}"),
            IsolationError::IoError(err) => write!(f, "io error: {err}"),
            IsolationError::EstablishmentLocked(msg) => write!(f, "establishment locked: {msg}"),
        }
    }
}

impl std::error::Error for IsolationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IsolationError::IoError(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for IsolationError {
    fn from(err: std::io::Error) -> Self {
        IsolationError::IoError(err)
    }
}

#[cfg(target_os = "linux")]
impl From<nix::Error> for IsolationError {
    fn from(err: nix::Error) -> Self {
        match err {
            nix::Error::EPERM => IsolationError::InsufficientCapabilities(
                "operation not permitted (EPERM)".to_string(),
            ),
            nix::Error::EACCES => {
                IsolationError::InsufficientCapabilities("permission denied (EACCES)".to_string())
            }
            other => IsolationError::NamespaceError(format!("nix error: {other}")),
        }
    }
}

/// An opaque isolation context token.
///
/// This type is `Clone + Send` but not `Copy` to ensure proper resource management
/// if the internal representation grows to include owned data (e.g., sandbox profile
/// strings, cgroup paths).
///
/// # Usage
///
/// - Daemon creates `IsolationContext` based on session's `IsolationTier`
/// - Daemon passes it to MASH during environment setup
/// - MASH stores it in `Env` and passes it to platform spawn traits
/// - Platform spawn traits interpret the token to apply isolation
#[derive(Debug, Clone)]
pub struct IsolationContext {
    /// The isolation tier this context represents.
    tier: IsolationTier,
    /// Platform-specific isolation configuration (opaque bytes).
    config: Vec<u8>,
    established: Established,
}

/// The live platform resource that actually enforces this context.
#[derive(Debug, Clone)]
pub enum Established {
    Nothing,
    #[cfg(windows)]
    JobObject(std::sync::Arc<super::job_objects::JobObject>),
    Container(String),
}

impl IsolationContext {
    /// Create a new isolation context for the Bare tier (no isolation).
    pub fn bare() -> Self {
        Self {
            tier: IsolationTier::Bare,
            config: Vec::new(),
            established: Established::Nothing,
        }
    }

    /// Create a new isolation context for the Restricted tier.
    pub fn restricted() -> Self {
        Self {
            tier: IsolationTier::Restricted,
            config: Vec::new(),
            established: Established::Nothing,
        }
    }

    /// Create a new isolation context for the Capped tier.
    pub fn capped() -> Self {
        Self {
            tier: IsolationTier::Capped,
            config: Vec::new(),
            established: Established::Nothing,
        }
    }

    /// Create a new isolation context for the Contained tier.
    pub fn contained() -> Self {
        Self {
            tier: IsolationTier::Contained,
            config: Vec::new(),
            established: Established::Nothing,
        }
    }

    /// Get the isolation tier for this context.
    pub fn tier(&self) -> IsolationTier {
        self.tier
    }

    /// Returns true if this is the Bare tier (no isolation).
    pub fn is_bare(&self) -> bool {
        matches!(self.tier, IsolationTier::Bare)
    }

    /// Returns true if this context has isolation configured.
    pub fn has_isolation(&self) -> bool {
        !self.is_bare()
    }

    /// Set opaque platform-specific configuration bytes.
    ///
    /// This is used by the daemon to inject platform-specific isolation
    /// configuration that the spawn traits will interpret.
    pub fn set_config(&mut self, config: Vec<u8>) {
        self.config = config;
    }

    /// Get the opaque platform-specific configuration bytes.
    pub fn config(&self) -> &[u8] {
        &self.config
    }

    #[cfg(windows)]
    pub fn establish_job_object(
        &mut self,
        job: std::sync::Arc<super::job_objects::JobObject>,
    ) -> Result<(), IsolationError> {
        self.ensure_unestablished("Job Object")?;
        self.established = Established::JobObject(job);
        Ok(())
    }

    #[cfg(windows)]
    pub fn job_object(&self) -> Option<&std::sync::Arc<super::job_objects::JobObject>> {
        match &self.established {
            Established::JobObject(job) => Some(job),
            _ => None,
        }
    }

    /// Record the HCS compute-system identity after a helper reports it
    /// performed. This is deliberately separate from requesting containment.
    pub fn establish_container(&mut self, id: String) -> Result<(), IsolationError> {
        self.ensure_unestablished("HCS container")?;
        self.established = Established::Container(id);
        Ok(())
    }

    pub fn container_id(&self) -> Option<&str> {
        match &self.established {
            Established::Container(id) => Some(id),
            _ => None,
        }
    }

    /// Remove a lost enforcement mechanism without changing the requested
    /// tier. This records degraded reality but never permits a later upgrade.
    pub fn clear_established(&mut self) {
        self.established = Established::Nothing;
    }

    fn ensure_unestablished(&self, attempted: &str) -> Result<(), IsolationError> {
        if matches!(self.established, Established::Nothing) {
            Ok(())
        } else {
            Err(IsolationError::EstablishmentLocked(format!(
                "cannot establish {attempted} after a session mechanism already exists"
            )))
        }
    }
}

impl From<IsolationTier> for IsolationContext {
    fn from(tier: IsolationTier) -> Self {
        match tier {
            IsolationTier::Bare => Self::bare(),
            IsolationTier::Restricted => Self::restricted(),
            IsolationTier::Capped => Self::capped(),
            IsolationTier::Contained => Self::contained(),
        }
    }
}

/// Convert from protocol IsolationTier to platform IsolationContext.
/// This bridges the wire format (protocol) to the implementation (platform).
#[allow(clippy::match_same_arms)]
impl From<malt_protocol::common::IsolationTier> for IsolationContext {
    fn from(tier: malt_protocol::common::IsolationTier) -> Self {
        match tier {
            malt_protocol::common::IsolationTier::Bare => Self::bare(),
            malt_protocol::common::IsolationTier::Restricted => Self::restricted(),
            malt_protocol::common::IsolationTier::Capped => Self::capped(),
            malt_protocol::common::IsolationTier::Contained => Self::contained(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_context_has_no_isolation() {
        let ctx = IsolationContext::bare();
        assert!(ctx.is_bare());
        assert!(!ctx.has_isolation());
        assert_eq!(ctx.tier(), IsolationTier::Bare);
    }

    #[test]
    fn restricted_context_has_isolation() {
        let ctx = IsolationContext::restricted();
        assert!(!ctx.is_bare());
        assert!(ctx.has_isolation());
        assert_eq!(ctx.tier(), IsolationTier::Restricted);
    }

    #[test]
    fn capped_context_has_isolation() {
        let ctx = IsolationContext::capped();
        assert!(!ctx.is_bare());
        assert!(ctx.has_isolation());
        assert_eq!(ctx.tier(), IsolationTier::Capped);
    }

    #[test]
    fn contained_context_has_isolation() {
        let ctx = IsolationContext::contained();
        assert!(!ctx.is_bare());
        assert!(ctx.has_isolation());
        assert_eq!(ctx.tier(), IsolationTier::Contained);
    }

    #[test]
    fn context_is_cloneable() {
        let ctx = IsolationContext::restricted();
        let cloned = ctx.clone();
        assert_eq!(ctx.tier(), cloned.tier());
    }

    #[test]
    fn context_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<IsolationContext>();
    }

    #[test]
    fn config_roundtrip() {
        let mut ctx = IsolationContext::restricted();
        let config = vec![1, 2, 3, 4, 5];
        ctx.set_config(config.clone());
        assert_eq!(ctx.config(), &config[..]);
    }

    #[test]
    fn from_tier_conversion() {
        assert!(matches!(
            IsolationContext::from(IsolationTier::Bare),
            IsolationContext {
                tier: IsolationTier::Bare,
                ..
            }
        ));
        assert!(matches!(
            IsolationContext::from(IsolationTier::Restricted),
            IsolationContext {
                tier: IsolationTier::Restricted,
                ..
            }
        ));
    }

    #[test]
    fn isolation_error_display() {
        let err = IsolationError::UnsupportedPlatform("test".to_string());
        assert!(err.to_string().contains("unsupported platform"));

        let err = IsolationError::InsufficientCapabilities("test".to_string());
        assert!(err.to_string().contains("insufficient capabilities"));

        let err = IsolationError::NamespaceError("test".to_string());
        assert!(err.to_string().contains("namespace error"));

        let err = IsolationError::CgroupError("test".to_string());
        assert!(err.to_string().contains("cgroup error"));

        let err = IsolationError::OverlayfsError("test".to_string());
        assert!(err.to_string().contains("overlayfs error"));

        let err = IsolationError::SeccompError("test".to_string());
        assert!(err.to_string().contains("seccomp error"));

        let err = IsolationError::NetworkError("test".to_string());
        assert!(err.to_string().contains("network error"));

        let err = IsolationError::JobObjectError("test".to_string());
        assert!(err.to_string().contains("job object error"));

        let err = IsolationError::TokenError("test".to_string());
        assert!(err.to_string().contains("token error"));

        let err = IsolationError::SandboxError("test".to_string());
        assert!(err.to_string().contains("sandbox error"));

        let err = IsolationError::RlimitError("test".to_string());
        assert!(err.to_string().contains("rlimit error"));

        let err = IsolationError::HcsError("test".to_string());
        assert!(err.to_string().contains("hcs error"));

        let err = IsolationError::EstablishmentLocked("test".to_string());
        assert!(err.to_string().contains("establishment locked"));
    }

    #[test]
    fn isolation_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let err = IsolationError::from(io_err);
        assert!(matches!(err, IsolationError::IoError(_)));
    }

    #[test]
    fn tier_equality() {
        assert_eq!(IsolationTier::Bare, IsolationTier::Bare);
        assert_ne!(IsolationTier::Bare, IsolationTier::Restricted);
        assert_ne!(IsolationTier::Restricted, IsolationTier::Capped);
        assert_ne!(IsolationTier::Capped, IsolationTier::Contained);
    }

    #[test]
    fn tier_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(IsolationTier::Bare);
        set.insert(IsolationTier::Restricted);
        set.insert(IsolationTier::Capped);
        set.insert(IsolationTier::Contained);
        assert_eq!(set.len(), 4);
    }

    #[test]
    fn contained_does_not_alias_capped_requirements() {
        assert_ne!(
            tier_requirements(IsolationTier::Capped),
            tier_requirements(IsolationTier::Contained)
        );
    }

    #[test]
    fn establishment_cannot_be_upgraded_after_session_creation() {
        let mut context = IsolationContext::contained();
        context
            .establish_container("first-container".to_string())
            .expect("first mechanism is allowed during session creation");
        assert!(context
            .establish_container("replacement-container".to_string())
            .is_err());
        assert_eq!(context.container_id(), Some("first-container"));
    }
}
