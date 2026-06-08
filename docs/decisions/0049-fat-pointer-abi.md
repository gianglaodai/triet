# ADR-0049: Fat-Pointer ABI for String (Slice provenance deferred)

## 1. Status
**Approved (O + G, 2026-06-08)** — Phase-0 ĐÓNG. Vào Phase-1 Implementation.

**Ràng buộc G (bất biến — không được vi phạm trong implementation):**
1. **`free` shim ABI = 2-arg-rời `free(ptr, cap)`.** Cấm truyền `*const FatStr` cho consuming shim. Bung field qua `stack_load` rồi gọi — tường minh, nhanh, không deref thừa.
2. **`return-fat` (e.g. `concat`) = `sret` StackSlot + `ReturnShape::Struct`.** Tái dùng mẫu Gate A. Caller cấp StackSlot, callee điền qua implicit first argument.
3. **ObjectHeader 8B GIỮ NGUYÊN.** Layout heap: `[Header 8B][data...]`. `ptr` trỏ data; dealloc = `free(ptr - 8, layout(cap))`. Dọn sẵn cho RefCount (ADR-0022) — không đập offset lần hai.

## 2. Context & Motivation
Mô hình bộ nhớ hiện tại của Triết (Bậc C) quản lý chuỗi (String) thông qua một handle (I64) trỏ vào một khối heap liền mạch `[ObjectHeader 8B] [len i64] [cap i64] [data...]`.
Mọi thao tác thay đổi chuỗi (như `append`, `concat`) đều tuân theo mô hình functional: cấp phát khối mới, copy data, giải phóng khối cũ và trả về handle mới. Mô hình này **Sound** (không có dangling pointers) nhưng gặp giới hạn lớn về hiệu năng (O(n) cho mỗi thao tác push) và không cho phép chia sẻ bộ nhớ (sub-slicing).

**Động lực chính (Motivation):**
Chúng ta KHÔNG vá lỗi UB. Chúng ta đang **mở khóa tính năng**:
1. Cho phép `append` đạt amortized O(1) thông qua cap-doubling và realloc (có thể relocate buffer -> ptr đổi). Caller nhận ptr mới qua writeback. Việc `ptr` có thể đổi chính là (a) raison-d'être của fat-pointer writeback, (b) lý do mọi shared borrow phải bị cấm trong lúc append.
2. Cho phép các tham chiếu biến đổi (mutable references) thấy được sự thay đổi của String thông qua pointer.
3. Hỗ trợ String View / Slice (`&[T]`) chia sẻ chung buffer với String gốc mà không cần copy.

## 3. Quyết định Kiến trúc (Architectural Decisions)

### 3.1. Layout của Fat-Pointer trên StackSlot
- **Owned String**: Sử dụng mô hình **3-field** `[ptr, len, cap]`. Điều này cho phép thao tác `append` đọc `cap` trực tiếp trên stack mà không cần dereference heap, tối ưu số vòng cycle.
- **Borrowed Slice (`&str` / `&[T]`)**: Sử dụng mô hình **2-field** `[ptr, len]`. Slice không quản lý capacity và không chịu trách nhiệm free.

### 3.2. Layout trên Heap
- Heap block sẽ chỉ còn `[ObjectHeader 8B] [data...]`.
- Các trường `len` và `cap` được chuyển hoàn toàn lên StackSlot.
- `ptr` của fat-pointer sẽ trỏ thẳng vào vùng `data`.
- Thao tác dealloc/free sẽ tính toán địa chỉ base bằng cách lấy `ptr - 8` và kích thước giải phóng từ `layout(cap)` (với `cap` bắt buộc phải được truyền từ StackSlot vì không còn nằm trên heap).

### 3.3. Phạm vi triển khai (Scope)
- **String-only**: Bậc D sẽ chỉ tập trung áp dụng mô hình này cho `String`. `Vector` và `HashMap` sẽ được giữ nguyên (hoặc bị vô hiệu hóa tạm thời) để giới hạn blast radius cho đến khi ABI của String được chứng minh là ổn định.

## 4. Spike & Probes (Kiểm định)
Trước khi bước vào code thực tế, 4 câu hỏi ABI đã được mổ xẻ và kết luận như sau:

1. **Q2 - Shim ABI:** *PROVEN (một phần).*
   - Lớp shim `mutate-writeback` (như `append`, `clear`): Đã dựng spike và chứng minh thành công SystemV C-ABI truyền `*mut FatStr` in-place, caller đọc lại (reload stack_load) thấy đúng ptr+len mới.
   - *TBD:* Còn 4 lớp shim chưa đo: (a) `read-scalar` (len/is_empty) - có thể bỏ hoàn toàn shim vì biến đã có sẵn trên stack; (b) `read-buffer` (eq/contains) - **chốt: bung tham số rời** (ptr,len mỗi chuỗi; eq(a_ptr,a_len,b_ptr,b_len) = 4 args, SysV 6 thanh ghi vẫn đủ), cấm `*const FatStr`; (c) `return-fat` (như `concat`) - **chốt: sret StackSlot + `ReturnShape::Struct`** (G xác nhận); (d) `free/deinit-shim` - heap bỏ cap nên free phải nhận cap từ StackSlot, **chốt: `free(ptr, cap)` 2-arg-rời**, cấm `*const FatStr`. Sẽ đo trong quá trình implementation.
   - **Nguyên tắc G (ABI):** `*mut FatStr` CHỈ dùng cho shim mutate-writeback (`append`/`clear`). Mọi shim còn lại — read-scalar, read-buffer, free/consuming — bung field rời qua `stack_load`, không truyền struct-ptr (tránh deref thừa + lãng phí L1 cache).
2. **Q1/Q3 - Deinit & Move (Tombstone):** *Chốt Design-level.*
   - Tombstone chống double-free: Khi di chuyển (move) fat-pointer, chỉ cần zero `ptr` (field 0) là thao tác load-bearing. Zero `len/cap` chỉ mang tính vệ sinh. Hàm free sẽ tận dụng guard sẵn có `if ptr == 0`.
   - *DEFER:* Spike hiện tại chỉ là mock ở tầng Rust. Tác vụ chuyển đổi `def_var` sang `stack_store(0, slot, 0)` trong Cranelift lowering (cho Move và Deinit thật) được bảo lưu cho bước implementation.
3. **Q4 - Exclusivity (E2440):** *ĐÓNG cho scope String-only.*
   - Việc chuyển từ 1 i64-handle sang 3-field StackSlot là chi tiết ở tầng codegen (dưới MIR). Ở tầng borrowck, 1 String vẫn là 1 Place. Các luật Exclusivity E2440 độc lập hoàn toàn với StackSlot nên vẫn chạy đúng.
   - *CAVEAT (Slice):* Thiết kế fat-pointer này dọn đường cho mảng "cheap sub-slice sharing". Tuy nhiên, borrowck hiện tại chưa có mô hình provenance liên kết alias giữa một sub-slice (được sinh ra dưới dạng 1 local mới) với buffer của String gốc. Vấn đề "slice provenance" nằm ngoài scope này và sẽ thuộc về một ADR khác trong tương lai.
## 5. Consequences
- **Tích cực:** Mở đường cho Heap memory management chuẩn mực, tối ưu hiệu năng append, mở khóa tính năng slice.
- **Tiêu cực:** Tăng độ phức tạp của StackSlot, cần viết lại toàn bộ logic FFI shims và deinit cho String.
- **Nợ kỹ thuật Bậc D:** Cần giải quyết `is_propagated` bypass (nested scope) và hợp nhất 2 tầng borrowck (typecheck + MIR) như đã ghi nhận từ handoff.
