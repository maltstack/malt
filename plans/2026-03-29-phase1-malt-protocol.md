# `malt-protocol` Crate — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `malt-protocol` Rust crate — VNP message types (Vexil-generated), wire framing, envelope helpers, and priority mapping.

**Architecture:** `build.rs` invokes `vexilc build` per schema file, generates Rust modules into `$OUT_DIR`, then writes a unified `mod.rs`. Domain modules in `src/` include the generated code and re-export. Framing and priority are hand-written. Spec: `malt/specs/phase1-malt-protocol-crate.md`.

**Tech Stack:** Rust, vexil-runtime, thiserror, vexilc (build-time CLI)

---

## File Structure

```
orix/malt/
  Cargo.toml                          # MODIFY: add workspace member
  crates/
    malt-protocol/
      Cargo.toml
      build.rs                        # Invokes vexilc per schema, writes mod.rs
      src/
        lib.rs                        # Public API: re-exports all modules
        framing.rs                    # Frame, FrameFlags, FrameReader, FrameWriter, FrameError
        priority.rs                   # Priority enum, priority_of() const fn
        envelope.rs                   # Re-export + encode_message/decode_envelope helpers
        generated.rs                  # include! of $OUT_DIR/malt/mod.rs (all generated code)
      tests/
        framing.rs                    # Framing roundtrip tests
        envelope_golden.rs            # Golden-byte envelope test
        priority.rs                   # Exhaustive priority mapping test
        roundtrip.rs                  # Message encode/decode per domain
```

**Key simplification:** Instead of one `src/shell.rs`, `src/render.rs`, etc. that each `include!` a single generated file, we use a single `src/generated.rs` that includes the top-level generated `mod.rs`. The vexilc output already organizes code into `malt::shell`, `malt::render`, etc. with proper `mod.rs` files. We just include the root and re-export from `lib.rs`:

```rust
// src/lib.rs
pub use generated::malt::*;  // exposes common, shell, render, etc. as modules
```

This avoids maintaining 15 near-identical wrapper files.

---

## Task 1: Workspace and Crate Scaffold

**Files:**
- Modify: `orix/malt/Cargo.toml` (add workspace)
- Create: `orix/malt/crates/malt-protocol/Cargo.toml`
- Create: `orix/malt/crates/malt-protocol/src/lib.rs`

- [ ] **Step 1: Create the workspace Cargo.toml**

If `orix/malt/Cargo.toml` doesn't exist, create it. If it does, add workspace members.

Create `orix/malt/Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/malt-protocol"]

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.85"
license = "MIT OR Apache-2.0"
repository = "https://github.com/orix-systems/malt"
```

- [ ] **Step 2: Create the crate Cargo.toml**

Create `orix/malt/crates/malt-protocol/Cargo.toml`:

```toml
[package]
name = "malt-protocol"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "VNP message types, wire framing, and protocol primitives for MALT"

[dependencies]
vexil-runtime = { path = "../../../../vexil-lang/crates/vexil-runtime" }
thiserror = "2"

[build-dependencies]
# vexilc is invoked as a CLI tool in build.rs, not a library dep
```

- [ ] **Step 3: Create the initial lib.rs**

Create `orix/malt/crates/malt-protocol/src/lib.rs`:

```rust
//! VNP message types, wire framing, and protocol primitives for MALT.
//!
//! This crate is the L0 foundation — every other MALT crate depends on it.
//! Message types are generated from `.vexil` schemas by `vexilc` at build time.

pub mod framing;
pub mod priority;
pub mod envelope;

// Generated code from vexilc — all domain modules (common, shell, render, etc.)
#[allow(clippy::all, unused_qualifications)]
mod generated;
pub use generated::malt::*;
```

- [ ] **Step 4: Create placeholder modules so it compiles**

Create `orix/malt/crates/malt-protocol/src/framing.rs`:

```rust
//! VNP wire framing — length-prefixed frames with a flags byte.
```

Create `orix/malt/crates/malt-protocol/src/priority.rs`:

```rust
//! Bus priority mapping for VNP message types.
```

Create `orix/malt/crates/malt-protocol/src/envelope.rs`:

```rust
//! Envelope encode/decode helpers.
```

Create `orix/malt/crates/malt-protocol/src/generated.rs`:

```rust
//! Vexilc-generated code. This file is overwritten by build.rs.
//! Placeholder until build.rs is implemented.
```

- [ ] **Step 5: Verify it compiles**

Run: `cd orix/malt && cargo check -p malt-protocol`
Expected: Compiles (empty modules).

- [ ] **Step 6: Commit**

