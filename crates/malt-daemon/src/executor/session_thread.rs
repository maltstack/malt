use crate::bus::{Bus, BusConfig, BusMessage};
use crate::connection::authority::AuthorityTracker;
use crate::executor::command_worker::{
    spawn_command_worker, ExecutionCompletion, ExecutionIngress,
};
use crate::executor::events::{
    DeliveryOutcome, EventLog, GapReason, LifecycleEvent, LifecycleEventKind, SubscriberSink,
};
use crate::executor::input::{InputError, SessionInputChannel};
use crate::executor::output_log;
use crate::DaemonError;
use malt_compat::CompatTranslator;
use malt_layout::resolve::compute_resolved_panes;
use malt_layout::{LayoutConfig, Rect};
use malt_protocol::common::{
    ClientCapabilities, IsolationTier, PaneId, PaneKind, ResolvedPane, SessionId,
};
use malt_protocol::input::KeyEvent;
use malt_protocol::persist::session::{PersistedPane, PersistedPaneType, PersistedSession};
use malt_protocol::priority::Priority;
use malt_protocol::render::{InitialState, RenderBatch};
use malt_renderer::host::{PaneFrame, RendererHost};
use malt_session::pane::{CommandBlock, PaneRuntime, DEFAULT_MAX_BLOCKS};
use malt_session::session::SessionRuntime;
use malt_term::{EditMode, EditResult, Editor};
use mash::env::{Env, EnvSnapshot, Variable};
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{info, warn};

#[cfg(windows)]
#[derive(Debug)]
struct HcsProcessSpawner {
    session_id: SessionId,
    container_id: String,
}

#[cfg(windows)]
impl mash::env::ExternalProcessSpawner for HcsProcessSpawner {
    fn spawn(
        &self,
        config: malt_platform::process::SpawnConfig,
    ) -> Result<malt_platform::process::Child, malt_platform::process::SpawnError> {
        validate_hcs_stdio(&config)?;
        let request = hcs_process_request(&self.container_id, &config)?;
        let launch = crate::elevate_client::start_hcs_process(self.session_id.clone(), request)
            .map_err(malt_platform::process::SpawnError::Io)?;
        let mut child = malt_platform::process::child_from_hcs_process(
            launch.process_id,
            launch.process_handle,
            launch.stdin_handle,
            launch.stdout_handle,
            launch.stderr_handle,
        )?;
        install_hcs_stdio_relays(&mut child, config)?;
        Ok(child)
    }
}

#[cfg(windows)]
fn hcs_process_request(
    container_id: &str,
    config: &malt_platform::process::SpawnConfig,
) -> Result<malt_protocol::elevate::HcsProcessRequest, malt_platform::process::SpawnError> {
    use malt_protocol::elevate::HcsEnvironmentEntry;

    let unicode = |value: &std::ffi::OsStr, field: &str| {
        value.to_str().map(str::to_owned).ok_or_else(|| {
            malt_platform::process::SpawnError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("HCS process {field} is not valid Unicode"),
            ))
        })
    };
    Ok(malt_protocol::elevate::HcsProcessRequest {
        id: container_id.to_string(),
        program: config.program.to_str().map(str::to_owned).ok_or_else(|| {
            malt_platform::process::SpawnError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "HCS process program is not valid Unicode",
            ))
        })?,
        arguments: config
            .args
            .iter()
            .map(|argument| unicode(argument, "argument"))
            .collect::<Result<_, _>>()?,
        working_directory: config
            .cwd
            .as_ref()
            .map(|cwd| {
                cwd.to_str().map(str::to_owned).ok_or_else(|| {
                    malt_platform::process::SpawnError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "HCS process working directory is not valid Unicode",
                    ))
                })
            })
            .transpose()?,
        environment: config
            .env
            .iter()
            .map(|(key, value)| {
                Ok(HcsEnvironmentEntry {
                    key: unicode(key, "environment key")?,
                    value: unicode(value, "environment value")?,
                    _unknown: Vec::new(),
                })
            })
            .collect::<Result<_, malt_platform::process::SpawnError>>()?,
        argv0: config
            .argv0
            .as_ref()
            .map(|argv0| unicode(argv0, "argv0"))
            .transpose()?,
        _unknown: Vec::new(),
    })
}

#[cfg(windows)]
fn validate_hcs_stdio(
    config: &malt_platform::process::SpawnConfig,
) -> Result<(), malt_platform::process::SpawnError> {
    for (name, io) in [
        ("stdin", &config.stdin),
        ("stdout", &config.stdout),
        ("stderr", &config.stderr),
    ] {
        if matches!(io, malt_platform::process::Io::Handle(_)) {
            return Err(malt_platform::process::SpawnError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("HCS process {name} does not support a raw Win32 handle"),
            )));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn install_hcs_stdio_relays(
    child: &mut malt_platform::process::Child,
    config: malt_platform::process::SpawnConfig,
) -> Result<(), malt_platform::process::SpawnError> {
    use malt_platform::process::Io;

    match config.stdin {
        Io::Pipe => {}
        Io::File(mut source) => {
            let mut destination = child.take_stdin().ok_or_else(missing_hcs_stream)?;
            child.add_io_worker(thread::spawn(move || {
                copy_ignoring_broken_pipe(&mut source, &mut destination)
            }));
        }
        Io::Null => drop(child.take_stdin()),
        Io::Inherit => {
            let mut destination = child.take_stdin().ok_or_else(missing_hcs_stream)?;
            child.add_io_worker(thread::spawn(move || {
                let stdin = std::io::stdin();
                copy_ignoring_broken_pipe(&mut stdin.lock(), &mut destination)
            }));
        }
        Io::Handle(_) => unreachable!("validated before the helper launch"),
        _ => return Err(unsupported_hcs_stdio("stdin")),
    }
    install_hcs_output_relay(child, config.stdout, true)?;
    install_hcs_output_relay(child, config.stderr, false)
}

#[cfg(windows)]
fn install_hcs_output_relay(
    child: &mut malt_platform::process::Child,
    io: malt_platform::process::Io,
    stdout: bool,
) -> Result<(), malt_platform::process::SpawnError> {
    use malt_platform::process::Io;

    if matches!(io, Io::Pipe) {
        return Ok(());
    }
    let mut source: Box<dyn std::io::Read + Send> = if stdout {
        Box::new(child.take_stdout().ok_or_else(missing_hcs_stream)?)
    } else {
        Box::new(child.take_stderr().ok_or_else(missing_hcs_stream)?)
    };
    match io {
        Io::File(mut destination) => child.add_io_worker(thread::spawn(move || {
            copy_ignoring_broken_pipe(&mut source, &mut destination)
        })),
        Io::Null => child.add_io_worker(thread::spawn(move || {
            copy_ignoring_broken_pipe(&mut source, &mut std::io::sink())
        })),
        Io::Inherit if stdout => child.add_io_worker(thread::spawn(move || {
            let stdout = std::io::stdout();
            copy_ignoring_broken_pipe(&mut source, &mut stdout.lock())
        })),
        Io::Inherit => child.add_io_worker(thread::spawn(move || {
            let stderr = std::io::stderr();
            copy_ignoring_broken_pipe(&mut source, &mut stderr.lock())
        })),
        Io::Pipe => unreachable!("returned before taking the HCS stream"),
        Io::Handle(_) => unreachable!("validated before the helper launch"),
        _ => return Err(unsupported_hcs_stdio("output")),
    }
    Ok(())
}

#[cfg(windows)]
fn missing_hcs_stream() -> malt_platform::process::SpawnError {
    malt_platform::process::SpawnError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "helper reported an HCS launch without the required standard stream",
    ))
}

#[cfg(windows)]
fn unsupported_hcs_stdio(name: &str) -> malt_platform::process::SpawnError {
    malt_platform::process::SpawnError::Io(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        format!("HCS process {name} uses an unsupported standard-stream mode"),
    ))
}

#[cfg(windows)]
fn copy_ignoring_broken_pipe(
    reader: &mut dyn std::io::Read,
    writer: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    match std::io::copy(reader, writer) {
        Ok(_) => writer.flush(),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error),
    }
}

/// Placeholder Job Object resource caps for the Capped tier,
/// pending a real per-session/group configuration surface (see
/// `docs/BACKLOG.md`'s isolation-policy item). Deliberately conservative-but-
/// generous rather than tuned: the point of this pass is that Capped sessions
/// get a real, different-from-Restricted resource limit, not that these
/// specific numbers are load-bearing.
#[cfg(windows)]
const CAPPED_MEMORY_LIMIT_MB: u64 = 2048;
#[cfg(windows)]
const CAPPED_CPU_RATE_PERCENT: u32 = 200;

/// Job Object limits only for tiers whose declared mechanism is a Job Object.
/// `Contained` intentionally has no mapping: it denotes HCS and must be
/// refused until an HCS-aware MASH spawn path exists.
#[cfg(windows)]
fn job_object_limits_for_tier(isolation: IsolationTier) -> Option<(u64, u32)> {
    let platform_tier = match isolation {
        IsolationTier::Bare => malt_platform::isolation::IsolationTier::Bare,
        IsolationTier::Restricted => malt_platform::isolation::IsolationTier::Restricted,
        IsolationTier::Capped => malt_platform::isolation::IsolationTier::Capped,
        IsolationTier::Contained => malt_platform::isolation::IsolationTier::Contained,
    };
    match malt_platform::isolation::tier_requirements(platform_tier).mechanism {
        malt_platform::isolation::IsolationMechanism::JobObject => Some(match isolation {
            IsolationTier::Restricted => (0, 0),
            IsolationTier::Capped => (CAPPED_MEMORY_LIMIT_MB, CAPPED_CPU_RATE_PERCENT),
            _ => return None,
        }),
        _ => None,
    }
}

