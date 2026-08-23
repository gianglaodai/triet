# ADR-0046: Return-borrow Elision — Tier C Slice 3

**Status:** ACCEPTED — Signed by O + G on 2026-06-08
**Date:** 2026-06-08
**Author:** AI (peer D, implementer)
**Reviewers:** Mentor O (semantics, soundness) — SIGNED 2026-06-08 · Mentor G (layout, ABI, codegen) — SIGNED 2026-06-08
**Scope:** Enable `fn id(s: &0 T) -> &0 T { return s }` — callee returns a reference borrowed from a parameter, caller keeps owner frozen while reference remains live. Built on the PropagatedLoan infrastructure established in ADR-0045 §4.

---

## Summary

ADR-0045 §5 BLOCKED the `-> &0 T` return type with E1042 to seal the return-borrow slice. This slice re-opens it: typecheck permits `-> &0 T`, lowerer populates `return_borrow_map` (the sole broken link), and the PropagatedLoan engine (already present in the checker since ADR-0045 §4) re-issues the loan at the caller → owner remains frozen while the reference stays live, preventing use-after-free.

Phase-0 experimental probing confirmed: 3/4 of the infrastructure is already in place. The only remaining gap is the empty `return_borrow_map` in lowerer (`lib.rs:168`). The only code changes needed: populate this map + open the E1042 gate for `&0`.

---

## §0 — Facts

| # | Fact | Location |
|---|------|----------|
| F1 | Elision decision logic (0/1/multi ref-params) is already present. `check_lifetime_elision` (`check.rs:494`) fires E2400 when count ≠ 1. | `check.rs:494-551` |
| F2 | Dangling return `&0 <local>` is already caught by E2450. `storage-end` of a local terminates the loan → returning a ref to a local is rejected. | `borrowck/checker.rs` (E2450) |
| F3 | The PropagatedLoan engine is already present in the checker (`checker.rs:754-796`), with unit test `returned_reference_extends_source_lifetime` (`checker.rs:1384`). Engine is LIVE-in-test but DEAD-in-production because `return_borrow_map` is always empty (F5). | `checker.rs:754-796`, `checker.rs:1384` |
| F4 | The driver has already wired `callee_sigs` + `check_body_with` (`main.rs:95, 102`). The second link of PropagatedLoan was connected in ADR-0045. | `main.rs:95-102` |
| F5 | **Sole vulnerability:** `return_borrow_map` in lowerer is always instantiated as empty `::new()` (`lib.rs:168-170`). Without population → PropagatedLoan does not re-issue loans at callers → `let stolen = m;` following `id(&0 m)` slips past borrowck. | `lower/lib.rs:168-170` |
| F6 | The E1042 gate (`check.rs:398`) currently blocks ALL `-> Ref T` via `if let Type::Reference(_, _)`. Needs conversion to an explicit match to whitelist `&0`. | `check.rs:398-408` |
| F7 | Mentor O experimentally probed: temporarily disabling E1042 for `&0`, testing `id(&0 m); let stolen = m; use r` → slips through (no borrow error), exit 0. Confirms F5 is a real hole. | Probe session 2026-06-08 |

---

## §1 — E1042 Form-Gate: Whitelist `&0`, Refuse the Rest

**Decision:** Change `if let Type::Reference(_, _)` (`check.rs:398`) into a `match` that exclusively allows `ReferenceForm::BorrowReadOnly` through, while preserving the E1042 refusal for `StrongFrozen`/`StrongMutable`/`BorrowExclusiveMutable`/`WeakObserver`.

**Rationale:** `&0` (shared read-only) is the only form that can safely support return-borrow under the current infrastructure:
- `&+` (strong owning) → involves refcounting/ObjectHeader, defer.
- `&+ mutable` → similarly deferred.
- `&0 mutable` → exclusive borrow requires more complex exclusivity guarantees, defer.
- `&-` (weak observer) → no clear use case yet, defer.

