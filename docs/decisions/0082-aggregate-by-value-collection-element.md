# ADR 0082 — Aggregate by-value as Collection Elements (Struct/Enum in Vector/HashMap, NO Native-Packing)

> # 🩸 CORE PRINCIPLE (Proposed by O, awaiting G to carve in stone)
> # `Vector<User>` must work. BUT the price MUST NOT be shattering the invariant
> # **"every value = one i64" (8B-granular)** — the sole anchor keeping the JIT solvable.
> # The Achilles heel is NOT dimensions — but **RECURSIVE DROP-GLUE**: dropping a
> # `Vector<User>` (where User contains `String`) while raw-freeing element memory = **LEAK**;
> # byte-copying element ptrs and dropping twice = **DOUBLE-FREE**. This ADR locks that
> # conservative invariant + designs the recursive free machinery, and **stomps sub-8B packing**
> # (B-β) out of this release.

**Status:** 📝 **DRAFT — Awaiting G review + signature. NOT a single line of code written yet.** Applies to Tier C+. Enables `Vector<UserStruct>` / `Vector<Enum>` and `HashMap<K, UserStruct>` / `HashMap<K, Enum>` (value-side) by-value. This is exactly **P2** promised by ADR-0077/0078 and REFUSED at the P1 boundary.

**Scope Locked (G Approved 2026-07-08):** = **B-α** (aggregate by-value elements).
- ✅ IN: Struct/Enum as Vector elements, as HashMap VALUES.
- ⛔ OUT — **B-β sub-8B packing** (Trit=1B…): REJECTED. Preserves 8B-granular layout. Value-model i64 remains inviolable.
- ⛔ OUT — **B-γ multi-register struct returns**: deferred indefinitely.
- ⛔ OUT — Struct/Enum as HashMap **KEYS**: requires recursive hash+eq on aggregates → SEPARATE campaign.
- ⛔ OUT — `get()` **by-value** on aggregate elements: REFUSED (like String, ADR-0077) — extracted via `pop`/`remove` (move-out) or borrowed via `get_ref` (ADR-0079).

**Siblings / Inherited:** ADR-0066/0067 (No-Box heap-in-aggregate — `collect_heap_leaves`, recursive drop-glue, `LeafKind`), ADR-0076 (heap-`T?` sentinel-no-op R4), ADR-0077 (Typed Vector P1 — fat-element ABI stride>8 by-pointer, element-free loop), ADR-0078/0080 (Typed HashMap value/key — `emit_hashmap_free_value`), ADR-0079 (get-borrow — `get_ref` stride-conditional).
**STRICTLY EXCLUDED:** ADR-0068 (Box/recursive — STRICTLY FORBIDDEN), native multi-field layouts (B-β — deferred), ADR-0081 (get-borrow-mutable — FROZEN, requires deref-assign).

---

## Issue

ADR-0077/0078 opened collections for **built-in elements** (compile-time CONSTANT element sizes: scalar/handle=8B, String=24B). Aggregate by-value was HARD-BLOCKED at exactly one chokepoint:

- **`vector_elem_size` REFUSES Struct/Enum** — `mir_lower.rs:524-531`:
  `Struct(_) | Enum(_) | Capability(_) | Outcome{..} → Err(JitError::Unsupported("… by-value aggregate elements need native-layout, deferred to P2"))`. This is the sole P1/P2 boundary.

Consequence: `Vector<Point>`, `HashMap<String, User>` do not compile. The language has structs, has collections, but CANNOT store structs in collections — the very deficiency condemned in ADR-0077's core principle, one level deeper.

**Trap to Avoid:** The label "native multi-field layout" tempts packing sub-8B fields (Trit=1B) for "C compatibility". That is **B-β** — which directly destroys the i64 value-model invariant (where JIT loads/stores all fields via `stack_load(I64, slot, off)`, `mir_lower.rs:633-770`), forcing typed loads/stores I8/I16/I32 + extensions across EVERY field site in exchange for negligible density improvements **no one requested**. This ADR DOES NOT implement B-β.

---

## Decision

Enable **Aggregate-by-value collection elements (B-α)** through a **single controlled extension** of existing machinery, under a **strictly locked invariant**.

### §1 — FOUNDATIONAL INVARIANT (Locked firmly, the "byte-image definition" demanded by G)