/// Milliseconds since the Unix epoch, saturating to 0 if the system clock is
/// before the epoch. Used for render staleness tracking and command history
/// timestamps.
pub(crate) fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Apply a session's isolation tier to its MASH environment: sets the opaque
/// isolation context token, and on Windows, creates a Job Object every
/// externally-spawned command in this session gets assigned to (see
/// `mash::executor`'s spawn call site). Failure is returned to the caller so
/// the required policy can refuse session creation before either thread is
/// made reachable.
///
/// Bare tier does nothing. Restricted gets an uncapped Job Object (group-kill
/// only), while Capped gets a Job Object with real memory/CPU limits. Contained
/// is not a Job Object tier and is handled by the helper/HCS branch below.
///
/// Contained is different: it receives an authenticated helper-owned HCS
/// compute system and an injected MASH spawner that asks that helper to create
/// every external command inside it. The context is established only after
/// both entitlement registration and compute-system creation report
/// `Performed`.
struct AppliedSessionIsolation {
    context: malt_platform::isolation::IsolationContext,
    selected_image: Option<String>,
}

fn apply_session_isolation(
    env: &mut Env,
    session_id: SessionId,
    isolation: IsolationTier,
    image_id: Option<String>,
) -> Result<AppliedSessionIsolation, DaemonError> {
    let context = malt_platform::isolation::IsolationContext::from(isolation);

    #[cfg(windows)]
    {
        if isolation == IsolationTier::Bare {
            env.set_isolation_context(context.clone());
            return Ok(AppliedSessionIsolation {
                context,
                selected_image: None,
            });
        }
        if isolation == IsolationTier::Contained {
            let storage_root = malt_config::paths::data_dir();
            crate::elevate_client::register_session_entitlement(
                session_id.clone(),
                &storage_root,
                &[std::process::id()],
            )
            .map_err(|error| {
                DaemonError::IsolationUnavailable(format!(
                    "contained session entitlement was not registered: {error}"
                ))
            })?;
            let response = crate::elevate_client::manage_hcs_container(
                session_id.clone(),
                None,
                Some(format!("malt-{}", session_id.0)),
                image_id,
            )
            .map_err(|error| {
                DaemonError::IsolationUnavailable(format!(
                    "contained HCS creation request did not complete: {error}"
                ))
            })?;
            if response.kind != malt_protocol::elevate::OutcomeKind::Performed {
                return Err(DaemonError::IsolationUnavailable(
                    response.detail.unwrap_or_else(|| {
                        "contained HCS creation was not performed by the helper".to_string()
                    }),
                ));
            }
            let payload = response.payload.ok_or_else(|| {
                DaemonError::IsolationUnavailable(
                    "helper performed contained HCS creation without a result payload".to_string(),
                )
            })?;
            let mut reader = malt_protocol::vexil_runtime::BitReader::new(&payload);
            let created = <malt_protocol::elevate::HcsContainerCreated as malt_protocol::vexil_runtime::Unpack>::unpack(&mut reader)
                .map_err(|error| {
                DaemonError::IsolationUnavailable(format!(
                    "helper returned an invalid contained HCS creation result: {error}"
                ))
            })?;
            if created.id.is_empty() {
                return Err(DaemonError::IsolationUnavailable(
                    "helper performed contained HCS creation without a container id".to_string(),
                ));
            }
            let container_id = created.id;
            context
                .establish_container(container_id.clone())
                .map_err(|error| DaemonError::IsolationUnavailable(error.to_string()))?;
            env.set_external_process_spawner(std::sync::Arc::new(HcsProcessSpawner {
                session_id,
                container_id,
            }));
            env.set_isolation_context(context.clone());
            return Ok(AppliedSessionIsolation {
                context,
                selected_image: created.selected_image,
            });
        }
        let (memory_limit_mb, cpu_rate) =
            job_object_limits_for_tier(isolation).ok_or_else(|| {
                DaemonError::IsolationUnavailable(format!(
                    "{isolation:?} is not provided by a Job Object on this platform"
                ))
            })?;
        let job_name = format!("malt-session-{}", session_id.0);
        let job = match malt_platform::isolation::job_objects::create_job_object(
            &job_name,
            memory_limit_mb,
            cpu_rate,
        ) {
            Ok(job) => job,
            Err(error) => return Err(DaemonError::IsolationUnavailable(error.to_string())),
        };
        malt_platform::isolation::job_objects::query_active_processes(&job).map_err(|error| {
            DaemonError::IsolationUnavailable(format!(
                "created Job Object could not be externally inspected: {error}"
            ))
        })?;
        context
            .establish_job_object(std::sync::Arc::new(job))
            .map_err(|error| DaemonError::IsolationUnavailable(error.to_string()))?;
        env.set_isolation_context(context.clone());
        Ok(AppliedSessionIsolation {
            context,
            selected_image: None,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = session_id;
        if isolation == IsolationTier::Bare {
            env.set_isolation_context(context.clone());
            Ok(AppliedSessionIsolation {
                context,
                selected_image: None,
            })
        } else {
            Err(DaemonError::IsolationUnavailable(
                "no session isolation backend is wired on this platform".to_string(),
            ))
        }
    }
}

/// Result of running a command line through mash: the captured stdout/stderr
/// text (what callers display) plus the command's exit code (what `$?`
/// would be).
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub command_id: u32,
    pub output: String,
    pub stderr: String,
    pub exit_code: i32,
    /// Whether `output` and/or `stderr` were cut short of the command's
    /// real output because they exceeded `EXEC_REPLY_CAP_BYTES` (research
    /// R3). Never left for the caller to infer -- a truncated reply that
    /// looks complete is exactly the failure this field exists to prevent.
    pub truncated: bool,
    /// How many bytes were left out of `output`/`stderr` combined when
    /// `truncated` is true; 0 otherwise.
    pub omitted_bytes: u64,
}

/// The control and worker handles created for an active session.  Keeping
/// these separate makes ownership explicit: the control thread owns UI,
/// persistence, and lifecycle state; the worker alone owns MASH state.
pub struct SessionSpawn {
    /// Shared with the MASH environment. Coordinator status reads this exact
    /// carrier instead of reconstructing a parallel mechanism report.
    pub isolation_context: malt_platform::isolation::IsolationContext,
    /// Immutable helper-owned image identity selected during contained setup.
    pub selected_image: Option<String>,
    pub control_tx: mpsc::SyncSender<SessionCommand>,
    pub ingress: ExecutionIngress,
    pub control_thread: JoinHandle<()>,
    pub worker_thread: JoinHandle<()>,
}

/// Bound on the session's control channel (`SessionCommand`, all variants).
///
/// Sized generously above ordinary control traffic so it is never the
/// binding constraint for anything except a command producing output far
/// faster than this actor can drain it -- that is precisely the case
/// `OutputChunk` (research R4) needs a bound for: the worker must block
/// rather than let an unbounded backlog of undelivered output grow the
/// daemon's memory without limit. Blocking the worker here is safe and
/// intentional -- the control actor is this session's own thread, always
/// draining, never the untrusted party a subscriber bound (`SUBSCRIBER_BUFFER`
/// in `events.rs`) exists to protect against.
const SESSION_CONTROL_CHANNEL_CAPACITY: usize = 1024;

/// Cap on `output`/`stderr` returned by the one-shot `/exec` reply
/// (`CommandOutput`, research R3).
///
/// A command's real output can be unbounded (SC-004's 100 MB case), but the
/// daemon must not accumulate an unbounded amount of it just to answer one
/// synchronous request. 1 MiB comfortably covers ordinary command output
/// (build logs, directory listings) without approaching a memory concern
/// per in-flight `/exec` call. Beyond the cap, the reply states that it was
/// truncated (`CommandOutput::truncated`/`omitted_bytes`) rather than
/// silently answering with an incomplete picture, and directs the caller to
/// the output stream for the rest.
const EXEC_REPLY_CAP_BYTES: usize = 1024 * 1024;

/// Truncate `output`/`stderr` to `cap` bytes each if they exceed it,
/// recording how much was cut. Truncates at a UTF-8 char boundary so the
/// result is never invalid, which is also why each stream is capped
/// independently rather than the pair sharing one combined budget -- there
/// is no single boundary that is valid for both strings at once.
fn cap_command_output(mut output: CommandOutput, cap: usize) -> CommandOutput {
    let mut omitted: u64 = 0;
    if output.output.len() > cap {
        let boundary = floor_char_boundary(&output.output, cap);
        omitted += (output.output.len() - boundary) as u64;
        output.output.truncate(boundary);
    }
    if output.stderr.len() > cap {
        let boundary = floor_char_boundary(&output.stderr, cap);
        omitted += (output.stderr.len() - boundary) as u64;
        output.stderr.truncate(boundary);
    }
    if omitted > 0 {
        output.truncated = true;
        output.omitted_bytes = omitted;
    }
    output
}

/// The largest byte index `<= index` that lands on a UTF-8 char boundary in
/// `s`. Stable `str::floor_char_boundary` is still nightly-only; this is the
/// same walk-back-at-most-3-bytes algorithm.
fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut boundary = index;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

/// A completed command awaiting commit.
///
/// No longer stages a compat-grid feed across multiple actor turns: a
/// top-level command's unredirected output already reached the live grid
/// incrementally as `SessionCommand::OutputChunk` arrived during execution
/// (research R7, US2). Re-feeding `output.output`/`output.stderr` here would
/// render the same content a second time. What remains to commit is cheap
/// (snapshot swap, history, lifecycle event, reply) and always finishes in
/// the same actor turn `begin_finalization` starts it -- there was never a
/// reason to spread that part across turns; only the old synchronous grid
/// feed was.
struct Finalization {
    sequence: u64,
    output: CommandOutput,
    snapshot: EnvSnapshot,
    reply: mpsc::Sender<Result<CommandOutput, DaemonError>>,
    finalized: mpsc::Sender<()>,
}

/// Something the session pushes to one attached client.
///
/// A single ordered stream per client rather than a channel per message kind:
/// a client must not learn it lost input authority *after* rendering frames
/// that arrived later, and two channels cannot promise that ordering.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ClientMessage {
    /// A frame to render.
    Render(RenderBatch),
    /// Input authority changed hands. Sent to every attached client, not just
    /// the two involved: a client that believes it can type when it cannot
    /// types into a void (FR-019).
    AuthorityChanged { holder: Option<u64> },
    /// A piece of a running command's output, as it was produced.
    ///
    /// A rendering client does not need this -- the same bytes already
    /// reached it via `Render`, fed through the compat grid. It exists for a
    /// client that wants the byte stream itself; one that ignores it still
    /// renders correctly (contracts/output-chunk-vnp.md). Carried on this
    /// same ordered stream, not a second channel, for the same reason
    /// `AuthorityChanged` is: a chunk arriving after the frame that already
    /// renders it would be incoherent.
    OutputChunk {
        sequence: u64,
        command_id: u32,
        stream: malt_protocol::shell::OutputStream,
        data: Vec<u8>,
        produced_at: u64,
    },
}

