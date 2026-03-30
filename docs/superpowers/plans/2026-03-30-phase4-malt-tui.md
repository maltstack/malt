# Phase 4.2: malt-tui Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the TUI terminal client that renders RenderCommands via ratatui and captures input via crossterm.

**Architecture:** Style converter maps ResolvedStyle to ratatui::Style. Renderer applies RenderCommands to ratatui Buffer. Input handler maps crossterm events to VNP messages. App ties it together with a mock connection.

**Tech Stack:** Rust, ratatui 0.30, crossterm 0.29, malt-protocol

---

## File Structure

```
crates/malt-tui/
  Cargo.toml
  src/
    main.rs           — entry point
    app.rs            — App event loop
    render.rs         — RenderCommand → Buffer
    input.rs          — crossterm → VNP input
    style.rs          — ResolvedStyle → ratatui::Style
    connection.rs     — DaemonConnection trait + MockConnection
```

---

## Tasks

### Task 1: Crate scaffolding + style converter (4 tests)
### Task 2: Renderer — RenderCommand to Buffer (4 tests)
### Task 3: Input handler — crossterm to VNP (4 tests)
### Task 4: App + mock connection + main
### Task 5: Final verification
