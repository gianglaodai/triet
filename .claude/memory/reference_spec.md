---
name: reference-spec
description: Pointer tới canonical docs Triết. SPEC/VISION/ROADMAP/TODO/ADR index là source of truth — không recall snapshot từ memory.
metadata: 
  node_type: memory
  type: reference
  originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---

Source-of-truth docs trong repo (luôn ưu tiên đọc trực tiếp thay vì recall memory):

- `/mnt/M2-STORAGE/Work/workspace/gh/rust/triet/SPEC.md` — language semantics authoritative (lexical, type system, arithmetic, logic, modules, generics, memory model, operator precedence, …)
- `/mnt/M2-STORAGE/Work/workspace/gh/rust/triet/VISION.md` — 5 trụ cột kiến trúc + OS-capable trajectory
- `/mnt/M2-STORAGE/Work/workspace/gh/rust/triet/ROADMAP.md` — phasing v0.2.x → v3.0 + changelog các phase đã ship
- `/mnt/M2-STORAGE/Work/workspace/gh/rust/triet/TODO.md` — sub-task hiện tại + commit short-hashes
- `/mnt/M2-STORAGE/Work/workspace/gh/rust/triet/docs/decisions/README.md` — ADR index theo phase + cách đọc/viết ADR
- `/mnt/M2-STORAGE/Work/workspace/gh/rust/triet/CLAUDE.md` — collaboration model, conventions, dev cadence, error code namespace

**Why:** Memory drift sau mỗi phase ship; canonical docs trong git là ground truth. Section numbering trong SPEC có thể đổi giữa các version → grep trực tiếp thay vì cite section number từ memory.

**How to apply:**
- Trước khi trả lời câu hỏi về semantics, đọc SPEC.md trực tiếp (grep theo keyword, đừng cite số section từ memory)
- Khi cần ADR rationale, đọc file ADR cụ thể trong `docs/decisions/`
- Khi user reference "v0.x đã làm gì", đọc ROADMAP.md changelog
- Khi đề xuất implementation, cite SPEC section / ADR number tương ứng thay vì invent semantics
