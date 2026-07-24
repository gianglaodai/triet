# ADR 0086 — Taxonomy mã lỗi `LowerError` (`triet::lower::E11XX`)

**Trạng thái:** Quyết định (WO-Front-A, O + G ký). Áp dụng từ Bậc C.
Chuyển `LowerError` (`crates/triet-lower/src/lib.rs`) từ struct phẳng
`{message, span}` KHÔNG mã lỗi sang `enum` 8 biến thể, mỗi biến thể mang một
mã `triet::lower::E11XX` + `#[derive(miette::Diagnostic)]`, mirror
`CapabilityError` (`crates/triet-typecheck/src/capability_check.rs`).

**Issue:** `LowerError` là struct duy nhất trong pipeline compiler KHÔNG có
mã lỗi và KHÔNG impl `miette::Diagnostic` — vi phạm CLAUDE.md §Error code
namespace (mọi tầng khác đã có: lexer E0000, parser E000X, typecheck E10XX,
modules E21XX, capability E22XX, borrowck E24XX, actor E25XX) và ADR-0027
(diagnostic format standard). Driver in lỗi bằng `eprintln!("{path}: lowerer
error: {e}")` — chữ trần, không span, không code, không cùng khuôn dạng với
parse/typecheck/borrowck (vốn render qua `miette::Report` +
`GraphicalReportHandler`). Có ~47 điểm dựng `LowerError` trong lowerer (8 hàm
dựng đặt tên + ~39 điểm dựng nội tuyến) — không có mã để tra cứu, không có
cách phân biệt "chương trình user sai" với "compiler chưa hỗ trợ" với
"invariant nội bộ vỡ (ICE)" ngoài việc đọc chuỗi message.

## Quyết định

**8 mã, 4 lớp ngữ nghĩa**, dựa trên recon toàn bộ ~47 điểm dựng lỗi hiện có
trong `crates/triet-lower/src/lib.rs`:

| Mã | Variant | Lớp | Ý nghĩa |
|---|---------|-----|---------|
| `E1100` | `ConstructNotYetLowered` | Compiler-completeness gap | AST construct hợp lệ về ngữ nghĩa nhưng backend hiện tại chưa lower — KHÔNG phải lỗi chương trình user. |
| `E1120` | `NullableEnumPayloadUnsupported` | Design fence | `Enum?` payload-bearing trong aggregate — disc-niche nullable repr (ADR-0065 §12.7) chỉ sound cho unit-only enum. Khóa kiến trúc, không phải "chưa làm". |
| `E1121` | `NullableStructReturnHeapField` | Design fence | `Struct?` return có field heap-bearing — sret buffer tag-prepend không có drop-glue (ADR-0065 §4 B8). Khóa kiến trúc. |
| `E1122` | `EscapingClosureSealed` | Design fence | Closure escaping/first-class (`Expr::Lambda`) bị niêm phong CÓ CHỦ Ý (YAGNI, ADR-0039 recon) — không phải lỗ hổng compiler. |
| `E1140` | `UndefinedLocal` | User error | Biến local không tồn tại trong scope. |
| `E1141` | `NullLiteralWithoutExpectedType` | User error | Constructor `~+`/`~0`/`~-` thiếu expected type từ ngữ cảnh để suy ra kiểu đích. |
| `E1142` | `LiteralOutOfRange` | User error | Giá trị literal (Trit/Tryte/Long/Integer trong pattern match) vượt phạm vi biểu diễn được của kiểu. |
| `E1190` | `InternalInvariant` | ICE (Internal Compiler Error) | Một bất biến nội bộ lowerer dựa vào (name resolution đã resolve, exhaustiveness scan, fixpoint hội tụ, …) bị vi phạm. Đây là **compiler bug**, không phải lỗi chương trình user — help text yêu cầu report kèm input tối thiểu. |

### Vì sao E1190 gom TẤT CẢ 35/47 site còn lại vào MỘT mã ICE

