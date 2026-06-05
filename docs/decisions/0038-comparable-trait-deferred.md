# ADR 0038 — `Comparable` trait với `compare() -> Trit` (design lock, implementation deferred)

**Trạng thái:** **Đã duyệt — DESIGN LOCK** (Author + Mentor, 2026-06-05).
**Implementation:** **DEFERRED** — chờ Trait system land. KHÔNG build built-in tạm.

## Bối cảnh

Author muốn phép so sánh 3 trạng thái ternary-native: thay vì ba phép `a < b` /
`a == b` / `a > b` (tư duy nhị phân), một phép `compare(a, b)` trả về 3 trạng thái
để `match` 3 nhánh. Đây là phép toán hợp bản sắc tam phân nhất có thể — so sánh
= dấu của hiệu, và SPEC đã có nền: `function sign(n: Integer) -> Trit`
(SPEC.md §quanh 1295) + nguyên tắc "hàm dấu là trit MSB khác 0 đầu tiên — không
cần phép so sánh riêng" (SPEC.md §quanh 486).

Đồng thời author xác nhận: **Triết chắc chắn sẽ có Trait** (không Interface).
Hiện tại (2026-06-05) chưa có: AST `Item` chỉ có Function/Struct/Enum
(`ast_item.rs:112-120`), lexer không có keyword `trait`/`impl`; "trait" mới chỉ
xuất hiện làm `GenericBound` và protocol built-in trên giấy (Iterator ADR-0003
chưa land, Display).

## Quyết định (4 điểm khóa)

1. **`Comparable` là TRAIT** (không Interface, không protocol built-in
   special-case), method `compare() -> Trit`. Triển khai khi Trait system land —
   Comparable là instance đầu tiên đi nhờ cỗ xe Trait, KHÔNG phải lý do
   gold-plate cả hệ trait.
2. **Kết quả là `Trit`** (Negative = less, Zero = equal, Positive = greater) —
   **KHÔNG** tạo `enum Ordering {Less, Equal, Greater}`. Lý do: (a) user enum
   dùng i64 discriminant (ADR-0037) — dùng enum cho ordering phản bội bản sắc
   tam phân; (b) Trit đã LÀ kiểu 3-trạng-thái; (c) tận dụng `sign` có sẵn.
   Tên `Ordering` cũng đã bị chiếm bởi atomic memory ordering (ADR-0026,
   SPEC.md §quanh 1098). Có thể cấp hằng đặt tên (`less`/`equal`/`greater` =
   `-1_trit`/`0_trit`/`1_trit`) cho match dễ đọc — vẫn là Trit.
3. **`compare` chỉ dành cho TỔNG thứ tự** (Integer/String/Tryte/…, không có
   unknown). So sánh dính **Trilean/unknown** Ở LẠI với operator `==`/`<`
   (Ł3-aware, trả Trilean) per SPEC §4.2 — bản sắc "compare with unknown ⇒
   result unknown" (SPEC.md §quanh 653) không bị `compare` nuốt mất. Nếu sau
   này cần so sánh bộ phận (partial order), dùng `Trit?` (null = không so sánh
   được) — quyết sau, không khóa ở đây.
4. **Operator `<` `<=` `>` `>=` `==` `!=` giữ nguyên** (SPEC §4.2, trả
   Trilean). Hai bề mặt tách bạch bằng kiểu trả về: operator = Trilean-aware
   để branch; `compare` = Trit để `match`/sort. Không desugar operator qua
   compare ở Bậc A.

## Vì sao defer (không phải lười)

- **Built-in tạm = đồ bỏ đi:** special-case `Comparable` bây giờ là skeleton
  biết trước sẽ đập khi Trait system thật land — vi phạm Phương án A
  (defer cleanly, not ship temporary).
- **Chưa có consumer:** backend ở Bậc A — chưa có Vector để sort, chưa có
  generic bound để ràng `T: Comparable`. Build bây giờ thì không gì gọi nó
  (bài học `enum_layouts` dead-field).
- **Không ai bị kẹt:** operator so sánh (SPEC §4.2) vẫn phục vụ đầy đủ; chỉ
  dạng 3-way match-able là chờ.

## Ghi chú implementation (cho phase tương lai)

- `match compare(a,b) { -1_trit => …, 0_trit => …, 1_trit => … }` là match
  trên **Trit literal** — đường lowering KHÁC enum-match 4g (SwitchInt keyed
  trên `enum_layouts`). Cần match-on-Trit path riêng.
- Trigger triển khai: Trait system land (trait decl + impl + dispatch tối
  thiểu) HOẶC nhu cầu sort/BTree trong stdlib — cái nào đến trước thì
  Comparable đi cùng.

## Tham chiếu

- SPEC.md §4.2 (operator so sánh, Ł3), §quanh 486 (hàm dấu), §quanh 653
  (unknown identity), §quanh 1295 (`sign -> Trit`).
- [ADR-0003](0003-iterator-protocol.md) — Iterator protocol (chưa land; sẽ là
  bạn đồng hành của Comparable trên Trait system).
- [ADR-0037](0037-enum-tagged-union-layout.md) — i64 discriminant cho user
  enum (lý do không dùng enum cho Ordering).
- [ADR-0026](0026-actor-boundary-send-rules.md) — `Ordering` (atomic) đã
  chiếm tên.
