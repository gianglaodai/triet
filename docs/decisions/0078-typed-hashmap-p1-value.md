# ADR 0078 — Typed HashMap P1 (value-typed: `HashMap<Integer, T>`, T built-in heap)

> # 🩸 NGUYÊN LÝ CỐT LÕI (G khắc đá 2026-06-30)
> # "Mảng thưa chứa chuỗi" (sparse array / ID-lookup table với giá trị String) là
> # nền của mọi data-structure thực dụng. Ownership của VALUE phải sound: nhét vào
> # (insert), lấy ra (remove), chết (drop) — không rỉ một byte. Tái dùng cỗ máy
> # Vector P1 (ADR-0077), KHÔNG phát minh lại bánh xe. KEY-typed = mặt trận khác.

**Trạng thái:** 📝 **DRAFT — chờ implement + O verify máu + G sign-off.** Áp dụng Bậc C+.
Mở `HashMap<Integer, T>` với T = built-in (scalar / String / Vector / HashMap / Nullable tương ứng).
**KEY giữ Integer cứng** (key-typed = ADR sau, đụng hash/eq per-type + Comparable ADR-0038).
Continuation của ADR-0077 (Typed Vector P1) — tái dùng stride / typed-free loop / move-track / by-ptr ABI cho VALUE.

**Sibling/kế thừa:** ADR-0077 (Typed Vector P1 — cỗ máy value-storage tái dùng nguyên), ADR-0060 (P1/P2 tách-tầng),
ADR-0043 (HashMap builtins gốc), ADR-0076 (sentinel-no-op free R4).
**KHÔNG đụng:** key-typed (`HashMap<String, V>` — Tầng 2, defer), native-layout (Option D), ADR-0068 (Box CẤM).

---

## Issue — 3 tầng độ khó (recon O 2026-06-30, file:line)

HashMap hiện **Integer→Integer cứng**. Recon lật ra HashMap KHÔNG phải "Vector pattern × 2" — nó tách 3 tầng:

1. **Tầng 1 — VALUE typing (= Vector y hệt):** value chỉ store/free/move → **đúng cỗ máy Vector** (stride/typed-free/move-track/by-ptr). Reuse trọn.
2. **Tầng 2 — KEY typing (MỚI, "heavy" thật):** KEY cần **hash + equality per key-type**. `mir_lower.rs:4015-4027`: `hash = k % cap` (integer-modulo), `stored_k == k` (i64-eq) — i64-only. String key đòi string-hash (`cap_id_hash`@3155 mẫu FNV-1a) + `__triet_string_eq`. **Vector element KHÔNG bao giờ so sánh; HashMap key PHẢI.** → **DEFER (ADR sau).**
3. **Tầng 3 — typecheck repr:** HashMap = `Type::UserStruct { name:"HashMap", fields:[__key:Integer,__value:Integer] }` (env.rs:336) — KHÔNG dedicated `Type::HashMap(K,V)`. MIR bare `MirType::HashMap` (mir/lib.rs:498).

**HM-P1 = Tầng 1 + Tầng 3.** Tầng 2 trói gô vào backlog.

---

## Quyết định

Mở `HashMap<Integer, T>` (value-typed) qua các mũi, **K=Integer hardcode**, đối xứng Typed Vector P1.

### Mũi A — typecheck repr: đập `UserStruct` → dedicated `Type::HashMap(Box<K>, Box<V>)`
- `types.rs`: thêm variant `HashMap(Box<Self>, Box<Self>)`. Giết `UserStruct{name:"HashMap",__key,__value}` giả cầy.
- `extract_type_params` (check/exprs.rs:2274 mẫu Vector arm): thêm `(HashMap(pk,pv), HashMap(ak,av))` walk cả 2 slot.
- `env.rs`: declare generic `hashmap_new<V>() -> HashMap<Integer,V>` · `insert<V>(HashMap<Integer,V>, Integer, V) -> HashMap<Integer,V>` · `get<V>(HashMap<Integer,V>, Integer) -> V?` · `remove<V>(HashMap<Integer,V>, Integer) -> V?`. K-slot = Integer cứng (KHÔNG type-param cho key).
- MIR `MirType::HashMap` → `HashMap(Box<MirType>, Box<MirType>)` (repr fidelity; chỉ VALUE drives typed-free vì K=Integer Copy). Blast ~ giống Vector MŨI 1 (rustc-guided).