**Implementation:**
```rust
// check.rs:398 — replace if let Type::Reference(_, _) with:
match &return_type {
    Type::Reference(form, _) => match form {
        ReferenceForm::BorrowReadOnly => { /* allow through */ }
        _ => {
            // E1042 for StrongFrozen / StrongMutable /
            // BorrowExclusiveMutable / WeakObserver
            self.errors.push(TypeError::BorrowReturnNotYetSupported { ... });
        }
    },
    _ => {}
}
```

---

## §2 — Elision: REUSE E2400, DO NOT Introduce E1043

**Decision:** `check_lifetime_elision` (`check.rs:494`) is the single source-of-truth for elision decisions. Lowerer MUST NOT rewrite rejection logic. Do not create error code E1043.

**Rationale:** E2400 already catches count ≠ 1 with clean diagnostics + 3 fix suggestions (ADR-0025 §3.4). Adding E1043 in the lowerer is redundant — typecheck is already a fatal gate (driver `main.rs:64` `ExitCode::from(3)`). Lowerer only needs a defense-in-depth Err (non-panicking) in case typecheck ever leaks (see §3).

**Note:** G initially considered introducing E1043 during ADR discussions — dropped after O pointed out that E2400 already exists with superior diagnostics. This ADR explicitly records the rationale so future sessions do not recreate it.

---

## §3 — Lowerer Populates `return_borrow_map`

**Decision:** At `lower/lib.rs:168`, replace `ReturnBorrowMap::new()` with the following logic:
- Count ref-parameters (non-owning `&0`/`&0 mutable`/`&-`).
- `count == 1` → `return_borrow_map[FieldPath::Root] = {param_index}`.
- `count != 1` → `Err(LowerError{...})` **WITHOUT panicking!**.

**Rationale for Err-not-panic:** Typecheck E2400 is fatal (`ExitCode 3` in driver `main.rs:64`) — when lowerer runs, count ≠ 1 is already guaranteed impossible. HOWEVER:
- The `run_fixture` harness (`integration_tests.rs:64`) intentionally pushes through type errors to test `// ERROR` annotations — a `panic!` would trigger SIGABRT and kill all 74 fixtures.
- With `Err`, the harness catches it at lines 71-74: `Err(e) => { errors.push(...); return Err(...) }` → that fixture fails cleanly (lower error), corpus continues to next fixture — NO SIGABRT abort.

**LowerError Pattern:**
```rust
_ => return Err(LowerError {
    message: "internal: return-borrow elision expects exactly 1 ref-param               (typecheck E2400 should have rejected this)".into(),
    span,
}),
```

**Prior Art:** `lib.rs:1200` — `ok_or_else(|| LowerError { ... })` for missing enum variants. Identical pattern: internal invariant violated → Err, not panic.

**TECH-DEBT (O, 2026-06-08):** `is_propagated` skips E2450 at Drop (`checker.rs:692`) based on the assumption that Triet does not yet have nested block scopes and early moves are already blocked by E2440, so propagated refs CANNOT outlive their owners. If nested block scopes are added later or refs are allowed broader escape (e.g. captured in closures, multi-tier returned refs), this skip must be re-audited — propagated refs could genuinely outlive owners in those constructs. Documented to avoid forgetting when expanding the scope system.

---

## §4 — ELIMINATE E1043 (Explicit Rationale)

**Decision:** Do not create error code `E1043` for "return-borrow elision failed at lowerer".

**Rationale:**
1. E2400 already covers this case in typecheck with more comprehensive diagnostics (3 Fix suggestions).
2. Typecheck is a fatal gate — lowerer will never encounter count ≠ 1 in production.
3. Defense-in-depth in lowerer uses `LowerError` (string message) — no dedicated error code is needed since this is an internal invariant rather than a user-facing diagnostic.

Recorded in this ADR so future sessions do not recreate E1043.

---

## §5 — Teeth (3 Groups)

### Group A — Caller-hole (Load-bearing)

