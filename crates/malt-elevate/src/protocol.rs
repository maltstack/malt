//! Generated VNP types for the elevate channel.
//!
//! The schema is compiled by `malt-protocol`; this crate deliberately only
//! re-exports those types so its transport cannot drift from the wire contract.

pub use malt_protocol::elevate::{
    ContainerOperation, DaemonEnrollmentRequest, DaemonEnrollmentResponse, ElevateCapabilities,
    ElevateHello, ElevateHelloAck, ElevateRequest, ElevateRequestEnvelope, ElevateResponse,
    ElevateShutdown, HcsEnvironmentEntry, HcsProcessLaunch, HcsProcessRequest, ImageOperation,
    OperationCapability, OutcomeKind, ProvisionedImage, ProvisionedImageList, ReasonCode,
    SessionEntitlementRequest, SessionEntitlementResponse, SCHEMA_VERSION,
};
