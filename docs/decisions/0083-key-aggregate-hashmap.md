# ADR 0083 — Key-Aggregate HashMap (`HashMap<Struct, V>`, Structural Content Hash/Eq via fnptr-in-header)

> # 🩸 CORE PRINCIPLE (Proposed by O, carved in stone by G)
> # A `Struct` used as a HashMap **key** must be hashable and comparable for equality. HOWEVER, key hash/eq
> # **HAS ZERO CONNECTION to the `==` operator or Ł3 Trilean algebra** — it is
> # **recursive structural content/bit-equality over the physical layout** (ADR-0080 already
> # carved `Ord ≠ Hash`, content-eq ≠ `==`). This is our semantic anchor: aggregate keys
> # DO NOT reopen the Trilean quagmire.
> #
> # The fatal hazard is NOT semantics — it is the **SIZE COLLISION TRAP**:
> # `String` keys have `key_stride == 24` (FatStr = ptr+len+cap). A
> # `struct{a,b,c: Integer}` also has **exactly 24B**. If probe-shims disambiguate via
> # `key_stride == 24` (as in O's initial design — REJECTED BY G), it reads the first 8B of the struct
> # as `len`, casts the next 8B to `ptr`, and dereferences → **CATASTROPHIC SIGABRT / memory corruption**.
> # `key_stride` is merely total byte-width, NOT structural metadata → NEVER use stride
> # as a discriminator for aggregates.
> #
> # Prevention strategy (G mandate): **fnptr-in-header + null-sentinel**. Type-aware hash/eq
> # emitted by JIT (JIT possesses `StructLayout`; Rust shims do not), passed to probe functions via
> # function pointers **residing in the header** (rehash executes INSIDE insert → fnptr must be
> # reachable within the shim). The presence of `hash_fn != NULL` IS the discriminator —
> # NOT stride. Preserves the FORBIDDEN-dynamic-dispatch rule of ADR-0080: fnptrs resolved
> # at JIT-compile time per key type, exactly like per-type JIT-emitted free loops.

## Scope

- ✅ **IN (Slice 1):** `HashMap<Struct, V>` — `Struct` as KEY. Leaves of key struct
  ∈ `{non-nullable scalar, String, nested-struct satisfying recursive rule}`. Operations:
  `insert` / `get` / `get_ref` / `contains` / `remove` / drop. `V` preserves existing
  supported domain (scalar / String / Vector / HashMap / aggregate-value Slice C ADR-0082).
- ❌ **OUT — EXPLICIT REFUSALS (E1048 or JIT-refusal with safeguards):**
  - **Enum key** → Slice 2 (deferred). Discriminant matching + padding bits +
    variant size mismatch = separate isolated campaign.
  - **Collection leaves in key** (`Vector` / `HashMap` field): mutable collections
    as keys = hazard — mutating post-insert alters hash → element vanishes into
    thin air. Emit E1048.
  - **Nullable leaves in key** (`Integer?`, …): sentinel bit-patterns carry special
    semantics; while bit-equality could run, avoid semantic hazards in Slice 1. Emit
    E1048. Unlock incrementally once Slice 1 is clean.
  - `Outcome` leaves, and anything outside `is_hashable_leaf`.

## Issue — Recon by O on 2026-07-12 (file:line, `mir_lower.rs`)

1. **Probe = Monolithic Rust shim.** `__triet_hashmap_insert` (`@5182`) /
   `_get` (`@5309`) / `_get_ref` (`@5350`) / `_remove` (`@5411`) /
   `_contains` (`@5477`) — probe loops reside in Rust `extern "C"`, NOT JIT-emitted.
2. **Hash/eq dispatch SOLELY by `key_stride`.** `hashmap_key_hash(key_stride,
   k, cap)` (`@5049`): `key_stride > 8 ? __triet_string_hash(FatStr) : identity(k)`.
   `hashmap_key_eq(slot_ptr, key_stride, k)` (`@5067`): `key_stride > 8 ?
   __triet_string_eq : i64 ==`. **`key_stride` is purely a byte-width number** — for
   struct keys it does NOT indicate which fields are String (content) / Integer (identity) /
   nested. These two fixed Rust functions CANNOT compute structural hash/eq.
