# Research: Windows OCI image provisioning and HCS materialization

## Decision: MALT owns the image and prepared-layer lifecycle

Use an elevated-helper-owned global image store rather than Docker Desktop,
the legacy Vexil cache, or arbitrary user directories. The helper accepts an
image reference and returns opaque image IDs; it is never asked to interpret a
daemon-supplied layer path or HCS storage document.

**Rationale**: Docker Desktop exposes one active engine mode and is not a
security boundary MALT controls. Vexil-v2's cache is neither MALT-owned nor
adequately verified. The current helper already authenticates the daemon and
owns HCS process creation, so it is the correct owner for privileged prepared
layers and scratch state.

**Alternatives considered**:

- Rely on Docker's Windows image store: rejected; it makes MALT depend on
  Docker's engine selection and cache lifetime.
- Reuse `C:\\ProgramData\\Vexil`: rejected; an external cache is not an owned
  trust root and no Vexil dependency is permitted.
- Let the daemon pass prepared absolute paths: rejected; it reintroduces raw
  privileged filesystem authority across the helper boundary.

## Decision: Separate OCI acquisition from Windows HCS materialization

Create the ADR-required `malt-image` L1 crate for registry manifest selection,
content-digest verification, safe archive decoding, and immutable image
records. Add `malt-platform::isolation::layers` for HCS import/setup, writable
layer initialization, storage-filter attachment/detachment, and owned cleanup.

**Rationale**: OCI is network/archive logic, while `HcsImportLayer`,
`HcsInitializeWritableLayer`, and storage filter calls are Windows OS APIs.
This implements ADR-0005's layer split and preserves the platform boundary.

**Alternatives considered**:

- Implement HCS calls inside the registry crate: rejected by Constitution II.
- Extend the daemon with direct HCS calls: rejected because it bypasses the
  authenticated privileged helper.

## Decision: First release accepts only a tested public Windows OCI variant

Support public registry references that resolve to an explicitly selected
Windows/amd64 manifest with an immutable manifest digest. The first live test
uses a current Microsoft base image reference/digest selected at test time;
the recorded manifest identity, not a mutable tag, becomes the provisioned
identity. Private registries and user-provided tar archives are not accepted.

**Rationale**: Windows image compatibility depends on host/image version and
isolation mode. Microsoft does not maintain `latest` Windows base-image tags;
a manifest must be selected and assessed rather than assumed runnable.

**Alternatives considered**:

- Accept any OCI archive: rejected; it makes archive/path validation and
  provenance unbounded for the initial containment feature.
- Treat a downloaded tag as ready: rejected; it violates FR-003 and repeats
  the current empty-layer false claim.

## Decision: Verify bytes before publishing state

Download descriptor bytes to a helper-owned staging area, compare stream size
and SHA-256 to the selected descriptor, safely decode the archive without
path traversal, duplicate entries, symbolic links, or special files. A hard
link is accepted only when it names a regular file in the same verified layer,
after the full archive has been validated, because standard Microsoft Windows
base layers use those links for equivalent system data. Then atomically
publish the immutable record and materialized layers. Failed staging and HCS
transactions remove only their own state.

**Rationale**: Descriptor digests are the OCI identity boundary. A content
addressed store cannot truthfully report the manifest/layer identity unless it
has checked the received bytes. Transactional publication ensures partial
imports do not become selectable.

**Alternatives considered**:

- Trust `Content-Length` or registry headers: rejected; neither verifies blob
  identity.
- Extract directly into the final prepared-layer directory: rejected; failed
  extraction can look like a usable cache.

## Decision: Materialize parent layers and scratch in HCS order

For every verified ordered parent layer, create/import its MALT-owned HCS
layer representation. Per session, initialize a distinct writable layer over
the ordered parent descriptors, attach its storage filter, create/start the
compute system from that verified chain, and use the existing HCS process
handoff. Teardown destroys the compute system, closes handles, detaches the
storage filter, and removes only that session's owned writable layer.

**Rationale**: HCS documents import, writable-layer initialization, storage
filter attachment, and explicit teardown as distinct operations. The current
empty `Storage.Layers` configuration has no usable image filesystem.

**Alternatives considered**:

- Keep `Storage.Layers: []`: rejected; it cannot provide an image-backed
  contained environment.
- Share one writable layer: rejected; one session could mutate another and
  cleanup/reference accounting would be unsound.

## Decision: Readiness is assessed before use, not inferred from cache

Store acquisition, preparation, and a last successful HCS probe separately.
Before new contained construction, the helper re-evaluates Containers/HCS
availability and selected Windows image compatibility. It returns a concrete
not-ready reason rather than reporting cached preparation as an established
container.

**Rationale**: The current host build is newer than the legacy image build;
legacy Vexil source permits a relaxed pairing, but only a live HCS operation
proves the combination works on this host. Host upgrades and feature changes
also invalidate cached assumptions.

**Alternatives considered**:

- Reject solely because build numbers differ: rejected; version policy and
actual HCS execution must be evaluated together.
- Declare readiness after a manifest download: rejected by FR-003.

## Decision: Dormancy destroys ephemeral containment

Destroy helper-owned HCS and writable-layer state when a contained session
becomes dormant, and rebuild it after a fresh readiness assessment when the
session later needs external process execution. Active image references are
held only while that ephemeral state exists.

**Rationale**: The daemon currently can drop an isolation context during
dormancy without helper teardown; a live compute system or scratch directory
would then be orphaned. Persisting HCS handles across helper restarts would be
less honest and more complex.

**Alternatives considered**:

- Persist HCS handles/scratch as session state: rejected; HCS process state
  and helper enrollment are not safely resumable after restart.

## Sources and prior-art limits

- [HCS API overview](https://learn.microsoft.com/en-us/virtualization/api/hcs/reference/apioverview)
  establishes asynchronous operation result handling.
- [HcsImportLayer](https://learn.microsoft.com/en-us/virtualization/api/hcs/reference/hcsimportlayer),
  [HcsInitializeWritableLayer](https://learn.microsoft.com/en-us/virtualization/api/hcs/reference/hcsinitializewritablelayer),
  and [HcsAttachLayerStorageFilter](https://learn.microsoft.com/en-us/virtualization/api/hcs/reference/hcsattachlayerstoragefilter)
  establish the separate parent and writable-layer operations.
- [Windows container version compatibility](https://learn.microsoft.com/en-us/virtualization/windowscontainers/deploy-containers/version-compatibility)
  explains why manifest platform/version data must be assessed against the
  actual host.
- ADR-0005 is the binding architecture decision for the new `malt-image`
  crate and `malt-platform::isolation::layers` split.
- Vexil-v2 is reference-only. Its production route proves that image
  acquisition and HCS preparation must both be reachable; its missing
  end-to-end proof and unsafe cache/privilege assumptions are not adopted.
