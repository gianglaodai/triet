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

## v0.5 — CAS Packaging (in progress)

Per [ROADMAP.md § v0.5](ROADMAP.md).

- [x] v0.5.1 — ADR-0014 hash scheme refinement (3-cấp hash tree) `f876006`
- [x] v0.5.2 — ADR-0015 package store layout (CAS filesystem) `f7b49c8`
- [x] v0.5.3 — 3-cấp hash tree (term + module + pkg) in `triet-pack` + abi_version 1 → 2 `b6d170c`
- [x] v0.5.4 — Package store filesystem (`~/.triet/store/`) + atomic install + GC `2425e25`
- [x] v0.5.5 — Hash-based dep resolver + `triet.lock` format `2c43e69`
- [x] v0.5.6 — Shared loading demo (VISION §3.1 gate at iface level; body-level defer) `6291bc1`
- [x] v0.5.7 — `triet store {import,list,gc}` CLI (v=1 lossy migration deferred) `8b4ce12`
- [x] v0.5.8 — Cross-module enum variant import `from X import Variant` (Item 1 of 2) `07323a1`
  - Item 2 (lowerer emit `WitnessCall` for cross-package generics) **deferred** out of v0.5 — needs package-aware lowering, multi-week architectural milestone. Reschedules to a future phase (multi-package compile or v0.7 self-hosting).
- [ ] v0.5.9 — Verify gate (ADR-0009) + bump Cargo `0.4.0 → 0.5.0` + docs sync

---

## How to update this file

- Mark a task `[x]` and move it to **Done** when its commit lands on `main`.
- Add the commit short-hash next to completed tasks for quick git reference.
- Keep the order: **Done** → **In progress** → **Pending**.
- When a whole phase (e.g. v0.2.x) ships, archive its summary into
  `ROADMAP.md` (under the changelog section) and delete the detailed
  checkboxes from this file.
