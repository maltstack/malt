<!--
Sync Impact Report — 2026-07-24 (re-run via /speckit-constitution)
Version change: 1.0.0 → 1.0.1 (PATCH)
Rationale: This constitution was originally seeded from CLAUDE.md's "Hard
Invariants" without running the /speckit-constitution skill (skill was
unavailable in that session). Running it now: no principle content changed,
version normalized to MAJOR.MINOR.PATCH form, and propagation to dependent
templates/commands was verified rather than assumed.
Modified principles: none (wording unchanged)
Added sections: none
Removed sections: the "seeded, not drafted" caveat paragraph (superseded —
the skill has now actually run)
Templates checked:
  - .specify/templates/plan-template.md — ✅ no update needed (Constitution
    Check section is a dynamic placeholder, not principle-specific text)
  - .specify/templates/spec-template.md — ✅ no update needed (no
    constitution-specific references)
  - .specify/templates/tasks-template.md — ✅ no update needed (no
    constitution-specific references)
  - .claude/skills/speckit-*/SKILL.md, .agents/skills/speckit-*/SKILL.md —
    ✅ generic Spec Kit command files, no CLAUDE-only or stale references
  - AGENTS.md — ✅ already documents .specify/memory/constitution.md and
    docs/adr/ correctly, no change needed
Follow-up TODOs: none
-->

# MALT Constitution

## Core Principles

### I. VT Codes Confined
No crate other than `malt-compat` may import `vte` or handle escape
sequences. The daemon is the terminal authority, not any client — VT
handling is a compatibility-shim concern, not something that should leak
into the protocol or session model.

### II. OS Calls Confined
No `nix`, `windows-sys`, `libc`, or `std::os::unix` outside `malt-platform`.
Every other crate goes through `malt-platform`'s cross-platform
abstractions. This is what makes the rest of the codebase portable without
`#[cfg]` sprawl.

### III. Dependency-Free Foundations
`malt-protocol` has zero workspace dependencies (external deps only).
`malt-plugin-sdk` has zero internal deps. These are the crates other
projects might consume standalone — they can't carry the rest of MALT with
them.

### IV. Safety Is Explicit
Every `unsafe` block requires a `// SAFETY:` comment explaining why it's
sound. No `unwrap()` or `expect()` outside `#[cfg(test)]` code. Two real
bugs (undersized `IO_COUNTERS` struct, hardcoded active-process-limit of 1
in `job_objects.rs`) were found in 2026-07 specifically because tests
exercised real code paths instead of constructing structs directly — trust
tests that prove something, not tests that pass by construction.

### V. VNP Is the Only Inter-Component Protocol
No component talks to another except through VNP (Vexil Native Protocol) —
typed, schema-defined, bitpack-encoded. No ad-hoc JSON side-channels
between components that are supposed to be behind the protocol boundary.

### VI. The Shell Ships When POSIX Conformance Passes
Smoosh (183/183 on native Windows, 186/186 WSL) is the bar for `mash`
being considered correct, not a subjective "feels done."

### VII. Layer Violations Are Compile Errors, Not Conventions
No upward dependencies in the crate graph (L0 → L1 → L2 → L3). Enforced by
Rust's own visibility/orphan rules plus `deny.toml`, not by remembering a
rule.

### VIII. Vendor, Never Depend on Unstable Siblings
Established in ADR-0001 after reverting an uncommitted dependency on
`malt-stack` (itself abandoned, zero commits, most rings still
placeholders). If something from a sibling project is genuinely useful,
port it in as owned source, rewritten to match these invariants — never a
live path/git dependency on a project that isn't itself stable and
versioned.

### IX. No Silent Scope-Jumps
If something looks like it needs a bigger rethink mid-task, that becomes a
written, deferred proposal (an ADR draft, a backlog item) — not an
in-flight pivot. This single habit, applied at any point, would have
stopped the vexil-v2 → malt → malt-stack rewrite chain. See
`docs/BACKLOG.md`'s "Done" section and `docs/adr/` for what this looks like
in practice.

### X. Commit at Real Checkpoints
This project was abandoned three times after multi-day uncommitted sprints.
If a change is big enough to feel risky to commit, that's a signal to
commit sooner, not later.

## Documentation System

- `docs/adr/` — decisions and their reasoning (MADR-adjacent format).
  Check before re-deciding something already settled.
- `docs/findings/` — dated evidence from actually running/testing the
  product. Not conclusions — the "how do we know this" record.
- `docs/BACKLOG.md` — living, prioritized "what's next and why."
- `specs/` — Spec Kit's per-feature specs (`specs/NNN-name/`), created via
  `/speckit-specify`. For new feature work going forward.
- `docs/design/architecture.md` — the target-state design doc (~2,380
  lines), "Draft v0.1" — describes where the system is meant to go, not a
  status tracker of what's built.
- `docs/superpowers/` — deprecated 2026-07-24. No new content. Historical
  plans/specs left as point-in-time record.

## Governance

This constitution supersedes ad-hoc practice where they conflict.
Amendments should be documented (an ADR is the right place) with the
reasoning for the change, not just the change itself — the "why" is what
lets future work judge edge cases the letter of the rule doesn't cover.

**Version**: 1.0.1 | **Ratified**: 2026-07-24 | **Last Amended**: 2026-07-24
