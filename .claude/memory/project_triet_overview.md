---
name: project-triet-overview
description: Workspace structure Triết + pointers tới canonical docs. Version-agnostic — luôn đọc TODO/ROADMAP để biết state hiện tại.
metadata: 
  node_type: memory
  type: project
  originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---

Triết (哲) là ngôn ngữ balanced-ternary, AI-first, OS-capable trajectory, viết bằng Rust. Cảm hứng từ Setun (1958).

**Workspace shape (Cargo, Rust 2024):**

Pipeline crates: `triet-lexer` → `triet-parser` → `triet-modules` → `triet-typecheck` → `triet-ir` → `triet-vm` → `triet-pack` → `triet-cli`.

Foundation crates: `triet-core` (Trit/Tryte/Integer/Long), `triet-logic` (Trilean Ł3/K3), `triet-syntax` (AST arena).

**Khi cần biết state hiện tại (version, phase, test count, commit hash)** — đừng dựa vào memory này, đọc:
- `ROADMAP.md` — phasing v0.2.x → v3.0 + changelog các phase đã ship
- `TODO.md` — sub-task hiện tại + commit short-hashes
- `docs/decisions/README.md` — ADR index theo phase
- `SPEC.md` — semantics authoritative
- `VISION.md` — 5 trụ cột kiến trúc
- `CLAUDE.md` — collaboration model + conventions

**Why:** State version-cụ-thể drift sau mỗi phase ship. Memory chỉ giữ những thứ KHÔNG đổi (workspace shape, doc pointers, identity).

**How to apply:** Khi user hỏi "đang ở version nào / phase tiếp theo là gì / bao nhiêu test", đọc TODO.md + ROADMAP.md trực tiếp trước khi trả lời. Không recall snapshot version cũ. Liên quan: [[project-vision-os-capable]], [[reference-spec]].
