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

All 12 sub-tasks done (v0.3.0–v0.3.11) + v0.3.x.cleanup phase.
All gates met (ADR-0009 § A/B/C/D):
- IR spec + bytecode format ✓
- Differential tests: **11/11** byte-identical ✓
- Bench: VM 1.26× interpreter (3× gate deferred to v0.4 perf pass) ✓
- IR snapshot tests ✓

Final v0.3 commit: `28e7da0`. Final cleanup commit: `251f954`.

---

## v0.3.x.cleanup ✅ SHIPPED

Archived to [ROADMAP.md § v0.3.x.cleanup](ROADMAP.md).

Gate-closing phase before v0.4. Locks [ADR-0009](docs/decisions/0009-version-gate-policy.md)
as the policy for every future version bump.

8 sub-tasks done (v0.3.x.cleanup.1–8). 835 tests, 0 ignored, clippy clean.

---

## v0.3.x.ternary ✅ SHIPPED

Archived to [ROADMAP.md § v0.3.x.ternary](ROADMAP.md).

Ternary-native IR refactor per [ADR-0010](docs/decisions/0010-ternary-native-ir.md).
Removes binary-thinking leak ở control flow: `BrTrilean` 3-way branch
replaces `BrIf` for all Trilean conditions, strict `if` Unknown→panic,
Ł3-aware `Eq`/`Ne`, `.triv` v1 → v2.

7 sub-tasks done (v0.3.x.ternary.1–8, 4+5 merged). 838 tests, 0 ignored, clippy clean, 11/11 differential.

---

## v0.4 — Crate-Pack + Stable ABI ✅ SHIPPED

Archived to [ROADMAP.md § v0.4](ROADMAP.md).

9 sub-tasks done (v0.4.1–v0.4.9). All gates met (ADR-0009):
- ADR-0011/12/13 trilogy locked.
- `triet-pack` crate landed (write/read .tripack + plan_link).
- `WitnessCall` opcode + `.triv` v3 wire format.
- `std.result` shipped; SPEC §2.5 promotes `T?` as primary.
- 867 tests, 0 ignored, clippy clean, differential 11/11.

Final v0.4 commit: this commit.

---

## v0.5 — CAS Packaging ✅ SHIPPED

Archived to [ROADMAP.md § v0.5](ROADMAP.md).

9 sub-tasks done (v0.5.1–v0.5.9). All gates met (ADR-0009 § A/B/C/D):
- ADR-0014/0015 locked. 3-cấp hash tree, package store, atomic install, GC.
- Resolver + `triet.lock`. `triet store {import,list,gc}` CLI.
- Shared loading demo (iface-level dedup proven).
- Cross-module enum variant import closed.
- 918 tests, 0 ignored, clippy clean, differential 11/11.

**Defer out of v0.5** (rescheduled):
- Lowerer emit `WitnessCall` for cross-package generics — needs package-aware lowering, multi-week milestone. Future phase (multi-package compile or v0.7 self-hosting).
- v=1 `.tripack` lossy migration — lands when v=1 packs exist in wild.
- Body-level RAM dedup (`term/<hash>/body.bin`) — chờ lowerer per-term IR body split.

Final v0.5 commit: this commit.

---

## v0.6 — Capability System (pending)

Per [ROADMAP.md § v0.6](ROADMAP.md). Tasks will be added when v0.6 work begins.

- [ ] ADR-NNNN — Capability type system (Trit-level grant/deny/ambient)
- [ ] ADR-NNNN — `Trilean::Unknown` runtime policy hook
- [ ] ADR-NNNN — Loader semantics (refuse-to-load on capability mismatch)
- [ ] Enforce `sys.*` / `dev.*` / `usr.*` top-level namespaces
- [ ] `Capability<T>` type in stdlib
- [ ] Crate-pack metadata: capability requirements (slot reserved since v0.4)
- [ ] Demo: `usr.app` cannot touch `dev.*` without capability token

---

## How to update this file

- Mark a task `[x]` and move it to **Done** when its commit lands on `main`.
- Add the commit short-hash next to completed tasks for quick git reference.
- Keep the order: **Done** → **In progress** → **Pending**.
- When a whole phase (e.g. v0.2.x) ships, archive its summary into
  `ROADMAP.md` (under the changelog section) and delete the detailed
  checkboxes from this file.
