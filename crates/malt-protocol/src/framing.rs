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

pub struct FrameReader<R> {
    inner: R,
    max_frame_size: u32,
}

impl<R: Read> FrameReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            inner: reader,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
        }
    }
    pub fn with_max_frame_size(reader: R, max: u32) -> Self {
        Self {
            inner: reader,
            max_frame_size: max.min(PROTOCOL_MAX_FRAME_SIZE),
        }
    }

    pub fn read_frame(&mut self) -> Result<Frame, FrameError> {
        let mut len_buf = [0u8; 4];
        self.read_exact_or_eof(&mut len_buf)?;
        let payload_len = u32::from_le_bytes(len_buf);

        if payload_len > self.max_frame_size {
            return Err(FrameError::FrameTooLarge {
                size: payload_len,
                max: self.max_frame_size,
            });
        }

        let mut flags_buf = [0u8; 1];
        self.read_exact_or_eof(&mut flags_buf)?;

        if flags_buf[0] & RESERVED_MASK != 0 {
            return Err(FrameError::ReservedFlagsSet(flags_buf[0]));
        }

        let mut payload = vec![0u8; payload_len as usize];
        self.read_exact_or_eof(&mut payload)?;

        Ok(Frame {
            flags: FrameFlags(flags_buf[0]),
            payload,
        })
    }

    fn read_exact_or_eof(&mut self, buf: &mut [u8]) -> Result<(), FrameError> {
        match self.inner.read_exact(buf) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Err(FrameError::UnexpectedEof),
            Err(e) => Err(FrameError::Io(e)),
        }
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
        let len = frame.payload.len() as u32;
        self.inner.write_all(&len.to_le_bytes())?;
        self.inner.write_all(&[frame.flags.0])?;
        self.inner.write_all(&frame.payload)?;
        Ok(())
    }
}