> **INV-B-α: One layout, two residences, byte-identical.**
> The byte-image of a struct/enum inside a **collection cell** = its byte-image in a **StackSlot** — SHARING `StructLayout`/`EnumLayout` (same field-offsets, same 8B-granular size, same heap-leaf repr: String=24B fat {ptr@0,len@8,cap@16}, Vector/HashMap=8B handle). NO secondary layout. NO sub-8B packing. `stride = total_size` from `struct_layouts`/`enum_layouts`.

**Why INV-B-α is Load-Bearing:** Recursive drop-glue calculates field-offsets from `struct_layouts` (`collect_heap_leaves`, `mir_lower.rs:433`). If cell images DIFFER from stack images (e.g. if someone later packs cells to save memory), drop-walking reads incorrect offsets → frees garbage pointers → SIGSEGV/double-free. A single unique layout = drop-walking is always correct. Maintaining 8B-granularity is a MATTER OF SURVIVAL: it guarantees that stack images (where fields are written via `stack_store(I64)`) and cell images (where drop-walk reads) **are identical, at zero conversion cost**.

### §2 — Marshal Side (Entering/Exiting Cells): RIDES EXISTING Fat-Element ABI, NO New Tasks

ADR-0077 fat-element ABI is already generic over `stride`, NOT special-cased for String:
- **push** (`mir_lower.rs:3027-3059`): `stride > 8` → passes `stack_addr` of element slot → shim `copy_nonoverlapping(elem, dst, stride)` (`4171`). Struct elements **already reside in `struct_slots`** → routes by-pointer + memcpy automatically. Merely requires `vector_elem_size` returning `total_size`.
- **pop** (`4282`) / **hashmap_remove** (fat): `stride > 8` → memcpy to `out_ptr` (dest slot), sret.
- **insert** (HashMap fat value, `3060+`): `value_stride > 8` → by-pointer, identical path.

⇒ The marshal side of B-α reduces to **updating exactly one function** (`vector_elem_size` returning sizes for Struct/Enum). Push/pop/insert/remove generalize verbatim.

### §3 — NO New Double-Free Fronts Opened (Evidence: Byte-Wise MOVE)

`__triet_vector_push` is a **functional-MOVE, not a deep clone** (`4163-4177`):
`copy_nonoverlapping(old_data, new_data, old_len*stride)` relocates elements byte-exact to the new buffer, then `__triet_vector_free(vec)` frees **ONLY the old buffer** (comment `4174`: "freeing elements here would double-free"). Element heap-ptrs (including Strings nested in structs) are **relocated byte-exact without duplication** → freed exactly ONCE during `Drop(v_new)`. Caller M3-zeroes `v_old` post-call → `Drop(v_old)` becomes a no-op. **This sequence generalizes verbatim for struct elements** — nested Strings ride along intact, incurring zero double-free liabilities. (Verified by O at `4166/4176`.)

### §4 — Drop Side (SOUNDNESS ACHILLES HEEL): Recursive Drop-Glue

This is what G mandated: *"recursively invoke Struct drop-glue to clean nested String leaves."* The machinery already exists for **stack structs**; B-α generalizes it to be **address-based** to operate on element cells.

**Pre-existing Components (Reused):**
- `collect_heap_leaves(name, base_off, body, depth, out)` (`433`) — recursive descent returning flat `Vec<(offset, LeafKind)>`. Recurses nested structs, converts enums to `LeafKind::Enum` (runtime tag-switch), heap-`T?` to `LeafKind::Heap` (sentinel-no-op R4). DAG-terminating, depth-limit 64 (`440` → ADR-0068 safety net). **Copy structs return empty.**
- `emit_enum_drop_glue_at(builder, body, enum_name, base_addr)` (`1457`) — address-based, already used for enums inside structs. Blueprint for structs.
- `emit_heap_free_at(builder, addr, ty)` (`972`) — frees an individual leaf (String: ptr@0/cap@16; Vector/HashMap: recursive element loop).
- `emit_vector_element_free_loop` (`1054`) / `emit_hashmap_free_value` (`1129`) — element free loop calling `emit_heap_free_at` per element.

**New Modifications (Exactly 3 Touch Points):**

1. **Extract `emit_struct_drop_glue_at(builder, body, struct_name, base_addr)`** — clone of `emit_enum_drop_glue_at`, walking `collect_heap_leaves` (currently **inlined** at 3 sites: `1952`, `2341`, `2481`), for each leaf:
   - `LeafKind::Heap(ty)` → `emit_heap_free_at(base_addr + off, ty)`
   - `LeafKind::Enum(name)` → `emit_enum_drop_glue_at(base_addr + off, name)`
