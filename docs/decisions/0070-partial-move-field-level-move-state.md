# ADR 0070 — Partial-move & Field-level Move-state (ZST/Capability)

**Status:** **🔒 SEALED** (Signed by Mentor G 2026-06-25; blood-verified by O across 5 independent failing poison tests + byte-identical restore). Applicable to rewrite-era (Tier C). Unlocks field-level partial-move for ZST/Capability fields, completing the canonical proof for schema §10 `HardwareToken` (destructuring `let vga = hw.vga`). Amends [ADR-0025](0025-borrow-checker-rules.md) §5.3 + §9 schema BorrowChecker. Sibling to [ADR-0069](0069-zst-capability-token-luk3.md) (capability Ł3 ZST token).

**Issue:** The capability engine (ADR-0069) shipped complete Ł3 support — but Slice 4 (A2) had to **evade** the original schema §10 vision: capabilities were passed via *separate parameters* (`use_vga(vga: VgaBuffer)`) instead of being bundled inside a struct `Hardware { vga, uart }` and destructured (`let vga = hw.vga`). Reason for evasion: the borrow checker tracked move-state **per-Local** (`var_states: BTreeMap<Local, VarState>` — `checker.rs:153`), lacking the ability to represent "`hw.vga` has been moved but `hw.uart` is still alive". Consequently: Δ3 (`checker.rs:615–624`) **strictly forbade** extracting Move-types out of fields (`CannotCopyMoveTypeOut`). Without per-Place move-state, partial-move cannot exist, and schema §10 — the core proof that capabilities represent pure zero-cost ownership — could never compile.

§5.3 ADR-0025 locked a 3-state model (Owned/Moved/Conditionally-moved) *per-binding* while stating "details deferred to implementation phase". This ADR fills that exact deferral debt for the **field-level** dimension.

---

## Decision

**Promote borrow checker move-state from per-Local → per-Place (field-aware), matching existing loan-tracking symmetry** (`places_conflict`, `checker.rs:46–89`, which is already field-aware).

Four concrete commitments:

1. **Per-field move-state.** Each `Local` additionally maintains a set of moved fields (single-level field projection: `hw.vga`). Reading out a Move-type field (`let v = hw.vga`) records that field into the moved set — NO longer refused.

2. **Serrated boundary — ZST/Capability only.** Partial-move is unlocked only for fields of type **`Capability`** (ZST, runtime drop is no-op, no heap allocation → no double-free possible). **Heap** fields (String/Vector/HashMap) or other Move-types extracted from structs **REMAIN REFUSED** with `CannotCopyMoveTypeOut` — hard error, **NO panic**. (Copy fields were already permitted since `is_copy`=true bypasses Δ3.) Heap-field partial-moves are deferred to the No-Box ledger (requiring JIT dynamic drop-flags/tombstones to prevent runtime double-frees).

3. **Reusing E2420 UseAfterMove** (NO new error codes). Reusing any moved portion constitutes UseAfterMove:
   - `let v = hw.vga; hw.vga` → E2420 (field already moved).
   - `let v = hw.vga; let w = hw` → E2420 (touching dead struct whose field was extracted).
   - Unmoved siblings (`hw.uart`) → valid.
   - Diagnostic MAY explicitly state "partially moved", but **canonical code = E2420**.

4. **Per-Place merge at CFG joins = union, conservative, monotonic.** Field moved on ANY predecessor branch → moved at join. Union is a monotonic operation (ascending lattice) ⇒ fixpoint **converges** (no infinite loops — vital requirement for per-Place dataflow).

### Concrete Execution

Canonical proof (schema §10, now compiles + runs):