### Mũi B — slot fat-value: inline value bằng value-stride (KHÔNG box)
- Slot hiện `[key8 | value8 | state1 | pad7]` = 24B; value-cell 8B **KHÔNG chứa nổi String fat 24B**.
- **Quyết: inline-grow** (đối xứng Vector, KHÔNG box — box = +alloc +indirection, đã bác ở ADR-0077 §ph.án 2). Slot = `[key8 | value@value_stride | state]`; `value_stride` từ value-type (8 scalar / 24 String) qua **`vector_elem_size` helper tái dùng** (ADR-0077). Probing KHÔNG đổi (key@0, state@offset cố định sau value cell).
- `insert` value-stride-aware: fat value **by-ptr memcpy** (như push@MŨI4); **rehash loop** (`mir_lower.rs:3925`) memcpy value-cell theo stride (KHÔNG `v_ptr.read_unaligned()` i64) — đây là "cày cuốc cẩn thận" G dặn.

### Mũi C — typed drop-glue (JIT-emitted, tái dùng Vector MŨI 3)
- HashMap Drop site: iterate `cap` slot, `state==occupied(1)` → free value@value-cell qua **`emit_heap_free_at`** (registry-routed, đếm được — chống vacuity như Vector). KEY=Integer KHÔNG free. Sentinel-no-op R4.

### Mũi D — move-track + take-out
- **insert = Move value:** `arg_consumes` value-arg element-type-aware (heap→consume, Copy→no-op) — đúng cỗ máy push Vùng 3 ADR-0077 (borrowck move-track + M3-zero + JIT).
- **Take-out = `remove(map,key) -> V?` (shim MỚI):** move-out value + tombstone slot (state→deleted). Ownership cắt đứt (như pop). **`get(HashMap<Integer, heap>)` → E1047 REFUSE** (copy-out heap value, defer clone/borrow — đối xứng Vector get). `get(HashMap<Integer,Integer>)` Copy → vẫn `V?`.

### Ranh giới (defer — đụng là chết)
KEY-typed `HashMap<String,V>` (Tầng 2: hash/eq per-type, Comparable ADR-0038) · get-clone/borrow heap value · `HashMap<_, UserStruct>` (P2 native-layout) · ADR-0068 Box.

---

## Phương án đã cân nhắc
| # | Phương án | Kết luận |
|---|-----------|----------|
| 1 | **Inline value by value-stride** (chọn) | tái dùng Vector machinery, 1 alloc, value-semantics |
| 2 | Box value (cell = 8B ptr→heap value) | bác — +alloc +indirection (như ADR-0077 §2) |
| 3 | Gộp key-typed cùng campaign | bác (G) — Tầng 2 lôi theo Comparable ADR-0038, chết chìm |
| 4 | get-heap-value copy-out | bác — clone-shim/borrow-lifetime chưa có; dùng `remove` move-out |

## Hậu quả
**Tích cực:** sparse-array/ID-table chứa heap value sound; dedicated `Type::HashMap(K,V)` (đập UserStruct giả cầy) = nền cho key-typed sau; tái dùng Vector machinery (0 cỗ máy free mới). **Tiêu cực:** `MirType::HashMap` arity đổi → blast rustc-guided; insert rehash value-stride-aware (bounded). **Rủi ro:** rehash memcpy value sai stride → corruption (teeth); insert không consume heap value → double-free (teeth SIGABRT 134); drop không loop value → leak (teeth).

## Teeth (O verify máu — poison phải đỏ, cp-snapshot KHÔNG git checkout)
| # | Tooth | Poison → RED |
|---|---|---|
| 1 💀💀 | insert heap value SIGABRT 134 (G gold std) | value-arg consume→false → caller double-free (real-allocator) |
| 2 💀 | drop leak | gỡ typed-free slot-loop → occupied String value FREE==0 |
| 3 | rehash value-stride | poison rehash dùng i64-read thay memcpy stride → corruption khi grow + fat value |
| 4 | remove take-out | remove move-out + tombstone → value freed once via caller; poison tombstone → double-free |
| 5 | get-heap refuse | `get(HashMap<Integer,String>)` → E1047 |
| 6 | backward-compat | `HashMap<Integer,Integer>` insert/get/remove corpus xanh |

## Slices (đối xứng Vector A/B)
- **HM-P1a (backend):** Mũi A-MIR + B slot + C typed-free + insert value-stride + remove shim. Verify hand-built MIR + counting.
- **HM-P1b (typecheck-open):** Mũi A-typecheck (`Type::HashMap`, generic builtins) + D move-track + get-heap E1047. End-to-end source + SIGABRT 134.

## Ngày hiệu lực
Bậc C+ khi từng slice landed (O verify máu, G ký). Không hồi tố `HashMap<Integer,Integer>` (fast-path bảo tồn).
