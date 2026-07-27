# Quickstart: prove a real Windows contained image lifecycle

This is the required end-to-end validation path after implementation. It
complements the focused unit and integration tests; it does not replace them.

## Prerequisites

- A Windows host with the Containers feature and HCS available.
- A freshly built MALT workspace. Installation copies the helper to the
  administrator-owned `%ProgramFiles%\MALT\malt-elevate.exe`; the service
  never executes Cargo's `target\debug\malt-elevate.exe` build output.
- The elevated helper installed and reachable:

  ```powershell
  .\target\debug\malt.exe elevate install
  .\target\debug\malt.exe elevate status
  ```

- A running MALT daemon that has been authorized with the helper. Docker
  Desktop may be absent or running Linux containers; this workflow must not
  use its image store or engine.

## 1. Provision and inspect

Provision a current supported public Microsoft Windows base image reference.
Record the exact command and resulting manifest digest in a dated finding.

```powershell
.\target\debug\malt.exe image provision <public-windows-image-reference>
.\target\debug\malt.exe image list
.\target\debug\malt.exe image inspect <image-id>
```

Expected:

- One immutable `sha256:` manifest identity.
- Selected `windows/amd64` platform and OS version.
- Every layer listed in order.
- `readiness: ready` only after helper preparation/assessment succeeded; an
  unavailable result names the reason rather than claiming containment.

## 2. Refusal checks

Attempt to provision a non-Windows or wrong-platform reference and a deliberately
invalid digest/reference. Each command must fail without adding a selectable
image record. Inspect the helper-owned image list after each failure.

## 3. Required contained execution

Create a session with the provisioned image and required containment:

```powershell
.\target\debug\malt.exe new --name contained-proof --isolation contained --image <image-id>
.\target\debug\malt.exe exec <session-id> "cmd /c ver"
.\target\debug\malt.exe isolation capabilities
```

Expected:

- Session creation reports `Contained` established and the selected immutable
  manifest digest.
- The external command is launched through the HCS process path and returns
  its output/exit status.
- Capabilities describe the real session-path mechanism, not merely an OS
  primitive.

## 4. Cleanup and in-use protection

While the session is active, try removal:

```powershell
.\target\debug\malt.exe image remove <image-id>
```

It must refuse and name the dependent session. Then destroy the session and
inspect the absence of the helper-owned writable workspace and HCS compute
system using the feature's diagnostic command/test evidence. Finally remove
the image:

```powershell
.\target\debug\malt.exe kill <session-id>
.\target\debug\malt.exe image remove <image-id>
.\target\debug\malt.exe image list
```

Expected: removal succeeds only after teardown; no record or owned prepared
state remains. Repeat the destroy path after an intentionally failed HCS
construction to prove rollback.

## 5. Spec 008 closeout evidence

Use the successful required contained command and cleanup record to complete
Spec 008's live HCS tasks. Run the full required test, format, and lint suite
from the rebuilt workspace, then append the exact host/image/helper evidence
to the dated finding before marking the old tasks complete.