```bash
cd orix/malt
git add Cargo.toml crates/
git commit -m "feat(malt-protocol): scaffold workspace and crate structure"
```

---

## Task 2: Build Script — vexilc Integration

**Files:**
- Create: `orix/malt/crates/malt-protocol/build.rs`
- Modify: `orix/malt/crates/malt-protocol/src/generated.rs`

The build script invokes `vexilc build` for each `.vexil` schema, then writes a unified `mod.rs` since vexilc overwrites it on each invocation.

- [ ] **Step 1: Write build.rs**

Create `orix/malt/crates/malt-protocol/build.rs`:

```rust
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    // Schema files are at ../../schemas/ relative to crate root
    let schemas_dir = manifest_dir.join("../../schemas");
    let schemas_dir = schemas_dir.canonicalize().expect("schemas/ directory not found");

    // Find vexilc — check VEXILC_PATH env, then PATH
    let vexilc = env::var("VEXILC_PATH").unwrap_or_else(|_| "vexilc".to_string());

    // Collect all .vexil files
    let mut schema_files = Vec::new();
    collect_vexil_files(&schemas_dir, &mut schema_files);

    // Build each schema file — vexilc generates into out_dir/malt/<domain>.rs
    for schema in &schema_files {
        let status = Command::new(&vexilc)
            .arg("build")
            .arg(schema)
            .arg("--include")
            .arg(&schemas_dir)
            .arg("--output")
            .arg(&out_dir)
            .arg("--target")
            .arg("rust")
            .status()
            .unwrap_or_else(|e| panic!("failed to run vexilc: {e}"));

        if !status.success() {
            panic!(
                "vexilc build failed for {}",
                schema.display()
            );
        }
    }

    // vexilc overwrites mod.rs on each invocation, so we regenerate the
    // top-level mod.rs with all modules declared.
    write_mod_rs(&out_dir);

    // Rerun if any schema changes
    println!("cargo::rerun-if-changed={}", schemas_dir.display());
    for schema in &schema_files {
        println!("cargo::rerun-if-changed={}", schema.display());
    }
}

fn collect_vexil_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_vexil_files(&path, files);
        } else if path.extension().is_some_and(|e| e == "vexil") {
            files.push(path);
        }
    }
}

fn write_mod_rs(out_dir: &Path) {
    let malt_dir = out_dir.join("malt");

    // Collect all .rs files that aren't mod.rs
    let mut modules = Vec::new();
    if let Ok(entries) = fs::read_dir(&malt_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path.extension().is_some_and(|e| e == "rs")
                && path.file_stem().is_some_and(|s| s != "mod")
            {
                if let Some(name) = path.file_stem() {
                    modules.push(name.to_string_lossy().to_string());
                }
            } else if path.is_dir() {
                // Subdirectory (e.g., persist/) — check if it has a mod.rs
                if path.join("mod.rs").exists() {
                    if let Some(name) = path.file_name() {
                        modules.push(name.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    modules.sort();

    let mut content = String::from("// Code generated by malt-protocol build.rs. DO NOT EDIT.\n\n");
    for module in &modules {
        content.push_str(&format!("pub mod {module};\n"));
    }

    fs::write(malt_dir.join("mod.rs"), &content).unwrap();

    // Also handle persist/ subdir mod.rs
    let persist_dir = malt_dir.join("persist");
    if persist_dir.is_dir() {
        let mut persist_mods = Vec::new();
        if let Ok(entries) = fs::read_dir(&persist_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file()
                    && path.extension().is_some_and(|e| e == "rs")
                    && path.file_stem().is_some_and(|s| s != "mod")
                {
                    if let Some(name) = path.file_stem() {
                        persist_mods.push(name.to_string_lossy().to_string());
                    }
                }
            }
        }
        persist_mods.sort();

        let mut persist_content =
            String::from("// Code generated by malt-protocol build.rs. DO NOT EDIT.\n\n");
        for module in &persist_mods {
            persist_content.push_str(&format!("pub mod {module};\n"));
        }
        fs::write(persist_dir.join("mod.rs"), &persist_content).unwrap();
    }

    // Write root mod.rs that declares the malt module
    let root_mod = format!(
        "// Code generated by malt-protocol build.rs. DO NOT EDIT.\n\npub mod malt;\n"
    );
    fs::write(out_dir.join("mod.rs"), &root_mod).unwrap();
}
```

- [ ] **Step 2: Update generated.rs to include build output**

Replace `orix/malt/crates/malt-protocol/src/generated.rs`:

```rust
//! Vexilc-generated code — all VNP domain modules.
//!
//! This file includes the output of `vexilc build` from build.rs.
//! The generated module tree: malt::{common, shell, render, ...}

include!(concat!(env!("OUT_DIR"), "/mod.rs"));
```

