# ADR-0041: Nullable (`T?`) Representation — Bậc A

**Status:** Mentor O đã ký (semantics & soundness, 2026-06-06). Chờ ký Mentor G (layout, ABI, codegen).
**Date:** 2026-06-06
**Author:** AI (khảo sát + đề xuất), quyết định cuối: Giang Hoàng
**Reviewers:** Mentor G (layout, ABI, codegen), Mentor O (semantics, soundness)
**Scope:** chỉ `T?` thuần. KHÔNG đụng `T~E` / `T?~E` (Outcome cần packed ABI,
defer Bậc C — guard hiện hành ở `triet-jit/src/mir_lower.rs:758-789`).

---

## Tóm tắt

Quyết định biểu diễn runtime cho `T?` ở Bậc A ("mọi giá trị = 1×i64"), để mở
khóa builtin `get(vector, index) -> Integer?` — consumer đầu tiên của nullable
trong backend mới. ADR này trình bày **6 phương án** (PA-1 … PA-6) với phân
tích soundness từng cái. Sau 2 vòng review, cả hai mentor chốt **PA-3c
(uniform sentinel)**: `NULL_SENTINEL = i64::MIN` cho **mọi** `T?` — scalar lẫn
heap. Kèm theo: read-shim trap-on-0 (defense-in-depth cho heap), canary N1 ràng
vào `triet_core::Integer::MIN`, và addendum ADR-0001 sửa cả bảng gán trit lẫn
điều khoản `T??`.

## Động lực

1. **`get` là cổng vào tiếp theo.** Vector Bậc A (4.3b) có `push`/`len` nhưng
   KHÔNG có cách đọc phần tử. `get` phải total (không panic — safety contract
   `feedback_explicit_strictness`: property access 100% safe), nghĩa là trả
   `Integer?`. Không có representation cho `Integer?` thì không có `get`.
2. **ADR-0039 (`?`-family) design-locked nhưng implementation deferred** vì
   "Backend hiện chưa lower được cả `?.`" — mọi operator họ `?` đều chờ
   representation này.
3. **ADR-0040 §6 đã flag tồn đọng:** "Nullable String: **chưa thiết kế
   representation.** Lưu ý xung đột sentinel-0 (moved-out ≡ null value)."
   ADR này trả món nợ đó — và chọn giải pháp triệt để: uniform MIN để
   moved-out (0) và null (MIN) **không bao giờ trùng**.

---

## §0 — Dữ kiện đã verify (không phỏng đoán)

| # | Dữ kiện | Vị trí | Hệ quả thiết kế |
|---|---------|--------|-----------------|
| F1 | `Integer` = 27 trit, range `±3_812_798_742_493` ≈ ±2^41.8 | `triet-core/src/integer.rs:39-42` | Carrier i64 rộng hơn range hợp lệ ~4 triệu lần → có "niche" khổng lồ cho sentinel |
| F2 | `Tryte` range `±9_841`, `Trit` `±1`, `Trilean` 3 giá trị | `triet-core/src/tryte.rs:39-42`, `trit.rs` | Mọi scalar Triết đều có niche trong i64 |
| F3 | `Long` = 81 trit, MAX ≈ 2.2×10³⁸ — **không vừa i64** | `triet-core/src/long.rs:53-54` | Long chưa từng là giá trị Bậc A hợp lệ → `Long?` defer vô điều kiện |
| F4 | JIT arithmetic là **raw i64**: `BinOp::Add → iadd`, `Mul → imul`, `__triet_pow` dùng `wrapping_mul` | `triet-jit/src/mir_lower.rs:1124-1126,1251-1270` | Range ternary KHÔNG được enforce ở runtime → niche không được "canh cổng" (debt D1, §6.2) |
| F5 | Heap value = 1×i64 body_ptr; moved-out được zero (M1-M4); `free(0)` = no-op | ADR-0040 §1.3, §2.5; `mir_lower.rs` Drop handler | `free(0)` no-op GIỮ NGUYÊN — Drop của giá trị đã-move phải êm (C4) |
| F6 | Enum đã compile được: `StackSlot` + `EnumLayout` (disc i64@0, payload@8), match hoạt động (fixtures 25-32) | `mir_lower.rs:168-173,511-546`; ADR-0037 | Máy móc tagged-union có sẵn nếu chọn two-word repr |
| F7 | Shim C ABI: ký số cố định `fn_1_0/fn_1_1/fn_2_1` — trả về **đúng 1×i64** | `triet-driver/src/main.rs:123-141` | Shim không trả được giá trị 2-word; muốn thì phải out-param |
| F8 | Typecheck đã có `Type::Nullable(Box<T>)` + widening `T ⊂ T?`; `Type::Outcome{allow_null_state}` riêng | `triet-typecheck/src/types.rs:44,97-104,165-203` | Frontend sẵn sàng; chỉ thiếu lowering + repr |
| F9 | MIR type là **string** (`LocalDecl::ty: String`); `is_copy` default-Move cho unknown; canonical `is_vec_type()` ở triet-mir (bài học 4.3c) | `triet-mir/src/lib.rs:163-174,2047-2073` | `"Integer?"` sẽ rơi vào default-Move nếu không thêm rule — phải có `is_nullable_type()` canonical. **Va chạm:** `is_vec_type("Vector<Integer>?")` = true (§5.1) |
| F10 | MIR đã có `OutcomeDiscriminant/OutcomeUnwrap/OutcomeUnwrapError` nhưng JIT **từ chối** chúng (chưa reachable) | `triet-mir/src/lib.rs:245-274`; `mir_lower.rs:758-789` | Có chỗ đậu MIR-level nếu cần statement riêng; guard hiện hành không được gỡ bởi ADR này |
| F11 | Discriminator semantics LOCKED: `Trit::Positive` = value, `Trit::Zero` = null, `Trit::Negative` = reserved (T?) / error (T?~E) | ADR-0020 §10.1 | Encoding logic phải theo cực trit này |
| F12 | Pattern match `T?` bắt buộc arm tường minh `~+ binding` / `~0` (E1032) | ADR-0020 §10.4 | Match codegen chỉ cần 2 arm, không widening trong pattern |
| F13 | `is_copy` trả `true` cho `"?"` trần (type-unknown); `is_vec_type` dùng `starts_with("Vector<")` — nuốt `"Vector<Integer>?"` | `triet-mir/src/lib.rs:2047-2054` | Helper `is_nullable_type` phải hỏi TRƯỚC mọi phân loại khác; `"?"` trần phải được pin là **không** nullable (§5.1) |

