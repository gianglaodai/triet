---
name: user-communication-vietnamese
description: "The user's primary language is Vietnamese. Tone depends on context — webapp/Java analogies when they ask for an explanation, more technical when they ask for detail. Note: every document is still written in English; only the conversation is Vietnamese."
metadata: 
  node_type: memory
  type: user
  originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---

The user chats in Vietnamese. Replies should be in Vietnamese, except where English is mandatory:
- Code, identifiers, error codes (`E2100`), ADR titles, file paths
- Commit messages — conventional format in English (e.g. `feat(v0.4.5): add witness table dispatch`)
- Rustdoc comments on public APIs
- Section names in SPEC/VISION when citing them
- **Every document in the repo** — all `.md` files are written entirely in English; `*.vi.md` is the only exception (locked 2026-08-02)

**Tone defaults:**
- Concise, technical enough to be actionable, never academic
- When the user says "explain it again for a non-engineer" / "I have no knowledge of X" — use webapp / Java / Spring / npm / REST / database-migration analogies. Avoid compiler-theory jargon (SSA, monomorphization, witness tables, …) unless it has already been explained
- When presenting trade-offs: frame them in terms the user cares about (UX, philosophy, scope, risk timing, the ternary identity) — do not bias toward performance or elegance unless asked
- When the user asks about a specific technical detail (e.g. "BLAKE3 vs SHA-256", "what is a witness table") — being more technical is fine; they are a developer and want to understand

**Why:** the user is a Vietnamese webapp developer with no system/compiler/language-design background. The right language and analogies let them verify a recommendation before approving it. The "build a house" analogy used to explain v0.1→v0.4 has worked well many times.

**How to apply:** reply in Vietnamese by default. When explaining a large piece of architecture, open with a webapp/Java analogy before the details. When presenting options, format them as `Option A — <short>`, `Option B (recommended) — <short>` and then explain the trade-off in user-facing terms. Related: [[user-role-webapp-dev-visionary]].