- [ ] **Step 3: Verify build works**

Run: `cd orix/malt && cargo check -p malt-protocol`
Expected: Compiles. The generated code is included and types are available.

- [ ] **Step 4: Verify a generated type is accessible**

Add a temporary test to `src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn generated_types_accessible() {
        let _id = crate::common::PaneId(42);
        let _tier = crate::common::IsolationTier::Bare;
    }
}
```

Run: `cd orix/malt && cargo test -p malt-protocol`
Expected: PASS.

Remove the temporary test after verification.

- [ ] **Step 5: Commit**

```bash
cd orix/malt
git add crates/malt-protocol/build.rs crates/malt-protocol/src/generated.rs
git commit -m "feat(malt-protocol): add build.rs — vexilc codegen integration for all schemas"
```

---

## Task 3: Framing Layer

**Files:**
- Modify: `orix/malt/crates/malt-protocol/src/framing.rs`
- Create: `orix/malt/crates/malt-protocol/tests/framing.rs`

- [ ] **Step 1: Write framing tests**

Create `orix/malt/crates/malt-protocol/tests/framing.rs`:

```rust
use malt_protocol::framing::{Frame, FrameError, FrameFlags, FrameReader, FrameWriter};
use std::io::Cursor;

#[test]
fn roundtrip_empty_payload() {
    let frame = Frame {
        flags: FrameFlags::new(),
        payload: vec![],
    };
    let mut buf = Vec::new();
    FrameWriter::new(&mut buf).write_frame(&frame).unwrap();

    let decoded = FrameReader::new(Cursor::new(&buf))
        .read_frame()
        .unwrap();
    assert_eq!(frame.payload, decoded.payload);
    assert_eq!(frame.flags, decoded.flags);
}

#[test]
fn roundtrip_with_payload() {
    let frame = Frame {
        flags: FrameFlags::new(),
        payload: vec![0xDE, 0xAD, 0xBE, 0xEF],
    };
    let mut buf = Vec::new();
    FrameWriter::new(&mut buf).write_frame(&frame).unwrap();

    let decoded = FrameReader::new(Cursor::new(&buf))
        .read_frame()
        .unwrap();
    assert_eq!(frame.payload, decoded.payload);
}

#[test]
fn roundtrip_all_flags() {
    let mut flags = FrameFlags::new();
    flags.set_compressed(true);
    flags.set_json_encoded(true);
    flags.set_continuation(true);

    let frame = Frame {
        flags,
        payload: vec![1, 2, 3],
    };
    let mut buf = Vec::new();
    FrameWriter::new(&mut buf).write_frame(&frame).unwrap();

    let decoded = FrameReader::new(Cursor::new(&buf))
        .read_frame()
        .unwrap();
    assert!(decoded.flags.compressed());
    assert!(decoded.flags.json_encoded());
    assert!(decoded.flags.continuation());
}

#[test]
fn flags_individual_bits() {
    let mut f = FrameFlags::new();
    assert!(!f.compressed());
    assert!(!f.json_encoded());
    assert!(!f.continuation());

    f.set_compressed(true);
    assert!(f.compressed());
    assert!(!f.json_encoded());

    f.set_json_encoded(true);
    assert!(f.json_encoded());

    f.set_continuation(true);
    assert!(f.continuation());
}

#[test]
fn reject_frame_too_large() {
    // Write a frame claiming 1 MiB payload
    let mut buf = Vec::new();
    buf.extend_from_slice(&(1_048_576u32).to_le_bytes()); // length
    buf.push(0x00); // flags
    buf.extend(vec![0u8; 1_048_576]); // payload

    // Reader with 64 KiB max should reject
    let result = FrameReader::with_max_frame_size(Cursor::new(&buf), 65_536)
        .read_frame();
    assert!(matches!(result, Err(FrameError::FrameTooLarge { .. })));
}

#[test]
fn reject_truncated_frame() {
    // Header says 100 bytes but only 5 bytes of payload follow
    let mut buf = Vec::new();
    buf.extend_from_slice(&100u32.to_le_bytes());
    buf.push(0x00);
    buf.extend(vec![0u8; 5]);

    let result = FrameReader::new(Cursor::new(&buf)).read_frame();
    assert!(matches!(result, Err(FrameError::UnexpectedEof)));
}

#[test]
fn reject_reserved_flags() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.push(0b1000_0000); // bit 7 set (reserved)
    buf.push(0xFF);

    let result = FrameReader::new(Cursor::new(&buf)).read_frame();
    assert!(matches!(result, Err(FrameError::ReservedFlagsSet(_))));
}

#[test]
fn wire_format_is_length_flags_payload() {
    let frame = Frame {
        flags: FrameFlags::new(),
        payload: vec![0xAA, 0xBB],
    };
    let mut buf = Vec::new();
    FrameWriter::new(&mut buf).write_frame(&frame).unwrap();

    // [2, 0, 0, 0] (LE u32 length=2) + [0x00] (flags) + [0xAA, 0xBB] (payload)
    assert_eq!(buf, vec![2, 0, 0, 0, 0x00, 0xAA, 0xBB]);
}

#[test]
fn multiple_frames_sequential() {
    let frames = vec![
        Frame { flags: FrameFlags::new(), payload: vec![1] },
        Frame { flags: FrameFlags::new(), payload: vec![2, 3] },
        Frame { flags: FrameFlags::new(), payload: vec![4, 5, 6] },
    ];

    let mut buf = Vec::new();
    let mut writer = FrameWriter::new(&mut buf);
    for f in &frames {
        writer.write_frame(f).unwrap();
    }

    let mut reader = FrameReader::new(Cursor::new(&buf));
    for expected in &frames {
        let decoded = reader.read_frame().unwrap();
        assert_eq!(expected.payload, decoded.payload);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd orix/malt && cargo test -p malt-protocol --test framing`
