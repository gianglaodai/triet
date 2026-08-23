# ADR-0059 — Stack-borrow (`&0`) for Heap Vector/HashMap + Clearing Generic Return-Bind Debt

- **Status:** 🔓 APPROVED (scope) — awaiting implementation C.1→C.2. Drafted by Mentor O on 2026-06-11, grounded in MIR+typecheck+JIT line-cited probes (real driver run, directly measured exit codes).
- **Date:** 2026-06-11
- **Drafted by:** Mentor O (root cause probe: measured what already ran for `&0 String`, pinpointed 3 breakages in Vector/HashMap, decoupled `&+` from scope).
- **Signatures:** O ✅ (root cause proven via driver-run + line citations; `&0` vs `&+` boundary proven) · G ✅ (scope approved 2026-06-11 — locked path (b), sealed `&+` per YAGNI, mandated lethal poison SIGABRT).
- **Related:** [ADR-0045](0045-borrow-params-heap.md) (`&0 String` borrow param — precedent reused), [ADR-0046](0046-return-borrow-elision.md) (return-borrow + reference-drops-before-owner sort), [ADR-0042](0042-ownership-across-boundary.md) (B7-lift heap param Move + Deinit tombstone), [ADR-0050](0050-mir-type-enum.md) (MirType — Vector/HashMap bare), [ADR-0022](0022-trit-balanced-ownership.md) (S6 5-form — source of `&+`/`&-` EXCLUDED from scope).

---

## 1. Context — `&0 String` is Live; Vector/HashMap Still Broken in 3 Places

Probe on 2026-06-11 (Mentor O) measured using `triet-driver run` on real files:

**ALREADY running end-to-end** (in the 160-fixture corpus, line-cited per point):
- `&0 String` / `&0 mutable String` borrow params: fixture `77_borrow_read_len`, `100_endgame_string_roundtrip` (append/realloc in callee), `84/101` return-borrow.
- Wiring mechanism: call-sites pass heap args via `stack_addr(slot)` (pointer-to-caller-slot, `triet-jit/src/mir_lower.rs:1463-1483`); callee **borrows** WITHOUT copying + WITHOUT Dropping (`triet-lower/src/lib.rs:621-626` skips `push_owned` for ref-types; test `borrow_param_no_drop_in_callee:4346`). JIT binds pointer directly for Reference-locals.

→ **Backend wiring for stack-borrow heap IS ALREADY CLOSED.** Track C is not built from scratch; it addresses three remaining typecheck/lower gaps for Vector/HashMap.

## 2. Root Cause — MEASURED FROM CODE, Three Breakages

| # | Symptom (driver-run) | Root Cause (file:line) | Layer |
|---|---|---|---|
| **B1** | `make()->Vector<Integer>; let ys=make(); len(ys)` → `lowerer error: len() on type ?` | `lower_type`/`lower_type_simple` (`triet-lower/src/lib.rs:740-789`, `802-848`) **lacks a `TypeExpr::Generic` branch**. `Vector<Integer>` (parser produces `Generic`, `triet-parser/src/type_expr.rs:202`) falls through to `_ => MirType::Unknown`. User-fn return-type Vector → result-local Unknown → `len()` error. | Lowerer |
| **B2** | `peek(v: &0 Vector<Integer>)->Integer{return len(v)}` → `E1041 no overload of len` | `triet-typecheck/src/env.rs`: `len` only has owned `(String)`/`(Vector<Integer>)`/`(HashMap)` (lines 273-349) — **NO `&0` variant**. `get` (lines 292-339) is similarly owned-only. (`contains`/`is_empty` ALREADY have `&0` for all three: 389-456.) | Typecheck |
| **B3** | `peek(s: &+ String)` → `E1041`, help only suggests `(String)`/`(&0 String)` | `&+` StrongFrozen is not accepted by any stdlib function | **EXCLUDED from scope** (§5) |

**Exact Asymmetry (Measured):** the open `&0` read-op gap exists solely in **`len` + `get`**. `contains`/`is_empty` already support `&0` for String/Vector/HashMap. `length` supports `&0 String` but is a distinct function (not `len`).

