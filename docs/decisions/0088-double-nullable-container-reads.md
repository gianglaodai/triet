# ADR 0088 — Double-Nullable Container Reads (`T??` on `get`-family)

**Trạng thái:** **Lane A ĐÓNG** (O✅/G✅/Giang✅ 2026-07-27, `d9b659a` — xem
**§AMEND-1**: hàng rào `E1055` 2 tầng / 2 lớp nguồn + 9 răng cưa 511-519).
**Lane B (thiết kế `T??` thật) HOÃN VÔ THỜI HẠN.** Thân bài dưới đây là bản
gốc 2026-07-25 (defer `get`-family qua E1051) — nó CHỈ phủ `get`/`get_ref`,
và §Quyết định của nó có **một nhận định sai về `contains`** đã được §AMEND-1
§88A.4 đính chính. Đọc §AMEND-1 trước khi hành động theo thân bài.

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

---

## §AMEND-1 — Lane A: hàng rào `E1055` 2 tầng / 2 lớp nguồn + 9 răng cưa

**Trạng thái:** Lane A ĐÓNG (O✅/G✅/Giang✅, 2026-07-27, `d9b659a`).
**Lane B (thiết kế `T??` thật) HOÃN VÔ THỜI HẠN** — G chốt: khi chưa có
use-case thật đòi phân biệt "key không tồn tại" vs "giá trị lưu là null",
thiết kế repr 3-trạng-thái là xây cầu khi chưa có sông.

### §88A.1 — Recon lật khung: thân bài ADR này CHỈ phủ `get`-family

Thân bài trên chỉ nói về `get`/`get_ref`. O recon 20 probe trên binary
release (2026-07-27) đo ra bức tranh rộng hơn: **`T??` viết TRỰC TIẾP** (ngoài
`get`-family) chưa từng được ai đo. Kết quả: **KHÔNG có UB** — mọi đường
fail-closed — nhưng chẩn đoán VỠ: cùng một khái niệm `T??` đẻ ra **5 mã lỗi
khác nhau**, trong đó:

- 🔴 `struct S { v: Integer?? }` + match → **E1190** — mã **ICE**
  ("please report this as a compiler bug") cho chương trình user hợp lệ cú
  pháp. Vi phạm taxonomy ADR-0086 (E1190 dành riêng cho compiler bug).
- ⚠️ local/param/return/`pop`/`pop_front`/`remove`/`!!` → message MIR verifier
  nói *"heap-nullable… ADR-0065 §4 (B8) Struct?/Enum? Copy-only… [Fix 1]
  Remove the heap field from the struct/enum"* — **sai hướng hoàn toàn**:
  `Integer??` không có heap, không có struct. Vi phạm ADR-0027
  machine-fixable.
- ⚠️ `enum E { A(Integer??) }` → E1141 "đòi annotate" (sai nguyên nhân).
- ⚠️ `Integer??~E` → E0001 parse (lexer `?~` compound token).

**Cơ chế đang giữ trước Lane A:** `is_lowerable_nullable_payload`
(`crates/triet-mir/src/lib.rs:1796`) là một **allow-list** (scalar / heap /
Enum / Struct / Reference); `Nullable(_)` không có tên trong đó nên `T??` rơi
ra ngoài → refuse **by default**. Đây là **may mắn cấu trúc**, không phải hàng
rào chủ động — và **0 fixture nào canh giữ** (grep `??` trong dòng code của
toàn bộ corpus = 0 hit). Ai thêm arm `Nullable` vào allow-list đó — chính là
việc đầu tiên Lane B sẽ làm — thì 7 đường lọt xuống JIT cùng lúc, **câm, gate
vẫn xanh**. Đây là hình dạng SPOF-một-lớp đã bịt ở WO-SPOF-1.

### §88A.2 — HAI lớp nguồn sinh `T??` (lý do WO không thể "một điểm chạm")

| Lớp | Nơi sinh `Nullable(Nullable(_))` | Phủ |
|---|---|---|
| **A — khai báo** | `resolve_type`, **HAI bản sao**: `check.rs:1365` + `check_resolved.rs:597` | local annotation · param · return · struct field · enum payload · `!!` trên local có annotation |
| **B — suy diễn** | `check_call`, sau `return_type.substitute(&sub_map)` (`check/exprs.rs`) | `pop` / `pop_front` / `remove` trên container `<T?>` |

`let x = pop(v)` **không có annotation nào** để đi qua `resolve_type` — shape
chỉ tồn tại SAU substitution. Guard đặt ở Lớp A sẽ bỏ lọt trọn đường này.

⚠️ **Bất biến khắc:** Lớp A có **2 bản sao** `resolve_type` — cùng hình dạng
`is_fat_ret` 3-bản-sao (ADR-0065 §14.7). Ai đụng một bản PHẢI grep bản còn lại.

**Vị trí Lớp B — D bác gợi ý `env.rs` của WO và ĐÚNG:** `env.rs:374/394/506`
chỉ khai báo `pop`/`pop_front`/`remove` MỘT LẦN lúc khởi tạo env, với `T`/`V`
còn là `TypeParameter` trừu tượng — nó không biết `T` sẽ bind ra gì tại từng
call-site. `check_call` là chokepoint DUY NHẤT nơi shape thật sự tồn tại.
Guard **không name-gate**: bất kỳ generic function nào return `Nullable(T)` và
bind `T` ra nullable đều bị bắt như nhau (nhất quán với Lớp A).

### §88A.3 — Lớp MIR = tầng 2, và nó SẼ thành "code ma" nếu không có unit test

