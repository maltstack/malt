# Phase 3B: Compat Translator

**Date:** 2026-03-30
**Status:** Approved
**Scope:** `malt-compat` crate — VT parser, terminal grid, compat translator emitting FrameElements
**Depends on:** malt-protocol (FrameElement, ResolvedStyle)

---

## Context

The Compat Translator is the ONLY component in MALT that handles VT escape codes. It reads raw bytes from legacy programs, parses VT sequences, maintains a virtual terminal grid, and emits `VtPassthrough` FrameElements for the renderer pipeline.

---

## VT Parser Core

### Components

- **VtParser** — wraps the `vte` crate state machine, implements `vte::Perform` to drive grid updates
- **TerminalGrid** — virtual screen buffer: 2D grid of cells, cursor, scroll region, current style
- **Cell** — character + style per grid position
- **CompatTranslator** — orchestrates parser + grid, exposes public API for feeding bytes and emitting FrameElements

### TerminalGrid State

- 2D grid of `Cell` (character + ResolvedStyle)
- Cursor position (row, col) with wrap flag
- Scroll region (top, bottom rows)
- Current style (accumulated SGR state)
- Dirty flag per row for efficient emission

### Output

After processing bytes, the translator emits `FrameElement::VtPassthrough` containing the raw bytes. Clients with VT passthrough capability render directly; clients without skip the element (graceful degradation handled by renderer).

---

## Deployment Modes

**In-process (this sub-project):** `CompatTranslator` runs in session executor thread. Simple, no IPC.

**Out-of-process (Phase 3E):** Separate `malt-compat-worker` binary with heartbeat/restart logic. Deferred to Process Supervisor sub-project. The translator core is the same — extraction is a deployment concern.

---

## Module Structure

```
malt-compat/
  src/
    lib.rs              — crate root, re-exports
    error.rs            — CompatError enum
    grid.rs             — TerminalGrid: 2D cell buffer, cursor, scroll region
    cell.rs             — Cell: character + style, Row type
    parser.rs           — VtParser: vte wrapper, Perform impl
    translator.rs       — CompatTranslator: orchestrates parser+grid, emits FrameElements
  tests/
    cell.rs             — cell/row operations
    grid.rs             — grid cursor movement, scrolling, erase
    parser.rs           — VT sequence parsing → grid state changes
    translator.rs       — end-to-end: bytes in → FrameElements out
```

### Dependencies

- `malt-protocol` — FrameElement, ResolvedStyle
- `vte` — VT escape sequence parser
- `thiserror`, `tracing`

### Public API

```
CompatTranslator::new(cols: u16, rows: u16) -> Self
CompatTranslator::feed(&mut self, data: &[u8])
CompatTranslator::frame_element(&self) -> FrameElement
CompatTranslator::resize(&mut self, cols: u16, rows: u16)
CompatTranslator::is_dirty(&self) -> bool
CompatTranslator::clear_dirty(&mut self)
```

### Not In Scope

- Out-of-process worker binary (Phase 3E)
- Heartbeat/restart logic (Phase 3E)
- Memory limit enforcement (Phase 3E, via isolation context)
- Scrollback storage (Phase 3D)

---

## Testing Strategy

### cell.rs (4 tests)
- Default cell is space with default style
- Cell with character and custom style
- Row creation with correct width
- Row clear resets all cells

### grid.rs (8 tests)
- Cursor movement (absolute, relative)
- Line wrapping at column boundary
- Scroll up shifts rows, clears bottom
- Scroll down shifts rows, clears top
- Erase line (to end, to start, whole)
- Erase screen (to end, to start, whole)
- Scroll region limits scrolling
- Resize preserves content where possible

### parser.rs (6 tests)
- Plain text writes characters to grid
- SGR sets foreground/background colors
- SGR sets attributes (bold, italic, underline)
- Cursor movement sequences (CUU, CUD, CUF, CUB, CUP)
- Erase sequences (ED, EL)
- Unknown sequences ignored (no crash)

### translator.rs (4 tests)
- Feed bytes → VtPassthrough FrameElement with raw data
- Resize updates grid dimensions
- Dirty tracking: feed sets dirty, clear_dirty resets
- Empty feed → not dirty
