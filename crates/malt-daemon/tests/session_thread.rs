use malt_daemon::executor::session_thread::{SessionExecutor, SessionCommand};
use malt_daemon::bus::BusMessage;
use malt_protocol::common::{InputAuthority, IsolationTier, PaneId, SessionId};
use malt_protocol::priority::Priority;

#[test]
fn session_executor_starts_and_stops() {
    let (cmd_tx, handle) = SessionExecutor::spawn(
        SessionId(1),
        PaneId(1),
        IsolationTier::Bare,
    );
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn session_executor_processes_input() {
    let (cmd_tx, handle) = SessionExecutor::spawn(
        SessionId(1),
        PaneId(1),
        IsolationTier::Bare,
    );
    let msg = BusMessage {
        domain: 2,
        msg_type: 1,
        priority: Priority::Critical,
        producer_id: 0,
        payload: vec![1, 2, 3],
    };
    cmd_tx.send(SessionCommand::Deliver(msg)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn session_executor_attach_detach() {
    let (cmd_tx, handle) = SessionExecutor::spawn(
        SessionId(1),
        PaneId(1),
        IsolationTier::Bare,
    );
    cmd_tx.send(SessionCommand::AttachClient {
        client_id: 100,
        authority: InputAuthority::Exclusive,
    }).unwrap();
    cmd_tx.send(SessionCommand::DetachClient { client_id: 100 }).unwrap();
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}

#[test]
fn session_executor_resize() {
    let (cmd_tx, handle) = SessionExecutor::spawn(
        SessionId(1),
        PaneId(1),
        IsolationTier::Bare,
    );
    cmd_tx.send(SessionCommand::Resize { cols: 120, rows: 40 }).unwrap();
    cmd_tx.send(SessionCommand::Shutdown).unwrap();
    handle.join().expect("session thread panicked");
}
