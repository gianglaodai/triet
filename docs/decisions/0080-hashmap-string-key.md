# ADR 0080 — Key-typed HashMap P1 (`HashMap<String, V>`, content hash/eq)

> # 🩸 NGUYÊN LÝ CỐT LÕI (Giang khắc đá 2026-07-03)
> # `Ord ≠ Hash`. Comparable (ADR-0038, `compare() -> Trit`, TỔNG THỨ TỰ) là cỗ xe SAI —
> # trộn thứ-tự vào băm là con đường ngắn nhất làm nát kiến trúc. HashMap cần **Hash + Eq**
> # NỘI DUNG, không phải `< >`. String key mang **NỢ MÁU** (drop-obligation): mỗi key trong
> # slot là một String heap sống. Nỗi lo #1 KHÔNG phải "băm ra số mấy" mà là **HEAP LEAK &
> # DOUBLE FREE**. Không rỉ một byte. CẤM `Hashable` trait, CẤM dynamic dispatch runtime.

**Trạng thái:** ✅ **APPROVED — Author + O + G ký 2026-07-03.** Hiến pháp thông qua; WO KM-P1a phát
cho D. Chưa có code (chưa hồi tố "IMPLEMENTED" — chỉ lên khi slice landed + O verify máu). Áp dụng Bậc C+.
Continuation của ADR-0078 (Typed HashMap P1 value) — mở **Tầng 2 (KEY typing)** mà 0078 gô vào backlog.

**Sibling/kế thừa:** ADR-0078 (value-typed HashMap — cỗ máy value-storage / slot-stride / typed-free
tái dùng NGUYÊN), ADR-0077 (Typed Vector P1 — stride helper), ADR-0049 §6.3 (String repr: heap =
`header + data`, KHÔNG len/cap trên heap), ADR-0079 (get-borrow `&0 container`), ADR-0069 Lát 3
(`cap_id_hash` FNV-1a — mẫu cho string-hash).
**BÁC BỎ làm cỗ xe:** ADR-0038 (Comparable = Ord, không phải Hash). **KHÔNG đẻ:** `Hashable` trait,
key ∈ {Tryte, UserStruct, Enum, …} (REFUSE), native-layout (Option D), ADR-0068 Box.

---

## Issue — recon O 2026-07-03 (file:line)

HashMap hiện **key = identity-hash trên i64**. Đúng cho Integer key (value = i64); SAI cho String key.
Recon lật ra **một bức tường layout thật**, không phải chuyện nối shim:

1. **Key slot CỨNG 8 byte.** `mir_lower.rs:4054` — `hashmap_slot_size = 8 + value_stride + 1`.
   Value có `value_stride` co giãn (ADR-0078); **key thì không** — luôn 8B, đọc/ghi bằng
   `hashmap_key_ptr → *mut i64` (`:4075`).
2. **String KHÔNG lưu `len` trên heap** (ADR-0049 §6.3, `string_layout` `:3428` = `HEADER_SIZE + cap`,
   không len/cap). `{ptr, len, cap}` sống ở **fat pointer 24B trên stack**. Bằng chứng cứng:
   `__triet_string_eq(a_ptr, a_len, b_ptr, b_len)` (`:3542`) **BẮT BUỘC nhận `len` rời** — không đọc
   được len từ con trỏ heap.
   → **Đây là mấu chốt D1:** muốn content-hash/eq một String key ta cần `{ptr, len}`, nhưng slot
   8B chỉ nhét nổi `ptr`. Không có len ⇒ không hash/eq được. Slot key PHẢI rộng ra để chứa fat.
3. **Hash/eq hiện identity.** `:4247` `hash = (k % cap + cap) % cap`, `:4253` `stored_k == k` —
   i64-modulo + i64-eq. Integer key: đúng. String key: hash trên **địa chỉ con trỏ** ⇒ hai String
   cùng nội dung khác allocation ra hai key khác nhau. SAI ngữ nghĩa.
4. **Content-eq CÓ, content-hash CHƯA.** `__triet_string_eq` (`:3542`) đã tồn tại. Chưa có
   `__triet_string_hash`. Mẫu FNV-1a sẵn: `cap_id_hash` (`:3372`).
