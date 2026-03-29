# Phase 0: Protocol Validation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Validate that Vexil bitpack encoding can serve as the VNP wire format — deterministic, evolvable, and competitively performant — or identify the point where it fails and pivot to an existing format.

**Architecture:** Phase 0 works entirely within the existing `orix/vexil-lang/` workspace. No MALT crate scaffolding. Deliverables are: (1) formal encoding edge-case spec additions, (2) schema evolution stress tests, (3) golden byte compliance vectors, (4) TypeScript decoder passing the same vectors, (5) benchmarks, (6) written go/no-go justification.

**Tech Stack:** Rust (vexil-lang workspace), TypeScript (`packages/runtime-ts/` in vexil-lang workspace), criterion (benchmarks), insta (snapshot tests)

**Spec reference:** `malt/specs/architecture.md` §16 Phase 0 checklist.

---

## File Structure

> **Validated 2026-03-29.** Items marked ✅ exist in `orix/vexil-lang/` at v0.4.2.

```
orix/vexil-lang/                         # Existing workspace (v0.4.2) — all Phase 0 work lands here
  spec/
    vexil-spec.md                        # ✅ Has §11 Encoding Edge Cases (§1–§14 + 2 appendices)
  corpus/
    valid/
      019_evolution_append_field.vexil   # ✅ All 8 schemas exist (plus 027, 028 added beyond plan)
      020_evolution_add_variant.vexil    # ✅
      021_empty_optionals.vexil          # ✅
      022_nested_schemas.vexil           # ✅
      023_recursive_depth.vexil          # ✅
      024_zero_length_payload.vexil      # ✅
      025_evolution_deprecate.vexil      # ✅
      026_required_to_optional.vexil     # ✅
      027_delta_on_message.vexil         # ✅ (beyond original plan)
      028_typed_tombstone.vexil          # ✅ (beyond original plan)
    MANIFEST.md                          # ✅ Updated (28 valid, 56 invalid)
  crates/
    vexil-runtime/
      src/
        bit_writer.rs                    # ✅ Has recursion_depth, enter_recursive(), leave_recursive()
        bit_reader.rs                    # ✅ Has recursion_depth, enter_recursive(), leave_recursive(), trailing byte tolerance
        error.rs                         # ✅ Has MaxRecursionDepth variants
    vexil-codegen-rust/
      tests/
        golden/                          # ✅ Golden files for corpus schemas
        evolution_roundtrip.rs           # ✅ Cross-version encode/decode tests
        golden_bytes.rs                  # ✅ Golden byte vector tests
        delta_compliance.rs              # ✅ (beyond original plan)
        union_array_roundtrip.rs         # ✅ (beyond original plan)
    vexil-codegen-ts/                    # ✅ Rust crate that generates TypeScript code from schemas
    vexil-bench/                         # ✅ Benchmark crate (workspace member)
      Cargo.toml
      benches/
        encode_decode.rs                 # ✅ Criterion benchmarks
      src/
        lib.rs
        messages.rs                      # ✅ VNP-representative message types
  compliance/                            # ✅ Golden byte vectors
    vectors/
      README.md                          # ✅
      primitives.json                    # ✅
      sub_byte.json                      # ✅
      messages.json                      # ✅
      enums.json                         # ✅
      unions.json                        # ✅
      optionals.json                     # ✅
      arrays_maps.json                   # ✅
      evolution.json                     # ✅
      delta.json                         # ✅ (beyond original plan)
  packages/
    runtime-ts/                          # ✅ TypeScript runtime (@vexil-lang/runtime)
      src/
        bit-reader.ts                    # ✅ BitReader (LSB-first)
        bit-writer.ts                    # ✅ BitWriter (LSB-first)
        handshake.ts                     # ✅
        index.ts                         # ✅
      tests/
        compliance.test.ts               # TODO: add golden vector compliance tests
      package.json                       # ✅ Published as @vexil-lang/runtime
      vitest.config.ts                   # ✅

orix/malt/
  specs/
    phase0-justification.md              # TODO: go/no-go writeup after benchmarks
```

---

## Task 1: Encoding Edge Cases — Spec Additions ✅ COMPLETE

> **Validated 2026-03-29:** §11 "Encoding Edge Cases" exists at line 814 of `vexil-spec.md`. Spec now has 14 sections plus 2 appendices (A: Wire Envelope Format, B: Future Work).

**Files:**
- ~~Modify:~~ `orix/vexil-lang/spec/vexil-spec.md` — **done**

Document the edge cases that a VNP implementation must handle correctly. These are normative additions to the Vexil spec.

- [x] **Step 1: Read the current spec to find the right insertion point**

Run: `wc -l orix/vexil-lang/spec/vexil-spec.md` and read the last section.

- [x] **Step 2: Write the edge case appendix**

Append a new section to the spec covering:

```markdown
## 11. Encoding Edge Cases (Normative)

### 11.1 Empty Optionals

An `optional<T>` with no value encodes as a single 0 bit. No payload follows.
An `optional<T>` with a value encodes as a 1 bit followed by T's encoding.

For nested optionals (`optional<optional<T>>`):
- None → 0 (1 bit)
- Some(None) → 1 0 (2 bits)
- Some(Some(v)) → 1 1 [v encoding]

### 11.2 Zero-Length Payloads

A message with zero fields encodes as zero bytes (empty payload).
A union variant with no fields encodes as: discriminant (LEB128) + length 0 (LEB128).

### 11.3 Maximum Recursion Depth

Recursive types (self-referencing messages via optional or array) have a
maximum nesting depth of 64 at encode and decode time. Exceeding this depth
is an error, not undefined behavior.

Encoder: returns EncodeError::MaxDepthExceeded.
Decoder: returns DecodeError::MaxDepthExceeded.

### 11.4 Trailing Bytes

When a decoder has consumed all declared fields of a message, any remaining
bytes in the payload are ignored. This enables forward compatibility — a v2
encoder may append new fields that a v1 decoder skips.

Decoders MUST NOT reject messages with trailing bytes after the last known
field. Decoders MUST NOT interpret trailing bytes.

### 11.5 Sub-Byte Boundary at Message End

After encoding all fields, the encoder calls flush_to_byte_boundary(). Padding
bits MUST be zero. The decoder calls flush_to_byte_boundary() after reading all
known fields, before checking for trailing bytes.

### 11.6 Union Discriminant Overflow

If a decoder encounters a union discriminant value that doesn't match any known
variant:
- If the union is @non_exhaustive: decode as Unknown { discriminant, data }
- If the union is exhaustive: return DecodeError::UnknownVariant

The length-prefixed payload enables skipping unknown variants without knowing
their structure.

### 11.7 NaN Canonicalization

All f32 NaN values encode as 0x7FC00000 (canonical quiet NaN).
All f64 NaN values encode as 0x7FF8000000000000 (canonical quiet NaN).
Signaling NaN, negative NaN, and NaN with payload are all mapped to canonical
qNaN before encoding. This ensures bit-identical encoding for any NaN input.

### 11.8 Negative Zero

-0.0 is preserved on the wire (distinct from +0.0). IEEE 754 defines -0.0 == +0.0,
but their bit patterns differ. Vexil preserves the bit pattern for determinism.

### 11.9 String Encoding Errors

String fields use UTF-8. An encoder receiving non-UTF-8 data returns
EncodeError::InvalidUtf8. A decoder encountering invalid UTF-8 in a string
field returns DecodeError::InvalidUtf8. Bytes fields have no encoding
restriction.

### 11.10 Schema Evolution Compatibility Rules

Adding a field to a message (new ordinal, appended in declaration order):
- v1 encoder → v2 decoder: v2 decoder reads known fields, new field gets
  its default value (zero/empty/None depending on type)
- v2 encoder → v1 decoder: v1 decoder reads its known fields, ignores
  trailing bytes (§11.4)

Adding a variant to a @non_exhaustive union:
- v2 encoder → v1 decoder: v1 decoder reads discriminant, doesn't recognize
  it, reads length-prefixed payload as Unknown { discriminant, data }
- v1 encoder → v2 decoder: works unchanged (old variants still valid)

Deprecating a field (marking @deprecated, field still encoded):
- No wire change. @deprecated is a source-level annotation only — the field
  is still encoded and decoded normally. Generated code emits a deprecation
  warning. The ordinal remains reserved and MUST NOT be reused.

Changing a required field to optional<T> (**BREAKING**):
- This changes the wire encoding (the field now has a 1-bit presence flag
  before T's encoding). Wire version bump required. A v1 decoder reading
  v2-encoded data would misinterpret the presence flag as part of the field
  value.
- This is listed in the Vexil spec's breaking changes table (§10) and is
  verified by a golden byte test showing the wire layout difference.
```

