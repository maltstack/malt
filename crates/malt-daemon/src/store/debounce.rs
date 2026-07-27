use super::{SessionStore, StoreError};
use malt_protocol::common::SessionId;
use malt_protocol::persist::daemon::DaemonState;
use malt_protocol::persist::session::PersistedSession;
use std::collections::HashMap;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use tracing::error;

enum DirtyMsg {
    DirtySession(SessionId, PersistedSession, Instant),
    DirtyDaemon(DaemonState, Instant),
    Flush(mpsc::Sender<()>),
}

struct Inner {
    tx: mpsc::Sender<DirtyMsg>,
    store: Arc<SessionStore>,
}

/// Debounced wrapper around `SessionStore`.
///
/// Callers mark entries dirty; a background thread flushes entries that have
/// been dirty for ≥ 1 second. `flush_all()` drains everything synchronously
/// (use before shutdown).
///
/// `Clone` is cheap — all clones share the same background thread via `Arc`.
#[derive(Clone)]
pub struct DebouncedStore {
    inner: Arc<Inner>,
}

impl DebouncedStore {
    pub fn new(store: SessionStore) -> Self {
        let (tx, rx) = mpsc::channel::<DirtyMsg>();
        let store = Arc::new(store);
        let thread_store = Arc::clone(&store);

        std::thread::Builder::new()
            .name("malt-debounce-flush".to_string())
            .spawn(move || background_thread(rx, thread_store))
            .expect("failed to spawn debounce flush thread");

        Self {
            inner: Arc::new(Inner { tx, store }),
        }
    }

    /// Mark a session dirty. The latest value overwrites any pending dirty entry.
    pub fn mark_dirty(&self, id: SessionId, session: PersistedSession) {
        let _ = self
            .inner
            .tx
            .send(DirtyMsg::DirtySession(id, session, Instant::now()));
    }

    /// Mark the daemon-level state dirty.
    /// The latest value overwrites any pending dirty entry.
    pub fn mark_dirty_daemon(&self, state: DaemonState) {
        let _ = self
            .inner
            .tx
            .send(DirtyMsg::DirtyDaemon(state, Instant::now()));
    }

    /// Load daemon state synchronously from the underlying store.
    /// Called once at coordinator startup.
    pub fn load_daemon_state(&self) -> Result<DaemonState, StoreError> {
        self.inner.store.load_daemon_state()
    }

    /// Flush all dirty sessions and daemon state synchronously.
    /// Blocks until the background thread confirms the flush is complete.
    pub fn flush_all(&self) {
        let (reply_tx, reply_rx) = mpsc::channel();
        if self.inner.tx.send(DirtyMsg::Flush(reply_tx)).is_ok() {
            // Wait up to 10 s for confirmation; if the thread is gone, skip.
            let _ = reply_rx.recv_timeout(Duration::from_secs(10));
        }
    }

    /// List all persisted session IDs synchronously.
    pub fn list_sessions(&self) -> Result<Vec<SessionId>, StoreError> {
        self.inner.store.list_sessions()
    }

    /// Load a single persisted session synchronously.
    pub fn load_session(&self, id: &SessionId) -> Result<PersistedSession, StoreError> {
        self.inner.store.load_session(id)
    }

    /// Delete a persisted session synchronously.
    ///
    /// # Note
    ///
    /// Callers must call `flush_all()` before `delete_session` if any prior
    /// `mark_dirty` for this ID may be outstanding in the background queue.
    /// Otherwise the background thread may re-create the deleted file after
    /// the delete completes.
    pub fn delete_session(&self, id: &SessionId) -> Result<(), StoreError> {
        self.inner.store.delete_session(id)
    }
}

