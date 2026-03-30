# Phase 3F: API Gateway Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an HTTP REST API gateway for MALT with local auth, rate limiting, and FrameElement-to-JSON serialization.

**Architecture:** Extractable subsystem — communicates through `GatewayBackend` trait, no direct daemon dependency. Axum handles routing, tower middleware handles auth and rate limiting. Mock backend for testing.

**Tech Stack:** Rust, axum 0.8, tokio, serde/serde_json, tower, malt-protocol, thiserror, tracing

---

## File Structure

```
crates/malt-gateway/
  Cargo.toml
  src/
    lib.rs              — crate root, re-exports
    error.rs            — GatewayError enum with IntoResponse
    types.rs            — request/response types
    backend.rs          — GatewayBackend trait
    auth.rs             — AuthScope, AuthContext
    rate_limit.rs       — TokenBucket rate limiter
    shadow.rs           — FrameElement to JSON
    server.rs           — build_router function
    routes/
      mod.rs            — route module declarations
      health.rs         — GET /health
      sessions.rs       — session CRUD + exec/send/output
      panes.rs          — pane list/split/close
  tests/
    auth.rs             — scope hierarchy tests
    rate_limit.rs       — token bucket tests
    shadow.rs           — FrameElement to JSON tests
    routes.rs           — endpoint integration tests
```

---

## Tasks

### Task 1: Crate scaffolding + error + types + backend trait

Create the crate with Cargo.toml, error.rs (GatewayError with IntoResponse), types.rs (API request/response structs), backend.rs (GatewayBackend trait), lib.rs, and stub modules. Add to workspace. See spec for exact type definitions.

### Task 2: Auth + Rate Limiting (4+4 tests)

Implement AuthScope enum (Monitor < Read < Interact < Admin), AuthContext with scope checking, and TokenBucket rate limiter with per-client isolation.

### Task 3: Shadow Tree Serialization (4 tests)

Implement frame_to_json() that converts FrameElement trees to semantic JSON. Text, Paragraph, Split, VtPassthrough variants.

### Task 4: Routes + Server (6 tests)

Implement axum routes for health, sessions (list/create/get/destroy/exec/send/output), and panes (list/split/close). Mock backend for testing. build_router() function.

### Task 5: Final Verification

Clippy, full workspace tests.

---

See the full task descriptions with complete code in the plan body above. Each task includes exact file paths, complete code blocks, test code, and commit messages.