### §0.1 — Ghi chú mâu thuẫn ADR-0001 vs ADR-0020 (cần addendum)

ADR-0001 (2026-04, body gốc) chứa **hai** điều khoản đã bị các ADR khóa sau
này ghi đè:

1. **Bảng gán trit:** ADR-0001 gán `is_null: -1 = null, +1 = present,
   0 reserved "uninitialized"`. ADR-0020 §10.1 (2026-05-17, mới hơn, LOCKED)
   gán `+1 = value, 0 = null, -1 = reserved/error`. Hai bảng mâu thuẫn;
   addendum v0.7.4.3 của ADR-0001 nói "no change to memory layout" nhưng
   thực tế **vị trí trit của null đã đổi** (−1 → 0).

2. **Điều khoản `T??`:** ADR-0001 "Hậu quả" ghi nguyên văn: "`T??` không
   flatten — hai tầng phân biệt được". ADR-0039 (2026-06-05, LOCKED, mới
   hơn) + C6 của ADR này: "`T??` không tồn tại — auto-flatten". Hai văn bản
   locked mâu thuẫn lần thứ hai.

ADR-0041 theo các nguồn mới hơn (ADR-0020 §10.1, ADR-0039). **Đề nghị:** nếu
ADR này được duyệt, ghi addendum vào ADR-0001 chốt cả hai điều: (a) bảng gán
trit theo ADR-0020 §10.1; (b) `T??` auto-flatten theo ADR-0039. Sửa một lần
cả hai — không để lần sau ai đó lại viết §0.1 thứ ba cho cùng một file.

---

## §1 — Ràng buộc thiết kế

| # | Ràng buộc | Nguồn |
|---|-----------|-------|
| C1 | Semantic model của `T?` là **discriminator + payload** (ADR-0001). Sentinel/niche được phép **chỉ với tư cách optimization không đổi semantic** — ADR-0001 cho phép tường minh: "như Rust niche optimization … không thay đổi semantic" | ADR-0001 |
| C2 | Cực discriminator: `+` = value, `0` = null (theo §0.1) | ADR-0020 §10.1 |
| C3 | Bậc A ABI: mỗi giá trị 1×i64; shim trả 1×i64 (F7) | ADR-0040 §2.5 |
| C4 | M1-M4 zeroing dùng giá trị **0** trên heap ptr; `free(0)` no-op (F5). **Drop của biến đã moved phải êm.** | ADR-0040 §1.3 |
| C5 | `is_copy` quyết định theo type-string, default-Move, helper canonical đặt ở triet-mir (F9, bài học 4.3c) | HANDOFF 4.3c |
| C6 | `T??` không tồn tại — auto-flatten | ADR-0039 |
| C7 | JIT arithmetic KHÔNG wrap về range ternary (F4) — mọi lập luận "giá trị nằm ngoài range không thể xuất hiện" chỉ đúng *modulo* món nợ arithmetic-fidelity có sẵn | F4 |
| C8 | Compiler never panics: mọi case chưa hỗ trợ → `Err(LowerError)` có span | Track B rule 1 |
| C9 | **Shim trap-on-0:** dưới PA-3c, 0 = dead value (moved-out / OOM) — không bao giờ là giá trị heap hợp lệ. Mọi shim nhận heap value làm input (kể cả read hay consume) đều **trap** khi nhận 0. Ngoại lệ duy nhất: `free(0)` **giữ no-op** — Drop của giá trị chết phải êm. | PA-3c (§6.1) |

---

## §2 — Không gian giải pháp (viết hết, kể cả cái sẽ bác)

### PA-1 — Sentinel/niche thuần: `T?` = 1×i64, null = giá trị ngoài range của T

