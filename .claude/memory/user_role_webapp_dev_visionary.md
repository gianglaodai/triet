---
name: User — webapp dev, vision-driven, defers technical decisions
description: The user is a webapp developer with no system-level background. They supply the vision and the requirements, and delegate every technical decision to the assistant.
type: user
originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---
User profile (from self-disclosure 2026-05-09):
- Background: a software developer working mainly on webapps.
- No deep experience with system languages, ABIs, OS internals, or compiler internals.
- Supplies clear vision and requirements (e.g. "the language must be able to write an OS").
- **Delegates all technical and architectural decisions to the assistant**, scoped to implementing the Triết language.

**How to apply:**
- Do not ask the user "should we use witness tables or monomorphization?" — that is my decision.
- ASK about: user-facing UX, philosophy, scope, priorities, or a trade-off only the user can settle (e.g. the language name, an aesthetic syntax choice).
- DO NOT ASK about: implementation strategy, prior-art selection, ADR content, internal technique.
- Document decisions in ADRs — the user can read them but does not approve them line by line.
- When explaining, use webapp analogies (Java/Spring, npm packages, REST APIs) rather than deep system internals — the user will understand "DLL Hell" better through "a version conflict in node_modules".
