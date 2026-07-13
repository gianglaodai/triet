# ADR 0083 — Key-Aggregate HashMap (`HashMap<Struct, V>`, structural content hash/eq qua fnptr-in-header)

> # 🩸 NGUYÊN LÝ CỐT LÕI (O đề, chờ G khắc đá)
> # Một `Struct` làm **key** của HashMap phải hash/eq được. NHƯNG hash/eq của key
> # **KHÔNG dính một mảy may nào tới operator `==` / đại số Ł3 Trilean** — nó là
> # **structural content/bit-equality đệ quy trên physical layout** (ADR-0080 đã
> # khắc `Ord ≠ Hash`, content-eq ≠ `==`). Đây là mỏ neo semantics: key-aggregate
> # KHÔNG mở lại đầm lầy Trilean.
> #
> # Tử huyệt KHÔNG phải ngữ nghĩa — mà là **BẪY VA CHẠM KÍCH THƯỚC (Size Collision
> # Trap)**: `String` key có `key_stride == 24` (FatStr = ptr+len+cap). Một
> # `struct{a,b,c: Integer}` cũng **đúng 24B**. Nếu probe-shim disambiguate bằng
> # `key_stride == 24` (như thiết kế đầu của O — BỊ G BÁC), nó đọc 8B đầu struct
> # làm `len`, 8B kế ép thành `ptr`, deref → **SIGABRT / memory corruption toàn
> # tập**. `key_stride` là tổng byte-width, KHÔNG mang cấu trúc → KHÔNG bao giờ
> # được dùng làm discriminator cho aggregate.
> #
> # Cách chặn (G mandate): **fnptr-in-header + null-sentinel**. Hash/eq type-aware
> # do JIT sinh (JIT có `StructLayout`; Rust shim thì không), trao cho probe qua
> # con trỏ hàm **cư trú trong header** (rehash chạy BÊN TRONG insert → fnptr phải
> # reachable từ trong shim). Sự hiện diện `hash_fn != NULL` LÀ discriminator —
> # KHÔNG phải stride. Bảo toàn CẤM-dynamic-dispatch của ADR-0080: fnptr resolved
> # lúc JIT-compile per-key-type, y hệt free-loop JIT-emit per-type.

## Scope

- ✅ **IN (Slice 1):** `HashMap<Struct, V>` — `Struct` làm KEY. Leaves của key struct
  ∈ `{scalar KHÔNG-nullable, String, nested-struct thỏa cùng luật đệ quy}`. Ops:
  `insert` / `get` / `get_ref` / `contains` / `remove` / drop. `V` giữ nguyên miền
  đã hỗ trợ (scalar / String / Vector / HashMap / aggregate-value Slice C ADR-0082).
- ❌ **OUT — REFUSE tường minh (E1048 hoặc JIT-refuse có teeth):**
  - **Enum key** → Slice 2 (defer). Discriminant matching + padding-bits rác +
    variant size-mismatch = mặt trận riêng, cô lập.
  - **Collection-leaf trong key** (`Vector` / `HashMap` field): mutable collection
    làm key = tội ác — user mutate sau insert → hash đổi → phần tử bốc hơi tàng
    hình. Vã E1048.
  - **Nullable-leaf trong key** (`Integer?`, …): sentinel bit-pattern mang ý nghĩa
    đặc biệt; dù bit-eq có thể chạy, KHÔNG mở rủi ro ngữ nghĩa trong Slice 1. Vã
    E1048. Mở dần khi Slice 1 xanh sạch.
  - `Outcome` leaf, và mọi thứ ngoài `is_hashable_leaf`.

## Issue — recon O 2026-07-12 (file:line, `mir_lower.rs`)

1. **Probe = Rust shim nguyên khối.** `__triet_hashmap_insert` (`@5182`) /
   `_get` (`@5309`) / `_get_ref` (`@5350`) / `_remove` (`@5411`) /
   `_contains` (`@5477`) — vòng probe nằm trong Rust `extern "C"`, KHÔNG JIT-emit.
2. **Hash/eq dispatch DUY NHẤT bằng `key_stride`.** `hashmap_key_hash(key_stride,
   k, cap)` (`@5049`): `key_stride > 8 ? __triet_string_hash(FatStr) : identity(k)`.
   `hashmap_key_eq(slot_ptr, key_stride, k)` (`@5067`): `key_stride > 8 ?
   __triet_string_eq : i64 ==`. **`key_stride` là một con số byte-width** — với
   struct key nó KHÔNG cho biết field nào String (content) / Integer (identity) /
   nested. Hai hàm Rust cố định này KHÔNG THỂ tính structural hash/eq.
