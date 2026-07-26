//! Request dispatch for generated `ElevateRequest` values.
//!
//! An outcome is only `Performed` after its handler completes the requested
//! effect. Everything else is an explicit refusal or an indeterminate result.

use std::collections::HashMap;
use std::path::Path;

use crate::protocol::{
    ContainerOperation, ElevateRequest, ElevateResponse, HcsProcessLaunch, HcsProcessRequest,
    OutcomeKind, ReasonCode,
};

/// Dispatch a request to the operation handler.
pub fn dispatch_request(request_id: u32, request: &ElevateRequest) -> ElevateResponse {
    match request {
        ElevateRequest::CreateNamespace { .. } => {
            unsupported_or_unimplemented(request_id, "CreateNamespace", cfg!(target_os = "linux"))
        }
        ElevateRequest::MountOverlay { .. } => {
            unsupported_or_unimplemented(request_id, "MountOverlay", cfg!(target_os = "linux"))
        }
        ElevateRequest::SetCgroup { .. } => {
            unsupported_or_unimplemented(request_id, "SetCgroup", cfg!(target_os = "linux"))
        }
        ElevateRequest::SetupNetns { .. } => {
            unsupported_or_unimplemented(request_id, "SetupNetns", cfg!(target_os = "linux"))
        }
        ElevateRequest::ApplySeccomp { .. } => {
            unsupported_or_unimplemented(request_id, "ApplySeccomp", cfg!(target_os = "linux"))
        }
        ElevateRequest::CreateSymlink { target, link } => {
            dispatch_create_symlink(request_id, Path::new(target), Path::new(link))
        }
        ElevateRequest::CreateRestrictedToken { .. } => unsupported_or_unimplemented(
            request_id,
            "CreateRestrictedToken",
            cfg!(target_os = "windows"),
        ),
        ElevateRequest::ManageHcsContainer { .. } => {
            if !cfg!(windows) {
                unsupported_or_unimplemented(request_id, "ManageHcsContainer", false)
            } else if let Err(error) = malt_platform::isolation::hcs::ensure_hcs_runtime() {
                refused(
                    request_id,
                    ReasonCode::OsError,
                    format!("ManageHcsContainer is unavailable on this host: {error}"),
                )
            } else {
                refused(
                    request_id,
                    ReasonCode::NotEntitled,
                    "ManageHcsContainer requires helper-owned session entitlement authority",
                )
            }
        }
        ElevateRequest::ApplySeatbelt { .. } => {
            unsupported_or_unimplemented(request_id, "ApplySeatbelt", cfg!(target_os = "macos"))
        }
        ElevateRequest::BindPort { .. } => refused(
            request_id,
            ReasonCode::NotImplemented,
            "BindPort is not implemented by this helper build",
        ),
        ElevateRequest::Unknown { .. } => refused(
            request_id,
            ReasonCode::InvalidParameters,
            "unknown elevate operation cannot be validated",
        ),
        _ => refused(
            request_id,
            ReasonCode::InvalidParameters,
            "unrecognized elevate operation cannot be validated",
        ),
    }
}

/// Dispatch an operation after the server has resolved the request's session
/// to helper-owned authority. This deliberately has no arbitrary config or
/// path argument: the only filesystem location HCS receives comes from the
/// entitlement record the service already verified.
pub fn dispatch_entitled_request(
    request_id: u32,
    session_id: u32,
    storage_root: &Path,
    target_process_id: u32,
    request: &ElevateRequest,
    containers: &mut HcsContainerRegistry,
) -> ElevateResponse {
    match request {
        ElevateRequest::ManageHcsContainer { operation } => dispatch_hcs_container(
            request_id,
            session_id,
            storage_root,
            target_process_id,
            operation,
            containers,
        ),
        ElevateRequest::CreateSymlink { target, link } => {
            let target = Path::new(target);
            let link = Path::new(link);
            match (
                malt_platform::fs::canonical_path_within(storage_root, target),
                malt_platform::fs::canonical_creation_path_within(storage_root, link),
            ) {
                (Ok(true), Ok(true)) => dispatch_create_symlink(request_id, target, link),
                (Ok(_), Ok(_)) => refused(
                    request_id,
                    ReasonCode::NotEntitled,
                    "CreateSymlink target and link must remain within the session storage root",
                ),
                (Err(error), _) | (_, Err(error)) => refused(
                    request_id,
                    ReasonCode::InvalidParameters,
                    format!("CreateSymlink path validation failed: {error}"),
                ),
            }
        }
        _ => dispatch_request(request_id, request),
    }
}