/// Who sent a piece of input.
///
/// Input has to be attributable before authority can mean anything: the
/// interactive path carried no client identity at all, which is the concrete
/// reason arbitration could not be enforced however complete the tracker was.
///
/// `Unattributed` is deliberately not a fake client id. The HTTP surface
/// authenticates with bearer tokens and has no per-connection identity, so
/// inventing one would put a lie in the type. It is accepted only while nobody
/// holds authority -- so an unattached session behaves exactly as before, and
/// an agent cannot use the HTTP door to type over a human holding the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputOrigin {
    /// An attached client, identified by its connection id.
    Client(u64),
    /// A caller with no per-connection identity, e.g. the HTTP surface.
    Unattributed,
}

/// Commands sent from the coordinator to a session executor.
///
/// Variant sizes differ substantially: `ExecutionCompleted` carries a full
/// shell snapshot while others carry a couple of integers. Boxing the large
/// ones would shrink the enum at the cost of an allocation on the hot
/// execution-completion path, for a message type that is moved once through a
/// channel and immediately destructured. Not worth it here.
#[allow(clippy::large_enum_variant)]
pub enum SessionCommand {
    /// Deliver a message to the session's bus.
    Deliver(BusMessage),
    /// Resize the terminal.
    Resize { cols: u16, rows: u16 },
    /// Raw bytes from PTY output (from reader thread).
    PtyOutput { pane_id: PaneId, data: Vec<u8> },
    /// Execute a command via mash (from exec_command API).
    RunCommand {
        command: String,
        reply: mpsc::Sender<CommandOutput>,
    },
    /// The sole MASH owner announcing that it has begun a request. Recorded
    /// as an open history block; the matching `ExecutionCompleted` finalizes
    /// it.
    ExecutionStarted {
        command_id: u32,
        command: String,
        started_at: u64,
    },
    /// A result sent by the sole MASH owner. It is committed on this actor
    /// before the worker may take another request.
    ExecutionCompleted(ExecutionCompletion),
    /// A piece of a running command's stdout/stderr, as it was produced.
    ///
    /// Sent on this same channel as `ExecutionStarted`/`ExecutionCompleted`
    /// -- not a separate channel -- so a command can never report finishing
    /// before its last chunk is delivered (research R4). The control actor
    /// assigns the session-wide sequence number on receipt; the worker does
    /// not know it.
    OutputChunk {
        command_id: u32,
        stream: malt_protocol::shell::OutputStream,
        data: Vec<u8>,
        produced_at: u64,
    },
    /// Write input to PTY stdin.
    WriteInput { data: Vec<u8> },
    /// Deliver raw bytes to whatever is reading this session's input.
    ///
    /// Deliberately separate from command submission. Routing a prompt answer
    /// through `run_mash_command` would give it a command id, a persisted
    /// history entry, and a published lifecycle event carrying its text --
    /// and prompt answers are routinely passwords.
    RawInput {
        origin: InputOrigin,
        data: Vec<u8>,
        reply: mpsc::Sender<Result<(), InputError>>,
    },
    /// Signal end-of-input to whatever is currently reading -- Ctrl-D.
    ///
    /// Ends the current read, not the session: a fresh input pipe is put in
    /// place so the next command can still be given input. Without this a
    /// command that consumes to the end (`cat`, `wc`) would never terminate,
    /// because a session's stdin has no natural end.
    EndOfInput {
        reply: mpsc::Sender<Result<(), InputError>>,
    },
    /// Get the current output snapshot as styled-grid JSON (requester sends
    /// back via channel). Built for human rendering clients
    /// (`malt-tui`/`maltty`) — for a program-readable variant see
    /// `GetOutputText`.
    GetOutput { reply: mpsc::Sender<String> },
    /// Get the current output snapshot as plain text, no styling — built
    /// for programmatic/agent consumption. Same underlying grid as
    /// `GetOutput`, different rendering.
    GetOutputText { reply: mpsc::Sender<String> },
    /// Subscribe to this session's lifecycle events. `resume_from` replays
    /// everything after that position, preceded by a gap if the position
    /// predates the retained window.
    SubscribeEvents {
        resume_from: Option<u64>,
        reply: mpsc::Sender<tokio::sync::mpsc::Receiver<LifecycleEvent>>,
    },
    /// Subscribe to this session's streamed command output. Same resume
    /// semantics as `SubscribeEvents`, over the separate byte-retention
    /// window in `output_log` (data-model.md: the log is not the terminal
    /// grid).
    SubscribeOutput {
        resume_from: Option<u64>,
        reply: mpsc::Sender<tokio::sync::mpsc::Receiver<output_log::OutputEvent>>,
    },
    /// Report which client holds input authority, if any (FR-015).
    GetInputAuthority { reply: mpsc::Sender<Option<u64>> },
    /// Claim input authority for an attached client (FR-016).
    ///
    /// Succeeds immediately; the previous holder is told it no longer holds.
    /// Consent is deliberately not required -- an unresponsive or departed
    /// holder would otherwise strand the session, the exact failure FR-018
    /// exists to prevent.
    ClaimInputAuthority {
        client_id: u64,
        authority: malt_protocol::common::InputAuthority,
        reply: mpsc::Sender<Result<Option<u64>, InputError>>,
    },
    /// Get this session's command execution history, oldest first.
    GetCommandHistory {
        reply: mpsc::Sender<Vec<CommandBlock>>,
    },
    /// Register a VNP client with this session's renderer.
    RegisterVnpClient {
        client_id: u64,
        /// What the client asked for when it attached. Previously parsed off
        /// the wire and discarded, which is why every client could type.
        authority: malt_protocol::common::InputAuthority,
        capabilities: ClientCapabilities,
        render_tx: SyncSender<ClientMessage>,
        initial_reply: mpsc::Sender<InitialState>,
    },
    /// Remove a VNP client from this session's renderer.
    UnregisterVnpClient { client_id: u64 },
    /// A typed keyboard event from a VNP client.
    KeyInput { origin: InputOrigin, key: KeyEvent },
    /// A frame acknowledgement from a VNP client.
    AckFrame { client_id: u64, frame_seq: u64 },
    /// Take a snapshot of the current session state for persistence.
    /// The reply channel receives a `PersistedSession` built from current env + layout.
    Snapshot {
        reply: mpsc::Sender<PersistedSession>,
        name: Option<String>,
        isolation: IsolationTier,
        selected_image: Option<String>,
    },
    /// Decide whether the last client may transition this session to Dormant.
    /// Because this command is ordered behind all earlier control events, it
    /// observes editor/input admission and finalization without racing them.
    PrepareDormancy {
        reply: mpsc::Sender<Option<PersistedSession>>,
        name: Option<String>,
        isolation: IsolationTier,
        selected_image: Option<String>,
    },
    /// Graceful shutdown.
    Shutdown,
}

// Manual Debug impl since mpsc::Sender doesn't implement Debug
impl std::fmt::Debug for SessionCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Deliver(m) => f.debug_tuple("Deliver").field(m).finish(),
            Self::Resize { cols, rows } => f
                .debug_struct("Resize")
                .field("cols", cols)
                .field("rows", rows)
                .finish(),
            Self::PtyOutput { pane_id, data } => f
                .debug_struct("PtyOutput")
                .field("pane_id", pane_id)
                .field("len", &data.len())
                .finish(),
            Self::RunCommand { command, .. } => f
                .debug_struct("RunCommand")
                .field("command", command)
                .finish(),
            Self::ExecutionStarted { command_id, .. } => f
                .debug_struct("ExecutionStarted")
                .field("command_id", command_id)
                .finish(),
            Self::ExecutionCompleted(_) => f.debug_struct("ExecutionCompleted").finish(),
            Self::OutputChunk {
                command_id,
                stream,
                data,
                ..
            } => f
                .debug_struct("OutputChunk")
                .field("command_id", command_id)
                .field("stream", stream)
                .field("len", &data.len())
                .finish(),
            Self::RawInput { data, .. } => f
                .debug_struct("RawInput")
                .field("bytes", &data.len())
                .finish(),
            Self::EndOfInput { .. } => f.debug_struct("EndOfInput").finish(),
            Self::WriteInput { data } => f
                .debug_struct("WriteInput")
                .field("len", &data.len())
                .finish(),
            Self::GetOutput { .. } => f.debug_struct("GetOutput").finish(),
            Self::GetOutputText { .. } => f.debug_struct("GetOutputText").finish(),
            Self::SubscribeEvents { resume_from, .. } => f
                .debug_struct("SubscribeEvents")
                .field("resume_from", resume_from)
                .finish(),
            Self::SubscribeOutput { resume_from, .. } => f
                .debug_struct("SubscribeOutput")
                .field("resume_from", resume_from)
                .finish(),
            Self::GetInputAuthority { .. } => f.debug_struct("GetInputAuthority").finish(),
            Self::ClaimInputAuthority { client_id, .. } => f
                .debug_struct("ClaimInputAuthority")
                .field("client_id", client_id)
                .finish(),
            Self::GetCommandHistory { .. } => f.debug_struct("GetCommandHistory").finish(),
            Self::RegisterVnpClient { client_id, .. } => f
                .debug_struct("RegisterVnpClient")
                .field("client_id", client_id)
                .finish(),
            Self::UnregisterVnpClient { client_id } => f
                .debug_struct("UnregisterVnpClient")
                .field("client_id", client_id)
                .finish(),
            Self::KeyInput { .. } => f.debug_struct("KeyInput").finish(),
            Self::AckFrame {
                client_id,
                frame_seq,
            } => f
                .debug_struct("AckFrame")
                .field("client_id", client_id)
                .field("frame_seq", frame_seq)
                .finish(),
            Self::Snapshot { .. } => write!(f, "Snapshot"),
            Self::PrepareDormancy { .. } => write!(f, "PrepareDormancy"),
            Self::Shutdown => write!(f, "Shutdown"),
        }
    }
}