New fixture: `id(&0 m); let stolen = m; use r` — move `m` while `r` (borrowed from `m`) remains live → borrowck must reject (error code verified via RUN during implementation, expected E2440 or E2450).

| Fixture | Directive | Expectation (guard PRESENT — populate active) | Teeth (remove populate → empty `::new()`) |
|---------|-----------|------------------------------------------------|-------------------------------------------|
| `79_return_borrow_caller_freeze.tri` | `// ERROR: E24xx` (negative fixture) | Borrow error fired → test PASS | Slips through, exit 0, no error → fixture RED |

**Important:** This is a negative fixture — directive `// ERROR:`, not `// EXPECT:`. When guard is present, borrowck fires error → test passes. When populate is removed, code compiles + runs with exit 0 (latent use-after-free) → fixture turns red because EXPECT error found no error. This is the primary regression test for slice 3 — if anyone removes populate, this test MUST fail, proving the guard genuinely protects.

### Group B — Return-local

Fixture: `fn f(s: &0 String) -> &0 String { let x; return &0 x }` → E2450.

Already live (E2450 operates independently of `return_borrow_map`). Merely requires fixturization.

### Group C — 0/Multi ref-params

| Test | Input | Expectation |
|------|-------|-------------|
| `81_return_borrow_0_param.tri` | 0 ref-param → `fn f() -> &0 String` | E2400 |
| `82_return_borrow_multi_param.tri` | 2+ ref-params → `fn f(a: &0 String, b: &0 String) -> &0 String` | E2400 |

Already live (E2400 operates independently). Merely requires fixturization.

---

## Implementation Plan

| # | Task | Primary File | Teeth |
|---|------|--------------|-------|
| 1 | ADR → commit | `docs/decisions/0046-return-borrow-elision.md` | O reviews line-refs before coding |
| 2 | §1 E1042 form-gate | `typecheck/src/check.rs:398` | Fixture: `-> &+ T` remains E1042; `-> &0 T` passes typecheck |
| 3 | §3 lower populate `return_borrow_map` | `lower/src/lib.rs:168` | Group A teeth: remove populate → slips through |
| 4 | count ≠ 1 → Err(LowerError) | `lower/src/lib.rs:168` | Multi-param fixture: harness does not SIGABRT, produces E2400 |
| 5 | Fixture return-local → E2450 | Fixtures | Already live, fixturize |
| 6 | Fixture 0/multi param → E2400 | Fixtures | Already live, fixturize |
| 7 | Close report | `scripts/gate.sh` raw | O independently reruns and validates teeth |

Gate `scripts/gate.sh` raw after every step. Every new fixture is added to corpus.

---

## Q&A

### O-Q1: Why not panic in lowerer when count ≠ 1?

The `run_fixture` harness intentionally pushes through type errors (`integration_tests.rs:64`). A `panic!` → SIGABRT kills all 74 fixtures. `Err(LowerError)` → harness catches and continues. Prior art: `lib.rs:1200`.

### O-Q2: Why not introduce E1043?

E2400 already exists with superior diagnostics (3 Fix suggestions), and acts as a fatal gate (typecheck aborts with `ExitCode::from(3)` in `main.rs:64`). E1043 is redundant. G previously proposed and dropped it — recorded in this ADR to avoid re-creation (§4).

### G-Q1: Are there any changes to the PropagatedLoan engine?

No. The engine (`checker.rs:754-796`) has been functioning correctly in unit tests since ADR-0045 §4. This slice merely populates `return_borrow_map` in the lowerer — the engine automatically re-issues loans when it sees a non-empty map.

### G-Q2: Are there ABI changes?

No. Return-borrow continues to pass i64 handles by-value as before. The ABI is identical for owned and borrowed values — the difference is purely semantic, governed by borrowck + lowerer.

### G-Q3: `&0 mutable` / `&+` return-borrow?

Deferred. Only `&0` shared read-only is supported in this slice (§1).
