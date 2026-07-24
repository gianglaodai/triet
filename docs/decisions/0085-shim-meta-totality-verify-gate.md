# ADR 0085 — Bảng `builtin_shim_meta` toàn phần + cổng tồn tại ở `Body::verify()`

**Trạng thái:** Quyết định (G ✅ 2026-07-24 · O ✅ 2026-07-24). Áp dụng cho Bậc C+.
Đóng lỗ SPOF của bảng metadata shim: một `CallDispatch` gọi shim hệ thống
(`__triet_*`) mà bảng không có entry sẽ bị từ chối ở cổng well-formedness MIR,
thay vì bị nuốt câm rồi miscompile.

**Issue:** `triet_mir::builtin_shim_meta(name) -> Option<BuiltinShimMeta>` là
**một bảng tĩnh, đọc bởi NĂM site** ở ba crate (borrowck ×3, JIT ×1, lowerer ×1
— xem §Bảng caller). Cả năm site dùng `if let Some(meta)` / `is_some_and`, nên
một **entry thiếu bị bỏ qua câm**:

- JIT M3 (`mir_lower.rs:4784`) không zero-on-consume → con trỏ heap cũ còn sống.
- Lowerer (`lib.rs:1517`) coi mọi arg là borrow → `push_owned` lịch Drop cho arg
  mà shim đã tiêu thụ → **double-free** khi shim heap-consuming thiếu entry.
- Borrowck (`checker.rs:1288/1319`) bỏ mark-Moved và bỏ check mutate-while-borrowed
  → **bỏ E2420/E2440 câm**.

Đây KHÔNG phải defense-in-depth (năm khóa độc lập) mà là **một điểm hỏng duy nhất
tỏa ra năm nơi**: bảng nói dối bằng cách vắng mặt thì cả năm site sai cùng chiều.
Hôm nay latent — **tám** shim đang thiếu entry (`__triet_string_contains`, `_hash`,
`__triet_vector_contains`, `__triet_hashmap_contains`, `__triet_cap_check`,
`__triet_pow`, `__triet_string_append`, `_clear`) đều là **borrow/scalar**, nên
default all-borrow tình cờ đúng (đã verify body: `__triet_string_append(slot,
byte-scalar)` — tham số thứ hai là byte i64 Copy, không phải con trỏ heap). Một
shim **heap-consuming** tương lai quên entry sẽ nổ câm.

> **AMEND 2026-07-24 (7→8):** Bảng gốc liệt kê 7 (recon O `comm` **chỉ** JIT-dispatch-
> names vs meta). D bắt lỗi khi tự đối chiếu lại (đúng luật "bảng kiến trúc sư
> cũng là giả định"): `__triet_vector_contains` emit từ **lowerer** (`lib.rs:2607`,
> nhánh `ty.is_vec()`), không nằm trong grep JIT của O → sót. Đang chạy sống ở
> fixture `86_contains_vector_run.tri` (`contains(v4, 42)`). Không thêm entry #8 thì
> cổng verify() sẽ giết fixture 86 (452→451). Đây đúng bài học luật 19 O tự viết vào
> WO ("grep TOÀN BỘ họ — lower ∪ jit — trước khi khoanh bán kính") mà O vẫn vi phạm
> lúc recon. Đường đo đúng: `comm -23 <(grep lower + jit) <(grep meta)` = 8.

Đây là **Nhịp 1** của kế hoạch hai-nhịp (G chốt 2026-07-24, Option 4 chia-để-trị).
Nhịp 1 đóng **P-exist** (entry thiếu). **P-flag** (bool sai trong entry có sẵn) là
**Nhịp 2** — behavioral canary — ngoài phạm vi ADR này.

## Quyết định

### 1. Bảng toàn phần
Thêm entry tường minh cho **tám** shim đang thiếu. Bảng phải phủ **mọi** shim
`__triet_*` mà compiler có thể emit:

| shim | `arg_consumes` | `mutates_arg` | ghi chú |
|---|---|---|---|
| `__triet_string_contains` | all-false | `None` | borrow thuần |
| `__triet_string_hash` | all-false | `None` | borrow (content hash) |
| `__triet_vector_contains` | `[false, false]` | `None` | borrow thuần (arity 2 = `fn_2_1`; AMEND — D bắt) |
| `__triet_hashmap_contains` | all-false | `None` | borrow thuần |
| `__triet_cap_check` | all-false | `None` | ZST capability token |
| `__triet_pow` | all-false | `None` | scalar Copy, không heap |
| `__triet_string_append` | `[false, false]` | `None` (Nhịp 1) | mutate tại chỗ NHƯNG xem AMEND-2 |
| `__triet_string_clear` | `[false]` | `None` (Nhịp 1) | mutate tại chỗ NHƯNG xem AMEND-2 |

> **AMEND-2 2026-07-24 (`Some(0)`→`None` cho append/clear):** Bản gốc cắm
> `mutates_arg: Some(0)` cho append/clear để "vá luôn E2440 mutate-while-borrowed"
> (scope-creep của O). Khi D wire vào thật → **5 fixture sống nổ E2440 self-conflict**
> (`93_clear_run`, `96/97_append_*`, `99_append_then_clear`, `100_endgame`). Gốc (O
> verify độc lập, toggle `Some(0)↔None`): append/clear dùng calling-convention
> `clear(&0 mutable m)` — lowerer trả **thẳng Local của `m`** cho arg[0], và evaluate
> `&0 mutable m` **tạo một active loan `source=m`**; M3 precheck (`checker.rs:1288`)
> thấy loan đó xung đột với chính arg đang mutate → E2440 với **loan của chính nó**.
> `pop`/`remove` không dính vì truyền container Local trần, không qua `&0`. `mutates_arg:
> Some(0)` **đúng ngữ nghĩa** (append realloc khi có `&0` share = hazard thật) nhưng
> **cần checker phân biệt self-loan khỏi loan khác** — đó là behavior change, thuộc
> **Nhịp 2**. Nhịp 1 đặt `None` = **đúng behavior pre-WO** (append/clear chưa từng có
> entry = None) → không regression, không lỗ mới. `arg_consumes` giữ nguyên (đúng mục
> tiêu P-exist chống double-free). E2440-cho-string-mutate + checker self-loan-exclusion
> defer Nhịp 2. **Luật 18: O phản xạ đắp cơ chế không hợp calling-convention; D wire+đo
> mới lộ.**

Arity (`arg_consumes.len()`) lấy từ chữ ký `ShimSymbol::fn_N_M` đăng ký ở
`driver/main.rs`, KHÔNG đoán bằng mắt. `append`/`clear` dùng `mutates_arg: None`
ở Nhịp 1 (xem AMEND-2 — `Some(0)` tự-đụng loan; E2440-cho-string-mutate defer Nhịp 2).

### 2. Cổng tồn tại ở `Body::verify()` (discriminator `__triet_`)
`Body::verify()` (`triet-mir/src/lib.rs:1855`) — cổng well-formedness MIR đã chạy
ở **Phase 3.5 của driver, TRƯỚC borrowck (P4) và JIT (P5)** (`driver/main.rs:82-90`,
comment: *"Run BEFORE borrowck and JIT so they can assume well-formed MIR"*) — thêm
một bất biến mới:

> Với mỗi terminator `CallDispatch { callee_name, .. }`: nếu
> `callee_name.starts_with("__triet_")` mà `builtin_shim_meta(callee_name)` trả
> `None` → `Err(MirError::UnknownShim { name })`.

Discriminator `__triet_` là **ranh giới cấu trúc, không phải danh sách chép tay**:
mọi shim hệ thống mang tiền tố `__triet_`; user-fn (`concrete_fn`, `"fibonacci"`,
`"f"`) và synthetic borrowck (`"consume"`, `"__test_shim_multiply"`) KHÔNG bao giờ
mang tiền tố đó — nên `None` cho chúng vẫn hợp lệ. Cổng tự-thực-thi trên tiền tố:
bất kỳ `__triet_*` tương lai thiếu entry đều tự vấp, không cần ai nhớ cập nhật một
danh sách thứ hai.