- [x] **Step 3: Commit**

```bash
git add spec/vexil-spec.md
git commit -m "spec: add §11 encoding edge cases (normative)"
```

---

## Task 2: Edge Case Corpus Schemas ✅ COMPLETE

> **Validated 2026-03-29:** All 8 schemas exist (019–026), plus 027 (`delta_on_message`) and 028 (`typed_tombstone`) were added beyond the original plan. MANIFEST.md covers 28 valid + 56 invalid schemas.

**Files:**
- ~~Create:~~ `orix/vexil-lang/corpus/valid/019_evolution_append_field.vexil` — **done**
- ~~Create:~~ `orix/vexil-lang/corpus/valid/020_evolution_add_variant.vexil` — **done**
- ~~Create:~~ `orix/vexil-lang/corpus/valid/021_empty_optionals.vexil` — **done**
- ~~Create:~~ `orix/vexil-lang/corpus/valid/022_nested_schemas.vexil` — **done**
- ~~Create:~~ `orix/vexil-lang/corpus/valid/023_recursive_depth.vexil` — **done**
- ~~Create:~~ `orix/vexil-lang/corpus/valid/024_zero_length_payload.vexil` — **done**
- ~~Create:~~ `orix/vexil-lang/corpus/valid/025_evolution_deprecate.vexil` — **done**
- ~~Create:~~ `orix/vexil-lang/corpus/valid/026_required_to_optional.vexil` — **done**
- ~~Modify:~~ `orix/vexil-lang/corpus/MANIFEST.md` — **done**

- [x] **Step 1: Write 019_evolution_append_field.vexil**

```vexil
namespace test.evolution.append

@version "1.0.0"

// V1 message — baseline
message HeaderV1 {
    kind   @0 : u8
    status @1 : u8
}

// V2 message — field appended (compatible change)
message HeaderV2 {
    kind   @0 : u8
    status @1 : u8
    flags  @2 : u16
}
```

- [x] **Step 2: Write 020_evolution_add_variant.vexil**

```vexil
namespace test.evolution.variant

@version "1.0.0"

@non_exhaustive
union ShapeV1 {
    Circle { radius @0 : f32 } @0
    Rect { w @0 : f32  h @1 : f32 } @1
}

@non_exhaustive
union ShapeV2 {
    Circle { radius @0 : f32 } @0
    Rect { w @0 : f32  h @1 : f32 } @1
    Triangle { base @0 : f32  height @1 : f32 } @2
}
```

- [x] **Step 3: Write 021_empty_optionals.vexil**

```vexil
namespace test.empty.optionals

message WithOptionals {
    name  @0 : optional<string>
    value @1 : optional<u32>
    flag  @2 : optional<bool>
}

message NestedOptional {
    inner @0 : optional<optional<u32>>
}

message AllEmpty {
    a @0 : optional<string>
    b @1 : optional<u32>
    c @2 : optional<bool>
}
```

- [x] **Step 4: Write 022_nested_schemas.vexil**

```vexil
namespace test.nested.schemas

message Coord {
    x @0 : i32
    y @1 : i32
}

message Rect {
    origin @0 : Coord
    size   @1 : Coord
}

message Canvas {
    bounds @0 : Rect
    name   @1 : string
    layers @2 : array<Rect>
}
```

- [x] **Step 5: Write 023_recursive_depth.vexil**

```vexil
namespace test.recursive.depth

message TreeNode {
    value    @0 : u32
    children @1 : array<TreeNode>
}

message LinkedList {
    value @0 : i64
    next  @1 : optional<LinkedList>
}
```

- [x] **Step 6: Write 024_zero_length_payload.vexil**

```vexil
namespace test.zero.payload

message Empty {}

message Wrapper {
    inner @0 : Empty
    tag   @1 : u8
}

union Event {
    Ping {} @0
    Pong {} @1
    Data { payload @0 : bytes } @2
}
```

- [x] **Step 7: Write 025_evolution_deprecate.vexil**

```vexil
namespace test.evolution.deprecate

@version "2.0.0"

message Config {
    name     @0 : string
    @deprecated "use name_v2 instead"
    old_name @1 : string
    timeout  @2 : u32
}
```

- [x] **Step 8: Write 026_required_to_optional.vexil**

These two messages demonstrate the breaking wire change when a required field becomes optional. V1 has a required u32; V2 wraps it in optional. The wire layout differs (presence bit inserted).

```vexil
namespace test.evolution.reqopt

// V1: required field
message SettingsV1 {
    timeout @0 : u32
    name    @1 : string
}

// V2: timeout becomes optional (BREAKING — wire layout changes)
message SettingsV2 {
    timeout @0 : optional<u32>
    name    @1 : string
}
```

- [x] **Step 9: Update MANIFEST.md with all 8 new entries**

Append to the valid section:

```markdown
| 019_evolution_append_field | §11.10 | Schema evolution: field appended to message |
| 020_evolution_add_variant  | §11.10 | Schema evolution: variant added to @non_exhaustive union |
| 021_empty_optionals        | §11.1  | Empty and nested optional encoding |
| 022_nested_schemas         | §4.1   | Nested message references, arrays of messages |
| 023_recursive_depth        | §11.3  | Self-recursive and mutual recursive types |
| 024_zero_length_payload    | §11.2  | Empty messages, empty union variants |
| 025_evolution_deprecate    | §11.10 | Schema evolution: deprecated field (no wire change) |
| 026_required_to_optional   | §11.10 | Breaking change: required→optional wire layout difference |
```

- [x] **Step 10: Run existing tests to verify new schemas compile**

Run: `cd orix/vexil-lang && cargo test --workspace`
Expected: All existing tests pass; new corpus files are picked up by glob-based test harness.

- [x] **Step 11: Commit**

```bash
git add corpus/valid/019_*.vexil corpus/valid/020_*.vexil corpus/valid/021_*.vexil \
        corpus/valid/022_*.vexil corpus/valid/023_*.vexil corpus/valid/024_*.vexil \
        corpus/valid/025_*.vexil corpus/valid/026_*.vexil \
        corpus/MANIFEST.md
git commit -m "corpus: add 8 schemas for encoding edge cases and evolution"
```

---

## Task 3: Recursion Depth Tracking in Runtime ✅ COMPLETE

> **Validated 2026-03-29:** `BitWriter` and `BitReader` both have `recursion_depth: u32` fields with `enter_recursive()`/`leave_recursive()` methods. `MAX_RECURSION_DEPTH` = 64 in `lib.rs`. Error variants `EncodeError::MaxRecursionDepth` and `DecodeError::MaxRecursionDepth` in `error.rs`.
>
> **Note:** Actual method names are `enter_recursive()`/`leave_recursive()`, not `enter_nested()`/`leave_nested()` as originally planned below. The code examples below are preserved for reference but the real API uses the `_recursive` suffix.

**Files:**
- ~~Modify:~~ `orix/vexil-lang/crates/vexil-runtime/src/bit_writer.rs` — **done**
- ~~Modify:~~ `orix/vexil-lang/crates/vexil-runtime/src/bit_reader.rs` — **done**
- ~~Modify:~~ `orix/vexil-lang/crates/vexil-runtime/src/error.rs` (error types) — **done**

The spec says max recursion depth is 64. The runtime now enforces this via `enter_recursive()` / `leave_recursive()`.

- [x] **Step 1: Read current BitWriter and BitReader to understand structure**

Read: `orix/vexil-lang/crates/vexil-runtime/src/bit_writer.rs`
Read: `orix/vexil-lang/crates/vexil-runtime/src/bit_reader.rs`
Read: `orix/vexil-lang/crates/vexil-runtime/src/lib.rs`

