# ADR-0045: Borrow Params Heap — Tier C Slice 2 (Shared Read-Only)

**Status:** ACCEPTED — O + G sign-off 2026-06-08
**Date:** 2026-06-08
**Author:** AI (collaborator D, survey + proposal)
**Reviewers:** Mentor O (semantics, soundness) — SIGNED 2026-06-08 · Mentor G (layout, ABI, codegen) — SIGNED 2026-06-08
**Scope:** `&0 T` (shared read-only) for heap types (String, Vector, HashMap) across user-fn boundaries. Enable minimal read-ops via references.

---

## Summary

The B7-lift (ADR-0042) allowed heap types across function boundaries with **Move-only** semantics —
one could not read a String without relinquishing ownership. Tier C Slice 2 permits `&0 String` /
`&0 Vector<T>` / `&0 HashMap<K,V>` as parameters: callee reads via a shared reference,
caller retains ownership, and continues using the value after the call. The JIT remains unchanged — references
pass an i64 handle by value, maintaining an ABI identical to owned types. The difference is purely semantic,
managed by borrowck + lowerer.

---

## §0 — Facts

| # | Fact | Location |
|---|------|----------|
| F1 | B7-lift (ADR-0042) allowed heap types in parameters, but only via Move. Move semantics: callee owns + drops, caller zeroes handle after call + borrowck M3+ marks as Moved. | `lower/lib.rs:468-478`, `checker.rs:828-853` |
| F2 | `type_name` (lower) rendered reference type `&0 T` as `"?"` (fallback `_ => "?"`). `"?"` was classified as Copy by `is_copy` (both lower `simple_is_copy` and MIR `is_copy`). | `lower/lib.rs:522` (type_name), `lower/lib.rs:549` (simple_is_copy), `mir/lib.rs:2221` (is_copy) |
| F3 | Consequence of F2: callee STILL emitted `Drop(_0)` for ref parameters — but Drop was harmless because JIT skips free for Copy (handle was not freed). Borrowing currently worked due to an accidental chain of behaviors, not by design. | MIR dump confirmed: `fn process(a: &0 String) → Drop(_0)` |
| F4 | `ReturnBorrowMap` + `PropagatedLoan` engine existed in checker (`checker.rs:754-796`), protected by unit test (`returned_reference_extends_source_lifetime`). The engine worked in tests but never ran in production due to two broken links: (a) `lower/lib.rs:168` always initialized an empty `ReturnBorrowMap::new()`; (b) `driver/main.rs:96` called `check_body` (without signatures). | `checker.rs:754-796`, `checker.rs:1372-1419` |
| F5 | `-> &0 T` as a return type was accepted by typecheck. `fn id(s: &0 String) -> &0 String { return s }` compiled without error and was not caught by checker. Created a latent use-after-free: moving the owner after 1 statement of padding passed the checker; became an actual error as soon as read operations via references were enabled. | MIR dump confirmed: `fn id(...) -> ? { Drop(_0) Return(_0) }` |
| F6 | JIT `CallDispatch` (`mir_lower.rs:944-1004`) passed arguments uniformly: struct → `stack_addr`, enum → `stack_load`, scalar/ref → `use_var`. No distinction between Borrow/Move. Heap handle = i64, reference passed that handle by value. | `mir_lower.rs:974-989` |
| F7 | `checker.rs:828-853` (M3+) marked ALL non-Copy arguments of `CallTarget::Jit` as Moved, without distinguishing Borrow vs Move. | `checker.rs:834-852` |

---

## §1 — ABI: handle i64 by value, no double-pointer

**Decision:** Heap references (`&0 String`, `&0 Vector<T>`, `&0 HashMap<K,V>`) =
handle i64 passed by value, identical ABI to owned types. No pointer-to-handle.

**Rationale:** The heap handle is already a pointer (i64). A reference to a heap value uses
this exact handle — the callee reads through the same pointer without requiring an additional
layer of indirection. F6 confirms the JIT requires no changes.

**Affirmation:** G-Q1 "pass handle directly, eliminate pointer-to-handle" — absolute
consensus with O.

---

## §2 — Codegen rule: callee does not Drop, caller does not zero

### Callee

**Option A (Chosen):** Lowerer does not invoke `push_owned` for borrow parameters → MIR does not
emit `Drop` for that parameter in the first place.

Rejected the option to "retain Drop and rely on is_copy" — fragile: once §3 lands (concrete type
for references), if someone accidentally changes `is_copy(&0 String) → false`, callee Drop would
free the handle owned by the caller → runtime double-free.

**Implementation:**
- `lower/lib.rs:472`: `push_owned(l)` → only called when `passing_mode == Move`
- No `StorageDead` for borrow parameters (already managed by `push_owned`)

### Caller