3. **Size Collision Trap (proof):** `FatStr` (`@4410`) = 24B; String
   `key_stride == 24` (`c24 @1307`). `struct{3×Integer}` = 24B → `key_stride == 24`
   → collision with String branch. → stride MUST NEVER serve as discriminator.
4. **Header currently 8B, no capacity for type metadata.** `HASHMAP_HEADER_SIZE = 8`
   (`@4945`) = `[refcount:u32 @0][packed:u32 @4]` (packed = `key_stride<<16 |
   value_stride`, ADR-0080 Prong A). `body = ptr.add(HEADER)` (`@5108`),
   `header = body.sub(HEADER)` (`@5146`), `hashmap_layout` (`@4971`).
5. **Gate currently REFUSES aggregate keys.** `refuse_hashmap_aggregate_key`
   (`@625`) + `refuse_hashmap_aggregate_kv` (`@601`). Typecheck E1048
   (`exprs.rs:1015`, `env.rs:356/372`) hardcodes keys ∈ {Integer, String}.
6. **Reused machinery (~60-70%):** by-pointer key marshaling when stride>8
   (`@3343/3422/3457`); allocator already receives `key_stride` (`@5089`); layout-walk template
   `collect_heap_leaves` (`@433`); key free-loop skeleton `emit_hashmap_key_free_loop`
   (String-only currently); aggregate free-recursion pattern `aggregate_needs_drop` (Slice C).

## Decision

### §1 — FOUNDATIONAL SEMANTICS (locked): key-eq/hash ≠ `==`/Ł3
Key structural equality = **recursive content/bit-equality on physical layout**, strictly
separated from the `==` operator (Trilean Ł3) and `Comparable` trait (ADR-0038, `Ord`). Precedent:
ADR-0080 line 4 (`Ord ≠ Hash`), `hashmap_key_eq` uses `__triet_string_eq` (byte
comparison) + i64 identity, **WITHOUT touching `==`**. Consequence: aggregate keys DO NOT require,
DO NOT touch, and DO NOT reopen Trilean algebra. **FORBID** `Hashable` trait, **FORBID** runtime
dynamic dispatch (inheriting ADR-0080).

### §2 — ABI: Fixed 24B Header + fnptr calling convention (G MANDATE)
- **Fixed 24B Header** (C-ABI, uniform across all maps):
  `[refcount:u32 @0][packed:u32 @4][hash_fn:u64 @8][eq_fn:u64 @16]`. Bump
  `HASHMAP_HEADER_SIZE` 8→24. fnptr @8/@16 naturally 8-byte aligned.
- **`__triet_hashmap_alloc` signature update:** `(len, cap, key_stride, value_stride,
  hash_fn: i64, eq_fn: i64)`; writes hash_fn@8 / eq_fn@16 into header after allocation.
  Internal rehash (`@5203`) propagates both fnptrs from old map header.
- **Null-sentinel (discriminator):**
  - K = Integer / String → JIT passes `hash_fn = eq_fn = NULL (0)`.
  - K = Struct → JIT passes `func_addr` of the emitted walker (§3).
- **fnptr calling convention (locked):**
  - `hash_fn(key_ptr: *const u8) -> i64` — returns **raw FNV hash**; Rust shim computes
    `(raw % cap + cap) % cap` (matching `@5057`). *Rationale (approved by G): division
    of responsibility — JIT handles bit-mixing, Rust shim handles table-index mapping
    (`cap` is already in shim registers); avoids bloating walker ABI with `cap`.*
  - `eq_fn(slot_key_ptr: *const u8, probe_key_ptr: *const u8) -> i64` — 1=eq, 0=ne.

### §3 — JIT Walkers (type-aware, recursing `StructLayout`)
- **`emit_struct_key_hash`** — layout descent following `collect_heap_leaves`: scalar
  leaves → mix raw i64 into FNV; String leaves → `__triet_string_hash(ptr,len)` then
  mix; nested structs → recurse. Emits ONE FuncId per key layout; address obtained via
  `declare_func_in_func` + `func_addr`.