- **Scalar** (`Integer?`, `Tryte?`, `Trit?`, `Trilean?`): null = hằng số
  `NULL_SENTINEL = i64::MIN`. Hợp lệ vì range ternary của mọi scalar Triết
  chừa trống tuyệt đại đa số carrier i64 (F1, F2) — không cắt một giá trị
  hợp lệ nào của T → KHÔNG vi phạm "phạm vi đối xứng" mà ADR-0001 dùng để
  bác sentinel (lập luận đó áp cho phần cứng ternary nơi mọi pattern n-trit
  đều có nghĩa; carrier i64 của Bậc A thì không).
- **Heap** (`String?`, `Vector?`): null = ptr 0 (precedent Rust
  `Option<Box<T>>` null-niche).

**Tính chất đẹp nhất:** widening `T ⊂ T?` và constructor `~+ e` là
**identity** — không codegen gì cả. `let x: Integer? = 5` là chính số 5.
Function boundary, shim ABI, M1-M4, borrowck — tất cả nguyên trạng vì vẫn
là 1×i64.

| Pro | Con |
|-----|-----|
| Zero máy móc mới ở JIT/ABI; `get` shim trả thẳng 1×i64 | **D1:** arithmetic raw i64 (C7) về lý thuyết có thể "chế" ra đúng `i64::MIN` → phantom null (phân tích §6.2) |
| Widening/`~+` = no-op | Heap: null 0 ≡ moved-out 0 → mất phân biệt defense-in-depth (§6.1) |
| `Integer?` qua user-fn boundary tự do (Copy, 1×i64) | Mỗi kind cần bảng sentinel riêng (heap 0, scalar MIN) — lowerer phải biết type tại `~0` |
| Match/Elvis = 1 phép so sánh | Debugger thấy `-9223372036854775808` thay vì "null" — ergonomics kém |

### PA-2 — Tagged two-word: `T?` = enum nội sinh 2-variant qua máy ADR-0037

Lowerer synthesize `EnumLayout` cho mỗi `T?`: disc i64@0 (giá trị **+1/0**
theo C2 — `EnumLayout.discriminant_value: i64` cho phép gán tùy ý, không
bắt buộc 0/1/2), payload i64@8, sống trong `StackSlot` như enum thường (F6).

| Pro | Con |
|-----|-----|
| **Không có D1** — disc tách bạch, không nhờ range | `T?` không qua được call boundary (enum param/return chưa có — open item "sret-like by-pointer") → cần B-refusal mới cho `T?` param/return user-fn |
| Match `~+/~0` rơi ra gần miễn phí từ enum match codegen | `get` shim không trả được 2-word (F7) → cần out-param: `__triet_vector_get(vec, idx, out_ptr) -> disc`, thêm arity `fn_3_1` |
| Trung thành tuyệt đối với ADR-0001 (disc vật lý hiện hữu) | `String?` đụng tường B8 (enum payload Move-type bị refuse) → hoặc mở B8 riêng hoặc hybrid hóa |
| Disc dùng cực trit +1/0 → sẵn sàng cho `T?~E` thêm cực −1 sau này | Widening/`~+` thành slot-store; Copy semantics của `Integer?` cần copy 16 byte slot — máy móc Assign-giữa-slot phải kiểm lại |

### PA-3 — Hybrid theo kind (per-type niche selection — đúng chiến lược layout của Rust)

Chia theo bản chất payload:

- **PA-3a (hybrid 2-sentinel):** heap `T?` = null-ptr 0; scalar `T?` = sentinel
  `i64::MIN`. Mỗi kind một sentinel — lowerer phải biết type để chọn sentinel
  đúng khi lower `~0`. **Bị bác bởi cả hai mentor (Q3).**

- **PA-3b (zero-debt):** heap `T?` = null-ptr 0; scalar `T?` = enum-slot
  PA-2. Không có D1, đổi lấy: scalar nullable bị nhốt trong function
  (B-refusal mới) + out-param shim. **Bị bác:** trả tiền thật cho hàng sẽ vứt
  khi Bậc C packed ABI đến.

- **PA-3c (uniform sentinel — ĐƯỢC CHỌN):** `NULL_SENTINEL = i64::MIN` cho
  **mọi** `T?` — scalar lẫn heap. Moved-out vẫn 0 (C4) → null (MIN) và dead
  (0) phân biệt vĩnh viễn. Ưu điểm so với PA-3a: (i) uniform → `~0` lowering
  type-agnostic, đơn giản hơn; (ii) defense-in-depth cho heap — 0 là dead
  value, read-shim trap khi gặp nó. Chi tiết §5–§6.

### PA-4 — Boxed nullable: mọi `T?` = heap object {disc, payload}, ptr-or-0

**Bác ngay, ghi lại để khỏi ai đề xuất lại:** allocation cho từng `Integer?`;
`Integer?` biến thành Move type (phá kỳ vọng Copy của scalar, lan vào
borrowck); cần Drop machinery cho scalar nullable; chậm vô cớ. Không có ưu
điểm nào PA-2 không có.

### PA-5 — SSA pair splitting: lowerer tách `T?` thành 2 MIR local (disc, payload)