fn background_thread(rx: mpsc::Receiver<DirtyMsg>, store: Arc<SessionStore>) {
    // Key by the inner u32 since SessionId does not implement Hash/Eq.
    // The full SessionId is stored alongside for use in store calls.
    let mut sessions: HashMap<u32, (SessionId, PersistedSession, Instant)> = HashMap::new();
    let mut daemon_slot: Option<(DaemonState, Instant)> = None;
    let threshold = Duration::from_secs(1);
    let poll_interval = Duration::from_millis(100);

    loop {
        match rx.recv_timeout(poll_interval) {
            Ok(DirtyMsg::DirtySession(id, session, ts)) => {
                sessions.insert(id.0, (id, session, ts));
            }
            Ok(DirtyMsg::DirtyDaemon(state, ts)) => {
                daemon_slot = Some((state, ts));
            }
            Ok(DirtyMsg::Flush(reply)) => {
                // Drain all session entries.
                for (_key, (id, session, _)) in sessions.drain() {
                    if let Err(e) = store.save_session(&id, &session) {
                        error!(?id, %e, "debounce flush: save_session failed");
                    }
                }
                // Drain daemon slot.
                if let Some((state, _)) = daemon_slot.take() {
                    if let Err(e) = store.save_daemon_state(&state) {
                        error!(%e, "debounce flush: save_daemon_state failed");
                    }
                }
                let _ = reply.send(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                // Flush session entries that have been dirty for ≥ 1 s.
                let expired_keys: Vec<u32> = sessions
                    .iter()
                    .filter(|(_, (_, _, ts))| now.duration_since(*ts) >= threshold)
                    .map(|(key, _)| *key)
                    .collect();
                for key in expired_keys {
                    if let Some((id, session, _)) = sessions.remove(&key) {
                        if let Err(e) = store.save_session(&id, &session) {
                            error!(?id, %e, "debounce auto-flush: save_session failed");
                            // Re-insert with fresh timestamp to retry in 1 s.
                            sessions.insert(key, (id, session, Instant::now()));
                        }
                    }
                }
                // Flush daemon slot if dirty for ≥ 1 s.
                let should_flush_daemon = daemon_slot
                    .as_ref()
                    .map(|(_, ts)| now.duration_since(*ts) >= threshold)
                    .unwrap_or(false);
                if should_flush_daemon {
                    if let Some((ref state, _)) = daemon_slot {
                        if let Err(e) = store.save_daemon_state(state) {
                            error!(%e, "debounce auto-flush: save_daemon_state failed");
                            // Reset timestamp to retry in 1 s.
                            if let Some((_, ref mut ts)) = daemon_slot {
                                *ts = Instant::now();
                            }
                        } else {
                            daemon_slot = None;
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(next_id: u32) -> DaemonState {
        DaemonState {
            schema_version: 1,
            sessions: vec![],
            active_groups: vec![],
            next_session_id: next_id,
            next_pane_id: 1,
            _unknown: vec![],
        }
    }

    #[test]
    fn flush_all_is_immediate() {
        let dir = tempfile::tempdir().unwrap();
        let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));

        // Mark daemon state dirty and flush immediately — no 1s wait.
        store.mark_dirty_daemon(make_state(7));
        let before = Instant::now();
        store.flush_all();
        let elapsed = before.elapsed();

        assert!(
            elapsed < Duration::from_millis(500),
            "flush_all took too long: {elapsed:?}"
        );

        // Data must be on disk after flush_all
        let loaded = store.load_daemon_state().unwrap();
        assert_eq!(loaded.next_session_id, 7);
    }

    #[test]
    fn daemon_latest_value_wins() {
        let dir = tempfile::tempdir().unwrap();
        let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));

        // Two rapid mark_dirty_daemon calls — only the latest should be saved.
        store.mark_dirty_daemon(make_state(1));
        store.mark_dirty_daemon(make_state(42));
        store.flush_all();

        let loaded = store.load_daemon_state().unwrap();
        assert_eq!(loaded.next_session_id, 42);
    }

    #[test]
    fn flushes_after_one_second_idle() {
        let dir = tempfile::tempdir().unwrap();
        let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));

        store.mark_dirty_daemon(make_state(99));

        // Wait 1.2 s — background thread should auto-flush.
        std::thread::sleep(Duration::from_millis(1200));

        let loaded = store.load_daemon_state().unwrap();
        assert_eq!(loaded.next_session_id, 99);
    }

    #[test]
    fn clone_shares_flush_channel() {
        let dir = tempfile::tempdir().unwrap();
        let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));
        let clone = store.clone();

        clone.mark_dirty_daemon(make_state(55));
        store.flush_all(); // flush from original — should flush clone's dirty entry too

        let loaded = store.load_daemon_state().unwrap();
        assert_eq!(loaded.next_session_id, 55);
    }

    #[test]
    fn passthrough_list_load_delete() {
        use malt_protocol::common::{LayoutNode, PaneId};
        use std::collections::BTreeMap;

        let dir = tempfile::tempdir().unwrap();
        let inner = SessionStore::new(dir.path().to_path_buf());
        let store = DebouncedStore::new(inner);

        // Nothing persisted yet.
        assert!(store.list_sessions().unwrap().is_empty());

        // Save a session via mark_dirty + flush_all, then load it back.
        let sid = SessionId(7);
        let persisted = PersistedSession {
            schema_version: 1,
            id: sid.clone(),
            name: Some("test".to_string()),
            layout: LayoutNode::Leaf { pane_id: PaneId(1) },
            focus: PaneId(1),
            panes: BTreeMap::new(),
            theme: None,
            group: None,
            isolation: malt_protocol::common::IsolationTier::Bare,
            selected_image: None,
            _unknown: vec![],
        };
        store.mark_dirty(sid.clone(), persisted.clone());
        store.flush_all();

        let ids = store.list_sessions().unwrap();
        assert_eq!(ids, vec![sid.clone()]);

        let loaded = store.load_session(&sid).unwrap();
        assert_eq!(loaded.name, persisted.name);

        store.delete_session(&sid).unwrap();
        assert!(store.list_sessions().unwrap().is_empty());
    }
}