Expected: FAIL — `FrameFlags`, `FrameReader`, `FrameWriter` don't exist yet.

- [ ] **Step 3: Implement framing.rs**

Replace `orix/malt/crates/malt-protocol/src/framing.rs`:

```rust
//! VNP wire framing — length-prefixed frames with a flags byte.
//!
//! Wire layout: `[4-byte LE payload length] [1-byte flags] [payload]`
//!
//! The payload contains the envelope + message body. Framing is agnostic
//! to payload contents.

use std::io::{self, Read, Write};

/// Maximum protocol frame size (16 MiB). Hard limit, not configurable.
const PROTOCOL_MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

/// Default maximum frame size (64 KiB). Configurable per transport.
const DEFAULT_MAX_FRAME_SIZE: u32 = 64 * 1024;

const FLAG_COMPRESSED: u8 = 1 << 0;
const FLAG_JSON_ENCODED: u8 = 1 << 1;
const FLAG_CONTINUATION: u8 = 1 << 2;
const RESERVED_MASK: u8 = !0b0000_0111;

/// A framed VNP message.
#[derive(Debug, Clone)]
pub struct Frame {
    pub flags: FrameFlags,
    pub payload: Vec<u8>,
}

/// Flags byte for a VNP frame.
///
/// Bit 0: compressed (zstd). Bit 1: JSON-encoded. Bit 2: continuation.
/// Bits 3-7: reserved (must be 0).
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

/// Framing errors.
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

/// Reads VNP frames from a `Read` source.
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
        let max = max.min(PROTOCOL_MAX_FRAME_SIZE);
        Self {
            inner: reader,
            max_frame_size: max,
        }
    }

    pub fn read_frame(&mut self) -> Result<Frame, FrameError> {
        // Read 4-byte LE length
        let mut len_buf = [0u8; 4];
        match self.inner.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Err(FrameError::UnexpectedEof);
            }
            Err(e) => return Err(FrameError::Io(e)),
        }
        let payload_len = u32::from_le_bytes(len_buf);

        if payload_len > self.max_frame_size {
            return Err(FrameError::FrameTooLarge {
                size: payload_len,
                max: self.max_frame_size,
            });
        }

        // Read 1-byte flags
        let mut flags_buf = [0u8; 1];
        match self.inner.read_exact(&mut flags_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Err(FrameError::UnexpectedEof);
            }
            Err(e) => return Err(FrameError::Io(e)),
        }

        if flags_buf[0] & RESERVED_MASK != 0 {
            return Err(FrameError::ReservedFlagsSet(flags_buf[0]));
        }

        let flags = FrameFlags(flags_buf[0]);

        // Read payload
        let mut payload = vec![0u8; payload_len as usize];
        match self.inner.read_exact(&mut payload) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Err(FrameError::UnexpectedEof);
            }
            Err(e) => return Err(FrameError::Io(e)),
        }

        Ok(Frame { flags, payload })
    }
}

/// Writes VNP frames to a `Write` sink.
pub struct FrameWriter<W> {
    inner: W,
}

impl<W: Write> FrameWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { inner: writer }
    }

    pub fn write_frame(&mut self, frame: &Frame) -> Result<(), FrameError> {
        let len = frame.payload.len() as u32;
        self.inner.write_all(&len.to_le_bytes())?;
        self.inner.write_all(&[frame.flags.0])?;
        self.inner.write_all(&frame.payload)?;
        Ok(())
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd orix/malt && cargo test -p malt-protocol --test framing`
Expected: All 8 tests pass.