Sau khi tách 4 mã "design fence" (E1120/E1121/E1122 — khóa kiến trúc có ADR
riêng) + 3 mã "user error" (E1140/E1141/E1142 — chương trình user thật sự có
thể viết ra để kích hoạt) + 1 mã "completeness gap" (E1100 — construct hợp lệ
chưa lower), phần còn lại (35/47 điểm dựng nội tuyến) đều là các nhánh
**"typecheck lẽ ra đã từ chối trước khi tới đây"**:

- Trùng lặp arm (`duplicate ~+ arm`, `duplicate ~0 arm`, `duplicate catch-all`)
  — exhaustiveness/uniqueness đã được typecheck kiểm tra (SPEC §A1.2); nếu
  lowerer thấy trùng nghĩa là lowerer đang chạy trên AST mà typecheck lẽ ra
  phải chặn.
- Pattern sai hình dạng (`unsupported sub-pattern in ~+ arm`, `~- arm on
  nullable type`, …) — cùng lý do: typechecker gate hình dạng pattern theo
  kiểu scrutinee trước khi lowerer thấy nó.
- Name resolution chưa resolve (`unresolved enum variant`, `unknown enum`,
  `unknown variant`) — `pattern_resolutions`/`method_resolutions` là output
  của typecheck; một entry thiếu là lowerer đọc bảng sai, không phải input
  user sai.
- Fixpoint không hội tụ (`struct/enum layout sizing did not converge`) —
  ADR-0068 cấm kiểu đệ quy/Box nên đồ thị kiểu luôn là DAG hữu hạn; không
  hội tụ nghĩa là bất biến "DAG hữu hạn" bị vi phạm ở đâu đó ngược dòng.
- Elision bất biến vỡ (`return-borrow elision expects exactly 1 ref-param`) —
  comment tại chỗ đã ghi rõ "typecheck E2400 should have rejected this".

Gom vào một mã thay vì 35 mã riêng vì: (a) không có ADR/spec section nào cho
mỗi trường hợp — chúng không phải quyết định thiết kế, chúng là "điều này
không nên xảy ra"; (b) hành động sửa của người dùng là GIỐNG NHAU cho cả 35
site (không phải sửa chương trình — báo compiler bug); (c) tách nhỏ hơn sẽ
tạo ảo giác rằng mỗi nhánh là một "loại lỗi user" riêng biệt, trong khi thực
tế cả 35 đều thuộc một lớp: "một tầng trước lowerer lẽ ra phải chặn cái này".
Message text giữ nguyên tại mỗi site (không đổi), nên thông tin chi tiết
(tên biến/variant/pattern cụ thể) không mất — chỉ gom chung một mã tra cứu.

### 2 ruling biên (site không tự nhiên khớp 3 lớp trên)

- **`:5419` (bản gốc, dòng trôi theo edit) → `E1100`**, không phải `E1190`.
  Site này refuse trả về `Vector`/`HashMap`/`Enum`/`Reference` từ trait
  method (`callee_ret` không phải scalar) — comment tại chỗ ghi rõ "nợ #2
  scope": đây là backend CHƯA CÓ ABI cho các trường hợp trả-về này (multi-
  value return, Outcome 2-reg ABI, …), không phải một bất biến bị vi phạm.
  Chương trình user hoàn toàn hợp lệ (trait method trả `Vector` là ngữ nghĩa
  đúng) — chỉ là backend hiện tại chưa lower được. Đây là compiler-
  completeness gap kinh điển → `E1100`.

- **`:5935` (bản gốc) → `E1122`**, không phải `E1100` hay `E1190`. Site này
  refuse `Expr::Lambda` (closure escaping/first-class). Comment tại chỗ ghi
  rõ đây là niêm phong CÓ CHỦ Ý (YAGNI theo ADR-0039 recon Phase 14.0) —
  các họ toán tử nullable/Outcome (`~+>`, `~->`) lower qua AST node riêng,
  không có consumer nào cần closure escaping thật. Đây KHÔNG phải "chưa làm
  xong" (E1100) — làm xong sẽ không bao giờ xảy ra vì thiết kế chủ động
  không có đường vào. Cũng không phải ICE (E1190) — không có bất biến nào bị
  vi phạm, đây là refuse có chủ đích trên input hợp lệ về cú pháp/type. Do
  đó cần mã riêng `E1122` thay vì rơi vào một trong hai lớp còn lại.