5. **KEY giờ mang drop-obligation (nợ máu MỚI).** Key = heap String ⇒ phát sinh free-path chưa ai
   làm. Free-loop JIT-emit hiện chỉ free VALUE (`emit_hashmap_value_free_loop` `:1133`). Không đụng
   tới key ⇒ **leak toàn bộ key khi map chết** + hàng loạt death-point khác (§Mũi D).
6. **Typecheck hardcode Integer key.** `env.rs:342–391` khai báo `hashmap_new/insert/remove/get` với
   K = `Integer` cứng; `check.rs:1101` build `Type::HashMap(K,V)`. Mở String key phải generic-hóa
   cột KEY (song song việc HM-P1b `f5c11e1` đã làm cho cột VALUE).

---

## Quyết định (Author chốt D1–D5, 2026-07-03)

Mở `HashMap<String, V>`, **key ∈ {Integer, String} — đóng băng**. V = tập giá trị đã hỗ trợ
(scalar / String / Vector / HashMap / Nullable). Đối xứng cỗ máy value ADR-0078; thêm **cột KEY
đối xứng cột VALUE** + **content hash/eq** + **key drop-glue**.

### Mũi A — slot: `key_stride` song song `value_stride` (D1 = phương án (a), 24B fat trọn ổ)
- Slot mới: `[key@key_stride | value@value_stride | state1]`. `slot_size = key_stride + value_stride + 1`.
  `hashmap_key_ptr = base + idx*slot`; `hashmap_value_ptr = key_ptr + key_stride`;
  `hashmap_state_ptr = value_ptr + value_stride`.
- `key_stride ∈ {8, 24}`: Integer key = 8 (i64), String key = 24 (fat `{ptr,len,cap}` trọn ổ). Chứa
  **cả len** ⇒ hash/eq đọc thẳng từ slot, KHÔNG cần len-trên-heap. **Đây là lý do bắt buộc 24B** —
  gắn thẳng vào requirement hash/eq ở Issue #2.
- **BÁC form 16B `{ptr,len}` bỏ cap:** `__triet_string_free(ptr, cap)` (`:3482`) CẦN cap thật để
  `dealloc` đúng layout. Bỏ cap ⇒ free sai layout ⇒ UB/segfault. "Bóp size thì dễ, vá unsoundness
  thì đổ máu" (Giang). Giữ 24B.
- **Buffer TỰ MÔ TẢ key kind.** `alloc` gánh thêm `key_stride` (song song `value_stride` đang nằm ở
  reserved-word header `:4122`). `key_stride == 24 ⟺ String key ⟺ dùng content hash/eq`;
  `== 8 ⟺ Integer ⟺ identity`. **`key_stride` kiêm luôn discriminator dispatch** — không đẻ tag
  riêng. Cách đóng gói chính xác (byte thứ mấy trong header / body-word / 2 shim monomorphize) =
  **implementer's choice** (D chọn ít-churn nhất), BẤT BIẾN BẮT BUỘC: buffer phải tự mô tả key kind
  để `free`/`rehash` KHÔNG cần type-info ngoài (chúng chỉ có con trỏ). CẤM dynamic trait dispatch.

### Mũi B — content hash/eq (D2 + D3)
- **Hash:** shim MỚI `__triet_string_hash(ptr, len) -> i64` = FNV-1a trên `len` byte nội dung
  (mirror `cap_id_hash` `:3372`, đổi input `&str`→`(ptr,len)`). Deterministic theo nội dung.
  Slot = `(hash % cap + cap) % cap`. **P1 KHÔNG cache hash** (recompute mỗi probe / mỗi rehash từ
  `{ptr,len}` trong slot — key thường là ID nhỏ, tối giản magic).
- **Eq:** probe khi `key_stride == 24` gọi `__triet_string_eq(slot_ptr, slot_len, k_ptr, k_len)`
  (`:3542`) thay `stored_k == k`. `slot_len` có sẵn vì slot chứa fat 24B (Mũi A).
