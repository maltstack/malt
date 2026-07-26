# Legacy HCS layer compatibility check — 2026-07-27

## Question

Whether the existing `vexil-v2` Windows container material could supply the
missing image input for MALT's helper-owned HCS proof without switching Docker
Desktop to Windows-container mode.

## What was inspected

`vexil-v2` did not depend on Docker Desktop's image store. Its daemon pulled
OCI images itself, extracted them below `~/.vexil/images/layers`, prepared
them with HCS, and cached the resulting layers below
`C:\ProgramData\Vexil\windows-hcs-cache`.

This host still has the following prepared base layer:

```text
C:\ProgramData\Vexil\windows-hcs-cache\windows-nanoserver-ltsc2022\layer-0
```

It contains `Files`, `Hives`, and `blank-base.vhdx`, so it is an HCS-prepared
Windows Server 2022 / Nano Server LTSC 2022 layer, not a Docker Desktop cache.

The host is now Windows build `10.0.26200.8875` (25H2). The legacy Vexil
metadata and its HCS-layer test fixture identify the cached Nano Server image
as build `10.0.20348` (LTSC 2022). Vexil's compatibility test explicitly
allows a 26200 host to use that older image under its relaxed policy, so build
numbers alone do not explain the failed MALT construction.

## Live boundary evidence

The installed MALT helper was reachable and the current daemon was explicitly
enrolled through the UAC-backed `malt elevate authorize-daemon <daemon-pid>`
operation. A required contained-session request crossed that authenticated
helper boundary and reached `HcsCreateComputeSystem`.

HCS refused construction asynchronously:

```text
HRESULT=0x80071126
OperationFailure: Construct
```

The existing MALT helper still renders `Storage.Layers` as an empty list and
uses the session directory directly as `Storage.Path`. It does not prepare a
layer chain or initialise an HCS writable layer. The legacy cache supplies
one prepared read-only layer, but MALT lacks the trusted image selection,
layer-chain validation, writable-scratch preparation, and configuration
assembly needed to use it. Therefore the generic HCS `Construct` failure does
not establish a host/image incompatibility.

## Correction to the Docker inference

Docker Desktop remains in Linux-container mode, but switching it was never a
technical prerequisite for Vexil's old path. The relevant limitation is the
missing MALT-owned image and HCS-layer preparation contract—not Docker's
active engine mode.

## What this does not establish

- It does not prove the existing 20348 layer will complete a contained MALT
  session once MALT has the missing layer-provisioning pipeline.
- It does not implement or vendor Vexil's OCI image pull, image validation,
  layer preparation, or cache lifecycle. Those are a separate feature.
- It does not establish a contained MALT session or an HCS-created child
  process on the current host.

## Result

Contained isolation remains unavailable and required requests remain
fail-closed. The next live proof needs a reviewed MALT-owned image/layer
provisioning feature; it may use the legacy cache as a test fixture, but must
not rely on it as an undocumented runtime dependency.
