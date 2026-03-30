pub mod error;
pub use error::StoreError;

use malt_protocol::common::SessionId;
use malt_protocol::persist::daemon::DaemonState;
use malt_protocol::persist::session::PersistedSession;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

pub struct SessionStore {
    base_dir: PathBuf,
}

impl SessionStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn sessions_dir(&self) -> PathBuf {
        self.base_dir.join("sessions")
    }

    fn session_path(&self, id: &SessionId) -> PathBuf {
        self.sessions_dir().join(format!("{}.vxb", id.0))
    }

    fn daemon_state_path(&self) -> PathBuf {
        self.base_dir.join("daemon.vxb")
    }

    pub fn save_session(
        &self,
        id: &SessionId,
        session: &PersistedSession,
    ) -> Result<(), StoreError> {
        let dir = self.sessions_dir();
        fs::create_dir_all(&dir)?;
        let bytes = pack_to_bytes(session)?;
        atomic_write(&self.session_path(id), &bytes)?;
        info!(?id, "session saved");
        Ok(())
    }

    pub fn load_session(&self, id: &SessionId) -> Result<PersistedSession, StoreError> {
        let path = self.session_path(id);
        if !path.exists() {
            return Err(StoreError::SessionNotFound(id.clone()));
        }
        let bytes = fs::read(&path)?;
        unpack_from_bytes(&bytes, &path)
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionId>, StoreError> {
        let dir = self.sessions_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(stem) = name.strip_suffix(".vxb") {
                if let Ok(id) = stem.parse::<u32>() {
                    ids.push(SessionId(id));
                }
            }
        }
        Ok(ids)
    }

    pub fn delete_session(&self, id: &SessionId) -> Result<(), StoreError> {
        let path = self.session_path(id);
        if path.exists() {
            fs::remove_file(&path)?;
            info!(?id, "session deleted");
        }
        Ok(())
    }

    pub fn save_daemon_state(&self, state: &DaemonState) -> Result<(), StoreError> {
        fs::create_dir_all(&self.base_dir)?;
        let bytes = pack_to_bytes(state)?;
        atomic_write(&self.daemon_state_path(), &bytes)?;
        info!("daemon state saved");
        Ok(())
    }

    pub fn load_daemon_state(&self) -> Result<DaemonState, StoreError> {
        let path = self.daemon_state_path();
        if !path.exists() {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "daemon.vxb not found",
            )));
        }
        let bytes = fs::read(&path)?;
        unpack_from_bytes(&bytes, &path)
    }
}

fn pack_to_bytes<T: Pack>(value: &T) -> Result<Vec<u8>, StoreError> {
    let mut w = BitWriter::new();
    value
        .pack(&mut w)
        .map_err(|e| StoreError::Encode(format!("{e}")))?;
    Ok(w.finish())
}

fn unpack_from_bytes<T: Unpack>(bytes: &[u8], path: &Path) -> Result<T, StoreError> {
    let mut r = BitReader::new(bytes);
    T::unpack(&mut r).map_err(|e| StoreError::Decode(format!("{}: {e}", path.display())))
}

fn atomic_write(path: &Path, data: &[u8]) -> Result<(), StoreError> {
    let tmp_path = path.with_extension("vxb.tmp");
    fs::write(&tmp_path, data)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}