Năm read-site (R1–R5) **giữ nguyên** `if let Some` — giờ provably-`Some` cho tên
`__triet_` sau khi `verify()` đã gác; nhánh `None` của chúng thành phòng thủ thừa
vô hại. Bán kính đổi code = **một crate** (`triet-mir`: entry bảng + biến thể lỗi
+ vòng verify), không phải năm site ở ba crate.

## Các phương án đã cân nhắc

| # | Phương án | Ưu | Nhược | Kết luận |
|---|-----------|---|-------|----------|
| α | None→Err tại **từng** read-site (R1–R5) | phòng thủ mỗi tầng | **năm bản sao** cùng một predicate `__triet_ && None` ở ba crate (ba error-type) → site thứ 6 quên là SPOF tái sinh; đúng cái duplication đang diệt | **Bác** (G 2026-07-24) |
| β | **Cổng đơn ở `Body::verify()`** + giữ R1–R5 | một nguồn sự thật (DRY); chạy P3.5 trước cả borrowck/JIT; teeth thỏa "compile PHẢI nổ"; bán kính 1 crate | không gác unit-test dựng `Body` trực tiếp lách verify | **CHỌN** — hợp đồng compiler bảo vệ user-input (luôn qua driver P3.5); test nhồi MIR rác tự chịu |
| γ | Registry enum: JIT-dispatch + meta + emit cùng key một enum, match non-exhaustive = compile-error | giết SPOF by-construction, không cần cổng runtime | đại phẫu core (25 shim × 3 tầng), blast radius quá lớn cho một bảng tĩnh | **Bác** (G 2026-07-24 — "ngu ngốc về chiến thuật hiện tại") |
| δ | Existence canary: test liệt kê mọi shim, assert có entry | rẻ | **danh sách là bản sao thứ tư của bảng** — người quên entry cũng quên danh sách test; bắt được con số 0 (oracle-vòng, luật 14) | **Bác** — "đồ giả" |

## Hậu quả

### Tích cực
- Entry thiếu cho một shim `__triet_*` không còn nói dối câm: nổ ở P3.5 với
  `MirError::UnknownShim`, trước khi chạm borrowck/JIT.
- `append`/`clear` mang `mutates_arg: None` (Nhịp 1) — E2440-cho-string-mutate
  defer Nhịp 2 cùng checker self-loan-exclusion (AMEND-2).
- Predicate ở một chỗ duy nhất — không thể lệch giữa các bản sao.

### Tiêu cực
- `verify()` không gác các đường test dựng `Body` trực tiếp không gọi `verify()`.
  Chấp nhận: hợp đồng compiler là bảo vệ user-input qua driver pipeline.

### Rủi ro cần mitigate
- **Teeth (luật 1+2):** xóa thử entry `__triet_vector_push` khỏi bảng → compile
  một fixture dùng `push` → `verify()` PHẢI trả `MirError::UnknownShim`, compiler
  nổ ở P3.5, KHÔNG sinh mã câm → khôi phục (cp-snapshot md5, không `git checkout`).
  Teeth này canh **sự tồn tại** — đúng nguyên tắc "canh entry trước, cờ sau".

## Ngày hiệu lực

- Bậc C+ — cổng verify + bảng toàn phần kích hoạt ngay khi Nhịp 1 land.
- **Nhịp 2 (ngoài ADR này):** behavioral roundtrip canary per consuming shim đóng
  P-flag (bool sai). Oracle FREE-count phải dedup con trỏ (luật 14).
- Liên quan: ADR-0040 §3.1/§3.6 (shim registry + M3 consume-tracking),
  ADR-0079 (`returns_borrow_of`/`mutates_arg`), ADR-0082 §AMEND (M3 zeroing).