3. **Size Collision Trap (bằng chứng):** `FatStr` (`@4410`) = 24B; String
   `key_stride == 24` (`c24 @1307`). `struct{3×Integer}` = 24B → `key_stride == 24`
   → va chạm THẬT với nhánh String. → stride KHÔNG bao giờ được là discriminator.
4. **Header hiện 8B, không chỗ chứa type-info.** `HASHMAP_HEADER_SIZE = 8`
   (`@4945`) = `[refcount:u32 @0][packed:u32 @4]` (packed = `key_stride<<16 |
   value_stride`, ADR-0080 Mũi A). `body = ptr.add(HEADER)` (`@5108`),
   `header = body.sub(HEADER)` (`@5146`), `hashmap_layout` (`@4971`).
5. **Gate hiện đang REFUSE aggregate key.** `refuse_hashmap_aggregate_key`
   (`@625`) + `refuse_hashmap_aggregate_kv` (`@601`). Typecheck E1048
   (`exprs.rs:1015`, `env.rs:356/372`) hardcode key ∈ {Integer, String}.
6. **Máy tái dùng (~60-70%):** key marshal by-pointer khi stride>8
   (`@3343/3422/3457`); alloc đã nhận `key_stride` (`@5089`); layout-walk template
   `collect_heap_leaves` (`@433`); key free-loop skeleton `emit_hashmap_key_free_loop`
   (String-only nay); aggregate free-recursion pattern `aggregate_needs_drop` (Slice C).

## Quyết định

### §1 — NỀN SEMANTICS (khóa cứng): key-eq/hash ≠ `==`/Ł3
Key structural equality = **content/bit-equality đệ quy trên physical layout**, tách
HẲN operator `==` (Trilean Ł3) và trait `Comparable` (ADR-0038, `Ord`). Tiền lệ:
ADR-0080 dòng 4 (`Ord ≠ Hash`), `hashmap_key_eq` dùng `__triet_string_eq` (byte
compare) + i64 identity, **KHÔNG chạm `==`**. Hệ quả: key-aggregate KHÔNG đòi hỏi,
KHÔNG đụng, KHÔNG mở lại đại số Trilean. **CẤM** `Hashable` trait, **CẤM** dynamic
dispatch runtime (thừa kế ADR-0080).

### §2 — ABI: Fixed-header 24B + fnptr calling-convention (G MANDATE)
- **Header cố định 24B** (C-ABI, KHÔNG "lúc có lúc không"):
  `[refcount:u32 @0][packed:u32 @4][hash_fn:u64 @8][eq_fn:u64 @16]`. Bump
  `HASHMAP_HEADER_SIZE` 8→24. fnptr @8/@16 tự-align-8.
- **`__triet_hashmap_alloc` đổi signature:** `(len, cap, key_stride, value_stride,
  hash_fn: i64, eq_fn: i64)`; ghi hash_fn@8 / eq_fn@16 vào header sau alloc.
  rehash-internal (`@5203`) truyền lại 2 fnptr từ header map cũ.
- **Null-sentinel (discriminator):**
  - K = Integer / String → JIT truyền `hash_fn = eq_fn = NULL (0)`.
  - K = Struct → JIT truyền `func_addr` của walker vừa emit (§3).
- **fnptr calling-convention (khóa):**
  - `hash_fn(key_ptr: *const u8) -> i64` — trả **raw FNV hash**; Rust shim tự
    `(raw % cap + cap) % cap` (khớp `@5057`). *Lý do (G duyệt tối đa): tách trách
    nhiệm — JIT lo bit-mixing, Rust shim lo table-index mapping (`cap` đã nằm sẵn
    trong thanh ghi shim); không phình ABI walker bằng `cap`.*
  - `eq_fn(slot_key_ptr: *const u8, probe_key_ptr: *const u8) -> i64` — 1=eq, 0=ne.

### §3 — JIT walkers (type-aware, đệ quy `StructLayout`)
- **`emit_struct_key_hash`** — descent theo mẫu `collect_heap_leaves`: leaf
  scalar → mix raw i64 vào FNV; leaf String → `__triet_string_hash(ptr,len)` rồi
  mix; nested-struct → đệ quy. Emit MỘT FuncId per key-layout; địa chỉ qua
  `declare_func_in_func` + `func_addr`.
