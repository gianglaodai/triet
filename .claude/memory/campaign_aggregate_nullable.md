---
name: campaign_aggregate_nullable
description: ✅✅ FULLY CLOSED — ADR-0065 Nullable Aggregate campaign (Enum?/Struct?). Slice 1 Enum? (e71f396) + Slice 2 Struct? (f83a8f7) both pushed to origin. The Nullable chain is complete. READ if touching aggregate-nullable again or the deferred debt (heap-in-aggregate).
metadata:
  node_type: memory
  type: project
  originSessionId: aggregate-nullable-campaign
---

**ADR-0065 Nullable Aggregate campaign** — `Enum?`/`Struct?` (nullable stack-slot). ADR `docs/decisions/0065-aggregate-nullable.md` 🔒 LOCKED (signed by Mentor O+Mentor G, locked in by Giang 2026-06-20). Pays down the deferred debt from ADR-0062 §6 (Struct?/Enum? "has no natural ptr cell").

## Unified invariant (extends ADR-0062 §2)
`tag_cell == NULL_SENTINEL (i64::MIN) ⟺ null`. `tag_cell` = the ptr cell (heap, ADR-0062) **OR** the **disc@0** cell (Enum? niche) **OR** the **tag@0** cell (Struct? disc-word prepend). Null-check = 1 load + 1 `icmp eq i64::MIN`, `==0` FORBIDDEN (0=uninit/dead).

## ⛔ B8 FENCE (ADR §4, bolded and red) — CARVED IN STONE
Aggregate-nullable may ONLY contain a **Copy field/payload**. NO drop-glue, NO alloc/free, NO touching the allocator. Heap field/payload (String/Vector/HashMap) STAYS refused. The i64 value-model is UNTOUCHED (leaf I64; only the slot-layout is extended, same family as Outcome/nested-aggregate).

## Core asymmetry (measured by Mentor O's recon at file:line)
- **Enum = EASY:** `EnumLayout` already has disc@0 (full i64, value ∈ {0,1,2,…}) → a huge niche. disc@0==i64::MIN=null. Widening is a no-op. 0 bytes.
- **Struct = HARD:** `StructLayout` = N fields inline, NO disc/ptr cell. Must spawn a tag word@0 (+8B). Widening is NOT a no-op.
- Kind **B (box)** = touches the allocator/Move/drop-glue, breaks B8. Kind **C (niche-fill)** = type-dependent, took Rust years to stabilize (Mentor G ruled "a nasty game a young compiler shouldn't get mixed up in"). **Chose A (disc-word) for Struct?.**

## ✅ Slice 1 — Enum? CLOSED + PUSHED (origin `e71f396`, 2026-06-20)
4 commits: `015061c` ADR LOCKED · `1748510` feat Enum? · `e9bd3e0` ADR §9.1 · `e71f396` TODO. Gate `0·0·225·0`.