```triet
capability VgaBuffer grant
capability UartPort  grant
struct Hardware { vga: VgaBuffer, uart: UartPort }   // all-ZST aggregate

function use_vga(v: VgaBuffer) -> Integer { return 10 }
function use_uart(u: UartPort) -> Integer { return 7 }

function main() -> Integer {
    let hw = Hardware { vga: mint VgaBuffer, uart: mint UartPort };
    let v = hw.vga;        // PARTIAL MOVE — vga extracted from hw, 0 runtime bytes
    let a = use_vga(v);    // vga MOVED → 10
    let u = hw.uart;       // sibling — STILL ALIVE (uart not moved)
    let b = use_uart(u);   // → 7
    return a + b;          // 17
    // hw.vga;             // ← if accessed: E2420 (vga already moved)
    // let w = hw;         // ← if accessed: E2420 (hw partially moved)
}
```

Serrated boundary (heap fields — must fail hard, no panic):

```triet
struct Box { name: String }
function main() -> Integer {
    let b = Box { name: "hi" };
    let n = b.name;        // E24xx CannotCopyMoveTypeOut — NOT opened for heap
    return 0
}
```

CFG dataflow boundary (field moved in one branch, used at join):

```triet
let hw = Hardware { vga: mint VgaBuffer, uart: mint UartPort };
if cond {
    let v = hw.vga;        // moved only on then-branch
}
let again = hw.vga;        // JOIN — E2420 (union: moved on ≥1 branch)
```

### Data Model (Design direction — details in Work Order)

`BlockState` adds a per-Local field tracking moved fields, for example:

```rust
partial_moves: BTreeMap<Local, BTreeSet<String>>   // local → {field names moved}
```

- Use of base `hw` (unprojected) → moved if `var_states[hw]==Moved` **OR** `partial_moves[hw]` is non-empty.
- Use of `hw.f` → moved if base Moved **OR** `f ∈ partial_moves[hw]`.
- `merge`: `partial_moves` computes the **union** of field-sets across predecessors.
- `StorageLive` / fresh re-assignment → clears `partial_moves[local]`.

Single-level field depth (`hw.vga`) is the scope of this ADR — deep nesting `hw.a.b` → conservative whole-base move (or deferred), NOT supporting multi-level field paths here.

---

## Alternatives Considered

| # | Alternative | Pros | Cons | Conclusion |
|---|-------------|------|------|------------|
| 1 | **Per-Place move-state, ZST/Capability scope** (chosen) | Matches existing loan-tracking symmetry; sound zero-cost (ZST drop no-op); monotonic union → fixpoint converges; canonical §10 runs | Touches core dataflow, requires thorough CFG branch testing | **CHOSEN** |
| 2 | Per-Place move-state opened immediately for **heap fields** | More general | Requires JIT dynamic drop-flags at runtime to prevent double-frees → scope explosion, touches Cranelift | Deferred → No-Box ledger |
| 3 | Keep per-Local, "force whole-struct move" and forbid reusing base | No data structure change | Kills sibling fields (`hw.uart` dies collateral death) → NOT partial-move, violates schema §10 | Rejected |
| 4 | New error code E2421 for partial-move-reuse | "Explicit" | Semantics identical to UseAfterMove → code clutter; E2421 already assigned to SelfOwnershipParadox | Rejected — reuse E2420 |

---

## Consequences

### Positive
- Completes canonical proof for schema §10 `HardwareToken`: capability = pure ownership, destructure-move, zero-cost — *actually runs* rather than remaining "design only".
- Borrow checker gains per-Place move-state — foundational for all future partial-moves (including heap once No-Box opens).
- Restores symmetry: move-tracking now matches the field resolution of loan-tracking.

### Negative
- Increases complexity in `BlockState` + `merge` + use-check (three sites requiring synchronized updates).
- Single-level field depth — `hw.a.b` not yet supported (conservative fallback).