2. **`emit_heap_free_at` adds Struct/Enum Arms** (`972`, currently early-returns `!is_any_heap()` at `978`):
   - `MirType::Struct(name)` → `emit_struct_drop_glue_at(addr, name)`
   - `MirType::Enum(name)` → `emit_enum_drop_glue_at(addr, name)`
   Thereafter, the element-free loop (`1102` calling `emit_heap_free_at(elem_addr, eff)`) **automatically recurses** for struct elements — without loop modifications.
3. **`vector_elem_size` (`509`) Returns `total_size`** for Struct/Enum (replacing `Err`) — sourced from `struct_layouts`/`enum_layouts`. **Signature change:** currently static `fn(ty)`; requires `body` to query layouts → `fn(body, ty)` or method. Cascades across ALL stride call-sites (push/pop/insert/remove/free — `2873/2885/3001/3017/…`). Mechanical but BROAD.

### §5 — Guards: Copy-Struct Fast-Path vs Heap-Bearing Structs

The element-free loop (`1062`) currently guards `if !eff.is_any_heap() return` — **`Struct` IS NOT `is_any_heap()` → struct elements are SKIPPED → String leaves never freed → SILENT LEAK.** Guard must become: *loop required iff element is heap **OR** struct/enum containing heap leaves*. Predicate: `aggregate_needs_drop(body, ty)` = `!collect_heap_leaves(name).is_empty()` (struct) / enum with heap variants. **Copy structs (empty leaves) remain no-ops → byte-compatible** with `Vector<Point>` containing purely scalars (NO loop, NO `__triet_string_free` declared).

### §6 — Boundary Read-Side Operations (Locked, NO New Code)

| Op | Copy Aggregate Element | Heap-Bearing Aggregate Element |
|---|---|---|
| `get(v,i)` by-value | ⚠️ defer/refuse (element copy = shallow copy heap-ptr → double-free) — REFUSED like String | ❌ REFUSED with E-code |
| `get_ref(v,i)` (ADR-0079) | ✅ returns cell_ptr (stride>8 → `&0 Struct`, `4254`) | ✅ returns cell_ptr |
| `pop`/`remove` | ✅ move-out | ✅ move-out |
| `push`/`insert` | ✅ by-ptr memcpy | ✅ by-ptr memcpy |

`get` by-value on aggregates is **REFUSED** (including Copy structs — maintaining consistency and avoiding shallow-copy pathways). Reading = `get_ref` (borrow) or `pop`/`remove` (move-out). `get_ref` stride>8 already returns cell_ptr (`4254`) → `&0 Struct` functional since ADR-0079 §AMEND-1.

---

## Death Points (Each with FAILURE SIGNALS — feedback_failure_mode_precision)

| # | Flaw | Failure Signal if Breached | Guard |
|---|---|---|---|
| **DP-1** | Element-free loop guard `is_any_heap()` skips structs | **Silent LEAK** (`FREE==0`, no crash signal) | §5 predicate `aggregate_needs_drop` |
| **DP-2** | `emit_heap_free_at` early-returns on non-heap Structs | **Silent LEAK** | §4.2 Struct/Enum branches |
| **DP-3** | `vector_elem_size` miscalculates size (returns 8 instead of total_size) | Wrong stride → memcpy stomps adjacent fields / drop reads garbage disc → **SIGSEGV 139** | §4.3 `total_size` from layout + INV-B-α |
| **DP-4** | Double-drop when String leaf pointers duplicated (if push shallow-clones) | **SIGABRT 134** (double-free) | §3: push MOVES byte-wise, frees buffer only (`4176`) |
| **DP-5** | Copy-struct enters redundant loops / declares `__triet_string_free` breaking caller byte-compat | Existing Copy-struct fixtures turn **unexpectedly RED** | §5: empty-leaf → no-op |
| **DP-6** | Nested `Vector<Vector<User>>` / `User{Vector<String>}` fails to recurse all levels | Inner level LEAKS | `collect_heap_leaves` + `emit_heap_free_at` Vector branch (`987`) recurses; DAG depth-64 net (`440`) |
| **DP-7** | Image in cell ≠ image on stack (INV-B-α broken) | Drop walk reads wrong offsets → **SIGSEGV/134** | §1 locks single layout; marshal = memcpy verbatim `total_size` |