## Các phương án đã cân nhắc

| # | Phương án | Ưu | Nhược | Kết luận |
|---|-----------|---|-------|----------|
| 1 | Giữ struct phẳng, chỉ thêm field `code: &'static str` | Ít việc nhất | Không có `#[diagnostic]`/miette rendering, không type-safe (code là string rời rạc với message), driver vẫn phải tự lắp `Report` thủ công | Loại — không đạt parity với typecheck/borrowck/capability. |
| 2 | Enum 47 biến thể (1 biến thể / 1 điểm dựng) | Độ chi tiết tối đa | 35/47 site không có ADR/ý nghĩa riêng biệt để đặt tên — tạo ảo giác 35 "loại lỗi" khác nhau trong khi cùng một lớp "invariant vỡ"; bảo trì nặng (mỗi refactor lowerer phải sửa tên biến thể) | Loại — vi phạm "Simplicity First" (CLAUDE.md §2). |
| 3 | Enum 8 biến thể theo 4 lớp ngữ nghĩa (đã chọn) | Cân bằng: đủ chi tiết để phân biệt user-error/design-fence/gap/ICE, message text giữ nguyên nên không mất thông tin cụ thể | Lớp ICE (E1190) gom 35 site khác nhau dưới 1 mã — cần ADR này để giải trình rõ ranh giới | **Chọn.** |

## Hậu quả

### Tích cực
- `LowerError` đạt parity với `TypeError`/`BorrowError`/`CapabilityError`/
  `ConcurrencyError` — tất cả đều `miette::Diagnostic` với mã `triet::<area>::EXXXX`.
  CLAUDE.md §Error code namespace bổ sung dòng `triet::lower::E11XX`.
- Driver in lỗi lowerer bằng cùng khuôn dạng `miette::Report` +
  `NamedSource` như parse/typecheck/borrowck (span highlight thay vì chữ trần).
- 8 named constructor giữ nguyên signature — 47/47 call site không cần sửa
  chữ ký gọi hàm, chỉ 39 điểm dựng nội tuyến đổi `LowerError { .. }` thành
  `LowerError::Variant { .. }`.
- Test `tests/diagnostics.rs` (mới) khóa cứng 8 mã bằng assertion trực tiếp
  trên `miette::Diagnostic::code()`; 2 trong 8 (`E1120`, `E1121`) trigger qua
  fixture thật (414, 440) thay vì hand-built, chứng minh mã thật sự phát ra
  từ pipeline, không chỉ từ construct-tay.

### Tiêu cực
- Mã `E1190` mất độ chi tiết: 35 nguyên nhân gốc khác nhau chia sẻ một mã
  tra cứu. Người đọc log phải đọc `message` (giữ nguyên, đủ chi tiết) để biết
  chính xác bất biến nào vỡ — mã chỉ nói "đây là ICE".
- `crates/triet-lower/Cargo.toml` thêm 2 dependency (`thiserror`, `miette`)
  — build time crate này tăng nhẹ (không đáng kể so với `triet-typecheck` đã
  có sẵn hai dep này).

### Rủi ro cần mitigate
- Nếu một site tương lai nào đó thuộc E1190 hóa ra CÓ THỂ bị kích hoạt bởi
  chương trình user hợp lệ (nghĩa là tầng typecheck KHÔNG chặn được như giả
  định) — đó là một lỗ hổng typecheck cần vá riêng, không phải lý do đổi mã
  của site đó sang user-error. Việc gán nhầm lớp chỉ nên sửa SAU KHI đã chứng
  minh (Rule #7 refuse-over-guess) rằng typecheck thực sự không chặn được.

## Ngày hiệu lực

- Bậc C+ — áp dụng ngay khi WO-Front-A merge.
- Không áp dụng hồi tố cho log/báo cáo cũ đã dùng message trần (git history
  giữ nguyên struct cũ, không cần migrate ngược).
