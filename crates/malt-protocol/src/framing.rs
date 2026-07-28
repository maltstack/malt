//! VNP wire framing — length-prefixed frames with a flags byte.
//!
//! Wire layout: `[4-byte LE payload length] [1-byte flags] [payload]`

use std::io::{self, Read, Write};

const PROTOCOL_MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;
const DEFAULT_MAX_FRAME_SIZE: u32 = 64 * 1024;
const FLAG_COMPRESSED: u8 = 1 << 0;
const FLAG_JSON_ENCODED: u8 = 1 << 1;
const FLAG_CONTINUATION: u8 = 1 << 2;
const RESERVED_MASK: u8 = !0b0000_0111;

#[derive(Debug, Clone)]
pub struct Frame {
    pub flags: FrameFlags,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameFlags(u8);

impl FrameFlags {
    pub fn new() -> Self {
        Self(0)
    }
    pub fn compressed(&self) -> bool {
        self.0 & FLAG_COMPRESSED != 0
    }
    pub fn set_compressed(&mut self, v: bool) {
        if v {
            self.0 |= FLAG_COMPRESSED;
        } else {
            self.0 &= !FLAG_COMPRESSED;
        }
    }
    pub fn json_encoded(&self) -> bool {
        self.0 & FLAG_JSON_ENCODED != 0
    }
    pub fn set_json_encoded(&mut self, v: bool) {
        if v {
            self.0 |= FLAG_JSON_ENCODED;
        } else {
            self.0 &= !FLAG_JSON_ENCODED;
        }
    }
    pub fn continuation(&self) -> bool {
        self.0 & FLAG_CONTINUATION != 0
    }
    pub fn set_continuation(&mut self, v: bool) {
        if v {
            self.0 |= FLAG_CONTINUATION;
        } else {
            self.0 &= !FLAG_CONTINUATION;
        }
    }
}

impl Default for FrameFlags {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame size {size} exceeds maximum {max}")]
    FrameTooLarge { size: u32, max: u32 },
    #[error("unexpected end of stream")]
    UnexpectedEof,
    #[error("reserved flags bits are set: {0:#04x}")]
    ReservedFlagsSet(u8),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Framed reader over a byte stream.
///
/// **Partial reads are retained across calls.** This matters whenever the
/// underlying stream can return early — most importantly a socket with a read
/// timeout, which the VNP listener sets to 16 ms so it can interleave reading
/// with draining render batches.
///
/// The previous implementation called `read_exact` per field and dropped
/// whatever it had already consumed when the timeout fired mid-frame. The
/// caller then saw `WouldBlock`, treated it as "idle, try again" — which is
/// what a timeout normally means — and the *next* `read_frame` began parsing
/// from the middle of a frame, reading payload bytes as a length prefix. The
/// connection was silently desynchronized from that point, and died on
/// whatever garbage it decoded next.
///
/// That bug was invisible on Windows: the listener sets its 16 ms timeout on
/// the write-side clone, and a receive timeout is per-descriptor there, so the
/// short timeout never applied to reads at all. On Linux, where `SO_RCVTIMEO`
/// is per-socket, it applied and split frames corrupted the connection.
pub struct FrameReader<R> {
    inner: R,
    max_frame_size: u32,
    /// Bytes read so far for the frame in progress. Empty between frames.
    partial: Vec<u8>,
}

impl<R: Read> FrameReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            inner: reader,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
            partial: Vec::new(),
        }
    }
    pub fn with_max_frame_size(reader: R, max: u32) -> Self {
        Self {
            inner: reader,
            max_frame_size: max.min(PROTOCOL_MAX_FRAME_SIZE),
            partial: Vec::new(),
        }
    }

    /// True when a frame has been partially read and is awaiting more bytes.
    ///
    /// A caller that treats a `WouldBlock` as "connection is idle" can use
    /// this to tell an idle connection from one mid-frame — the two are not
    /// the same, and conflating them is what this type exists to prevent.
    pub fn has_partial_frame(&self) -> bool {
        !self.partial.is_empty()
    }

    /// Read into `partial` until it holds at least `want` bytes.
    ///
    /// Returns `Ok(())` only when that is satisfied. Every error path leaves
    /// the bytes already consumed in `partial`, so a retry resumes rather than
    /// restarting — that is the whole point of this type.
    fn fill_to(&mut self, want: usize) -> Result<(), FrameError> {
        while self.partial.len() < want {
            let mut chunk = [0u8; 4096];
            let need = want - self.partial.len();
            let take = need.min(chunk.len());
            match self.inner.read(&mut chunk[..take]) {
                Ok(0) => return Err(FrameError::UnexpectedEof),
                Ok(n) => self.partial.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(FrameError::Io(e)),
            }
        }
        Ok(())
    }

    pub fn read_frame(&mut self) -> Result<Frame, FrameError> {
        self.fill_to(4)?;
        let mut len_buf = [0u8; 4];
        len_buf.copy_from_slice(&self.partial[..4]);
        let payload_len = u32::from_le_bytes(len_buf);

        if payload_len > self.max_frame_size {
            // Drop the partial frame: the stream cannot be resynchronized once
            // a length this large has been read, and retaining it would make
            // every subsequent call fail the same way.
            self.partial.clear();
            return Err(FrameError::FrameTooLarge {
                size: payload_len,
                max: self.max_frame_size,
            });
        }

        self.fill_to(5)?;
        let flags = self.partial[4];
        if flags & RESERVED_MASK != 0 {
            self.partial.clear();
            return Err(FrameError::ReservedFlagsSet(flags));
        }

        let total = 5 + payload_len as usize;
        self.fill_to(total)?;

        let payload = self.partial[5..total].to_vec();
        self.partial.drain(..total);

        Ok(Frame {
            flags: FrameFlags(flags),
            payload,
        })
    }
}

