# Elevated image protocol contract

All new helper operations are VNP messages defined in `schemas/elevate.vexil`.
They are authenticated with the existing nonce/VNP handshake and are valid
only for an enrolled daemon identity. JSON, arbitrary local paths, archive
payloads, and HCS storage documents are not protocol fields.

## Requests

| Operation | Required fields | Helper rule |
|-----------|-----------------|-------------|
| `ProvisionImage` | public registry reference | Resolve/select a Windows/amd64 manifest; verify blobs; atomically prepare an owned record. |
| `ListImages` | none | Return sanitized immutable views of helper-owned records. |
| `InspectImage` | opaque ID or manifest digest | Resolve only a unique owned record and perform/return readiness assessment. |
| `RemoveImage` | opaque ID | Refuse if helper or daemon reports active workspace/reference; clean only owner-marked artifacts. |
| `PrepareContainedWorkspace` | session ID, opaque image ID | Re-assess readiness, initialize/attach a private writable layer, and construct the verified HCS compute system. |
| `TeardownContainedWorkspace` | session ID, opaque workspace ID | Stop/close compute system, detach/destroy the owned writable layer, then release the image reference. |

The existing `StartProcess` request stays process-only. It identifies the
already-created helper workspace/session context and duplicates only handles
from the authenticated peer process. It cannot create or select an image.

## Results

Every operation returns either `Performed` with opaque identifiers and a
sanitized `ImageReadinessAssessment`, or `Refused`/`Failed` with a stable
reason code. `Performed` for HCS construction is allowed only after each
asynchronous HCS operation has completed successfully.

## Boundary invariants

1. The daemon sends an image ID, never layer/scratch paths or arbitrary HCS
   JSON.
2. The helper constructs all privileged paths under its owned root and checks
   ownership markers before removal.
3. The helper re-verifies the selected immutable content and record before
   materializing it; a daemon-maintained display record cannot authorize HCS.
4. The helper cannot remove an image with a live compute system even if daemon
   accounting is stale.
5. Any uncertain transport result is reported as indeterminate and triggers
   helper-owned reconciliation before the resource becomes selectable again.