- [x] **Step 2: Write failing test for depth tracking**

Add to `orix/vexil-lang/crates/vexil-runtime/src/bit_writer.rs` (or a test module):

```rust
#[cfg(test)]
mod depth_tests {
    use super::*;

    #[test]
    fn depth_increment_decrement() {
        let mut w = BitWriter::new();
        assert_eq!(w.depth(), 0);
        w.enter_nested().unwrap();
        assert_eq!(w.depth(), 1);
        w.leave_nested();
        assert_eq!(w.depth(), 0);
    }

    #[test]
    fn depth_max_64_exceeded() {
        let mut w = BitWriter::new();
        for _ in 0..64 {
            w.enter_nested().unwrap();
        }
        assert!(w.enter_nested().is_err());
    }
}
```

- [x] **Step 3: Run test to verify it fails**

Run: `cd orix/vexil-lang && cargo test -p vexil-runtime depth_tests`
Expected: FAIL — `enter_nested` method doesn't exist yet.

- [x] **Step 4: Implement depth tracking in BitWriter**

Add to `BitWriter`:

```rust
const MAX_DEPTH: u32 = 64;

// In BitWriter struct:
depth: u32,

// New methods:
pub fn enter_nested(&mut self) -> Result<(), EncodeError> {
    if self.depth >= MAX_DEPTH {
        return Err(EncodeError::MaxDepthExceeded);
    }
    self.depth += 1;
    Ok(())
}

pub fn leave_nested(&mut self) {
    self.depth = self.depth.saturating_sub(1);
}

pub fn depth(&self) -> u32 {
    self.depth
}
```

Initialize `depth: 0` in `BitWriter::new()`.

- [x] **Step 5: Implement matching depth tracking in BitReader**

Same pattern:

```rust
// In BitReader struct:
depth: u32,

pub fn enter_nested(&mut self) -> Result<(), DecodeError> {
    if self.depth >= MAX_DEPTH {
        return Err(DecodeError::MaxDepthExceeded);
    }
    self.depth += 1;
    Ok(())
}

pub fn leave_nested(&mut self) {
    self.depth = self.depth.saturating_sub(1);
}
```

- [x] **Step 6: Add error variants if missing**

Check if `EncodeError::MaxDepthExceeded` and `DecodeError::MaxDepthExceeded` exist. If not, add them to the error enums in `lib.rs`.

> **Actual:** Error variants are `EncodeError::MaxRecursionDepth` and `DecodeError::MaxRecursionDepth` in `error.rs`.

- [x] **Step 7: Run tests**

Run: `cd orix/vexil-lang && cargo test -p vexil-runtime`
Expected: All pass, including new depth tests.

- [x] **Step 8: Commit**

```bash
git add crates/vexil-runtime/
git commit -m "feat(vexil-runtime): add recursion depth tracking (max 64)"
```

---

## Task 4: Codegen Emits Depth Tracking for Recursive Types ✅ COMPLETE

> **Validated 2026-03-29:** The codegen emits `enter_recursive()`/`leave_recursive()` calls (not `enter_nested()`/`leave_nested()` as originally planned). Golden files updated.

**Files:**
- ~~Modify:~~ `orix/vexil-lang/crates/vexil-codegen-rust/src/lib.rs` — **done**
- ~~Modify:~~ golden files — **done**

The Rust codegen backend emits `enter_recursive()` / `leave_recursive()` calls in Pack/Unpack implementations for types that contain self-references (direct or via optional/array).

- [x] **Step 1: Read the current codegen to understand how Pack/Unpack are generated**

- [x] **Step 2: Write failing test — compile and run the recursive schema with deep nesting**

- [x] **Step 3: Modify codegen to emit depth tracking**

> **Actual API:** Codegen emits `enter_recursive()`/`leave_recursive()` (not `enter_nested()`/`leave_nested()`).

- [x] **Step 4: Regenerate golden files**

- [x] **Step 5: Verify recursive golden file now includes depth tracking calls**

- [x] **Step 6: Run all tests**

- [x] **Step 7: Commit**

```bash
git add crates/vexil-codegen-rust/
git commit -m "feat(vexil-codegen-rust): emit depth tracking for recursive types"
```

---

## Task 5: Golden Byte Compliance Vectors ✅ COMPLETE

> **Validated 2026-03-29:** All 8 planned JSON files exist plus `delta.json` (9 total). `compliance/vectors/README.md` present.

**Files:**
- ~~Create:~~ `orix/vexil-lang/compliance/vectors/README.md` — **done**
- ~~Create:~~ `orix/vexil-lang/compliance/vectors/primitives.json` — **done**
- ~~Create:~~ `orix/vexil-lang/compliance/vectors/sub_byte.json` — **done**
- ~~Create:~~ `orix/vexil-lang/compliance/vectors/messages.json` — **done**
- ~~Create:~~ `orix/vexil-lang/compliance/vectors/enums.json` — **done**
- ~~Create:~~ `orix/vexil-lang/compliance/vectors/unions.json` — **done**
- ~~Create:~~ `orix/vexil-lang/compliance/vectors/optionals.json` — **done**
- ~~Create:~~ `orix/vexil-lang/compliance/vectors/arrays_maps.json` — **done**
- ~~Create:~~ `orix/vexil-lang/compliance/vectors/evolution.json` — **done**

Golden byte vectors are the compliance contract. Every conformant implementation must produce identical bytes for the same input and reconstruct identical values from the same bytes.

- [x] **Step 1: Write the vector format README**

```markdown
# Vexil Compliance Vectors

Each JSON file contains an array of test vectors. Each vector has:

```json
{
  "name": "human-readable test name",
  "schema": "inline Vexil schema text",
  "type": "the message/enum/union type name to encode",
  "value": { "field": "value", ... },
  "expected_bytes": "hex-encoded expected output",
  "notes": "optional explanation"
}
```

## Generating Expected Bytes

Expected bytes are generated by the Rust reference implementation and verified
by hand for correctness. The `validate.rs` script re-encodes each vector and
asserts byte-identical output.

## Cross-Implementation Testing

Any conformant Vexil implementation MUST:
1. Encode each vector's `value` and produce `expected_bytes`
2. Decode `expected_bytes` and produce `value`

Failure on any vector is a conformance failure.
```

- [x] **Step 2: Write the Rust vector generator**

This is a Rust binary/test that takes known inputs, encodes them with the reference implementation, and outputs the JSON vectors. We'll start by writing the vectors by hand for the simplest cases, then build a generator for the complex ones.

Create `orix/vexil-lang/compliance/vectors/primitives.json`:

```json
[
  {
    "name": "bool_false",
    "schema": "namespace test.prim\nmessage M { v @0 : bool }",
    "type": "M",
    "value": { "v": false },
    "expected_bytes": "00",
    "notes": "bool false = 0 bit, flush to byte = 0x00"
  },
  {
    "name": "bool_true",
    "schema": "namespace test.prim\nmessage M { v @0 : bool }",
    "type": "M",
    "value": { "v": true },
    "expected_bytes": "01",
    "notes": "bool true = 1 bit, flush to byte = 0x01"
  },
  {
    "name": "u8_zero",
    "schema": "namespace test.prim\nmessage M { v @0 : u8 }",
    "type": "M",
    "value": { "v": 0 },
    "expected_bytes": "00"
  },
  {
    "name": "u8_max",
    "schema": "namespace test.prim\nmessage M { v @0 : u8 }",
    "type": "M",
    "value": { "v": 255 },
    "expected_bytes": "ff"
  },
  {
    "name": "u16_le",
    "schema": "namespace test.prim\nmessage M { v @0 : u16 }",
    "type": "M",
    "value": { "v": 258 },
    "expected_bytes": "0201",
    "notes": "258 = 0x0102, little-endian = 02 01"
  },
  {
    "name": "u32_le",
    "schema": "namespace test.prim\nmessage M { v @0 : u32 }",
    "type": "M",
    "value": { "v": 305419896 },
    "expected_bytes": "78563412",
    "notes": "0x12345678, little-endian = 78 56 34 12"
  },
  {
    "name": "i32_negative",
    "schema": "namespace test.prim\nmessage M { v @0 : i32 }",
    "type": "M",
    "value": { "v": -1 },
    "expected_bytes": "ffffffff",
    "notes": "-1 in two's complement i32"
  },
  {
    "name": "f32_nan_canonical",
    "schema": "namespace test.prim\nmessage M { v @0 : f32 }",
    "type": "M",
    "value": { "v": "NaN" },
    "expected_bytes": "0000c07f",
    "notes": "canonical qNaN 0x7FC00000, little-endian = 00 00 C0 7F"
  },
  {
    "name": "f64_negative_zero",
    "schema": "namespace test.prim\nmessage M { v @0 : f64 }",
    "type": "M",
    "value": { "v": "-0.0" },
    "expected_bytes": "0000000000000080",
    "notes": "-0.0 = 0x8000000000000000, little-endian"
  },
  {
    "name": "string_hello",
    "schema": "namespace test.prim\nmessage M { v @0 : string }",
    "type": "M",
    "value": { "v": "hello" },
    "expected_bytes": "0568656c6c6f",
    "notes": "LEB128 length 5 + UTF-8 'hello'"
  },
  {
    "name": "string_empty",
    "schema": "namespace test.prim\nmessage M { v @0 : string }",
    "type": "M",
    "value": { "v": "" },
    "expected_bytes": "00",
    "notes": "LEB128 length 0, no payload"
  }
]
```