/// Per-session executor running on a dedicated thread.
pub struct SessionExecutor {
    session: SessionRuntime,
    bus: Bus,
    authority: AuthorityTracker,
    terminal_size: Rect,
    layout_config: LayoutConfig,
    compat: Option<CompatTranslator>,
    /// Full shell state as of the last finalized command. The worker's live
    /// Env is never shared or read by this actor.
    env_snapshot: EnvSnapshot,
    ingress: ExecutionIngress,
    renderer: RendererHost,
    editor: Editor,
    render_pushers: HashMap<u64, SyncSender<ClientMessage>>,
    resolved_panes: Vec<ResolvedPane>,
    finalization: Option<Finalization>,
    expected_completion_sequence: u64,
    /// Command execution history for this session's first (shell) pane.
    ///
    /// The worker announces each execution as it starts (`ExecutionStarted`)
    /// and this actor pushes an *open* block; the matching
    /// `ExecutionCompleted` finalizes it. Recording the start separately is
    /// what keeps a running command visible in history and what leaves an
    /// honestly-unfinished record if the daemon stops mid-command.
    pane_runtime: PaneRuntime,
    /// Where raw client input goes. Its read end is registered at fd 0 in
    /// the worker's `mash::Env`, which is what makes `read` take input from
    /// the session instead of the daemon's own console.
    input_channel: SessionInputChannel,
    /// Bounded catch-up window for reconnecting subscribers, and the
    /// authority on this session's event sequence numbers.
    event_log: EventLog,
    /// Live subscribers. Each has an independent position and its own bound;
    /// none may block the others or this actor.
    event_sinks: Vec<SubscriberSink>,
    next_subscriber_id: u64,
    /// Bounded-by-bytes catch-up window for this session's streamed command
    /// output, and the authority on output sequence numbers. Not the
    /// terminal grid -- the grid is a rendered screen with its own bounded
    /// scrollback; this is the byte stream both it and output subscribers
    /// are fed from (data-model.md).
    output_log: output_log::OutputLog,
    /// Live output subscribers. Same non-blocking, lag-reporting policy as
    /// `event_sinks`, deliberately -- see `output_log`'s module doc.
    output_sinks: Vec<output_log::OutputSubscriberSink>,
    next_output_subscriber_id: u64,
    /// Per-subscriber channel depth. Always `output_log::SUBSCRIBER_BUFFER`
    /// in production; overridable only through
    /// `spawn_with_capacity_and_output_bound` so a test can force a
    /// subscriber to lag without publishing hundreds of real chunks.
    output_subscriber_buffer: usize,
}

impl SessionExecutor {
    /// Spawn a new session executor on a dedicated thread.
    /// Returns the command sender and thread handle.
    pub fn spawn(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
    ) -> Result<(mpsc::SyncSender<SessionCommand>, JoinHandle<()>), DaemonError> {
        let spawned =
            Self::spawn_with_capacity(session_id, first_pane, isolation, 256, Vec::new())?;
        Ok((spawned.control_tx, spawned.control_thread))
    }

    /// Start the split control/worker architecture with an explicit bounded
    /// pending-execution capacity.
    pub fn spawn_with_capacity(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
        capacity: usize,
        command_blocks: Vec<CommandBlock>,
    ) -> Result<SessionSpawn, DaemonError> {
        let mut env = Env::from_os();
        env.set_interactive(true);
        let applied_isolation =
            apply_session_isolation(&mut env, session_id.clone(), isolation, None)?;
        Self::spawn_with_env(
            session_id,
            first_pane,
            isolation,
            capacity,
            env,
            command_blocks,
            output_log::MAX_RETAINED_BYTES,
            output_log::SUBSCRIBER_BUFFER,
            applied_isolation.context,
            applied_isolation.selected_image,
        )
    }

    pub fn spawn_with_capacity_and_image(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
        image_id: Option<String>,
        capacity: usize,
        command_blocks: Vec<CommandBlock>,
    ) -> Result<SessionSpawn, DaemonError> {
        let mut env = Env::from_os();
        env.set_interactive(true);
        let applied_isolation =
            apply_session_isolation(&mut env, session_id.clone(), isolation, image_id)?;
        Self::spawn_with_env(
            session_id,
            first_pane,
            isolation,
            capacity,
            env,
            command_blocks,
            output_log::MAX_RETAINED_BYTES,
            output_log::SUBSCRIBER_BUFFER,
            applied_isolation.context,
            applied_isolation.selected_image,
        )
    }

    /// As [`SessionExecutor::spawn_with_capacity`], but also overrides the
    /// output log's byte-retention bound and per-subscriber channel depth.
    /// Test-support only: production callers always want
    /// [`output_log::MAX_RETAINED_BYTES`]/[`output_log::SUBSCRIBER_BUFFER`],
    /// but a test exercising retention eviction or subscriber lag should not
    /// have to generate megabytes of real output or hundreds of real chunks
    /// to do it (SC-004/SC-005 "in miniature").
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_with_capacity_and_output_bound(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
        capacity: usize,
        output_log_capacity_bytes: usize,
        output_subscriber_buffer: usize,
        command_blocks: Vec<CommandBlock>,
    ) -> Result<SessionSpawn, DaemonError> {
        let mut env = Env::from_os();
        env.set_interactive(true);
        let applied_isolation =
            apply_session_isolation(&mut env, session_id.clone(), isolation, None)?;
        Self::spawn_with_env(
            session_id,
            first_pane,
            isolation,
            capacity,
            env,
            command_blocks,
            output_log_capacity_bytes,
            output_subscriber_buffer,
            applied_isolation.context,
            applied_isolation.selected_image,
        )
    }

    /// Spawn a new session executor on a dedicated thread, setting the working
    /// directory and restoring shell state from a prior session.
    ///
    /// If `initial_cwd` is not a directory at spawn time, a warning is logged
    /// and the OS-inherited cwd is used instead. `shell_path`, if present, is
    /// restored into the `SHELL` env var (previously computed and persisted
    /// but never read back — see `docs/BACKLOG.md`). `env_snapshot`, if
    /// present, restores exported/non-exported variables, options, aliases,
    /// functions, directory stack, and traps via `Env::apply_snapshot` —
    /// this is the actual fix for "session state silently dropped on
    /// restart," not just the two scalars (`SHELL`/`PWD`) this function
    /// already handled.
    pub fn spawn_with_cwd(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
        initial_cwd: std::path::PathBuf,
        shell_path: Option<String>,
        env_snapshot: Option<mash::env::EnvSnapshot>,
        command_blocks: Vec<CommandBlock>,
    ) -> Result<(mpsc::SyncSender<SessionCommand>, JoinHandle<()>), DaemonError> {
        let spawned = Self::spawn_with_cwd_and_capacity(
            session_id,
            first_pane,
            isolation,
            initial_cwd,
            shell_path,
            env_snapshot,
            256,
            command_blocks,
        )?;
        Ok((spawned.control_tx, spawned.control_thread))
    }