/// Helper-owned compute-system handles. The daemon sees only a generated id
/// and can use it only through its own session entitlement.
#[derive(Debug, Default)]
pub struct HcsContainerRegistry {
    containers: HashMap<String, ManagedContainer>,
}

#[derive(Debug)]
struct ManagedContainer {
    session_id: u32,
    system: malt_platform::isolation::hcs::HcsComputeSystem,
}

/// Construct a refusal for an operation the current host cannot perform.
pub fn unsupported_or_unimplemented(
    request_id: u32,
    operation: &str,
    supported_platform: bool,
) -> ElevateResponse {
    let (reason, detail) = if supported_platform {
        (
            ReasonCode::NotImplemented,
            format!("{operation} is not implemented by this helper build"),
        )
    } else {
        (
            ReasonCode::UnsupportedPlatform,
            format!("{operation} is unsupported on this platform"),
        )
    };
    refused(request_id, reason, detail)
}

/// Construct a refusal. Refused outcomes never carry an effect payload.
pub fn refused(request_id: u32, reason: ReasonCode, detail: impl Into<String>) -> ElevateResponse {
    ElevateResponse {
        request_id,
        kind: OutcomeKind::Refused,
        reason: Some(reason),
        detail: Some(detail.into()),
        payload: None,
        _unknown: Vec::new(),
    }
}

fn dispatch_create_symlink(request_id: u32, target: &Path, link: &Path) -> ElevateResponse {
    tracing::info!(
        target = %target.display(),
        link = %link.display(),
        "creating symlink through malt-platform"
    );
    match malt_platform::fs::create_symlink(target, link) {
        Ok(()) => ElevateResponse {
            request_id,
            kind: OutcomeKind::Performed,
            reason: None,
            detail: None,
            payload: Some(Vec::new()),
            _unknown: Vec::new(),
        },
        Err(error) => refused(
            request_id,
            ReasonCode::OsError,
            format!("CreateSymlink failed: {error}"),
        ),
    }
}

fn dispatch_hcs_container(
    request_id: u32,
    session_id: u32,
    storage_root: &Path,
    target_process_id: u32,
    operation: &ContainerOperation,
    containers: &mut HcsContainerRegistry,
) -> ElevateResponse {
    if !cfg!(windows) {
        return unsupported_or_unimplemented(request_id, "ManageHcsContainer", false);
    }
    if let Err(error) = malt_platform::isolation::hcs::ensure_hcs_runtime() {
        return refused(
            request_id,
            ReasonCode::OsError,
            format!("ManageHcsContainer is unavailable on this host: {error}"),
        );
    }
    match operation {
        ContainerOperation::Create {
            memory_limit_mb,
            hostname,
        } => create_hcs_container(
            request_id,
            session_id,
            storage_root,
            *memory_limit_mb,
            hostname.as_deref(),
            containers,
        ),
        ContainerOperation::Start { id } => {
            if containers
                .containers
                .get(id)
                .is_some_and(|container| container.session_id == session_id)
            {
                refused(
                    request_id,
                    ReasonCode::InvalidParameters,
                    "ManageHcsContainer Start is invalid because helper-created compute systems are already started",
                )
            } else {
                refused(
                    request_id,
                    ReasonCode::NotEntitled,
                    "compute-system id is not owned by this session",
                )
            }
        }
        ContainerOperation::Terminate { id } => {
            terminate_hcs_container(request_id, session_id, id, containers)
        }
        ContainerOperation::StartProcess { request } => start_hcs_process(
            request_id,
            session_id,
            target_process_id,
            request,
            containers,
        ),
        _ => refused(
            request_id,
            ReasonCode::InvalidParameters,
            "unknown container operation cannot be validated",
        ),
    }
}