- [x] **Step 3: Write sub_byte.json**

```json
[
  {
    "name": "u3_u5_packed",
    "schema": "namespace test.sub\nmessage M { a @0 : u3  b @1 : u5 }",
    "type": "M",
    "value": { "a": 5, "b": 18 },
    "expected_bytes": "95",
    "notes": "a=101(3bit) b=10010(5bit) → byte: 10010_101 = 0x95"
  },
  {
    "name": "u3_u5_u6_cross_byte",
    "schema": "namespace test.sub\nmessage M { a @0 : u3  b @1 : u5  c @2 : u6 }",
    "type": "M",
    "value": { "a": 7, "b": 31, "c": 63 },
    "expected_bytes": "fffc",
    "notes": "a=111 b=11111 c=111111 → bits: 11111_111 | 00_111111 → FF FC"
  },
  {
    "name": "u1_bool_equivalent",
    "schema": "namespace test.sub\nmessage M { v @0 : u1 }",
    "type": "M",
    "value": { "v": 1 },
    "expected_bytes": "01"
  }
]
```

- [x] **Step 4: Write messages.json with multi-field and nested messages**

```json
[
  {
    "name": "empty_message",
    "schema": "namespace test.msg\nmessage Empty {}",
    "type": "Empty",
    "value": {},
    "expected_bytes": "",
    "notes": "Zero fields = zero bytes"
  },
  {
    "name": "two_u32_fields",
    "schema": "namespace test.msg\nmessage M { x @0 : u32  y @1 : u32 }",
    "type": "M",
    "value": { "x": 1, "y": 2 },
    "expected_bytes": "0100000002000000"
  },
  {
    "name": "mixed_types",
    "schema": "namespace test.msg\nmessage M { flag @0 : bool  count @1 : u16  name @2 : string }",
    "type": "M",
    "value": { "flag": true, "count": 42, "name": "test" },
    "expected_bytes": "012a000474657374",
    "notes": "bool(1 bit, flush) + u16(LE) + string(LEB128 len + bytes)"
  }
]
```

- [x] **Step 5: Write optionals.json, unions.json, enums.json, arrays_maps.json**

Each file follows the same pattern with increasing complexity. Key vectors:
- `optional_none`: 1 bit = 0, flush
- `optional_some_u32`: 1 bit = 1, then u32 LE
- `nested_optional_none`: `optional<optional<u32>>` with None = 0 bit
- `nested_optional_some_none`: `optional<optional<u32>>` with Some(None) = 10 bits
- `array_empty`: LEB128 count = 0
- `array_three_u32`: LEB128 count = 3, then 3 × u32 LE
- `map_one_entry`: LEB128 count = 1, key + value
- `union_known_variant`: discriminant + length + payload
- `union_non_exhaustive_unknown`: discriminant for future variant

- [x] **Step 6: Write evolution.json — cross-version vectors**

```json
[
  {
    "name": "v1_encode_v2_decode_appended_field",
    "schema_v1": "namespace test.evo\nmessage M { x @0 : u32 }",
    "schema_v2": "namespace test.evo\nmessage M { x @0 : u32  y @1 : u16 }",
    "type": "M",
    "value_v1": { "x": 42 },
    "encoded_v1": "2a000000",
    "decoded_as_v2": { "x": 42, "y": 0 },
    "notes": "v2 decoder fills y with default (0) when reading v1-encoded bytes"
  },
  {
    "name": "v2_encode_v1_decode_trailing_ignored",
    "schema_v1": "namespace test.evo\nmessage M { x @0 : u32 }",
    "schema_v2": "namespace test.evo\nmessage M { x @0 : u32  y @1 : u16 }",
    "type": "M",
    "value_v2": { "x": 42, "y": 99 },
    "encoded_v2": "2a0000006300",
    "decoded_as_v1": { "x": 42 },
    "notes": "v1 decoder reads x, ignores trailing bytes (y=99)"
  }
]
```

- [x] **Step 7: Commit**

```bash
git add compliance/
git commit -m "feat: add golden byte compliance vectors for all type categories"
```

---

## Task 6: Rust Compliance Validator ✅ COMPLETE

> **Validated 2026-03-29:** `golden_bytes.rs` exists in `crates/vexil-codegen-rust/tests/`.

**Files:**
- ~~Create:~~ `orix/vexil-lang/crates/vexil-codegen-rust/tests/golden_bytes.rs` — **done**

A test that compiles each vector's schema, generates Rust code, and verifies encode output matches expected bytes. This uses the existing vexil-lang pipeline.

- [x] **Step 1: Write the test harness**

```rust
//! Golden byte vector compliance tests.
//!
//! Reads JSON vectors from compliance/vectors/, compiles each schema,
//! generates Rust encode code, and verifies byte-identical output.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Deserialize)]
struct Vector {
    name: String,
    schema: String,
    #[serde(rename = "type")]
    type_name: String,
    value: serde_json::Value,
    expected_bytes: String,
    #[serde(default)]
    notes: String,
}

fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()  // crates/
        .parent().unwrap()  // vexil-lang/
        .join("compliance/vectors")
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn primitives_vectors_compile() {
    let path = vectors_dir().join("primitives.json");
    let data = fs::read_to_string(&path).expect("read primitives.json");
    let vectors: Vec<Vector> = serde_json::from_str(&data).expect("parse vectors");

    for v in &vectors {
        let result = vexil_lang::compile(&v.schema);
        assert!(
            result.compiled.is_some(),
            "vector '{}' schema failed to compile: {:?}",
            v.name,
            result.diagnostics
        );
    }
}

// Additional tests for each vector file follow the same pattern.
// Encode verification requires generated code execution, which needs
// a compile-and-run harness or trybuild-style approach.
// For Phase 0, we verify compilation + schema hash consistency.
```

- [x] **Step 2: Run tests**

Run: `cd orix/vexil-lang && cargo test -p vexil-codegen-rust golden_bytes`
Expected: PASS — all vector schemas compile.

- [x] **Step 3: Write encode verification test**

For encode verification, the approach is: compile schema → codegen to temp file → build with vexil-runtime → run → compare bytes. This is heavy. A lighter approach for Phase 0: hand-write the encode calls using the generated type names and BitWriter directly, referencing the golden files.

Create a second test that uses `insta` snapshots of actual encoded bytes:

```rust
use vexil_runtime::BitWriter;

#[test]
fn verify_bool_false_bytes() {
    let mut w = BitWriter::new();
    w.write_bool(false);
    w.flush_to_byte_boundary();
    let bytes = w.finish();
    assert_eq!(hex::encode(&bytes), "00");
}

#[test]
fn verify_bool_true_bytes() {
    let mut w = BitWriter::new();
    w.write_bool(true);
    w.flush_to_byte_boundary();
    let bytes = w.finish();
    assert_eq!(hex::encode(&bytes), "01");
}

#[test]
fn verify_u16_le_bytes() {
    let mut w = BitWriter::new();
    w.write_u16(258);
    w.flush_to_byte_boundary();
    let bytes = w.finish();
    assert_eq!(hex::encode(&bytes), "0201");
}

#[test]
fn verify_sub_byte_packing() {
    let mut w = BitWriter::new();
    w.write_bits(5, 3);   // u3 = 5
    w.write_bits(18, 5);  // u5 = 18
    w.flush_to_byte_boundary();
    let bytes = w.finish();
    assert_eq!(hex::encode(&bytes), "95");
}

#[test]
fn verify_nan_canonical() {
    let mut w = BitWriter::new();
    w.write_f32(f32::NAN);
    w.flush_to_byte_boundary();
    let bytes = w.finish();
    assert_eq!(hex::encode(&bytes), "0000c07f");
}

#[test]
fn verify_negative_zero() {
    let mut w = BitWriter::new();
    w.write_f64(-0.0_f64);
    w.flush_to_byte_boundary();
    let bytes = w.finish();
    assert_eq!(hex::encode(&bytes), "0000000000000080");
}

#[test]
fn verify_string_hello() {
    let mut w = BitWriter::new();
    w.write_string("hello");
    w.flush_to_byte_boundary();
    let bytes = w.finish();
    assert_eq!(hex::encode(&bytes), "0568656c6c6f");
}
```

- [x] **Step 4: Run tests**

Run: `cd orix/vexil-lang && cargo test -p vexil-codegen-rust golden_bytes`
Expected: PASS — byte output matches golden vectors exactly.

- [x] **Step 5: Commit**

```bash
git add crates/vexil-codegen-rust/tests/golden_bytes.rs
git commit -m "test: golden byte compliance validator for primitives and sub-byte"
```

---

## Task 7: Schema Evolution Roundtrip Tests ✅ COMPLETE

> **Validated 2026-03-29:** `evolution_roundtrip.rs` exists in `crates/vexil-codegen-rust/tests/`.

**Files:**
- ~~Create:~~ `orix/vexil-lang/crates/vexil-codegen-rust/tests/evolution_roundtrip.rs` — **done**

Test that v1-encoded messages can be decoded by v2 code and vice versa, matching the evolution vectors.

- [x] **Step 1: Write the evolution roundtrip test**

```rust
//! Schema evolution roundtrip tests.
//!
//! Verifies forward and backward compatibility:
//! - v1 encode → v2 decode (new fields get defaults)
//! - v2 encode → v1 decode (trailing bytes ignored)
//! - v1 encode → v2 encode of same logical value → prefix is bit-identical

use vexil_runtime::{BitWriter, BitReader};

/// Simulates a v1 message with one u32 field.
fn encode_v1(x: u32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_u32(x);
    w.flush_to_byte_boundary();
    w.finish()
}

/// Simulates a v2 message with u32 + u16 fields.
fn encode_v2(x: u32, y: u16) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_u32(x);
    w.write_u16(y);
    w.flush_to_byte_boundary();
    w.finish()
}

/// V1 decoder: reads one u32.
fn decode_v1(bytes: &[u8]) -> u32 {
    let mut r = BitReader::new(bytes);
    let x = r.read_u32().unwrap();
    r.flush_to_byte_boundary();
    // Trailing bytes are ignored (§11.4)
    x
}

/// V2 decoder: reads u32 + u16. If bytes are too short (v1 encoded),
/// the second field gets default value 0.
fn decode_v2(bytes: &[u8]) -> (u32, u16) {
    let mut r = BitReader::new(bytes);
    let x = r.read_u32().unwrap();
    let y = if r.remaining_bytes() >= 2 {
        r.read_u16().unwrap()
    } else {
        0 // default for missing field
    };
    r.flush_to_byte_boundary();
    (x, y)
}

#[test]
fn v1_encode_v2_decode_field_gets_default() {
    let bytes = encode_v1(42);
    assert_eq!(hex::encode(&bytes), "2a000000");
    let (x, y) = decode_v2(&bytes);
    assert_eq!(x, 42);
    assert_eq!(y, 0); // default
}

#[test]
fn v2_encode_v1_decode_trailing_ignored() {
    let bytes = encode_v2(42, 99);
    assert_eq!(hex::encode(&bytes), "2a0000006300");
    let x = decode_v1(&bytes);
    assert_eq!(x, 42);
}

#[test]
fn v1_v2_prefix_bit_identical() {
    let v1_bytes = encode_v1(42);
    let v2_bytes = encode_v2(42, 99);
    // First 4 bytes (v1 fields) must be identical
    assert_eq!(&v1_bytes[..4], &v2_bytes[..4]);
}

#[test]
fn v2_encode_v2_decode_roundtrip() {
    let bytes = encode_v2(42, 99);
    let (x, y) = decode_v2(&bytes);
    assert_eq!(x, 42);
    assert_eq!(y, 99);
}

// --- Deprecation: no wire change ---

/// Deprecated field still encodes normally — @deprecated is source-level only.
#[test]
fn deprecated_field_still_encodes() {
    // Message with 3 fields: string, string (@deprecated), u32
    // Deprecation doesn't change the wire format
    let mut w = BitWriter::new();
    w.write_string("current");   // @0 name
    w.write_string("old");       // @1 old_name (@deprecated)
    w.write_u32(30);             // @2 timeout
    w.flush_to_byte_boundary();
    let bytes = w.finish();

    let mut r = BitReader::new(&bytes);
    assert_eq!(r.read_string().unwrap(), "current");
    assert_eq!(r.read_string().unwrap(), "old");
    assert_eq!(r.read_u32().unwrap(), 30);
}

// --- Required → Optional: BREAKING wire change ---

/// Demonstrates that required→optional changes the wire layout.
/// V1: u32 (4 bytes). V2: optional<u32> (1 bit + 4 bytes if present).
/// These are NOT compatible — a v1 decoder misreads v2 bytes.
#[test]
fn required_to_optional_is_breaking() {
    // V1 encode: required u32
    let mut w1 = BitWriter::new();
    w1.write_u32(42);
    w1.write_string("test");
    w1.flush_to_byte_boundary();
    let v1_bytes = w1.finish();

    // V2 encode: optional<u32> with Some(42)
    let mut w2 = BitWriter::new();
    w2.write_bool(true);         // presence bit
    w2.flush_to_byte_boundary();
    w2.write_u32(42);
    w2.write_string("test");
    w2.flush_to_byte_boundary();
    let v2_bytes = w2.finish();

    // Wire layouts are DIFFERENT — this is a breaking change
    assert_ne!(v1_bytes, v2_bytes);
    // V1 starts with raw u32 bytes, V2 starts with presence bit + alignment
    assert_ne!(v1_bytes[0], v2_bytes[0]);
}
```

- [x] **Step 2: Run tests**

Run: `cd orix/vexil-lang && cargo test -p vexil-codegen-rust evolution_roundtrip`
Expected: PASS.

- [x] **Step 3: Commit**

```bash
git add crates/vexil-codegen-rust/tests/evolution_roundtrip.rs
git commit -m "test: schema evolution roundtrip tests (v1↔v2 compat)"
```

---

## Task 8: Benchmark Suite ✅ COMPLETE

> **Validated 2026-03-29:** `vexil-bench` crate exists as a workspace member with `benches/encode_decode.rs` and `src/messages.rs`.

**Files:**
- ~~Create:~~ `orix/vexil-lang/crates/vexil-bench/Cargo.toml` — **done**
- ~~Create:~~ `orix/vexil-lang/crates/vexil-bench/src/lib.rs` — **done**
- ~~Create:~~ `orix/vexil-lang/crates/vexil-bench/src/messages.rs` — **done**
- ~~Create:~~ `orix/vexil-lang/crates/vexil-bench/benches/encode_decode.rs` — **done**
- ~~Modify:~~ `orix/vexil-lang/Cargo.toml` (already a workspace member) — **done**

Benchmark Vexil bitpack encode/decode throughput on representative VNP-like messages.

- [x] **Step 1: Create the bench crate Cargo.toml**

```toml
[package]
name = "vexil-bench"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
vexil-runtime = { path = "../vexil-runtime" }

[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "encode_decode"
harness = false
```

- [x] **Step 2: Add to workspace Cargo.toml**

