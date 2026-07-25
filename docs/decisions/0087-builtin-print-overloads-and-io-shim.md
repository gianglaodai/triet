# ADR 0087 — Builtin Print — Overloads & I/O Shim

**Trạng thái:** Chấp thuận (Mentor G ký 2026-07-25). Áp dụng cho Bậc C+.

**Issue:** `print`/`println` là stdout-write ĐẦU TIÊN được thiết kế cho backend
rewrite. Typecheck đã khai báo cả hai (`crates/triet-typecheck/src/env.rs:144`
`print`, `:152` `println`, cả hai `String → Unit`), nhưng lowerer
(`crates/triet-lower/src/lib.rs`) KHÔNG có arm builtin cho chúng: `match
callee_name.as_str()` tại `:2661` liệt kê `concat`/`len`/`vector_new`/`push`/…
nhưng không có `"print"`/`"println"`, nên lời gọi rớt xuống default arm
`_ => { /* fall through to user-defined function dispatch */ }` tại `:3241` —
lowerer coi `print`/`println` là hàm user-defined, không tìm thấy định nghĩa,
và JIT thất bại với `callee 'println' not found`, exit 4. Đây KHÔNG phải
silent-miscompile (không có invariant nào bị vi phạm âm thầm) — thuần túy là
feature-gap: typecheck hứa nhưng backend chưa lower.

## Quyết định

### 1. Bốn signature overload

`print(String)`, `print(&0 String)`, `println(String)`, `println(&0 String)`.

- **Owned `String`** (by-value) = MOVE = tiêu thụ giá trị — caller không thể
  dùng lại biến sau lời gọi (mirror ownership rule của mọi hàm nhận `String`
  by-value hiện có, ADR-0042).
- **`&0 String`** (borrow, read-only reference) = Reference là Copy (S6, SPEC
  §10) → tái sử dụng được sau khi in.

Cả hai hình thức cần vì: in một literal/biểu thức tạm (move tự nhiên, không
cần giữ lại) và in một biến sẽ còn dùng tiếp (cần borrow) là hai tình huống
phổ biến ngang nhau trong code thực tế; chỉ hỗ trợ một trong hai buộc lập
trình viên phải `concat`/clone giả tạo chỉ để in ra màn hình.

### 2. Bốn extern-C shim tách biệt theo symbol name (không truyền cờ `is_owned`)

| Signature | Shim | arity | `arg_consumes` | Hành vi |
|---|---|---|---|---|
| `print(String)` | `__triet_print` | 3 (ptr, len, cap) | `[true]` | write ptr..len ra stdout → `free(ptr, cap)` |
| `print(&0 String)` | `__triet_print_ref` | 2 (ptr, len) | `[false]` | write only, không free |
| `println(String)` | `__triet_println` | 3 | `[true]` | write + `\n` → `free` |
| `println(&0 String)` | `__triet_println_ref` | 2 | `[false]` | write + `\n`, không free |

Memory-responsibility được hardcode vào TÊN symbol (4 symbol riêng), không
truyền một cờ runtime `is_owned` để một shim chung tự rẽ nhánh free/không-free.

- **Owned = `arg_consumes: [true]`:** move-in nghĩa là callee (shim) sở hữu
  giá trị ⇒ shim tự `free`. Caller-side slot bị zero hóa bởi M3 (move-tracking
  đã có sẵn cho mọi lời gọi consume) ⇒ Deinit của caller thấy slot rỗng ⇒
  `free(0)` là no-op ⇒ đúng một lần free trên toàn vòng đời. Đây là mẫu đã có
  tiền lệ với `__triet_vector_push` (khi push một `String` owned vào
  `Vector<String>`).
- **Ref = `arg_consumes: [false]`:** owner giữ quyền sở hữu, `free` xảy ra ở
  scope của owner như bình thường (Deinit tombstone, ADR-0042), không phải ở
  shim.

### 3. Trả về `Unit` đàng hoàng — không throwaway i64

Thêm một nhánh xử lý return-Unit vào `emit_shim_call`
(`crates/triet-lower/src/lib.rs:1669`): khi shim không có giá trị trả về ý
nghĩa, KHÔNG alloc một `dest` local rồi bind nó như thể nhận một i64 rác —
không alloc dest, không bind return value. `ShimSymbol` phía JIT
(`crates/triet-jit/src/mir_lower.rs:104`) đã có sẵn mẫu void
(`has_return: false`, dùng bởi `fn_1_0`/`fn_2_0`/`fn_5_0` hiện hành) — 4 shim
print/println mới dùng đúng mẫu này (`fn_3_0`/`fn_2_0` tùy arity), KHÔNG cần
thêm biến thể `ShimSymbol` mới về registration, nhưng **`emit_shim_call` phía
lowerer thì cần sửa** vì hàm này hiện tại LUÔN alloc `dest` + luôn gán
`return_shape: ReturnShape::Scalar` bất kể shim có trả giá trị hay không
(xác nhận tại `:1685`/`:1698` — không có nhánh rẽ theo `has_return`/void hiện
tại).

Đây là quyết định có chủ đích để mọi builtin trả `Unit` trong tương lai
(không riêng print) đi qua một đường lower sạch, thay vì mỗi call site tự
chế một dest rác rồi không dùng.