---

## Slicing (Proposed — G Locked)

- **Slice A — Vector<Struct>:** §4.3 vector_elem_size + §5 guard + §4.1 `emit_struct_drop_glue_at` + §4.2 Struct branch in `emit_heap_free_at`. Teeth O: push N structs-with-Strings → drop → `FREE == N*(#String-leaves)` + buffer; pop → drops ownership cleanly; Copy-struct → byte-compatible.
- **Slice B — Vector<Enum>:** §4.2 Enum branch (reusing `emit_enum_drop_glue_at` verbatim). Teeth: enum elements with heap variants vs Copy variants, tag-switch frees exact arm.
- **Slice C — HashMap<K, Struct/Enum> Value:** `emit_hashmap_free_value` value-loop leverages expanded `emit_heap_free_at`. Teeth: insert/remove/drop aggregate values.
- **Slice D (Optional) — Refactor 3 inline sites (`1952/2341/2481`) → invoke `emit_struct_drop_glue_at`:** reduces duplication tech debt.

Struct KEYS, get-by-value aggregates, B-β, B-γ = **OUT OF SCOPE**, refuse-over-guess.

---

## Teeth (O Verification Plan — cp-Snapshot, NEVER git checkout)

1. **T-LEAK (DP-1/2):** `Vector<User>` (User{name:String}) push 3 → drop; count FREE. Remove §5 guard-fix → `FREE == 0` (leak) turns poison red; retaining fix → `FREE == 3`.
2. **T-DOUBLE (DP-4):** push → pop 1 → drop; FREE == exact count, **NO SIGABRT 134**. Simulating shallow-clone MUST trigger 134.
3. **T-STRIDE (DP-3):** `Struct{a:Integer, s:String, b:Integer}` (total 40B) push → get_ref field `b`; wrong stride → reads garbage. Control variable: alter total_size hardcode → RED.
4. **T-COPY (DP-5):** `Vector<Point>` (Point{x,y:Integer}) push → drop → 0 String-frees, byte-compatible.
5. **T-NEST (DP-6):** `Vector<User>` with User{tags: Vector<String>} → drop → frees both levels completely.
6. **T-ENUM (Slice B):** `enum {A(String), B}` in vector → frees exact arm based on discriminant.
7. **T-REFUSE:** `get(v,i)` by-value aggregate → E-code; struct KEY → E-code. NO silent failures/panics.

---

## Consequences

**Gains:** `Vector<UserStruct>`, `HashMap<K,UserStruct>` functional — collections truly generalized over user-defined types. Value-model i64 intact. Recursive drop-glue (`collect_heap_leaves`) runs on heap cells instead of stack slots only — under **one unified layout and walk**.
**Costs:** `vector_elem_size` signature change (broad mechanical ripple). New helper (`emit_struct_drop_glue_at`). Element-loop guard requires additional predicate.
**Locked Out:** Sub-8B packing (B-β), multi-register returns (B-γ), aggregate KEYS, get-by-value aggregates. Everything out of scope is REFUSED with concrete E-codes.

---

**Signatures:** Proposed by O (2026-07-08). Author locked scope (B-α, approved).
**G Signed (2026-07-08):** APPROVED. Rigorous design, firmly preserving the 8B-granular anchor (INV-B-α) and correctly pinpointing DP-1 leaks and DP-4 double-frees. Proceed with Slice A.

---

## §AMEND-1 — 2 Gaps Outside Touch-List, Discovered by D in T0 Probing (O Ruled Post-G-Signature)

D probed `Vector<User>` (User{name:String}) at step T0 → uncovered **2 items outside the 6 WO touch-points**, one of which **directly impacted §3**. Recorded transparently.

### AMEND-1.1 — 🩸 GAP IN §3: Byte-Wise MOVE Generalizes Verbatim AT RUNTIME, but Compile-Time M3 Zero-Guard DOES NOT

Original §3 concluded "byte-wise MOVE sequence generalizes verbatim for struct elements" based on **runtime shims** (`__triet_vector_push` relocating bytes + freeing buffer only, `4166/4176`). Correct — but **missed a layer**: Compile-time M3 Zeroing-on-Move (`mir_lower.rs:3436-3443`) when tombstoning consumed arguments only special-cased **a single type** (`layout.name == "String"`); struct-slot-backed locals fell into `def_var(var, zero)` — zeroing the **Variable**, NOT **slot leaves**. However, `Drop(struct_local)` reads the **SLOT** (via `collect_heap_leaves` + `copy_base_addr`), not the Variable → slot retained the original String pointer → **freed a 2nd time** (1st in element-free-loop of `Drop(v)` post byte-move). **`Vector<User>` → double-free 134.**

