---
name: project-triet-overview
description: The Triết workspace structure plus pointers to the canonical docs. Version-agnostic — always read TODO/ROADMAP for the current state.
metadata: 
  node_type: memory
  type: project
  originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---

Triết (哲) is a balanced-ternary language on an OS-capable trajectory, written in Rust. Inspired by Setun (1958). ⚠️ STALE: this entry originally called it "AI-first"; that claim was REMOVED on 2026-06-22 (see VISION §5 tombstone).

**Workspace shape (Cargo, Rust 2024):**

⚠️ STALE pipeline (this is the DELETED v0.2-v0.10 compiler): `triet-lexer` → `triet-parser` → `triet-modules` → `triet-typecheck` → `triet-ir` → `triet-vm` → `triet-pack` → `triet-cli`. The current pipeline is in CLAUDE.md (`triet-lower` → `triet-mir` → `triet-borrowck` → `triet-jit` → `triet-driver`); `triet-ir`, `triet-vm`/`triet-interpreter`, `triet-bootstrap`, and `triet-cli` no longer exist.

Foundation crates: `triet-core` (Trit/Tryte/Integer/Long), `triet-logic` (Trilean Ł3/K3), `triet-syntax` (the AST arena).

**When you need the current state (version, phase, test count, commit hash)** — do not rely on this memory, read:
- `ROADMAP.md` — the v0.2.x → v3.0 phasing + a changelog of shipped phases
- `TODO.md` — the current sub-task + commit short hashes
- `docs/decisions/README.md` — the ADR index by phase
- `SPEC.md` — authoritative semantics
- `VISION.md` — the 5 architectural pillars
- `CLAUDE.md` — the collaboration model + conventions

**Why:** version-specific state drifts after every shipped phase. Memory keeps only what does NOT change (workspace shape, doc pointers, identity).

**How to apply:** when the user asks "what version are we on / what is the next phase / how many tests", read TODO.md + ROADMAP.md directly before answering. Never recall an old version snapshot. Related: [[project-vision-os-capable]], [[reference-spec]].