Mọi thứ vẫn là i64 Variable (không StackSlot). Lowerer quản lý cặp local
như một giá trị logic.

| Pro | Con |
|-----|-----|
| Không StackSlot, không sentinel, không D1 | MIR `Place`/`Assign` là 1-place — mỗi assign nullable thành 2 statement, **mọi pass phải biết cặp này là một** |
| JIT codegen rẻ | Borrowck: 2 local = 1 biến logic → E2420 reporting, VarState, local_names đều phải pair-aware — đụng sâu vào checker đang chạy tốt |
| | Call boundary vẫn kẹt (2 giá trị); shim vẫn cần out-param |
| | Là một dạng "scalar replacement of aggregates" làm sớm — máy móc xuyên 3 crate cho một type duy nhất |

Chi phí lan tỏa (lowerer + borrowck + JIT đều phải hiểu "cặp") vượt xa cái
nó tiết kiệm so với PA-2. Không khuyến nghị, ghi lại làm đối chứng.

### PA-6 — Né nullable: ship `get_or(vec, i, default)` / `has(vec, i)` trước

**Bác:** (1) handoff đã chốt thứ tự "nullable repr → get"; (2) `get -> T?`
là hình dạng SPEC-aligned, `get_or` là API tạm sẽ phải deprecate — vi phạm
"stability over speed"; (3) không trả được món ADR-0039 đang chờ. Né hôm
nay = trả lãi kép ngày mai.

---

## §3 — Bảng so sánh

| Tiêu chí | PA-1 | PA-2 | PA-3c (CHỌN) | PA-3b | PA-4 | PA-5 |
|---|---|---|---|---|---|---|
| Soundness debt mới | D1 | 0 | D1 | 0 | 0 | 0 |
| Máy móc mới (lower/JIT) | ~0 | trung bình | nhỏ | trung bình | lớn | lớn (3 crate) |
| `get` shim | trả thẳng i64 | out-param fn_3_1 | trả thẳng | out-param | trả ptr | out-param |
| `Integer?` qua user-fn | ✅ tự do | ❌ refuse | ✅ | ❌ refuse | ✅ (nhưng Move!) | ❌ |
| `String?` | ✅ (0-niche) | ❌ B8 | ✅ (MIN-niche) | ✅ | ✅ | ⚠️ |
| Elvis `?:` | so sánh sentinel | tái dùng enum match | so sánh MIN | mixed | load+so sánh | so sánh disc local |
| Widening `T ⊂ T?` | **no-op** | slot store | **no-op** | mixed | alloc! | copy disc+payload |
| Defense-in-depth heap moved≢null | ❌ | n/a | ✅ (trap-on-0) | ❌ | ❌ | ✅ |
| `~0` lower cần biết type? | có (2 sentinel) | không | **không** (uniform) | có | không | không |
| Đường lên Bậc C packed ABI | thay sentinel→packed | repr đã gần packed | thay sentinel→packed | mixed | vứt | vứt |

---

## §4 — Quyết định cuối cùng (sau 2 vòng review, cả hai mentor đồng thuận)

**Phương án được chọn: PA-3c (uniform sentinel).** `NULL_SENTINEL = i64::MIN`
cho mọi `T?` — scalar lẫn heap.

| # | Câu hỏi | Kết quả | Mentor |
|---|---------|---------|--------|
| Q1 | Chấp nhận D1 (phantom-null bounded by arithmetic debt)? | **Chấp nhận.** Lập luận §6.2 đứng vững: mọi đường tới `i64::MIN` đều đi qua chương trình đã-sai-theo-SPEC (F4), và D1 chết tự nhiên khi arithmetic wrap mod-3²⁷. Điều kiện: đủ 3 nghĩa vụ §6.2 + canary N1. | G ✓, O ✓ |
| Q2 | Sentinel: `i64::MIN`? | **`i64::MIN`.** Canary N1 ràng vào `triet_core::Integer::MIN` — tối hậu thư của G, O đồng ký. Cấm hardcode hai lần. | G ✓, O ✓ |
| Q3 | Heap null: 0 (PA-3a) hay `i64::MIN` (PA-3c)? | **PA-3c — uniform `i64::MIN`.** Kết luận của G thắng; lập luận sửa theo O (conservative=true over-rejects chứ không leak; defense-in-depth thật đến từ trap-on-0 — §6.1). Nghĩa vụ: mọi read-shim trap khi nhận 0; `free(0)` giữ no-op. | G ✓, O ✓ (kết luận) |
| Q4 | Match `~+/~0` vào Bậc A, hay chỉ Elvis `?:`? | **Chỉ Elvis + widening + `~0`.** Match → Bậc B. G chặn đúng: kỷ luật scope, làm `get(v,2) ?: -1` cắn cả tĩnh lẫn động trước đã. O rút đề xuất ban đầu. | G ✓, O ✓ (nhượng G) |
| Q5 | ADR-0001 addendum? | **Có.** Scope mở rộng theo R2: sửa cả bảng gán trit (theo ADR-0020 §10.1) lẫn điều khoản `T??` (auto-flatten theo ADR-0039) trong một lần. | G ✓, O ✓ |

