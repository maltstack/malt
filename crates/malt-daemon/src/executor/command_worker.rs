//! Single-owner MASH execution worker and bounded admission ingress.
//!
//! The control actor never borrows the worker's [`mash::env::Env`].  It only
//! receives a completed result plus a full snapshot, finalizes that boundary,
//! and then acknowledges it.  That acknowledgement is the ordering barrier
//! which prevents the next command from observing half-finalized state.

use super::session_thread::{now_epoch_ms, CommandOutput, SessionCommand};
use crate::DaemonError;
use malt_protocol::common::SessionId;
use mash::env::Env;
use mash::executor::execute_list;
use mash::parser;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};

const ACCEPTING: u8 = 0;
const CLOSING: u8 = 1;
const UNAVAILABLE: u8 = 2;

/// Reply type for externally submitted execution. The control actor sends it
/// only after it has made the result and shell snapshot authoritative.
pub type ExecutionReply = mpsc::Sender<Result<CommandOutput, DaemonError>>;

/// A request that has been admitted to a session-local FIFO.
#[derive(Debug)]
pub struct ExecutionRequest {
    pub sequence: u64,
    pub command: String,
    pub reply: ExecutionReply,
}

/// Result produced by the sole MASH owner before the control actor commits it.
#[derive(Debug)]
pub struct WorkerOutput {
    pub sequence: u64,
    pub result: CommandOutput,
    pub env_snapshot: mash::env::EnvSnapshot,
}

/// Worker-to-control transfer. The worker must wait for `finalized` before it
/// reads its next request, preserving FIFO state handoff.
#[derive(Debug)]
pub struct ExecutionCompletion {
    pub result: Result<WorkerOutput, DaemonError>,
    pub reply: ExecutionReply,
    pub finalized: mpsc::Sender<()>,
}

struct AdmissionState {
    session_id: SessionId,
    capacity: usize,
    state: AtomicU8,
    pending: AtomicUsize,
    active: AtomicBool,
    next_sequence: AtomicU64,
    sender: Mutex<Option<mpsc::SyncSender<ExecutionRequest>>>,
}

/// Cloneable, nonblocking submission handle for one session's execution FIFO.
#[derive(Clone)]
pub struct ExecutionIngress {
    state: Arc<AdmissionState>,
}

impl std::fmt::Debug for ExecutionIngress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionIngress")
            .field("session_id", &self.state.session_id)
            .field("capacity", &self.state.capacity)
            .finish_non_exhaustive()
    }
}

impl ExecutionIngress {
    pub fn new(
        session_id: SessionId,
        capacity: usize,
    ) -> Result<(Self, mpsc::Receiver<ExecutionRequest>), DaemonError> {
        if capacity == 0 {
            return Err(DaemonError::InvalidPoolConfig(
                "session_channel_size must be greater than zero".to_string(),
            ));
        }
        let (sender, receiver) = mpsc::sync_channel(capacity);
        Ok((
            Self {
                state: Arc::new(AdmissionState {
                    session_id,
                    capacity,
                    state: AtomicU8::new(ACCEPTING),
                    pending: AtomicUsize::new(0),
                    active: AtomicBool::new(false),
                    next_sequence: AtomicU64::new(1),
                    sender: Mutex::new(Some(sender)),
                }),
            },
            receiver,
        ))
    }

    pub fn capacity(&self) -> usize {
        self.state.capacity
    }

    pub fn submit(&self, command: String, reply: ExecutionReply) -> Result<(), DaemonError> {
        match self.state.state.load(Ordering::Acquire) {
            CLOSING => return Err(DaemonError::SessionShuttingDown(self.state.session_id.clone())),
            UNAVAILABLE => return Err(DaemonError::ExecutionUnavailable(self.state.session_id.clone())),
            _ => {}
        }

        let sender = self
            .state
            .sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or_else(|| DaemonError::ExecutionUnavailable(self.state.session_id.clone()))?;
        let sequence = self.state.next_sequence.fetch_add(1, Ordering::AcqRel);
        self.state.pending.fetch_add(1, Ordering::AcqRel);
        match sender.try_send(ExecutionRequest {
            sequence,
            command,
            reply,
        }) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => {
                self.state.pending.fetch_sub(1, Ordering::AcqRel);
                Err(DaemonError::ExecutionQueueFull {
                session_id: self.state.session_id.clone(),
                capacity: self.state.capacity,
                })
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.state.pending.fetch_sub(1, Ordering::AcqRel);
                self.mark_unavailable();
                Err(DaemonError::ExecutionUnavailable(self.state.session_id.clone()))
            }
        }
    }

    pub fn close(&self) {
        self.state.state.store(CLOSING, Ordering::Release);
        let mut sender = self
            .state
            .sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sender.take();
    }

    pub fn mark_unavailable(&self) {
        self.state.state.store(UNAVAILABLE, Ordering::Release);
        let mut sender = self
            .state
            .sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sender.take();
    }

    pub fn is_idle(&self) -> bool {
        self.state.pending.load(Ordering::Acquire) == 0
            && !self.state.active.load(Ordering::Acquire)
    }

    fn is_closing(&self) -> bool {
        self.state.state.load(Ordering::Acquire) == CLOSING
    }

    fn worker_received(&self) {
        self.state.pending.fetch_sub(1, Ordering::AcqRel);
    }

    fn set_active(&self, active: bool) {
        self.state.active.store(active, Ordering::Release);
    }
}

