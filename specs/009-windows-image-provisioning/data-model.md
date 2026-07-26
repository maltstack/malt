# Data Model: Windows contained-image provisioning

## ProvisionedImage

An immutable, helper-owned image identity exposed to the daemon and CLI.

| Field | Meaning | Validation |
|-------|---------|------------|
| `id` | Opaque MALT image identifier | Helper-issued; callers cannot construct paths from it. |
| `manifest_digest` | OCI manifest SHA-256 | Canonical `sha256:<64 hex>`; verified from downloaded bytes. |
| `source_reference` | Registry reference used to discover it | Provenance only; never used as mutable identity after provision. |
| `platform` | OS, architecture, optional OS version | Must be selected Windows/amd64 variant applicable to current host policy. |
| `layers` | Ordered immutable layer descriptors | Nonempty where the selected image requires them; every descriptor digest and size verified. |
| `state` | Acquisition/preparation/readiness state | See state machine below. |
| `prepared_at` | Time the prepared chain was published | Present only after atomic publish. |
| `last_assessment` | Current host readiness verdict and evidence | Refreshed before contained-session use. |

## PreparedLayerSet

The privileged representation of one `ProvisionedImage`'s read-only parent
chain. It is visible externally only through `ProvisionedImage` state.

| Field | Meaning | Validation |
|-------|---------|------------|
| `image_id` | Owning provisioned image | Must match the immutable manifest record. |
| `ordered_layer_ids` | Helper-owned layer identifiers in HCS parent order | Derived from verified descriptors; never caller supplied. |
| `preparation_receipt` | HCS import/setup outcome for this host | Every operation result completed successfully. |
| `owner_marker` | MALT ownership/version marker | Cleanup refuses directories without it. |

## WritableWorkspace

A session-scoped writable HCS layer built over a `PreparedLayerSet`.

| Field | Meaning | Validation |
|-------|---------|------------|
| `session_id` | Owning MALT session | Unique while active. |
| `image_id` | Selected provisioned image | Must be ready and retained. |
| `workspace_id` | Opaque helper-owned scratch identity | Never an exposed filesystem path. |
| `state` | Initializing, attached, active, tearing-down, removed | Forward transitions only except failure to `removed`. |
| `owner_marker` | MALT ownership/version marker | Required before detach/destroy/removal. |

## ImageReadinessAssessment

A current decision made by the helper for a specific `ProvisionedImage` on the
current host.

| Field | Meaning |
|-------|---------|
| `verdict` | `Ready` or `Unavailable` |
| `reason_code` | Stable, actionable class: missing Containers/HCS, unsupported platform, version policy refusal, corrupted store, preparation failure, or helper unavailable |
| `detail` | Sanitized operator explanation; never a privileged path |
| `assessed_host` | Windows version/capability facts used by the assessment |
| `assessed_at` | Time assessment was made |
| `evidence_basis` | Verified live HCS probe, verified preparation, or unavailable reason |

## ImageUseReference

Daemon-owned accounting record that prevents deletion while a contained
session has an active helper workspace.

| Field | Meaning |
|-------|---------|
| `image_id` | Referenced image |
| `session_id` | Dependent session |
| `workspace_id` | Helper workspace identity once allocated |
| `acquired_at` | Reference start |
| `release_state` | Active, release-pending, released |

The helper independently refuses removal of a workspace/image with an active
compute-system record; daemon accounting is not the sole safety check.

## State transitions

```text
Unseen
  -> Downloading
  -> Verified
  -> Preparing
  -> Prepared
  -> Ready | Unavailable

Downloading | Verified | Preparing --failure--> Discarded
Ready | Unavailable --host/image reassessment--> Ready | Unavailable
Ready --first active workspace--> InUse
InUse --last workspace removed--> Ready | Unavailable
Ready | Unavailable --remove--> Removed
```

`Discarded` and `Removed` have no selectable record. `InUse` removal is
refused. A preparation failure removes only the transaction's staging,
prepared-layer, and workspace artifacts.