- **`emit_struct_key_eq`** — đệ quy layout, **short-circuit** ngay leaf lệch:
  scalar → i64-eq (`read_unaligned`); String → `__triet_string_eq`; nested → đệ quy.

### §4 — Key drop-glue đệ quy
`emit_hashmap_key_free_loop` (hiện String-only) → đệ quy `StructLayout` free MỌI
String-leaf (tái dùng `aggregate_needs_drop` + value-free-loop pattern Slice C).
Áp cả (a) map-drop free toàn bộ resident key · (b) `remove` free removed key qua
out-param registry-routed (ADR-0080 §AMEND-1) — aggregate remove-free cũng đệ quy.

### §5 — Typecheck: `is_hashable_leaf` predicate + E1048 biên
Nới E1048 (`exprs.rs:1015`, `env.rs:356/372`) cho Struct key, gate qua predicate
MỚI **`is_hashable_leaf()`**: hợp lệ ⟺ mọi leaf ∈ `{scalar non-nullable, String,
nested-struct thỏa đệ quy}`. Gặp `Vector`/`HashMap`/`Enum`/`Nullable`/`Outcome`
leaf, hoặc Enum-key top-level → **E1048** (thông điệp mới: non-hashable/mutable leaf).

### §6 — Probe dispatch order (LÁ CHẮN va chạm 24B)
`hashmap_key_hash`/`hashmap_key_eq` thêm tham số `hash_fn`/`eq_fn` (caller đọc từ
header truyền xuống). **Thứ tự BẤT DI BẤT DỊCH:**
```
if (fn != NULL)      { call_fn(...) }        // aggregate — type-aware
else if (stride > 8) { FNV(String) }         // String
else                 { identity(Integer) }   // Integer
```
fnptr-check **TRƯỚC** stride-check. Chỉ có thứ tự này mới giữ Struct-24B (fnptr≠NULL)
KHÔNG bao giờ dẫm lên String-24B (fnptr=NULL).

## Death points (mỗi cái kèm TÍN HIỆU LỖI — feedback_failure_mode_precision)
- **DP-1 Collision-24B:** dispatch sai thứ tự (stride trước fnptr) → struct 24B vào
  String-branch → **SIGABRT / corruption**. (§6 chặn.)
- **DP-2 Header-offset sót:** bump HEADER 8→24 mà sót một raw offset → con trỏ lệch
  16B → **memory vỡ vụn** (không nhất định SIGABRT ngay — có thể silent corruption).
- **DP-3 Key-leaf leak:** key free-loop không đệ quy Struct → String-leaf của key
  **LEAK câm** (FREE < N).
- **DP-4 Remove double-free:** remove-key-free đệ quy trùng map-drop → **SIGABRT 134**.
- **DP-5 func_addr unresolved:** setup JIT self-reference sai → **"unresolved
  symbol/relocation" lúc chạy** (Risk #1).
- **DP-6 Vacuous refuse:** predicate/gate neuter mà test vẫn xanh → **"compile
  SUCCEEDED"** (leak/corruption risk).

## Slicing (G chốt)
- **Slice 1 (mở NGAY):** Struct key. Thông đường ống fnptr + header + walkers.
- **Slice 2 (defer):** Enum key.

## Teeth (kế hoạch O verify máu — cp-snapshot, KHÔNG git checkout)
- **★ G-MANDATE COLLISION-TRAP:** key `struct K3{a,b,c: Integer}` (đúng 24B) insert/get
  round-trip đúng. Poison: đảo §6 (bỏ fnptr-check-trước) → **SIGABRT/corruption**.
- **content hash/eq:** cùng nội dung (String-leaf khác địa chỉ) → collide đúng (get thấy);
  poison walker → miss.
- **key drop (counting-tooth THƯỜNG TRỰC, không chỉ fixture-harness):** insert N key
  `struct{name:String,id}` / drop → FREE==N; poison §4 → leak.
- **remove key-free:** remove aggregate-key → đệ quy, no double-free; poison → 134.
- **refuse non-vacuous:** `HashMap<K{v:Vector},_>`→E1048 · `HashMap<Enum,_>`→E1048/refuse
  (đăng ký shim để non-vacuous); poison neuter predicate → "compile SUCCEEDED".
