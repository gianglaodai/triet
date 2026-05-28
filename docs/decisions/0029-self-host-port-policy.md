# ADR 0029 — Self-host Port Policy (lockstep on language surface)

**Trạng thái:** **Locked** (v0.9.0.2, author sign-off 2026-05-29). Codifies lessons từ v0.8.x.completion.4 retrospective: self-host port lag là recurring pattern, broken bootstrap byte-identical gate claim. Author confirmed 2 decisions: §2 mandatory lockstep (no discretion); §4 3-layer detection (smoke + count-based release-check + TODO checklist). §4 detection implementation lands as v0.9.0.2.c sub-task.

**Issue:** Two retroactive self-host ports trong v0.8:

1. **v0.8.x.review.3** (`46c8722`) — ported ownership lexer tokens `&+`/`&0`/`&-`/`&` vào `compiler/parser/lexer.tri` (~23 LOC) AFTER v0.8 ship. Audit phát hiện ROADMAP §v0.8.12 claimed "Triết-in-Triết parser handles ownership tokens (read-only)" but reality was 0 mention `Ampersand*` trong self-host file.

2. **v0.8.x.completion.2** (`3ad4874`) — ported parser AST `ReferenceForm` vào `compiler/parser/parser.tri` (~145 LOC) AFTER v0.8 ship. Same paperwork-vs-reality gap pattern as #1.

Both reactive cleanups — design + Rust impl shipped first, self-host port lagged, audit caught discrepancy, retroactive sub-task closed. Pattern likely repeats trong v0.9+ unless policy locked.

**Root concern:** Triết claims **self-hosting compiler** per [ADR-0019](0019-self-hosting-compiler-bootstrap.md) + ROADMAP §v0.7 SHIPPED status. Bootstrap byte-identical gate `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` is the proof. If self-host parser can't read current Triết syntax, the "self-hosting" claim is **frozen at version X**, not actually self-hosting. ADR-0029 chooses: lockstep on language surface vs explicit freeze.

ADR-0029 locks **lockstep on language surface** (lexer / parser AST / SPEC grammar elements). Internal compiler details (IR opt, typecheck impl, runtime backend) may lag. Detection mechanism via existing smoke tests + new `release-check.sh` extension.

---

## §1 — Scope axes

Self-host has **3 layers** with different lag tolerance:

**Layer A — Language surface (lockstep mandatory):**

- Lexer tokens (e.g., `&+`, `~+`, `?~` compound tokens).
- Parser AST nodes (e.g., `ReferenceForm` enum, `OutcomeType` variant).
- SPEC §X grammar rules (e.g., reserved keywords, statement forms).

If self-host can't parse current `.tri` source, byte-identical gate is meaningless. **Lockstep required.**

**Layer B — Internal compiler implementation (defer-OK):**

- Typecheck algorithm changes (e.g., E1033 logic refinement).
- IR lowerer optimizations.
- Borrow checker enforcement (E2440 NLL — internal verifier).
- Generic monomorphization order changes.

These produce **same output for same input**. Self-host stable internal impl + current Rust impl = same .khi bytes. Lag-OK.

**Layer C — Runtime / backend (independent timeline):**

- VM opcode additions (per ADR-0028 §1 builtin shim adds IDs 27-39).
- AOT codegen (v2.0 LLVM).
- JIT (v0.9 Cranelift).

These don't affect `.tri` → `.khi` compilation; affect `.khi` execution. Self-host doesn't need to know JIT/AOT details. Independent.

---

## §2 — Lockstep policy (Layer A)

**Author review required.**

**Decision:** Every ADR introducing Layer A change (lexer / parser AST / SPEC grammar) **MUST** ship self-host port in the same phase as the Rust impl. No exceptions for "paperwork only" or "design lock without code".

**Practical workflow:**

1. ADR design phase locks language surface change.
2. Phase sub-task plan **MUST** include explicit self-host port sub-task (e.g., `v0.X.Y.self-host` or `v0.X.Y.b` as paired sub-task).
3. Phase release commit CANNOT bump version until self-host port lands.
4. Smoke tests (`lexer_self_smoke`, `parser_type_smoke`, etc.) extend with assertion covering new surface.

