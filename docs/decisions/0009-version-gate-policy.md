# ADR 0009 — Version Gate Policy: v0.4 Entry Requirements

**Status:** Decided. Applies to all version bumps v0.x → v0.(x+1) starting from v0.3 → v0.4. Locked as a core project principle.

**Issue:** When closing the v0.3 phase (commit `28e7da0`, ROADMAP § v0.3 "shipped"), several gates remained in a partial state:

- Differential VM ≡ interpreter: **3/11** (8 ignored with `#[ignore]`)
- VM bench: **1.26×** interpreter (gate target was 3×)
- Cargo workspace version: still `0.1.0` (out of sync with SPEC v0.3)
- Accumulated clippy warnings: 109+ in `triet-ir/lib` despite `CLAUDE.md` requiring *"fix every new warning"*
- TODO comments `TODO(v0.3.4)`, `TODO(v0.3.5)` remained in code despite v0.3 closure

This represents drift between **"phase closed"** (per ROADMAP) and **"phase truly clean"**. The `stability over speed` principle (VISION § 6) demands the opposite: phase N cannot be closed if gates are not 100% met.

This ADR locks an **uncompromising gate policy** for all future version bumps. Not to punish v0.3 retroactively — but to clearly define what "closing a phase" means, preventing repeat occurrences of this pattern when transitioning v0.4 → v0.5.

## Decision

A phase v0.N **can only close** (and open v0.(N+1)) when **all** of the following conditions are met **simultaneously**:

### Gate A — Functional completeness

| Criterion | Measured by |
|---|---|
| All deliverables listed in ROADMAP § v0.N status = ✅ | Manual cross-check |
| Every numerical `gate target N×` in ROADMAP is met or exceeded | Reproducible benchmark or test |
| No new `#[ignore]` or `#[allow(...)]` attributes added in this phase | `grep -r "#\[ignore\]\|#\[allow" crates/` |
| No remaining `TODO(vX.Y)` with version ≤ N in source code | `grep -rn "TODO(v" crates/` |

### Gate B — Code hygiene

| Criterion | Measured by |
|---|---|
| `cargo test --workspace` passing, 0 ignored tests without explicit documented reason | CI |
| `cargo clippy --workspace --all-targets -- -D warnings` clean | CI |
| `cargo fmt --all --check` clean | CI |
| No source file > 2000 lines (signal for module splitting) | `find crates -name '*.rs' \| xargs wc -l \| awk '$1>2000'` |

### Gate C — Documentation sync

| Criterion | Measured by |
|---|---|
| `SPEC.md` header reflects exact version v0.N | Manual |
| `ROADMAP.md` § v0.N contains complete sub-task changelog with commit hashes | Manual |
| `README.md` status, test count, workspace structure match reality | Manual diff vs `cargo test --workspace 2>&1 \| grep "test result"` |
| `Cargo.toml workspace.package.version` = `0.N.0` | `grep version Cargo.toml` |
| `dao info` CLI subcommand prints correct version | `./target/release/dao info` |
| ADRs for all major architectural decisions of the phase are merged | Manual cross-check |

### Gate D — Self-consistency

| Criterion | Measured by |
|---|---|
| All `.tri` files in `examples/` execute successfully via tree-walker | `for f in examples/*.tri; do dao run "$f"; done` |
| All `.tri` files in `demos/` execute successfully via tree-walker | Idem |
| Every feature specified in SPEC is tested at least once | Manual cross-check of SPEC chapters |

## Application to v0.4 Entry

Before opening **any** sub-task `v0.4.x`, the following conditions **must** be met:

1. **Differential VM ≡ interpreter: 11/11 byte-identical** in `crates/triet-cli/tests/differential_tests.rs`. Currently 3/11 — 8 `#[ignore]` attributes must be resolved (not by deleting tests, but by completing the lowerer + VM).
2. **VM bench gate**: ROADMAP § v0.3 sets 3× — if still not met after cleanup, **do not bypass**. Two valid options:
   - Complete optimization until 3× is achieved, **OR**
   - Write ADR-0010 (revise) lowering the gate to the measured number, explicitly documenting rationale (VM is a development tier per VISION § 4.3, not a production runtime).
