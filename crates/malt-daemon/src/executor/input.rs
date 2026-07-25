//! The session input channel: where raw client input goes, and where a
//! command blocked on `read` gets it from.
//!
//! The shape is dictated by one constraint: **the control actor must never
//! block on a client.** An OS pipe write blocks once the pipe buffer fills,
//! and a client that types faster than a command reads will fill it. So the
//! actor does not write to the pipe directly. It hands bytes to a bounded
//! channel with `try_send`, and a dedicated writer thread owns the pipe end
//! and does the blocking write.
//!
//! That gives two bounds in series — the pipe's own buffer, then the queue —
//! and a definite answer when both are full, rather than a stalled session.
//! It is the same discipline `events.rs` uses for subscriber delivery, and
//! for the same reason.
//!
//! The read end is registered at fd `0` in the session's `mash::Env`. That is
//! the whole trick for delivery: `read` already resolves `env.open_fd_read(0)`
//! before falling back to `std::io::stdin()`, so registering it both routes
//! client input to the builtin *and* makes the fall-through to the daemon's
//! own console unreachable — with no change to `mash` itself.

use std::io::Write;
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};

/// How many un-consumed input submissions a session will hold beyond what
/// the OS pipe buffer already absorbs.
///
/// This is the type-ahead bound. Input sent while nothing is reading is
/// retained for the next read; past this, submissions are refused with a
/// clear error rather than queued without limit or silently dropped.
pub const INPUT_QUEUE_DEPTH: usize = 256;

/// Why a raw input submission was not accepted.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum InputError {
    #[error("session input buffer is full; the command has not consumed prior input")]
    BufferFull,
    #[error("session input channel is closed")]
    Closed,
    /// The sender does not hold input authority for this session.
    ///
    /// Names the holder so the caller can say something useful rather than
    /// "refused": a client that cannot see who has the keyboard cannot decide
    /// whether to claim it (FR-014, FR-015).
    #[error("client {client_id:?} does not hold input authority; client {holder} does")]
    NotAuthority { client_id: Option<u64>, holder: u64 },
    /// A client tried to claim authority without being attached.
    #[error("client {client_id} is not attached to this session")]
    NotAttached { client_id: u64 },
}

/// One session's raw-input destination.
///
/// Holds the sending half of the queue. The writer thread and the pipe's
/// write end live behind it; the read end is handed to the caller at
/// construction so it can be registered as the session's fd 0.
#[derive(Debug)]
pub struct SessionInputChannel {
    tx: SyncSender<InputMessage>,
}

/// What the writer thread is being asked to do.
///
/// End-of-input is a queued message rather than an immediate action so that it
/// keeps its place in line: bytes sent before it must reach the reader before
/// the stream ends, which a side-channel flag could not guarantee.
#[derive(Debug)]
enum InputMessage {
    Bytes(Vec<u8>),
    EndOfInput,
}

impl SessionInputChannel {
    /// Create the channel and return it alongside the pipe's read end.
    ///
    /// The caller registers that read end at fd `0` in the session's
    /// `mash::Env`, and passes the env's [`fd_registry`] handle so the channel
    /// can replace fd 0 later when a client signals end-of-input.
    ///
    /// [`fd_registry`]: mash::env::Env::fd_registry
    pub fn new(
        session_id: u32,
        registry: malt_platform::vfs::SharedFdRegistry,
    ) -> std::io::Result<(Self, std::fs::File)> {
        Self::with_depth(session_id, registry, INPUT_QUEUE_DEPTH)
    }

    /// As [`SessionInputChannel::new`] with an explicit queue depth, so tests
    /// can exhaust the bound without writing a pipe buffer's worth of data.
    pub fn with_depth(
        session_id: u32,
        registry: malt_platform::vfs::SharedFdRegistry,
        depth: usize,
    ) -> std::io::Result<(Self, std::fs::File)> {
        let (read_end, write_end) = malt_platform::io::create_pipe()?;
        let (tx, rx) = sync_channel::<InputMessage>(depth.max(1));

        // The writer thread exists so the blocking write happens somewhere
        // that is allowed to block. It ends when the channel closes, which
        // happens when the session drops its sender.
        std::thread::Builder::new()
            .name(format!("session-input-{session_id}"))
            .spawn(move || {
                let mut write_end = write_end;
                while let Ok(message) = rx.recv() {
                    match message {
                        InputMessage::Bytes(bytes) => {
                            if write_end.write_all(&bytes).is_err() {
                                break;
                            }
                            if write_end.flush().is_err() {
                                break;
                            }
                        }
                        InputMessage::EndOfInput => {
                            // Dropping the only write end is what the reader
                            // observes as EOF -- there is no in-band byte that
                            // means "end of input", which is exactly why a
                            // sentinel value would not work here.
                            let (next_read, next_write) = match malt_platform::io::create_pipe() {
                                Ok(pair) => pair,
                                Err(e) => {
                                    // Without a replacement pipe the session
                                    // could never be given input again. Stop
                                    // the writer rather than pretend.
                                    tracing::error!(
                                        session_id,
                                        %e,
                                        "session input: could not replace pipe after EOF; \
                                         this session can no longer receive input"
                                    );
                                    break;
                                }
                            };
                            // Install the replacement *before* dropping the old
                            // write end, so a command starting immediately
                            // after EOF finds a live fd 0 rather than a closed
                            // one.
                            registry.register_file_at(0, next_read);
                            write_end = next_write;
                        }
                    }
                }
            })?;

        Ok((Self { tx }, read_end))
    }

