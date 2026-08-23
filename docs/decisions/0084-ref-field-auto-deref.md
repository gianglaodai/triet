# ADR 0084 — Field projection through read-only reference (auto-deref member-access)

> Status: **SIGNED (2026-07-26) — O ✅ G ✅.** O verified probe (CLI twice) + blood-verified new-poison-386 (E2450 vanishes + compiles CLEANLY = chase is sole-guard on path to runtime, not defense-in-depth). G accepted full ADR (Slice 1a + §AMEND 1b) + diamond gate `0·clean·0·463·0`. Verification campaign closed.

> # 🩸 CORE PRINCIPLE (Semantic author: O)
> # Member-access `e.f` when `e : &0 T` (read-only reference to UserStruct `T`)
> # **auto-dereferences EXACTLY 1 level** then projects field `f` on pointee. The kind of the
> # result is **strictly determined by the field's static type** — UNAMBIGUOUS:
> #  - `f` is **scalar** (Trit/Tryte/Integer/Long/Trilean) → **scalar value**
> #    (Copy, read via borrow). Terminal.
> #  - `f` is **aggregate** (Struct) OR **heap-leaf** (String/Vector/HashMap) →
> #    **`&0 F` sub-borrow** (zero-copy place projection, SAME root loan).
> #    Chainable: `(&0 Outer).inner` : `&0 Inner` → `.x` auto-dereferences further.
> #  - **NEVER copy/move** aggregates or heap values — only scalars read
> #    by-value. Copying heap via borrow = aliasing heap pointers → double-free on drop
> #    (the exact hazard forbidden by ADR-0082/§AMEND-3).
> #  - **1 level:** each `.f` dereferences exactly 1 preceding `&0`; chaining works because each
> #    step re-borrows 1 level.
> #  - **Read-only:** writes via `&0` (`e.f = v`, `*e = v`) REMAIN refused. `&0 mutable`
> #    may follow the same dereference for READS, while writes remain independently refused.

## Scope

- ✅ **IN (Slice 1a — this revision):** `e.f` with `e : &0 T` / `e : &0 mutable T`, `T`
  = UserStruct, and `f` is a **SCALAR** field. Single-level auto-deref → read field by
  value (Copy). Both reference-PARAMS (`function read(p: &0 Point)`) and
  reference-to-LOCALS (`let r = &0 v`). Trivial leading-scalar chains
  (`(&0 v).x`).
- 🔜 **Slice 1b (DEFERRED):** `f` is an **aggregate/heap** field → `&0 F` zero-copy
  sub-borrow (chainable). Requires place-projection mechanism preserving loans through lowerer/JIT
  not yet built — opened separately to prevent covert heap copies. Semantics locked in CORE PRINCIPLE above.
- ❌ **OUT:**
  - `&0 Enum` field-access (`(&0 e).field`) → retains **E1015 UnknownMember**. Enums
    are accessed via `match`, not `.field`.
  - Writes via reference (`e.f = v`) → refused (§WART: currently parser E0007 blocks all
    non-identifier assignment targets, so no write surface exists to test).
  - `&+`/`&-`/`WeakObserver` field-access → out of scope (uninvestigated).

## Sites (Wired — Slice 1a)

1. **Typecheck** `crates/triet-typecheck/src/check/exprs.rs` `check_field_access`
   (~1608): at function entry, if `object_ty = Type::Reference(BorrowReadOnly |
   BorrowExclusiveMutable, UserStruct{fields})` and field exists and
   `field_ty.is_scalar()` → return `field_ty.clone()`. Aggregate/heap fields DO NOT
   match `is_scalar()` → fall back to `UnknownMember` as before (unlocked in Slice 1b).
2. **Lowerer** `crates/triet-lower/src/lib.rs`:
   - `place_result_type` (~1699): add arm `Projection::Deref` → unwrap
     `MirType::Reference{inner}`.
   - `lower_place` `Expr::FieldAccess` arm (~1682): if base resolves to
     `MirType::Reference{..}` → insert `Projection::Deref` BEFORE `Field`. Nested chains
     emit only 1 Deref at the root (standard struct fields do not store themselves as pointers).
