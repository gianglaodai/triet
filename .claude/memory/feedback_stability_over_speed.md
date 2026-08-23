---
name: Stability over speed — decision discipline
description: The user explicitly ranks stability and certainty above delivery speed for Triết's architectural decisions. A slow decision backed by an ADR beats shipping something and fixing it later.
type: feedback
originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---
**Rule:** every major architectural decision for Triết must:
1. Be documented (an ADR in `docs/decisions/`).
2. Cite specific prior art (Unison, Mojo, Pony, Swift, Genode, …).
3. List the alternatives considered and why they were rejected.
4. Not be rushed by shipping pressure.

**Why:** the user stated it plainly (2026-05-09): "I do not need a fast implementation. We are doing something insane; we want to produce something that turns the world upside down, a language that is fast and safe — but the implementation process should be made of certain, safe decisions. Stability comes first."

**How to apply:**
- Before committing to a new architecture, write an ADR with context / decision / alternatives / consequences.
- Between "a pretty, fast feature" and "a less attractive feature that lays a solid foundation" → choose the foundation.
- Between "invent our own solution" and "adopt tested prior art" → choose prior art (unless it conflicts with Triết's balanced-ternary identity).
- Pace the timeline on a 5-10 year scale for v3.0. Never promise short timelines.
- Explain to the user when a feature needs a long time — they accept it and prioritize quality.