/// Spawn the sole owner of this session's MASH state.
///
/// `start_command_id` seeds the execution id sequence. It is the highest id
/// already present in restored history (0 for a fresh session), so ids stay
/// unique within a pane's history across a daemon restart instead of
/// restarting at 1 and colliding with persisted records.
pub fn spawn_command_worker(
    session_id: SessionId,
    mut env: Env,
    requests: mpsc::Receiver<ExecutionRequest>,
    control_tx: mpsc::Sender<SessionCommand>,
    ingress: ExecutionIngress,
    start_command_id: u32,
) -> Result<JoinHandle<()>, DaemonError> {
    thread::Builder::new()
        .name(format!("session-exec-{}", session_id.0))
        .spawn(move || {
            let mut next_command_id = start_command_id;
            while let Ok(request) = requests.recv() {
                ingress.worker_received();
                if ingress.is_closing() {
                    let _ = request.reply.send(Err(DaemonError::SessionShuttingDown(session_id.clone())));
                    continue;
                }
                ingress.set_active(true);
                next_command_id = next_command_id.saturating_add(1);
                // Announce the start before running, so a command that is
                // still executing is visible in history, and one interrupted
                // by a daemon stop leaves an honestly-unfinished record. The
                // control actor owns the history buffer; the worker owns
                // MASH -- neither reaches into the other.
                if control_tx
                    .send(SessionCommand::ExecutionStarted {
                        command_id: next_command_id,
                        command: request.command.clone(),
                        started_at: now_epoch_ms(),
                    })
                    .is_err()
                {
                    ingress.mark_unavailable();
                    break;
                }
                let result = catch_unwind(AssertUnwindSafe(|| {
                    run_command(&mut env, &request.command, next_command_id)
                }))
                .map(|result| WorkerOutput {
                    sequence: request.sequence,
                    result,
                    env_snapshot: env.to_snapshot(),
                })
                .map_err(|_| DaemonError::ExecutionUnavailable(session_id.clone()));

                let worker_failed = result.is_err();
                if worker_failed {
                    ingress.mark_unavailable();
                }
                let (finalized_tx, finalized_rx) = mpsc::channel();
                if control_tx
                    .send(SessionCommand::ExecutionCompleted(ExecutionCompletion {
                        result,
                        reply: request.reply,
                        finalized: finalized_tx,
                    }))
                    .is_err()
                {
                    ingress.mark_unavailable();
                    break;
                }
                if finalized_rx.recv().is_err() {
                    ingress.mark_unavailable();
                    break;
                }
                ingress.set_active(false);
                if worker_failed {
                    while let Ok(pending) = requests.try_recv() {
                        ingress.worker_received();
                        let _ = pending
                            .reply
                            .send(Err(DaemonError::ExecutionUnavailable(session_id.clone())));
                    }
                    break;
                }
            }
        })
        .map_err(DaemonError::Io)
}