**Implementation:**
- `lower/lib.rs:1399-1405` (sret path): `to_zero` filter → skip argument if callee parameter is Borrow
- `lower/lib.rs:1433-1439` (scalar path): same
- `checker.rs:828-853` (M3+ move-mark): skip argument if callee parameter is Borrow
- **Lowerer side:** add `func_param_modes: HashMap<String, Vec<ParameterPassing>>`
  to `LowerCtx`, built at `lib.rs:389` alongside `func_return_types`.
  Query `func_param_modes[callee_name][i]` to decide Deinit/Moved.
  Do not use `FunctionSignature` (MIR type) — MIR does not yet exist when building
  the registry, and return-borrow is CUT (§5) so `return_borrow_map` is not needed.
- **Borrowck side:** no new registry needed. `check_body_with(body, callee_sigs)`
  already has parameter support — step 4 wires driver to call `check_body_with`
  using signatures from `lower_program` (each Body already carries a complete `.signature`).

---

## §3 — Concrete type for references (Foundation)

**Decision:** `type_name` must render the concrete reference type (`&0 String`,
`&0 Vector<Integer>`, etc.) instead of the `"?"` fallback. `is_copy(reference_type)` =
`true` **by design** — references are Copy: copying a valid reference handle does not
cause a double-free because the callee does not Drop (§2).

**Eliminate reliance on accidental `"?"` behavior (F2-F3).** This is a prerequisite — every other
step depends on concrete types being present in MIR.

**Important distinction:** `TypeExpr::Reference { form, inner }` is an AST type
expression (`triet-syntax/src/type_ast.rs:121`, struct variant — **not** a
tuple). `Type::Reference` in schema (`generated/types.rs:204`) is spec-only,
not yet wired. `is_copy`/`simple_is_copy` match on **strings** (string prefix
rendered by `type_name`: `"&0 String"`), not on enum variants.

**TECH-DEBT (G-mandate, SIGNED 2026-06-08):** Using `s.starts_with("&0 ")` as a
type-tag at the MIR level is an **acceptable evil** — MIR currently stores types as
`String`, with precedent in `is_vec_type`/`is_hashmap_type` using `starts_with`. Accepted
so as not to break the MIR schema mid-Tier C. DEBT: eventually migrate MIR Type from
`String` to an explicit AST Node / enum. All prefix matches added in this slice
MUST carry the comment `// TECH-DEBT(ADR-0045): MIR-type-as-string, see §3`.

**Implementation:**
- `lower/lib.rs:type_name` (522): add branch `TypeExpr::Reference { form, inner }` → format `"&0 {inner}"` / `"&+ {inner}"` / `"&- {inner}"` based on `form`
- `mir/lib.rs:is_copy` (2221): add prefix match `s if s.starts_with("&0 ") || s.starts_with("&+ ") || s.starts_with("&- ")` → `true`
- `lower/lib.rs:simple_is_copy` (549): add matching prefix match → `true`

---

## §4 — PropagatedLoan engine: KEEP + TODO, do not delete

**Decision:** KEEP the PropagatedLoan engine (`checker.rs:754-796`) + KEEP unit test
`returned_reference_extends_source_lifetime` + add TODO pointing to future return-borrow slice.

**Rationale for rejecting deletion:** Unit test was teeth-verified by O (failed when removed) — engine
logic is sound and protected by tests. Deleting it would discard proven assets.

**TODOs:**
- `checker.rs:754`: `// TODO(return-borrow slice): wire return_borrow_map from lowerer into driver → check_body_with`
- `lower/lib.rs:168`: `// TODO(return-borrow slice): populate return_borrow_map when callee returns &0 T`

**Note for G:** Engine is DEAD-in-production (due to two broken links: lowerer does not
populate + driver does not pass sigs — G only saw the first link), but
LIVE-in-test. Do not delete.

---

## §5 — Return-of-borrow: CUT (typecheck refuses `-> &0 T`)

**Decision:** Typecheck refuses `-> &0 T` (and `-> &+ T`, `-> &- T`) in this slice.
Error code **E1042 BorrowReturnNotYetSupported**.

**Rationale:** Accepted-wrong vulnerability (F5): `fn id(s: &0 String) -> &0 String {
return s }` compiled without error and was not caught by borrowck. Currently harmless because
read ops via ref do not exist; becomes an actual use-after-free once ops are enabled (§8).
Close the door before opening ops.

**Reopening:** Future return-borrow slice (after PropagatedLoan is wired into
production — §4).

**Error code note:** E1041 had a dual collision: (a) pre-existing code in `error.rs:46`
`NoMatchingOverload` — currently emitted and tested in `lib.rs:1410`; (b) ADR-0039 reserved
`NullableHasNoErrorState` for `?->`. E1042 avoids both entirely. The name
`BorrowReturnNotYetSupported` accurately reflects "not yet supported" (will be reopened) —
not permanently prohibited.

---

## §6 — Mutability: shared read-only only