- **`emit_struct_key_eq`** — layout recursion, **short-circuiting** immediately on mismatch:
  scalars → i64-eq (`read_unaligned`); Strings → `__triet_string_eq`; nested structs → recurse.

### §4 — Recursive Key Drop-Glue
`emit_hashmap_key_free_loop` (currently String-only) → recurses over `StructLayout` freeing ALL
String leaves (reusing `aggregate_needs_drop` + Slice C value-free-loop pattern).
Applies to both (a) map-drop freeing all resident keys · (b) `remove` freeing removed keys via
registry-routed out-params (ADR-0080 §AMEND-1) — aggregate remove-free is also recursive.

### §5 — Typecheck: `is_hashable_leaf` predicate + E1048 boundaries
Relax E1048 (`exprs.rs:1015`, `env.rs:356/372`) for Struct keys, gated through NEW
predicate **`is_hashable_leaf()`**: valid ⟺ all leaves ∈ `{non-nullable scalar, String,
nested-struct recursively valid}`. Encountering `Vector`/`HashMap`/`Enum`/`Nullable`/`Outcome`
leaves, or top-level Enum keys → **E1048** (updated diagnostic: non-hashable/mutable leaf).

### §6 — Probe Dispatch Order (24B Collision Shield)
`hashmap_key_hash`/`hashmap_key_eq` add parameters `hash_fn`/`eq_fn` (caller reads from
header and forwards). **INVIOLABLE dispatch order:**
```
if (fn != NULL)      { call_fn(...) }        // aggregate — type-aware
else if (stride > 8) { FNV(String) }         // String
else                 { identity(Integer) }   // Integer
```
fnptr-check executes **BEFORE** stride-check. Only this order guarantees Struct-24B (fnptr≠NULL)
NEVER collides with String-24B (fnptr=NULL).

## Failure Modes & Death Points (each with error signal — feedback_failure_mode_precision)
- **DP-1 Collision-24B:** inverted dispatch order (stride before fnptr) → 24B struct enters
  String branch → **SIGABRT / corruption**. (Shielded by §6.)
- **DP-2 Missed Header Offset:** bumping HEADER 8→24 while missing a raw offset site → 16B
  offset shift → **memory corruption** (not necessarily immediate SIGABRT — silent corruption risk).
- **DP-3 Key-leaf Leak:** key free-loop fails to recurse into Struct → String leaves in keys
  **silently LEAK** (FREE < N).
- **DP-4 Remove Double-Free:** remove-key-free recursion overlaps with map-drop → **SIGABRT 134**.
- **DP-5 Unresolved func_addr:** incorrect JIT self-reference setup → **"unresolved
  symbol/relocation" runtime error** (Risk #1).
- **DP-6 Vacuous Refusal:** neutered predicate/gate while tests remain green → **"compile
  SUCCEEDED"** (leak/corruption risk).

## Slicing (Finalized by G)
- **Slice 1 (unlocked NOW):** Struct keys. Unblocks fnptr pipeline + header + walkers.
- **Slice 2 (deferred):** Enum keys.

## Safeguards (O verification plan — snapshot tests, NO git checkout)
- **★ G-MANDATE COLLISION-TRAP:** key `struct K3{a,b,c: Integer}` (exactly 24B) insert/get
  round-trip correct. Poison test: invert §6 (omitting fnptr-check-first) → **SIGABRT/corruption**.
- **content hash/eq:** identical content (String leaves with different addresses) → hash collision resolved correctly (get succeeds);
  poison walker → lookup misses.
- **key drop (PERMANENT counting safeguard, not just fixture-harness):** insert N keys
  `struct{name:String,id}` / drop → FREE==N; poison §4 → leaks.
- **remove key-free:** remove aggregate key → recurses, no double-free; poison → 134.
- **non-vacuous refusal:** `HashMap<K{v:Vector},_>`→E1048 · `HashMap<Enum,_>`→E1048/refuse
  (shims registered to ensure non-vacuous); poison neutered predicate → "compile SUCCEEDED".
