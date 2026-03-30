# Phase 4.2: malt-tui — TUI Terminal Client

**Date:** 2026-03-30
**Status:** Approved
**Scope:** `malt-tui` crate — ratatui + crossterm TUI client rendering RenderCommands
**Depends on:** malt-protocol (RenderCommand, InputEvent, ResolvedStyle)

---

## Architecture

TUI client that renders RenderCommand streams and captures terminal input. Built with ratatui (rendering) + crossterm (terminal I/O). Uses mock connection for now — real daemon connection deferred to integration phase.

---

## RenderCommand Mapping

| RenderCommand | ratatui |
|---|---|
| DrawText | buf.set_string(x, y, text, style) |
| DrawRect | Block::new().style(style).render(rect, buf) |
| DrawBorder | Block::bordered().style(style).render(rect, buf) |
| Clear | buf.reset() |
| SetClip | Track clip rect |
| ClearClip | Reset clip |
| WriteRaw | Raw stdout write within rect |
| Flush | Force refresh |

## Input Mapping

- crossterm KeyEvent → malt_protocol input::KeyEvent
- crossterm MouseEvent → malt_protocol input::MouseEvent
- Terminal resize → malt_protocol input::Resize

---

## Module Structure

```
malt-tui/
  Cargo.toml
  src/
    main.rs           — entry point, terminal setup/teardown
    app.rs            — App event loop, state
    render.rs         — RenderCommand → ratatui Buffer
    input.rs          — crossterm → VNP input
    style.rs          — ResolvedStyle → ratatui::Style
    connection.rs     — DaemonConnection trait + MockConnection
```

---

## Testing

- style.rs (4 tests): color conversion, modifiers, bold+italic combo, default style
- render.rs (4 tests): DrawText, DrawRect, Clear, multiple commands
- input.rs (4 tests): key mapping, mouse mapping, resize, modifier keys

---

## Deferred

- Real daemon VNP connection (integration phase)
- FrameAck backpressure (needs live connection)
- Mouse capture mode toggling
- Scrollback navigation