fn create_hcs_container(
    request_id: u32,
    session_id: u32,
    storage_root: &Path,
    memory_limit_mb: Option<u32>,
    hostname: Option<&str>,
    containers: &mut HcsContainerRegistry,
) -> ElevateResponse {
    let hostname =
        match hostname {
            Some(value) if is_valid_hostname(value) => value,
            Some(_) => return refused(
                request_id,
                ReasonCode::InvalidParameters,
                "ManageHcsContainer hostname may contain only ASCII letters, digits, and hyphens",
            ),
            None => "malt",
        };
    let root = match storage_root.to_str() {
        Some(root) => root,
        None => {
            return refused(
                request_id,
                ReasonCode::InvalidParameters,
                "session storage root is not valid UTF-8",
            )
        }
    };
    let id = format!("malt-session-{session_id}-{request_id}");
    if containers.containers.contains_key(&id) {
        return refused(
            request_id,
            ReasonCode::InvalidParameters,
            "compute-system id is already registered for this helper lifetime",
        );
    }
    let config = hcs_config(&id, hostname, root, memory_limit_mb);
    let config = malt_platform::isolation::hcs::HcsConfig {
        id: id.clone(),
        config_json: config,
    };
    match malt_platform::isolation::hcs::create_compute_system(&config) {
        Ok(system) => {
            containers
                .containers
                .insert(id.clone(), ManagedContainer { session_id, system });
            ElevateResponse {
                request_id,
                kind: OutcomeKind::Performed,
                reason: None,
                detail: Some(format!(
                    "ManageHcsContainer created and started helper-owned compute system {id}"
                )),
                payload: Some(id.into_bytes()),
                _unknown: Vec::new(),
            }
        }
        Err(error) => refused(
            request_id,
            ReasonCode::OsError,
            format!("ManageHcsContainer failed: {error}"),
        ),
    }
}

fn start_hcs_process(
    request_id: u32,
    session_id: u32,
    target_process_id: u32,
    request: &HcsProcessRequest,
    containers: &mut HcsContainerRegistry,
) -> ElevateResponse {
    if let Err(detail) = validate_hcs_process_request(request) {
        return refused(request_id, ReasonCode::InvalidParameters, detail);
    }
    let Some(container) = containers.containers.get(&request.id) else {
        return refused(
            request_id,
            ReasonCode::NotEntitled,
            "compute-system id is not owned by this session",
        );
    };
    if container.session_id != session_id {
        return refused(
            request_id,
            ReasonCode::NotEntitled,
            "compute-system id is not owned by this session",
        );
    }
    let parameters = malt_platform::isolation::hcs::HcsProcessParameters {
        application_name: Some(request.program.clone()),
        command_line: malt_platform::process::windows_command_line(
            request.argv0.as_deref().unwrap_or(&request.program),
            &request.arguments,
        ),
        working_directory: request.working_directory.clone(),
        environment: request
            .environment
            .iter()
            .map(|entry| (entry.key.clone(), entry.value.clone()))
            .collect(),
        create_stdin_pipe: true,
        create_stdout_pipe: true,
        create_stderr_pipe: true,
    };
    let launch = match malt_platform::isolation::hcs::create_process(
        container.system.raw_handle(),
        &parameters,
    ) {
        Ok(launch) => launch,
        Err(error) => {
            return refused(
                request_id,
                ReasonCode::OsError,
                format!("ManageHcsContainer StartProcess failed: {error}"),
            )
        }
    };
    let handoff = match launch.duplicate_into_process(target_process_id) {
        Ok(handoff) => handoff,
        Err(error) => {
            return tear_down_after_process_launch_failure(
                request_id,
                &request.id,
                containers,
                format!(
                "could not duplicate HCS process handles into the authenticated daemon: {error}"
            ),
            )
        }
    };
    let mut writer = malt_protocol::vexil_runtime::BitWriter::new();
    let result = HcsProcessLaunch {
        process_id: handoff.process_id,
        process_handle: handoff.process_handle,
        stdin_handle: handoff.stdin_handle,
        stdout_handle: handoff.stdout_handle,
        stderr_handle: handoff.stderr_handle,
        _unknown: Vec::new(),
    };
    if let Err(error) = malt_protocol::vexil_runtime::Pack::pack(&result, &mut writer) {
        return tear_down_after_process_launch_failure(
            request_id,
            &request.id,
            containers,
            format!("could not encode duplicated HCS process handles: {error}"),
        );
    }
    ElevateResponse {
        request_id,
        kind: OutcomeKind::Performed,
        reason: None,
        detail: Some(format!(
            "ManageHcsContainer started a helper-owned process in {} for the authenticated daemon",
            request.id
        )),
        payload: Some(writer.finish()),
        _unknown: Vec::new(),
    }
}

