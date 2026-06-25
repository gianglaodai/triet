---
name: user-communication-vietnamese
description: "User dùng tiếng Việt làm ngôn ngữ chính. Tone tùy ngữ cảnh — analogy webapp/Java khi user hỏi explain, technical hơn khi user hỏi detail."
metadata: 
  node_type: memory
  type: user
  originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---

User dùng tiếng Việt khi chat. Reply nên tiếng Việt trừ những chỗ buộc phải tiếng Anh:
- Code, identifier, error code (`E2100`), ADR title, file path
- Commit message — conventional format tiếng Anh (vd. `feat(v0.4.5): add witness table dispatch`)
- Doc comments rustdoc của public API
- Tên section trong SPEC/VISION khi cite

**Tone defaults:**
- Concise, technical đủ để actionable, không academic
- Khi user nói "giải thích lại cho non-engineer" / "tôi không có kiến thức X" — dùng analogy webapp / Java / Spring / npm / REST / database migration. Tránh compiler-theory jargon (SSA, monomorphization, witness table, …) trừ khi đã explain trước
- Khi present tradeoffs: phrase theo những gì user quan tâm (UX, philosophy, scope, risk timing, identity tam phân) — không bias theo performance/elegance trừ khi user hỏi
- Khi user hỏi detail technical cụ thể (vd. "BLAKE3 vs SHA-256", "witness table là gì") — OK technical hơn, user là dev và muốn hiểu

**Why:** User là webapp dev người Việt, không có kiến thức system/compiler/language-design. Ngôn ngữ + analogy phù hợp giúp user verify recommend trước khi approve. Pattern "build a house" analogy explain v0.1→v0.4 đã thấy work tốt nhiều lần.

**How to apply:** Reply tiếng Việt mặc định. Khi explain kiến trúc lớn, mở bằng analogy webapp/Java rồi mới đi vào detail. Khi present option, format `Option A — <ngắn>`, `Option B (recommend) — <ngắn>` rồi explain tradeoff bằng từ user-facing. Liên quan: [[user-role-webapp-dev-visionary]].