## 3. Decision (G locked path (b) — scope approved 2026-06-11). TWO SLICES.

Track C is redefined: **Eradicate Type/Generic debt + extend Stack-Borrow (`&0`) to heap Vector/HashMap.** `&+`/`&-` EXCLUDED (§5).

### Slice C.1 — Clear Generic Return-Bind Debt (B1)

Add `TypeExpr::Generic { name, arguments }` arm to **both** converters `lower_type` (`lib.rs:740`) **and** `lower_type_simple` (`lib.rs:802`):
- `name == "Vector"` → `MirType::Vector` (strip element type — Tier A bare, per ADR-0050).
- `name == "HashMap"` → `MirType::HashMap`.
- Otherwise → retain `_ => MirType::Unknown` (refuse-over-guess).

Consequence: user functions `-> Vector<Integer>` / `-> HashMap<...>` assign proper types to result-locals → `len`/`get`/`contains` on call results function properly.

### Slice C.2 — `&0` Read-Overloads for Vector/HashMap (B2)

`env.rs`: add `declare_overload` for `len` and `get` with `&0` variants:
- `len(&0 Vector<Integer>) -> Integer`, `len(&0 HashMap) -> Integer`.
- `get(&0 Vector<Integer>, Integer) -> Integer?`, `get(&0 HashMap, Integer) -> Integer?`.
- (Optional symmetry: `len(&0 String)` — decided during review if fixtures require it.)

**OUT OF SCOPE for C.2:** `push`/`insert` (mutate → currently consume + return owned; converting to `&0 mutable` in-place is a semantic shift, left to a separate ADR if needed). `contains`/`is_empty` already have `&0` — untouched.

Backend: `len` already strips the `&0` prefix (`lib.rs:1733-1737`), JIT binds pointer + shim receives pointer-to-slot — `len(&0 Vector)` runs immediately once the overload exists. **HOWEVER, `get` (`lib.rs:1939-1948`) dispatches via `arg0_ty.is_vec()/is_hashmap()` DIRECTLY WITHOUT stripping references** (`is_vec` = `matches!(self, Vector)`, unable to see through `Reference`). → `get(&0 Vector)` passes typecheck but FAILS in lowerer with `get() on type &0 Vector`. **C.2 MUST include lower-fix for `get`**: strip references like `len`. (Correction §8 — initial claim that "backend needs no modification" was FALSE for `get`.)

## 4. Teeth (Boundary of Life and Death) — Route-lower via `lower_source`, FORBID Hand-Building MirBuilder

### C.1 (Generic Return-Bind)
- **Positive:** fixture `make()->Vector<Integer>{...} ; main(){ let ys=make(); return len(ys) }` → RUN, exit = correct len.
- **Poison:** revert `TypeExpr::Generic` arm → fixture regresses to `lowerer error: len() on type ?`. Test must fail when arm is reverted; pass when restored.

### C.2 (`&0` Borrow Vector) — ⚰️ LETHAL POISON (G demands SIGABRT)
- **Positive:** `peek(v:&0 Vector<Integer>)->Integer{return len(v)}`; caller reuses `xs` after borrow → NO E2420; RUN exit equals correct len.
- **🩸 EXECUTION ORDER (Correction §8 — exact poison location verified via real crash):** poison `triet-lower/src/lib.rs:608` — strip Reference → owned inner type (`if let MirType::Reference{inner,..} = ty { ty = *inner; }`) → borrow param lowers to OWNED Vector → callee copies `{ptr,len,cap}` + Drops/frees caller's buffer → caller reuses + scope-Drops a second time → **DOUBLE-FREE → SIGABRT (134)**, `free(): double free detected`.
  - ⚠️ **DO NOT poison `lib.rs:624`** (`push_owned` guard): O attempted this — lowerer emits `Drop(_0)` but JIT Drop handler is **type-gated** (does not free `Reference`-typed locals) → neutralized → exit 0, NO crash. Borrow-param protection consists of TWO independent layers; only poisoning type-classification (608) defeats both simultaneously.
  - **Proven via real crash on `&0 String` path** (fixture 77 under type-poison → exit 134, `free(): double free detected in tcache 2`, 2026-06-11). The Vector mechanism is IDENTICAL (sharing `lower_function:608`) — executable once C.2 acquires overloads.
  - Measure `exit == 134` directly without pipe; **capture for G**. This is an ACTUAL observable hazard (glibc double-free abort) — unlike vacuous deferral cap@24 (ADR-0058).
  - Restore via `cp` snapshot from `/tmp` (NEVER `git checkout` — teeth-never-checkout rule).