---

## §5 — Đặc tả chi tiết (PA-3c uniform sentinel)

### 5.1 — Hằng số + helper canonical (triet-mir, một nguồn sự thật — bài học 4.3c)

```rust
// triet-mir/src/lib.rs
/// Sentinel encoding the `~0` (null) state of **all** `T?` at Bậc A
/// (scalar and heap — uniform). INVARIANT: lies outside every Triết
/// scalar range (see canary test N1, §9).
pub const NULL_SENTINEL: i64 = i64::MIN;

/// `"Integer?"` → true. Canonical — mọi crate dùng cái này, cấm ad-hoc
/// `ends_with("?")` rải rác (đúng pattern `is_vec_type`).
///
/// **Ordering rule:** is_nullable_type MUST be called BEFORE any other
/// type-string classifier (is_vec_type, etc.) at every consumer.
/// Reason: `"Vector<Integer>?"` starts with `"Vector<"` and would be
/// misclassified as a bare Vector by `is_vec_type`.
///
/// **Pin:** `is_nullable_type("?")` MUST return `false`. The bare `"?"`
/// type-string means "type unknown" (is_copy treats it as Copy) — it
/// must NOT be classified as "nullable of empty string."
pub fn is_nullable_type(ty: &str) -> bool;

/// `"Integer?"` → `Some("Integer")`; non-nullable → `None`.
///
/// **Pin:** `nullable_payload("Vector<Integer>?")` MUST return
/// `Some("Vector<Integer>")` — `is_vec_type` must NOT consume
/// a nullable type-string. Verify in N2.
pub fn nullable_payload(ty: &str) -> Option<&str>;
```

Type-string format: `<payload>?` — `"Integer?"`, `"String?"`. (Type-string
của Outcome chưa tồn tại trong MIR và KHÔNG được định nghĩa ở đây.)

`is_copy` thêm một nhánh **trước** default-Move, và **trước** mọi phân loại
khác (ordering rule trên):

```text
is_nullable_type(ty) → nullable_payload(ty) = Some(p) → is_copy(p, body)
```

→ `"Integer?"` Copy, `"String?"` Move. `"String?"` Move ⇒ B7/B8 refusals
hiện hành **tự động** áp dụng cho nó (không cần code mới).

### 5.2 — Sentinel (uniform)

| Loại | Null encoding | Ghi chú |
|------|---------------|---------|
| **Mọi `T?`** (scalar + heap) | `NULL_SENTINEL` (`i64::MIN`) | Uniform — `~0` lowering type-agnostic |
| `Long?` | — | **Refuse** (`Err`): Long không tồn tại ở Bậc A (F3) |

Hệ quả của uniform sentinel:

- **0 không bao giờ là null.** 0 = dead value (moved-out hoặc OOM). Dưới
  PA-3c, heap pointer hợp lệ không bao giờ là 0 (null là MIN, không phải 0).
- **`~0` lowering type-agnostic:** luôn `iconst(i64::MIN)`, không cần biết
  payload type — đơn giản hơn PA-3a (vốn cần chọn 0 cho heap, MIN cho scalar).
- **Elvis null-check:** `icmp eq val, i64::MIN` — một phép so sánh, không cần
  biết type.

### 5.3 — Lowering từng construct

| Construct | Lowering | Ghi chú |
|---|---|---|
| `~0` (`Expr::NullLiteral`) | `iconst(NULL_SENTINEL)` | Type-agnostic (luôn MIN). **Vẫn cần expected type** để xác nhận đây là `T?` chứ không phải `T~E` (Outcome guard). Thiếu expected type → `Err(LowerError)` (C8). Các vị trí hỗ trợ Bậc A: `let x: T? = ~0`, `return ~0` từ hàm trả `T?`. Vị trí khác → `Err`. |
| Widening `let x: Integer? = e` | identity — copy thường | Không codegen thêm |
| `~+ e` | identity | Như widening; tồn tại vì đối xứng cú pháp |
| `e ?: default` (Elvis) | `brif(icmp eq e, NULL_SENTINEL)` → nhánh default / nhánh e | **Branch, không `select`** — RHS Elvis là Expression đầy đủ kể cả block/return (ADR-0039 điều khoản 2), có side effect, không được eager-evaluate |
| `match m { ~+ x => …, ~0 => … }` | **KHÔNG lower Bậc A** → `Err` | Defer Bậc B (Q4). Dưới PA-3c chỉ là compare+branch — cơ học, thêm sau khi Elvis + get đã cắn |
| `?.`, `?+>`, `.unwrap_value(msg)` | **KHÔNG lower Bậc A** → `Err` | Scope-out §7 |

### 5.4 — `get` builtin

```text
Typecheck prelude:  get(Vector, Integer) -> Integer?     (overload infra 4.3b)
Shim:               __triet_vector_get(vec: i64, idx: i64) -> i64
                    borrow, copy → scalar-or-sentinel
                    idx < 0 || idx >= len  →  NULL_SENTINEL
                    ngược lại              →  data[idx]
BUILTIN_SHIM_META:  arg_consumes = [false, false]        (borrow vec — KHÔNG consume,
                                                          khác push; vec dùng tiếp được)
```

