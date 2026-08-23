# ADR 0067 — Axis B Slice 2: Nested-Flat & Enum-Payload Heap Drop-glue (No-Box)

> # ⚖️🩸 IRON LAW OF SOUNDNESS (inherited from ADR-0066 — remains in full effect)
> # `byte-copy` ⟶ `tombstone-source` MUST BE ATOMIC WITHIN THE SAME BASIC BLOCK.
> All moves of aggregates containing heap (nested or enum-payload) must comply with the IRON LAW:
> copy + tombstone immediately adjacent, with no panics/CFG-branches/calls interposed. (Carved in stone by G, 2026-06-21.)

**Status:** Proposed (clean-room scaffold — pre-recon completed, NO code yet; awaiting G's signature per step).
Applicable to Tier C+. Extends heap-in-aggregate from **single-level FLAT** (ADR-0066 Slice 1) to
**nested non-recursive (bounded)** + **enum-payload heap** — **WITHOUT boxes, WITHOUT recursion**.

**Issue:** ADR-0066 Slice 1 unlocked `struct{ f: String }` (DIRECT heap leaves, single-level FLAT).
The M-2 barrier (`lib.rs:~3052`) still **refuses transitive types** — `struct{ inner: HasHeap }` (where HasHeap
contains String) is blocked at construction; GP-1 drop-glue (`mir_lower.rs:1728`) only walks ONE level, filtering on
`is_any_heap()` (= String/Vector/HashMap, EXCLUDING Struct), thus skipping struct-typed fields → leaks.
Enum-payload heap lacks drop-glue (the Drop handler only covers `MirType::Struct`; enum heap only handles 2-arm
Outcome). This is a physical layer inheriting 100% of Slice 1 foundations — WITHOUT touching allocator/box.

**Related ADRs:** Inherits ADR-0066 (GP-1 inline drop-glue + GP-2 copy-then-tombstone + M-1/M-2).
Generalizes `emit_outcome_drop_glue` (ADR-0057, 2-arm) → N-arm for enums. Nested layout:
ADR-0060 (size fixup for aggregate fields). Box `&+` + true recursion → **ADR-0068 (deferred)**.

---

## Decision (scaffold — details locked per recon step)

Extend B8 for **bounded heap-in-aggregate without boxes** in 2 steps:

### Step 2a — Nested non-recursive heap-in-struct (bounded recursive drop-glue)
`struct Holder { inner: HasHeap }` (HasHeap contains String/Vector/HashMap, NOT self-referential):
- **Relax M-2:** allow Struct/Enum typed fields containing **transitive** heap (NOT self-referential/recursive —
  already blocked by typecheck + deferred to ADR-0068). Continue refusing box/`&+`.
- **GP-1 → STATIC recursive drop-glue:** walk layout recursively at compile-time; each struct-containing-heap
  field → recurse into its layout, **accumulating offset**; each heap LEAF → emit free at absolute offset.
  Depth = STATIC nesting (compile-time, struct graph is a DAG because recursion is forbidden) → **NO runtime
  recursion, NO stack overflow**.

#### 🔒 Step 2a — IMPLEMENTED (awaiting O blood verification + signature; D does NOT self-sign)
- **2a-1 M-2 relaxation (lib.rs ~3061):** add `is_nested_struct` = bare `Struct` whose layout resolves → ALLOW.
  **ONLY Struct, NOT Enum** (intentionally narrowed from WO's wording "Struct/Enum"): `collect_heap_leaves`
  only recurses on Structs; enum-payload heap is tag-dependent → Step 2b. Copy-enums continue passing via `ctx_is_copy`;
  heap-enum fields REMAIN refused (forcing Step 2b to handle enum drop-glue). `Nullable(heap)` (ADR-0062) + box/`&+` REMAIN refused.
- **2a-2 `collect_heap_leaves` (mir_lower.rs):** compile-time associated fn, recurses on Structs, accumulates
  absolute offset, returns flat `Vec<(i32, MirType)>`. **Depth-limit 64 → JitError** (guard against stack overflow).
  **SHARED between Drop + Deinit** (Life-Death symmetry mandated by G): Drop `emit_heap_free_at(copy_base_addr(local,abs))`;
  Deinit `stack_store(0, slot, abs)`. Single-level FLAT (Slice 1) = depth-0 case → stays passing.
- **2a-3 Nested move:** reuses 1b/1c (aggregate byte-copy total_size>8 + 2a-2 recursive Deinit). 0 new lines.
- **Safeguards (verified by O):** R-leak-nested (collect non-recursive → Drop Unsupported, refuse > leak) · R-double-free-nested
  (single-level Deinit → FREE_COUNT==2 double-free) · R-recursive-creep (removing depth-limit → stack-overflow SIGABRT).
  Fixtures 263/264/265, counting `struct_nested_heap_counting`, unit `collect_heap_leaves_recursive`.
- **Fixture 257 FLIP** (RULE 3, signed off by O): 1d negative `Outer{inner:Inner}` refuse → 2a unlock → EXPECT.

### Step 2b — Enum-payload heap (tag-switch drop-glue N-variant)
`enum E { A(String), B(Integer), C }`:
- **Construction:** lift enum-payload-heap refusal (lib.rs:1890).
- **Drop-glue:** generalize `emit_outcome_drop_glue` (2-arm) → **N-arm tag-switch**: read disc →
  switch → free heap payload of the ACTIVE variant (leaving garbage of inactive variants untouched). No-op for unit/scalar
  variants.

#### 🔒 Step 2b — IMPLEMENTED (awaiting O blood verification + signature; D does NOT self-sign)
**Pre-recon caught payload-layout gap (D, analog of 1a STEP-4; O verified + ruled IN-SCOPE):** enum payload size
hardcoded to 8 (lib.rs:603) → String-payload slot is 16B; construction only stores ptr@8 (STEP-4 fat-sync only touched
`struct_slots`); drop-glue reads cap@payload+16 = slot+24 = OOB on 16B slot → UB. Vector/HashMap (thin 8B) have NO gap.
→ Add 2 auxiliary pieces:
- **2b-0a (lib.rs:603):** heap-aware enum payload size — `String→24`, Vector/HashMap/scalar→8. String-payload
  `total_size = 8+24 = 32` → slot accommodates `{disc@0, ptr@8, len@16, cap@24}`. (M-1 struct-fixup DOES NOT touch enums — sole site.)
- **2b-0b (mir_lower.rs Assign):** fat-store String payload into `enum_slots` (analogous to STEP-4): copy len@payload+8/cap@payload+16 from src String-slot, reading src BEFORE M1-zeroing.
- **2b-1 (lib.rs gate EnumLiteral + EnumVariant-call):** `is_direct_heap_leaf || ctx_is_copy` → allow leaf/Copy;
  refuse struct-transitive-heap (collect-in-arm = 2b+) + Nullable(heap). EnumLiteral path previously lacked gate.
- **2b-2 (`emit_enum_drop_glue`):** N-arm `brif` chain — read disc@0, each heap-payload variant → arm `icmp disc==dv`
  → `emit_heap_free_at(stack_addr(slot, payload_off=8), variant.payload.ty)`; scalar/unit variants DO NOT emit arms.
- **2b-3 (Deinit):** tombstone zeros ONLY payload ptr@8, **DOES NOT touch disc@0** (disc=0 is a VALID variant — unlike Outcome).
- **Safeguards (verified by O):** R-enum-leak (str count 0) · R-enum-double-free-move (count 2) · **⚔ R-enum-wrong-variant**
  (`Pair{Text(String),Buf(Vector)}` — cross-wiring → calls wrong shim → per-type counts wrong; Buf→vec=1/str=0, Text→str=1/vec=0) ·
  **R-enum-cap** (poisoning 2b-0b → corrupt cap ≠ 5). Fixtures 266/267/268, counting `enum_heap_payload_counting`.
- **~~Deferred 2b+~~ → AMENDED below (opened under G signature 2026-06-24):** enum-in-struct-field now resolved in §2b+.
  payload-struct-containing-heap (recursive collect WITHIN enum arms) remains deferred (no current use case).

---

## ✚ AMEND §2b+ — Enum-in-Struct field (No-Box bridge, opened with G signature 2026-06-24)

**Objective:** `struct Wrapper { msg: Msg }` (with `Msg.Text(String)`) construct + move + drop SOUND,
FREE_COUNT==1. Complete NO-BOX — plugging the hole of enums nested inside structs.

### Recon (Measured by O — carved in stone)
- **HEAD = CLEAN REFUSAL, NO silent leaks.** `lib.rs:3107` gate blocks construction (`ctx_is_copy(Enum Msg)`
  in lib.rs:1013-1022 already recurses correctly → returns `false` for heap-payload variants → non-Copy field → REJECT
  `heap_type_not_supported`). Probing `triet-driver probe.tri` → clean exit + diagnostic. No OOM hazard
  at HEAD; leaks only occur IF gate is relaxed WITHOUT bridging drop-glue.
- **3 blind spots:** (A) `mir_lower.rs:446` `collect_heap_leaves` skips enum fields (`_ => {}`);
  (B) `mir_lower.rs:1028` `emit_enum_drop_glue` is **slot-based** (requires local typed Enum + `enum_slots.get(local)`)
  → cannot be invoked for an enum nested INSIDE a struct (no standalone enum_slot); (C) `lib.rs:3107` gate refuses.

### Core Architectural Problem
Current leaf list `Vec<(i32, MirType)>` is **STATIC** (unconditional free, compile-time offset). Enum-drop is
**DYNAMIC** (runtime tag-switch — freeing only active variant). Enum CANNOT be flattened into a static leaf list →
infrastructure branching is MANDATORY.

### Bridge Design (4 pieces)
- **2b+-A `LeafKind`:** change leaf list `Vec<(i32, MirType)>` → `Vec<(i32, LeafKind)>` with
  `LeafKind::Heap(MirType)` | `LeafKind::Enum(String)`. `collect_heap_leaves:446` pushes `(abs, Enum(name))`
  for enum fields. **DOES NOT recurse into enum** (payload is tag-dependent — runtime).
- **2b+-B address-based core:** extract `emit_enum_drop_glue_at(builder, body, enum_name, base_addr)` reading
  disc@`base_addr+0`, payload@`base_addr+8`. Slot-based form becomes thin wrapper (`base_addr=stack_addr(slot,0)`
  → invokes core). **2b top-level REMAINS byte-identical** (mandatory regression teeth — 266/267/268 + counting).
- **2b+-C consumer dispatch:** Drop (mir_lower.rs:1880) loops over leaves → `Heap`→`emit_heap_free_at(base+abs)`;
  `Enum`→`emit_enum_drop_glue_at(copy_base_addr(local,abs), name)`. Deinit (mir_lower.rs:1479) → `Heap` zeros
  ptr@abs (as before); `Enum` zeros payload word @`abs+8` **STATICALLY** (ptr=0 → free is no-op regardless of disc;
  **DOES NOT touch disc@abs+0** — rule 2b-3).
- **2b+-D gate (lib.rs:3107):** add `is_nested_enum = matches!(decl_ty, Enum(n) if c.enum_layouts.contains_key(n))`
  in parallel with `is_nested_struct`. Self-ref blocked upstream in typecheck; depth-64 safety net maintained.

### ⚠️ CRITICAL HAZARDS (offset + fat-store — O warned, G underlined)
- **Offsets match by construction:** compile-time absolute offset (`collect` accumulates `base+f.offset`) == runtime
  offset (`copy_base_addr(local,abs)=stack_addr(slot,0)+abs`) — SAME `layout.fields[].offset`. Disc@base+abs+0,
  payload@base+abs+8. **1-byte mismatch = SIGSEGV.**
- **Fat-store landmine (analogous to 1a-STEP4 / 2b-0b):** relaxing gate → construction `_0.msg = move _1` MUST copy
  **FULL enum width** (disc+payload, 32B if String-payload), not merely 8B. Narrow 8B store path → drop reads
  GARBAGE disc/cap → SIGSEGV. **MUST audit store path + `total_size` of struct containing enum fields (from `enum_layouts.total_size`,
  NOT `_=>8`) — D neglecting this = REJECT.**

### Safeguards (counting FREE_COUNT — poison BRIDGE, do not poison HEAD)
- **R-enum-in-struct-leak** (ISOLATION): stub line 446 to skip pushing Enum leaf → FREE_COUNT==0 (leak) FAILS; baseline==1.
- **⚔ R-wrong-variant:** ignore disc in core → free wrong variant → per-type count wrong / corrupted cap FAILS.
- **R-double-free-move:** `let w2=w` stubbing Deinit enum-field tombstone → FREE_COUNT==2 FAILS.
- **R-fat-store-cap:** instrument actual cap (==len) vs garbage → catches truncated 8B store path.
- **Regression 2b:** 266/267/268 + counting PASS (address-based refactor does not break top-level).

### Deferred (remains closed — no current use case)
- payload-struct-containing-heap (`enum Msg { Rec(Wrapper) }` with Wrapper containing String) — recursive collect INSIDE arm.
- True-recursive/box → ADR-0068.

### Signatures §2b+ (CLOSED 2026-06-25)
O ✅ (verified 4 independent poison safeguards failing: fatal line #2→SIGABRT134 · R-leak→Drop-Unsupported ·
⚔R-wrong-variant→2 fails · R-double-free-move→count≠1; 2b byte-identical) · G ✅ co-signed.
**🏁 NO-BOX (ADR-0067 2a+2b+2b+) FULLY CLOSED.** Latent debt noted for `Nullable(Enum)` sizing
arm (struct_map→8, correct-now, synchronized when opening ADR-0062 §6).

---

## ⛔ DEFERRED — Moved to ADR-0068 (Slice 3 — Box Campaign, dedicated foundational initiative)
- **2c True-recursive types** (`struct Node { next: &+ Node }` / `(&+ Node)?`): requires `&+` heap-box
  backend (allocator allocation + box-drop) — DOES NOT YET EXIST (only MirType variant + S6 borrowck, sealed under
  YAGNI ADR-0059).
- **#0 Typecheck self-ref** (`resolve_type` check.rs:1020 → self-ref `Node` raises UnknownType): patched
  alongside 2c (self-ref only valid WHEN routed through box/indirection).
- **Iterative drop to prevent stack overflow:** deep linked-list/tree → recursive runtime drop blows stack → requires
  iterative (pointer-following loop) or bounded depth. Major ABI decision belonging to ADR-0068.

---

## Alternatives Considered
(locked per recon step — scaffold)

## Consequences
### Positive
- Unlocks real-world nested records (`struct Person { name: String, address: Address }`).
- Enum sum-types containing heap (`Result`-like, AST nodes) — foundation for all structured data.
- Inherits 100% of Slice 1 foundation, WITHOUT touching allocator/value-model.

### Risks to Mitigate
- **R-leak-nested:** recursive drop-glue misses a level → leak. Safeguard: 2-level nesting, poison recursion → FREE < N.
- **R-enum-wrong-variant:** tag-switch frees incorrect variant → freeing garbage / double-free. Safeguard: poison switch.
- **R-recursive-creep:** self-ref slips past M-2 (typecheck miss) → compile-time recursive drop-glue loops infinitely. Safeguard: self-ref MUST be refused.

## Effective Date
- Tier C Slice 2: 2a nested-flat + 2b enum-payload (no-box).
- Deferred to ADR-0068: 2c true-recursive + box + iterative-drop + #0 typecheck self-ref.

---

**Signatures ADR-0067:** (scaffold — awaiting recon per step + G signature)

---

## ✚ AMEND — Construction-into-field source tombstone (double-free)

**Coverage hole.** §2a/2b+ only tested *inline* construction (fixtures 263/264 —
`Outer { inner: Inner { ... } }`): inner aggregate is a TEMP, lacking scope-end Drop,
thus avoiding double-free. Construct-from-*named-local* was never tested:

```triet
let i = Inner { name: n };  let h = Holder { inner: i, tag: 5 };   // struct-payload
let m = Msg::Text("hi");    let w = Wrapper { msg: m, tag: 5 };    // enum-payload
```

→ live UB double-free at HEAD (both struct- and enum-payload), clean in release mode, **exit 134**.

**Root cause.** Two layers:
- Producer `triet-lower/src/lib.rs` (StructLiteral): emits `_d.field = move field_val`
  but DOES NOT emit `Deinit(field_val)` when `field_val` is a local struct/enum containing
  heap. Compare with 1c (lib.rs:1395), arg-move (2427/2474/2511) — all of which DO Deinit
  the source; only construction-into-field was missed.
- JIT `mir_lower.rs:1759` aggregate byte-copy DOES NOT tombstone source (false assumption
  `// Struct/enum types are Copy in Tier A` — struct/enum containing heap = **Move**).
  Source slot keeps heap pointer live → `Drop(source) + Drop(dest)` = double-free.

**Fix (Option A — lower-side Deinit, signed by G).** Lower emits `Deinit(field_val)` IMMEDIATELY
AFTER field-Assign when `is_nested_struct || is_nested_enum`, **atomic within same BB**, inserting
nothing between. Reuses proven JIT Deinit recursive-tombstone (struct
`collect_heap_leaves`; enum 2b-3 zero payload ptr). Scalar heap-leaf fields retain existing
JIT M1-zeroing path (untouched). NO edits to JIT/borrowck. Option B
(implicit tombstone in JIT) was REJECTED by G.

**Safeguards** (set independently by O, outside D's scope):
- **R-construct-from-local:** stub Deinit → SIGABRT/exit 134 or FREE==2 fails; restore → passes.
- **R-atomic:** MIR-dump greps `Deinit` immediately following field-`Assign`, within same BB.

---

## ✚ AMEND-2 — Enum-Payload-Aggregate Sizing (submitted by D, awaiting O verify + G sign)

**Coverage hole.** §2b-0a sizing enum payload only had 2 branches: heap-leaf (`String`/
`String?` = 24B) and everything else = **hardcoded 8B**, including an **aggregate**
payload (`Struct`/`Enum`) > 8B. The `struct{ inner: Enum }` field-fixup loop
(§ADR-0060 P2, `lib.rs:~574`) intentionally **did not touch enum payloads** (original comment:
"the M-1 struct-field fixup loop does NOT touch enum payloads") — making the hardcoded 8B site
the **SOLE site** sizing enum payloads, with no secondary fixup mechanism.

**Blood proof (O, prior to WO assignment).** `enum MyEnum(Big)` with `Big{p,q}`
(16B) — 2 adjacent `MyEnum` values in `struct Pair` — poisoned (at the exact hardcoded 8B site):
**SIGILL 132** (Cranelift `trapnz` caught garbage overflowing from 16B payload overwriting
adjacent 8B slot). Control (exact 8B payload): 204 clean. Absolute isolation: sizing
is the culprit, not HashMap/hash-walker.

**Root cause.** `triet-lower/src/lib.rs:551` (original version prior to fix):
```rust
let size = if ty.is_string_repr() { 24 } else { 8 };
```
No branches existed for `MirType::Struct`/`MirType::Enum`/`MirType::Nullable(Struct|Enum)`.

**Fix (submitted by D).** CO-FIXPOINT struct+enum in a SINGLE loop
(`triet-lower/src/lib.rs`, new `resolve_aggregate_size` function + merged `loop`):
- Enum payload pass: `MirType::Struct(name)` → `struct_map[name].total_size`;
  `MirType::Enum(name)` → `enum_map[name].total_size` (recursive — enum-in-enum
  converges over iterations); `String`/`String?` = 24B; `Vector`/`HashMap` =
  8B handle; `Nullable(Struct|Enum)` retains existing struct-field-fixup rules
  (symmetric, UNCHANGED).
- Struct field pass: legacy logic (ADR-0060 P2 / ADR-0067 2b+), now SHARING
  `resolve_aggregate_size` with enum pass — eliminating diverging duplicate implementations.
- Gauss-Seidel: each enum pass iteration sees newest `struct_map`, and struct pass
  sees newest `enum_map` in the SAME iteration; converges because sizes grow
  monotonically and type graph is a finite DAG (ADR-0068 forbids Box/recursion). Cap
  `FIXPOINT_ITERATION_LIMIT = 64` → `Err(LowerError)` if exceeded (no panic).
- **ABI UNCHANGED** — `EnumLayout::compute` (`triet-mir/src/lib.rs`) remains
  `{disc@0, payload@8, total_size = 8 + max_payload_size}`; only the incoming
  `max_payload_size` value changes.
- **Copy-only, preserving heap refusal.** `Expr::EnumLiteral` construction gate
  (`lib.rs` — `is_direct_heap_leaf || ctx_is_copy`) UNTOUCHED — an aggregate payload
  containing `String`/`Vector`/`HashMap` (neither direct-leaf nor Copy) remains REFUSED
  at construction. This WO purely handles sizing for already-permitted paths (Copy-aggregates).
- **Lift E1048.** `triet-typecheck/src/types.rs`
  `Type::is_hashable_enum_payload` (previously: scalar+String only, matching old
  sizing limitation) now delegates to `is_hashable_leaf` (recursive
  `UserStruct`/`UserEnum` field-by-field) — an enum-variant payload aggregate
  is now hashable like any other leaf. Synchronously remove defense-in-depth guard in
  JIT (`triet-jit/src/mir_lower.rs::enum_payload_variants`, hard refusal on
  `Struct`/`Enum` payloads) — `emit_key_hash_value`/`emit_key_eq_value` walkers
  already recurse generally into `MirType::Struct`/`MirType::Enum` (used for
  struct-keys and enum-as-struct-leaf, ADR-0083 §AMEND-1), so removing guard suffices
  without new walkers.

**Safeguards** (submitted by D, fixtures in `crates/triet-driver/tests/fixtures/`):
368 (struct payload 16B, 2 adjacent enums in struct — poison 132 confirmed by D) ·
369 (`Vector<EnumAggregate>` push+get roundtrip, stride) · 370/360
(`HashMap<EnumAggregate,_>` key roundtrip — 360 was OLD E1048 fixture, now SUPERSEDED
to positive since this WO unlocks it) · 371
(`HashMap<_,EnumAggregate>` non-destructive get-by-value) · 372 (enum-in-enum,
2-level self-convergence) · 373 (negative control: heap-bearing payload still REFUSED,
no regression) · 173 (regression, EXPECT 30 retained).

**⚠️ Out-of-scope finding (reported to O, NOT fixed in this WO):** a `match` nested
SYNTACTICALLY inside another `match` arm (`match a { X => match b {...}, ... }`,
or `~+ e => match e {...}` in Outcome match) triggers Cranelift backend error —
**not a Triet MIR verifier bug** (`body.verify()` passes cleanly; error resides in
`triet-jit` codegen during function definition). Minimal reproduction WITHOUT
enum-payload/aggregate (2 disjoint unit-only enums):
```triet
enum A { X, Y }
enum B { P, Q }
function main() -> Integer {
    let a = A::X; let b = B::P;
    return match a { A::X => match b { B::P => 1, B::Q => 2 }, A::Y => 3 };
}
```
→ `VerifierError { message: "a terminator instruction was encountered before
the end of block1" }` (original T5) or `"uses value vNN from non-dominating instNN"`
(original T2/T4, dominance) depending on CFG shape. D bypassed this by extracting arm body
to standalone function (all fixtures 369/371/372 use this pattern) — NO edits to `triet-jit`,
as this is a separate CFG/dominance-lowering front unrelated to sizing.

**D2 FIX (2026-07-15, submitted by D, awaiting O verify + G sign):** root cause was NOT
dominance but **SwitchInt synthetic-block collision** — `mir_lower.rs`
pre-declared fallthrough-cascade blocks for EVERY `SwitchInt` in function using
a SINGLE cumulative counter (`next_synthetic`), but terminator lowering (line
~4663, legacy) recomputed `synth_base = body.build_cfg().blocks.len()` — IDENTICAL
for every switch — causing switch#2 to overwrite switch#1's synthetic block →
verifier failed. Fix: `JitContext.switch_synth_base: HashMap<BasicBlock, usize>`
stores the CORRECT base when pre-declaring synthetics (only when `n_cases > 1`), and
terminator reads from map instead of recomputing. Bug applied to **≥2 SwitchInt in 1 function**
(sequential OR nested, not just nested) — new safeguards `378/379/380`
(`crates/triet-driver/tests/fixtures/`, poison exit 4 confirmed). Fixtures
369/371/372 now INLINE nested-match directly into arms (removing `pick_*`
workarounds), EXPECT values remain 204/7009/42.

---

**Signatures §AMEND-2:** D submitted (2026-07-15). Clean build (`cargo check
--workspace` 0 warnings), poison T1 confirmed manually (132 → 204). Awaiting O
verification + G signature.