Add `"crates/vexil-bench"` to the workspace members list.

- [x] **Step 3: Write representative VNP-like message types**

`src/messages.rs`:

```rust
//! Hand-written message types that mirror what VNP would generate.
//! Used for benchmarking encode/decode throughput.

use vexil_runtime::{BitWriter, BitReader, EncodeError, DecodeError};

/// Simulates a VNP OutputChunk message (high frequency, medium size).
pub struct OutputChunk {
    pub session_id: u32,
    pub pane_id: u16,
    pub sequence: u64,
    pub data: Vec<u8>,
    pub command_tag: Option<String>,
}

impl OutputChunk {
    pub fn encode(&self, w: &mut BitWriter) -> Result<(), EncodeError> {
        w.write_u32(self.session_id);
        w.write_u16(self.pane_id);
        w.write_u64(self.sequence);
        w.write_bytes(&self.data);
        match &self.command_tag {
            Some(s) => {
                w.write_bool(true);
                w.write_string(s);
            }
            None => {
                w.write_bool(false);
            }
        }
        w.flush_to_byte_boundary();
        Ok(())
    }

    pub fn decode(r: &mut BitReader<'_>) -> Result<Self, DecodeError> {
        let session_id = r.read_u32()?;
        let pane_id = r.read_u16()?;
        let sequence = r.read_u64()?;
        let data = r.read_bytes()?;
        let command_tag = if r.read_bool()? {
            Some(r.read_string()?)
        } else {
            None
        };
        r.flush_to_byte_boundary();
        Ok(Self { session_id, pane_id, sequence, data, command_tag })
    }
}

/// Simulates a VNP Envelope header (every message has this).
pub struct Envelope {
    pub version: u8,    // 4 bits
    pub domain: u8,     // 4 bits
    pub msg_type: u8,   // 7 bits
    pub session_id: u32,
    pub timestamp: u64, // 48 bits
    pub msg_id: Option<u32>,
}

impl Envelope {
    pub fn encode(&self, w: &mut BitWriter) -> Result<(), EncodeError> {
        w.write_bits(self.version as u64, 4);
        w.write_bits(self.domain as u64, 4);
        w.write_bits(self.msg_type as u64, 7);
        w.flush_to_byte_boundary();
        w.write_u32(self.session_id);
        w.write_bits(self.timestamp, 48);
        match self.msg_id {
            Some(id) => {
                w.write_bool(true);
                w.flush_to_byte_boundary();
                w.write_u32(id);
            }
            None => {
                w.write_bool(false);
            }
        }
        w.flush_to_byte_boundary();
        Ok(())
    }

    pub fn decode(r: &mut BitReader<'_>) -> Result<Self, DecodeError> {
        let version = r.read_bits(4)? as u8;
        let domain = r.read_bits(4)? as u8;
        let msg_type = r.read_bits(7)? as u8;
        r.flush_to_byte_boundary();
        let session_id = r.read_u32()?;
        let timestamp = r.read_bits(48)?;
        let msg_id = if r.read_bool()? {
            r.flush_to_byte_boundary();
            Some(r.read_u32()?)
        } else {
            None
        };
        r.flush_to_byte_boundary();
        Ok(Self { version, domain, msg_type, session_id, timestamp, msg_id })
    }
}

/// Simulates a VNP RenderCommand (high frequency, variable size).
pub struct DrawText {
    pub x: u16,
    pub y: u16,
    pub fg: [u8; 3],    // RGB
    pub bg: [u8; 3],    // RGB
    pub bold: bool,
    pub italic: bool,
    pub text: String,
}

impl DrawText {
    pub fn encode(&self, w: &mut BitWriter) -> Result<(), EncodeError> {
        w.write_u16(self.x);
        w.write_u16(self.y);
        w.write_raw_bytes(&self.fg);
        w.write_raw_bytes(&self.bg);
        w.write_bool(self.bold);
        w.write_bool(self.italic);
        w.flush_to_byte_boundary();
        w.write_string(&self.text);
        w.flush_to_byte_boundary();
        Ok(())
    }

    pub fn decode(r: &mut BitReader<'_>) -> Result<Self, DecodeError> {
        let x = r.read_u16()?;
        let y = r.read_u16()?;
        let mut fg = [0u8; 3];
        let mut bg = [0u8; 3];
        r.read_exact(&mut fg)?;
        r.read_exact(&mut bg)?;
        let bold = r.read_bool()?;
        let italic = r.read_bool()?;
        r.flush_to_byte_boundary();
        let text = r.read_string()?;
        r.flush_to_byte_boundary();
        Ok(Self { x, y, fg, bg, bold, italic, text })
    }
}
```

`src/lib.rs`:

```rust
pub mod messages;
```

- [x] **Step 4: Write the criterion benchmark**

`benches/encode_decode.rs`:

```rust
use criterion::{criterion_group, criterion_main, Criterion, black_box};
use vexil_runtime::{BitWriter, BitReader};
use vexil_bench::messages::{OutputChunk, Envelope, DrawText};

fn bench_output_chunk_encode(c: &mut Criterion) {
    let msg = OutputChunk {
        session_id: 1,
        pane_id: 0,
        sequence: 12345,
        data: vec![b'x'; 4096], // typical cargo output line
        command_tag: Some("cargo build".to_string()),
    };

    c.bench_function("OutputChunk encode 4KiB", |b| {
        b.iter(|| {
            let mut w = BitWriter::new();
            msg.encode(&mut w).unwrap();
            black_box(w.finish());
        })
    });
}

fn bench_output_chunk_decode(c: &mut Criterion) {
    let msg = OutputChunk {
        session_id: 1,
        pane_id: 0,
        sequence: 12345,
        data: vec![b'x'; 4096],
        command_tag: Some("cargo build".to_string()),
    };
    let mut w = BitWriter::new();
    msg.encode(&mut w).unwrap();
    let bytes = w.finish();

    c.bench_function("OutputChunk decode 4KiB", |b| {
        b.iter(|| {
            let mut r = BitReader::new(&bytes);
            black_box(OutputChunk::decode(&mut r).unwrap());
        })
    });
}

fn bench_envelope_encode(c: &mut Criterion) {
    let env = Envelope {
        version: 1,
        domain: 3,
        msg_type: 42,
        session_id: 1,
        timestamp: 1234567890123,
        msg_id: Some(99),
    };

    c.bench_function("Envelope encode", |b| {
        b.iter(|| {
            let mut w = BitWriter::new();
            env.encode(&mut w).unwrap();
            black_box(w.finish());
        })
    });
}

fn bench_envelope_decode(c: &mut Criterion) {
    let env = Envelope {
        version: 1,
        domain: 3,
        msg_type: 42,
        session_id: 1,
        timestamp: 1234567890123,
        msg_id: Some(99),
    };
    let mut w = BitWriter::new();
    env.encode(&mut w).unwrap();
    let bytes = w.finish();

    c.bench_function("Envelope decode", |b| {
        b.iter(|| {
            let mut r = BitReader::new(&bytes);
            black_box(Envelope::decode(&mut r).unwrap());
        })
    });
}

fn bench_draw_text_encode(c: &mut Criterion) {
    let cmd = DrawText {
        x: 10, y: 5,
        fg: [255, 255, 255],
        bg: [0, 0, 0],
        bold: true, italic: false,
        text: "fn main() { println!(\"hello\"); }".to_string(),
    };

    c.bench_function("DrawText encode", |b| {
        b.iter(|| {
            let mut w = BitWriter::new();
            cmd.encode(&mut w).unwrap();
            black_box(w.finish());
        })
    });
}

fn bench_draw_text_decode(c: &mut Criterion) {
    let cmd = DrawText {
        x: 10, y: 5,
        fg: [255, 255, 255],
        bg: [0, 0, 0],
        bold: true, italic: false,
        text: "fn main() { println!(\"hello\"); }".to_string(),
    };
    let mut w = BitWriter::new();
    cmd.encode(&mut w).unwrap();
    let bytes = w.finish();

    c.bench_function("DrawText decode", |b| {
        b.iter(|| {
            let mut r = BitReader::new(&bytes);
            black_box(DrawText::decode(&mut r).unwrap());
        })
    });
}

fn bench_batch_encode(c: &mut Criterion) {
    // Simulate encoding a frame batch: 1 envelope + 50 DrawText commands
    let env = Envelope {
        version: 1, domain: 6, msg_type: 1,
        session_id: 1, timestamp: 0, msg_id: None,
    };
    let cmds: Vec<DrawText> = (0..50).map(|i| DrawText {
        x: 0, y: i,
        fg: [200, 200, 200], bg: [30, 30, 30],
        bold: false, italic: false,
        text: format!("line {} of output from cargo build", i),
    }).collect();

    c.bench_function("Frame batch encode (1 env + 50 DrawText)", |b| {
        b.iter(|| {
            let mut w = BitWriter::new();
            env.encode(&mut w).unwrap();
            for cmd in &cmds {
                cmd.encode(&mut w).unwrap();
            }
            black_box(w.finish());
        })
    });
}

criterion_group!(
    benches,
    bench_output_chunk_encode,
    bench_output_chunk_decode,
    bench_envelope_encode,
    bench_envelope_decode,
    bench_draw_text_encode,
    bench_draw_text_decode,
    bench_batch_encode,
);
criterion_main!(benches);
```

