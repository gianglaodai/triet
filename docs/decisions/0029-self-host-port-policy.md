# ADR 0029 — Self-host Port Policy (lockstep on language surface)

**Status:** **Locked** (v0.9.0.2, author sign-off 2026-05-29). Codifies lessons from the v0.8.x.completion.4 retrospective: self-host port lag was a recurring pattern that undermined the bootstrap byte-identical gate claim. The author confirmed two decisions: §2 mandatory lockstep (no discretion); §4 3-layer detection (smoke + count-based release-check + TODO checklist). The §4 detection implementation lands as the v0.9.0.2.c sub-task.

**Issue:** Two retroactive self-host ports occurred in v0.8:

1. **v0.8.x.review.3** (`46c8722`) — ported ownership lexer tokens `&+`/`&0`/`&-`/`&` into `compiler/parser/lexer.tri` (~23 LOC) AFTER v0.8 shipped. An audit discovered that ROADMAP §v0.8.12 claimed "Triet-in-Triet parser handles ownership tokens (read-only)" but in reality there was zero mention of `Ampersand*` in the self-host file.

2. **v0.8.x.completion.2** (`3ad4874`) — ported parser AST `ReferenceForm` into `compiler/parser/parser.tri` (~145 LOC) AFTER v0.8 shipped. Same paperwork-vs-reality gap pattern as #1.

Both were reactive cleanups — design + Rust impl shipped first, self-host port lagged, an audit caught the discrepancy, and a retroactive sub-task closed it. This pattern will likely repeat in v0.9+ unless a strict policy is locked.

**Root concern:** Triet claims a **self-hosting compiler** per [ADR-0019](0019-self-hosting-compiler-bootstrap.md) + ROADMAP §v0.7 SHIPPED status. The bootstrap byte-identical gate `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` is the proof. If the self-host parser cannot read current Triet syntax, the "self-hosting" claim is **frozen at version X**, rather than being genuinely self-hosting. ADR-0029 decides: lockstep on language surface vs. explicit freeze.

ADR-0029 locks **lockstep on language surface** (lexer / parser AST / SPEC grammar elements). Internal compiler details (IR optimizations, typecheck implementation, runtime backend) may lag. Detection mechanisms operate via existing smoke tests + a new `release-check.sh` extension.

---

## §1 — Scope axes

Self-hosting has **3 layers** with different lag tolerances:

**Layer A — Language surface (lockstep mandatory):**

- Lexer tokens (e.g., `&+`, `~+`, `?~` compound tokens).
- Parser AST nodes (e.g., `ReferenceForm` enum, `OutcomeType` variant).
- SPEC §X grammar rules (e.g., reserved keywords, statement forms).

If the self-host compiler cannot parse current `.tri` source, the byte-identical gate is meaningless. **Lockstep is strictly required.**

**Layer B — Internal compiler implementation (defer-OK):**

- Typecheck algorithm changes (e.g., E1033 logic refinement).
- IR lowerer optimizations.
- Borrow checker enforcement (E2440 NLL — internal verifier).
- Generic monomorphization order changes.

These produce the **same output for the same input**. A stable self-host internal implementation + current Rust implementation produce identical `.khi` bytes. Lag is acceptable.

**Layer C — Runtime / backend (independent timeline):**

- VM opcode additions (per ADR-0028 §1 builtin shim adds IDs 27-39).
- AOT codegen (v2.0 LLVM).
- JIT (v0.9 Cranelift).

These do not affect `.tri` $\rightarrow$ `.khi` compilation; they affect `.khi` execution. The self-host compiler does not need to know JIT/AOT details. These timelines are independent.

---

## §2 — Lockstep policy (Layer A)

**Author review required.**

**Decision:** Every ADR introducing a Layer A change (lexer / parser AST / SPEC grammar) **MUST** ship a self-host port in the same phase as the Rust implementation. No exceptions for "paperwork only" or "design lock without code".

**Practical workflow:**

1. The ADR design phase locks the language surface change.
2. The phase sub-task plan **MUST** include an explicit self-host port sub-task (e.g., `v0.X.Y.self-host` or `v0.X.Y.b` as a paired sub-task).
3. The phase release commit CANNOT bump the version until the self-host port lands.
4. Smoke tests (`lexer_self_smoke`, `parser_type_smoke`, etc.) are extended with assertions covering the new surface.