**Root Cause:** M3-tombstoning and Drop-glue are **symmetric twins** mandated by G ("free N tiers → zero N tiers"). The tombstone-on-let-move site (`1938-1968`) ALREADY generalized correctly (walking `collect_heap_leaves`, zeroing each leaf). The tombstone-on-arg-consume site (`3436`) DID NOT — String-only. Prior to Slice A, the struct-consume-arg path had never executed (pushing structs was refused at `vector_elem_size`) → **latent gap, Slice A was the first caller to expose it**.

**RULING (O):** BLOCKING, patch WITHIN Slice A (double-free is on critical path — `Vector<User>` cannot ship with it). Added **T7** to WO: generalize guard `3436` to struct-slot tombstoning symmetric with `1938` (sharing `collect_heap_leaves` walk via helper `tombstone_slot_leaves` called from BOTH `1938` and `3436`). Committed SEPARATELY (pre-existing latent gap).

### AMEND-1.2 — ⚠️ `vector_elem_size` Shared Across Vector + HashMap → T2 Leaks Scope into `HashMap<K,Struct>`

`vector_elem_size` serves both Vector elements AND HashMap keys/values (8 call-sites in T2 including 4 HashMap sites). Opening the Struct arm makes `HashMap<Integer, User>` **source-reachable immediately** (typecheck + borrowck pass), BUT T5 only patched the vector-loop guard (`1062`) — while the hashmap value-loop guard (`emit_hashmap_value_free_loop:1286`) STILL checked `!eff.is_any_heap()` → **skipping struct values → SILENT LEAK** on map Drop.

**RULING (O):** MAINTAIN G's locked boundary — HashMap values belong to Slice C, **NOT Slice A**. Added **T8**: explicit REFUSE guard at HashMap marshal/op emit-sites — key or value being `Struct`/`Enum` → `Err(JitError::Unsupported("HashMap<_,aggregate> = ADR-0082 Slice C, not yet enabled"))`. `vector_elem_size` Struct arm retained; blocked at HashMap-op layer.

---

## §AMEND-2 — Value Move-Out for Aggregates (Vector pop / HashMap remove by-value) — Campaigns D-1+D-2

**Context:** Original ADR §2–§4 + §AMEND-1 covered **push+drop** (aggregates INTO collections; Slices A/B/C). The outbound direction — moving elements out **by-value** — was refused/deferred throughout A/B/C. §AMEND-2 closes the outbound direction: `Vector<T>` pop (D-1, `03a7638`+`f2e8bd8`) + `HashMap<K,V>` remove (D-2, `5644f6e`) returning aggregates by-value. Continuation of B-α, **NO new foundational ADR needed**.

### AMEND-2.1 — ① MOVE-OUT TOMBSTONE CONTRACT (Load-Bearing, Mandatory)

Move-out = element leaves container ownership to destination local. Preventing double-frees (container drop + destination drop freeing the same leaf) requires **MANDATORY source tombstoning**, mechanism per container:

- **Vector pop — `len--`** (`__triet_vector_pop`). Popped cell is not zeroed; `len--` excludes it from drop iteration (`i < len`). **Load-bearing:** O poisoned by removing `len--` → popped cell double-freed (FREE 3).
- **HashMap remove — `state→2`** (shim) + value-free-loop gate `state==1` (`emit_hashmap_value_free_loop`). Value cell NOT zeroed (see ③); state=2 causes map-drop to skip it. **Load-bearing AT BOTH ENDS** (G-MANDATE): GATE-A (widening gate `state≥1`) → SIGSEGV · GATE-B (removing `state→2`) → double-free tcache SIGABRT.

### AMEND-2.2 — ② TRUTH ABOUT SLICE-A-BUG-1 (History Unvarnished)