- [x] **Step 5: Run benchmarks**

Run: `cd orix/vexil-lang && cargo bench -p vexil-bench`
Expected: Benchmark results printed. Record baseline numbers.

- [x] **Step 6: Commit**

```bash
git add crates/vexil-bench/ Cargo.toml
git commit -m "feat: add encode/decode benchmark suite with VNP-representative messages"
```

---

## Task 9: TypeScript Compliance Tests ← PARTIALLY COMPLETE

> **Validated 2026-03-29:** `packages/runtime-ts/` (v0.4.1, published as `@vexil-lang/runtime`) already exists with a comprehensive runtime (BitReader, BitWriter, SchemaHandshake) and 134 passing tests across 6 test files. The runtime has all needed primitives including NaN canonicalization, negative zero preservation, recursion depth tracking (`enterNested()`/`leaveNested()`), sub-byte read/write, LEB128, ZigZag, and all integer/float/string/bytes operations.
>
> **Note:** The TS runtime uses `enterNested()`/`leaveNested()` while the Rust runtime uses `enter_recursive()`/`leave_recursive()`. This is a naming asymmetry but functionally equivalent.
>
> `tests/compliance.test.ts` already exists with a schema-driven approach: it parses field definitions from the vector JSON, encodes/decodes via BitWriter/BitReader, and compares against `expected_bytes`. Currently covers **primitives.json**, **sub_byte.json**, and **messages.json** (33 tests).
>
> **What remains:** Extend compliance coverage for compound types: **optionals.json**, **enums.json**, **unions.json**, **arrays_maps.json**, and **evolution.json**. These require extending the minimal `parseFields` regex and `writeField`/`readField` functions in `compliance.test.ts` to handle `optional<T>`, enum variants (bit-packed), union discriminant+length+payload, `array<T>`, and `map<K,V>` syntax. The evolution vectors also use a different JSON structure (`schema_v1`/`schema_v2` instead of `schema`).

**Files:**
- Modify: `orix/vexil-lang/packages/runtime-ts/tests/compliance.test.ts` — extend for compound types
- Possibly create: `orix/vexil-lang/packages/runtime-ts/tests/evolution-compliance.test.ts` — if evolution tests need a separate harness due to different JSON schema

The second independent implementation already exists and passes. Remaining work extends compliance coverage to all vector categories.

- [x] **Step 1: Audit existing runtime-ts for compliance readiness**

> **Done 2026-03-29:** Runtime has all needed primitives. 134 tests pass. `compliance.test.ts` already covers primitives.json, sub_byte.json, messages.json (33 tests). Runtime API: `readBits`/`writeBits`, `readBool`/`writeBool`, all integer/float types, `readString`/`writeString`, `readBytes`/`writeBytes`, `flushToByteBoundary`, `enterNested`/`leaveNested`, NaN canonicalization, negative zero preservation, LEB128, ZigZag. No missing methods.

- [x] **Step 2: Existing compliance.test.ts covers primitives, sub_byte, messages**

> **Already done.** Schema-driven test harness with `parseFields`, `writeField`, `readField`, `encodeValue`, `decodeValue`, `valuesMatch` helper functions. Uses vitest. Covers primitives.json (all types including NaN, -0.0, string), sub_byte.json (u3/u5 packing, cross-byte u3/u5/u6), messages.json (empty message, two u32, mixed types).

- [x] **Step 3: Existing bit-reader/bit-writer tests cover remaining primitives**

> **Already done.** `bit-reader.test.ts` (50 tests) and `bit-writer.test.ts` (40 tests) provide comprehensive unit coverage.

- [ ] **Step 4: Extend compliance.test.ts for optionals.json**

The `parseFields` regex (`/(\w+)\s+@\d+\s*:\s*(\w+)/g`) only handles simple types. Extend it to match `optional<T>` syntax. Add `writeField`/`readField` cases for optional — write presence bit (0 or 1), if present then flush + write inner type. Vector values use `null` for None.

`optionals.json` has 2 vectors: `optional_none` (value `null` → `"00"`) and `optional_some_u32` (value `42` → `"012a000000"`).

- [ ] **Step 5: Extend compliance.test.ts for enums.json**

Enums use bit-packed discriminants. `enums.json` has 2 vectors with 2-variant enum (1 bit). Extend the parser to recognize `enum` definitions and map variant names to discriminant values. Write discriminant bits, read and compare.

- [ ] **Step 6: Extend compliance.test.ts for unions.json**

Unions encode as: LEB128 discriminant + LEB128 payload length + payload bytes. `unions.json` has 1 vector (Circle variant). Extend the parser to recognize `union` definitions with variant fields. More complex than enums due to the length-prefixed payload structure.

- [ ] **Step 7: Extend compliance.test.ts for arrays_maps.json**

Arrays encode as: LEB128 count + repeated element encoding. Maps encode as: LEB128 count + repeated (key + value) pairs. `arrays_maps.json` has 3 vectors: empty array, 3-element u32 array, single-entry string→u32 map. Extend `parseFields` to match `array<T>` and `map<K,V>` syntax.

- [ ] **Step 8: Add evolution-compliance.test.ts**

The evolution vectors use a different JSON structure: `schema_v1`/`schema_v2` instead of `schema`, `value_v1`/`value_v2` instead of `value`, `encoded_v1`/`encoded_v2` instead of `expected_bytes`, and `decoded_as_v1`/`decoded_as_v2` for cross-version decode results. Needs a separate test file (or a separate `describe` block) with its own harness.

`evolution.json` has 2 vectors: v1 encode→v2 decode (appended field gets default) and v2 encode→v1 decode (trailing bytes ignored).

- [ ] **Step 9: Run full compliance suite**

Run: `cd orix/vexil-lang/packages/runtime-ts && npx vitest run`
Expected: All pass — TS encode/decode matches Rust golden vectors for all type categories.

- [ ] **Step 10: Commit**

```bash
cd orix/vexil-lang
git add packages/runtime-ts/tests/
git commit -m "test(runtime-ts): extend compliance vectors to optionals, enums, unions, arrays, maps, evolution"
```

---

## Task 10: Performance Comparison ← REMAINING

> **Validated 2026-03-29:** `vexil-bench` crate exists with 4 benchmark groups (Envelope encode/decode, DrawText encode/decode, OutputChunk 4KiB encode/decode, Batch 1+50 encode/decode) using criterion 0.8. No protobuf/Cap'n Proto/FlatBuffers comparison yet — only Vexil benchmarks.
>
> **Note:** Adding comparison benchmarks (`prost`, `capnp`, `flatbuffers`) to `vexil-bench` is a general improvement (benchmarking against established formats for the Vexil project), not MALT-specific.

**Files:**
- Modify: `orix/vexil-lang/crates/vexil-bench/benches/encode_decode.rs` (add comparison data)
- Modify: `orix/vexil-lang/crates/vexil-bench/Cargo.toml` (add prost, possibly capnp/flatbuffers dev-deps)
- Create: `orix/malt/specs/phase0-justification.md`

Compare bitpack throughput against established formats on the same message shapes.