Sau khi Lớp A+B chặn ở typecheck, **không còn đường `.tri` nào chạm tầng MIR**.
Message tại `triet-mir/src/lib.rs` được sửa để mô tả đúng nested-nullable
(không còn mượn lời ADR-0065 heap/Struct?), và bắt buộc kèm **unit test riêng**
`nested_nullable_refused_with_correct_message` truyền thẳng
`MirType::Nullable(Nullable(Integer))` vào helper. Tiền lệ: N1/N3 ở ADR-0083
PA-A — N1 chặn mọi đường fixture nên N3 phải có răng riêng.

⚠️ **Bẫy sai-dương D tự phát hiện và tự sửa:** bản nháp message MIR có chứa
chuỗi `"(E1055)"`; harness so bằng `.contains(code)` nên poison gỡ guard
typecheck vẫn "xanh giả" (tầng MIR nổ, message tình cờ chứa "E1055"). D tự đào
ra, bỏ chuỗi mã khỏi message runtime, chạy lại bằng harness thật. **Message
runtime của tầng MIR KHÔNG được chứa chuỗi mã lỗi của tầng khác** — nếu không,
mọi poison xuyên tầng đều vô hiệu hoá câm.

### §88A.4 — ĐÍNH CHÍNH thân bài: `contains` KHÔNG "không bị chặn"

Thân bài §Quyết định viết: *"Guard KHÔNG chặn `contains` (chỉ trả `Trilean!`
found/not-found, không có double-nullable nào phát sinh)"* — **mô tả một hành
vi không tồn tại**. Đo thật (`triet-driver run`, 2026-07-27):

```
contains(m, 1)  với  m : HashMap<Integer, Integer?>
→ E1041 NoMatchingOverload
   available overloads: (String, String) · (Vector<Integer>, Integer)
   · (HashMap<Integer, Integer>, Integer) · (HashMap<String, Integer>, String) · …
```

`contains` **cũng không dùng được** với `V = Integer?` — nhưng vì overload
table không khai báo `V` generic, chứ KHÔNG phải vì nó được cho qua. Không
phải UB, không phải lỗ; là **nhãn tài liệu sai**. Ai đọc thân bài mà tưởng
`contains` là workaround hợp lệ cho `HashMap<K,V?>` sẽ trượt.

### §88A.5 — 9 răng cưa + giao thức verify 2 mũi poison

Fixtures **511–519** (7 đường trần, item `pop`-family tách 3 ca):
`511` local · `512` param · `513` return · `514` pop · `515` pop_front ·
`516` remove · `517` struct field (**cấm E1190**) · `518` enum payload
(**cấm E1141**) · `519` `!!`.

**Giao thức verify BẮT BUỘC cho mọi thay đổi chạm hàng rào này** (G phê chuẩn
sau khi O bác giao thức 1-mũi ban đầu của G — 1 mũi sẽ đẩy người verify vào
bẫy "poison không đỏ" rồi buộc phải bịa mũi giả cho nổ):

| Mũi | Poison | Kỳ vọng ĐÚNG |
|---|---|---|
| 1a | tắt guard Lớp A | **6** fixture đỏ: 511·512·513·517·518·519; 514·515·516 **xanh** |
| 1b | tắt guard Lớp B | **3** fixture đỏ: 514·515·516; 6 fixture kia **xanh** |
| 2 | nới allow-list MIR | **chỉ unit test MIR** đỏ; **0 fixture** đỏ |

O verify độc lập 2026-07-27, đủ cả 3 mũi + đặc hiệu hai chiều (6+3=9, không
lớp nào đội lốt lớp kia); dưới mũi 1a, `517`/`518` **lộ lại đúng ICE cũ**
(`unsupported match pattern` / `requires an expected type`) ⇒ guard mới chính
là thứ giết E1190/E1141. Răng chứng minh ở **tầng harness** (đổi `// ERROR:`
của 514 sang `E9999` → ra dòng FAIL expected/got, luật 15). Khôi phục bằng
`cp`-snapshot + md5 khớp, KHÔNG `git checkout`.

**Control chống over-refuse (phải giữ nguyên mãi mãi):** struct `Integer?`
MỘT tầng → `16` · `HashMap<K,Integer?>` insert-store → `5` · `?+>` flatMap
175/212/213 xanh (`exprs.rs:361-364` giữ body nullable, **không bao giờ sinh
`U??`**) · 465/466/467 **giữ E1051**, không bị E1055 cướp · 468 control dương.

### §88A.6 — Ranh giới E-code (một E-code, một hợp đồng)

- **E1051** — `get`/`get_ref` trên container có element/value nullable.
  GIỮ NGUYÊN, Lane A không đụng.
- **E1055** `NestedNullableUnsupported` — nested `T??` ở **mọi vị trí khác**
  (khai báo + suy diễn). Mã mới, cấp tại `triet-typecheck/src/error.rs`.
- Cấm gộp hai mã; cấm để E1055 cướp chỗ E1051.

### Ngày hiệu lực §AMEND-1

- Hiệu lực từ `d9b659a` (2026-07-27). Gate `0 · clean · 0 · 511 · 0 · CLEAN`
  (fixtures 502 → 511).
- Lane B mở lại **chỉ khi** có use-case thật + ADR thiết kế repr đồng bộ
  AST/typecheck/MIR/JIT/match. Ngày đó, việc đầu tiên là nới allow-list MIR —
  và 9 răng cưa trên sẽ nổ đỏ nếu Lane B chưa làm đủ. Đó chính là mục đích
  chúng tồn tại.