- [ ] **Step 5: Commit**

```bash
cd orix/malt
git add crates/malt-protocol/src/framing.rs crates/malt-protocol/tests/framing.rs
git commit -m "feat(malt-protocol): implement framing layer — Frame, FrameFlags, FrameReader, FrameWriter"
```

---

## Task 4: Priority Mapping

**Files:**
- Modify: `orix/malt/crates/malt-protocol/src/priority.rs`
- Create: `orix/malt/crates/malt-protocol/tests/priority.rs`

- [ ] **Step 1: Write priority tests**

Create `orix/malt/crates/malt-protocol/tests/priority.rs`:

```rust
use malt_protocol::priority::{priority_of, Priority};

// Domain IDs from schema design spec
const HANDSHAKE: u8 = 0;
const SHELL: u8 = 1;
const INPUT: u8 = 2;
const MUX: u8 = 3;
const SESSION: u8 = 4;
const TASK: u8 = 5;
const RENDER: u8 = 6;
const SYSTEM: u8 = 7;

#[test]
fn input_messages_are_critical() {
    // KeyEvent=0x01, MouseEvent=0x02, SignalInput=0x03, Resize=0x04
    assert_eq!(priority_of(INPUT, 0x01), Some(Priority::Critical));
    assert_eq!(priority_of(INPUT, 0x02), Some(Priority::Critical));
    assert_eq!(priority_of(INPUT, 0x03), Some(Priority::Critical));
    assert_eq!(priority_of(INPUT, 0x04), Some(Priority::Critical));
}

#[test]
fn handshake_messages_are_reliable() {
    assert_eq!(priority_of(HANDSHAKE, 0x01), Some(Priority::Reliable));
    assert_eq!(priority_of(HANDSHAKE, 0x02), Some(Priority::Reliable));
    assert_eq!(priority_of(HANDSHAKE, 0x03), Some(Priority::Reliable));
}

#[test]
fn shell_output_chunk_is_normal() {
    assert_eq!(priority_of(SHELL, 0x04), Some(Priority::Normal));
}

#[test]
fn shell_command_messages_are_reliable() {
    assert_eq!(priority_of(SHELL, 0x01), Some(Priority::Reliable));
    assert_eq!(priority_of(SHELL, 0x02), Some(Priority::Reliable));
    assert_eq!(priority_of(SHELL, 0x03), Some(Priority::Reliable));
}

#[test]
fn render_batch_is_high() {
    assert_eq!(priority_of(RENDER, 0x01), Some(Priority::High));
}

#[test]
fn frame_ack_is_normal() {
    assert_eq!(priority_of(RENDER, 0x02), Some(Priority::Normal));
}

#[test]
fn system_heartbeat_is_low() {
    assert_eq!(priority_of(SYSTEM, 0x04), Some(Priority::Low));
}

#[test]
fn system_error_is_reliable() {
    assert_eq!(priority_of(SYSTEM, 0x05), Some(Priority::Reliable));
}

#[test]
fn unknown_domain_returns_none() {
    assert_eq!(priority_of(15, 0x01), None);
}

#[test]
fn unknown_type_returns_none() {
    assert_eq!(priority_of(SHELL, 0x7F), None);
}
```

- [ ] **Step 2: Implement priority.rs**

Replace `orix/malt/crates/malt-protocol/src/priority.rs`:

```rust
//! Bus priority mapping for VNP message types.
//!
//! Maps (domain, msg_type) from the envelope to a bus priority class.
//! This is a workaround for vexil-lang Gap 2 (no custom annotations).
//! When vexil-lang ships custom annotation support, this hand-written
//! table will be replaced by codegen-emitted constants.

/// Bus priority class for VNP messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Priority {
    /// Resize, Signal — inline delivery, never dropped.
    Critical,
    /// CommandStarted, StructuredOutput — never evicted.
    Reliable,
    /// RenderBatch — oldest overwritten by newer.
    High,
    /// OutputChunk, FrameAck — oldest dropped when full.
    Normal,
    /// Heartbeat, PluginEvent, Diagnostic — oldest dropped when full.
    Low,
}

/// Domain IDs from the VNP schema design spec.
mod domain {
    pub const HANDSHAKE: u8 = 0;
    pub const SHELL: u8 = 1;
    pub const INPUT: u8 = 2;
    pub const MUX: u8 = 3;
    pub const SESSION: u8 = 4;
    pub const TASK: u8 = 5;
    pub const RENDER: u8 = 6;
    pub const SYSTEM: u8 = 7;
}

/// Look up the bus priority for a message by its envelope domain and type.
///
/// Returns `None` for unknown domain/type combinations.
pub const fn priority_of(domain_id: u8, msg_type: u8) -> Option<Priority> {
    match (domain_id, msg_type) {
        // Handshake: all Reliable
        (domain::HANDSHAKE, 0x01..=0x03) => Some(Priority::Reliable),

        // Shell: CommandStarted, CommandFinished, PromptReady = Reliable; OutputChunk = Normal
        (domain::SHELL, 0x01..=0x03) => Some(Priority::Reliable),
        (domain::SHELL, 0x04) => Some(Priority::Normal),

        // Input: all Critical
        (domain::INPUT, 0x01..=0x04) => Some(Priority::Critical),

        // Mux: all Reliable
        (domain::MUX, 0x01..=0x0B) => Some(Priority::Reliable),

        // Session: all Reliable
        (domain::SESSION, 0x01..=0x07) => Some(Priority::Reliable),

        // Task: all Reliable
        (domain::TASK, 0x01..=0x03) => Some(Priority::Reliable),

        // Render: RenderBatch = High, FrameAck = Normal,
        //         InitialState/SyncRequest/SlowClientDisconnect = Reliable,
        //         ScrollbackRequest/Response = Normal
        (domain::RENDER, 0x01) => Some(Priority::High),
        (domain::RENDER, 0x02) => Some(Priority::Normal),
        (domain::RENDER, 0x03..=0x05) => Some(Priority::Reliable),
        (domain::RENDER, 0x06..=0x07) => Some(Priority::Normal),

        // System: StructuredOutput = Reliable, PluginEvent = Low, Diagnostic = Low,
        //         Heartbeat = Low, Error = Reliable
        (domain::SYSTEM, 0x01) => Some(Priority::Reliable),
        (domain::SYSTEM, 0x02..=0x04) => Some(Priority::Low),
        (domain::SYSTEM, 0x05) => Some(Priority::Reliable),

        _ => None,
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cd orix/malt && cargo test -p malt-protocol --test priority`
Expected: All 10 tests pass.

- [ ] **Step 4: Commit**

```bash
cd orix/malt
git add crates/malt-protocol/src/priority.rs crates/malt-protocol/tests/priority.rs
git commit -m "feat(malt-protocol): add priority mapping — const fn (domain, type) → Priority"
```

---

## Task 5: Envelope Helpers

**Files:**
- Modify: `orix/malt/crates/malt-protocol/src/envelope.rs`
- Create: `orix/malt/crates/malt-protocol/tests/envelope_golden.rs`

- [ ] **Step 1: Write the golden-byte test**

Create `orix/malt/crates/malt-protocol/tests/envelope_golden.rs`:

```rust
use malt_protocol::envelope::Envelope;
use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

#[test]
fn envelope_roundtrip() {
    let env = Envelope {
        wire_version: 1,
        domain: 1,
        msg_type: 42,
        session_id: 100,
        timestamp: 1_000_000,
        msg_id: Some(99),
    };

    let mut w = BitWriter::new();
    env.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = Envelope::unpack(&mut r).unwrap();
    assert_eq!(env.wire_version, decoded.wire_version);
    assert_eq!(env.domain, decoded.domain);
    assert_eq!(env.msg_type, decoded.msg_type);
    assert_eq!(env.session_id, decoded.session_id);
    assert_eq!(env.timestamp, decoded.timestamp);
    assert_eq!(env.msg_id, decoded.msg_id);
}

#[test]
fn envelope_without_msg_id() {
    let env = Envelope {
        wire_version: 0,
        domain: 7,
        msg_type: 127,
        session_id: 0,
        timestamp: 0,
        msg_id: None,
    };

    let mut w = BitWriter::new();
    env.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = Envelope::unpack(&mut r).unwrap();
    assert_eq!(decoded.msg_id, None);
}

#[test]
fn envelope_golden_bytes() {
    // Known envelope:
    //   wire_version=1 (4 bits: 0001)
    //   domain=2       (4 bits: 0010)
    //   msg_type=5     (7 bits: 0000101)
    //   session_id=42  (32 bits LE)
    //   timestamp=1000 (48 bits LE)
    //   msg_id=None    (1 bit: 0)
    //
    // Bit layout (LSB-first within bytes):
    //   Byte 0: wire_version(4) + domain(4) = 0001 | 0010 = 0x21
    //   Byte 1: msg_type(7) + msg_id presence(1) = 0000101 | 0 = 0x0A
    //     (7 bits of msg_type in bits 0-6, presence bit in bit 7)
    //   Bytes 2-5: session_id LE = 42, 0, 0, 0
    //   Bytes 6-11: timestamp LE (48-bit) = 0xE8, 0x03, 0, 0, 0, 0
    //
    // Total: 12 bytes when msg_id absent

    let env = Envelope {
        wire_version: 1,
        domain: 2,
        msg_type: 5,
        session_id: 42,
        timestamp: 1000,
        msg_id: None,
    };

    let mut w = BitWriter::new();
    env.pack(&mut w).unwrap();
    let bytes = w.finish();

    // Verify the first byte: wire_version=1 in low 4 bits, domain=2 in high 4 bits
    assert_eq!(bytes[0], 0x21, "byte 0: wire_version(1) | domain(2) << 4");

    // Verify roundtrip produces identical bytes
    let mut w2 = BitWriter::new();
    env.pack(&mut w2).unwrap();
    assert_eq!(bytes, w2.finish(), "deterministic encoding");
}
```