- Total function: index âm / out-of-bounds → `~0`, **không panic** —
  đúng contract "property access 100% safe".
- `fn_2_1` có sẵn (F7) — không cần arity mới.
- `get` trên `Vector<String>` chưa tồn tại (Vector Bậc A monomorphic
  `Vector<Integer>`, 4.3b) → không đụng câu hỏi "trả heap nullable từ shim".

### 5.5 — Drop / borrowck / trap-on-0

**Triết lý:** *"đọc giá trị chết → nổ; thả giá trị chết → êm."*

- **Drop(`"Integer?"`)**: Copy → no-op (nhánh có sẵn).
- **Drop(`"String?"`)**: Move. Lowerer KHÔNG sinh thêm lệnh branch. Nó gọi `__triet_string_free(val)` vô điều kiện. Hàm free shim sẽ check cả 0 (dead value) lẫn MIN (null value) và đều biến thành no-op. Giữ codegen đơn giản và tuân thủ Track B rule 4 (heap-nullable chưa có producer, không viết codegen chết).
- **Shim trap-on-0 (C9):** Mọi shim nhận heap value làm input (read lẫn consume: `__triet_string_len`, `__triet_vector_push`, `__triet_vector_get`, …) **trap** khi nhận giá trị 0: `if val == 0 { SIGABRT }`. 0 = dead value — không bao giờ là heap pointer hợp lệ dưới PA-3c. Ngoại lệ duy nhất là free shim. Đây là defense-in-depth:
  nếu borrowck có soundness bug để lọt read-after-move, chương trình nổ ồn
  ào thay vì âm thầm đọc dữ liệu rác.

  Lưu ý: trap-on-0 áp dụng cho **mọi** heap shim trừ free. Với heap không-nullable (String, Vector), giá trị 0 chỉ xuất hiện khi borrowck lọt bug (ví dụ truyền biến đã move vào `push` hoặc `get`) — trap biến bug đó thành tiếng nổ. Không có overhead cho code đúng.

- **Borrowck không đổi cơ chế:** `is_copy` mở rộng là đủ. `VarState`/E2420/E2450
  áp nguyên xi cho `String?`.

---

## §6 — Phân tích soundness

### 6.1 — Defense-in-depth: trap-on-0 (tại sao chọn PA-3c thay vì PA-3a)

**Cơ chế borrowck hiện hành — chính xác hóa:**

`places_conflict(a, b, conservative=true)` ở `triet-borrowck/src/checker.rs:64-66`:
khi `conservative=true` và `a.local != b.local`, hàm trả về `true` (conflict) —
tức là **over-reject** (E2440 oan cho chương trình đúng), **không phải leak**
(soundness hole). Món nợ TODO ghi trong code là **precision**, không phải
soundness. Borrowck hiện hành có teeth: E2420 verified 8/8 fixtures, E2450
Drop-while-borrowed hoạt động.

**Tuy nhiên,** E2420/VarState là cơ chế còn trẻ (mới 2 phase). PA-3a chọn
heap null = 0, nghĩa là: nếu một ngày borrowck có soundness bug để lọt
read-after-move (không phải over-reject — mà under-reject: không báo lỗi
cho chương trình sai), chương trình sẽ thấy null (0 ≡ MIN của PA-3a heap)
êm ái thay vì nổ. **PA-3c trả một khoản phí bảo hiểm rẻ để mua tiếng nổ
cho kịch bản đó:**

1. **Uniform MIN:** null = `i64::MIN` cho mọi loại → 0 không bao giờ là null.
   0 = dead value (moved-out hoặc OOM).
2. **Read-shim trap-on-0:** mọi read-shim gặp 0 → SIGABRT. Borrowck bug
   → nổ ồn ào, không âm thầm.
3. **`free(0)` vẫn no-op:** Drop của giá trị đã-move phải êm (C4). Triết lý
   nhất quán: đọc dead → nổ; thả dead → êm.
4. **Không đánh thuế null-check:** null-check trong Elvis/`~0`/match chỉ so
   MỘT magic (`i64::MIN`). Chỉ free shim guard hai (0 và MIN), và guard 0 đã
   có sẵn. Khoản "2 magic" mà draft v1 lo ngại thực tế không đáng kể.

**Kết luận:** PA-3c không mua "sự hoàn hảo của borrowck" — nó mua bảo hiểm rẻ cho cơ chế E2420/VarState còn trẻ. Phí bảo hiểm: trap-on-0 trong các hàm shim (0 dòng code mới cho chương trình đúng, 1 `if val == 0 { abort() }` đầu mỗi shim nhận heap ptr trừ free). Quyền lợi: mọi soundness bug của borrowck biến thành SIGABRT thay
vì silent wrong.

**Re-evaluate:** heap-nullable chưa có producer Bậc A (§7: wiring defer đến
khi có producer). Khi producer đầu tiên đến, cửa re-evaluate PA-3a vẫn còn
nguyên — nhưng với trap-on-0 đã có sẵn, động lực quay về PA-3a gần như bằng 0.