**Decision:** This slice only supports `&0 T` (shared read-only). `&0 mutable T` (exclusive
mutable borrow) and `&+ T` (strong owning reference) are deferred to subsequent slices.

**Rationale:** `&0 mutable` requires exclusivity guarantees (E2440 for overlapping borrows)
— beyond the scope of this slice. `&+` relates to refcount / ObjectHeader — deferred.

---

## §7 — Call surface: explicit `&0` at call site

**Decision:** Callers must write `callee(&0 s)` — explicit borrow at call site.
Parser already supports this syntax (`Expr::Borrow`). No auto-borrow.

**Rationale:** Aligns with Triet's explicit-strictness convention. Auto-borrow hides
ownership transfers and blurs the semantic boundary between Move and Borrow.

---

## §8 — Ops for ref: enable minimal read-ops

**Decision:** Enable a minimal set of read operations on `&0 T` — otherwise, borrow parameters
are purely decorative (compilable but unusable). Scope strictly bounded
in this ADR; expansions are incremental (adding fixtures without changing ABI).

**Shim ABI unchanged:** Same i64 handle. Only typecheck accepts reference types in parameter
positions of builtin shims.

**Strictly bounded scope:**
- `length(s: &0 String) -> Integer` / `length(v: &0 Vector<T>) -> Integer`
  — always safe: returns Integer (Copy), does not expose heap handle.
- `get(s: &0 String, index: Integer) -> Integer` — read char code: safe (Integer Copy).
- `get(v: &0 Vector<T>, index: Integer) -> T` — **only when T is Copy**
  (`Vector<Integer>.get` OK; `Vector<String>.get` deferred).
- HashMap: similar to Vector — value must be Copy to enable.

**Soundness constraint for `get` with non-Copy elements:** Pattern Δ3
(`cannot_copy_move_type_out_of_field`, E2423). `get(v: &0 Vector<String>) -> String`
would copy the String handle out of the borrowed Vector → callee + caller hold the same
handle → double-free. The sound solution for non-Copy elements is return-borrow
(`-> &0 T`, CUT in §5) or clone (not yet available). Defer to return-borrow slice.

**Future extensions:** `contains`/`is_empty` for HashMap, iterators, slice windows —
incremental, adding fixtures without modifying ABI.

---

## Implementation Plan (after two sign-offs)

In strict prerequisite order (§3 is the foundation, must be done first):

| # | Task | Primary Files | Teeth (must fail when removed) |
|---|------|---------------|--------------------------------|
| 1 | Concrete type for references (§3) | `lower/lib.rs:type_name` (522), `mir/lib.rs:is_copy` (2221), `lower/lib.rs:simple_is_copy` (549) | Fixture: ref param → MIR carries type `&0 String`, not `"?"` |
| 2 | Lower does not push_owned for borrow params (§2 callee) | `lower/lib.rs:466-478` | Fixture: callee MIR contains no `Drop(_0)` for ref param; removing guard → Drop reappears |
| 3 | Caller does not zero borrow arg (§2 caller) | `lower/lib.rs:1399,1433` + add `func_param_modes` | Fixture: caller uses owner after `peek(&0 s)` → executes correctly, no E2420 |
| 4 | Driver collects sigs + calls `check_body_with` (link b of F4) | `driver/main.rs:96` | Unit test `returned_reference_extends_source_lifetime` remains green |
| 5 | Typecheck refuses `-> &0 T` (§5) | typecheck | Fixture: `-> &0 String` → E1042; closes accepted-wrong hole |
| 6 | Enable read-ops via `&0` (§8) — `length` + `get` (Copy-only) | typecheck + lower | RUN fixture: `length(&0 s)` → correct number; `get(&0 v, 0)` for Vector<Integer> → correct |
| 7 | TODO + retain engine (§4) | `checker.rs:754`, `lower/lib.rs:168` | Unit test `returned_reference_extends_source_lifetime` remains fail-on-removal |

Run raw `scripts/gate.sh` after each step. Add every new fixture to the corpus.

---

## Q&A

### G-Q1: ABI for heap borrow parameters?

Handle i64 by value. No double-pointer. (§1)

### G-Q2: How does callee know not to Drop ref parameters?

Not a runtime mechanism — lowerer does not emit Drop in the first place. (§2, Option A)

### G-Q3: PropagatedLoan engine — keep or delete?

KEEP + TODO. Engine is sound, test verified fail-on-removal. (§4)

### O-Q1: Explicit borrow at call site?

Yes. `callee(&0 s)`. (§7)

### O-Q3: `-> &0 T` return type?

CUT. Typecheck refuses with E1042 in this slice. (§5)

### O-Q4: `&+ T` / `&0 mutable T`?

Defer. This slice only covers `&0 T` shared read-only. (§6)

### O-Q5: Which ops are enabled for ref?

`length` + `get` (Copy-only). Scope strictly bounded in §8. Future expansion is incremental,
without changing ABI.
