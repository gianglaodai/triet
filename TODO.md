# TODO

Sub-task tracking — short-term work in progress.

- Long-term phasing: [`ROADMAP.md`](ROADMAP.md)
- Architectural decisions: [`docs/decisions/`](docs/decisions/)
- Language semantics: [`SPEC.md`](SPEC.md), [`VISION.md`](VISION.md)

This file tracks the **current phase** only. When a phase finishes, its summary archives to `ROADMAP.md` and detailed checkboxes are deleted from here.

---

## v0.2 — v0.7 archived

All shipped phases now live in [`ROADMAP.md`](ROADMAP.md):

| Phase | ADRs | Final test count |
|---|---|---|
| v0.2.x Module system | 0005, 0006 | 700+ |
| v0.3 Bytecode VM + Stable IR | 0007, 0008 | 835 |
| v0.3.x.cleanup | 0009 | 835 |
| v0.3.x.ternary | 0010 | 838 |
| v0.4 Crate-Pack + Stable ABI | 0011, 0012, 0013 | 867 |
| v0.5 CAS Packaging | 0014, 0015 | 918 |
| v0.5.x.review | 0015 Addendum | 924 |
| v0.6 Capability System | 0016, 0017, 0018 | 1079 |
| v0.6.x.review | 0018 Addendum | 1085 |
| v0.7 Self-hosting Compiler | 0019, 0020, 0021, 0024 | 1345 |

---

## v0.8 — Ownership Foundation + Concurrency Primitives (BYOS) 🔄 in progress

**Quyết định kiến trúc:** [ADR-0026 v2](docs/decisions/0026-actor-boundary-send-rules.md) (BYOS + Send rules), [ADR-0022](docs/decisions/0022-trit-balanced-ownership.md) (Ownership model). See ROADMAP.md §v0.8 for deliverables + gates.

### Shipped (8 sub-tasks)

- [x] **v0.8.1** — SPEC §10 rewrite — S6 ownership model lock
- [x] **v0.8.2** — ROADMAP §v0.8 detail
- [x] **v0.8.3** — Object header memory layout — `triet-core::memory::ObjectHeader` (8-byte binary, 54-trit ternary, refcount atomic ops, sentinels). 11 tests.
- [x] **v0.8.4** — Lexer tokens ownership — `&+/&0/&-` compound + bare `&`. 10 tests.
- [x] **v0.8.5** — Parser + AST ownership — `ReferenceForm` enum, 5 forms, postfix precedence. 8 tests.
- [x] **v0.8.6** — Type system reference tracking — TypeExpr::Reference resolves transparently; enforcement deferred v0.9+.
- [x] **v0.8.7-byos** — ADR-0026 v2 rewrite — BYOS philosophy + Send rules universal + Atomic placeholder + capability gates + refuse scheduler keywords.
- [x] **v0.8.8** — Send derivation algorithm — Auto-derive Send for 13 type categories per ADR-0026 v2 §2.1. Type system tracks ReferenceForm. E2500 fires.
- [x] **v0.8.9** — Capability registration — `dao.package` schema extended with concurrency capabilities (`sys.raw_thread`, `sys.atomic`, `dev.ffi`, etc.) and typo detection.

*(Note: v0.8.7 Actor model lexer + parser was reverted per BYOS).*

### Remaining v0.8 sub-tasks
- [ ] **v0.8.10** — Diagnostic format compliance
  - E24XX (E2400/E2402-E2403/E2410-E2411/E2420-E2422/E2430/E2440) + E25XX (E2500/E2510/E2520) skeleton diagnostics với AI-first format per ADR-0027. ~30 tests.
- [ ] **v0.8.11** — Demo + integration suite
  - Atomic-based counter (no scheduler) + capability gate end-to-end (request `sys::raw_thread` → grant → declare-but-not-use placeholder). ~20 integration tests.
- [ ] **v0.8.12** — Self-hosting compiler smoke
  - Triết-in-Triết parser handles ownership tokens (read-only). Bootstrap chain verify Stage 1 ≡ Stage 2 byte-identical.
- [ ] **v0.8.13** — Verify gate + release
  - tests pass (~1550+), clippy clean, version bump 0.7.0 → 0.8.0, SPEC v0.8 header, README synced, `dao info` updated.
