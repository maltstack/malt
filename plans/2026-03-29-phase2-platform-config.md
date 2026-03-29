# Phase 2: `malt-platform` Core + `malt-config` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the two L0 foundation crates: `malt-platform` (PTY, process spawning, signals, sockets, env) and `malt-config` (Vexil Store config loading). These unblock all L1 Phase 2 crates.

**Architecture:** `malt-platform` abstracts OS interactions behind traits with per-platform implementations in separate files. `malt-config` wraps `vexil-store` for schema-validated `.vx` config loading. Reference code at vexil-v2 — port the logic, rewrite with proper quality. Spec: `malt/specs/phase2-platform-config.md`.

**Tech Stack:** Rust, nix (Unix), windows-sys (Windows), tokio (optional), vexil-store, thiserror, tracing

---

## Tasks Overview

| Task | What | Files |
|------|------|-------|
| 1 | Crate scaffold | Cargo.toml, lib.rs, module stubs |
| 2 | PTY module | pty/mod.rs, unix.rs, windows.rs, tests |
| 3 | I/O + env | io.rs, env.rs |
| 4 | Signals module | signals/mod.rs, unix.rs, windows.rs, tests |
| 5 | Process spawning | process/mod.rs, unix.rs, windows.rs, tests |
| 6 | Sockets module | sockets/mod.rs, unix.rs, windows.rs, tests |
| 7 | malt-config crate | Cargo.toml, build.rs, lib.rs, paths.rs, schemas, tests |
| 8 | Architecture spec update | specs/architecture.md |

Tasks 1-6 are `malt-platform`. Task 7 is `malt-config`. Task 8 is documentation.

Dependencies: 1 → {2,3,4,5,6} → 7 (independent). Task 8 independent.

Reference implementation: `C:\Users\mamuk\projects\vexil-v2\vexil-platform\src\`

**IMPORTANT:** Port the logic and architecture from the reference. Rewrite the code with proper quality — the reference's code organization and style were weak. Key patterns to preserve: trait-based abstraction, cfg-split platform files, SAFETY comments on all unsafe, OwnedFd/RAII for handles, single source of truth for signal mapping.

See the full plan at the spec file for complete code listings for each task.
