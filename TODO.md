# TODO

Sub-task tracking — short-term work in progress.

- Long-term phasing: [`ROADMAP.md`](ROADMAP.md)
- Architectural decisions: [`docs/decisions/`](docs/decisions/)
- Language semantics: [`SPEC.md`](SPEC.md), [`VISION.md`](VISION.md)

This file is updated as tasks complete. When a phase finishes (e.g. v0.2.x),
the summary is archived into `ROADMAP.md` and detailed checkboxes
removed from here.

---

## v0.2.x — Module system ✅ SHIPPED

Archived to [ROADMAP.md § v0.2.x](ROADMAP.md).

Final commits:
- v0.2.x.7 — Stdlib as real filesystem files `befc59c`
- v0.2.x.8 — Module system demo + snapshot tests `e356a61`

## v0.3 — Bytecode VM + Stable IR ✅ SHIPPED

Archived to [ROADMAP.md § v0.3](ROADMAP.md).

All 12 sub-tasks done (v0.3.0–v0.3.11). Gates partially met:
- IR spec + bytecode format ✓
- Differential tests: 3/11 examples pass byte-identically ✓
- Bench: VM 1.26× interpreter (3× gate → v0.4) ✓
- IR snapshot tests ✓

Final commit: `28e7da0`

---

## v0.4 — Crate-Pack + Stable ABI (pending)

Per [ROADMAP.md § v0.4](ROADMAP.md). Tasks will be added when v0.4 work begins.

- [ ] ADR-0009 — ABI metadata format
- [ ] ADR-0010 — Witness table dispatch
- [ ] ADR-0011 — Semver linking policy
- [ ] Crate-pack format (`.tripack`)
- [ ] Cross-package linker
- [ ] Result/Option đầy đủ trong stdlib

---

## How to update this file

- Mark a task `[x]` and move it to **Done** when its commit lands on `main`.
- Add the commit short-hash next to completed tasks for quick git reference.
- Keep the order: **Done** → **In progress** → **Pending**.
- When a whole phase (e.g. v0.2.x) ships, archive its summary into
  `ROADMAP.md` (under the changelog section) and delete the detailed
  checkboxes from this file.