### 6.2 — Scalar: debt D1 (phantom null qua arithmetic không-wrap)

**Phát biểu chính xác:** dưới PA-3c, một chuỗi phép toán raw-i64 (F4) về
lý thuyết có thể sinh ra bit-pattern `i64::MIN` và được hiểu nhầm là `~0`
khi đổ vào một chỗ chứa `Integer?`.

**Vì sao bounded:**

1. Để chạm `i64::MIN` bằng `iadd/imul` từ các literal hợp lệ, giá trị trung
   gian phải rời range hợp lệ `±3.8×10¹²` — một phép `imul` của hai Integer
   hợp lệ (3.8e12 × 3.8e12 ≈ 1.45×10²⁵) đã tràn i64 trong một bước. Tức
   chương trình **đã** vi phạm semantics mod-3²⁷ của SPEC từ trước, kết quả
   đã sai theo định nghĩa ngôn ngữ. D1 chỉ đổi *dạng* của cái sai.
2. Giá trị đúng-range không bao giờ va sentinel: khoảng cách từ range hợp
   lệ đến `i64::MIN` là ~2^62.
3. **Lối thoát có sẵn trong roadmap:** khi arithmetic Bậc B wrap đúng
   mod-3²⁷ (món nợ độc lập, phải trả dù ADR này chọn gì), mọi giá trị
   runtime nằm trong range → niche được enforce → D1 đóng vĩnh viễn,
   không cần đổi repr.

**Nghĩa vụ đi kèm khi duyệt PA-3c:** (a) ghi D1 vào danh mục debt
(`spec/plans/REPORT` kế tiếp); (b) canary test N1 ràng `NULL_SENTINEL` ngoài
range triet-core (§9) — nếu ai đổi width Integer, test đỏ; (c) comment tại
`NULL_SENTINEL` trỏ về §6.2 này.

### 6.3 — Những thứ ADR này KHÔNG làm suy yếu

- Guard JIT từ chối `OutcomeDiscriminant/Unwrap/UnwrapError` (F10) **giữ
  nguyên** — `T?` Bậc A không đi qua các statement đó (compare trực tiếp
  trong codegen của Elvis), nên không có cớ gỡ guard.
- `T??` vẫn không tồn tại (C6) — typecheck đã chặn, repr không cần nghĩ
  về nested.

---

## §7 — Scope Bậc A (IN / OUT)

| IN | OUT (defer) | Defer đến |
|----|-------------|-----------|
| `~0` literal (type-agnostic, cần expected type ở các vị trí được hỗ trợ) | `?.` safe call, `?+>` map/flatMap (ADR-0039) | Bậc B |
| Widening `T ⊂ T?`, `~+ e` | `.unwrap_value(message)` (cần trap + message ABI) | Bậc B |
| `get(vector, index) -> Integer?` | `match` 2-arm `~+/~0` (Q4) | Bậc B |
| `e ?: default` (Elvis, branch-based) | `T~E` / `T?~E` Outcome (packed ABI) | Bậc C |
| `String?`/`Vector?` **repr định nghĩa** (null=MIN) | `Long?` (F3) | Bậc B+ |
| | … **wiring heap-nullable defer**: chưa có producer nào trả `String?` ở Bậc A (get là `Integer?`) — định nghĩa repr để ADR trọn vẹn, không ship code chết (Track B rule 4) | khi có producer |

---

## §8 — Đường migration

| Mốc | Việc | Repr đổi? |
|-----|------|-----------|
| Bậc B: arithmetic wrap mod-3²⁷ | D1 đóng tự nhiên | Không |
| Bậc B: refcount thật (`&+` lower) | Heap nullable: null=MIN → không phải 0, refcount code không bị nhầm | Không |
| Bậc B: match `~+/~0` | Thêm compare+branch — cơ học, không đổi repr | Không |
| Bậc C: packed Outcome ABI (`T~E` 2-word) | `T?` CÓ THỂ migrate sang packed cho đồng nhất với `T?~E`, hoặc giữ niche làm fast-path (Rust giữ cả hai) — quyết định lúc đó, ADR mới | Có thể, cục bộ |

PA-3c chạm ít file nhất hôm nay và không chôn quyết định nào cản Bậc C.

---

## §9 — Verification plan

### Fixtures (tiếp số 40+)

| # | Fixture | Kỳ vọng |
|---|---------|---------|
| 40 | `get` in-bounds: push 3 phần tử, `get(v,1) ?: -1` | EXPECT: phần tử |
| 41 | `get` out-of-bounds: `get(v,99) ?: -1` | EXPECT: -1 |
| 42 | `get` index âm: `get(v,-1) ?: 7` | EXPECT: 7 |
| 43 | Elvis với widening: `let x: Integer? = 5; x ?: 0` | EXPECT: 5 (Elvis trên non-null) |
| 44 | widening + `~0` qua return: fn trả `Integer?`, caller Elvis | EXPECT |
| 45 | `get` không consume vec: get rồi `len(v)` vẫn dùng được | EXPECT: len |
| 46 | E2420: dùng `get` làm use-site sau move của vector (push xong gọi get) | EXPECT: E2420 (đối ngẫu tĩnh của fixture 38) |