Slice B AM1 refused `__triet_vector_pop` for Struct/Enum, original comment: *"needs recursive move-out tombstone… deferred"* — implying move-out was unsound due to **missing tombstones**. **TRUTH (Exposed by D-1b):** That refusal did NOT mask an unfixable tombstone bug. It masked an **UNCONSTRUCTED ABI LAYER:** pop-dest was ALWAYS `Nullable(Struct)` (`triet-lower/lib.rs:2460`), with tag-prepended slot (ADR-0076 Option A, `tag@0/fields@+8`, `mir_lower.rs:1906`). Legacy marshal wrote fields@+0 → **overwrote tag word** → field access (+8) read garbage, drop-glue read garbage tags → freed invalid memory. `len--` source tombstone had never been an issue (available since ADR-0077). **D-1b constructed the marshal layer:** out_ptr=`slot+8` (non-String), tag=`(shim_ret==NULL_SENTINEL)?SENTINEL:1`@`slot+0`.

### AMEND-2.3 — ③ STATE-GATE DECISION: DO NOT Zero Value-Cells (Performance Trade-off, Blood-Tested Proof)

HashMap remove moves values out to out_ptr BUT **does not** `write_bytes(vptr,0,value_stride)` (unlike KEY paths which are zeroed — ADR-0080 §AMEND-1). Safety relies ENTIRELY on `state→2` + gate `state==1`. G-MANDATE required proving gate tightness BEFORE approving omission of zeroing.

**Results (Independently verified by O, cp-snapshot restore md5 `267f1cbb`, baseline GREEN before each test):** GATE-A red (SIGSEGV) · GATE-B red (double-free tcache SIGABRT). Both load-bearing.

**DECISION (Signed by G 2026-07-11):** RETAIN design WITHOUT zeroing value-cells — state-gate is sufficiently robust to prevent double-frees, saving a `write_bytes` per remove. **Conditional constraint:** if future code inspects tombstone regions (iterators, rehash, compaction touching state=2 cells) → must either (a) gate `state==1` there, OR (b) zero value-cells at that point.

### Teeth D-1+D-2 (Poison-Cemented, Independently Verified by O)
- **Vector:** `len--` → FREE 3 · T9-enum → SIGILL · present-tag loop-reuse 341/342 → (1→0) · field_off → corpus SIGABRT.
- **HashMap:** GATE-A → SIGSEGV · GATE-B → double-free SIGABRT · field_off → corpus 343 SIGABRT · present-tag 345/346 → (1→0).
- **Shared Marshal:** Dest-bind fat in `mir_lower.rs` (`vector_pop_fat || hashmap_remove_fat`) → 1 poison on present-tag fails ALL 4 loop-reuse fixtures (341/342/345/346).

---

## §AMEND-3 — Get-by-Value Aggregates, **Copy-ONLY** (Read-and-Copy, NOT Move-Out, NOT Deep-Clone)

**Status:** Signed by O (2026-07-15) and G (2026-07-15). Scope: **Copy-aggregates ONLY**.

**Context:** `get(container, k)` currently supports: scalar V → by-value `V?` (env.rs:346/438/448); heap-scalar V (String/Vector/HashMap directly) → E1047 → borrow via `get_ref` (exprs.rs:1174). **Aggregate V (Struct/Enum, scalar key)** previously fell into **E1041 NoMatchingOverload** without distinguishing Copy vs heap-bearing. §AMEND-3 enables get-by-value for **PURE Copy** aggregates; heap-bearing aggregates are REFUSED explicitly.

### AMEND-3.1 — ⚖ Copy-vs-Clone Boundary IS Soundness Boundary (Self-Imposed Barrier by O)

get-by-value **IS NOT a move-out** (§AMEND-2.1). The element **REMAINS IN** the container; a copy is returned to destination local. Non-destructive → **NO source tombstone** (AMEND-2.1 rules DO NOT apply — no transfer of ownership).

Sound **IF AND ONLY IF** the aggregate **has no heap leaves** (Copy):
- **Copy Aggregate** (`!aggregate_needs_drop` — all scalar leaves): copy is **bitwise-identical, SHARING NO heap allocations** with container element. Both drop independently → **NO double-free**. SOUND. Precedent: Slice A T-COPY `Vector<Point>` → FREE == 0.
- **Heap-Bearing Aggregate** (has ≥1 String/Vector/HashMap leaf): bitwise copy **aliases heap pointers** → two owners for one allocation → dropped twice → **double-free**. Supporting this requires **recursive deep-Clone** (allocating new copies of each leaf) = NEW capability, touching move-only **ADR-0042** + "Clone strictly forbidden (hidden allocations = anti-pattern)" ADR-0079. **MUST NOT be implicit on `get`.**

