# Vexil Language Gaps

Shortcomings in `vexil-lang` discovered during VNP schema design. Each gap includes the problem, its impact on MALT schemas, the current workaround, and the requested fix for the vexil-lang project.

These are tracked here (not as vexil-lang issues) because they were discovered during MALT design. They should be filed as vexil-lang issues when implementation begins.

---

**Status update (2026-07-24):** re-checked against vexil-lang's current history
(379 commits, v1.0.0 released 2026-04-09). Gaps 1, 2, 3, and the unlabeled
`@`-after-import gap below were all fixed upstream by 2026-03-31 — before this
document's previous edit (2026-04-08), meaning the tracking here was already
stale on the day it was last touched. Only **Gap 4 (`vexil.std` types)** is
still genuinely open; vexil-lang's own docs defer it to their package-manager
milestone. See inline FIXED annotations below.

## ~~Gap 1: Map keys don't accept newtypes~~ FIXED (vexil-lang `c981a06`, 2026-03-30)

**Problem:** `map<PaneId, PersistedPane>` is invalid because the Vexil spec restricts map key types to primitives, string, bytes, uuid, enum, and flags. Newtypes are excluded even when they wrap a valid key type.

**Impact:** Persistence schemas use `map<u32, PersistedPane>` instead of `map<PaneId, PersistedPane>`, losing type safety on map keys. Generated Rust code uses raw `u32` keys.

**Workaround:** Use the raw primitive type for map keys. Document the intended key type with `@doc`.

**Requested fix:** Newtypes wrapping valid map key types should themselves be valid map key types. The wire encoding is identical to the inner type — this is purely a type-checker restriction.

---

## ~~Gap 2: No custom/user-defined annotation support~~ FIXED (vexil-lang `684fffc`/`0f00e1f`, 2026-03-30)

**Problem:** Cannot attach arbitrary key-value metadata to declarations. VNP needs to associate priority class, routing hints, and other protocol metadata with message types.

**Impact:** Priority class (Critical, Reliable, High, Normal, Low) must be maintained as a separate Rust const table in `malt-protocol` rather than co-located with the schema definition.

**Workaround:** Document priority via `@doc("Priority: Critical")` on each message and maintain a `priority_of()` mapping function in `malt-protocol` Rust code.

**Requested fix:** Support user-defined annotations — e.g., `@meta(key, value)` or a pluggable annotation registry. Codegen backends should emit custom annotations as associated constants or a queryable metadata map.

---

## ~~Gap 3: Potential `None` variant conflict in Rust codegen~~ FIXED (vexil-lang `63c76bc`, 2026-03-31)

**Problem:** Enum variants named `None` (e.g., `ColorDepth::None`) may conflict with `Option::None` in generated Rust code, depending on codegen output and usage context.

**Impact:** Several VNP enums use `None` as a variant name (`ColorDepth`, `UnicodeLevel`, `ImageProtocol`).

**Workaround:** Verify codegen output during Phase 1 sub-project 2 (malt-protocol crate). If conflicts arise, rename variants to `Off` or `Unsupported`.

**Requested fix:** Rust codegen should handle `None` variant names safely — either by always qualifying enum variant usage or by emitting a warning when a variant name collides with Rust built-in names.

---

## Gap 4: No standard library types

**Problem:** Common semantic types like `Duration`, `IpAddr`, `SemVer`, `Url` are listed as future `vexil.std` additions but not available today.

**Impact:** VNP schemas use raw primitives with `@doc` annotations explaining semantics — `u64` for microsecond durations, `string` for version strings, `i64` for timestamps. Type safety and self-documentation suffer.

**Workaround:** Use raw types with `@doc` annotations.

**Requested fix:** Implement `vexil.std` with at least: `Duration` (i64 microseconds), `Timestamp` (i64 microseconds since epoch), `SemVer` (string with validation).

---

## ~~Gap 5: Parser treats `@` after bare import as version constraint~~ FIXED (vexil-lang #48)

---

## ~~Gap 6: Sub-byte types >8 bits generate wrong Rust type~~ FIXED (vexil-lang #52)

**Problem:** `vexil-codegen-rust` generates `u8` for sub-byte types larger than 8 bits (e.g., `u48`). The `read_bits(48)` call returns `u64` but the generated code casts to `u8`, truncating the value.

**Impact:** The `Envelope.timestamp` field (48-bit microsecond timestamp) is typed `u8` in generated code. Values above 255 are silently truncated. This makes the timestamp field unusable for its intended purpose.

**Workaround:** Envelope tests use values within `u8` range. The full 48-bit timestamp will work once this is fixed.

**Requested fix:** Sub-byte types with width >8 should map to the smallest Rust integer that fits: `u8` for 1-8 bits, `u16` for 9-16, `u32` for 17-32, `u64` for 33-64. The `as u8` cast in generated code should be `as u64` (or the appropriate target type).

## ~~Gap 7: `@` after bare import parsed as version constraint~~ FIXED (vexil-lang #43, `d170051`/`458bbd4`, 2026-03-29)

**Problem:** `vexilc check` fails on any file that has `import malt.common` (without a `@ ^X.Y.Z` version constraint) followed by any `@annotation`. The parser reads the `@` from `@doc(...)` or `@domain(...)` as the start of a version constraint on the import statement, then fails with "expected `^` after `@` in version constraint."

**Impact:** Every domain schema that imports from `common.vexil` fails to compile with `vexilc check`. Only self-contained schemas (no imports) compile.

**Workaround:** None that preserves correct schema semantics. Removing annotations would strip documentation and wire dispatch metadata. The `vexilc build` command (with `--include`) may handle this differently but was not tested.

**Requested fix:** The parser should terminate import statement parsing at the newline (or next non-whitespace token that isn't `@` followed by `^`). An `@` followed by an identifier (like `@doc`) should not be parsed as part of the import's version constraint.

**Reproduction:**
```vexil
@version("0.1.0")
namespace malt.test
import malt.common
@doc("This triggers the bug.")
message Foo { x @0 : u32 }
```
Run: `vexilc check test.vexil` → Error: "expected `^` after `@` in version constraint"
