# ADR 0088 — Double-Nullable Container Reads (`T??` on `get`-family)

**Trạng thái:** Deferred / Backlog (Mentor G ký defer 2026-07-25). KHÔNG áp
dụng cho bất kỳ version nào — refuse tường minh (E1051) giữ chỗ cho tới khi
`T??` được thiết kế đầy đủ.

**Issue:** `get`/`get_ref` trên container (`Vector<T>`/`HashMap<K,T>`) trả về
`T?` — outer `?` mã hoá "key/index có tồn tại hay không". Khi `T` bản thân đã
là `T0?` (giá trị lưu trong container CÓ THỂ null, ví dụ
`HashMap<Integer,Integer?>`), kết quả logic của `get` là **double-nullable**:
`(T0?)?` / `T0??` — outer `?` = "key found", inner `?` = "giá trị lưu là
null". Trước WO-GetRefNullableRefuse, trường hợp này rơi xuống
`NoMatchingOverload` (E1041) generic — một lỗi ĐÚNG (không compile được)
nhưng SAI LÝ DO (trông như "không có overload nào" chứ không phải "double-
nullable chưa được thiết kế").

Tension cụ thể lộ ra khi recon: `crates/triet-mir/src/lib.rs:4030` (comment
test `nullable_type_helpers_round_trip`) viết *"Edge case: 'Integer??' — can't
happen (C6: T?? auto-flatten), but helper must be defined"*, và ngay dòng
`:4032` test vẫn tự tay dựng `MirType::Nullable(Box::new(MirType::Nullable(
Box::new(MirType::Integer))))` để chứng minh `is_nullable()`/
`nullable_payload()` không panic trên hình dạng đó — tức là **MIR CÓ THỂ
biểu diễn `T??`** dù comment khẳng định nó "không xảy ra được". Đối chiếu
`crates/triet-typecheck/src/lib.rs` (`nullable_flatmap_flattens_nullable_body`,
~dòng 89): auto-flatten **CHỈ xảy ra tại một điểm cụ thể** — toán tử `?+>`
(flatMap) của `T?`, nơi body trả `U?` được flatten về `U?` (không phải
`U??`) theo thiết kế của chính operator đó (ADR-0020 §3 ternary map family).
Đây KHÔNG phải một bất biến toàn cục của type-system — nó là hành vi cục bộ
của MỘT toán tử. `get()` trên container không đi qua `?+>`, nên không có gì
tự động flatten hộ nó; nếu không refuse, typecheck sẽ hạ type `(T0?)?` xuống
overload table và rơi vào hành vi không xác định (silent-flatten ở tầng nào
đó, hoặc double-nullable sống sót tới MIR/JIT nơi ABI 1-bit-sentinel
(`i64::MIN`, ADR-0041 PA-3c) không có chỗ cho 2 tầng null độc lập).

## Quyết định

**Refuse tường minh** double-nullable container reads tại typecheck
(`E1051`, `crates/triet-typecheck/src/error.rs`), thay vì:
(a) silent-flatten (mất phân biệt not-found vs stored-null — SAI kết quả), hoặc
(b) ship một biểu diễn `T??` nửa vời chỉ đủ cho `get`-family mà chưa được
kiểm chứng ở AST/MIR/JIT/match.

Guard đặt trong `resolve_overload` (`crates/triet-typecheck/src/check/
exprs.rs`), ngay sau guard heap-violation hiện có (ADR-0077 Slice B / ADR-
0078 P1b) và trước aggregate-key arm (ADR-0083 §5): nếu `name == "get"` và
container (`Vector<V>`/`HashMap<_,V>`, kể cả qua `&0` borrow) có `V =
Nullable(_)` → push `TypeError::GetContainerNullableValueUnsupported` (E1051),
`return Type::Unknown`. Guard KHÔNG chặn `contains` (chỉ trả `Trilean!`
found/not-found, không có double-nullable nào phát sinh).

### Hình thức cụ thể

```
function f(m: &0 HashMap<Integer, Integer?>, k: Integer) -> Integer =
    get(m, k)  // E1051: get()/get_ref cannot return a double-nullable Integer?? value type
```

Refuse xảy ra TẠI typecheck — không có construct nào chạm MIR/JIT (không có
"refuse-gap" kiểu ADR-0065 §15/ADR-0085 — sound theo cấu trúc, không phải
theo may mắn).

## Các phương án đã cân nhắc

| # | Phương án | Ưu | Nhược | Kết luận |
|---|-----------|---|-------|----------|
| 1 | Silent-flatten `(T?)? → T?` giống `?+>` | Không cần lỗi mới, "chạy được" ngay | **Mất thông tin**: không phân biệt được "key không tồn tại" vs "key tồn tại, giá trị null" — hai tình huống ngữ nghĩa khác nhau bị gộp làm một, kết quả SAI cho chương trình dùng `HashMap<K, V?>` để lưu "có thể vắng mặt" một cách tường minh | Bác bỏ — sai hơn là refuse |
| 2 | Thiết kế đầy đủ `T??` (kiểu tuple `(present, present_inner, value)` hoặc tag 3-trạng-thái) ngay bây giờ | Giải quyết tận gốc | Đụng AST + typecheck + MIR layout + JIT ABI (sentinel hiện tại chỉ có 1 bit null) + match ergonomics — khối lượng một campaign riêng, chưa có ADR nào lock thiết kế 3-tầng | Defer — đúng như G quyết, không ship nửa mùa |
| 3 | **Refuse tường minh (E1051), defer thiết kế** | An toàn ngay lập tức, lỗi đúng lý do, không đóng cửa thiết kế tương lai | Người dùng tạm thời không dùng được `get` trên container-giá-trị-nullable (workaround: sentinel value hoặc wrapper Struct có cờ "present" tường minh) | **Chọn** |

## Hậu quả

### Tích cực
- `get`/`get_ref` không bao giờ trả một type mà pipeline (MIR/JIT ABI) không
  biểu diễn được đúng đắn — refuse xảy ra sớm nhất có thể (typecheck), trước
  khi bất kỳ MIR nào được lower.
- Lỗi đúng lý do (E1051 thay vì E1041 mơ hồ) — người dùng biết CHÍNH XÁC vì
  sao và cách tránh (sentinel/wrapper Struct).
- Không đóng cửa: khi `T??` được thiết kế đầy đủ (campaign riêng), guard này
  chỉ cần gỡ bỏ/thu hẹp — không có code nào khác phụ thuộc vào sự vắng mặt
  của `T??`.

### Tiêu cực
- `HashMap<K, V?>` / `Vector<V?>` bị hạn chế: `get`/`get_ref` không dùng
  được, phải `contains` trước rồi tính toán khác, hoặc đổi model dữ liệu.

### Rủi ro cần mitigate
- Guard hiện tại chỉ bắt `Vector(Nullable(_))`/`HashMap(_, Nullable(_))` một
  tầng nông — nếu tương lai `V` là aggregate CHỨA field `Nullable` (không
  phải bản thân `V` là `Nullable`), guard này KHÔNG bắt (đó là phạm vi khác:
  ADR-0082/0083 aggregate rules, không phải double-nullable). Không nhầm
  lẫn hai lớp refuse khi review code liên quan.

## Ngày hiệu lực

- Không áp dụng version nào — đây là backlog/deferred, không phải quyết định
  "locked" cho một tính năng đã ship. E1051 có hiệu lực ngay khi WO-
  GetRefNullableRefuse merge (refuse là code thật, ADR chỉ ghi nhận lý do +
  defer thiết kế đầy đủ).
- Mở lại khi có campaign riêng cho `T??` semantics (AST/typecheck/MIR/JIT/
  match đồng bộ) — ADR này là điểm neo tham chiếu, không phải thiết kế đó.