3. **JIT** `crates/triet-jit/src/mir_lower.rs`:
   - `walk_projections` (~349): add arm `Projection::Deref` → unwrap
     `Reference{inner}`, offset UNCHANGED (dereferencing does not shift address;
     the "pointer-based fallback" branch in `load_place`/`store_place` @~1295 already adds
     `total_offset` to the pointer value in `place.local`'s variable).
   - **Blocker B (§WART-B)** `Statement::Borrow` (~3110): extended — extracts
     `stack_addr` for ALL slot-backed locals (`struct_slots`/`enum_slots`), not
     just String. String also resides in `struct_slots` (name == "String"), so the new
     branch SUBSUMES the legacy special case.
4. **Borrowck** — UNTOUCHED. Sub-borrows (Slice 1b) will be projections of existing
   `&0` loans, generating no new loans; reads via `&0` = read-use, non-conflicting.

## ⚠️ WART — Lexical Borrowck (non-NLL) — Explicitly Recorded per G's Order

The borrow checker is currently **lexical**: loans expire at the end of scope, WITHOUT NLL
last-use narrowing. Consequence for auto-deref: a LOCAL borrow (`let r = &0 v`) surviving
until the **owner's return point** triggers a **false E2450 DropWhileBorrowed**
(ADR-0046 Case-D conservative — fixtures 21/24 lock this behavior). This is **NOT
unsound** — merely cumbersome. To use auto-deref on local borrows, force the borrow
to expire BEFORE return via:

- **nested block scopes:** `let n = { let r = &0 v; r.x + r.y }; return n;`
  (`pop_scope` drops references first → borrow drops at block exit, before return).
- **reference-PARAMS:** `function read(p: &0 Point) -> Integer { return p.x + p.y }`
  invoking `read(&0 pt)` (borrow lives only across call).

**NLL = bottomless pit, deferred INDEFINITELY.** DO NOT touch `flush_all_for_return` /
ADR-0046 to "remedy" this wart.

### §WART-B — `Statement::Borrow` slot-address (Blocker B, patched)

Prior to this ADR, `Statement::Borrow` codegen only special-cased String to obtain slot
addresses; all other struct/enum locals used `use_var(var(source.local))`. However,
aggregate locals are constructed via field-level `stack_store` — their Cranelift Variables
are NEVER `def_var`'d → `use_var` returned **undefined** values → borrow
pointed to garbage → **SIGSEGV** on the first field-read through the reference. Undetected previously
because NO fixture had borrowed a local struct directly (only scalars / String /
container-handles / params). Fix: obtain `stack_addr` for all slot-backed locals.

## Rationale

- **Zero-copy = G's philosophy:** reading via borrow does not copy; aggregate/heap fields
  (Slice 1b) are sub-borrows, never duplicated.
- **Unambiguous static kind:** result kind (value vs sub-borrow) is statically determined by field type
  — requiring no runtime inference.
- **No heap copies → preserves ADR-0082 soundness:** copying heap values through borrows creates
  pointer aliases → double-free. Slice 1a reads scalars only (Copy), ensuring absolute
  safety; heap/aggregate deferred to 1b with sub-borrow mechanics.
- **Read-only → avoids Cluster D / ADR-0081 (mutable-borrow FROZEN):** Slice 1a
  is READ-only; write-paths do not exist (parser gate).

## Guard Layering Theorem (Signed by G 2026-07-26)

The two guard layers DO NOT replace each other — each possesses its own exclusive domain:
- **Typecheck E2400** (BorrowLifetimeInferenceFailed) = primary guard for **UNBOUND
  return-escapes** (returning `&0` un-tied to inputs). Blocked at typecheck, never reaching borrowck.
- **Borrowck reborrow-chase** = **SOLE-GUARD** for:
  (a) **move-while-borrowed** (387 / E2440) — typecheck remains silent, caught exclusively by borrowck;
  (b) **BOUND return-escapes** (new 386 / E2450) — parameter-tied escape bypassing E2400, chase exclusively catches
  dangling local sub-borrows.
Removing chase yields actual UB (dangling pointers reaching JIT) for BOTH (a) and (b), NOT mere phantom errors.

## Safeguards (Slice 1a)

- **381** (`381_ref_field_scalar_param.tri`, EXPECT 30) — scalar field via `&0
  Point` PARAM; caller `read(&0 pt)` borrows local struct → exercises Blocker-B
  fix. 2-tier poison: remove auto-deref in typecheck → E1015; revert Blocker-B →
  SIGSEGV 139.
- **382** (`382_ref_field_scalar_block_local.tri`, EXPECT 30) — scalar field via
  `&0` to LOCAL struct, borrow confined inside nested block (bypassing §WART E2450).
- **T3 nested (confirmed belonging to 1b, NOT written in 1a):** all nested accesses
  (`o.inner.x` with `inner : Inner` aggregate) require sub-borrowing at `.inner` step →
  Slice 1b. NO valid nested-scalar safeguards in 1a.
- **No negative mutation safeguards:** parser E0007 (`InvalidAssignmentTarget`) already blocks
  all non-identifier assignment targets at parse time — vacuous for auto-deref.

## §AMEND — Slice 1b landed (SIGNED — O ✅ G ✅)

> Status for this section: **SIGNED (2026-07-26) — O ✅ G ✅.** Verified alongside top-level package.

Slice 1b implements the exact DEFERRED portion of CORE PRINCIPLE (aggregate/heap fields via `&0`
→ `&0 F` zero-copy sub-borrow). NO new semantics. 4 layers:

1. **Typecheck** `check_field_access` (exprs.rs ~1620): expanded auto-deref branch —
   after matching `Reference(BorrowReadOnly|BorrowExclusiveMutable, UserStruct)` and
   verifying field existence: `is_scalar()` → value (1a preserved); `UserStruct{..}` OR
   `is_heap()` (String/Vector/HashMap) → return
   `Type::Reference(BorrowReadOnly, field_ty)` (sub-borrows are ALWAYS read-only, even on
   base `&0 mutable`). Other field kinds (Enum, Nullable-aggregate) fall back to
   `UnknownMember` (§OUT).
2. **Lowerer** `Expr::FieldAccess` rvalue (lib.rs ~3313): if `source` (generated by
   `lower_place`) contains `Projection::Deref` AND `place_result_type` is
   `Struct`/heap → emit `Statement::Borrow{form:BorrowReadOnly, source:[Deref,
   Field]}` (targeting `Reference`-typed destination), rather than `Assign` value-copy. Scalar
   terminals (1a) and owned-base move-outs (WO-0074/0075, LACKING Deref) fall through to
   legacy `Assign` — unchanged.
3. **JIT** `Statement::Borrow` codegen (mir_lower.rs ~3127): `source` may now be
   projected. New branch — `walk_projections` (Deref adds 0 offset, unwrapping
   type only; Field adds field offset) returns `total_offset`, then adds it to the SAME base
   (slot address or pointer value) used by bare-local branches. Pure address arithmetic,
   NO loading or copying bytes.
4. **Borrowck** `Statement::Borrow` (checker.rs ~646): **WHOLE-OBJECT FALLBACK**
   (refuse-over-guess, ordered by G) — if `source.projection` contains `Deref`, loan
   DOES NOT fine-grain by field; anchors onto whole object. **REBORROW CHASE:** compound
   form `(&0 h).name` lowers into TWO Borrows (`tmp = &0 h` followed by sub-borrow
   `tmp.name`); `tmp` is an immediate temporary → stripping back to
   `Place::local(source.local)` would anchor loan to `tmp`, allowing `Drop(h)` to miss
   the active loan → silent dangling pointer escape. Fix: if `source.local` IS the `dest`
   of an active loan (newly borrowed) → inherit the `source` of that loan (chasing back to `h`);
   if it is a PARAM `&0 T` (no originating loan) → use `Place::local(source.local)`.

### Safeguards (Slice 1b) — 383–387, gate `0·0·381·0`

- **383** (`383_ref_field_heap_leaf_sub_borrow.tri`, EXPECT 5) — heap-leaf
  `h.name` (`String`) via `&0 Holder` PARAM → sub-borrow `&0 String` → `length`.
  Path promised by E1049. **Leading `tag: Integer`** shifts `name` to offset≠0 (preventing
  accidental offset-0 masking). Poison JIT (reverting projected-addr) → silent error (reads
  `tag` bit-pattern as `{ptr,len,cap}` → 383 returns 140155117966008).
- **384** (`384_ref_field_nested_scalar_sub_borrow.tri`, EXPECT 7) — chain
  `o.inner.x`: `.inner` sub-borrows `&0 Inner`, `.x` is scalar terminal. Leading
  `pad` on BOTH structs → both offsets≠0. (Operates via Assign scalar-read, NOT
  via Statement::Borrow → poison-JIT does not touch — exercises chain-typecheck +
  offset-accumulation.)
- **385** (`385_ref_field_nested_heap_sub_borrow.tri`, EXPECT 4) — 2-level chain
  to heap: `o.inner.name` → `&0 String`. Poison JIT → 385 returns 2 (incorrect).
- **386** (`386_ref_field_sub_borrow_dangling.tri`, ERROR E2450) — **load-bearing
  user-visible safeguard** (rebuilt from legacy vacuous test, see §b). `bad(dummy: &0
  String) -> &0 String` — param `dummy` allows typecheck to TIE return-borrow to input
  → BYPASSES E2400. Pipeline executes through borrowck; `Drop(h)` while sub-borrow `(&0 h).name`
  remains live and returned → **E2450 (real CLI, exit 3, no other errors — measured twice,
  verified by O independently)**. Poisoning reborrow-chase (checker.rs) → E2450 **disappears AND
  compilation SUCCEEDS** → dangling `&0 String` reaches JIT = real UAF. Chase is **sole-guard
  on path to runtime**, NOT defense-in-depth behind typecheck.
- **387** (`387_ref_field_sub_borrow_move_while_borrowed.tri`, ERROR E2440) —
  POISON move-while-borrowed: `let s=(&0 h).name; let h2=h; length(s)` → moves `h`
  while `s` sub-borrow active → E2440 (uses move instead of mutate since `h.x=` blocked by E0007).
  Poisoning borrowck (removing chase) → E2440 vanishes, compilation passes → dangling `s`.

### Inquiries / Hypotheses (Reported honestly by D)

- **(a) Whole-object false-conflict:** two sub-borrows of DIFFERENT fields via the SAME
  reference (`h.name` and `h.other`) will false-conflict (whole-object loan). This is the
  COST of refuse-over-guess accepted by G. NO valid fixture in existing corpus
  hits this case (no surface exists reading 2 fields concurrently via `&0`).
- **(b) [RESOLVED 2026-07-26, signed by G] Legacy 386 was VACUOUS — replaced:** old 386
  (`bad() -> &0 String` WITHOUT params) failed with **typecheck E2400** (BorrowLifetimeInferenceFailed,
  typecheck/check.rs:532) — FATAL, CLI halted before borrowck (main.rs:58-64), users NEVER
  observed E2450. E2450 only appeared in multi-phase harness (integration_tests.rs:64
  intentionally continuing past fatal typecheck) = **vacuous safeguard**: proved chase *generated* E2450
  even though E2400 independently blocked the dangling pointer ⇒ chase was not sole-guard for that case. **Fix:**
  add `dummy: &0 String` param → typecheck ties return to dummy → bypasses E2400 → E2450 becomes
  the SOLE user-visible safeguard. Legacy §b closed.
- **(c) Vector/HashMap-field stride:** confirmed that both String-fields (fat 24B inline
  → addr of `{ptr,len,cap}` at field offset) AND Vector/HashMap-fields (thin 8B
  handle → addr of handle at field offset) yield correct addresses: JIT SHARES
  `walk_projections` offset + base-addr, independent of stride (returning ADDRESS only,
  without loading). Fixtures 383/385 test String-fields; Vector/HashMap-field share identical
  address pathways without standalone fixtures (narrowed: only String fields tested end-to-end
  with `length`; Vector/HashMap-field sub-borrows lack builtin readers to exercise —
  report to O if supplementary fixtures required).