- **Dispatch:** shim rẽ theo `key_stride` (8 → identity path cũ, giữ nguyên; 24 → string path).
  Fast-path Integer BẢO TỒN byte-compat.

### Mũi C — typecheck/borrowck: generic-hóa cột KEY (D4 vehicle + D5 scope)
- `types.rs` / `env.rs:342–391`: key thành type-param `K ∈ {Integer, String}`. Declare
  `hashmap_new<K,V>` · `insert<K,V>` · `get<K,V>` · `remove<K,V>` · `contains<K>`. Seed K từ
  `expected_type_stack` như V (HM-P1b).
- **REFUSE key khác:** K ∉ {Integer, String} → typecheck ĐẬP VỠ MẶT ở biên (mã lỗi mới, ví dụ
  E10xx `UnsupportedHashMapKey`; cấp số cụ thể khi implement). Không skeleton, không defer-mềm.

### Mũi D — QUẢN LÝ NỢ MÁU (Giang: TỐI QUAN TRỌNG) — 5 death-point
String key = heap owned. Mỗi điểm dưới là một mặt trận teeth riêng (teeth phải quét **cả biến thể**):

1. **Map drop → free MỌI key active.** JIT-emit **key-free loop** song song value-free loop
   (`emit_hashmap_value_free_loop` `:1133`): iterate `cap` slot, `state == occupied(1)` → free
   key@key-cell qua registry-routed emit (đếm được, chống vacuity). Integer key (`key_stride==8`) →
   KHÔNG free. Sentinel-no-op R4 (ADR-0076).
2. **Insert TRÙNG key (update) → trảm key dư.** Caller move một String key vào; nếu slot đã có key
   nội-dung-bằng, map GIỮ key cư trú, **key mới (đã move-in) phải `__triet_string_free` NGAY** —
   không có đích về = leak. LOCK: update ⇒ free-incoming-redundant-key, giữ resident.
3. **Insert = Move key.** borrowck/typecheck consume key-arg khi String (Copy no-op khi Integer) —
   đúng cỗ máy move-track value (Mũi D ADR-0078). Key-arg heap không consume ⇒ caller double-free.
4. **get / remove / contains key = BORROW `&0 String`.** Key chỉ để LOOKUP (hash/eq), KHÔNG store,
   KHÔNG consume. Bất đối xứng với insert (by-value Move). Typecheck: param key = `&0 String` cho các
   op đọc; borrowck: key ở lại của caller, caller free bình thường. (Tái dùng mô hình `&0 container`
   ADR-0079 cho arg key.)
5. **remove → free RESIDENT key (O vạch thêm, ngoài 4 điểm Author list).** `remove(map, k)` giết
   entry: value move-out cho caller (như ADR-0078), nhưng **key cư trú trong slot mất đích về → map
   sở hữu nó → phải `__triet_string_free` khi tombstone**. Bỏ sót = leak mỗi lần remove String-key.

**Bất biến rehash (`:4205` branch):** key move theo con trỏ old→new (memcpy **key-cell theo
`key_stride`**, KHÔNG i64-read 8B của fat 24B); `__triet_hashmap_free(old)` chỉ free BUFFER, KHÔNG
đụng nội dung key (chúng đã move) ⇒ không double-free. Poison i64-read → corrupt len → teeth.

### Ranh giới (defer — đụng là chết)
`Hashable` trait người-dùng-định-nghĩa · key ∉ {Integer,String} · get-borrow-mutable key · hash caching ·
`HashMap<_, UserStruct>` (P2 native-layout) · Comparable/Ord land (ADR-0038, mặt trận khác) · ADR-0068 Box.

---

## Phương án đã cân nhắc
| # | Phương án | Kết luận |
|---|-----------|----------|
| 1 | **key_stride 24B fat song song value_stride** (chọn, D1a) | tái dùng cỗ máy fat value; slot có len ⇒ hash/eq sound; free có cap |
| 2 | key 16B `{ptr,len}` bỏ cap | **BÁC** — `__triet_string_free` cần cap ⇒ free sai layout ⇒ UB (Giang chốt) |
| 3 | counted-string riêng cho key (len trên heap) | BÁC — key lệch repr String, +convert khi insert, xấu |
| 4 | amend ADR-0038 Comparable làm Hash | **BÁC** — Ord ≠ Hash, trộn = nát kiến trúc (Giang) |
| 5 | `Hashable` trait + dynamic dispatch | **BÁC** — trait system mới Tier-1 static (ADR-0061), dựng giờ = sụp móng |
| 6 | Tag key-kind riêng trong header | BÁC — `key_stride` đã kiêm discriminator, thêm tag = magic thừa |

