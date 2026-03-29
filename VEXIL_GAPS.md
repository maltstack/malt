# Vexil Language Gaps

Shortcomings in `vexil-lang` discovered during VNP schema design. Each gap includes the problem, its impact on MALT schemas, the current workaround, and the requested fix for the vexil-lang project.

These are tracked here (not as vexil-lang issues) because they were discovered during MALT design. They should be filed as vexil-lang issues when implementation begins.

---

## Gap 1: Map keys don't accept newtypes

**Problem:** `map<PaneId, PersistedPane>` is invalid because the Vexil spec restricts map key types to primitives, string, bytes, uuid, enum, and flags. Newtypes are excluded even when they wrap a valid key type.

**Impact:** Persistence schemas use `map<u32, PersistedPane>` instead of `map<PaneId, PersistedPane>`, losing type safety on map keys. Generated Rust code uses raw `u32` keys.

**Workaround:** Use the raw primitive type for map keys. Document the intended key type with `@doc`.

**Requested fix:** Newtypes wrapping valid map key types should themselves be valid map key types. The wire encoding is identical to the inner type — this is purely a type-checker restriction.

---

## Gap 2: No custom/user-defined annotation support

**Problem:** Cannot attach arbitrary key-value metadata to declarations. VNP needs to associate priority class, routing hints, and other protocol metadata with message types.

**Impact:** Priority class (Critical, Reliable, High, Normal, Low) must be maintained as a separate Rust const table in `malt-protocol` rather than co-located with the schema definition.

**Workaround:** Document priority via `@doc("Priority: Critical")` on each message and maintain a `priority_of()` mapping function in `malt-protocol` Rust code.

**Requested fix:** Support user-defined annotations — e.g., `@meta(key, value)` or a pluggable annotation registry. Codegen backends should emit custom annotations as associated constants or a queryable metadata map.

---

## Gap 3: Potential `None` variant conflict in Rust codegen

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