    /// As with `spawn_with_env`, each parameter is distinct construction
    /// state rather than an accidental accumulation.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_with_cwd_and_capacity(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
        initial_cwd: std::path::PathBuf,
        shell_path: Option<String>,
        env_snapshot: Option<EnvSnapshot>,
        capacity: usize,
        command_blocks: Vec<CommandBlock>,
    ) -> Result<SessionSpawn, DaemonError> {
        Self::spawn_with_cwd_and_capacity_and_image(
            session_id,
            first_pane,
            isolation,
            initial_cwd,
            shell_path,
            env_snapshot,
            None,
            capacity,
            command_blocks,
        )
    }

    /// Restore a session with the immutable image identity selected by the
    /// helper before it was persisted. This never re-resolves a mutable image
    /// reference on restore.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_with_cwd_and_capacity_and_image(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
        initial_cwd: std::path::PathBuf,
        shell_path: Option<String>,
        env_snapshot: Option<EnvSnapshot>,
        image_id: Option<String>,
        capacity: usize,
        command_blocks: Vec<CommandBlock>,
    ) -> Result<SessionSpawn, DaemonError> {
        let mut env = Env::from_os();
        env.set_interactive(true);
        let applied_isolation =
            apply_session_isolation(&mut env, session_id.clone(), isolation, image_id)?;
        if let Some(snapshot) = &env_snapshot {
            env.apply_snapshot(snapshot);
        }
        if let Some(shell_path) = shell_path {
            if let Err(error) = env.set_global("SHELL", Variable::exported_string(shell_path)) {
                warn!(%error, "spawn_with_cwd: failed to restore SHELL in mash env");
            }
        }
        if initial_cwd.is_dir() {
            let cwd = initial_cwd.to_string_lossy().to_string();
            if let Err(error) = env.set_global("PWD", Variable::exported_string(cwd)) {
                warn!(%error, "spawn_with_cwd: failed to restore PWD in mash env");
            }
        } else {
            warn!(
                ?initial_cwd,
                "spawn_with_cwd: directory no longer exists; falling back to OS cwd"
            );
        }
        Self::spawn_with_env(
            session_id,
            first_pane,
            isolation,
            capacity,
            env,
            command_blocks,
            output_log::MAX_RETAINED_BYTES,
            output_log::SUBSCRIBER_BUFFER,
            applied_isolation.context,
            applied_isolation.selected_image,
        )
    }

    /// Every parameter here is distinct session-construction state; bundling
    /// them into a struct purely to satisfy an argument count would add a type
    /// that exists for the linter rather than the reader.
    #[allow(clippy::too_many_arguments)]
    fn spawn_with_env(
        session_id: SessionId,
        first_pane: PaneId,
        isolation: IsolationTier,
        capacity: usize,
        env: Env,
        command_blocks: Vec<CommandBlock>,
        output_log_capacity_bytes: usize,
        output_subscriber_buffer: usize,
        isolation_context: malt_platform::isolation::IsolationContext,
        selected_image: Option<String>,
    ) -> Result<SessionSpawn, DaemonError> {
        let env = env;
        let snapshot = env.to_snapshot();
        let pane_cwd = env.get_str("PWD").to_string();
        // Resume the id sequence past everything already recorded so restored
        // history and new executions cannot collide on command_id. Derived
        // from the history itself rather than persisted separately -- a second
        // source of truth could only ever disagree with it. The worker owns id
        // assignment, so it is seeded here rather than on the control actor.
        let start_command_id = command_blocks
            .iter()
            .map(|block| block.command_id)
            .max()
            .unwrap_or(0);
        let pane_runtime = PaneRuntime::with_blocks(
            first_pane.clone(),
            PaneKind::Shell,
            pane_cwd,
            DEFAULT_MAX_BLOCKS,
            command_blocks,
        );
        // Create the session's input channel and register its read end at fd
        // 0 *before* `env` moves into the worker. `mash`'s `read` builtin
        // resolves `env.open_fd_read(0)` before falling back to
        // `std::io::stdin()`, so this single registration both routes client
        // input to the builtin and makes the fall-through to the daemon's own
        // console unreachable -- with no change to `mash`.
        // The channel is handed the env's descriptor table so that an
        // end-of-input signal can swap fd 0 for a fresh pipe. It cannot go
        // through `env`, which moves into the worker thread below and is
        // unreachable while a command is running -- which is precisely when a
        // client sends EOF.
        let (input_channel, input_read) =
            SessionInputChannel::new(session_id.0, env.fd_registry()).map_err(DaemonError::Io)?;
        env.register_fd(0, input_read);

        let (control_tx, control_rx) = mpsc::sync_channel(SESSION_CONTROL_CHANNEL_CAPACITY);
        let (ingress, requests) = ExecutionIngress::new(session_id.clone(), capacity)?;
        let worker_thread = spawn_command_worker(
            session_id.clone(),
            env,
            requests,
            control_tx.clone(),
            ingress.clone(),
            start_command_id,
        )?;
        let control_ingress = ingress.clone();
        let control_thread = match thread::Builder::new()
            .name(format!("session-control-{}", session_id.0))
            .spawn(move || {
                let mut executor = SessionExecutor {
                    session: SessionRuntime::new(session_id, first_pane, isolation),
                    bus: Bus::new(BusConfig::default()),
                    authority: AuthorityTracker::new(),
                    terminal_size: Rect::new(0, 0, 80, 24),
                    layout_config: LayoutConfig::default(),
                    compat: None,
                    env_snapshot: snapshot,
                    ingress: control_ingress,
                    renderer: RendererHost::new(),
                    editor: Editor::new(EditMode::Emacs),
                    render_pushers: HashMap::new(),
                    resolved_panes: Vec::new(),
                    finalization: None,
                    expected_completion_sequence: 1,
                    pane_runtime,
                    input_channel,
                    event_log: EventLog::new(),
                    event_sinks: Vec::new(),
                    next_subscriber_id: 1,
                    output_log: output_log::OutputLog::with_capacity(output_log_capacity_bytes),
                    output_sinks: Vec::new(),
                    next_output_subscriber_id: 1,
                    output_subscriber_buffer,
                };
                executor.run(control_rx);
            }) {
            Ok(control_thread) => control_thread,
            Err(error) => {
                // The worker owns the environment, including any Job Object.
                // If the second thread cannot start, explicitly close its
                // admission channel and wait for it to drop that environment
                // before reporting failure. Returning immediately here used
                // to strand a worker (and its isolation resource) even though
                // no SessionHandle was inserted.
                ingress.close();
                let _ = control_tx.send(SessionCommand::Shutdown);
                let _ = worker_thread.join();
                return Err(DaemonError::Io(error));
            }
        };
        Ok(SessionSpawn {
            isolation_context,
            selected_image,
            control_tx,
            ingress,
            control_thread,
            worker_thread,
        })
    }

    /// Initialize the compat translator for this session.
    pub fn init_compat(&mut self, cols: u16, rows: u16) {
        self.compat = Some(CompatTranslator::new(cols, rows));
    }

    fn run(&mut self, rx: mpsc::Receiver<SessionCommand>) {
        info!(session = ?self.session.id(), "session executor started");
        // Initialize compat translator with default terminal size
        self.init_compat(self.terminal_size.w, self.terminal_size.h);
        // Compute initial layout so resolved_panes is populated before any render.
        self.recompute_layout();

        loop {
            let next = if self.finalization.is_some() {
                rx.recv_timeout(Duration::from_millis(1))
            } else {
                rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
            };
            match next {
                Ok(SessionCommand::Shutdown) => {
                    info!(session = ?self.session.id(), "session executor shutting down");
                    self.ingress.close();
                    break;
                }
                Ok(SessionCommand::Deliver(msg)) => {
                    self.bus.publish(msg);
                }
                Ok(SessionCommand::Resize { cols, rows }) => {
                    self.terminal_size = Rect::new(0, 0, cols, rows);
                    if let Some(compat) = &mut self.compat {
                        compat.resize(cols, rows);
                    }
                    self.recompute_layout();
                }
                Ok(SessionCommand::RunCommand { command, reply }) => {
                    self.submit_legacy_command(command, reply);
                }
                Ok(SessionCommand::ExecutionStarted {
                    command_id,
                    command,
                    started_at,
                }) => {
                    self.pane_runtime.push_command_block(CommandBlock {
                        command_id,
                        cmd: command.clone(),
                        started_at,
                        finished_at: None,
                        exit_code: None,
                    });
                    self.publish_lifecycle(LifecycleEventKind::CommandStarted {
                        command_id,
                        cmd: command,
                        started_at,
                    });
                }
                Ok(SessionCommand::ExecutionCompleted(completion)) => {
                    self.begin_finalization(completion)
                }
                Ok(SessionCommand::OutputChunk {
                    command_id,
                    stream,
                    data,
                    produced_at,
                }) => {
                    // Fed to the live grid as it arrives -- this is what
                    // makes an attached client's view update during a
                    // command instead of only at its end (US2, research
                    // R7). `advance_finalization` no longer re-feeds this
                    // same content at completion; see its doc.
                    if let Some(compat) = &mut self.compat {
                        compat.feed(&data);
                    }
                    let sequence = self.publish_output(output_log::OutputEventKind::Chunk {
                        command_id,
                        stream,
                        data: data.clone(),
                        produced_at,
                    });
                    self.dispatch_render();
                    for tx in self.render_pushers.values() {
                        // Non-blocking, consistent with `Render`: a client
                        // too slow to keep up is already handled by the
                        // renderer's lag/shed logic, and this adds no
                        // second mechanism (contracts/output-chunk-vnp.md).
                        let _ = tx.try_send(ClientMessage::OutputChunk {
                            sequence,
                            command_id,
                            stream,
                            data: data.clone(),
                            produced_at,
                        });
                    }
                }
                Ok(SessionCommand::PtyOutput { data, .. }) => {
                    if let Some(compat) = &mut self.compat {
                        compat.feed(&data);
                    }
                    // Publish output to bus for subscribers
                    self.bus.publish(BusMessage {
                        domain: 1,   // Shell
                        msg_type: 4, // OutputChunk
                        priority: Priority::Normal,
                        producer_id: 0,
                        payload: data,
                    });
                    self.dispatch_render();
                }
                Ok(SessionCommand::RawInput {
                    origin,
                    data,
                    reply,
                }) => {
                    let _ = reply.send(match self.authority_error(origin) {
                        // Refused, never silently dropped: to a client, silence
                        // is indistinguishable from a dead connection (FR-014).
                        Some(denied) => Err(denied),
                        None => self.input_channel.try_write(&data),
                    });
                }
                Ok(SessionCommand::EndOfInput { reply }) => {
                    let _ = reply.send(self.input_channel.end_of_input());
                }
                Ok(SessionCommand::WriteInput { data }) => {
                    // Raw bytes to the session's input, not a command to run.
                    //
                    // This used to decode lossily, trim, and submit the result
                    // as a top-level command line. Each step corrupted input a
                    // different way -- from_utf8_lossy mangles non-text bytes,
                    // trim destroys whitespace a password may contain, and the
                    // empty check discarded a bare newline, which is exactly
                    // the byte a confirmation prompt waits for.
                    if let Err(e) = self.input_channel.try_write(&data) {
                        warn!(session = ?self.session.id(), error = %e, "raw input refused");
                    }
                }
                Ok(SessionCommand::GetOutput { reply }) => {
                    let output = self.get_grid_output();
                    let _ = reply.send(output);
                }
                Ok(SessionCommand::GetOutputText { reply }) => {
                    let output = self.get_plain_text_output();
                    let _ = reply.send(output);
                }
                Ok(SessionCommand::SubscribeEvents { resume_from, reply }) => {
                    let rx = self.subscribe_events(resume_from);
                    // If the requester vanished before we replied, the
                    // receiver drops and the sink is reaped on next publish.
                    let _ = reply.send(rx);
                }
                Ok(SessionCommand::SubscribeOutput { resume_from, reply }) => {
                    let rx = self.subscribe_output(resume_from);
                    let _ = reply.send(rx);
                }
                Ok(SessionCommand::GetCommandHistory { reply }) => {
                    let history: Vec<CommandBlock> =
                        self.pane_runtime.command_blocks().iter().cloned().collect();
                    let _ = reply.send(history);
                }
                Ok(SessionCommand::GetInputAuthority { reply }) => {
                    let _ = reply.send(self.authority.holder());
                }
                Ok(SessionCommand::ClaimInputAuthority {
                    client_id,
                    authority,
                    reply,
                }) => {
                    let previous = self.authority.holder();
                    if !self.render_pushers.contains_key(&client_id) {
                        // Only an attached client can hold the keyboard;
                        // otherwise a departed one could take it and strand
                        // the session it is no longer listening to.
                        let _ = reply.send(Err(InputError::NotAttached { client_id }));
                        continue;
                    }
                    self.authority.claim(client_id, authority);
                    let now = self.authority.holder();
                    let _ = reply.send(Ok(now));
                    // Re-claiming what you already hold changes nothing, so
                    // it notifies nobody (FR-019 is about actual changes).
                    if now != previous {
                        self.broadcast_authority();
                    }
                }
                Ok(SessionCommand::RegisterVnpClient {
                    client_id,
                    authority,
                    capabilities,
                    render_tx,
                    initial_reply,
                }) => {
                    // Attaching and acquiring authority are one event on the
                    // real path. They used to be separate commands and only
                    // the unused one told the tracker, so the tracker looked
                    // wired while production never reached it.
                    self.authority.attach(client_id, authority);
                    let _ = self.session.attach(client_id, authority);
                    self.renderer.register_client(client_id, capabilities);
                    self.render_pushers.insert(client_id, render_tx);
                    let element = match &self.compat {
                        Some(c) => c.frame_element(),
                        None => malt_protocol::frame_element::FrameElement::VtPassthrough {
                            data: Vec::new(),
                        },
                    };
                    let pane_id = self.session.focused_pane().clone();
                    let panes = vec![PaneFrame { pane_id, element }];
                    let layout = self.resolved_panes.clone();
                    let initial = self
                        .renderer
                        .snapshot_initial_state(&panes, &layout, client_id);
                    if initial_reply.send(initial).is_err() {
                        warn!(
                            client_id,
                            "RegisterVnpClient: initial_reply receiver dropped before send"
                        );
                    }
                }
                Ok(SessionCommand::UnregisterVnpClient { client_id }) => {
                    // Releases authority so a holder that vanished cannot
                    // strand the session (FR-017/FR-018).
                    let previous = self.authority.holder();
                    self.authority.detach(client_id);
                    let _ = self.session.detach(client_id);
                    self.renderer.remove_client(client_id);
                    self.render_pushers.remove(&client_id);
                    if self.authority.holder() != previous {
                        self.broadcast_authority();
                    }
                }
                Ok(SessionCommand::KeyInput { origin, key }) => {
                    if self.authority_error(origin).is_some() {
                        // An observer's keystrokes must not reach the session.
                        // Its output and events are untouched (FR-020).
                        continue;
                    }
                    if let Some(input_event) = crate::input_bridge::vnp_key_to_input_event(&key) {
                        match self.editor.feed(input_event) {
                            EditResult::Accept(line) => {
                                self.submit_discarded_command(line);
                                self.editor.reset();
                                self.dispatch_render();
                            }
                            EditResult::Interrupt => {
                                self.editor.reset();
                                if let Some(compat) = &mut self.compat {
                                    compat.feed(b"^C\r\n");
                                }
                                self.dispatch_render();
                            }
                            EditResult::Eof | EditResult::Suspend => {
                                self.editor.reset();
                                self.dispatch_render();
                            }
                            EditResult::Continue => {}
                            // EditResult is #[non_exhaustive]; handle future variants gracefully.
                            _ => {}
                        }
                    }
                }
                Ok(SessionCommand::AckFrame {
                    client_id,
                    frame_seq,
                }) => {
                    self.renderer.ack_frame(client_id, frame_seq);
                }
                Ok(SessionCommand::Snapshot {
                    reply,
                    name,
                    isolation,
                    selected_image,
                }) => {
                    let persisted = build_persisted_session(
                        self.session.id(),
                        self.session.focused_pane(),
                        name.as_deref(),
                        isolation,
                        selected_image.as_deref(),
                        &self.env_snapshot,
                        self.pane_runtime.command_blocks(),
                    );
                    let _ = reply.send(persisted);
                }
                Ok(SessionCommand::PrepareDormancy {
                    reply,
                    name,
                    isolation,
                    selected_image,
                }) => {
                    // This mailbox barrier is deliberately evaluated on the
                    // control actor. All earlier editor/input commands have
                    // already attempted ingress admission, so an accepted
                    // request is visible through `is_idle`; finalization is
                    // actor-owned and cannot be observed halfway through.
                    let persisted = if self.finalization.is_none() && self.ingress.is_idle() {
                        Some(build_persisted_session(
                            self.session.id(),
                            self.session.focused_pane(),
                            name.as_deref(),
                            isolation,
                            selected_image.as_deref(),
                            &self.env_snapshot,
                            self.pane_runtime.command_blocks(),
                        ))
                    } else {
                        None
                    };
                    let _ = reply.send(persisted);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    warn!(session = ?self.session.id(), "command channel closed");
                    break;
                }
            }
            self.advance_finalization();
        }
    }

    /// Record a lifecycle event and fan it out to every live subscriber.
    ///
    /// Never blocks: delivery is `try_send` only, so a subscriber that has
    /// stopped reading cannot add latency to command execution. A subscriber
    /// that is full or gone is told what it missed (best-effort) and dropped
    /// — never accommodated by growing its channel.
    fn publish_lifecycle(&mut self, kind: LifecycleEventKind) {
        let event = self.event_log.publish(kind);
        let latest = event.sequence;
        let mut dropped: Vec<u64> = Vec::new();
        for sink in &mut self.event_sinks {
            match sink.try_deliver(&event) {
                DeliveryOutcome::Delivered => {}
                DeliveryOutcome::Lagged => {
                    sink.try_notify_gap(latest, GapReason::SubscriberLagged);
                    dropped.push(sink.id);
                }
                DeliveryOutcome::Closed => dropped.push(sink.id),
            }
        }
        if !dropped.is_empty() {
            self.event_sinks.retain(|s| !dropped.contains(&s.id));
            for id in dropped {
                info!(session = ?self.session.id(), subscriber = id, "event subscriber removed");
            }
        }
    }

    /// Register a subscriber, replaying from `resume_from` if given.
    ///
    /// If the requested position predates what is still retained, a `Gap` is
    /// queued *before* the replayed events, so the client learns about the
    /// hole before receiving data that would otherwise look contiguous.
    fn subscribe_events(
        &mut self,
        resume_from: Option<u64>,
    ) -> tokio::sync::mpsc::Receiver<LifecycleEvent> {
        let id = self.next_subscriber_id;
        self.next_subscriber_id += 1;
        let (mut sink, rx) = SubscriberSink::new(id);

        if let Some(from) = resume_from {
            // The client already holds everything through `from`; without
            // this the gap below would claim it missed those events too.
            sink.set_position(from);
            let (replay, lost) = self.event_log.replay_after(from);
            if lost {
                let through = self
                    .event_log
                    .oldest_sequence()
                    .map(|o| o.saturating_sub(1))
                    .unwrap_or(from);
                sink.try_notify_gap(through, GapReason::RetentionExceeded);
            }
            for event in replay {
                if sink.try_deliver(&event) != DeliveryOutcome::Delivered {
                    // The replay itself outran the buffer -- say so rather
                    // than hand over a silently-truncated backlog.
                    sink.try_notify_gap(
                        self.event_log.latest_sequence(),
                        GapReason::SubscriberLagged,
                    );
                    break;
                }
            }
        }

        self.event_sinks.push(sink);
        rx
    }

    /// Record an output chunk (or, in principle, a gap -- callers never
    /// pass one) and fan it out to every live output subscriber. Mirrors
    /// `publish_lifecycle` exactly; see that method's doc.
    /// Returns the assigned sequence, so a caller that also needs to notify
    /// attached VNP clients (`ClientMessage::OutputChunk`) uses the same
    /// number the output stream resumes from, rather than a second,
    /// possibly-disagreeing counter.
    fn publish_output(&mut self, kind: output_log::OutputEventKind) -> u64 {
        let event = self.output_log.publish(kind);
        let latest = event.sequence;
        let mut dropped: Vec<u64> = Vec::new();
        for sink in &mut self.output_sinks {
            match sink.try_deliver(&event) {
                output_log::DeliveryOutcome::Delivered => {}
                output_log::DeliveryOutcome::Lagged => {
                    sink.try_notify_gap(latest, output_log::GapReason::SubscriberLagged);
                    dropped.push(sink.id);
                }
                output_log::DeliveryOutcome::Closed => dropped.push(sink.id),
            }
        }
        if !dropped.is_empty() {
            self.output_sinks.retain(|s| !dropped.contains(&s.id));
            for id in dropped {
                info!(session = ?self.session.id(), subscriber = id, "output subscriber removed");
            }
        }
        latest
    }

    /// Register an output subscriber, replaying from `resume_from` if given.
    /// Mirrors `subscribe_events` exactly; see that method's doc.
    fn subscribe_output(
        &mut self,
        resume_from: Option<u64>,
    ) -> tokio::sync::mpsc::Receiver<output_log::OutputEvent> {
        let id = self.next_output_subscriber_id;
        self.next_output_subscriber_id += 1;
        let (mut sink, rx) =
            output_log::OutputSubscriberSink::with_buffer(id, self.output_subscriber_buffer);

        if let Some(from) = resume_from {
            sink.set_position(from);
            let (replay, lost) = self.output_log.replay_after(from);
            if lost {
                let through = self
                    .output_log
                    .oldest_sequence()
                    .map(|o| o.saturating_sub(1))
                    .unwrap_or(from);
                sink.try_notify_gap(through, output_log::GapReason::RetentionExceeded);
            }
            for event in replay {
                if sink.try_deliver(&event) != output_log::DeliveryOutcome::Delivered {
                    sink.try_notify_gap(
                        self.output_log.latest_sequence(),
                        output_log::GapReason::SubscriberLagged,
                    );
                    break;
                }
            }
        }

        self.output_sinks.push(sink);
        rx
    }

    /// Dispatch a render frame to all registered VNP clients.
    /// Whether `origin` may send input right now, and why not if it may not.
    ///
    /// A session nobody holds is open to anyone -- that is what keeps it from
    /// becoming permanently unanswerable (FR-018), and it is why sessions with
    /// no attached client behave exactly as they did before authority existed.
    fn authority_error(&self, origin: InputOrigin) -> Option<InputError> {
        let holder = self.authority.holder()?;
        match origin {
            InputOrigin::Client(id) if id == holder => None,
            InputOrigin::Client(id) => Some(InputError::NotAuthority {
                client_id: Some(id),
                holder,
            }),
            InputOrigin::Unattributed => Some(InputError::NotAuthority {
                client_id: None,
                holder,
            }),
        }
    }

    /// Tell every attached client who holds input authority now (FR-019).
    ///
    /// Sent to all of them, not only the gaining and losing pair: a client
    /// that still believes it holds the keyboard will type into a void, which
    /// is the failure this exists to prevent.
    fn broadcast_authority(&mut self) {
        let holder = self.authority.holder();
        for tx in self.render_pushers.values() {
            // Non-blocking, consistent with frame delivery: a client too far
            // behind to accept this is already being shed.
            let _ = tx.try_send(ClientMessage::AuthorityChanged { holder });
        }
    }

    fn dispatch_render(&mut self) {
        let now_ms = now_epoch_ms();
        for stale_id in self.renderer.shed_stale_clients(now_ms) {
            // Dropping the sender here closes the VNP listener's render_rx,
            // which its main loop already treats as a disconnect signal
            // (see vnp_listener.rs's TryRecvError::Disconnected branch).
            warn!(
                client_id = stale_id,
                "shedding VNP client: no FrameAck in 10s"
            );
            self.render_pushers.remove(&stale_id);
        }

        if self.render_pushers.is_empty() {
            return;
        }
        let element = match &self.compat {
            Some(c) => c.frame_element(),
            // When compat is None, no content exists yet — skip the render dispatch.
            // Note: RegisterVnpClient sends an empty VtPassthrough frame in this case,
            // because the initial snapshot is required to complete the handshake regardless.
            None => return,
        };
        let pane_id = self.session.focused_pane().clone();
        let panes = vec![PaneFrame { pane_id, element }];
        let layout = self.resolved_panes.clone();
        let batches = self.renderer.process_frame(&panes, &layout);
        for crb in batches {
            if let Some(tx) = self.render_pushers.get(&crb.client_id) {
                // Non-blocking: drop if channel full (client lagging)
                let _ = tx.try_send(ClientMessage::Render(crb.batch));
            }
        }
    }

    fn submit_legacy_command(&mut self, command: String, reply: mpsc::Sender<CommandOutput>) {
        let (worker_reply, worker_result) = mpsc::channel();
        match self.ingress.submit_one_shot(command, worker_reply) {
            Ok(()) => {
                // A legacy SessionCommand caller expects a plain CommandOutput.
                // Keep that compatibility without making the control actor wait.
                let _ = thread::Builder::new()
                    .name("session-command-reply".to_string())
                    .spawn(move || match worker_result.recv() {
                        Ok(Ok(output)) => {
                            let _ = reply.send(output);
                        }
                        Ok(Err(error)) => {
                            let _ = reply.send(command_error_output(error));
                        }
                        Err(_) => {
                            let _ = reply.send(command_error_output(
                                DaemonError::ExecutionUnavailable(
                                    malt_protocol::common::SessionId(0),
                                ),
                            ));
                        }
                    });
            }
            Err(error) => {
                self.append_execution_diagnostic(&error);
                let _ = reply.send(command_error_output(error));
            }
        }
    }

    fn submit_discarded_command(&mut self, command: String) {
        let (reply, _result) = mpsc::channel::<Result<CommandOutput, DaemonError>>();
        if let Err(error) = self.ingress.submit(command, reply) {
            self.append_execution_diagnostic(&error);
        }
    }

    fn begin_finalization(&mut self, completion: ExecutionCompletion) {
        match completion.result {
            Ok(worker_output) => {
                if worker_output.sequence != self.expected_completion_sequence {
                    let error = DaemonError::ExecutionUnavailable(self.session.id().clone());
                    self.ingress.mark_unavailable();
                    let _ = completion.reply.send(Err(error));
                    let _ = completion.finalized.send(());
                    return;
                }
                self.finalization = Some(Finalization {
                    sequence: worker_output.sequence,
                    output: worker_output.result,
                    snapshot: worker_output.env_snapshot,
                    reply: completion.reply,
                    finalized: completion.finalized,
                });
            }
            Err(error) => {
                self.append_execution_diagnostic(&error);
                let _ = completion.reply.send(Err(error));
                let _ = completion.finalized.send(());
            }
        }
    }

    /// Commit a completed result: swap in the shell snapshot, finalize the
    /// history block, publish the lifecycle event, and reply.
    ///
    /// Used to also re-feed `output.output`/`output.stderr` into the compat
    /// grid here, spread across turns to keep the actor responsive during a
    /// large synchronous feed. That feed is no longer needed: the same
    /// bytes already reached the live grid incrementally as
    /// `SessionCommand::OutputChunk` arrived during execution (US2). Doing
    /// it again here would render the command's output twice.
    fn advance_finalization(&mut self) {
        let Some(finalization) = self.finalization.take() else {
            return;
        };
        if !finalization.output.output.is_empty() {
            self.bus.publish(BusMessage {
                domain: 1,
                msg_type: 4,
                priority: Priority::Normal,
                producer_id: 0,
                payload: finalization.output.output.as_bytes().to_vec(),
            });
        }
        self.env_snapshot = finalization.snapshot;
        let finished_at = now_epoch_ms();
        // Duration comes from the matching open history block rather than a
        // second clock read, so the event and the history entry cannot
        // disagree about when the command ran.
        let started_at = self
            .pane_runtime
            .current_block()
            .filter(|b| b.command_id == finalization.output.command_id)
            .map(|b| b.started_at);
        self.pane_runtime
            .finalize_current_block(finished_at, finalization.output.exit_code);
        self.expected_completion_sequence = finalization.sequence.saturating_add(1);
        // Published here -- at the commit point, not when the worker returned
        // -- so a finish event never arrives before the output that produced
        // it is visible to a subsequent get_output (research R9).
        self.publish_lifecycle(LifecycleEventKind::CommandFinished {
            command_id: finalization.output.command_id,
            exit_code: finalization.output.exit_code,
            finished_at,
            duration_us: started_at
                .map(|s| finished_at.saturating_sub(s).saturating_mul(1_000))
                .unwrap_or(0),
        });
        self.dispatch_render();
        // Capped here, at the reply -- everything above (bus, compat feed,
        // history) already saw the command's real, complete output. Nothing
        // accumulates unboundedly *before* this point either: the stream
        // (`output_log`, capped separately) is where an agent following a
        // large command should look; this reply exists for the common case
        // of a short command, and states plainly when it is not that case.
        let _ = finalization.reply.send(Ok(cap_command_output(
            finalization.output,
            EXEC_REPLY_CAP_BYTES,
        )));
        // The acknowledgement is deliberately last: only a completely
        // materialized view and reply let the worker begin its next request.
        let _ = finalization.finalized.send(());
    }

    fn append_execution_diagnostic(&mut self, error: &DaemonError) {
        let text = format!("malt: {error}\n");
        if let Some(compat) = &mut self.compat {
            compat.feed(text.as_bytes());
        }
        self.bus.publish(BusMessage {
            domain: 1,
            msg_type: 4,
            priority: Priority::Normal,
            producer_id: 0,
            payload: text.into_bytes(),
        });
        self.dispatch_render();
    }

    /// Extract the grid as plain text, no styling — for programmatic/agent
    /// consumption. Same underlying `TerminalGrid` as `get_grid_output`,
    /// just characters instead of styled spans.
    fn get_plain_text_output(&self) -> String {
        let Some(compat) = &self.compat else {
            return String::new();
        };
        let grid = compat.grid();
        let mut lines: Vec<String> = grid
            .rows_data()
            .iter()
            .map(|row| row.cells.iter().map(|cell| cell.ch).collect::<String>())
            .collect();

        // Trim trailing blank lines, matching get_grid_output's equivalent
        // trim of trailing empty rows.
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }

        lines.join("\n")
    }

    fn recompute_layout(&mut self) {
        self.resolved_panes = compute_resolved_panes(
            self.session.layout(),
            self.terminal_size,
            self.session.focused_pane().clone(),
            &self.layout_config,
        );
    }

    /// Extract the grid as styled JSON for the API.
    fn get_grid_output(&self) -> String {
        let Some(compat) = &self.compat else {
            return "[]".to_string();
        };
        let grid = compat.grid();
        let mut rows_json = Vec::new();

        for row in grid.rows_data() {
            let mut spans = Vec::new();
            let mut current_text = String::new();
            let mut current_fg = (0u8, 0u8, 0u8);
            let mut current_bg = (0u8, 0u8, 0u8);
            let mut current_bold = false;
            let mut first = true;

            for cell in &row.cells {
                let fg = cell.style.fg;
                let bg = cell.style.bg;
                let bold = cell.style.bold;

                if first {
                    current_fg = fg;
                    current_bg = bg;
                    current_bold = bold;
                    first = false;
                }

                if fg != current_fg || bg != current_bg || bold != current_bold {
                    // Flush current span
                    if !current_text.is_empty() {
                        spans.push(serde_json::json!({
                            "t": current_text,
                            "fg": [current_fg.0, current_fg.1, current_fg.2],
                            "bg": [current_bg.0, current_bg.1, current_bg.2],
                            "b": current_bold,
                        }));
                    }
                    current_text = String::new();
                    current_fg = fg;
                    current_bg = bg;
                    current_bold = bold;
                }
                current_text.push(cell.ch);
            }
            // Flush last span
            if !current_text.is_empty() {
                spans.push(serde_json::json!({
                    "t": current_text,
                    "fg": [current_fg.0, current_fg.1, current_fg.2],
                    "bg": [current_bg.0, current_bg.1, current_bg.2],
                    "b": current_bold,
                }));
            }
            rows_json.push(serde_json::Value::Array(spans));
        }

        // Trim trailing empty rows
        while rows_json.last().is_some_and(|r| {
            r.as_array().is_none_or(|a| {
                a.len() == 1
                    && a[0]
                        .get("t")
                        .and_then(|t| t.as_str())
                        .is_some_and(|s| s.trim().is_empty())
            })
        }) {
            rows_json.pop();
        }

        serde_json::to_string(&rows_json).unwrap_or_else(|_| "[]".to_string())
    }
}