fn validate_hcs_process_request(request: &HcsProcessRequest) -> Result<(), String> {
    if request.id.trim().is_empty() {
        return Err("HCS process request has an empty compute-system id".to_string());
    }
    if request.program.trim().is_empty() {
        return Err("HCS process request has an empty program".to_string());
    }
    let mut fields = request
        .arguments
        .iter()
        .chain(request.working_directory.iter())
        .chain(request.argv0.iter())
        .chain(
            request
                .environment
                .iter()
                .flat_map(|entry| [&entry.key, &entry.value]),
        );
    if request.id.contains('\0')
        || request.program.contains('\0')
        || fields.any(|field| field.contains('\0'))
    {
        return Err("HCS process request contains a NUL byte".to_string());
    }
    if request
        .environment
        .iter()
        .any(|entry| entry.key.is_empty() || entry.key.contains('='))
    {
        return Err("HCS environment keys must be non-empty and cannot contain '='".to_string());
    }
    Ok(())
}

fn tear_down_after_process_launch_failure(
    request_id: u32,
    id: &str,
    containers: &mut HcsContainerRegistry,
    detail: String,
) -> ElevateResponse {
    let cleanup = containers.containers.remove(id).map(|container| {
        malt_platform::isolation::hcs::terminate_compute_system(container.system.raw_handle())
    });
    let detail = match cleanup {
        Some(Ok(())) => format!("{detail}; the affected compute system was terminated"),
        Some(Err(error)) => format!("{detail}; compute-system teardown also failed: {error}"),
        None => format!("{detail}; the helper lost the compute-system record before teardown"),
    };
    refused(request_id, ReasonCode::OsError, detail)
}

fn terminate_hcs_container(
    request_id: u32,
    session_id: u32,
    id: &str,
    containers: &mut HcsContainerRegistry,
) -> ElevateResponse {
    let Some(container) = containers.containers.get(id) else {
        return refused(
            request_id,
            ReasonCode::NotEntitled,
            "compute-system id is not owned by this session",
        );
    };
    if container.session_id != session_id {
        return refused(
            request_id,
            ReasonCode::NotEntitled,
            "compute-system id is not owned by this session",
        );
    }
    let Some(container) = containers.containers.remove(id) else {
        return refused(
            request_id,
            ReasonCode::OsError,
            "helper lost the managed compute-system record before teardown",
        );
    };
    match malt_platform::isolation::hcs::terminate_compute_system(container.system.raw_handle()) {
        Ok(()) => ElevateResponse {
            request_id,
            kind: OutcomeKind::Performed,
            reason: None,
            detail: Some(format!("ManageHcsContainer terminated compute system {id}")),
            payload: Some(id.as_bytes().to_vec()),
            _unknown: Vec::new(),
        },
        Err(error) => refused(
            request_id,
            ReasonCode::OsError,
            format!("ManageHcsContainer teardown failed: {error}"),
        ),
    }
}