3. **Cargo version** bumped to `0.3.0` in sync with SPEC v0.3.
4. **README** accurately reflects v0.3 status.
5. **Clippy clean** with `-D warnings` workspace-wide.
6. **0 TODO(v0.3.x)** in source code.

If an item cannot be achieved within a reasonable timeframe, a dedicated ADR must document the deferral decision (as in the ADR-0010 example above), rather than silently skipping.

### Specific mapping for v0.3.x.cleanup phase

| Sub-task | Gate item |
|---|---|
| v0.3.x.cleanup.1 (this ADR-0009) | Lock policy |
| v0.3.x.cleanup.2 (Cargo version bump) | Gate C |
| v0.3.x.cleanup.3 (README sync) | Gate C |
| v0.3.x.cleanup.4 (Clippy fix) | Gate B |
| v0.3.x.cleanup.5–8 (Lowerer: enum, while, iterator, Long) | Gate A (resolve 8 `#[ignore]`) |
| v0.3.x.cleanup.9 (Verify) | All gates pass simultaneously |

## Consequences

- **Slower pace**: Truly closing v0.3 = 6–12 months (as estimated in ROADMAP) rather than "shipping v0.3 haphazardly with 3/11 differential and fixing later". However, this is precisely the commitment of VISION § 6.
- **v0.4 ABI designed on stabilized IR**: 11/11 differential pass = proof that IR + lowerer + VM are consistent across all v0.2 features. ABI metadata (v0.4) will encode the IR shape; if gaps remain in IR/lowerer, the ABI will encode those gaps → forcing an ABI redesign later.
- **No perpetual accumulation of "v0.3.5", "v0.3.6"**: gates ensure phases actually close. The `v0.3.x.cleanup` sub-task is the sole exception — valid because it *retroactively* satisfies the v0.3 gate before opening v0.4, rather than adding new features.
- **AI-as-collaborator**: this gate policy is easy to verify via grep + cargo. An AI assistant can self-verify whether a "phase is closed" without ambiguous subjective judgement.

## Alternatives Considered

- **Do not require** 100% code coverage (only feature coverage).
- **Do not require** zero clippy lint *suggestions* (only zero `warn`-level).
- **Do not require** binary backward compatibility (prior to v1.0).
- **Do not** apply retroactively to v0.1, v0.2 — only from v0.3 → v0.4 onward.

## Prior Art

- **Rust release process** — feature freeze + beta + stable, each with explicit gates.
- **TC39 stage process** (JavaScript) — stage 4 requires 2 implementations + spec tests passing.
- **Linux kernel merge windows** — Linus enforces the rule: regression tests must pass before the merge window closes.

## References

- [VISION § 6 — Stability over speed](../../VISION.md)
- [ROADMAP § v0.3 — Sub-task changelog](../../ROADMAP.md)
- [ADR-0007 — IR design](0007-ir-design.md)
- [ADR-0008 — `.triv` binary format](0008-triv-binary-format.md)

---

## Addendum — v0.8.x.cadence-fix (2026-05-28): Enforcement automation

**Trigger:** Audit revealed v0.8 release commit `78f2402` shipped with Gate B violated (3 clippy errors in `resolver.rs` + 21 unformatted files). Post-release v0.8.x.review (6 sub-tasks) + v0.8.x.docs-reorg (8 sub-tasks) = **14 cleanup commits** to close gates retroactively.

Author confirmed on 2026-05-28: "unintentional, did not notice cadence slip" — root cause = **policy existed, automation did not**. v0.3–v0.7 honored gates by manual verification; v0.8 skipped pre-release audit window + bundled v0.8.8–13 into the release commit simultaneously, leaving no moment for agents to flag "Gate B not passing".