## Hậu quả
**Tích cực:** `HashMap<String,V>` (map tên→giá trị — nền của mọi config / symbol-table / lookup thực
dụng) sound end-to-end; cột KEY generic (đập Integer-cứng) = nền cho key-type sau; tái dùng nguyên cỗ
máy fat value ADR-0078 (0 cơ chế free mới về ý tưởng, chỉ nhân đôi cho cột key). **Tiêu cực:** slot
layout đổi (base-offset + mọi `*_ptr` helper dịch) → blast rustc-guided; `insert` phải xử lý fat key
by-ptr + rehash key-stride-aware. **Rủi ro (⇒ teeth):** quên free key khi drop/remove → leak; quên
trảm key dư khi update → leak; key-arg không consume khi insert → double-free; identity-hash lọt vào
String path → get-miss ngữ nghĩa sai; rehash i64-read fat key → corruption.

## Teeth (O verify máu — poison PHẢI đỏ, cp-snapshot KHÔNG git checkout)

> **Author YÊU CẦU BẮT BUỘC:** phải có poison test cho **① Map drop leak key** và **② Update leak key**.
> Ghi cứng ở #1 và #2 dưới. Đo leak bằng counting harness (N7 subprocess `spawn_n7_child`,
> `--exact --test-threads=1`) — FREE-count, KHÔNG dựa "không crash".

| # | Tooth | Poison → RED |
|---|---|---|
| 1 💀💀 **BẮT BUỘC** | **Map drop leak key** | gỡ key-free slot-loop (Mũi D.1) → occupied String key `FREE == 0` (leak) qua counting |
| 2 💀💀 **BẮT BUỘC** | **Update leak key** | insert dup-content key; gỡ free-incoming-redundant-key (Mũi D.2) → key move-in leak (`FREE` thiếu 1) |
| 3 💀 | remove leak resident key (Mũi D.5) | gỡ free-resident-key trên remove String-key → leak mỗi remove |
| 4 💀 | insert = Move key double-free | key-arg consume→false (Mũi D.3) → caller free key đã move-in → SIGABRT 134 (real-allocator, G gold std) |
| 5 | content hash/eq đúng | hai String key **cùng nội dung khác allocation** → `get` phải HIT. Poison string-hash→identity (địa chỉ) → `get(equal-content)` = `NULL_SENTINEL` (miss) → assert-hit ĐỎ |
| 6 | get/remove/contains key = borrow | poison đánh dấu lookup-key **consumed** → chương trình hợp lệ tái dùng key sau lookup bị borrowck từ chối / hoặc caller double-free key mượn |
| 7 | rehash key-stride | poison rehash dùng i64-read 8B thay memcpy `key_stride` → grow + fat key → corrupt `slot_len` → eq garbage |
| 8 | key-type REFUSE | `HashMap<Tryte,V>` / `HashMap<Struct,V>` → typecheck E10xx `UnsupportedHashMapKey` (biến thể: cả Tryte, struct, enum) |
| 9 | Integer-key backward-compat | corpus `HashMap<Integer,V>` insert/get/remove/contains xanh (fast-path `key_stride==8` bảo tồn) |

## Slices (đối xứng ADR-0077/0078 A/B)
- **KM-P1a (backend):** Mũi A slot key_stride + Mũi B hash/eq shim + Mũi D.1/D.2/D.5 key-free/dup-trảm/
  remove-free + rehash key-stride. Verify hand-built MIR + counting harness.
- **KM-P1b (typecheck-open):** Mũi C generic KEY + REFUSE + Mũi D.3 (insert Move key) / D.4 (borrow
  lookup key). End-to-end source `.tri` + SIGABRT 134 + leak-counting.