fn run_command(env: &mut Env, input: &str, command_id: u32) -> CommandOutput {
    #[cfg(test)]
    if input == "__malt_test_injected_worker_panic" {
        panic!("injected command-worker panic");
    }
    let commands = match parser::parse(input) {
        Ok(commands) => commands,
        Err(error) => {
            return CommandOutput {
                command_id,
                output: format!("mash: parse error: {error}\n"),
                stderr: String::new(),
                exit_code: 1,
            }
        }
    };
    if commands.is_empty() {
        return CommandOutput {
            command_id,
            output: String::new(),
            stderr: String::new(),
            exit_code: 0,
        };
    }
    let result = execute_list(&commands, input, env);
    CommandOutput {
        command_id,
        output: String::from_utf8_lossy(&result.stdout).to_string(),
        stderr: String::from_utf8_lossy(&result.stderr).to_string(),
        exit_code: result.exit_code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn mash_env_can_move_to_the_single_owner_worker() {
        assert_send::<Env>();
    }

    #[test]
    fn zero_capacity_is_rejected_without_a_channel_panic() {
        let error = ExecutionIngress::new(SessionId(1), 0).unwrap_err();
        assert!(matches!(error, DaemonError::InvalidPoolConfig(_)));
    }

    #[test]
    fn ingress_rejects_full_queue_without_blocking() {
        let (ingress, _receiver) = ExecutionIngress::new(SessionId(7), 1).unwrap();
        let (first, _first_result) = mpsc::channel();
        ingress.submit("echo first".to_string(), first).unwrap();
        let (second, _second_result) = mpsc::channel();
        let error = ingress.submit("echo second".to_string(), second).unwrap_err();
        assert!(matches!(error, DaemonError::ExecutionQueueFull { capacity: 1, .. }));
    }

    #[test]
    fn one_thousand_admissions_have_strict_fifo_sequences() {
        let (ingress, receiver) = ExecutionIngress::new(SessionId(11), 1_000).unwrap();
        for index in 0..1_000 {
            let (reply, _result) = mpsc::channel();
            ingress.submit(format!("echo {index}"), reply).unwrap();
        }
        for expected in 1..=1_000 {
            assert_eq!(receiver.recv().unwrap().sequence, expected);
        }
    }

    #[test]
    fn closing_and_unavailable_ingress_have_distinct_errors() {
        let (closing, _receiver) = ExecutionIngress::new(SessionId(8), 1).unwrap();
        closing.close();
        let (reply, _result) = mpsc::channel();
        assert!(matches!(
            closing.submit("echo nope".to_string(), reply),
            Err(DaemonError::SessionShuttingDown(_))
        ));

        let (unavailable, _receiver) = ExecutionIngress::new(SessionId(9), 1).unwrap();
        unavailable.mark_unavailable();
        let (reply, _result) = mpsc::channel();
        assert!(matches!(
            unavailable.submit("echo nope".to_string(), reply),
            Err(DaemonError::ExecutionUnavailable(_))
        ));
    }

    #[test]
    fn worker_panic_fails_active_and_pending_work_then_closes_admission() {
        let session_id = SessionId(10);
        let (ingress, requests) = ExecutionIngress::new(session_id.clone(), 2).unwrap();
        let (control_tx, control_rx) = mpsc::channel();
        let worker = spawn_command_worker(
            session_id.clone(),
            Env::from_os(),
            requests,
            control_tx,
            ingress.clone(),
            0,
        )
        .unwrap();
        let (active_reply, active_result) = mpsc::channel();
        let (pending_reply, pending_result) = mpsc::channel();
        ingress
            .submit("__malt_test_injected_worker_panic".to_string(), active_reply)
            .unwrap();
        ingress.submit("echo must-not-run".to_string(), pending_reply).unwrap();

        // The start is announced before the command runs, so even a command
        // that panics the worker leaves a recorded execution behind rather
        // than vanishing.
        let SessionCommand::ExecutionStarted { command_id, .. } = control_rx.recv().unwrap() else {
            panic!("worker must announce the start before running a request");
        };
        assert_eq!(command_id, 1);

        let SessionCommand::ExecutionCompleted(completion) = control_rx.recv().unwrap() else {
            panic!("worker must report a completion to the control actor");
        };
        assert!(matches!(completion.result, Err(DaemonError::ExecutionUnavailable(_))));
        let _ = completion.reply.send(Err(DaemonError::ExecutionUnavailable(session_id.clone())));
        completion.finalized.send(()).unwrap();
        assert!(matches!(
            active_result.recv().unwrap(),
            Err(DaemonError::ExecutionUnavailable(_))
        ));
        assert!(matches!(
            pending_result.recv().unwrap(),
            Err(DaemonError::ExecutionUnavailable(_))
        ));
        let (later_reply, _later_result) = mpsc::channel();
        assert!(matches!(
            ingress.submit("echo later".to_string(), later_reply),
            Err(DaemonError::ExecutionUnavailable(_))
        ));
        worker.join().unwrap();
    }
}