/// Convert a live `CommandBlock` to its persisted shape. Lossless — see
/// `from_persisted_command_block` for the reverse direction.
pub(crate) fn to_persisted_command_block(
    block: &CommandBlock,
) -> malt_protocol::persist::session::PersistedCommandBlock {
    malt_protocol::persist::session::PersistedCommandBlock {
        command_id: block.command_id,
        cmd: block.cmd.clone(),
        started_at: block.started_at,
        finished_at: block.finished_at,
        exit_code: block.exit_code,
        _unknown: Vec::new(),
    }
}

/// Convert a persisted command record back to its live shape. A record whose
/// `finished_at`/`exit_code` are absent stays absent — a command interrupted
/// by a daemon stop is never reinterpreted as complete.
pub(crate) fn from_persisted_command_block(
    block: &malt_protocol::persist::session::PersistedCommandBlock,
) -> CommandBlock {
    CommandBlock {
        command_id: block.command_id,
        cmd: block.cmd.clone(),
        started_at: block.started_at,
        finished_at: block.finished_at,
        exit_code: block.exit_code,
    }
}

/// Convert mash's live `EnvSnapshot` to the schema-generated persisted
/// shape. See `from_persisted_env_snapshot` for the reverse direction.
fn to_persisted_env_snapshot(
    snapshot: &mash::env::EnvSnapshot,
) -> malt_protocol::persist::session::EnvSnapshot {
    use malt_protocol::persist::session::{
        EnvSnapshot as PEnvSnapshot, PersistedShellOptions, PersistedVarValue, PersistedVariable,
    };

    let variables = snapshot
        .variables
        .iter()
        .map(|(name, var)| {
            let value = match &var.value {
                mash::env::VarValue::String(s) => PersistedVarValue::Str { value: s.clone() },
                mash::env::VarValue::Integer(i, formatted) => PersistedVarValue::Int {
                    value: *i,
                    formatted: formatted.clone(),
                },
                mash::env::VarValue::Array(items) => {
                    PersistedVarValue::Arr { values: items.clone() }
                }
                mash::env::VarValue::AssocArray(map) => PersistedVarValue::AssocArr {
                    entries: map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                },
                // VarValue is #[non_exhaustive] in mash -- a future variant
                // this daemon build doesn't know about yet. Best-effort:
                // persist it as an empty string rather than failing the
                // whole snapshot over one unrecognized variable.
                _ => {
                    warn!(
                        variable = %name,
                        "unrecognized VarValue variant while snapshotting env; persisting as empty string"
                    );
                    PersistedVarValue::Str {
                        value: String::new(),
                    }
                }
            };
            (
                name.clone(),
                PersistedVariable {
                    value,
                    exported: var.exported,
                    readonly: var.readonly,
                    integer: var.integer,
                    _unknown: vec![],
                },
            )
        })
        .collect();

    let options = PersistedShellOptions {
        allexport: snapshot.options.allexport,
        errexit: snapshot.options.errexit,
        nounset: snapshot.options.nounset,
        pipefail: snapshot.options.pipefail,
        xtrace: snapshot.options.xtrace,
        verbose: snapshot.options.verbose,
        noglob: snapshot.options.noglob,
        notify: snapshot.options.notify,
        monitor: snapshot.options.monitor,
        noclobber: snapshot.options.noclobber,
        noexec: snapshot.options.noexec,
        nonlexicalctrl: snapshot.options.nonlexicalctrl,
        hash_cmds: snapshot.options.hash_cmds,
        nolog: snapshot.options.nolog,
        sourcepath: snapshot.options.sourcepath,
        _unknown: vec![],
    };

    PEnvSnapshot {
        variables,
        options,
        aliases: snapshot
            .aliases
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        functions: snapshot
            .functions
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        dir_stack: snapshot.dir_stack.clone(),
        cwd: snapshot.cwd.clone(),
        traps: snapshot
            .traps
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        _unknown: vec![],
    }
}

