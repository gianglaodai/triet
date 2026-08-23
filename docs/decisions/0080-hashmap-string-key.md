# ADR 0080 — Key-typed HashMap P1 (`HashMap<String, V>`, content hash/eq)

> # 🩸 CORE PRINCIPLE (Carved in stone by Giang on 2026-07-03)
> # `Ord ≠ Hash`. Comparable (ADR-0038, `compare() -> Trit`, TOTAL ORDERING) is the WRONG vehicle —
> # mixing ordering into hashing is the quickest route to architectural destruction. HashMap requires CONTENT
> # **Hash + Eq**, not `< >`. String keys carry **DROP-OBLIGATIONS**: each key in a
> # slot is a live heap String. The #1 concern is NOT "what number does it hash to" but **HEAP LEAKS &
> # DOUBLE FREES**. Not a single byte leaked. STRICTLY FORBID `Hashable` trait, STRICTLY FORBID runtime dynamic dispatch.

**Status:** ✅ **APPROVED — Signed by Author + O + G on 2026-07-03.** Constitution ratified; WO KM-P1a issued to D. No code yet (no retroactive "IMPLEMENTED" — promoted only when slice lands + O verifies teeth). Applies to Tier C+. Continuation of ADR-0078 (Typed HashMap P1 value) — opens **Tier 2 (KEY typing)** which 0078 backlogged.

**Siblings / Inherited:** ADR-0078 (value-typed HashMap — value storage / slot stride / typed free machinery reused INTACT), ADR-0077 (Typed Vector P1 — stride helpers), ADR-0049 §6.3 (String repr: heap = `header + data`, NO len/cap on heap), ADR-0079 (get-borrow `&0 container`), ADR-0069 Slice 3 (`cap_id_hash` FNV-1a — template for string-hash).
**REJECTED as Vehicle:** ADR-0038 (Comparable = Ord, not Hash). **DO NOT CREATE:** `Hashable` trait, keys ∈ {Tryte, UserStruct, Enum, …} (REFUSED), native layout (Option D), ADR-0068 Box.

---

## Issue — Recon by O, 2026-07-03 (file:line)

HashMap currently uses **identity-hash on i64**. Correct for Integer keys (value = i64); WRONG for String keys. Recon uncovered a **concrete layout barrier**, not merely a shim connection task:

1. **Key Slot is HARDCODED to 8 Bytes.** `mir_lower.rs:4054` — `hashmap_slot_size = 8 + value_stride + 1`. Values have elastic `value_stride` (ADR-0078); **keys do not** — always 8B, read/written via `hashmap_key_ptr → *mut i64` (`:4075`).
2. **String DOES NOT store `len` on the heap** (ADR-0049 §6.3, `string_layout` `:3428` = `HEADER_SIZE + cap`, no len/cap). `{ptr, len, cap}` lives in a **24B fat pointer on stack**. Hard proof: `__triet_string_eq(a_ptr, a_len, b_ptr, b_len)` (`:3542`) **MANDATES receiving separate `len`** — cannot read len from heap pointer.
   → **This is the core of D1:** to content-hash/eq a String key we need `{ptr, len}`, but an 8B slot only fits `ptr`. Without len ⇒ cannot hash/eq. The key slot MUST expand to accommodate fat pointers.
3. **Hash/Eq is Currently Identity.** `:4247` `hash = (k % cap + cap) % cap`, `:4253` `stored_k == k` — i64-modulo + i64-eq. Integer keys: correct. String keys: hashes on **pointer addresses** ⇒ two Strings with identical content but distinct allocations yield two different keys. Semantically incorrect.
4. **Content-Eq EXISTS, Content-Hash DOES NOT.** `__triet_string_eq` (`:3542`) already exists. `__triet_string_hash` does not exist yet. Pre-existing FNV-1a template: `cap_id_hash` (`:3372`).
5. **KEYS Now Carry Drop Obligations.** Key = heap String ⇒ introduces unhandled free-paths. Current JIT-emitted free loop only frees VALUES (`emit_hashmap_value_free_loop` `:1133`). Touching no keys ⇒ **leaks all keys when map is dropped** + multiple other death points (§Track D).
6. **Typecheck Hardcodes Integer Keys.** `env.rs:342–391` declares `hashmap_new/insert/remove/get` with rigid K = `Integer`; `check.rs:1101` builds `Type::HashMap(K,V)`. Supporting String keys requires genericizing the KEY column (parallel to what HM-P1b `f5c11e1` accomplished for the VALUE column).

---

## Decision (Author Locked D1–D5, 2026-07-03)