fn is_valid_hostname(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn hcs_config(
    id: &str,
    hostname: &str,
    storage_root: &str,
    memory_limit_mb: Option<u32>,
) -> String {
    let storage_root = json_escape(storage_root);
    let memory = memory_limit_mb
        .map(|limit| format!(r#","Memory":{{"SizeInMB":{limit}}}"#))
        .unwrap_or_default();
    format!(
        r#"{{"SchemaVersion":{{"Major":2,"Minor":1}},"Owner":"malt-session-{id}","ShouldTerminateOnLastHandleClosed":true,"Container":{{"Storage":{{"Path":"{storage_root}","Layers":[]}},"GuestOs":{{"HostName":"{hostname}"}}{memory}}}}}"#
    )
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str(r#"\""#),
            '\\' => escaped.push_str(r#"\\"#),
            '\u{08}' => escaped.push_str(r#"\b"#),
            '\u{0C}' => escaped.push_str(r#"\f"#),
            '\n' => escaped.push_str(r#"\n"#),
            '\r' => escaped.push_str(r#"\r"#),
            '\t' => escaped.push_str(r#"\t"#),
            character if character.is_control() => {
                use std::fmt::Write;
                let _ = write!(escaped, r#"\u{:04x}"#, character as u32);
            }
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use malt_protocol::common::IsolationTier;
    use malt_protocol::elevate::ContainerOperation;
    #[cfg(windows)]
    use std::sync::{Mutex, OnceLock};

    #[test]
    fn unimplemented_operations_are_refused_not_reported_as_success() {
        let requests = [
            ElevateRequest::CreateNamespace {
                pid: 1,
                tier: IsolationTier::Bare,
            },
            ElevateRequest::MountOverlay {
                lower: "/lower".into(),
                upper: "/upper".into(),
                merged: "/merged".into(),
            },
            ElevateRequest::SetCgroup {
                pid: 1,
                memory_mb: 512,
                cpu_pct: 50,
            },
            ElevateRequest::SetupNetns {
                pid: 1,
                bridge: "br0".into(),
                veth_host: "veth0".into(),
                veth_ns: "veth1".into(),
            },
            ElevateRequest::ApplySeccomp {
                pid: 1,
                policy: vec![0, 1, 2],
            },
            ElevateRequest::CreateRestrictedToken {
                pid: 1,
                tier: IsolationTier::Restricted,
            },
            ElevateRequest::ManageHcsContainer {
                operation: ContainerOperation::Create {
                    memory_limit_mb: None,
                    hostname: None,
                },
            },
            ElevateRequest::ApplySeatbelt {
                pid: 1,
                profile: "default".into(),
            },
            ElevateRequest::BindPort {
                port: 8080,
                socket_path: "/tmp/sock".into(),
            },
        ];

        for (index, request) in requests.iter().enumerate() {
            let response = dispatch_request(index as u32, request);
            assert_eq!(response.request_id, index as u32);
            assert_eq!(response.kind, OutcomeKind::Refused, "{request:?}");
            assert!(response.reason.is_some(), "{request:?}");
            assert!(response.payload.is_none(), "{request:?}");
        }
    }

    #[test]
    fn response_carries_request_id() {
        let request = ElevateRequest::BindPort {
            port: 443,
            socket_path: "/s".into(),
        };
        assert_eq!(dispatch_request(42, &request).request_id, 42);
    }

    #[test]
    fn hcs_configuration_comes_only_from_typed_fields_and_entitled_root() {
        let json = hcs_config("malt-session-7-8", "malt", r"C:\session-root", Some(512));
        assert!(json.contains(r#""Owner":"malt-session-malt-session-7-8""#));
        assert!(json.contains(r#""Path":"C:\\session-root""#));
        assert!(json.contains(r#""SizeInMB":512"#));
        assert!(!json.contains("config_json"));
    }

    #[test]
    fn hcs_rejects_hostnames_that_cannot_be_safely_rendered() {
        assert!(is_valid_hostname("malt-session-1"));
        assert!(!is_valid_hostname("malt\"injected"));
        assert!(!is_valid_hostname(""));
    }

    #[test]
    fn hcs_json_escapes_entitled_windows_paths() {
        assert_eq!(json_escape(r#"C:\malt\"session"#), r#"C:\\malt\\\"session"#);
    }

    #[test]
    fn entitled_symlink_refuses_path_outside_session_root() {
        let parent = tempfile::tempdir().expect("create test parent");
        let root = parent.path().join("root");
        std::fs::create_dir(&root).expect("create session root");
        let target = root.join("target");
        std::fs::write(&target, "target").expect("create target");
        let request = ElevateRequest::CreateSymlink {
            target: target.to_string_lossy().into_owned(),
            link: parent
                .path()
                .join("outside-link")
                .to_string_lossy()
                .into_owned(),
        };
        let response = dispatch_entitled_request(
            44,
            7,
            &root,
            std::process::id(),
            &request,
            &mut HcsContainerRegistry::default(),
        );
        assert_eq!(response.kind, OutcomeKind::Refused);
        assert_eq!(response.reason, Some(ReasonCode::NotEntitled));
    }

    #[cfg(windows)]
    fn hcs_environment_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(windows)]
    #[test]
    fn helper_owned_hcs_container_lifecycle_is_observable_and_tears_down() {
        let _guard = hcs_environment_lock();
        // SAFETY: process-local environment mutation is serialized by the
        // helper test mutex.
        unsafe {
            std::env::set_var("MALT_HCS_FAKE", "1");
        }
        let root = tempfile::tempdir().expect("create entitled storage root");
        let create = ElevateRequest::ManageHcsContainer {
            operation: ContainerOperation::Create {
                memory_limit_mb: Some(256),
                hostname: Some("malt-test".to_string()),
            },
        };
        let mut containers = HcsContainerRegistry::default();
        let created = dispatch_entitled_request(
            31,
            7,
            root.path(),
            std::process::id(),
            &create,
            &mut containers,
        );
        assert_eq!(created.kind, OutcomeKind::Performed, "{created:?}");
        let id = String::from_utf8(created.payload.expect("container id payload"))
            .expect("container id is UTF-8");
        assert!(
            malt_platform::isolation::hcs::open_compute_system(&id).is_ok(),
            "performed creation must leave a helper-owned compute system"
        );

        let terminate = ElevateRequest::ManageHcsContainer {
            operation: ContainerOperation::Terminate { id: id.clone() },
        };
        let terminated = dispatch_entitled_request(
            32,
            7,
            root.path(),
            std::process::id(),
            &terminate,
            &mut containers,
        );
        assert_eq!(terminated.kind, OutcomeKind::Performed);
        assert!(
            malt_platform::isolation::hcs::open_compute_system(&id).is_err(),
            "performed teardown must leave no compute system behind"
        );

        // SAFETY: paired with the serialized test-only mutation above.
        unsafe {
            std::env::remove_var("MALT_HCS_FAKE");
        }
    }

    #[cfg(windows)]
    #[test]
    fn helper_hands_hcs_process_handles_only_to_the_authenticated_peer() {
        let _guard = hcs_environment_lock();
        // SAFETY: process-local environment mutation is serialized by the
        // helper test mutex.
        unsafe {
            std::env::set_var("MALT_HCS_FAKE", "1");
        }
        let root = tempfile::tempdir().expect("create entitled storage root");
        let mut containers = HcsContainerRegistry::default();
        let created = dispatch_entitled_request(
            41,
            7,
            root.path(),
            std::process::id(),
            &ElevateRequest::ManageHcsContainer {
                operation: ContainerOperation::Create {
                    memory_limit_mb: Some(256),
                    hostname: Some("malt-test".to_string()),
                },
            },
            &mut containers,
        );
        let id = String::from_utf8(created.payload.expect("container id payload"))
            .expect("container id is UTF-8");

        let response = dispatch_entitled_request(
            42,
            7,
            root.path(),
            std::process::id(),
            &ElevateRequest::ManageHcsContainer {
                operation: ContainerOperation::StartProcess {
                    request: HcsProcessRequest {
                        id: id.clone(),
                        program: r"C:\\Windows\\System32\\cmd.exe".to_string(),
                        arguments: vec!["/c".to_string(), "exit 0".to_string()],
                        working_directory: None,
                        environment: Vec::new(),
                        argv0: None,
                        _unknown: Vec::new(),
                    },
                },
            },
            &mut containers,
        );
        assert_eq!(response.kind, OutcomeKind::Performed, "{response:?}");
        let payload = response.payload.expect("HCS process payload");
        let mut reader = malt_protocol::vexil_runtime::BitReader::new(&payload);
        let launch =
            <HcsProcessLaunch as malt_protocol::vexil_runtime::Unpack>::unpack(&mut reader)
                .expect("decode HCS process payload");
        assert_ne!(launch.process_handle, 0);
        assert_ne!(launch.stdin_handle, 0);
        assert_ne!(launch.stdout_handle, 0);
        assert_ne!(launch.stderr_handle, 0);

        let other_session = dispatch_entitled_request(
            43,
            8,
            root.path(),
            std::process::id(),
            &ElevateRequest::ManageHcsContainer {
                operation: ContainerOperation::StartProcess {
                    request: HcsProcessRequest {
                        id: id.clone(),
                        program: r"C:\\Windows\\System32\\cmd.exe".to_string(),
                        arguments: Vec::new(),
                        working_directory: None,
                        environment: Vec::new(),
                        argv0: None,
                        _unknown: Vec::new(),
                    },
                },
            },
            &mut containers,
        );
        assert_eq!(other_session.kind, OutcomeKind::Refused);
        assert_eq!(other_session.reason, Some(ReasonCode::NotEntitled));

        let _ = dispatch_entitled_request(
            44,
            7,
            root.path(),
            std::process::id(),
            &ElevateRequest::ManageHcsContainer {
                operation: ContainerOperation::Terminate { id },
            },
            &mut containers,
        );
        // SAFETY: paired with the serialized test-only mutation above.
        unsafe {
            std::env::remove_var("MALT_HCS_FAKE");
        }
    }
}
