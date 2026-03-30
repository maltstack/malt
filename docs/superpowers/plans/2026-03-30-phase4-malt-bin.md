# Phase 4.1: malt-bin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `malt` CLI command for daemon status, session management, and command forwarding via the gateway API.

**Architecture:** Clap derive for argument parsing, reqwest blocking client for HTTP, anyhow for errors. MaltClient wraps gateway REST API.

**Tech Stack:** Rust, clap 4 (derive), reqwest 0.13 (blocking), serde/serde_json, anyhow

---

## Tasks

### Task 1: Crate scaffolding + CLI + stubs
Create crate with Cargo.toml, cli.rs (clap commands), main.rs (dispatch), client.rs (stub), output.rs (formatting).

### Task 2: MaltClient HTTP implementation (5 tests)
Full HTTP client with JSON parsing. Tests for URL construction and response parsing.

### Task 3: CLI parse tests (4 tests)
Test clap parsing of subcommands and arguments.

### Task 4: Final verification
Clippy, binary build, workspace tests.

See the plan body in the previous message for complete code blocks.