Enable `HashMap<String, V>`, with **keys ∈ {Integer, String} — frozen**. V = supported value set (scalar / String / Vector / HashMap / Nullable). Symmetric with value machinery in ADR-0078; adds **KEY column symmetric to VALUE column** + **content hash/eq** + **key drop-glue**.

### Track A — Slot: `key_stride` Parallel to `value_stride` (D1 = Option (a), Complete 24B Fat)
- New slot layout: `[key@key_stride | value@value_stride | state1]`. `slot_size = key_stride + value_stride + 1`.
  `hashmap_key_ptr = base + idx*slot`; `hashmap_value_ptr = key_ptr + key_stride`; `hashmap_state_ptr = value_ptr + value_stride`.
- `key_stride ∈ {8, 24}`: Integer key = 8 (i64), String key = 24 (complete fat `{ptr,len,cap}`). Contains **len** ⇒ hash/eq reads directly from slot, WITHOUT needing len on the heap. **This is why 24B is mandatory** — directly tied to hash/eq requirements in Issue #2.
- **REJECTED 16B `{ptr,len}` omitting cap:** `__triet_string_free(ptr, cap)` (`:3482`) REQUIRES real cap to `dealloc` with correct layout. Omitting cap ⇒ freeing with wrong layout ⇒ UB/segfault. "Shrinking sizes is easy, patching unsoundness is costly" (Giang). Retain 24B.
- **Buffer SELF-DESCRIBES Key Kind.** `alloc` carries `key_stride` (parallel to `value_stride` residing in reserved-word header `:4122`). `key_stride == 24 ⟺ String key ⟺ uses content hash/eq`; `== 8 ⟺ Integer ⟺ identity`. **`key_stride` serves as the discriminator dispatch** — no separate tag allocated. Exact packaging (which byte in header / body-word / 2 monomorphized shims) = **implementer's choice** (D chooses minimal churn), MANDATORY INVARIANT: buffer must self-describe key kind so `free`/`rehash` DO NOT require external type-info (they only receive pointers). FORBID dynamic trait dispatch.

### Track B — Content Hash/Eq (D2 + D3)
- **Hash:** NEW shim `__triet_string_hash(ptr, len) -> i64` = FNV-1a over `len` content bytes (mirrors `cap_id_hash` `:3372`, adapting input `&str` → `(ptr,len)`). Deterministic based on content.
  Slot = `(hash % cap + cap) % cap`. **P1 DOES NOT cache hashes** (recomputed on each probe / each rehash from `{ptr,len}` in slot — keys are typically small IDs, minimizing complexity).
- **Eq:** Probing when `key_stride == 24` calls `__triet_string_eq(slot_ptr, slot_len, k_ptr, k_len)` (`:3542`) instead of `stored_k == k`. `slot_len` is readily available because slot contains 24B fat pointer (Track A).
- **Dispatch:** Shim branches on `key_stride` (8 → legacy identity path, preserved; 24 → string path). Fast-path Integer PRESERVES byte compatibility.

### Track C — Typecheck/Borrowck: Genericizing the KEY Column (D4 Vehicle + D5 Scope)
- `types.rs` / `env.rs:342–391`: key becomes type parameter `K ∈ {Integer, String}`. Declare `hashmap_new<K,V>` · `insert<K,V>` · `get<K,V>` · `remove<K,V>` · `contains<K>`. Seed K from `expected_type_stack` identically to V (HM-P1b).
- **REFUSE other keys:** K ∉ {Integer, String} → typecheck REJECTS explicitly at boundary (new error code, e.g. E10xx `UnsupportedHashMapKey`; concrete code assigned during implementation). No skeletons, no soft deferrals.

### Track D — MANAGING DROP OBLIGATIONS (Giang: CRITICALLY IMPORTANT) — 5 Death Points
String keys = heap owned. Each point below represents an independent teeth verification front (teeth must cover **all variants**):

