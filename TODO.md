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

## v0.5.x.review — Pre-v0.6 audit fixes ✅ SHIPPED

Archived to [ROADMAP.md § v0.5.x.review](ROADMAP.md).

4 sub-tasks done. Audit của AI trước v0.6 → 1 binary leak + 3 testing gap
được bít. 918 → 924 tests, ADR-0015 Addendum landed.

Final v0.5.x.review commit: this commit.

---

## v0.6 — Capability System (in progress)

Per [ROADMAP.md § v0.6](ROADMAP.md).

- [x] v0.6.1 — ADR-0016 — Capability type system (namespace + manifest, Trit-level grant/deny/ambient + Trilean::Unknown defer, `triet::capability::E22XX`) `cd65127`
- [x] v0.6.2 — ADR-0017 — Trilean policy hook protocol (`triet.policy` hybrid rules + TTY prompt, per-session cache, E2205 sub-variants) `0e6e94a`
- [x] v0.6.3 — ADR-0018 — Loader semantics (`triet.package` grammar, eager link-time check, TTY provenance prompt, E2208 sub-variants, `CapabilityClaim` struct rename) `6742948`
- [x] v0.6.4 — `CapabilityClaim` struct + 4-variant `CapabilityLevel` enum (ADR-0018 §6) + caps section wire format extend (ADR-0016 §4: cap_path + level u8 + reserved u8). 924 → 930 tests, clippy -D warnings clean, abi_version stays 2. `22151a4`
- [x] v0.6.5 — `triet.package` source manifest parser (ADR-0018 §1). Hand-rolled strict whitelist per ADR-0017 Addendum §A. `PackageManifest` + `PackageManifestError` (`E2208 Malformed`, `E2208 UnsupportedFormatVersion`, `E2206 InvalidCapabilityRoot`). 930 → 955 tests, clippy -D warnings clean. ASCII identifier subset at v0.6.5; XID Unicode deferred. `cb8aa7b`
- [x] v0.6.6 — `triet.policy` parser + shared `strict_parser` (ADR-0017 §3 + Addendum §A). Extracted whitelist tokenizer (`for_each_directive_line` + `LineViolation`) — refactored `PackageManifest::parse` to share, then built `PolicyRules` on top. Numeric token style per ADR-0018 §1 audience split. `PolicyError` 4 load-time variants (E2205). Lookup precedence per ADR-0017 §4 (exact origin > `*`). `default prompt` rejected. 955 → 996 tests, clippy -D warnings clean. Runtime sub-variants (NonTTYDefer/PromptCrash) defer v0.6.9–10. `2a3a6c6`
- [x] v0.6.7 — Cross-root capability check at type-check stage (ADR-0016 §5 rules 1+2). `triet-typecheck::check_capabilities(&ResolvedProgram, &PackageManifest)` + `CapabilityError` 2 variants (E2200 MissingCapabilityClaim, E2201 SelfContradictoryCapability). Scope reduced from original wording: E2206 already parse-stage (v0.6.5). Span placeholder `0..0`; refine v0.6.8. `triet-pack` added as dep of `triet-typecheck` (no cycle). 996 → 1012 tests, clippy -D warnings clean. `b41d47e`
- [x] v0.6.8 — Link-time capability check (ADR-0018 §2 Step 6a, ADR-0016 §5+§7). `triet-pack::check_link_capabilities(root, available) -> CapabilityLinkReport`. `CapabilityLinkError` 3 variants (E2200 missing/E2202 unresolved-path/E2203 refused with Deny|Ambient sub-level). `DeferredCap` collection for runtime resolver. Per-path dedupe + sorted requester aggregation; deterministic ordering. E2208 sub-variants defer with explicit trigger. 1012 → 1027 tests, clippy -D warnings clean. `24c34c3`
- [x] v0.6.9 — Capability resolver + per-session cache (ADR-0017 §4, ADR-0018 §2 Step 6b). `CapabilityResolver::new(rules: PolicyRules)` owns snapshot. `resolve(req) -> CachedDecision` with Trit outcome + DecisionSource (Cache/ConfigRule/AbstainFromRule/Default/InteractivePrompt/Error). `ResolverError::NonTTYDefer` fires when rule says prompt + tty_available=false (always at v0.6.9). PromptCrash variant placeholder for v0.6.10. ADR-0017 §5 monotonicity invariant verified via Cache replay tests. `triet-core` now direct dep of `triet-pack`. 1027 → 1044 tests, clippy -D warnings clean. `6151399`
- [ ] v0.6.10 — TTY prompt UX (`/dev/tty` I/O + provenance display, ADR-0018 §4)
- [ ] v0.6.11 — Demo `usr.app` vs `dev.*` + integration tests

---

## How to update this file

- Mark a task `[x]` and move it to **Done** when its commit lands on `main`.
- Add the commit short-hash next to completed tasks for quick git reference.
- Keep the order: **Done** → **In progress** → **Pending**.
- When a whole phase (e.g. v0.2.x) ships, archive its summary into
  `ROADMAP.md` (under the changelog section) and delete the detailed
  checkboxes from this file.