- [ ] **Step 2: Implement envelope.rs**

Replace `orix/malt/crates/malt-protocol/src/envelope.rs`:

```rust
//! Envelope encode/decode helpers.
//!
//! Re-exports the Vexil-generated `Envelope` type and adds convenience
//! functions for the common encode/decode pattern.

pub use crate::envelope_types::Envelope;

use vexil_runtime::{BitReader, BitWriter, DecodeError, Pack, Unpack};

/// Encode an envelope + message payload into bytes suitable for framing.
pub fn encode_message(envelope: &Envelope, payload: &[u8]) -> Vec<u8> {
    let mut w = BitWriter::new();
    envelope.pack(&mut w).expect("envelope encoding should not fail");
    let mut bytes = w.finish();
    bytes.extend_from_slice(payload);
    bytes
}

/// Decode an envelope from the front of a frame payload buffer.
/// Returns the envelope and the remaining bytes (message body).
pub fn decode_envelope(data: &[u8]) -> Result<(Envelope, &[u8]), DecodeError> {
    let mut r = BitReader::new(data);
    let envelope = Envelope::unpack(&mut r)?;
    let consumed = data.len() - r.remaining();
    Ok((envelope, &data[consumed..]))
}

// Private module alias for the generated envelope type.
// The generated code lives at crate::envelope (the module generated by vexilc),
// but we re-export it above as `Envelope` for convenience.
use crate::envelope as envelope_types;
```

- [ ] **Step 3: Fix lib.rs re-export to avoid module name collision**

The generated code creates a `malt::envelope` module, and we have a hand-written `src/envelope.rs`. These conflict. We need to rename one. Update `src/lib.rs`:

```rust
//! VNP message types, wire framing, and protocol primitives for MALT.
//!
//! This crate is the L0 foundation — every other MALT crate depends on it.
//! Message types are generated from `.vexil` schemas by `vexilc` at build time.

pub mod framing;
pub mod priority;

// Generated code from vexilc — all domain modules (common, shell, render, etc.)
#[allow(clippy::all, unused_qualifications)]
mod generated;

// Re-export generated domain modules at crate root
pub use generated::malt::common;
pub use generated::malt::handshake;
pub use generated::malt::shell;
pub use generated::malt::input;
pub use generated::malt::mux;
pub use generated::malt::session;
pub use generated::malt::task;
pub use generated::malt::render;
pub use generated::malt::frame_element;
pub use generated::malt::system;
pub use generated::malt::elevate;
pub use generated::malt::persist;

// Envelope: hand-written helpers wrapping the generated type
pub mod envelope;
// Make the generated envelope module accessible for the hand-written helpers
use generated::malt::envelope as envelope_generated;
```

Then update `src/envelope.rs` to use the correct path:

```rust
//! Envelope encode/decode helpers.

pub use crate::envelope_generated::Envelope;

use vexil_runtime::{BitReader, BitWriter, DecodeError, Pack, Unpack};

/// Encode an envelope + message payload into bytes suitable for framing.
pub fn encode_message(envelope: &Envelope, payload: &[u8]) -> Vec<u8> {
    let mut w = BitWriter::new();
    envelope.pack(&mut w).expect("envelope encoding should not fail");
    let mut bytes = w.finish();
    bytes.extend_from_slice(payload);
    bytes
}

/// Decode an envelope from the front of a frame payload buffer.
/// Returns the envelope and the remaining bytes (message body).
pub fn decode_envelope(data: &[u8]) -> Result<(Envelope, &[u8]), DecodeError> {
    let mut r = BitReader::new(data);
    let envelope = Envelope::unpack(&mut r)?;
    let consumed = data.len() - r.remaining();
    Ok((envelope, &data[consumed..]))
}
```