### Risks to Mitigate
- **Fixpoint loop hangs / soundness breaks at merge** — union (monotonic) mandatory, thorough CFG branch tests mandatory (safeguard #4). This is the exact risk highlighted by G.
- **Heap serrated boundary leak** — test extracting String from struct must yield `CannotCopyMoveTypeOut`, no panics (safeguard #5).
- **JIT all-ZST struct** — `struct Hardware` with all-ZST fields = 0 bytes; 0-byte StructAlloc/field-read may hit Cranelift StackSlot size-0 edge cases. Mandatory **Step 0 probing** before fixing, refuse-over-guess.

---

## Effective Date

- Rewrite-era Tier C, following ADR-0069 — activated upon closing Work Order ADR-0070 (O blood verification + G signature).
- Amends [ADR-0025](0025-borrow-checker-rules.md) §5.3 (expanding move-state to per-Place) — NOT retroactive, NO revisionism of old per-Local model (per-Local is a subset of per-Place when projection is empty).
- Amends schema §9 `BorrowChecker` (recording field-level move granularity).
- Heap-field partial-move (write-side, when ADR sealed): NOT applicable — deferred to No-Box ledger.
- **Read-side update (2026-06-26, WO Read-side heap field move-out + Addendum):**
  single-level **heap-SCALAR** field move-out (`let s = p.name` with `name: String/Vector/HashMap`)
  IS NOW UNLOCKED — borrowck records partial-move, JIT tombstones heap-leaf in base-slot (free exactly once).
  Lower type-propagation for heap fields (String→String instead of Unknown) included. **Heap-STRUCT field
  move-out (`let m = h.inner` with `inner` being struct containing heap) REMAINS DEFERRED (E2423):** blocked
  upstream by pre-existing construction-into-field double-free (ADR-0067, verified). Re-opened once
  construction-into-field tombstones source. Coffin-lid fixture: `300_field_moveout_heapstruct_e2423.tri`.

---

## ✚ AMEND — Phase 2: heap-STRUCT field move-out UNLOCKED (blessed by G 2026-06-27)

Construction-into-field double-free (ADR-0067) was patched (commit `e2b5c36`,
AMEND ADR-0067 — Deinit source tombstone), allowing the coffin lid to open. Phase 2
unlocks **single-level heap-STRUCT** field move-out (`let m = h.inner`, `inner`
being a struct containing heap leaves at any depth). 3 code sites (initial O recon
showed 2 — **missing site 3**; D probed SIGSEGV to the bottom, completing it):

1. **Borrowck** (`checker.rs` allow-arm): add `MirType::Struct(_)` to the partial-move
   recording branch. Use-after-move inherited for free (`partial_move_invalidates`
   tracks field names, type-agnostic): reusing moved field → **E2420**; continuing to
   read sibling fields OK; whole-base / multi-level → invalidated. Enum-fields + multi-level
   extractions REMAIN refused under **E2423** (deferred — no current use case).
2. **JIT** (`mir_lower.rs` read-side block): heap-STRUCT field → `collect_heap_leaves(
   name, field_off, ..)` (function already supports `base_offset`) returns leaves at
   **absolute offsets in PARENT slot**; zero per LeafKind (Heap@abs, Enum@abs+8).
   Byte-symmetric with Deinit struct-branch (`base_offset=0`→`field_off`). Free exactly once.
3. **Lower type-propagation** (`lib.rs` `Expr::FieldAccess`): **FATAL site 3.**
   Previously fields typed `MirType::Struct(_)` fell into `alloc_local()` = **Unknown** →
   JIT pre-pass DID NOT allocate stack-slot for dest move-out → aggregate-copy wrote to
   garbage addresses → **SIGSEGV**. Fix: propagate actual type `Struct(_)` (in parallel with
   existing nullable-aggregate / heap-scalar branches) → dest receives real slot.

   **Type-system decision (blessed by G):** Lower tier MUST propagate real types for
   `MirType::Struct(_)` field reads, NOT `Unknown`. Rationale: (a) dest-slot allocation
   for move-out; (b) fixes **latent 8B truncation bug** — all prior `let x = obj.copyStruct`
   with Copy-struct >8B read through 8B SSA-registers (truncation); now aggregate-copies
   with full width. `Unknown` remains only for scalar leaves (load i64). Propagating correct
   types = only path to Soundness.

**Safeguards (blood-verified by O independently):** `struct_field_moveout_phase2_counting`
(FREE==1, cap==5, poisoning JIT Struct-arm → count==2 double-free); negative
fixtures `301` (reuse → E2420), `302` (multi-level → E2423); fixture `300` FLIP
(E2423 → EXPECT:0). Regression: 263/264/265 + Phase-1 + nested/enum-in-struct
counting PASS. Gate `0·0·297·0`. Copy-struct >8B field read verified (reads correctly,
base reusable — Copy semantics preserved).

**Deferred:** Enum-field move-out · multi-level (`h.inner.x`) extraction · true-recursive (ADR-0068).

## ✚ AMEND — Phase 2b: Enum-field move-out UNLOCKED (WO-0074, signed by G 2026-06-29)

`let e = h.msg` (`msg` being an enum bearing heap payload) previously refused under **E2423**
(allow-arm only permitted {Capability, heap-scalar, Struct}). Construction + base-Drop of
enum-in-struct was already operational since ADR-0067; Phase 2b solely unlocks the **move-out path**.
3 sites symmetrical with Phase 2 (commit `e0b1ed7`):

1. **Lower** (`lib.rs` `Expr::FieldAccess`): add `matches!(field_ty,
   MirType::Enum(_))` to typed-slot allocation gate → dest receives `Enum` → JIT pre-pass
   allocates enum-slot. Missing this → dest Unknown → no-slot → aggregate-copy writes to
   garbage address → **SIGSEGV** (symmetrical with fatal site-3 in Phase 2).
2. **Borrowck** (`checker.rs` allow-arm): add `MirType::Enum(_)`. `partial_moves`
   key = simple field name ("msg"), **data structure UNCHANGED**. Nullable/Outcome
   field-moves REMAIN refused.
3. **JIT** (`mir_lower.rs` move-out tombstone): arm `Enum(_)` zeros **only
   payload-ptr@`field_off+8`** (retaining discriminant) → base tag-switch Drop reads ptr=0 →
   free is a no-op. Symmetrical with leaf-Enum tombstone (`abs+8`).

**Safeguards (blood-verified by O independently, snapshot tests):** 5 safeguards — borrowck allow (poisoning
→ E2423), double-free (poisoning JIT → FREE 2 vs 1), leak (negative polarity `==1`), cap+count
simultaneously (`STR_CAP==5 && STR_FREES==1`, assertion guard), **in-suite SIGSEGV**
(poisoning Lower → child subprocess signal 11 / exit 139, isolated crash). Gate
`0·0·303·0` (counting/subprocess tests are separate binaries → corpus 303 unchanged).

**Deferred:** multi-level (`h.inner.x`) extraction → AMEND Phase 3 below · true-recursive (ADR-0068).

## ✚ AMEND — Phase 3: Multi-level extraction (projection-path move-state) [signed by G 2026-06-29]

### Rationale
`let x = h.inner.x` (≥2 Field projections) refused under **E2423** because `single_field` returned
`None` for multi-level paths. Before building **Capability Ł3** (requiring capability tracking on
nested fields), borrowck must track **projection-paths** — otherwise Ł3 fractures at this exact
joint. G ruled: clear the foundations before building the palace.

**Root fissure:** `partial_moves: Map<Local, Set<String>>` — key was a SINGLE field name.
CANNOT represent "`inner.x` moved but `inner.y` is alive". Upgraded key to **projection-path**.

### 1. Data model: `Set<String>` → `Set<Vec<String>>` (NO Trie)
```rust
partial_moves: BTreeMap<Local, BTreeSet<Vec<String>>>   // local → {moved paths}
// h.msg → ["msg"]   |   h.inner.x → ["inner","x"]   |   whole h → []
```
Trie was rejected: moved-path set per local is small (few fields), prefix-scan O(paths×depth)
is negligible. `Vec<String>` retains existing set union idiom without over-engineering.

### 2. PREFIX-CONFLICT relationship (heart of the cascade)
P (read) and M (moved) **conflict** ⟺ one is a prefix of the other (including equality):
`conflict(P,M) ⟺ is_prefix(P,M) ∨ is_prefix(M,P)`.

| Read P | Moved M | Relationship | Result |
|---|---|---|---|
| `[inner,x]` | `[inner,x]` | exact | ❌ DEAD |
| `[inner,x]` | `[inner]` | M prefix P (parent moved) | ❌ DEAD |
| `[inner]` | `[inner,x]` | P prefix M (reading parent touches moved child) | ❌ DEAD |
| `[]` | `[inner,x]` | whole-base | ❌ DEAD |
| `[inner,y]` | `[inner,x]` | divergent | ✅ **LIVE** (sibling leaf) |
| `[other]` | `[inner,x]` | divergent | ✅ **LIVE** (sibling branch) |

Single-level is a specialized case (M=`[f]` exact; P=`[]` whole-base) → 100% backward-compatible.

### 3. Cascade across 3 functions
- `single_field` (checker.rs:403) → **`projection_path(place) -> Option<Vec<String>>`**:
  all Field projs → complete path; encountering non-Field proj → `None` (conservative
  whole-base); caller treats `None`/`[]` = whole-base.
- `partial_move_invalidates` (416): `moved.iter().any(|m| prefix_conflict(&p, m))`
  replaces `moved.contains(f)` — subsuming legacy logic (proven in §2).
- allow-arm record (702-721): `Some(path) if !path.is_empty() && (Cap|heap|Struct|
  Enum)` → `insert(path)`; non-Field proj / non-move-type → still **E2423**.

### 4. 🩸 FATAL fixpoint hole — plugged IMMEDIATELY in this amendment (separate commit)
Fixpoint check (checker.rs:520-521 entry + 541-542 exit) only compared `var_states` +
`active_loans`, **OMITTING `partial_moves`**. Because partial-move DOES NOT set
base→Moved (base remains Owned), the `partial_moves` delta was **silently discarded** → inside
loops, moves in iteration-1 failed to propagate to iteration-2 entry via back-edges →
**MISSED UAM = UNSOUND**. This was an **EXISTING latent hole** (present even for single-level,
as loop+partial-move had not been tested). Fix: add `&& new_entry.partial_moves ==
entry_states[b].partial_moves` (entry) + `|| new_exit.partial_moves !=
exit_states[b].partial_moves` (exit). **Separate commit, PRECEDING the feature commit**
(G mandate: 1 commit patching core bug, 1 commit feature — clean git history).

Union-merge (231-238) switches `Set<String>`→`Set<Vec<String>>` union; monotonic,
convergent; intersection is UNSOUND (forgets branch moves). Rationale UNCHANGED.

### 5. Reassignment clear — sub-path LOCKED via negative safeguard (G decision)
Whole-base re-assignment / fresh `StorageLive` → `partial_moves.remove(local)` (clearing
all paths) — CORRECT. **Sub-path re-assignment** `h.inner = fresh` after moving
`h.inner.x` (requiring `retain(|m| !is_prefix([inner], m))`): **NOT opened in Phase 3** —
no current use case warrants opening. Flagged + clean diagnostic + **negative safeguard
proving it is locked**. Re-evaluated when practical need arises.

### 6. Scope
- **PART A (HEART — core operation):** §1-5 borrowck core + §4 fixpoint. Signed by G.
- **PART B (periphery — reusing Phase 2):** JIT tombstones multi-level leaves at absolute
  offsets via `walk_projections` (which returns `(ty, abs_off)`); Lower `place_result_type`
  (lib.rs:1561) already loops all Field projs → multi-level leaf-type already resolved. Verified
  Site-1 coverage during WO drafting.
- **OUT:** non-Field projections (Index/Deref) · sub-path reassign (§5) · true-recursive (FORBIDDEN by ADR-0068).

### 7. Safeguards (8 tests — WO-0074 style, blood spilled BEFORE patching)
| # | Safeguard | Scenario | Fix | Poison → RED |
|---|---|---|---|---|
| A sibling-live | move `h.inner.x`, read `h.inner.y` | ✅ no error | base-only invalidate → false UAM |
| B ancestor-dead | move `h.inner.x`, read `h.inner` | ❌ UAM | remove "P prefix M" → no error |
| C exact-dead | move `h.inner.x`, read `h.inner.x` again | ❌ UAM | remove exact → no error |
| D whole-base-dead | move `h.inner.x`, read `h` | ❌ UAM | remove "[] prefix" → no error |
| E sibling-branch-live | move `h.inner.x`, read `h.other` | ✅ no error | over-conservative → false UAM |
| F ⚔ merge-union | move `h.inner.x` on 1 CFG branch, join, read | ❌ UAM | union→intersection → no error |
| G 🩸 fixpoint-loop | loop move+re-read via back-edge | ❌ UAM | remove `partial_moves` from fixpoint check → no error |
| H runtime | `let x=h.inner.x` executes | FREE==1 | remove JIT multi-level tombstone → FREE==2 |
| (neg) sub-path-locked | `h.inner=fresh` after moving `h.inner.x` | ❌ locked diagnostic | (§5 — proves locked) |

F + G form the soundness backbone. A-G borrowck (check-mode); H JIT counting.

### 8. Alternatives Considered
(a) **`Vec<String>` path** ✅ — simple, idiomatic, cheap scans. (b) Trie/radix —
premature, 0 measurable benefit. (c) Place-id interning — over-engineering. (d) Retain
multi-level refusal — blocks nested Ł3, rejected.

### 9. Consequences
**Positive:** projection-path move-state = foundation for nested Ł3 capabilities; plugs
existing fixpoint hole; unlocks multi-level extraction. **Negative:** `Vec<String>` clones more
than `&str` (acceptable — borrowck is not CPU-bound); cascade touches 4 core sites + fixpoint.
**Risks:** fatal bugs in merge/fixpoint (guarded by safeguards F+G); sub-path reassign locked (§5).

---

## ✚ AMEND — Phase 4: nullable-heap field move-out + 💀 COLLAPSE OF "dynamic drop-flag" PREMISE [signed by G 2026-06-29]

### 💀 WITHDRAWN Premise (ruled by G, proven by O via physical evidence)
§2 of this ADR (and ADR-0076 §Deferred Debt, fixture 310) stated: *"Partial-move field heap
deferred to No-Box ledger (**requiring JIT dynamic drop-flags/tombstones to prevent runtime
double-frees**)."* **The proposition "requires dynamic drop-flags" was FALSE — UNCONDITIONALLY WITHDRAWN.**

Why it was false (3 pieces of evidence measured at file:line by O, 2026-06-29):
1. **The slot ITSELF is the flag (static tombstone, zero-cost).** WO-0074/75/76 closed
   single/multi-level/struct/enum heap field move-out using **STATIC tombstones**
   (zeroing ptr@offset at move time) + **null-safe free shims** — ZERO runtime boolean
   flags. MIR at CFG joins issues `Drop(base)` **UNCONDITIONALLY**; correctness
   stems from three-states-in-one-instruction: ptr@offset ∈ {real ptr → free, 0 (moved-out)
   → no-op, NULL_SENTINEL (null) → no-op}. This is identical to §Conditional-drop =
   sentinel-no-op from ADR-0076 — **0 Cranelift `brif` instructions**.
2. **Code already ANTICIPATED this.** `collect_heap_leaves` (`mir_lower.rs:472`) arm
   `Nullable(inner) if inner.is_any_heap()` explicitly commented *"ptr@abs ∈
   {ptr, 0, NULL_SENTINEL}... 0 (moved-out)... no `brif` is needed"*. Drop-side
   support for nullable-field move-out was already present since ADR-0076.
3. **CFG-divergent witness.** `if c { let m = s.name }` followed by `Drop(s)` at join:
   move-taken (c=true) DOES NOT double-free, not-taken (c=false) frees normally —
   both exit 0 (probed by O). Conditional-drop achieved via slot-sentinel, without flags.

**When DO dynamic flags have a genuine use case?** Only if Triet introduces Move-types
with **stack-by-value + custom Deinit + NO niche/sentinel** (non-existent), OR
**Index-move** (`v[i]` — runtime offset → cannot tombstone statically). Both represent
the Collection-Semantics leviathan, **FORBIDDEN until forced by critical practical demand** (G).

### Phase 4 Decision
Promote **nullable-heap field move-out** (`let s = b.s` with `b.s : String?`/`Vector?`/
`HashMap?`) from **E2423 → RUN** using STATIC tombstones — fully symmetrical with Phase 2/2b/3.
This represents the **FINAL source-reachable E2423** on the Field-Move-out front → sealing the lid.

**3 sites (symmetrical with heap-aggregate campaign — WITHOUT touching value-model):**
1. **Borrowck allow-arm** (`checker.rs:751-769`): add branch
   `MirType::Nullable(inner) if inner.is_any_heap()` to allow-set (previously excluded
   `Nullable(_)` → triggered E2423) → records `partial_moves` projection-path like all
   heap fields. UAM/E2420/union-merge/fixpoint inherited without change (covered by Phase 3).
2. **JIT move-out tombstone** (`mir_lower.rs` move-out arm ~1523-1583): field
   `Nullable(heap)` zeros ptr@offset upon move (String? 24B fat → zero ptr-word;
   Vector?/HashMap? 8B → zero handle). Drop-side `mir_lower.rs:472` already no-ops on 0.
3. **Lower dest type propagation** (`FieldAccess`, symmetrical with Site-3 WO-0072/74/75):
   dest local receives real `Nullable(heap)` (NOT Unknown) → JIT allocates correctly
   sized slot (avoiding Unknown → no-slot → SIGSEGV).

**LOCKED Scope (deferrals retained in cage):** Index/Deref/enum-Payload move-out
(non-Field projections — remain E2423, Collection-Semantics) · sub-path reassign
(E2424, §5 Phase 3). NO scope creep.

### Phase 4 Safeguards (blood-verified by O independently — poison must fail RED, restore via cp WITHOUT git checkout)
| # | Safeguard | Scenario | Poison → RED |
|---|---|---|---|
| 1 💀 double-free | `let s=b.s` (present `~+"hi"`) + `Drop(b)` | remove Site-2 tombstone → **SIGABRT 134** (condition for G signature) |
| 2 leak count | present move-out, count FREE | poison `is_copy(Nullable(heap))→true` → struct Copy → no drop → FREE==0 |
| 3 null-state | `b.s = ~0` then move-out + Drop(b) | FREE==0, no crash; poison store-0-instead-of-sentinel → fails |
| 4 ⚔ CFG-divergent | move nullable-field on 1 `if` branch, Drop(b) join | dual-config (taken/not-taken) both clean; remove tombstone → taken-path 134 (condition for G signature) |
| 5 SIGSEGV | remove Site-3 → dest Unknown → no-slot | child wait_status 139 isolated subprocess |
| 6 E2420 preserve | reuse field after move-out | remains E2420 (use-after-move) |

Each probe covers `String?`/`Vector?`/`HashMap?` (lesson HP.3 — SAFEGUARDS cover full variant space).

### Fixtures
Flip `310_heap_nullable_field_moveout_e2423` → `*_run` (EXPECT 0) + add
Vector?/HashMap? variants + counting harness (present FREE==1 / null FREE==0 /
double-free FREE==2).

### Consequences
**Positive:** closes final source-reachable E2423 on Field-Move-out; Field-level
Ownership front CLOSED; 0 lines added to value-model; dynamic-flag premise permanently
buried (no future unearthing). **Negative:** allow-arm adds 1 branch + JIT move-out
adds 1 arm (surgical, bounded). **Risks:** wrong is_copy → leak (safeguard 2); store-0
instead of sentinel → incorrect match on `~0` (safeguard 3); dest Unknown → SIGSEGV (safeguard 5).