/// Convert a schema-generated persisted `EnvSnapshot` back to mash's live
/// shape, for `Env::apply_snapshot`. Reverse of `to_persisted_env_snapshot`.
pub(crate) fn from_persisted_env_snapshot(
    persisted: &malt_protocol::persist::session::EnvSnapshot,
) -> mash::env::EnvSnapshot {
    use malt_protocol::persist::session::PersistedVarValue;

    let variables = persisted
        .variables
        .iter()
        .map(|(name, pv)| {
            let value = match &pv.value {
                PersistedVarValue::Str { value } => mash::env::VarValue::String(value.clone()),
                PersistedVarValue::Int { value, formatted } => {
                    mash::env::VarValue::Integer(*value, formatted.clone())
                }
                PersistedVarValue::Arr { values } => mash::env::VarValue::Array(values.clone()),
                PersistedVarValue::AssocArr { entries } => mash::env::VarValue::AssocArray(
                    entries
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                ),
                // PersistedVarValue is #[non_exhaustive] too (includes the
                // schema's own forward-compat `Unknown` variant) -- treat
                // anything unrecognized as an empty string rather than
                // failing the whole restore over one variable.
                _ => mash::env::VarValue::String(String::new()),
            };
            (
                name.clone(),
                mash::env::Variable {
                    value,
                    exported: pv.exported,
                    readonly: pv.readonly,
                    integer: pv.integer,
                },
            )
        })
        .collect();

    let options = mash::env::ShellOptions {
        allexport: persisted.options.allexport,
        errexit: persisted.options.errexit,
        nounset: persisted.options.nounset,
        pipefail: persisted.options.pipefail,
        xtrace: persisted.options.xtrace,
        verbose: persisted.options.verbose,
        noglob: persisted.options.noglob,
        notify: persisted.options.notify,
        monitor: persisted.options.monitor,
        noclobber: persisted.options.noclobber,
        noexec: persisted.options.noexec,
        nonlexicalctrl: persisted.options.nonlexicalctrl,
        hash_cmds: persisted.options.hash_cmds,
        nolog: persisted.options.nolog,
        sourcepath: persisted.options.sourcepath,
    };

    mash::env::EnvSnapshot {
        variables,
        options,
        aliases: persisted
            .aliases
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        functions: persisted
            .functions
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        dir_stack: persisted.dir_stack.clone(),
        cwd: persisted.cwd.clone(),
        traps: persisted
            .traps
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}

/// Build a `PersistedSession` from current runtime state.
///
/// Called from the `Snapshot` command handler — runs on the session thread,
/// so all access to `env` is unsynchronized by design.
fn command_error_output(error: DaemonError) -> CommandOutput {
    CommandOutput {
        command_id: 0,
        output: String::new(),
        stderr: format!("malt: {error}\n"),
        exit_code: 1,
        truncated: false,
        omitted_bytes: 0,
    }
}

fn build_persisted_session(
    session_id: &SessionId,
    focused_pane: &PaneId,
    name: Option<&str>,
    isolation: IsolationTier,
    selected_image: Option<&str>,
    env: &EnvSnapshot,
    command_blocks: &VecDeque<CommandBlock>,
) -> PersistedSession {
    let shell_path = snapshot_string(env, "SHELL").unwrap_or_else(default_shell_path);
    let cwd = snapshot_string(env, "PWD").unwrap_or_else(|| {
        if env.cwd.is_empty() {
            ".".to_string()
        } else {
            env.cwd.clone()
        }
    });
    let env_snapshot = Some(to_persisted_env_snapshot(env));

    let pane = PersistedPane {
        cwd,
        title: None,
        pane_type: PersistedPaneType::Shell {
            shell_path,
            env_snapshot,
        },
        command_blocks: command_blocks
            .iter()
            .map(to_persisted_command_block)
            .collect(),
        _unknown: vec![],
    };

    let mut panes = std::collections::BTreeMap::new();
    panes.insert(focused_pane.0, pane);

    // NOTE: Layout is hardcoded as a single Leaf node — this is correct for
    // the current single-pane model. Phase F multi-pane will need to read
    // the actual layout state from the session executor and pass it here.
    PersistedSession {
        schema_version: 1,
        id: session_id.clone(),
        name: name.map(|s| s.to_string()),
        layout: malt_protocol::common::LayoutNode::Leaf {
            pane_id: focused_pane.clone(),
        },
        focus: focused_pane.clone(),
        panes,
        theme: None,
        group: None,
        isolation,
        selected_image: selected_image.map(str::to_owned),
        _unknown: vec![],
    }
}

fn snapshot_string(snapshot: &EnvSnapshot, name: &str) -> Option<String> {
    match snapshot.variables.get(name).map(|variable| &variable.value) {
        Some(mash::env::VarValue::String(value)) => Some(value.clone()),
        Some(mash::env::VarValue::Integer(value, _)) => Some(value.to_string()),
        _ => None,
    }
}

fn default_shell_path() -> String {
    #[cfg(unix)]
    {
        "/bin/sh".to_string()
    }
    #[cfg(not(unix))]
    {
        "cmd.exe".to_string()
    }
}

#[cfg(all(test, windows))]
mod isolation_tier_tests {
    use super::*;

    #[test]
    fn only_job_object_tiers_have_job_object_limits() {
        assert_eq!(job_object_limits_for_tier(IsolationTier::Bare), None);
        assert_eq!(
            job_object_limits_for_tier(IsolationTier::Restricted),
            Some((0, 0)),
            "Restricted should be group-kill only, no resource caps"
        );
    }

    #[test]
    fn capped_is_limited_and_contained_is_never_relabelled_as_a_job() {
        let capped = job_object_limits_for_tier(IsolationTier::Capped).unwrap();
        assert_ne!(
            capped,
            (0, 0),
            "Capped must get a real resource limit, not the same uncapped \
             treatment as Restricted -- that was the confirmed tier-blind bug"
        );
        assert_eq!(job_object_limits_for_tier(IsolationTier::Contained), None);
    }

    #[test]
    fn restricted_and_capped_are_actually_different() {
        assert_ne!(
            job_object_limits_for_tier(IsolationTier::Restricted),
            job_object_limits_for_tier(IsolationTier::Capped),
            "this is the core tier-blindness fix: Restricted and Capped must \
             no longer produce identical Job Object parameters"
        );
    }
}