pub struct FrameWriter<W> {
    inner: W,
}

impl<W> FrameWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { inner: writer }
    }

    /// Get a reference to the underlying writer.
    pub fn inner_ref(&self) -> &W {
        &self.inner
    }
}

impl<W: Write> FrameWriter<W> {
    pub fn write_frame(&mut self, frame: &Frame) -> Result<(), FrameError> {
        let payload_len = frame.payload.len();
        if payload_len > PROTOCOL_MAX_FRAME_SIZE as usize {
            return Err(FrameError::FrameTooLarge {
                size: payload_len.min(u32::MAX as usize) as u32,
                max: PROTOCOL_MAX_FRAME_SIZE,
            });
        }
        let len = payload_len as u32;
        self.inner.write_all(&len.to_le_bytes())?;
        self.inner.write_all(&[frame.flags.0])?;
        self.inner.write_all(&frame.payload)?;
        Ok(())
    }
}

#[cfg(test)]
mod partial_read_tests {
    use super::*;

    /// A reader that hands out `script`ed chunks, returning `WouldBlock`
    /// between them — exactly what a socket with a short `SO_RCVTIMEO` does
    /// when a frame arrives split across the timeout boundary.
    struct ChoppyReader {
        chunks: Vec<Vec<u8>>,
        next: usize,
        /// Bytes from the current chunk that did not fit in the caller's
        /// buffer. Without this the *fake* silently drops data, which reads as
        /// an implementation bug -- it cost a debugging round when first
        /// written.
        pending: Vec<u8>,
    }

    impl ChoppyReader {
        fn new(chunks: Vec<Vec<u8>>) -> Self {
            Self {
                chunks,
                next: 0,
                pending: Vec::new(),
            }
        }
    }

    impl Read for ChoppyReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pending.is_empty() {
                if self.next >= self.chunks.len() {
                    return Err(io::Error::new(io::ErrorKind::WouldBlock, "idle"));
                }
                let chunk = self.chunks[self.next].clone();
                self.next += 1;
                if chunk.is_empty() {
                    // Scripted timeout: no bytes this time round.
                    return Err(io::Error::new(io::ErrorKind::WouldBlock, "timeout"));
                }
                self.pending = chunk;
            }
            let n = self.pending.len().min(buf.len());
            buf[..n].copy_from_slice(&self.pending[..n]);
            self.pending.drain(..n);
            Ok(n)
        }
    }

    /// The regression this type exists for. A frame delivered in pieces, with
    /// timeouts interleaved, must decode to exactly the bytes that were sent.
    ///
    /// Before `FrameReader` retained partial reads, the timeout discarded the
    /// bytes already consumed; the caller treated `WouldBlock` as "idle" and
    /// retried, and the next parse started mid-frame — reading payload bytes
    /// as a length prefix and desynchronizing the connection permanently.
    #[test]
    fn a_frame_split_across_timeouts_still_decodes_intact() {
        let payload = b"hello-vnp-payload".to_vec();
        let mut wire = Vec::new();
        wire.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        wire.push(0);
        wire.extend_from_slice(&payload);

        // Split mid-length-prefix, mid-flags, and mid-payload, with a
        // scripted timeout (empty chunk) before each remaining piece.
        let chunks = vec![
            wire[..2].to_vec(),
            Vec::new(),
            wire[2..5].to_vec(),
            Vec::new(),
            wire[5..9].to_vec(),
            wire[9..].to_vec(),
        ];

        let mut reader = FrameReader::new(ChoppyReader::new(chunks));

        let frame = loop {
            match reader.read_frame() {
                Ok(f) => break f,
                Err(FrameError::Io(ref e)) if e.kind() == io::ErrorKind::WouldBlock => {
                    // Exactly what the VNP listener does: treat a timeout as
                    // "nothing yet" and come back round. This must be safe.
                    continue;
                }
                Err(e) => panic!("unexpected error: {e}"),
            }
        };

        assert_eq!(frame.payload, payload, "payload must survive a split read");
        assert_eq!(frame.flags.0, 0);
        assert!(
            !reader.has_partial_frame(),
            "a fully decoded frame must leave no partial state behind"
        );
    }

    /// Two frames back to back, delivered in one chunk, must both decode --
    /// the reader has to consume exactly one frame and keep the remainder.
    #[test]
    fn a_trailing_frame_in_the_same_chunk_is_not_lost() {
        let mut wire = Vec::new();
        for payload in [b"first".to_vec(), b"second".to_vec()] {
            wire.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            wire.push(0);
            wire.extend_from_slice(&payload);
        }
        let mut reader = FrameReader::new(ChoppyReader::new(vec![wire]));
        assert_eq!(reader.read_frame().unwrap().payload, b"first".to_vec());
        assert_eq!(reader.read_frame().unwrap().payload, b"second".to_vec());
    }

    /// A clean close mid-frame is still an error, not a silently short frame.
    #[test]
    fn eof_midway_through_a_frame_is_reported() {
        let mut wire = Vec::new();
        wire.extend_from_slice(&(64u32).to_le_bytes());
        wire.push(0);
        wire.extend_from_slice(b"only-a-few-bytes");
        let mut reader = FrameReader::new(ChoppyReader::new(vec![wire, vec![0u8; 0]]));
        // First call consumes what exists then hits the scripted timeout.
        match reader.read_frame() {
            Err(FrameError::Io(ref e)) if e.kind() == io::ErrorKind::WouldBlock => {}
            other => panic!("expected WouldBlock while incomplete, got {other:?}"),
        }
        assert!(reader.has_partial_frame());
    }
}