ADR-0009 §B originally referenced "CI" as enforcement mechanism (lines 34–36), but repo lacked CI configuration. This section codifies enforcement tools shipped in v0.8.x.cadence-fix phase.

### A — `scripts/release-check.sh` is the single source of truth

Before tagging any vX.Y release, `scripts/release-check.sh` **must** be executed. This script verifies all 4 gates + drift checks with a single command. Exit 0 = safe to tag; exit 1 = refuse to release.

Coverage:
- **Gate B Hygiene** (critical, blocking): `cargo test --workspace` + `cargo clippy --workspace --all-targets -- -D warnings` + `cargo fmt --all --check`.
- **Gate C Docs** (critical, blocking): Cargo.toml workspace.package.version sync with SPEC.md header.
- **Gate D Self-consistency** (warnings): no stray `TODO(vX.Y)` markers in `crates/*/src/`, TODO.md no unchecked items, archive table populated.
- **ADR status sanity**: no ADRs in "Draft" status referenced normatively from SPEC.

### B — Git hooks as commit-time guards

`.githooks/pre-commit` + `.githooks/pre-push` installed via `bash scripts/install-hooks.sh` (setting `core.hooksPath = .githooks`). Each clone must install once.

- **pre-commit** (~0.5s): `cargo fmt --all --check`. Blocks dirty formatting at commit-time — v0.8.x.review.1's 21 unformatted files will never ship again.
- **pre-push** (~1 min): full ADR-0009 Gate B (fmt + clippy + test). Last guard before dirty state reaches remote.

Bypass mechanisms (`--no-verify`) are intentionally available for WIP/private-fork cases, but **bypass on main = cadence violation** — an explicitly documented violation type, not a gray area.

### C — Pre-version audit window is MANDATORY

Cadence policy update: between the sub-task tail of phase v0.N and the version bump commit, a v0.N.x.review window **must** open (mirroring v0.5.x.review / v0.6.x.review / v0.8.x.review precedent). Audit window protocol:

1. AI agent (or author) runs `scripts/release-check.sh` before proposing version bump commit.
2. If script fails → open sub-tasks fixing all findings in v0.N.x.review section of TODO/ROADMAP.
3. If script passes → proceed to version bump.
4. Version bump commit (release commit) **must** be standalone from sub-task work, **never bundled**.

"Ship then audit" pattern = explicit cadence violation. v0.8 release commit `78f2402` bundling v0.8.8–13 = anti-pattern, never to be repeated.

### D — Gate A `#[ignore]` rule clarification

The "No new `#[ignore]` added in this phase" rule of Gate A (line 27) needs nuance per ADR-0019 §7 + Addendum: `#[ignore = "reason"]` with explicit justification string is **valid** (e.g., perf-deferred bootstrap tests). Bare `#[ignore]` without reason = violation. Release-check.sh does not enforce this automatically — author review during pre-version audit.

### Addendum Consequences

- **v0.8 slip not repeated**: dual guard layers (commit-time + release-time) guarantee Gate B violations do not slip through again.
- **AI agents empowered with tooling**: `scripts/release-check.sh` is a single command replacing 6–7 manual checks. Future audits like Phase v0.8.x.review should be exceptions, not the norm.
- **Future contributors**: README install instructions direct them via `scripts/install-hooks.sh`. Slip protection does not rely on tribal knowledge.
- **Subsequent CI (v0.9+)**: When GitHub Actions is configured, workflow simply wraps `scripts/release-check.sh` — a single-line CI configuration.

### Addendum Non-Goals / Alternatives Considered

- **Do not** make `--no-verify` impossible — WIP commits + private fork cases remain valid.
- **Do not** enforce ADR-0009 retroactively for commits prior to Addendum date.
- **Do not** add new gates beyond ADR-0009 §A–D — only codify enforcement of existing gates.
