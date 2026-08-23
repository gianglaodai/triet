---
name: reference-spec
description: Pointers to Triết's canonical docs. SPEC/VISION/ROADMAP/TODO and the ADR index are the source of truth — never recall a snapshot from memory.
metadata: 
  node_type: memory
  type: reference
  originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---

Source-of-truth docs in the repo (always read them directly rather than recalling memory):

- `/mnt/M2-STORAGE/Work/workspace/gh/rust/triet/SPEC.md` — authoritative language semantics (lexical, type system, arithmetic, logic, modules, generics, memory model, operator precedence, …)
- `/mnt/M2-STORAGE/Work/workspace/gh/rust/triet/VISION.md` — the 5 architectural pillars + the OS-capable trajectory
- `/mnt/M2-STORAGE/Work/workspace/gh/rust/triet/ROADMAP.md` — the v0.2.x → v3.0 phasing + a changelog of shipped phases
- `/mnt/M2-STORAGE/Work/workspace/gh/rust/triet/TODO.md` — the current sub-task + commit short hashes
- `/mnt/M2-STORAGE/Work/workspace/gh/rust/triet/docs/decisions/README.md` — the ADR index by phase + how to read and write ADRs
- `/mnt/M2-STORAGE/Work/workspace/gh/rust/triet/CLAUDE.md` — the collaboration model, conventions, dev cadence, error-code namespace

**Why:** memory drifts after every shipped phase; the canonical docs in git are ground truth. Section numbering in SPEC can change between versions → grep directly instead of citing a section number from memory.

**How to apply:**
- Before answering a semantics question, read SPEC.md directly (grep by keyword, do not cite a section number from memory).
- When you need an ADR's rationale, read that specific ADR file in `docs/decisions/`.
- When the user refers to "what v0.x did", read the ROADMAP.md changelog.
- When proposing an implementation, cite the corresponding SPEC section or ADR number instead of inventing semantics.