**Failure mode:** If self-host port impossibly hard same phase (e.g., needs new VM opcode that depends on later sub-task), phase author MUST:

1. Open issue / TODO with explicit "Self-host port deferred — gap acknowledged".
2. Document gap trong phase ROADMAP §"Không làm".
3. Plan reconciliation sub-task in next phase (`v0.(X+1).y.self-host-catchup`).

This is **explicit deferral**, not silent gap. v0.8 audits caught silent gaps (v0.8.x.review.3 + v0.8.x.completion.2 are precedent — both retroactive because gap was unacknowledged at v0.8 release).

**Rejected alternative:** Permanent freeze (self-host stays at v0.7 syntax). Reasons:

- Defeats bootstrap byte-identical gate (Stage 2 can't read newer .tri code → byte-identical compare impossible).
- "Self-hosting" claim becomes misleading marketing rather than technical reality.
- Author's "stability over speed" + "AI-first" + Triết identity all favor self-hosting symmetry.

**Rejected alternative:** Per-ADR opt-in (each feature ADR decides whether to port). Too discretionary — pattern v0.8 showed even with explicit ADRs (ADR-0022, 0025) port lag happened. Need policy default.

---

## §3 — Layer B defer rules (internal compiler implementation)

**Decision:** Layer B changes may lag arbitrarily as long as:

1. **Same input → same output**: Stage 2 (built by Stage 1 with older internal logic) and Rust impl produce byte-identical `.khi` for same `.tri` corpus.
2. **No new typecheck-level errors lost**: e.g., if Rust impl gains E2440 NLL enforcement, Stage 2 without it produces correct output for code that already complies. Code that violates NLL fails differently (might pass in Stage 2 but fail in Stage 1) — that's OK because Stage 1 is authoritative per ADR-0019.

**Detection:** Existing `bootstrap_determinism` test (`examples/*.tri` × 10 builds byte-identical) covers same-output property. If Layer B drift breaks determinism, bootstrap_determinism catches it.

**Defer triggers — explicit:**

- v0.9.x.borrow NLL enforcement is Layer B. Self-host doesn't need NLL — Stage 2 produces correct output for compliant code. Layer B defer = OK.
- v0.9.x.jit Cranelift is Layer C (runtime). Self-host doesn't care.

---

## §4 — Detection mechanism

**Author review required.**

**Decision:** Three-layer detection:

**Detection 1 — Smoke tests (existing, extend per-feature):**

- `lexer_self_smoke.rs::main` — covers lexer Layer A. Each new lexer token MUST add a `check_count("ops_new_feature", "<source>", N)` assertion in same phase as Rust impl ports.
- `parser_type_smoke.rs::main` — covers parser type-level Layer A. New TypeExpr variants MUST add `assert_parse_type("<source>", "<expected>")` assertion same phase.
- `parser_expr_smoke.rs`, `parser_stmt_smoke.rs`, `parser_item_smoke.rs`, `parser_pattern_smoke.rs` — analogous for other parser surfaces.

Phase release-check verifies smoke tests pass. If new lexer/parser surface added without smoke assertion → smoke test passes (no detection!) — gap goes undetected.

This is the **v0.8 failure mode** — author added lexer tokens to Rust without checking self-host. Smoke test passed because it didn't test the new tokens.

**Detection 2 — `release-check.sh` extension (new):**

Add new check in `scripts/release-check.sh` Gate D Self-consistency:

```
Gate D — Self-consistency (drift checks)
  ...
  self-host parser symmetry (counts) … ✓
```

Concrete implementation: count `Token` enum variants in `crates/triet-lexer/src/token.rs` vs `compiler/parser/lexer.tri`. Diff = drift signal. Same for `TypeExpr`/`Expr`/`Stmt`/`Pattern` enums between `triet-syntax` and `parser.tri`.

**Limitation:** Count-based detection misses ordering / payload structure changes. Catches "added 3 variants in Rust, 0 in self-host" but not "renamed variant". Future v0.10+ could add structural diff (parse both ASTs, compare) — defer.

**Detection 3 — Mandatory phase planning checklist:**

Every phase opening commit in TODO.md MUST include "Self-host port checklist" if Layer A is touched:

```markdown
### v0.X — Phase title

**Self-host port checklist** (per ADR-0029):

- [ ] Lexer changes in `crates/triet-lexer/src/token.rs` → port `compiler/parser/lexer.tri`
- [ ] Parser AST changes in `crates/triet-syntax/src/*` → port `compiler/parser/parser.tri`
- [ ] SPEC grammar additions in §X → reflect in `compiler/parser/parser.tri`
```

Drives author / AI agent to actively consider self-host scope per phase. Prevents silent gaps.

---

## §5 — ADR template addition

**Decision:** Update ADR template (per [docs/decisions/README.md](README.md) "How to write a new ADR") to include "Self-host port plan" field:

```markdown
## Self-host port plan (per ADR-0029)

- **Layer A surface changes:** [yes/no]. If yes, ports:
  - `compiler/parser/lexer.tri`: ... (specifics)
  - `compiler/parser/parser.tri`: ... (specifics)
- **Layer B internal changes:** [yes/no]. If yes, defer-OK.
- **Layer C runtime changes:** [yes/no]. Independent.
- **Same-phase port required:** [yes/no]. If no, defer to phase v0.Y with explicit reconciliation sub-task.
```

Forces ADR author to consider self-host scope upfront. Future ADRs explicitly document policy compliance.

**Retroactive:** ADR-0028 (v0.9.0.1, just locked) doesn't have this field — added in subsequent ADRs starting ADR-0030. ADR-0028 implicit fall-back: Layer A (lexer/parser change — see §4 of ADR-0028 type signatures + builtin call surface). Self-host port needed when ADR-0028 implementation ships in v0.9.x.atomic.

---

## §6 — Stage 2/3 byte-identical gate lift

**Decision:** Per ROADMAP §v0.9 Gate Functional, `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` lifts from `#[ignore]` to CI-required when JIT lifts VM perf (v0.9 ADR-0030 Cranelift).

ADR-0029 confirms this timeline: as long as lockstep policy is honored, Stage 2 (built by self-host parser) CAN read current Triết code → can produce same .khi → byte-identical gate provable.

Without ADR-0029 lockstep, lift impossible (Stage 2 would error on new syntax).

**Cross-reference:** [ADR-0019 §7 Addendum](0019-self-hosting-compiler-bootstrap.md#addendum--v0713-perf-gate--10-ph%C3%BAt-deferral) — perf gate deferral chained to JIT lift. ADR-0029 chained to same milestone.

---

## §7 — Backout protocol (when same-phase port impossible)

**Decision:** Rare cases where Layer A port can't ship same phase (e.g., depends on new VM opcode planned later in phase):

1. **Explicit deferral note in ADR** under "Self-host port plan":
   > "Self-host port deferred to v0.X.Y because <reason>. Reconciliation sub-task: v0.X.Y.self-host-catchup."
2. **TODO.md sub-task** for the catchup port, opened in current phase, closed in next.
3. **ROADMAP §"Không làm"** lists the deferred port explicitly.
4. **release-check.sh** does NOT block (warning only) — author has acknowledged gap.

**Counter-example pattern** (what v0.8 did WRONG):

- v0.8 shipped ownership lexer tokens + parser AST in Rust impl.
- No self-host port sub-task in TODO.
- No "Self-host port deferred" note in ADR-0022/0025/0026.
- Audit retroactively caught via paperwork-vs-reality check.
- v0.8.x.review.3 + v0.8.x.completion.2 retroactive ports.

ADR-0029 makes this pattern impossible going forward.

---

## Hệ quả

**Possible (positive):**

- Bootstrap byte-identical gate becomes provable (Stage 2 can read current Triết).
- Audit retrospective stops finding paperwork-vs-reality gaps (recurring v0.8 pattern closed).
- ADR template + TODO checklist drive proactive port planning.
- Stage 2 ≡ Stage 3 lift from `#[ignore]` becomes feasible (v0.9 milestone).
- Self-hosting claim becomes technical reality, not marketing.

**Constrained (cost):**

- Every Layer A change adds ~30-200 LOC self-host port + smoke test extension. Manageable — v0.8 evidence: lexer 23 LOC + parser 145 LOC = 168 LOC for 5-form ownership = 1 sub-task each phase.
- Phase pace slows ~10-20% due to port work. Acceptable given "stability over speed" principle.

**Costly (need verify):**

- Future Layer A changes that span multiple sub-tasks (e.g., generic monomorphization syntax) may genuinely need same-phase impossibility. Backout protocol §7 covers but increases process surface.
- Detection mechanism §4 count-based has gaps (misses structural drift). Future structural-diff tool needed (v0.10+).

---

## Không làm (explicitly rejected)

- **Permanent freeze** — self-host stuck at v0.7 syntax. Rejected per §2 reasoning (defeats self-hosting claim).
- **Auto-generated self-host from Rust impl** — proposes generator tool that auto-syncs. Tempting but: (a) generator is itself a layer that drifts; (b) self-host is also user-readable Triết source code, generated code rarely readable; (c) bootstrap byte-identical needs determinism; generators rarely deterministic across versions. Rejected — write self-host by hand.
- **Two self-host versions** (frozen + bleeding edge) — keeps both, complexity doubled. Rejected.
- **Per-ADR discretion** (each ADR decides) — v0.8 evidence: discretion → silent gaps. Rejected.
- **Defer detection to v1.0 freeze** — pre-v1.0 considered "breaking changes free" so port lag tolerable. Rejected — bootstrap claim is across ALL versions, not just v1.0+.

---

## Prior art

| Source | What we copy | What we change |
|---|---|---|
| Rust `rustc-bootstrap` | 3-stage chain (Stage 1 OCaml → Rust → modern Rust) | Triết: kept compiler/lexer.tri symmetric với crates/triet-lexer/ via explicit policy. Rust is "self-host frozen at version pinned in submodule" |
| Go `cmd/compile` | Self-hosted since 1.5 | Go bootstrap policy = lockstep (gofmt/parser/etc. all hand-ported per Go version). ADR-0029 matches |
| OCaml | Self-hosted since 1985+ | Per-version drift across compiler versions managed via specific build tooling |
| Pascal compilers (1970s) | First self-hosting compiler (Wirth) | Original lockstep pattern |

**What we invented:**

- **3-layer scope axes (A/B/C)** explicit categorization. Most languages don't formalize.
- **Phase TODO checklist** integration. ADR-0029 § 4-5 binds policy vào project's per-step commit cadence.
- **count-based smoke detection** as v0.9 starting point with structural diff deferred. Pragmatic for small codebase.

---

## Tham chiếu

- [ADR-0019](0019-self-hosting-compiler-bootstrap.md) — Self-hosting compiler bootstrap (the parent ADR, ADR-0029 enforces ongoing consistency).
- [ADR-0019 Addendum](0019-self-hosting-compiler-bootstrap.md#addendum--v0713-perf-gate--10-ph%C3%BAt-deferral) — perf gate deferral, JIT lift chain.
- [ADR-0009](0009-version-gate-policy.md) — Version gate policy (ADR-0029 §4 Detection extends release-check.sh per ADR-0009 Addendum §A).
- [ADR-0009 Addendum](0009-version-gate-policy.md#addendum--v08xcadence-fix-2026-05-28-enforcement-automation) — Enforcement automation (release-check.sh tooling ADR-0029 extends).
- [v0.8.x.completion.4 ROADMAP entry](../../ROADMAP.md) — Trigger context: "Self-host port lag is real and recurring".
- [v0.8.x.review.3 commit `46c8722`](../../) — Retroactive lexer port precedent.
- [v0.8.x.completion.2 commit `3ad4874`](../../) — Retroactive parser AST port precedent.