- [ ] **Step 1: Run Vexil benchmarks and record numbers**

Run: `cd orix/vexil-lang && cargo bench -p vexil-bench 2>&1 | tee bench-results.txt`

- [ ] **Step 2: Create comparison benchmarks for protobuf, Cap'n Proto, and FlatBuffers**

Add `prost` + `prost-build` to vexil-bench dev-deps. Define equivalent .proto messages for OutputChunk, Envelope, DrawText. Benchmark encode/decode.

For Cap'n Proto and FlatBuffers: add `capnp` and `flatbuffers` crates. Define equivalent schemas (.capnp and .fbs files) for the same three message types. Benchmark encode/decode.

If adding all three comparison frameworks is too heavy for Phase 0, prioritize protobuf (most widely comparable), and for Cap'n Proto / FlatBuffers, use published benchmark data for equivalent message sizes with cited sources. The justification document must address all three formats explicitly — the spec requires comparison against all three.

- [ ] **Step 3: Write the go/no-go justification document**

`orix/malt/specs/phase0-justification.md`:

```markdown
# Phase 0 Go/No-Go: Vexil Bitpack as VNP Wire Format

## Decision: [GO / NO-GO]

## The Question

Can Vexil bitpack serve as VNP's wire format? Specifically:

1. Is the encoding deterministic? (Same message → bit-identical bytes always)
2. Does schema evolution work? (v1↔v2 interop without coordination)
3. Is performance competitive with established formats?
4. What property do existing formats fail that justifies a custom format?

## Evidence

### Determinism

[Results from golden byte tests. Every vector produces identical bytes
across Rust and TypeScript implementations.]

### Schema Evolution

[Results from evolution roundtrip tests. Forward and backward compatibility
verified for field append and variant addition.]

### Performance

| Benchmark | Vexil bitpack | protobuf (prost) | Cap'n Proto | FlatBuffers | Notes |
|-----------|--------------|------------------|-------------|-------------|-------|
| OutputChunk encode 4KiB | X ns | Y ns | Z ns | W ns | |
| OutputChunk decode 4KiB | X ns | Y ns | Z ns | W ns | |
| Envelope encode | X ns | Y ns | Z ns | W ns | |
| Envelope decode | X ns | Y ns | Z ns | W ns | |
| DrawText encode | X ns | Y ns | Z ns | W ns | |
| Frame batch (50 cmds) | X ns | Y ns | Z ns | W ns | |
| Wire size: OutputChunk 4KiB | X bytes | Y bytes | Z bytes | W bytes | Smaller = better |
| Wire size: Envelope | X bytes | Y bytes | Z bytes | W bytes | |

### Why Not Existing Formats

The property existing formats fail: **deterministic encoding for BLAKE3
content addressing.**

- Protocol Buffers: field ordering is non-deterministic (map iteration
  order, oneof encoding order). `deterministic=true` serialization is
  best-effort and explicitly not guaranteed across implementations.
- Cap'n Proto: zero-copy, but pointer-based format with padding that
  varies by allocator behavior. Not bit-identical across implementations.
- FlatBuffers: similar pointer/vtable layout issues.
- MessagePack: map key ordering is implementation-defined.

Vexil bitpack is deterministic by construction: fields encode in ordinal
order, sub-byte packing is LSB-first with no degrees of freedom, NaN is
canonicalized. Same schema + same values = identical bytes on every platform.

This enables:
- BLAKE3 schema hash for wire version identity
- Content-addressed message caching
- Replay detection without sequence numbers
- Cross-platform verification

### Risks

[Document any issues discovered during Phase 0 testing.]

## Conclusion

[GO: proceed with Phase 1 using Vexil bitpack]
[NO-GO: adopt [format] and layer MALT framing on top]
```

- [ ] **Step 4: Fill in actual benchmark numbers and test results**

After running all benchmarks and tests, fill in the template with real data.

- [ ] **Step 5: Commit**

```bash
git add orix/malt/specs/phase0-justification.md bench-results.txt
git commit -m "docs: Phase 0 go/no-go justification with benchmark data"
```

---

## Task 11: Trailing Bytes Tolerance in BitReader ✅ COMPLETE

> **Validated 2026-03-29:** `bit_reader.rs` has a `trailing_bytes_not_rejected()` test confirming §11.4 compliance.

**Files:**
- ~~Modify:~~ `orix/vexil-lang/crates/vexil-runtime/src/bit_reader.rs` — **done**

The spec says decoders MUST NOT reject messages with trailing bytes (§11.4). BitReader tolerates trailing bytes.

- [x] **Step 1: Write failing test**

- [x] **Step 2: Run test**

- [x] **Step 3: If test fails, fix BitReader to not error on trailing data**

> **Actual:** BitReader already tolerates trailing bytes — no fix needed.

- [x] **Step 4: Commit**

---

## Task 12: Final Integration — Full Compliance Suite Run ← REMAINING

> **Partial validation 2026-03-29:**
> - Rust: `cargo test --workspace` → **478 tests passed**, 0 failed, 1 ignored
> - TypeScript: `npx vitest run` → **134 tests passed** (6 test files)
> - Clippy: 2 warnings in `vexil-store` tests (approx_constant for `3.14`, not Phase 0 related)
> - `cargo fmt`: not yet checked
>
> This task should be run after Tasks 9 and 10 are complete to record final numbers.

**Files:**
- No new files — this is a verification task

Run the complete compliance suite end-to-end: Rust golden byte tests + TypeScript compliance tests + evolution roundtrips + all existing corpus tests.

- [ ] **Step 1: Run Rust full test suite**

Run: `cd orix/vexil-lang && cargo test --workspace`
Expected: All tests pass (existing + new).

- [ ] **Step 2: Run TypeScript compliance tests**

Run: `cd orix/vexil-lang/packages/runtime-ts && npx vitest run`
Expected: All tests pass.

- [ ] **Step 3: Run clippy**

Run: `cd orix/vexil-lang && cargo clippy --workspace --all-targets -- -D warnings`
Expected: Clean.

- [ ] **Step 4: Run fmt check**

Run: `cd orix/vexil-lang && cargo fmt --all -- --check`
Expected: Clean.

- [ ] **Step 5: Record final pass/fail count**

Document: X Rust tests passing, Y TypeScript tests passing, Z golden byte vectors verified, benchmarks recorded.

- [ ] **Step 6: Update go/no-go document with final results**

Fill in `phase0-justification.md` with actual data from steps 1-5.

- [ ] **Step 7: Commit final state**

```bash
git add -A
git commit -m "chore: Phase 0 complete — compliance suite passing, benchmarks recorded"
```

---

## Phase 0 Exit Criteria

All of the following must be true to proceed to Phase 1:

| Criterion | Measurement | Status |
|-----------|-------------|--------|
| Deterministic encoding | Every golden byte vector produces identical bytes in Rust AND TypeScript | Rust ✅ (478 tests), TS ✅ for primitives/sub_byte/messages (33 of 134 tests), TS pending for optionals/enums/unions/arrays_maps/evolution |
| Schema evolution | v1↔v2 roundtrip tests pass (field append, variant addition) | ✅ `evolution_roundtrip.rs` in Rust; TS evolution tests pending |
| Trailing byte tolerance | BitReader does not reject messages with extra bytes | ✅ Rust tested; TS `remaining()` allows unread bytes |
| Recursion depth | Depth 64 succeeds, depth 65 returns error (not stack overflow) | ✅ Rust `enter_recursive()`/`leave_recursive()`, TS `enterNested()`/`leaveNested()` |
| NaN canonicalization | All NaN inputs produce canonical qNaN bytes | ✅ Both Rust and TS compliance vectors pass for f32/f64 NaN |
| Performance | Encode/decode throughput within 2x of protobuf on equivalent messages | **Pending** — vexil-bench exists, protobuf comparison not yet added |
| Two implementations | Rust and TypeScript (`packages/runtime-ts/`) pass the same compliance vectors | **Partial** — primitives/sub_byte/messages pass; compound types pending |
| Written justification | `phase0-justification.md` documents the GO decision with evidence | **Not started** |

If any criterion fails, the justification document explains the failure and recommends either fixing the issue or adopting an existing format.