**Failure mode:** If a self-host port is impossibly difficult in the same phase (e.g., it depends on a new VM opcode scheduled for a later sub-task), the phase author MUST:

1. Open an issue / TODO with an explicit "Self-host port deferred — gap acknowledged".
2. Document the gap in the phase ROADMAP §"Deferred / Alternatives Considered".
3. Plan a reconciliation sub-task in the next phase (`v0.(X+1).y.self-host-catchup`).

This represents **explicit deferral**, not a silent gap. Audits in v0.8 caught silent gaps (v0.8.x.review.3 + v0.8.x.completion.2 are precedents — both were retroactive because the gap was unacknowledged at the v0.8 release).

**Rejected alternative:** Permanent freeze (self-host stays at v0.7 syntax). Reasons:

- Defeats the bootstrap byte-identical gate (Stage 2 cannot read newer `.tri` code $\rightarrow$ byte-identical comparison is impossible).
- The "self-hosting" claim becomes misleading marketing rather than technical reality.
- The author's "stability over speed" + "AI-first" principles and the core Triet identity all favor self-hosting symmetry.

**Rejected alternative:** Per-ADR opt-in (each feature ADR decides whether to port). Too discretionary — evidence from v0.8 demonstrated that even with explicit ADRs (ADR-0022, 0025), port lag occurred. A mandatory policy default is required.

---

## §3 — Layer B defer rules (internal compiler implementation)

**Decision:** Layer B changes may lag arbitrarily as long as:

1. **Same input $\rightarrow$ same output**: Stage 2 (built by Stage 1 with older internal logic) and the Rust implementation produce byte-identical `.khi` for the same `.tri` corpus.
2. **No new typecheck-level errors lost**: e.g., if the Rust implementation gains E2440 NLL enforcement, Stage 2 without it still produces correct output for code that already complies. Code that violates NLL fails differently (it might pass in Stage 2 but fail in Stage 1) — this is acceptable because Stage 1 is authoritative per ADR-0019.

**Detection:** The existing `bootstrap_determinism` test (`examples/*.tri` $\times$ 10 byte-identical builds) covers the same-output property. If Layer B drift breaks determinism, `bootstrap_determinism` catches it.

**Explicit deferral triggers:**

- v0.9.x.borrow NLL enforcement is Layer B. Self-host does not need NLL — Stage 2 produces correct output for compliant code. Layer B deferral is acceptable.
- v0.9.x.jit Cranelift is Layer C (runtime). Self-host does not need to adapt.

---

## §4 — Detection mechanism

**Author review required.**

**Decision:** Three-layer detection:

**Detection 1 — Smoke tests (existing, extended per-feature):**

- `lexer_self_smoke.rs::main` — covers lexer Layer A. Each new lexer token MUST add a `check_count("ops_new_feature", "<source>", N)` assertion in the same phase as the Rust implementation port.
- `parser_type_smoke.rs::main` — covers parser type-level Layer A. New `TypeExpr` variants MUST add an `assert_parse_type("<source>", "<expected>")` assertion in the same phase.
- `parser_expr_smoke.rs`, `parser_stmt_smoke.rs`, `parser_item_smoke.rs`, `parser_pattern_smoke.rs` — analogous for other parser surfaces.

Phase `release-check` verifies that smoke tests pass. If a new lexer/parser surface is added without a corresponding smoke assertion $\rightarrow$ the smoke test passes without detecting the omission, and the gap goes unnoticed.

This was the **v0.8 failure mode** — the author added lexer tokens to Rust without checking self-host. Smoke tests passed because they did not assert coverage of the new tokens.

**Detection 2 — `release-check.sh` extension (new):**

Add a new check in `scripts/release-check.sh` Gate D Self-consistency:

```
Gate D — Self-consistency (drift checks)
  ...
  self-host parser symmetry (counts) … ✓
```

Concrete implementation: count `Token` enum variants in `crates/triet-lexer/src/token.rs` vs. `compiler/parser/lexer.tri`. Any diff serves as a drift signal. Apply the same comparison for `TypeExpr`/`Expr`/`Stmt`/`Pattern` enums between `triet-syntax` and `parser.tri`.

