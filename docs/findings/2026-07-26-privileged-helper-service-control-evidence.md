# Privileged-helper Service Control Manager evidence

**Date:** 2026-07-26  
**Feature:** `specs/008-privileged-helper/`  
**Host:** local Windows workstation; commands supplied from a high-integrity
PowerShell session (`Mandatory Label\\High Mandatory Level`).

## Observation

A disposable service named `MALT-Elevate-Probe-63184`, whose command was
`C:\\Windows\\System32\\cmd.exe /c exit 0`, was registered with `sc.exe` as a
demand-start service. `sc.exe query` then reported it as `STOPPED` with
`WIN32_EXIT_CODE 1077`. `sc.exe delete` succeeded, and the follow-up query
returned error `1060`, confirming that the service was no longer installed.

## What this establishes

- This host permits an explicitly elevated operator to create, inspect, and
  delete a local Windows service.
- The feature's service-registration test scenario can be exercised here once
  the actual `malt-elevate` service entry point and lifecycle wiring exist.

## Helper lifecycle follow-up

After the helper service and CLI were implemented, a normal (medium-integrity)
PowerShell invoked `malt elevate install`. Windows displayed the explicit UAC
consent flow, the elevated child completed successfully, and `malt elevate
status` reported `reachable` only after an authenticated generated-protocol
hello/ack exchange. The helper reported protocol version `2`.

The first removal attempt exposed a defect: `DeleteService` can mark a running
service for deletion while it continues to answer named-pipe requests. The
uninstall path was corrected to issue a service stop, wait for `STOPPED`, and
only then delete. A second UAC-approved `malt elevate uninstall` reported
success; `malt elevate status` then reported `not installed`, and `sc.exe
query MALT-Elevate` returned error `1060` (not installed).

## What this does not establish

- This proves SCM lifecycle, explicit UAC approval, and authenticated helper
  reachability. It does not prove session-entitlement validation, a rejected
  peer from another principal, replay refusal across the live channel, or any
  HCS containment operation.