1. **Map Drop → Free EVERY Active Key.** JIT-emit **key-free loop** parallel to value-free loop (`emit_hashmap_value_free_loop` `:1133`): iterate `cap` slots, `state == occupied(1)` → free key@key-cell via registry-routed emit (countable, preventing vacuity). Integer keys (`key_stride==8`) → DO NOT free. Sentinel-no-op R4 (ADR-0076).
2. **Insert DUPLICATE Key (Update) → Free Redundant Key.** Caller moves a String key in; if slot already holds equal-content key, map RETAINS resident key, **new incoming key (already moved-in) MUST be `__triet_string_free`'d IMMEDIATELY** — lacking destination = leak. LOCKED: update ⇒ free-incoming-redundant-key, retain resident.
3. **Insert = Move Key.** borrowck/typecheck consumes key argument for Strings (Copy no-op for Integers) — matches value move-tracking machinery (Track D ADR-0078). Heap key-arg not consumed ⇒ caller double-free.
4. **get / remove / contains Key = BORROW `&0 String`.** Key is used solely for LOOKUP (hash/eq), NOT stored, NOT consumed. Asymmetric with insert (by-value Move). Typecheck: parameter key = `&0 String` for read operations; borrowck: key remains owned by caller, caller frees normally. (Reuses `&0 container` model of ADR-0079 for key args.)
5. **Remove → Free RESIDENT Key (Discovered by O, beyond Author's 4 points).** `remove(map, k)` eliminates entry: value moved out to caller (per ADR-0078), but **resident key in slot loses destination → map owns it → must `__triet_string_free` upon tombstoning**. Missing this = leak on every String-key removal.

**Rehash Invariant (`:4205` branch):** keys move via pointers from old → new (memcpy **key-cell according to `key_stride`**, NOT 8B i64-reads of 24B fat); `__triet_hashmap_free(old)` frees ONLY BUFFER, WITHOUT touching key contents (which have moved) ⇒ no double-free. Poisoning i64-read → corrupts len → caught by teeth.

### Boundaries (Deferred — Touching Means Danger)
User-defined `Hashable` trait · keys ∉ {Integer,String} · get-borrow-mutable keys · hash caching · `HashMap<_, UserStruct>` (P2 native-layout) · Comparable/Ord domain (ADR-0038, distinct front) · ADR-0068 Box.

---

## Alternatives Considered
| # | Alternative | Conclusion |
|---|-------------|------------|
| 1 | **key_stride 24B fat parallel to value_stride** (selected, D1a) | Reuses fat value machinery; slot has len ⇒ hash/eq sound; free has cap |
| 2 | key 16B `{ptr,len}` omitting cap | **REJECTED** — `__triet_string_free` requires cap ⇒ frees wrong layout ⇒ UB (Giang ruled) |
| 3 | Dedicated counted-string for keys (len on heap) | REJECTED — deviates from String repr, requires conversion on insert, unwieldy |
| 4 | Amend ADR-0038 Comparable as Hash | **REJECTED** — Ord ≠ Hash, mixing ruins architecture (Giang) |
| 5 | `Hashable` trait + dynamic dispatch | **REJECTED** — Trait system is only Tier-1 static (ADR-0061), building now breaks foundation |
| 6 | Separate key-kind tag in header | REJECTED — `key_stride` already serves as discriminator, extra tag is redundant magic |

## Consequences
**Positive:** `HashMap<String,V>` (name → value maps — foundation of configs, symbol tables, lookups) sound end-to-end; generic KEY column (breaking rigid Integer) = foundation for future key types; reuses fat value machinery from ADR-0078 (0 conceptually new free mechanisms, mirrored for key column). **Negative:** Slot layout changes (base offset + all `*_ptr` helpers shift) → compiler-guided blast radius; `insert` must handle fat keys by-pointer + rehash key-stride-aware. **Risks (⇒ Teeth):** Forgetting to free keys on drop/remove → leaks; forgetting to free redundant keys on update → leaks; key-arg not consumed on insert → double-frees; identity hash slipping into String path → incorrect get-miss semantics; rehash i64-reading fat keys → corruption.

## Teeth (O Verifies Teeth — Poison MUST be Red, cp-Snapshot NEVER git checkout)

> **Author MANDATORY REQUIREMENT:** Must provide poison tests for **① Map drop key leak** and **② Update key leak**.
> Stamped firmly at #1 and #2 below. Measured via counting harness (N7 subprocess `spawn_n7_child`, `--exact --test-threads=1`) — checking FREE count, NOT relying on "no crash".

| # | Tooth | Poison → RED |
|---|---|---|
| 1 💀💀 **MANDATORY** | **Map drop leak key** | Remove key-free slot-loop (Track D.1) → occupied String key `FREE == 0` (leak) via counting |
| 2 💀💀 **MANDATORY** | **Update leak key** | Insert duplicate-content key; remove free-incoming-redundant-key (Track D.2) → moved-in key leaks (`FREE` count short by 1) |
| 3 💀 | Remove leak resident key (Track D.5) | Remove free-resident-key on String-key remove → leak per remove |
| 4 💀 | Insert = Move key double-free | Key-arg consume → false (Track D.3) → caller frees moved-in key → SIGABRT 134 (real allocator, G gold standard) |
| 5 | Content hash/eq correctness | Two String keys with **identical content but distinct allocations** → `get` MUST HIT. Poison string-hash → identity (address) → `get(equal-content)` = `NULL_SENTINEL` (miss) → assert-hit RED |
| 6 | get/remove/contains key = borrow | Poison marks lookup-key as **consumed** → valid program reusing key post-lookup rejected by borrowck / or caller double-frees borrowed key |
| 7 | Rehash key-stride | Poison rehash to use 8B i64-read instead of memcpy `key_stride` → grow + fat key → corrupts `slot_len` → garbage eq |
| 8 | Key-type REFUSAL | `HashMap<Tryte,V>` / `HashMap<Struct,V>` → typecheck E10xx `UnsupportedHashMapKey` (variants: Tryte, struct, enum) |
| 9 | Integer-key backward compatibility | Corpus `HashMap<Integer,V>` insert/get/remove/contains green (fast-path `key_stride==8` preserved) |

## Slices (Symmetric to ADR-0077/0078 A/B)
- **KM-P1a (Backend):** Track A slot key_stride + Track B hash/eq shims + Track D.1/D.2/D.5 key-free/dup-prune/remove-free + rehash key-stride. Verified via hand-built MIR + counting harness.
- **KM-P1b (Typecheck Opening):** Track C generic KEY + REFUSAL + Track D.3 (insert Move key) / D.4 (borrow lookup key). End-to-end source `.tri` + SIGABRT 134 + leak counting.

## Effective Date
Tier C+ when each slice lands (verified by O, signed by G). No retroactive impact on `HashMap<Integer,V>` (fast-path `key_stride==8` preserves byte compatibility).

---

## §AMEND-1 (2026-07-03) — ABI D.2/D.5: "Shim Signals, JIT Emits Free" (COUNTING-INTEGRITY)

**Recon D (KM-P1a), independently verified by O (file:line):** The counting harness substitutes `__triet_string_free` with `__test_counting_free`/`__hp2_count_free` **under the symbol name `__triet_string_free`** in the symbol table (`with_shims` `mir_lower.rs:808-809`). Only calls to `__triet_string_free` **emitted by JIT** (Cranelift `call` via `get_or_declare_shim`:258 / `emit_heap_free_at`:972) resolve to the counter. A free written DIRECTLY inside the Rust shim body (`super::__triet_string_free(...)` inside `__triet_hashmap_insert`/`_remove`) is a static link-time call, **BYPASSING symbol table → counter is BLIND**. Placing D.2/D.5 inside shim bodies ⇒ **teeth #2/#3 are VACUOUS from day one**. Work order instruction ":4250/:4363 free inside body" is **RETRACTED**.

**Mechanics Locked** (matching VALUE move-out precedent `__triet_hashmap_remove`/`__triet_vector_pop` out_ptr, JIT call-site :2952/:2968):
- **D.2 (Insert duplicate key prune):** `__triet_hashmap_insert` **adds out-param `is_update_out: i64`** — shim writes 1/0. JIT call-site (which already has fat key-arg address, like `vector_push`) post-call: `key_stride==24 && flag==1` → emits `__triet_string_free` **registry-routed** on the redundant moved-in key pointer.
- **D.5 (Remove prune resident key):** `__triet_hashmap_remove` **adds out-param `key_out_ptr: i64`** (JIT allocates temporary 24B StackSlot, like `dest_slot` in concat). Shim writes **resident-key fat `{ptr,len,cap}`** there + zero-tombstones key cell in slot. JIT call-site: `key_stride==24` → `emit_heap_free_at` on `key_out_ptr` (registry-routed). **INVARIANT:** resident key ≠ lookup key `k` (different instance, distinct allocation) — FORBID freeing `k` (which belongs to caller, borrowed per D.4) → double-free.

This is an authentic ABI expansion (adding parameters), NOT "minimal churn literal" — but is the SOLE way to maintain counting-testability (Teeth #1-3) mandated by this ADR and G. Invariant in §Track A ("free/rehash requires no external type info, buffer self-describes") REMAINS UNCHANGED.

**Signatures §AMEND-1:** D (recon) · **O ✅** (independently verified mechanics, retracted literal WO, locked resident≠lookup) · **G ✅** (APPROVED. Excellent catch on test-blindness. Mandating out-params to JIT so harness can count. Commit all and issue WO immediately).

---

## Signatures
- **Author (Giang Hoang)** ✅ — Locked D1 (24B fat) · D2/D3 (FNV-1a, minimal magic) · D4 (new ADR, REJECTED Hashable) · D5 (key ∈ {Integer,String}). Mandated: required poison tests for Map-drop-leak + Update-leak.
- **Mentor O** ✅ — Recon file:line, drafted; identified additional death point #5 (remove free resident key).
- **Mentor G** ✅ — APPROVED. Rock-solid design, comprehensive poison tests. O, you are authorized to issue Work Order (KM-P1a) to D. "Code is cheap. Show me the poison tests."