### 4. Capability = compile-time only, không gate runtime

Dựa vào cơ chế capability hiện có
(`crates/triet-typecheck/src/capability_check.rs`, E2200
`MissingCapabilityClaim` / E2201): `std` là ambient (bỏ qua namespace check),
`sys.io` cần khai grant. KHÔNG thêm một `__triet_cap_check` runtime call nào
trước khi gọi shim. Điều này bám theo VISION (capability là *constraint* thiết
kế / kiểm tra tĩnh, KHÔNG có lời hứa runtime enforcement ở tầng này).

## Các phương án đã cân nhắc

| # | Phương án | Ưu | Nhược | Kết luận |
|---|-----------|---|-------|----------|
| 1 | Consume-only: chỉ hỗ trợ `print(String)` owned, bắt buộc caller move/clone mọi lần in | Ít shim nhất (2 thay vì 4), lowerer đơn giản hơn | Ergonomic rác: in một biến còn dùng tiếp bắt buộc `concat`/clone giả tạo chỉ để thỏa mãn move; không mirror cách `&0` đã dùng cho các builtin đọc khác (`len(&0 String)`) | Loại — G bác. |
| 2 | Một shim `has_return: true` trả về i64 cố định (ví dụ 0) làm giá trị Unit throwaway | Tái dùng `emit_shim_call` không sửa gì, ít việc nhất | Nợ kỹ thuật: mọi `Unit`-return builtin tương lai lặp lại pattern rác "trả 0 rồi không ai đọc"; `dest` local được alloc + StorageLive cho một giá trị không tồn tại về mặt ngữ nghĩa | Loại — G bác, chọn sửa `emit_shim_call` một lần cho đường sạch. |
| 3 | Hai shim (`print`/`println`) + một cờ runtime `is_owned` truyền vào C-shim để tự rẽ nhánh free/không-free bên trong | Ít symbol hơn (2 thay vì 4) | Thêm branching runtime bên trong C-shim dựa trên dữ liệu do lowerer truyền — bẩn hơn so với encode memory-responsibility ngay ở tên symbol (compile-time, không có nhánh runtime nào có thể bị truyền sai giá trị) | Loại — G bác, chọn 4-symbol tách bạch. |
| 4 | Bốn signature overload, 4 shim riêng theo symbol name, sửa `emit_shim_call` thêm nhánh Unit (đã chọn) | Ergonomic đầy đủ (move + borrow), memory-responsibility rõ ràng tại compile-time (không cờ runtime), mở đường sạch cho mọi builtin `Unit` tương lai | Bốn shim mới cộng vào bảng `builtin_shim_meta` — mở rộng bề mặt SPOF đã ghi nhận ở ADR-0085 | **Chọn.** |

## Hậu quả

### Tích cực
- stdout hoạt động lần đầu trên backend rewrite (Bậc C) — chương trình `.tri`
  đầu tiên có thể tự in kết quả ra màn hình thay vì chỉ được biết qua exit
  code hoặc test harness.
- `emit_shim_call` có đường Unit sạch, dùng lại được cho mọi builtin `Unit`
  tương lai (ví dụ side-effect-only ops) mà không cần vá riêng từng call site.
- Cả hai chế độ move và borrow đều được hỗ trợ, nhất quán với cách các builtin
  đọc khác (`len`, `eq`, `concat`) đã hỗ trợ `&0 String`.

### Tiêu cực
- Bảng `builtin_shim_meta` (SPOF đã ghi nhận ở ADR-0085) thêm 4 entry mới —
  tăng bề mặt "khai láo vs hành vi C-shim thật" mà ADR-0085 đang phòng thủ.
- 4 symbol cho 2 hàm nguồn (`print`/`println`) — nhiều hơn số lượng shim tối
  thiểu về mặt lý thuyết (2), đổi lấy việc không có nhánh runtime rẽ theo cờ.

### Rủi ro cần mitigate
- Bốn entry `arg_consumes` mới cho `print`/`print_ref`/`println`/`println_ref`
  phải được canh bằng teeth FREE-count 2 chiều theo đúng discipline đã thiết
  lập ở ADR-0085 (Threat-1: bảng khai láo vs hành vi free thật của C-shim) —
  KHÔNG merge nếu thiếu canary cho 4 entry này.
- Nhánh `Unit`/void mới trong `emit_shim_call` phải được kiểm chứng không làm
  vỡ các call site hiện có (`concat`/`len`/`push`/…) vẫn đi qua nhánh
  `Scalar` cũ — cần test phân biệt rõ hai nhánh, không chỉ test riêng
  print/println.

## Ngoài phạm vi (defer)

- `read_line` (input, chưa nằm trong scope WO này dù đã khai báo ở
  `env.rs`).
- f-string / format runtime (interpolation) — chỉ scope `String`/`&0 String`
  literal/biến, không format string.
- Buffering policy (line-buffered vs unbuffered stdout) — không quyết định ở
  ADR này, giữ hành vi mặc định của runtime write hiện có.

## Ngày hiệu lực

- Bậc C+ — áp dụng khi WO hiện thực hóa ADR này merge.
- Không áp dụng hồi tố — không có code print/println nào tồn tại trước ADR
  này (feature-gap thuần túy, không phải thay đổi hành vi cũ).
