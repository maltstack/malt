# Windows image provisioning live evidence — 2026-07-27

## Scope

This record covers the Windows HCS image lifecycle in Spec 009, including the
prepared-layer removal failure found during the initial quickstart and the
subsequent complete retry. It is local evidence from this Windows host; it is
not a claim about hosted CI or a different Windows build.

## Host and helper

- The elevated `MALT-Elevate` service completed its authenticated VNP
  hello/ack probe at protocol 3.
- Docker Desktop's active context was `desktop-linux` and its server reported
  `linux/amd64`. MALT nevertheless provisioned and ran the Windows image from
  `%ProgramData%\\MALT\\images`; no Docker image store or engine switch was
  used.
- The tested image was
  `mcr.microsoft.com/windows/nanoserver:ltsc2022`, resolved to immutable
  manifest `sha256:852bbe55ef9eddac52f2e11b90d24d0d5b0d2518344ec813cf14891f76a8d47f`,
  platform `windows/amd64`, OS version `10.0.20348.5386`.

## Initial failure and diagnosis

After a contained session was destroyed, `malt image remove` correctly saw
`active: 0`, but HCS returned `DestroyLayer HRESULT=0x80070020`. Restarting
the helper and daemon did not change that result; `hcsdiag list` contained no
MALT compute system. Inspection of the helper-owned session root found stale
writable layers from earlier contained sessions. Those session directories,
not an active image reference, retained the prepared parent layer.

## Remediation evidence

The helper now writes an immutable image lease next to every newly-created
private writable workspace. Normal teardown removes both the HCS workspace and
that lease. Image removal first reconciles matching leased workspaces. A
strict compatibility path handles pre-lease workspaces only when the target is
the sole helper-owned image record; it refuses rather than infer ownership when
multiple image records exist.

Focused verification completed successfully:

```text
cargo test -p malt-platform -p malt-elevate
# 95 malt-platform tests passed; 22 malt-elevate unit tests, 4 integration
# tests, and 2 operation-outcome tests passed (the explicit elevated SCM test
# remained ignored).

$env:MALT_REAL_HCS_IMAGE_ID='sha256:852bbe55ef9eddac52f2e11b90d24d0d5b0d2518344ec813cf14891f76a8d47f'
cargo test -p malt-daemon --test contained_image_session -- --ignored --nocapture
# real_contained_image_executes_and_removes_its_workspace ... ok
```

The real test provisions the image, creates a required contained session, runs
`cmd /c ver` through the HCS process path, observes active image use, destroys
the session, removes the image, and confirms it no longer appears in
`malt image list`. After the final run, the prepared image root was absent and
`hcsdiag list` showed only the pre-existing WSL and Codex cowork VMs, with no
MALT compute system.

After removal, `malt isolation capabilities` correctly returned contained
unavailable because no HCS-prepared MALT image remained selectable. This is
expected evidence of removal, not a containment regression.

## Final workspace validation

With the daemon and helper stopped only to release their Windows executable
locks, all of the following commands exited successfully on this workspace:

```text
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The full test run retained its explicit privileged/opt-in ignores. Its
Windows isolation reality suite printed the expected failed-allocation message
from the memory-limit child process while the enclosing test passed; this is
the assertion mechanism, not a suite failure.

## Refusal evidence

With the rebuilt helper installed and enrolled, both negative provisioning
requests exited nonzero and `malt image list` remained empty afterwards:

```text
malt image provision docker.io/library/alpine:3.20
# image provision failed: manifest index contains no selectable windows/amd64 descriptor

malt image provision mcr.microsoft.com/windows/nanoserver@sha256:000...000
# image provision failed: registry returned unexpected status 404 Not Found
```

The Docker Hub alias is normalized to `registry-1.docker.io` before invoking
the OCI Distribution API. This avoids treating Docker Hub's web hostname as a
successful manifest response and lets the first request reach the intended
Windows-platform selection refusal.

## Two-image active-use evidence

Two distinct prepared MALT-owned images were present at once: Nano Server
LTSC 2022 (`sha256:852bbe...a8d47f`, OS `10.0.20348.5386`) and Nano Server
1809 (`sha256:fbd86b...0a67a2`, OS `10.0.17763.9020`). A required contained
session 129 was created from the LTSC 2022 image. While it was active,
removing that image exited nonzero and named session 129. Removal of the 1809
image then succeeded, and inspection of the LTSC 2022 image still reported
`active: 1`. After `malt kill 129`, removal of the LTSC 2022 image succeeded;
the final image list was empty.