## 5. OUT OF SCOPE — `&+` StrongFrozen / `&-` Weak → Backlog (YAGNI)

`&+` (StrongFrozen) / `&+ mutable` (StrongMutable) / `&-` (WeakObserver) under S6/ADR-0022 represent **shared-ownership refcounting**, requiring an 8-byte `ObjectHeader` + runtime retain/release shims + drop-decrement. Current heap shims (String/Vector/HashMap) are bare `{ptr,len,cap}` — NO ObjectHeader, NO refcount. This is a separate subsystem, not a param-passing refinement.

**Decision:** seal under YAGNI (same logic that closed C3/C4 in earlier campaigns). **Unlock condition:** when a genuine use-case consuming shared-ownership arises (e.g. 2 co-equal owners live concurrently on one heap object) → open a dedicated ADR for ObjectHeader refcount runtime, with 2 signatures.

## 6. Consequences

- (+) `Vector<Integer>`/`HashMap` become first-class across user function boundaries (return-bind + borrow read), closing the debt where "only bare local holds heap" on the Vector side while String already had it.
- (+) No additional runtime, no new shims — pure lowerer converter + typecheck overload additions.
- (−) `len`/`get` overload set expands — acceptable (symmetric with `contains`/`is_empty`).
- (−) `&+`/`&-` remain E1041 — intentional, with defined unlock conditions (§5).

## 7. Operational Directives

1. **Implementer builds slice by slice**: C.1 first (independent), submit to O for review + raw gate → O tests manually (poison Generic arm, measure failure) → G signs → commit. Then proceed to C.2.
2. C.2 review: O independently forces **SIGABRT 134** via poison at `lib.rs:608` (type-strip, §4 corrected) on the FINAL code, measures exit code directly, **captures output** for G. Without this SIGABRT, C.2 is rejected.
3. Every slice: raw gate first line (auto-reject if not raw), update TODO.md + handoff.

## 8. Amendment 2026-06-11 — Correcting Teeth + get Scope (Append-Only)

While drafting work-order C.2, O probed deeper + pre-verified teeth on real code, discovering TWO errors in the original draft (§3/§4/§7):

1. **Teeth poisoned wrong location.** Original draft specified poisoning `lib.rs:624` (`push_owned` guard). O forced this: lowerer DOES emit `Drop` for borrow params, but **JIT Drop handler is type-gated** — skips `Reference` locals → NO free → exit 0, NO SIGABRT. Protection is TWO independent layers. **The CORRECT poison location = `lib.rs:608`** (strip Reference → owned). Verified on `&0 String` (fixture 77) → **exit 134 `free(): double free detected in tcache 2`**. Lesson: pre-verify teeth BEFORE delegating; never promise crashes before seeing them happen.
2. **`get` requires lowerer fix.** Original claim that "backend needs no modification" held true for `len` (already stripped ref at `lib.rs:1733-1737`) but was FALSE for `get` (`lib.rs:1939-1948` used `arg0_ty.is_vec()` directly; `is_vec` = `matches!(Vector)` without looking through Reference). C.2 must add reference stripping for `get`.

- **Amendment Signatures §8:** O ✅ (verified correct poison location via real crash + measured `get` gap in code 2026-06-11) · G ✅ (Report reviewed. Poison must target the exact type-strip point (608) to pierce both defensive layers. Approved.)