### Teeth (đỏ-khi-sai, không chỉ xanh-khi-đúng)

| Gỡ gì | Test nào phải ĐỎ |
|-------|------------------|
| Bounds-check trong `__triet_vector_get` | 41/42 (đọc rác hoặc crash) |
| So sánh sentinel trong Elvis codegen | 41 (trả sentinel thô thay vì -1) |
| Nhánh `nullable_payload` trong `is_copy` | unit: `"String?"` phải Move — default-Move che mất nhánh này nếu chỉ test String?, nên test cả `"Integer?"` phải **Copy** |
| Trap-on-0 trong shim | unit: gọi `__triet_string_len(0)` hoặc `__triet_vector_push(0, x)` → SIGABRT (không phải return 0) |

### Unit / invariant

| # | Invariant | Cách verify |
|---|-----------|-------------|
| N1 | Canary: `NULL_SENTINEL < Integer::MIN.0` && ngoài range Tryte/Trit/Trilean — ràng vào hằng triet-core, không hardcode lại số | unit test ở triet-mir (dep triet-core đã có) |
| N2 | `is_nullable_type`/`nullable_payload` round-trip; `is_nullable_type("Vector<Integer>?")` = true và `nullable_payload("Vector<Integer>?")` = `Some("Vector<Integer>")` — `is_vec_type` không được nuốt; `is_nullable_type("?")` = false (pin "?" trần); `"Integer??"` → không bao giờ xuất hiện (typecheck chặn) nhưng helper trả về gì cũng phải định nghĩa | unit |
| N3 | `~0` không expected type → `Err(LowerError)` có span, không panic | lowerer unit |
| N4 | `get` arg_consumes [false,false]: borrowck KHÔNG mark Moved vec | borrowck unit (đối chứng với push) |
| N5 | `~0` lowering type-agnostic: cùng `iconst(i64::MIN)` cho `Integer?` và `String?` | lowerer unit |
| N6 | Typecheck: dùng `Integer?` ở chỗ đòi `Integer` (chiều ngược widening) phải bị reject — verify chiều của `Type::matches` trước khi tin | typecheck unit (nếu đã có thì trỏ tới, không viết trùng) |
| N7 | Shim trap-on-0: `__triet_string_len(0)`, `__triet_vector_push(0, _)` → SIGABRT; `__triet_string_free(0)` → no-op (không trap) | unit (có thể cần test riêng trong harness vì SIGABRT không bắt được bằng `#[should_panic]` thông thường) |

---

## §10 — ADR / tài liệu liên quan

| Tài liệu | Quan hệ |
|----------|---------|
| ADR-0001 | Semantic model disc+payload; cho phép niche-as-optimization; **cần addendum sửa bảng gán trit + điều khoản `T??`** (§0.1, Q5) |
| ADR-0020 §10 | Cực trit `+/0/−`, `~0` canonical, E1032 match arms — nền semantics |
| ADR-0037 | Máy enum StackSlot — dùng nếu PA-2/PA-3b thắng review (không được chọn) |
| ADR-0039 | `?`-family chờ repr này; Elvis RHS = Expression (ép branch-based lowering §5.3); `T??` auto-flatten (C6) |
| ADR-0040 | §2.5 ABI 1×i64; §1.3 M1-M4; §6 flag xung đột sentinel-0 — ADR này trả lời (chọn uniform MIN, xung đột được giải) |
| `feedback_explicit_strictness` | `get` total trả `T?`, không panic |
| `spec/plans/REPORT-2026-06-04.md` | Nơi ghi debt D1 |

---

## §11 — Tóm tắt cho người review 5 phút

1. **PA-3c (uniform sentinel):** `NULL_SENTINEL = i64::MIN` cho mọi `T?` —
   scalar lẫn heap. Uniform → `~0` lowering type-agnostic, đơn giản hơn
   PA-3a (vốn cần hai sentinel khác nhau).
2. **D1 (phantom null):** bounded bởi arithmetic-fidelity debt CÓ SẴN (F4),
   chết tự nhiên khi Bậc B wrap mod-3²⁷. Được cả hai mentor chấp nhận.
3. **Trap-on-0 (defense-in-depth):** dưới uniform MIN, 0 = dead value —
   không bao giờ là null. Mọi read-shim trap khi gặp 0 → borrowck bug biến
   thành SIGABRT. `free(0)` giữ no-op → Drop của moved-out vẫn êm.
4. **Scope Bậc A:** widening + `~0` + Elvis `?:` + `get`. Match `~+/~0` →
   Bậc B (kỷ luật scope, G chặn đúng).
5. **R1 (va chạm type-string):** `is_nullable_type` phải hỏi TRƯỚC mọi phân
   loại khác; `"?"` trần pin là không-nullable — ghi trong §5.1.
6. **Addendum ADR-0001 (R2):** sửa cả bảng gán trit lẫn điều khoản `T??`
   trong một lần — §0.1.
7. **Thứ tự implement:** canary N1 + helpers → widening + `~0` + Elvis →
   `get` + fixtures 40-46. Đăng ký shim cả driver lẫn harness (bài học 4.3b).