- **func_addr spike (Risk #1, completed BEFORE walker):** minimal JIT function returning 1 constant,
  extract `func_addr`, pass to Rust shim, print → proves relocation functions BEFORE writing recursive walkers (G mandate: fail fast).

## Consequences
- **+** Completes symmetry between "what can be a value" ↔ "what can be a key" for HashMaps; Struct
  keys sound end-to-end through JIT real-allocator with zero byte leakage.
- **+** Opens fnptr-in-header pipeline + JIT self-reference — reusable infrastructure for
  Slice 2 (Enum keys) and any future per-type dispatch.
- **−** Header +16B overhead per HashMap (all maps, including Integer/String — trade-off for uniform
  ABI without fragmentation). Accepted: safety first.
- **−** First instance of JIT self-referential `func_addr` — relocation risk (DP-5), de-risked
  via upfront spike.
- **Inherited Invariants:** INV-B-α (ADR-0082, single layout two targets 8B-granular) — key
  struct in HashMap slot = byte-image of struct in StackSlot; `Ord ≠ Hash`
  (ADR-0080); NO dynamic dispatch.

---

**Signatures:** Proposed by O (2026-07-12). ABI (§2/§6 fixed-header + null-sentinel + dispatch
order) **mandated by G** after REJECTING O's initial stride-branch design (Size Collision
Trap). fnptr contract (§2: hash=raw i64, eq=1/0) + `is_hashable_leaf` boundary blocking
Nullable leaves (§5) **fully approved by G**. Author (Giang) finalized direction (campaign ②
key-aggregates). **SIGNED BY G (Mentor G - 2026-07-12). All terms in this ADR are LAW. Forward march.**

---

## §AMEND-1 — Slice 2: Enum keys (`HashMap<Enum, V>`) — Proposed by O 2026-07-13, co-signed by G

Slice 2 was deferred in the original ADR scope (Scope §OUT). The ABI foundation **DOES NOT CHANGE**
(retaining fnptr-in-header + null-sentinel + §6 dispatch); only the **JIT walker internals** switch from
straight-line leaf folding (struct) → **disc-switch brif chains** (enum). NO new ADR required.

### Scope (Ruled by G 2026-07-13)
- ✅ **IN:** enums as KEYS (`HashMap<Enum,V>`); **enums as LEAVES of struct-keys** (`struct{tag: MyEnum}`); **nested enums** (variant payload is enum) — all via **unified recursion**, bounded at **depth-64** (JIT stack overflow during walker compilation = implementer error). Unit/scalar variants supported.
- ❌ **OUT (RETAIN REFUSAL E1048):** **`Enum?` (Nullable enum) keys** — Nullable carries unique sentinel bit-patterns (`NULL_SENTINEL` modifying `tag` = hazard); Slice 1 forbade Nullable leaves, and Slice 2 makes no exception. Vector/HashMap/Outcome leaves remain refused.

### §A1 — PRINCIPLE (Remedy for garbage/padding/size mismatch)
Hash/eq walkers **ONLY touch `disc@0` + declared leaves of the ACTIVE variant** (via disc-switch),
**NEVER** reading raw fixed-width images. Inactive/padding bytes = stale garbage (fixed-width
tagged-union: `total_size` = max variant, tails of smaller variants not rewritten on re-assignment)
→ **not reading = no corruption.** Matches the exact mechanism used by `emit_enum_drop_glue_at` (mir_lower:1886) —
freeing ONLY heap payloads of active variants, never touching inactive garbage. Slice 2 MIRRORS that pattern.

### §A2 — Walkers (disc-switch)
- **hash:** load `disc@0` → **mix disc into FNV** (disc IS part of identity: 2 distinct variants must hash differently) → brif-chain over variants → active arm mixes payload leaves @`payload_off=8`. Unit variant = disc only.
- **eq:** load `disc_a@0`/`disc_b@0` → **different discs → NE immediately** (short-circuit) → matching discs → brif active arm → compare declared leaves @+8; mismatched leaf → NE.
- **`collect_key_leaves` enum path:** previously struct-only flat (`:554`). Enums CANNOT flatten statically (variant-dependent) → per-variant leaf collection at `payload_off=8` (scalar/String/nested-struct/nested-enum recursion, depth-64).

