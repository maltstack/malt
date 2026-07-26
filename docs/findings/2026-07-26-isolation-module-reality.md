# Isolation module reality survey — 2026-07-26

## Scope and method

This is the pre-implementation survey for Spec Kit feature 007. It checks
whether each isolation module has a production caller outside its defining
crate and whether its tests perform OS operations rather than only construct
Rust values. The workspace baseline was `cargo test --workspace`: **1,448
tests passed, 0 failed**.

## Findings

| Module | Production caller outside `malt-platform` | Test behaviour | Result for feature 007 |
|---|---|---|---|
| `job_objects` | Yes: `malt-daemon::session_thread` puts its handle in MASH's environment | Real Win32 creation, assignment, query and teardown tests | Usable for the Job Object capability it actually provides. |
| `hcs` | No | Config-validation and fake-mode coverage; no native session launch | Cannot satisfy `Contained`; the default build does not enable its `hcs` feature and MASH is not HCS-aware. |
| `tokens` | No | Real token-handle creation tests, but no process-spawn integration | A token can be made, but no MALT process runs with it. Not containment. |
| `namespaces`, `cgroups`, `seccomp`, `overlayfs` | No | Module-local probes and configuration tests; no session process observed under them | Not claimable until spawn-path wiring and external observation exist. |
| `rlimit`, `sandbox` | No | Platform-local API/configuration tests; no session process observed under them | Not claimable until spawn-path wiring and external observation exist. |

## Consequence

The implementation must fail closed for `Contained` instead of re-labelling a
Job Object as a container. It may only report a Job Object tier as **assumed**
until a spawned child is observed in the object. Linux and macOS mechanisms
remain unavailable to session creation in this change because the existing
modules are not wired to a process spawn path.

## What this does not establish

This Windows-native run does not prove HCS, Linux namespaces/cgroups/seccomp,
or macOS sandboxing work on a capable host. It also does not prove that a
Job Object's memory limit binds for a MASH child; the feature needs that
external-observation test before claiming `Verified`.