**Limitation:** Count-based detection misses ordering and payload structure changes. It catches "added 3 variants in Rust, 0 in self-host" but not "renamed variant". Future v0.10+ work may add structural diffing (parsing both ASTs and comparing) — deferred for now.

**Detection 3 — Mandatory phase planning checklist:**

Every phase opening commit in `TODO.md` MUST include a "Self-host port checklist" if Layer A is touched:

```markdown
### v0.X — Phase title

**Self-host port checklist** (per ADR-0029):

- [ ] Lexer changes in `crates/triet-lexer/src/token.rs` → port `compiler/parser/lexer.tri`
- [ ] Parser AST changes in `crates/triet-syntax/src/*` → port `compiler/parser/parser.tri`
- [ ] SPEC grammar additions in §X → reflect in `compiler/parser/parser.tri`
```

This drives authors and AI agents to actively evaluate self-host scope during each phase, preventing silent gaps.

---

## §5 — ADR template addition

**Decision:** Update the ADR template (per [docs/decisions/README.md](README.md) "How to write a new ADR") to include a "Self-host port plan" section:

```markdown
## Self-host port plan (per ADR-0029)

- **Layer A surface changes:** [yes/no]. If yes, ports:
  - `compiler/parser/lexer.tri`: ... (specifics)
  - `compiler/parser/parser.tri`: ... (specifics)
- **Layer B internal changes:** [yes/no]. If yes, defer-OK.
- **Layer C runtime changes:** [yes/no]. Independent.
- **Same-phase port required:** [yes/no]. If no, defer to phase v0.Y with explicit reconciliation sub-task.
```

This requires every ADR author to consider self-host scope upfront. Future ADRs explicitly document policy compliance.

**Retroactive:** ADR-0028 (v0.9.0.1, recently locked) did not have this field — it is added in subsequent ADRs starting with ADR-0030. ADR-0028's implicit fallback: Layer A (lexer/parser change — see §4 of ADR-0028 for type signatures and builtin call surface). Self-host port is required when the ADR-0028 implementation ships in v0.9.x.atomic.

---

## §6 — Stage 2/3 byte-identical gate lift

**Decision:** Per ROADMAP §v0.9 Gate Functional, `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` is lifted from `#[ignore]` to CI-required when the JIT lifts VM performance (v0.9 ADR-0030 Cranelift).

ADR-0029 confirms this timeline: as long as the lockstep policy is honored, Stage 2 (built by the self-host parser) CAN read current Triet code $\rightarrow$ can produce identical `.khi` $\rightarrow$ the byte-identical gate is provable.

Without ADR-0029 lockstep, lifting this gate is impossible (Stage 2 would error on new syntax).

**Cross-reference:** [ADR-0019 §7 Addendum](0019-self-hosting-compiler-bootstrap.md#addendum--v0713-perf-gate--10-ph%C3%BAt-deferral) — perf gate deferral chained to JIT performance lift. ADR-0029 is tied to the same milestone.

---

## §7 — Backout protocol (when same-phase port impossible)

**Decision:** In rare cases where a Layer A port cannot ship in the same phase (e.g., it depends on a new VM opcode planned later in the phase):

1. **Explicit deferral note in ADR** under "Self-host port plan":
   > "Self-host port deferred to v0.X.Y because <reason>. Reconciliation sub-task: v0.X.Y.self-host-catchup."
2. **TODO.md sub-task** for the catchup port, opened in the current phase and closed in the next.
3. **ROADMAP §"Alternatives Considered / Deferred"** lists the deferred port explicitly.
4. **release-check.sh** does NOT block (issues a warning only) — the author has acknowledged the gap.

**Counter-example pattern** (what v0.8 did WRONG):

- v0.8 shipped ownership lexer tokens + parser AST in the Rust implementation.
- No self-host port sub-task existed in `TODO.md`.
- No "Self-host port deferred" note appeared in ADR-0022/0025/0026.
- Audits retroactively caught the omission via paperwork-vs-reality checks.
- v0.8.x.review.3 + v0.8.x.completion.2 required retroactive ports.

ADR-0029 eliminates this pattern going forward.

---

## Consequences

**Positive Outcomes:**

- The bootstrap byte-identical gate becomes provable (Stage 2 can read current Triet syntax).
- Retrospective audits eliminate paperwork-vs-reality gaps (closing the recurring v0.8 pattern).
- The ADR template + TODO checklist drive proactive port planning.
- Lifting Stage 2 ≡ Stage 3 from `#[ignore]` becomes feasible (v0.9 milestone).
- The self-hosting claim becomes technical reality rather than marketing.

**Constraints & Costs:**

- Every Layer A change adds ~30-200 LOC for the self-host port + smoke test extensions. This is manageable — evidence from v0.8: lexer 23 LOC + parser 145 LOC = 168 LOC for 5-form ownership = 1 sub-task per phase.
- Phase cadence slows by ~10-20% due to port work. This is acceptable given the "stability over speed" principle.

**Risks & Verification Needs:**

- Future Layer A changes that span multiple sub-tasks (e.g., generic monomorphization syntax) may genuinely face same-phase delivery challenges. The backout protocol in §7 addresses this but introduces process overhead.
- Count-based detection (§4) has limitations (it misses structural drift). A future structural-diff tool will be required (v0.10+).

---

## Rejected Alternatives

- **Permanent freeze** — self-host stuck at v0.7 syntax. Rejected per §2 reasoning (undermines the self-hosting claim).
- **Auto-generated self-host from Rust impl** — proposed a generator tool for automatic synchronization. Tempting, but: (a) the generator itself introduces another drifting layer; (b) the self-host compiler must remain human-readable Triet source code, whereas generated code is rarely readable; (c) the byte-identical bootstrap requires determinism, and generators rarely guarantee version-over-version determinism. Rejected — write self-host code by hand.
- **Two self-host versions** (frozen + bleeding edge) — maintaining both doubles complexity. Rejected.
- **Per-ADR discretion** (each ADR decides independently) — v0.8 demonstrated that discretion leads to silent gaps. Rejected.
- **Defer detection to v1.0 freeze** — pre-v1.0 was considered "free for breaking changes", making port lag tolerable. Rejected — the bootstrap guarantee applies across ALL versions, not just v1.0+.

---

## Prior Art

| Source | What We Adopted | What We Changed |
|---|---|---|
| Rust `rustc-bootstrap` | 3-stage chain (Stage 1 OCaml $\rightarrow$ Rust $\rightarrow$ modern Rust) | Triet keeps `compiler/lexer.tri` symmetric with `crates/triet-lexer/` via explicit policy. Rust relies on a "self-host frozen at the version pinned in a submodule" |
| Go `cmd/compile` | Self-hosted since 1.5 | Go bootstrap policy = lockstep (gofmt/parser/etc. are all hand-ported per Go version). ADR-0029 matches this model |
| OCaml | Self-hosted since 1985+ | Version-to-version drift across compiler versions is managed via specialized build tooling |
| Pascal compilers (1970s) | First self-hosting compiler (Wirth) | Original lockstep pattern |

**Novel Contributions in Triet:**

- **Explicit 3-layer scope axes (A/B/C)** categorization. Most language projects do not formalize this division.
- **Phase TODO checklist** integration. ADR-0029 §4-5 binds policy directly into the project's commit cadence.
- **Count-based smoke detection** as a pragmatic v0.9 starting point, with full structural diffing deferred.

---

## References

- [ADR-0019](0019-self-hosting-compiler-bootstrap.md) — Self-hosting compiler bootstrap (parent ADR; ADR-0029 enforces ongoing consistency).
- [ADR-0019 Addendum](0019-self-hosting-compiler-bootstrap.md#addendum--v0713-perf-gate--10-ph%C3%BAt-deferral) — Performance gate deferral and JIT integration timeline.
- [ADR-0009](0009-version-gate-policy.md) — Version gate policy (ADR-0029 §4 Detection extends `release-check.sh` per ADR-0009 Addendum §A).
- [ADR-0009 Addendum](0009-version-gate-policy.md#addendum--v08xcadence-fix-2026-05-28-enforcement-automation) — Enforcement automation (`release-check.sh` tooling extended by ADR-0029).
- [v0.8.x.completion.4 ROADMAP entry](../../ROADMAP.md) — Trigger context: "Self-host port lag is real and recurring".
- [v0.8.x.review.3 commit `46c8722`](../../) — Retroactive lexer port precedent.
- [v0.8.x.completion.2 commit `3ad4874`](../../) — Retroactive parser AST port precedent.
