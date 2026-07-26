# Implementation Plan: Windows contained-image provisioning

**Branch**: `009-windows-image-provisioning` | **Date**: 2026-07-27 | **Spec**: [spec.md](spec.md)

## Summary

Add a MALT-owned Windows OCI image lifecycle: resolve and verify a public
Windows image by immutable digest, retain it in the elevated helper's owned
store, materialize its ordered layers through the HCS APIs, and allocate a
private writable layer for each contained session. The daemon and CLI deal in
opaque image identifiers only; the helper resolves those identifiers inside
its owned store and remains the sole creator of HCS layer, compute-system, and
scratch state. This supplies the missing substrate for the existing Spec 008
HCS process handoff and ends with a live contained external-command and
cleanup proof.

## Technical Context

**Language/Version**: Rust 2021; MSRV 1.85.

**Primary Dependencies**: Existing Vexil runtime for typed VNP; existing
`windows-sys` HCS bindings in `malt-platform`; a new owned L1 `malt-image`
crate for OCI registry, manifest, digest, and archive handling. Any new
third-party crates required for SHA-256, HTTP, or gzip/tar decoding require
explicit dependency approval before implementation.

**Storage**: Helper-owned `%ProgramData%\\MALT\\images` content-addressed
store, containing verified OCI blobs, immutable image records, prepared HCS
layers, and session-scoped scratch directories. The daemon persists only
opaque image IDs and session-to-image references; it never persists a
privileged path as authority.

**Testing**: `cargo test --workspace`; focused unit and protocol tests; fake
platform ordering/rollback tests; Windows-only real HCS layer, compute-system,
process, and cleanup tests; final live quickstart evidence.

**Target Platform**: Windows host with Containers/HCS available. The command
surface remains cross-platform but reports Windows containment unavailable
elsewhere.

**Project Type**: Rust workspace daemon, privileged Windows service, and CLI.

**Performance Goals**: Stream image downloads and layer extraction without
holding a whole blob in memory; address blobs by digest; allocate one scratch
layer per contained session; no performance claim beyond the image registry's
actual transfer rate in the first release.

**Constraints**: Public registry images only; Windows/amd64 variant only;
pinned digest is required before a record becomes selectable; no Docker
runtime dependency; no caller-selected paths or HCS storage JSON crosses the
privilege boundary; a required session fails closed and a preferred session
uses the existing explicit downgrade path only.

**Scale/Scope**: One initial tested Microsoft Windows base-image family and
the complete lifecycle needed to turn it into an HCS process parent layer;
private registry authentication, arbitrary archive import, and Linux/macOS
image backends are explicitly outside this feature.

## Constitution Check

| Gate | Design response | Status |
|------|-----------------|--------|
| I. VT codes confined | No terminal parsing or `vte` usage is introduced. | Pass |
| II. OS calls confined | HCS import, writable-layer, attach/detach, and cleanup calls live only in `malt-platform::isolation::layers`; `malt-image` does no Windows API work. | Pass |
| III. Dependency-free foundations | `malt-protocol` remains independent; schema additions are VNP types only; `malt-plugin-sdk` is untouched. | Pass |
| IV. Safety explicit | Every new Windows FFI boundary has a `SAFETY:` justification and real-API tests; production code has no `unwrap`/`expect`. | Pass |
| V. VNP-only boundary | CLI/daemon/helper image operations are typed additions to `elevate.vexil`; no JSON or filesystem-path side channel is introduced. | Pass |
| VI. POSIX conformance | Mash semantics are unchanged; external process routing is exercised through the existing conformance-preserving spawner interface. | Pass |
| VII. Layer direction | New `malt-image` is L1 and can depend only on L0 foundations; `malt-platform` never depends upward. | Pass |
| VIII. Vendor only | Vexil-v2 is surveyed as prior art only; no code, cache, path, or dependency is reused. | Pass |
| IX. No silent scope jump | Public Windows OCI provisioning is bounded here. Private registries, archive import, and other OS backends remain excluded. | Pass |
| X. Checkpoints | Commit the completed plan, then implementation phases after independently passing tests. | Pass |

**Post-design re-check**: Pass. The design preserves the existing authenticated
helper boundary and adds no direct daemon-to-HCS path.

## Project Structure

### Documentation (this feature)

```text
specs/009-windows-image-provisioning/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── image-cli.md
│   └── elevate-image-protocol.md
└── tasks.md
```

### Source Code (repository root)

```text
crates/
├── malt-image/                         # L1 OCI discovery, verification, owned image records
│   └── src/
├── malt-platform/src/isolation/
│   └── layers.rs                        # Windows HCS layer/scratch materialization and cleanup
├── malt-protocol/
│   ├── src/                             # Generated/typed protocol exports
│   └── tests/                           # VNP round trips for image requests/results
├── malt-elevate/src/                    # Privileged image preparation and HCS lifecycle owner
├── malt-daemon/src/                     # Image selection, reference accounting, session lifecycle
├── malt-bin/src/                        # `malt image` operator commands
└── malt-daemon/tests/                   # Cross-crate contained-session integration tests

schemas/
└── elevate.vexil                        # Opaque image IDs and image operation messages
```

**Structure Decision**: The ADR-required `malt-image` crate isolates OCI
network/archive logic from OS calls. `malt-platform` owns all HCS layer FFI.
`malt-elevate` owns the global privileged store and turns opaque IDs into HCS
paths; `malt-daemon` remains the policy and session-reference authority.

## Lifecycle and Integration Design

1. `malt image provision <reference>` asks the daemon to use the authenticated
   helper. The helper resolves the reference with `malt-image`, chooses only a
   Windows/amd64 manifest compatible with the current host policy, downloads
   every selected descriptor to a temporary helper-owned directory, and
   verifies each byte stream against its declared digest and size.
2. Only after verification does the helper atomically publish an immutable
   image record, keyed by manifest digest, under its MALT-owned global store.
   Registry reference is provenance, never identity. The helper imports and
   prepares the ordered read-only layer chain through
   `malt-platform::isolation::layers`; failures roll back only state created by
   that transaction.
3. The helper reports an opaque `ProvisionedImageId`, the immutable digest,
   selected platform, ordered layer descriptors, and a readiness assessment.
   The daemon records that view and obtains a fresh assessment before each
   contained-session request. A caller cannot submit a layer directory,
   scratch directory, or raw HCS configuration.
4. For a selected ready image, contained session construction asks the helper
   for a session-scoped writable layer tied to the image ID and session ID.
   The helper initializes and attaches that layer, creates the compute system
   with the verified ordered parents, then accepts the existing authenticated
   HCS process request. It reports `Performed` only after the HCS operation
   result succeeds.
5. Session destroy and dormant-session transition both tear down the compute
   system and writable layer before clearing the daemon isolation context. A
   later restore performs fresh readiness assessment and construction; it
   never revives a stale HCS handle or scratch path. Image removal is refused
   while the daemon has any active session reference.

## Complexity Tracking

No constitution violation requires justification.