    /// Signal end-of-input to whatever is currently reading -- the daemon's
    /// equivalent of Ctrl-D.
    ///
    /// A command consuming to the end (`cat`, `wc`, `grep`) terminates on
    /// this. Without it those commands would wait forever, because a session's
    /// stdin has no natural end: the daemon holds the write end for the
    /// session's lifetime.
    ///
    /// This ends the *current* read, not the session. A fresh pipe is put in
    /// place immediately, so the next command can still be given input.
    pub fn end_of_input(&self) -> Result<(), InputError> {
        match self.tx.try_send(InputMessage::EndOfInput) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(InputError::BufferFull),
            Err(TrySendError::Disconnected(_)) => Err(InputError::Closed),
        }
    }

    /// Queue bytes for delivery to whatever is reading this session's input.
    ///
    /// Never blocks. Bytes are passed through unmodified — no decoding, no
    /// trimming, no dropping of empty input. A bare newline is a real answer
    /// to a confirmation prompt, and leading or trailing whitespace can be
    /// part of a password.
    pub fn try_write(&self, data: &[u8]) -> Result<(), InputError> {
        match self.tx.try_send(InputMessage::Bytes(data.to_vec())) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(InputError::BufferFull),
            Err(TrySendError::Disconnected(_)) => Err(InputError::Closed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn written_bytes_arrive_at_the_read_end_unmodified() {
        let (channel, mut read_end) =
            SessionInputChannel::new(1, malt_platform::vfs::SharedFdRegistry::new()).unwrap();

        // Each of these is a way the previous input path corrupted data.
        channel.try_write(b"  padded  \n").unwrap();

        let mut buf = [0u8; 11];
        read_end.read_exact(&mut buf).unwrap();
        assert_eq!(
            &buf, b"  padded  \n",
            "surrounding whitespace must survive; the old path trimmed it"
        );
    }

    #[test]
    fn bytes_that_are_not_valid_text_survive() {
        let (channel, mut read_end) =
            SessionInputChannel::new(2, malt_platform::vfs::SharedFdRegistry::new()).unwrap();
        let raw = [0xff, 0xfe, 0x00, 0x41, 0x0a];
        channel.try_write(&raw).unwrap();

        let mut buf = [0u8; 5];
        read_end.read_exact(&mut buf).unwrap();
        assert_eq!(
            buf, raw,
            "the old path ran from_utf8_lossy, which would replace these"
        );
    }

    #[test]
    fn a_bare_newline_is_delivered_rather_than_discarded() {
        let (channel, mut read_end) =
            SessionInputChannel::new(3, malt_platform::vfs::SharedFdRegistry::new()).unwrap();
        channel.try_write(b"\n").unwrap();

        let mut buf = [0u8; 1];
        read_end.read_exact(&mut buf).unwrap();
        assert_eq!(
            &buf, b"\n",
            "a bare newline is the answer to a confirmation prompt; the old \
             path treated it as empty and dropped it"
        );
    }

    #[test]
    fn a_full_queue_is_refused_rather_than_blocking() {
        // Depth 1, and nothing ever reads the pipe. Once the pipe buffer and
        // the queue are both full, further writes must return promptly.
        let (channel, _read_end) =
            SessionInputChannel::with_depth(4, malt_platform::vfs::SharedFdRegistry::new(), 1)
                .unwrap();

        let big = vec![b'x'; 256 * 1024]; // larger than a default pipe buffer
        let started = std::time::Instant::now();
        let mut refused = false;
        for _ in 0..8 {
            if matches!(channel.try_write(&big), Err(InputError::BufferFull)) {
                refused = true;
                break;
            }
        }
        let elapsed = started.elapsed();

        assert!(
            refused,
            "an unread session must eventually refuse input rather than accept it forever"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "refusal took {elapsed:?}; try_write must not block the control actor"
        );
    }

    #[test]
    fn submissions_are_delivered_in_order() {
        let (channel, mut read_end) =
            SessionInputChannel::new(5, malt_platform::vfs::SharedFdRegistry::new()).unwrap();
        channel.try_write(b"first\n").unwrap();
        channel.try_write(b"second\n").unwrap();

        let mut buf = [0u8; 13];
        read_end.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"first\nsecond\n");
    }
}