- **func_addr spike (Risk #1, làm TRƯỚC walker):** JIT nhỏ nhất trả 1 hằng số, lấy
  `func_addr`, truyền xuống Rust shim, in ra → chứng minh relocation chạy TRƯỚC khi
  viết recursive walker (G mandate: fail fast).

## Consequences
- **+** Hoàn tất đối xứng "cái gì làm value" ↔ "cái gì làm key" cho HashMap; Struct
  key sound end-to-end qua JIT real-allocator, không rỉ byte.
- **+** Mở đường ống fnptr-in-header + JIT self-reference — hạ tầng tái dùng cho
  Slice 2 (Enum key) và bất kỳ per-type dispatch tương lai.
- **−** Header +16B mỗi HashMap (mọi map, kể cả Integer/String — trade cho ABI cố
  định, không phân mảnh). Chấp nhận: thủ là sống.
- **−** Lần đầu JIT tự-tham-chiếu `func_addr` — rủi ro relocation (DP-5), de-risk
  bằng spike-first.
- **Bất biến thừa kế:** INV-B-α (ADR-0082, một layout hai nhà 8B-granular) — key
  struct trong slot HashMap = byte-image struct trong StackSlot; `Ord ≠ Hash`
  (ADR-0080); CẤM dynamic dispatch.

---

**Chữ ký:** O đề (2026-07-12). ABI (§2/§6 fixed-header + null-sentinel + dispatch
order) do **G mandate** sau khi BÁC thiết kế stride-branch đầu của O (Size Collision
Trap). Contract fnptr (§2: hash=raw i64, eq=1/0) + biên `is_hashable_leaf` chặn
Nullable-leaf (§5) **G duyệt tối đa**. Author (Giang) chốt hướng (mặt trận ②
key-aggregate). **G ĐÃ KÝ (Mentor G - 2026-07-12). Mọi điều khoản trong ADR này là LUẬT. Zô, xuất quân.**

---

## §AMEND-1 — Slice 2: Enum keys (`HashMap<Enum, V>`) — O đề 2026-07-13, chờ G co-sign

Slice 2 đã được scope-defer trong ADR gốc (Scope §OUT). Bản chất ABI **KHÔNG đổi**
(vẫn fnptr-in-header + null-sentinel + §6 dispatch); chỉ đổi **ruột JIT walker** từ
straight-line-leaf-fold (struct) → **disc-switch-brif-chain** (enum). KHÔNG đẻ ADR mới.

### Scope (G ruling 2026-07-13)
- ✅ **IN:** enum làm KEY (`HashMap<Enum,V>`); **enum làm LEAF của struct-key** (`struct{tag: MyEnum}`); **nested enum** (variant payload là enum) — tất cả qua **đệ quy đồng nhất**, giới hạn **depth-64** (tràn stack JIT lúc compile walker = lỗi implementer). Unit/scalar variants OK.
- ❌ **OUT (GIỮ REFUSE E1048):** **`Enum?` (Nullable enum) key** — Nullable mang sentinel bit-pattern riêng (`NULL_SENTINEL` chọc vào `tag` = bão); Slice 1 cấm Nullable-leaf thì Slice 2 không ngoại lệ. Vector/HashMap/Outcome leaf vẫn refuse.

### §A1 — NGUYÊN LÝ (thuốc giải cho garbage/padding/size-mismatch)
Walker hash/eq **CHỈ đụng `disc@0` + declared leaves của ACTIVE variant** (qua disc-switch),
**KHÔNG BAO GIỜ** đọc raw fixed-width image. Inactive/padding bytes = garbage stale (fixed-width
tagged-union: `total_size` = max variant, tail của variant nhỏ không được ghi lại khi reassign)
→ **không đọc = không phá.** Đây đúng cơ chế `emit_enum_drop_glue_at` (mir_lower:1886) đang dùng —
free CHỈ heap-payload của active variant, never touches inactive garbage. Slice 2 MIRROR pattern đó.

### §A2 — Walkers (disc-switch)
- **hash:** load `disc@0` → **mix disc vào FNV** (disc LÀ phần identity: 2 variant khác phải hash khác) → brif-chain over variants → active arm mix payload leaves @`payload_off=8`. Unit variant = chỉ disc.
- **eq:** load `disc_a@0`/`disc_b@0` → **disc khác → NE ngay** (short-circuit) → disc bằng → brif active arm → so declared leaves @+8; leaf lệch → NE.
- **`collect_key_leaves` enum path:** hiện struct-only flat (`:554`). Enum KHÔNG flatten phẳng (variant-dependent) → per-variant leaf collection tại `payload_off=8` (scalar/String/nested-struct/nested-enum recurse, depth-64).

### §A3 — ABI + free-loop = TÁI DÙNG
Header/§6/collision-shield/marshal-by-pointer/`func_addr`/`walker_ids` memoise (keyed enum-name) =
Slice 1 verbatim. **Key free-loop §4 = REUSE THẲNG `emit_enum_drop_glue_at`** (disc-switch active-variant
free đã đúng). Gỡ `refuse_hashmap_enum_key:880`; `is_hashable_key/leaf` (types.rs:163/177) mở Enum
(⟺ mọi variant payload hashable); overload wiring `exprs.rs:1190` thêm nhánh Enum (insert/remove đã generic).

### §A4 — Death points (mỗi cái + tín hiệu + tooth)
- **DP-E1 disc bỏ khỏi hash/eq** → 2 variant payload-giống collide/eq-nhầm → **silent wrong**. Tooth: insert V1, get V2 → MISS.
- **DP-E2 garbage inactive bytes vào hash/eq** → key bằng-nội-dung nhưng rác khác → hash/eq khác → **data mất câm**. **Tooth ép-rác (G mandate, xem §A5).**
- **DP-E3** = DP-E2 phía eq (false-inequality).
- **DP-E4 padding trong active payload** → chỉ walk declared leaves (không raw range) = an toàn như struct.
- **DP-E5 key free-loop enum heap-payload** (variant ôm String) → REUSE drop-glue; tooth counting FREE==N, poison → leak/double-free.
- **DP-E6 collision §6** (enum `total_size` có thể =24) → fnptr-first shield; tooth SIGSEGV khi đảo dispatch.

### §A5 — TOOTH ÉP-RÁC (G mandate cho DP-E2) — reassign-force-garbage
Cách ép garbage tàn nhẫn nhất qua source Triết:
```
let k = MyEnum::BigVariant(999, 888, 777);   // slot bôi kín data
k = MyEnum::SmallVariant(1);                 // tag+payload nhỏ ghi đè; tail {888,777} VẪN RÁC
let m2 = insert(m, k, 42);                    // key ngậm rác
let k_clean = MyEnum::SmallVariant(1);        // fresh, tail sạch
get(&0 m2, k_clean);                          // HIT=walker bỏ rác (sống) · MISS=nhai rác (chết)
```
Poison walker (thêm 1 leaf raw-range vào tail, hoặc hash cả fixed-width) → **MISS = RED**.
**⚠️ CẢNH BÁO O (bài học Slice 1 ptr-mix-vacuous):** nếu lowerer ZERO cả slot khi reassign → tail=0 →
tooth VACUOUS (rác không tồn tại). Nếu D thấy tooth không đỏ khi poison → **escalate O** (LUẬT 4),
KHÔNG kết luận "an toàn": hoặc (a) reassign-zero tự-mitigate hazard (cần white-box walker-output test
thay thế), hoặc (b) đường ép-rác khác (2 construction-history khác nhau). Phân định bằng probe, không xác suất.

### Teeth Slice 2 (kế hoạch O verify máu — cp-snapshot, KHÔNG git checkout)
DP-E1 (insert-V1-get-V2 MISS) · **DP-E2 reassign-garbage HIT** (§A5, poison→MISS) · DP-E5 enum-key-String-leaf
free counting (poison→leak) · DP-E6 §6-reverse SIGSEGV · enum-as-struct-leaf roundtrip · nested-enum roundtrip ·
`Enum?`-key → E1048 (non-vacuous) · unit-variant enum key (disc-only) roundtrip.

**Chữ ký §AMEND-1:** O đề (2026-07-13). Scope (enum-leaf ✅ · nested-enum ✅ depth-64 · `Enum?` ❌ REFUSE)
+ tooth ép-rác reassign (§A5) do **G ruling/mandate 2026-07-13**.

**G ĐÃ KÝ §AMEND-1 (Mentor G - 2026-07-13). Enum keys xuất chiến!** Mọi điều khoản §AMEND-1 là LUẬT. G ngồi đợi kết quả DP-E2 ép-rác — không SIGSEGV lãng xẹt.