## Ngày hiệu lực
Bậc C+ khi từng slice landed (O verify máu, G ký). Không hồi tố `HashMap<Integer,V>` (fast-path
`key_stride==8` bảo tồn byte-compat).

---

## §AMEND-1 (2026-07-03) — ABI D.2/D.5: "shim báo hiệu, JIT-emit free" (COUNTING-INTEGRITY)

**Recon D (KM-P1a), O verify độc lập (file:line):** counting harness thay `__triet_string_free` bằng
`__test_counting_free`/`__hp2_count_free` **dưới tên `__triet_string_free`** trong symbol table
(`with_shims` mir_lower.rs:808-809). Chỉ lời gọi `__triet_string_free` **do JIT-emit** (Cranelift
`call` qua `get_or_declare_shim`:258 / `emit_heap_free_at`:972) mới resolve tới counter. Một free viết
TRỰC TIẾP trong thân Rust shim (`super::__triet_string_free(...)` bên trong `__triet_hashmap_insert`/
`_remove`) = static link-time call, **KHÔNG qua symbol table → counter MÙ**. Đặt D.2/D.5 trong thân
shim ⇒ **teeth #2/#3 RỖNG ngay từ đầu** (vacuous-tooth — mẫu lịch sử 3 lần). WO literal ":4250/:4363
free trong thân" **RETRACTED**.

**Khóa cơ chế** (khớp tiền lệ VALUE move-out `__triet_hashmap_remove`/`__triet_vector_pop` out_ptr,
JIT call-site :2952/:2968):
- **D.2 (insert dup-key trảm):** `__triet_hashmap_insert` **+out-param `is_update_out: i64`** — shim
  ghi 1/0. JIT call-site (đã có địa chỉ fat key-arg, như `vector_push`) sau gọi: `key_stride==24 &&
  cờ==1` → emit `__triet_string_free` **registry-routed** trên con trỏ key move-in dư.
- **D.5 (remove trảm resident key):** `__triet_hashmap_remove` **+out-param `key_out_ptr: i64`** (JIT
  cấp StackSlot 24B tạm, như `dest_slot` của concat). Shim ghi **resident-key fat `{ptr,len,cap}`** ra
  đó + tombstone-zero key cell trong slot. JIT call-site: `key_stride==24` → `emit_heap_free_at`
  trên `key_out_ptr` (registry-routed). **BẤT BIẾN:** resident key ≠ lookup key `k` (khác instance,
  khác allocation) — CẤM free `k` (= của caller, borrow D.4) → double-free.

Đây là ABI mở rộng thật (thêm param), KHÔNG "ít-churn literal" — nhưng là cách DUY NHẤT giữ bất biến
counting-testable (Teeth #1-3) mà chính ADR này + G đã mandate. Bất biến §Mũi A ("free/rehash không cần
type-info ngoài, buffer tự mô tả") KHÔNG đổi.

**Chữ ký §AMEND-1:** D (recon) · **O ✅** (verify cơ chế độc lập, retract WO literal, khóa resident≠lookup) · **G ✅** (ĐÃ DUYỆT. Bắt bài test-blindness xuất sắc. Bắt buộc đẩy out-param ra JIT để harness đếm được. Commit toàn bộ và cấp WO ngay).

---

## Chữ ký
- **Author (Giang Hoàng)** ✅ — chốt D1 (24B fat) · D2/D3 (FNV-1a, ít magic) · D4 (ADR mới, BÁC Hashable) ·
  D5 (key ∈ {Integer,String}). Ra lệnh: poison bắt buộc cho Map-drop-leak + Update-leak.
- **Mentor O** ✅ — recon file:line, soạn; vạch thêm death-point #5 (remove free resident key). Chưa
  verify máu (chưa có code — đúng lệnh "không đụng 1 dòng logic").
- **Mentor G** ✅ — ĐÃ DUYỆT. Thiết kế sắt đá, poison đầy đủ. Thằng O, tao cho phép mày phát lệnh Work Order (KM-P1a) cho D. "Code is cheap. Show me the poison tests."