**Note:** The exact module paths may need adjustment based on how vexilc generates the code. The implementer should verify that `generated::malt::envelope::Envelope` exists after the build and adjust the `use` path accordingly. The `BitReader::remaining()` method must also be verified against the vexil-runtime API — it may be called `remaining()` or `remaining_bytes()`.

- [ ] **Step 4: Run tests**

Run: `cd orix/malt && cargo test -p malt-protocol --test envelope_golden`
Expected: All 3 tests pass.

- [ ] **Step 5: Commit**

```bash
cd orix/malt
git add crates/malt-protocol/src/envelope.rs crates/malt-protocol/src/lib.rs \
        crates/malt-protocol/tests/envelope_golden.rs
git commit -m "feat(malt-protocol): add envelope helpers with golden-byte verification"
```

---

## Task 6: Message Roundtrip Tests

**Files:**
- Create: `orix/malt/crates/malt-protocol/tests/roundtrip.rs`

- [ ] **Step 1: Write roundtrip tests**

Create `orix/malt/crates/malt-protocol/tests/roundtrip.rs`:

```rust
//! Roundtrip encode/decode tests for representative message types.

use vexil_runtime::{BitReader, BitWriter, Pack, Unpack};

#[test]
fn shell_command_started_roundtrip() {
    let msg = malt_protocol::shell::CommandStarted {
        command_id: 42,
        cmd: "cargo build --release".to_string(),
    };

    let mut w = BitWriter::new();
    msg.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = malt_protocol::shell::CommandStarted::unpack(&mut r).unwrap();
    assert_eq!(msg.command_id, decoded.command_id);
    assert_eq!(msg.cmd, decoded.cmd);
}

#[test]
fn common_pane_id_roundtrip() {
    let id = malt_protocol::common::PaneId(12345);

    let mut w = BitWriter::new();
    id.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = malt_protocol::common::PaneId::unpack(&mut r).unwrap();
    assert_eq!(id.0, decoded.0);
}

#[test]
fn common_isolation_tier_roundtrip() {
    let tier = malt_protocol::common::IsolationTier::Contained;

    let mut w = BitWriter::new();
    tier.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = malt_protocol::common::IsolationTier::unpack(&mut r).unwrap();
    assert_eq!(tier, decoded);
}

#[test]
fn input_key_event_roundtrip() {
    let msg = malt_protocol::input::KeyEvent {
        key: malt_protocol::input::KeyValue::Char {
            codepoint: 0x41, // 'A'
        },
        modifiers: malt_protocol::common::KeyModifiers(0), // no modifiers
    };

    let mut w = BitWriter::new();
    msg.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = malt_protocol::input::KeyEvent::unpack(&mut r).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn session_create_roundtrip() {
    let msg = malt_protocol::session::CreateSession {
        name: Some("dev".to_string()),
        isolation: malt_protocol::common::IsolationTier::Restricted,
        group: None,
    };

    let mut w = BitWriter::new();
    msg.pack(&mut w).unwrap();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    let decoded = malt_protocol::session::CreateSession::unpack(&mut r).unwrap();
    assert_eq!(msg, decoded);
}
```

**Note:** The exact generated struct/enum field names and union variant syntax may differ from the plan. The implementer must adjust based on actual vexilc output. For example, union variants might be `KeyValue::Char { codepoint: 0x41 }` or `KeyValue::Char(CharVariant { codepoint: 0x41 })` depending on codegen style. Check the generated code in `$OUT_DIR`.

- [ ] **Step 2: Run tests**

Run: `cd orix/malt && cargo test -p malt-protocol --test roundtrip`
Expected: All 5 tests pass.

- [ ] **Step 3: Commit**

```bash
cd orix/malt
git add crates/malt-protocol/tests/roundtrip.rs
git commit -m "test(malt-protocol): add message roundtrip tests for 5 representative types"
```

---

## Verification

After all tasks are done, confirm:

1. `cd orix/malt && cargo test -p malt-protocol` — all tests pass
2. `cd orix/malt && cargo clippy -p malt-protocol -- -D warnings` — clean (generated code may need `#[allow]`)
3. Generated types accessible: `malt_protocol::common::PaneId`, `malt_protocol::shell::OutputChunk`, `malt_protocol::render::RenderCommand`, etc.
4. Framing: encode → decode roundtrip, edge cases covered
5. Priority: exhaustive mapping for all defined message types
6. Envelope: golden-byte test passes, encode_message/decode_envelope work
