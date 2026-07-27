//! Request dispatch for generated `ElevateRequest` values.
//!
//! An outcome is only `Performed` after its handler completes the requested
//! effect. Everything else is an explicit refusal or an indeterminate result.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::protocol::{
    ContainerOperation, ElevateRequest, ElevateResponse, HcsProcessLaunch, HcsProcessRequest,
    ImageOperation, OutcomeKind, ProvisionedImage, ProvisionedImageList, ReasonCode,
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
        ElevateRequest::ManageImage { operation } => {
            dispatch_image_operation(request_id, operation)
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

/// Dispatch an authenticated daemon's image request. Image IDs are manifest
/// digests; helper code resolves them only in its ProgramData-owned store.
pub fn dispatch_image_operation(request_id: u32, operation: &ImageOperation) -> ElevateResponse {
    dispatch_image_operation_with_containers(
        request_id,
        operation,
        &HcsContainerRegistry::default(),
    )
}

/// Dispatch an image operation with the live helper compute-system registry.
/// The registry is the helper's independent authority for refusing removal of
/// an image whose writable workspace is still active.
pub fn dispatch_image_operation_with_containers(
    request_id: u32,
    operation: &ImageOperation,
    containers: &HcsContainerRegistry,
) -> ElevateResponse {
    if !cfg!(windows) {
        return unsupported_or_unimplemented(request_id, "ManageImage", false);
    }
    let store = match helper_image_store() {
        Ok(store) => store,
        Err(detail) => return refused(request_id, ReasonCode::OsError, detail),
    };
    match operation {
        ImageOperation::Provision { reference } => {
            match malt_image::acquire_public_windows_image(&store, reference).and_then(|record| {
                prepare_image(&store, record).map_err(malt_image::ProvisionError::Store)
            }) {
                Ok(record) => pack_image_response(
                    request_id,
                    image_view(&record, active_session_count(containers, &record)),
                ),
                Err(error) => refused(
                    request_id,
                    ReasonCode::OsError,
                    format!("image provision failed: {error}"),
                ),
            }
        }
        ImageOperation::List { .. } => match store.list_records() {
            Ok(records) => pack_image_list_response(
                request_id,
                records
                    .iter()
                    .map(|record| image_view(record, active_session_count(containers, record)))
                    .collect(),
            ),
            Err(error) => refused(
                request_id,
                ReasonCode::OsError,
                format!("could not list helper-owned images: {error}"),
            ),
        },
        ImageOperation::Inspect { id } => match parse_image_id(id).and_then(|digest| {
            store
                .load_record(&digest)
                .map_err(|error| error.to_string())
        }) {
            Ok(record) => pack_image_response(
                request_id,
                image_view(&record, active_session_count(containers, &record)),
            ),
            Err(detail) => refused(
                request_id,
                ReasonCode::InvalidParameters,
                format!("unknown helper-owned image: {detail}"),
            ),
        },
        ImageOperation::Remove { id } => match parse_image_id(id).and_then(|digest| {
            let record = store
                .load_record(&digest)
                .map_err(|error| error.to_string())?;
            let digest_text = record.manifest_digest.to_string();
            let dependent_sessions = containers
                .containers
                .values()
                .filter(|container| container.image_id.as_deref() == Some(digest_text.as_str()))
                .map(|container| container.session_id.to_string())
                .collect::<Vec<_>>();
            if !dependent_sessions.is_empty() {
                return Err(format!(
                    "cannot remove helper-owned image while contained sessions are active: {}",
                    dependent_sessions.join(", ")
                ));
            }
            if record.prepared {
                remove_prepared_image(&store, &record)?;
            }
            store
                .remove_record(&digest)
                .map_err(|error| error.to_string())?;
            Ok(())
        }) {
            Ok(()) => performed(
                request_id,
                Some(id.as_bytes().to_vec()),
                "helper-owned image record removed",
            ),
            Err(detail) => refused(request_id, ReasonCode::InvalidParameters, detail),
        },
        ImageOperation::Unknown { .. } => refused(
            request_id,
            ReasonCode::InvalidParameters,
            "unknown image operation",
        ),
        _ => refused(
            request_id,
            ReasonCode::InvalidParameters,
            "unrecognized image operation",
        ),
    }
}

fn remove_prepared_image(
    store: &malt_image::ImageStore,
    record: &malt_image::ImageRecord,
) -> Result<(), String> {
    let digest = record
        .manifest_digest
        .to_string()
        .trim_start_matches("sha256:")
        .to_string();
    let root = store.root().join("prepared").join(&digest);
    for index in (0..record.manifest.layers.len()).rev() {
        let layer = malt_platform::isolation::layers::prepared_layer(
            root.join("layers").join(index.to_string()),
        )
        .map_err(|error| error.to_string())?;
        malt_platform::isolation::layers::destroy_prepared_layer(layer)
            .map_err(|error| error.to_string())?;
    }
    malt_platform::isolation::layers::remove_owned_tree(store.root(), &root)
        .map_err(|error| error.to_string())
}

fn prepare_image(
    store: &malt_image::ImageStore,
    mut record: malt_image::ImageRecord,
) -> Result<malt_image::ImageRecord, malt_image::StoreError> {
    if record.prepared {
        return Ok(record);
    }
    if let Err(error) = malt_platform::isolation::hcs::ensure_hcs_runtime() {
        tracing::info!(%error, manifest = %record.manifest_digest, "image acquired but HCS preparation is unavailable");
        return Ok(record);
    }
    let digest = record
        .manifest_digest
        .to_string()
        .trim_start_matches("sha256:")
        .to_string();
    let root = store.root().join("prepared").join(&digest);
    let sources = root.join("sources");
    let layers_root = root.join("layers");
    let result = (|| -> Result<Vec<malt_platform::isolation::layers::PreparedLayer>, String> {
        std::fs::create_dir_all(&sources).map_err(|error| error.to_string())?;
        let mut parents = Vec::new();
        for (index, descriptor) in record.manifest.layers.iter().enumerate() {
            let source = sources.join(index.to_string());
            let input = std::fs::File::open(store.blob_path(&descriptor.digest))
                .map_err(|error| error.to_string())?;
            malt_image::extract_gzip_layer(input, &source).map_err(|error| error.to_string())?;
            let layer = malt_platform::isolation::layers::materialize_layer(
                &layers_root.join(index.to_string()),
                &source,
                &parents,
            )
            .map_err(|error| error.to_string())?;
            parents.push(layer);
        }
        Ok(parents)
    })();
    if let Err(error) = result {
        let _ = malt_platform::isolation::layers::remove_owned_tree(store.root(), &root);
        return Err(malt_image::StoreError::Io(std::io::Error::other(format!(
            "HCS image preparation failed: {error}"
        ))));
    }
    record.prepared = true;
    store.replace_record(&record)?;
    Ok(record)
}

fn helper_image_store() -> Result<malt_image::ImageStore, String> {
    let program_data = std::env::var_os("ProgramData")
        .ok_or_else(|| "ProgramData is not set for the elevated helper".to_string())?;
    let root = PathBuf::from(program_data).join("MALT").join("images");
    malt_platform::isolation::layers::ensure_owned_root(&root)
        .map_err(|error| error.to_string())?;
    malt_image::ImageStore::open(root).map_err(|error| error.to_string())
}

fn parse_image_id(value: &str) -> Result<malt_image::Digest, String> {
    value
        .parse::<malt_image::Digest>()
        .map_err(|error| error.to_string())
}

fn active_session_count(
    containers: &HcsContainerRegistry,
    record: &malt_image::ImageRecord,
) -> u32 {
    let digest = record.manifest_digest.to_string();
    u32::try_from(
        containers
            .containers
            .values()
            .filter(|container| container.image_id.as_deref() == Some(digest.as_str()))
            .count(),
    )
    .unwrap_or(u32::MAX)
}

fn image_view(record: &malt_image::ImageRecord, active_sessions: u32) -> ProvisionedImage {
    ProvisionedImage {
        id: record.manifest_digest.to_string(),
        manifest_digest: record.manifest_digest.to_string(),
        platform: format!("{}/{}", record.platform.os, record.platform.architecture),
        os_version: record.platform.os_version.clone(),
        ready: record.prepared,
        reason: (!record.prepared)
            .then(|| "image acquired and verified but not yet HCS-prepared".to_string()),
        active_sessions,
        _unknown: Vec::new(),
    }
}

fn pack_image_response(request_id: u32, image: ProvisionedImage) -> ElevateResponse {
    let mut writer = malt_protocol::vexil_runtime::BitWriter::new();
    match malt_protocol::vexil_runtime::Pack::pack(&image, &mut writer) {
        Ok(()) => performed(
            request_id,
            Some(writer.finish()),
            "helper-owned image operation completed",
        ),
        Err(error) => refused(
            request_id,
            ReasonCode::OsError,
            format!("could not encode image response: {error}"),
        ),
    }
}

fn pack_image_list_response(request_id: u32, images: Vec<ProvisionedImage>) -> ElevateResponse {
    let mut writer = malt_protocol::vexil_runtime::BitWriter::new();
    let list = ProvisionedImageList {
        images,
        _unknown: Vec::new(),
    };
    match malt_protocol::vexil_runtime::Pack::pack(&list, &mut writer) {
        Ok(()) => performed(
            request_id,
            Some(writer.finish()),
            "helper-owned image list completed",
        ),
        Err(error) => refused(
            request_id,
            ReasonCode::OsError,
            format!("could not encode image list response: {error}"),
        ),
    }
}

fn performed(
    request_id: u32,
    payload: Option<Vec<u8>>,
    detail: impl Into<String>,
) -> ElevateResponse {
    ElevateResponse {
        request_id,
        kind: OutcomeKind::Performed,
        reason: None,
        detail: Some(detail.into()),
        payload,
        _unknown: Vec::new(),
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
    image_id: Option<String>,
    system: malt_platform::isolation::hcs::HcsComputeSystem,
    workspace: Option<malt_platform::isolation::layers::WritableLayer>,
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
            image_id,
        } => create_hcs_container(
            request_id,
            session_id,
            storage_root,
            *memory_limit_mb,
            hostname.as_deref(),
            image_id.as_deref(),
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
    image_id: Option<&str>,
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
    let fake_test = cfg!(test)
        && matches!(
            std::env::var("MALT_HCS_FAKE").as_deref(),
            Ok("1") | Ok("true")
        );
    let store = match helper_image_store() {
        Ok(store) => store,
        Err(detail) => return refused(request_id, ReasonCode::OsError, detail),
    };
    let ready = match store.list_records() {
        Ok(records) => records
            .into_iter()
            .filter(|record| record.prepared)
            .collect::<Vec<_>>(),
        Err(error) => {
            return refused(
                request_id,
                ReasonCode::OsError,
                format!("could not inspect helper-owned images: {error}"),
            )
        }
    };
    let (parents, workspace, root, selected_image) = if fake_test {
        (
            Vec::new(),
            None,
            storage_root.to_string_lossy().to_string(),
            None,
        )
    } else {
        let record = match image_id {
            Some(image_id) => match ready.iter().find(|record| record.manifest_digest.to_string() == image_id) { Some(record) => record, None => return refused(request_id, ReasonCode::InvalidParameters, "selected image is not a ready helper-owned image") },
            None => match ready.as_slice() {
            [record] => record,
            [] => return refused(request_id, ReasonCode::InvalidParameters, "contained session requires one ready helper-owned Windows image; provision one with `malt image provision`"),
            _ => return refused(request_id, ReasonCode::InvalidParameters, "contained session requires an explicit image selector because multiple ready helper-owned images exist"),
            },
        };
        let digest = record
            .manifest_digest
            .to_string()
            .trim_start_matches("sha256:")
            .to_string();
        let parents = (0..record.manifest.layers.len())
            .map(|index| {
                malt_platform::isolation::layers::prepared_layer(
                    store
                        .root()
                        .join("prepared")
                        .join(&digest)
                        .join("layers")
                        .join(index.to_string()),
                )
            })
            .collect::<Result<Vec<_>, _>>();
        let parents = match parents {
            Ok(parents) => parents,
            Err(error) => {
                return refused(
                    request_id,
                    ReasonCode::OsError,
                    format!("could not reopen helper-owned prepared layers: {error}"),
                )
            }
        };
        let workspace = match malt_platform::isolation::layers::initialize_writable_layer(
            &store
                .root()
                .join("sessions")
                .join(session_id.to_string())
                .join(request_id.to_string()),
            &parents,
        ) {
            Ok(workspace) => workspace,
            Err(error) => {
                return refused(
                    request_id,
                    ReasonCode::OsError,
                    format!("could not create helper-owned contained workspace: {error}"),
                )
            }
        };
        let root = workspace.mount_path().to_string();
        (
            parents,
            Some(workspace),
            root,
            Some(record.manifest_digest.to_string()),
        )
    };
    let id = format!("malt-session-{session_id}-{request_id}");
    if containers.containers.contains_key(&id) {
        return refused(
            request_id,
            ReasonCode::InvalidParameters,
            "compute-system id is already registered for this helper lifetime",
        );
    }
    let config = hcs_config(&id, hostname, &root, memory_limit_mb, &parents);
    let config = malt_platform::isolation::hcs::HcsConfig {
        id: id.clone(),
        config_json: config,
    };
    match malt_platform::isolation::hcs::create_compute_system(&config) {
        Ok(system) => {
            containers.containers.insert(
                id.clone(),
                ManagedContainer {
                    session_id,
                    image_id: selected_image,
                    system,
                    workspace,
                },
            );
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
        Err(error) => {
            let cleanup = workspace
                .map(malt_platform::isolation::layers::destroy_writable_layer)
                .transpose();
            refused(
                request_id,
                ReasonCode::OsError,
                format!(
                    "ManageHcsContainer failed: {error}; workspace cleanup: {}",
                    cleanup
                        .map(|_| "complete".to_string())
                        .unwrap_or_else(|cleanup| cleanup.to_string())
                ),
            )
        }
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
        emulate_console: true,
        create_stdin_pipe: true,
        create_stdout_pipe: true,
        create_stderr_pipe: true,
    };
    let mut launch = match malt_platform::isolation::hcs::create_process(
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
    let process = match launch.take_process_for_reaper() {
        Ok(process) => process,
        Err(error) => {
            return tear_down_after_process_launch_failure(
                request_id,
                &request.id,
                containers,
                format!("could not retain HCS process until stream completion: {error}"),
            )
        }
    };
    if let Err(error) = std::thread::Builder::new()
        .name(format!("malt-hcs-reaper-{}", process.process_id))
        .spawn(move || {
            if let Err(error) =
                malt_platform::isolation::hcs::wait_process_exit(process.raw_handle())
            {
                tracing::warn!(%error, "HCS process reaper could not observe process exit");
            }
        })
    {
        return tear_down_after_process_launch_failure(
            request_id,
            &request.id,
            containers,
            format!("could not start HCS process reaper: {error}"),
        );
    }
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
            .and_then(|()| {
                container
                    .workspace
                    .map(malt_platform::isolation::layers::destroy_writable_layer)
                    .transpose()
            })
            .map(|_| ())
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
    match malt_platform::isolation::hcs::terminate_compute_system(container.system.raw_handle())
        .and_then(|()| {
            container
                .workspace
                .map(malt_platform::isolation::layers::destroy_writable_layer)
                .transpose()
        })
        .map(|_| ())
    {
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
    parents: &[malt_platform::isolation::layers::PreparedLayer],
) -> String {
    let mut container = serde_json::json!({
        "Storage": { "Path": storage_root, "Layers": parents.iter().map(|parent| serde_json::json!({ "Id": parent.id, "Path": parent.path })).collect::<Vec<_>>() },
        "GuestOs": { "HostName": hostname },
    });
    if let Some(limit) = memory_limit_mb {
        container["Memory"] = serde_json::json!({ "SizeInMB": limit });
    }
    serde_json::json!({
        "SchemaVersion": { "Major": 2, "Minor": 0 },
        "Owner": format!("malt-session-{id}"),
        "ShouldTerminateOnLastHandleClosed": true,
        "Container": container,
    })
    .to_string()
}

#[cfg(test)]
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
                    image_id: None,
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
        let parents = [malt_platform::isolation::layers::PreparedLayer {
            id: "layer-test".to_string(),
            path: std::path::PathBuf::from(r"C:\prepared\layer-test"),
        }];
        let json = hcs_config(
            "malt-session-7-8",
            "malt",
            r"C:\session-root",
            Some(512),
            &parents,
        );
        assert!(json.contains(r#""Owner":"malt-session-malt-session-7-8""#));
        assert!(json.contains(r#""Path":"C:\\session-root""#));
        assert!(json.contains(r#""SizeInMB":512"#));
        assert!(json.contains("layer-test"));
        assert!(json.contains(r#""SchemaVersion":{"Major":2,"Minor":0}"#));
        assert!(json.contains(r#""Id":"layer-test"#));
        assert!(!json.contains(r#""PathType""#));
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
                image_id: None,
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
                    image_id: None,
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