### §A3 — ABI + free-loop = REUSED
Header/§6/collision shield/by-pointer marshaling/`func_addr`/`walker_ids` memoization (keyed by enum name) =
replicated verbatim from Slice 1. **Key free-loop §4 = DIRECTLY REUSES `emit_enum_drop_glue_at`** (disc-switch
freeing active variants already correct). Remove `refuse_hashmap_enum_key:880`; `is_hashable_key/leaf` (types.rs:163/177)
unlocked for Enum (⟺ all variant payloads hashable); overload wiring `exprs.rs:1190` adds Enum branch (insert/remove already generic).

### §A4 — Failure Modes (each with signal + safeguard)
- **DP-E1 Disc omitted from hash/eq** → 2 variants with matching payloads collide/eq falsely → **silent bug**. Safeguard: insert V1, get V2 → MISS.
- **DP-E2 Inactive garbage bytes enter hash/eq** → keys with identical content but differing garbage → hash/eq diverge → **silent data loss**. **Forced-garbage safeguard (G mandate, see §A5).**
- **DP-E3** = DP-E2 on equality side (false-inequality).
- **DP-E4 Padding in active payload** → only walk declared leaves (avoiding raw ranges) = safe like structs.
- **DP-E5 Key free-loop for enum heap-payloads** (variant containing String) → REUSES drop-glue; counting safeguard FREE==N, poison → leak/double-free.
- **DP-E6 Collision §6** (enum `total_size` can =24) → fnptr-first shield; safeguard SIGSEGV when inverting dispatch.

### §A5 — FORCED-GARBAGE SAFEGUARD (G mandate for DP-E2) — reassign-force-garbage
Most rigorous method to force garbage through Triet source:
```
let k = MyEnum::BigVariant(999, 888, 777);   // slot fully populated
k = MyEnum::SmallVariant(1);                 // smaller tag+payload overwrites; tail {888,777} REMAINS GARBAGE
let m2 = insert(m, k, 42);                    // key contains garbage
let k_clean = MyEnum::SmallVariant(1);        // fresh, clean tail
get(&0 m2, k_clean);                          // HIT=walker ignores garbage (passes) · MISS=consumes garbage (fails)
```
Poison walker (adding 1 raw-range leaf to tail, or hashing full fixed-width) → **MISS = FAILS RED**.
**⚠️ WARNING FROM O (lesson from Slice 1 ptr-mix-vacuous):** if lowerer ZEROES the entire slot on re-assignment → tail=0 →
safeguard becomes VACUOUS (garbage does not exist). If D finds safeguard does not fail RED when poisoned → **escalate to O** (RULE 4),
DO NOT assume "it is safe": either (a) reassign-zeroing mitigates hazard (requires white-box walker-output tests
instead), or (b) alternative garbage-forcing paths exist (divergent construction histories). Probe deterministically, not probabilistically.

### Safeguards Slice 2 (O verification plan — snapshot tests, NO git checkout)
DP-E1 (insert-V1-get-V2 MISS) · **DP-E2 reassign-garbage HIT** (§A5, poison→MISS) · DP-E5 enum-key String-leaf
free counting (poison→leak) · DP-E6 §6-reverse SIGSEGV · enum-as-struct-leaf roundtrip · nested-enum roundtrip ·
`Enum?`-key → E1048 (non-vacuous) · unit-variant enum key (disc-only) roundtrip.

**Signatures §AMEND-1:** Proposed by O (2026-07-13). Scope (enum-leaf ✅ · nested-enum ✅ depth-64 · `Enum?` ❌ REFUSE)
+ reassign garbage safeguard (§A5) **ruled/mandated by G 2026-07-13**.

**SIGNED BY G §AMEND-1 (Mentor G - 2026-07-13). Enum keys deploy!** All terms in §AMEND-1 are LAW. G awaits DP-E2 garbage test results — zero pointless SIGSEGVs.