**Decision:** Copy-aggregates → get-by-value SOUND (bitwise memcpy). Heap-bearing → **REFUSED explicitly**, redirecting to `get_ref` (borrow). Deep-Clone decoupled into separate foundational campaign after explicit `.clone()` ADR.

### AMEND-3.2 — Predicate `is_copy_aggregate` (Typecheck, Mirroring `MirType::is_copy`)

Typecheck adds predicate `Type::is_copy_aggregate` (`types.rs:227`, adjacent to `is_hashable_key:165`): scalar (Trit/Tryte/Integer/Long/Trilean) → Copy; String/Vector/HashMap → Non-Copy; `UserStruct{fields}` → all fields Copy recursively; `UserEnum{variants}` → all payloads Copy recursively; `Nullable(inner)` → Copy iff `inner` is Copy.
Mirrors **`MirType::is_copy(Some(body))`** (`triet-mir/src/lib.rs:694` — "single source of truth for move/copy classification").

**Load-bearing Producer-Consumer Lock:** JIT defensive guard for `_get_copy` calls `MirType::is_copy(Some(body))` directly (HashMap path, `mir_lower:4130`). O verified: poisoning `is_copy_aggregate` heap → Copy causes `Vector<Tagged{String}>` get to **double-free 134**.

### AMEND-3.3 — Refuse Boundary for Heap-Bearing Aggregates

`get(Vector<Aggregate-heap>, i)` / `get(HashMap<scalar-K, Aggregate-heap>, k)` → REFUSED with dedicated diagnostic: **E-code `E1049` `GetAggregateByValueRequiresClone`** — message "aggregate has a heap-allocated leaf; copy-by-value would alias it" + `[Fix]` "Use `get_ref(container, k)` to borrow the element instead". Decoupled from E1047.

### AMEND-3.4 — JIT Copy-Out (Reusing `get_ref` Locate + Memcpy Stride, NO Tombstone, NO Free)

JIT path = **`get_ref` locates cell + `copy_nonoverlapping(stride)` → destination fat-slot**, OMITTING source-tombstone/free:
- Locate: `__triet_vector_get_ref` (`mir_lower:5291`) / `__triet_hashmap_get_ref` → cell_ptr (not-found → NULL_SENTINEL).
- Copy: `copy_nonoverlapping(cell_ptr, out_ptr, stride)`.
- Dest: `Nullable(Aggregate)` fat-slot tag-prepend ADR-0076 (`tag@0/fields@+8`), tag = `(ret==NULL_SENTINEL)?SENTINEL:1`. Shared destination marshal with `vector_pop_fat || hashmap_remove_fat`.
- **NO free, NO len--, NO state→2** — non-destructive.

### AMEND-3.5 — Borrowck: Read Without Consumption, NO Loan

get-by-value does not consume container, does not move element out, returns **independent owned value** (unlike `get_ref` returning `&0` with PropagatedLoan ADR-0079). Args `[false, false]`; container borrow terminates immediately at call site. No PropagatedLoan.

### Scope Container + Teeth (Independently Verified by O, cp-Snapshot Restoring md5 `a753366b`)
- **Vector + HashMap scalar-key** covered in this slice (dest-marshal shared). HashMap **aggregate-key** (ADR-0083) × aggregate-value composed subsequently.
- **Producer-Consumer Tooth (Load-Bearing):** Poisoning `is_copy_aggregate` heap → Copy → `Vector<Tagged{String}>` get → **double-free 134** (MIR: container `Drop(_2)` + copied-out `Drop(_12)` both free String). Typecheck E1049 gate is sole protection for Vector.
- **8B-Heap-Struct T9-Masking:** `Wrapper{v:Vector<Integer>}` (total_size=8, single handle) → correctly refused with **E1049** (thin path never receives it, fixture 367).

**Signatures §AMEND-3:**
- **O Signed 2026-07-15:** Verified gate CLEAN `0·0·361·0`; producer-consumer tooth red (double-free 134); 8B-masking refusal; predicate `Type::is_copy_aggregate` ↔ `MirType::is_copy` verified sound.
- **G CO-SIGN 2026-07-15:** APPROVED. E1049 is a vital safety gate — O's poison proved it load-bearing. Single-source-of-truth in `MirType::is_copy` is architecturally clean. Invariants from ADR-0042 (move-only) + ADR-0079 (forbid hidden clone) strictly maintained. **§AMEND-3 FROZEN.**