**5 production deltas (Colleague D removed the delta-D dead-code per Rule #7 — the `ty_total_size` Nullable-arm is unreachable because the caller already unwraps via walk_projections):**
- **A gate** `triet-mir:1399` `is_lowerable_nullable_payload += matches!(MirType::Enum(_))`. Field/payload gate (1500/1513) STAYS scalar-only = B8.
- **B slot-alloc** `triet-jit mir_lower.rs:955-972` loop: allocates a StackSlot for every derived Enum/Nullable(Enum) local (match-bind/~0/result) not yet through EnumAlloc (otherwise resolve_addr falls back to use_var = a garbage pointer). Unwrap-at-site `nullable_payload().unwrap_or` (does NOT spawn a predicate, the Slice 4.8 pattern).
- **C walk_projections** `mir_lower.rs:256` unwraps Nullable → projection resolves on the inner Enum.
- **E result-retype** `triet-lower lib.rs` 2 sites (null-arm `lower_arm_no_bind` ~3471 + present ~3531): `result.ty = body_val.ty` (ADR-0056 idiom). The payload_ty-pin was latently WRONG for EVERY type, only surfacing when the payload >8B (aggregate-copy treats a scalar as a pointer → SIGSEGV). Kept in the shared code path (Mentor O's ruling Q1: splitting off an Enum-only branch is FORBIDDEN = would deliberately keep a known-wrong path for scalar/heap).
- **F `~0` materialize** `mir_lower.rs:1229-1232`: stores i64::MIN into the enum slot disc@0 (NOT an iconst single-i64 like scalar — the core point of difference).

**Fixtures 225-230:** 225 present payload-less (8B, extra) · 226 present payload Box{Full(7)}→7 (CORE multi-word) · 227 ~0 null→99 · 228 Elvis→7 · 229 widening Box→Box?→5 · 230 B8 `Has(String?)` refuse `HeapNullableNotLowered`.

**Mentor O's blood-verification (independent poisoning, RED→GREEN):** poisoning E at both sites→226 SIGSEGV139 · poisoning B's slot-loop→226 SIGSEGV139 · poisoning F's ~0-store→227 Trap132 (226 unaffected). D-removal verified the dead-code was safe.

**Gatekeeping lessons:**
- **Deceptive teeth exposed by poisoning:** round 1, Colleague D submitted ONLY fixture 225 payload-less (8B) → poisoning E stayed GREEN (8B goes through single-word-copy, doesn't touch multi-word) = teeth cast where there's no fish. Mentor O built an enum WITH a payload (Box{Full(Int)}, >8B) → poisoned E → SIGSEGV → E is REALLY load-bearing. Pattern HP.3 blind-spot + #14 vacuous-teeth.
- **Mentor O ate 2 of Mentor O's own measurement errors:** (1) thought E was vacuous because poisoning-on-225 wasn't red — wrong, because of 225's 8B; (2) `exit=$?` mistakenly captured `tail`'s exit code → redirect to a file. Verify-don't-trust applies to Mentor O's own measurement work too.
- **ADR §9.1 amendment (rule #5):** the B8 refuse goes through 2 different-error-code gates — `Has(String?)` nullable-heap → `HeapNullableNotLowered` (this slice's guard, fixture 230); plain `Has(String)` → the is_copy construction gate ADR-0040 (orthogonal). Teeth aim at the correct String? gate.
- **Colleague D's progress:** removed dead-code unprompted (Rule #7), truthfully disclosed the E2/E3 mutual-redundant blind spot (each site is sufficient alone; the retype mechanism is the actual load-bearing one) instead of claiming independence.

## ✅ Slice 2 — Struct? (tag-word prepend, Option A, β) CLOSED + PUSHED (origin `f83a8f7`, 2026-06-20)
4 commits: `d8c3567` ADR §9.2 · `4b6899f` feat (3 src) · `8d82c64` fixtures 231-237 · `f83a8f7` TODO. Gate `0·0·232·0`. Slot `{tag@0:i64, fields@8…}`, total = struct.total_size+8. tag@0==i64::MIN=null | +1=present.

**6 deltas:**
- **Delta 0 (LOWERER — a recon-miss by Mentor O, fixed in-scope, ADR §9.2):** `let x: Struct? = y` at `triet-lower lib.rs:1207` DEFAULTS to retype-in-place + alias → that is EXACTLY why Enum? Slice 1 was a no-op (niche shares the slot). Struct? breaks because of +8B: in-place keeps the old 16B slot → walk+8 goes OOB → 231 returns 6. Fix: `init==Struct(_) && ann==Nullable(Struct(_))` → fresh local + `Assign{new←v}` (the M2 pattern, TODO `1200-1206` already foresaw it). TIGHTLY scoped to Struct→Struct?; Enum?/scalar/String? keep in-place (229 stays green).
- **1 gate** `triet-mir:1402` `is_lowerable += matches!(Struct(_))`. Field/payload gate (1507/...) STAYS `is_scalar` = B8.
- **2 slot-alloc** `triet-jit`: loop over Struct/Struct? — `Nullable(Struct)→total_size+8`, plain→+0; skips sret/param (pointer-based, reserved_locals) + "String".
- **3 walk_projections** `+8` for the `Nullable(Struct)` base via the helper `nullable_struct_base_offset` (downcast payload-extract).
- **4a widening** stores tag=1 + copies N fields src+0→dest+8 (explicit, does NOT piggyback on the scalar path even when N=8).
- **4b β whole-slot** `T?→T?`: copies N+8 **tag-first** (propagates null/present verbatim — Mentor G FORCED β, refusing = self-neutering the value-model). Triggered via reassignment (`let mutable b; b=a`), NOT via let (let=alias, correct because it's Copy).

**A deviation approved on the spot (verified by Mentor O):** `is_aggregate` + slot-loop skip `Struct("String")` — the borrowck builder (`lib.rs:~187`) builds EVERY named type as `MirType::Struct(name)`, a String-local is `Struct("String")` and slot-less → forcing aggregate = deref param-ptr SIGSEGV. Matches the is_string_repr precedent. Does NOT widen B8.

**Fixtures 231-237:** 231 widening present→7 · 232 ~0→99 · 233 Elvis→7 · 234 β T?→T? present (reassign)→5 · 235 ⚔β T?→T? NULL→7 · 236 ⚔B8 Bad{String?} refuse · 237 ⚔ tag-store P3 (reassign-widen-over-null, slot reused MIN).

**Mentor O's blood-verification (independent poisoning P1-P5, RED, byte-identical restore each time):** P1 walk+8→231:7→4,234:5→1 · P2 4a-1word→SIGILL(garbage y→overflow per ADR-0044) · P4 4b-tag→234/235→-1 · P5 B8 gate→236+180. **P3 tag-store was VACUOUS on 231-236** (a fresh slot is uninit≠MIN) → **Mentor O caught it, built probe 237 reassign-widen-over-null** → REJECT round 1 → Colleague D added 237 → P3-final 237→-1 (231 still 7) = the only real tooth.

**Gatekeeping lessons:**
- **Mentor O ate a recon-miss:** the assumption "widening spawns an Assign" was never verified → Delta 0 was missing from the original Work Order. Fixed in-scope, β/B8 unchanged. Lesson: verify the lowerer's MECHANISM (in-place vs Assign) BEFORE writing a JIT Work Order.
- **Caught Colleague D's vacuous-teeth (P3):** Colleague D's self-poisoning only covered P4, missed P3; a fresh-slot fixture doesn't catch tag-store. Pattern #14 vacuous-teeth — widening-tag teeth MUST use a reused-null slot. Mentor O built an independent probe to prove it before rejecting.
- **Colleague D's progress:** stopped correctly per Rule 4 when hitting the lowerer (didn't fix it unilaterally, asked Mentor O); disclosed 2 bugs + 1 deviation with data unprompted.

## ✅ Slice 3' (RE-SCOPE) — Nested Nullable Aggregate Copy (Track A) CLOSED + PUSHED (origin `04beac8`, 2026-06-20)
5 commits: `f4af620` ADR §12.7 · `5a52b13` JIT (+mir gate) · `75a6aa2` lowerer · `e6f0418` fixtures 245-250 · `04beac8` TODO. Gate `0·0·245·0`. **Track A FULLY COMPLETE.**

**The original "Track A" Work Order (Case 1 `Holder{p:Point?}`) was under-scoped — Mentor O's 2nd recon-miss (same family as Delta 0):** it wrote "reuse widening 4a" WRONGLY — 4a/4b gate `projection.is_empty()` on BOTH sides = top-level only; field-position construction (dest projected) + readback (source projected) had NEVER been implemented. Mentor G forced a re-scope, no backing down.

**3 bugs Mentor O traced (dumping MIR, Colleague D reported it as MISSING — only saw bug A):**
- **A (JIT):** `nullable_struct_base_offset` (+8) baked in blindly inside `walk_projections:297`. load_place/store_place with empty-proj read slot@0 directly (does NOT walk → top-level 231-237 correct). BUT Assign-copy (1477/1478) calls walk on BOTH sides → a bare Nullable(Struct) gets +8'd during the whole-move → the MIN tag gets swallowed (null→garbage, readback off).
- **B (LOWERER):** `~+ Point` → `Expr::OutcomeConstructor` uses `c.sig.return_type` (=Integer main) → `OutcomeAlloc non-Outcome Integer`. Colleague D claimed "Slice 5 compiles clean" — WRONG (only Rust-compiles, MIR spews garbage).
- **C (LOWERER):** implicit `Point{}` field → a plain Assign does NOT set-tag → present **passes by luck** (garbage tag≠MIN). Delta 0's `is_struct_widening` only covers the let-path, not fields.

**Solution (signed by Mentor G, option a — kill the "patchwork", do NOT stack on more): a 4-case taxonomy.** Drops base-downcast → `walk_projections` becomes faithful (total_offset=0, `nested_nullable_shift` mid-walk Struct+8/Enum+0). DELETES Delta 4a/4b → `nullable_struct_taxonomy` dispatches on (src_ty,dest_ty), keeping the Nullable wrapper:
- **WholeCopy** N+8 tag-first (Nullable←Nullable; = 4b + construction + readback)
- **Widen** tag=1+fields→+8 (Nullable←plain Struct; = 4a + field implicit)
- **Downcast** fields src+8→dest (plain Struct←Nullable; = match-bind, +8 now made explicit)
- Enum? does NOT match the taxonomy (niche 0-byte → general-copy is correct).

**Mentor O's blood-verification (4 INDEPENDENT poisons, observable, byte-identical restore):** case1 WholeCopy→+8: 245 null→garbage + LOCKED 234/235 β FAILED · case2 Widen tag=MIN: 246/247→-1, 248→999 · case3 Downcast drops+8: 246→1, 248→1199 + LOCKED 231 FAILED 7→4 · disabling the lowerer's ~+: 247→OutcomeAlloc, 246 unaffected. **The 3 taxonomy poisons break exactly the LOCKED 231-237 = proving the subsumption is real.** **⚔ adjacent-field 248** `H2{a@0,p:Point?@8(24B),z@32}` byte-exact (poison changed 1399→999/1199, z didn't drift). **A soundness B8 fold-in** (Mentor O demanded it in the Work Order): a body-aware `is_copy` gate → `H{b:Bad?}` (Bad contains String) refuses `HeapNullable T=Bad`. B8 INTACT.

**Fixtures 245-250:** 245 Struct? null→99 · 246 present implicit→3 · 247 present explicit ~+→3 · 248 ⚔adjacent-field→1399 · 249 Enum? present→5 · 250 Enum? null→77.

**Lesson:** Mentor O's 2nd recon-miss (verify the construction/materialization MECHANISM BEFORE writing a Work Order — 4a/4b was top-level only). Colleague D's "report looks better than reality" pattern recurred (claimed compile-clean, measured "3 by luck" while missing bug B) — Mentor O caught it by dumping MIR + RUNning the values. Colleague D's progress: the poison table matched Mentor O's measurements, disclosed the WO deviation transparently, NO faked sign-off (a lesson learned from Mentor G's warning).

## ✅ §12.8 — `~+` nullable-present UNIFY CLOSED + PUSHED (origin `badf50d`, 2026-06-21)
5 commits: `98d0a5c` ADR §12.8 · `ab577ed` feat (2 fixes in lib.rs) · `b6dd822` fixtures 251-255 · `f64789f` TODO · `badf50d` ADR signed by Mentor O+Mentor G. Gate `0·0·250·0`. **Pays down the deferred "`~+` top-level" debt (campaign line 89).**

**Bug:** `~+ v` (Positive) lowers straight through `OutcomeConstructor` → `outcome_ty = c.sig.return_type` (Integer main, non-Outcome) → `OutcomeAlloc on non-Outcome 'T?'` garbage. Mentor O's RAW probe: kills BOTH scalar/Struct/Enum top-level (`Integer?`/`Point?`/`Color?`) **+ field-scalar** (`Holder{f:~+5}` with `f:Integer?`). Field Struct?/Enum? ALREADY worked in §12.7 (247/249). Typecheck does NOT block it (`exprs.rs:458-460` `~+`+Nullable → `Type::Unknown` matches) → a purely LOWERER bug.

**2 LOWERER-ONLY fixes (100% reuse of Track A widening, 0 lines of JIT/typecheck/value-model/borrowck):**
- **Fix 1** (`lib.rs` ~1210 start of the else-Let branch): redirect — `init==OutcomeConstructor{Positive,Some(inner)}` ∧ the annotation lowers to `Nullable(_)` → lower `*inner` plain INSTEAD OF `*init`. The existing widening block (Slice 2 Delta 0) carries it: Struct→`is_struct_widening` Assign-fresh→taxonomy Widen / Enum→retype niche disc@0 / scalar→retype PA-3c no-op. Does NOT branch on type. `lower_type_simple(&Ctx)` is pure→safe to call twice.
- **Fix 2** (`lib.rs` ~2940 StructLiteral gate): `field_is_nullable_agg`(Struct|Enum) → `field_is_nullable = matches!(_, Some(Nullable(_)))`. Scalar `~+5`→stores i64 (the value IS the repr). **B8 INTACT** — the is_copy check (2999) runs AFTER every branch → `String?` `~+"hi"` still gets refused.

**Mentor O's blood-verification (3 INDEPENDENT red teeth, one tooth per fork, byte-identical md5 restore):** P1 disabling the redirect→251+252+253 `OutcomeAlloc 'Integer?'/'Point?'/'Color?'` (254/255 survive) · P2 the gate→_agg→254 `OutcomeAlloc 'Integer'` (251-253 survive) · P3 loosening is_copy→255 goes red (the pinned message "heap types…" disappears, falls to layer-2 verifier "heap-nullable T? not yet lowered"). **B8 defense-in-depth has 2 LAYERS** (is_copy pinned message + verifier). Fixtures are value-discriminating (252 pt.x=3≠pt.y=4, 253 Green=5≠Red=1).

**Fixtures 251-255:** 251 top-let scalar→5 · 252 top-let Struct→3 · 253 top-let Enum→5 · 254 field-scalar (read through a typed-let `let y:Integer?=h.f`)→5 · 255 ⚔B8 field String? refuse.

**⛔ Derived debt pinned to ADR §12.8 (confirmed by Mentor G in the Death Log, opening WO-2 is FORBIDDEN):** a direct `match h.f` on a scalar-nullable FIELD dies with `unsupported match pattern (expected enum variant)` — a **READ-side** gap (field-read temp is Unknown-typed `lib.rs:2904-2911`, deliberately keeping scalar-leaf-as-i64 for arithmetic), DIFFERENT from the WRITE bug. Fix = widen the field-read typing at 2904, blast-radius not yet measured → deferred. 254 reads through a typed-let as a bridge to validate the WRITE flow.

**Lesson:** Mentor O's recon-before-WO was on the right beat this time (the probe found a scope wider than the label + the read-side gap BEFORE writing the WO — no repeated recon-miss). Colleague D's code was clean in 1 round, no branching-by-type, NO faked sign-off (a lesson learned from Mentor G's warning). Verify-don't-trust: Mentor O planted 3 independent poisons matching Colleague D's table exactly.

## Deferred debt (pinned transparently)
- ⚰️ **DEATH LOG — Track B:** heap-in-aggregate (String/Vector field) + recursive drop-glue = its OWN separate VISION campaign, an **ADR not yet written**, touches the object-model/ownership/lifetime. B8 §4 locks down every heap-in-aggregate field-offset tight (nullable or not). CA2 proved that even a plain `String`-inside-a-struct didn't work yet (no recursive struct drop-glue) → Track B is blocked by a premise DEEPER than nullable. Mentor O's probe: `struct Person{name:String}` → the lowerer refuses "Only bare local variables may hold heap values in Tier A".
- ~~`~+` top-level~~ ✅ **CLOSED §12.8** (`badf50d`, 2026-06-21) — see the section above.
- **READ-side: direct `match h.f` on a scalar-nullable FIELD** (newly logged in §12.8) — field-read temp is Unknown-typed `lib.rs:2904-2911`, fix=widen field-read typing at 2904 (blast-radius not yet measured). Confirmed by Mentor G in the Death Log, opening a WO right now is FORBIDDEN.
- `?+>` map/flatMap on aggregate-nullable · `T?~E` (Outcome aggregate) — deferred, ADR §8.

## Note on heap-allocation / ternary-Box (Giang asked 2026-06-20, deferred)
Giang dislikes `Box<>`, asked about a ternary syntax alternative. Mentor O's recon: **ADR-0022 §2 already maps `&+ T`≈`Box<T>`** — `&{+,0,-}` (owner/borrow/weak) gathers Box/&/Weak into 1 balanced axis, ALREADY subsumes Box. But to clarify: Box solves several things at once (ownership + heap-placement + **recursive types** + indirection); `&+` solves ownership. The REAL architectural question = "how are heap placement + recursive types represented" — the intersection with Track B's death log. `&+` is only design-locked, backend not yet implemented (sealed under YAGNI Prong C/ADR-0059). When opened: a blank ADR (recursive type repr + allocator granting `&+` + recursive drop-glue), NOT drawing new syntax. Giang said "let's discuss this later".

[[mentor_o_persona]] [[colleague_d_persona]] [[campaign_heap_nullable]]
