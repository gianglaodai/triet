# ADR-0050: MirType — kết cấu hoá hệ type của MIR (trảm string-match)

## 1. Status
**Approved (O + G, 2026-06-09)** — Phase-0 Spike kế tiếp. Đây là **MÓNG cho B2**
(sáp nhập 2 tầng borrowck): borrowck cần một MIR-type vững chắc, không-đoán-mò
để tính liveness + exclusivity.

**Phán quyết G (bất biến — không được vi phạm trong implementation):**
1. **`MirType` viết tay trong `triet-mir`** (crate chỉ dep `triet-core`). MIR là
   Internal Representation của backend, **KHÔNG phải AST**. Ép MIR dùng chung
   schema `Type` (chứa `TypeId` cú pháp, native ints frontend chưa support) là
   leaky abstraction. → MirType cùng hạng với `Body`/`Statement`, không thuộc
   phạm vi schema-first (schema quản AST + S6 ownership, không quản IR backend).
2. **TÁCH `Struct(String)` và `Enum(String)` — CẤM gộp `UserType(String)`.**
   Tại thời điểm lowering, compiler biết chắc 100% name là struct hay enum.
   Vứt thông tin đó để rồi resolve phải mò cả hai bảng layout = lười biếng +
   refuse-over-guess ngược. Tách ra để hệ thống tự vạch mặt pha trỏ sai bảng
   (tra `Struct("X")` nhưng "X" là enum).
3. **Shim `MirType::parse(&str)` transitional ĐƯỢC phép — nhưng phải chết không
   tì vết ở commit CUỐI của B1a.** Gắn `// TECH-DEBT(B1a): MUST KILL THIS SHIM`.
   Stage cuối = nhổ răng cưa, xem codebase nổ đỏ ở chỗ nào quên migrate.

## 2. Context & Motivation

### 2.1. Ung nhọt: hệ type của MIR là string-match runtime có ordering ngầm
`triet-mir` **không có Type enum**. Mọi type là `pub ty: String` (3 field site:
`mir/lib.rs:165` `LocalDecl`, `:794` + `:842` field-layout), mang một **ngôn ngữ
con nhúng** parse lúc runtime ở mọi consumer:

- primitive: `"Integer"|"Trit"|"Tryte"|"Long"|"Trilean"|"Unit"`;
- `"?"` = sentinel "unknown" (pin: **không** phải nullable-của-rỗng);
- heap: `"String"`, `"Vector"`/`"Vector<…>"`, `"HashMap"`/`"HashMap<…>"`;
- **suffix `?`** = nullable, với **ordering-rule NGẦM**: mọi consumer phải gọi
  `is_nullable_type` TRƯỚC `is_vec_type`, nếu không `"Vector<Integer>?"` bị phân
  loại nhầm thành bare Vector. Đây là invariant *văn bản* (doc-comment), không
  *kết cấu* — một consumer quên thứ tự là sai âm thầm, gate vẫn xanh.
- **prefix `&+ /&+ mutable /&0 /&0 mutable /&- `** = 5 ref-form, parse bằng
  `starts_with('&')` (9 site).

### 2.2. Bề mặt va chạm (O grep toàn backend, 2026-06-09 — không đoán)
| Đối tượng | Site |
|---|---|
| `is_copy` (consumer) | 49 |
| `is_nullable_type` | 14 |
| `nullable_payload` | 14 |
| `is_vec_type` | 14 |
| `is_hashmap_type` | 7 |
| ref-form `starts_with('&')` | 9 |
| jit type-dispatch | 22 (20 literal `__triet_*` là **tên shim — GIỮ**) |
| Producer (cổ chai duy nhất) | `lower::type_name(arena,id)->String` (lower:613) |

### 2.3. Hai bom phụ (án tử trong B1a)
1. **`simple_is_copy` (lower:652) = bản sao thứ HAI** của logic move/copy. Test
   `simple_is_copy_agrees_with_canonical_is_copy` (lower:2906) tồn tại *chính vì*
   hai bản có thể trôi lệch → lỗ hổng soundness tiềm ẩn. **Hợp nhất về một nguồn.**
2. **ordering-rule nullable-before-vec** (§2.1) — diệt *kết cấu* bằng
   `Nullable(Box<MirType>)` wrap thay vì suffix-string.

### 2.4. Động lực — đây là MÓNG, không phải bugfix
B2 (sáp nhập 2 tầng borrowck) cần MIR-type không-đoán-mò để tính liveness +
exclusivity. String-match runtime với ordering ngầm là bom nổ chậm dưới chân B2.
Sai móng = sửa lần hai diện rộng → ADR-trước-code, hai chữ ký O+G.

## 3. Quyết định Kiến trúc

### 3.1. Hình dạng `MirType` (viết tay trong `triet-mir`)
```rust
pub enum MirType {
    // Scalars — Copy (SPEC §10.1)
    Integer, Trit, Tryte, Long, Trilean, Unit,
    Unknown,                                  // thay sentinel "?"
    // Heap — Move (ADR-0042)
    String,
    Vector,                                   // TRẦN — xem CORRECTION §3.1.1
    HashMap,                                  // TRẦN — xem CORRECTION §3.1.1
    // Modifiers
    Nullable(Box<MirType>),                   // KẾT CẤU → giết ordering-rule §2.1
    Reference { form: ReferenceForm, inner: Box<MirType> },  // reuse triet_mir::ReferenceForm
    // User types — TÁCH theo phán quyết G câu 2
    Struct(String),                           // resolve qua body.struct_layouts
    Enum(String),                             // resolve qua body.enum_layouts
}
```

### 3.1.1. CORRECTION post-probe (O, 2026-06-09) — Vector/HashMap TRẦN, KHÔNG payload
Bản ADR ký lần đầu viết `Vector(Box<MirType>)` + `HashMap{key,value}`. **O đo lại
trước S1 và rút:** không một consumer backend nào trích element/key/value type
(`rg "split('<'|generic|element_type|key_type" mir/jit/lower/borrowck` → 0 hit);
không diagnostic/fixture nào in generic-form `"Vector<Integer>"` (các assert
`"Vector<Integer>"` chỉ nằm trong unit-test của helper sắp bị xoá; fixtures
`.expected` rỗng generic-form; diagnostic lower:1581 in `"Vector or HashMap"` trần).
→ payload là **dead field**, vi phạm Track-B Rule #4. **Chốt: variant trần.**
Hệ quả: (a) rủi ro R1/R2 của blueprint phase7 (lo "phải đi qua typechecker Type
để parse generic args") **TAN** — producer `lower_type` chỉ-đọc-arena là đủ,
KHÔNG đụng typecheck `type_map`; (b) scope B1a co lại. Khi Bậc C cần generic
Vector thật, thêm payload + consumer CÙNG commit (đúng Rule #4) — không phải bây giờ.

### 3.1.2. CHỐT naming + Trilean (chống lệch blueprint)
- **Tên enum = `MirType`, KHÔNG `Type`.** `Type` trần va nghĩa với
  `typecheck::Type` và `generated::Type` (cả hai cùng tên) → nhầm 3 tầng. Bắt buộc `MirType`.
- **`Trilean` TRẦN, KHÔNG `Trilean { refined: bool }`.** Refinement (ADR-0021)
  là gate FRONTEND (quyết `if cond` hợp lệ), kiểm xong TRƯỚC khi tới MIR. O đo:
  0 chỗ backend match `Trilean { refined }` hay đọc field đó (các hit "refined"
  là chữ tiếng Anh thường trong doc-comment). → `refined` trong MIR = dead field,
  Rule #4. Giữ `Trilean` trần.
**Ghi chú dep:** `ReferenceForm` ĐÃ tồn tại trong `triet-mir` (`lib.rs:407`,
"mirrors triet_syntax"). `Statement::Borrow` đã mang `form: ReferenceForm` typed.
→ `MirType::Reference` reuse ngay, KHÔNG dep mới, KHÔNG enum trùng lặp.

### 3.2. Phân rã 5 helper string → method/match một-nguồn
- `is_nullable_type` → `matches!(self, MirType::Nullable(_))`.
- `nullable_payload` → `if let MirType::Nullable(inner) = self { Some(inner) }`.
- `is_vec_type` / `is_hashmap_type` → `matches!(self, MirType::Vector)` / `matches!(self, MirType::HashMap)` (variant trần — §3.1.1).
- `is_copy(&self, body)` → method match (recurse layout cho Struct/Enum giữ
  nguyên semantics canonical hiện tại).
- **`simple_is_copy` (lower) XOÁ** — gọi thẳng `MirType::is_copy`. Test
  `simple_is_copy_agrees_with_canonical` xoá theo (không còn 2 bản để so).

### 3.3. Producer
`lower::type_name(arena,id)->String` → `lower::lower_type(arena,id)->MirType`.
Đây là cổ chai DUY NHẤT sinh type — đổi một chỗ, mọi nơi nhận MirType.

### 3.4. `Display` cho diagnostic, `parse` transitional cho strangler
- `impl Display for MirType` — round-trip về string-form CŨ, **chỉ** cho
  diagnostic/error message (giữ output ổn định cho fixtures).
- `MirType::parse(&str) -> MirType` — shim transitional giữ gate xanh giữa các
  stage. **`// TECH-DEBT(B1a): MUST KILL THIS SHIM`**. Xoá ở stage cuối.

### 3.5. Outcome — giữ guarded, KHÔNG model trong B1a
`T~E`/`T?~E` chưa có producer (Outcome ops guarded `Err`). `type_name` hiện trả
`"?"` cho Outcome → MirType trả `Unknown`. Nhất quán, không phình scope.

## 4. Phạm vi & Defer (YAGNI)

### 4.1. Trong B1a (làm bây giờ)
Chỉ tầng backend: `triet-mir` + `triet-lower` + `triet-jit` + `triet-borrowck`.

### 4.2. Defer B1b (lát riêng, ADR sau) — KHÔNG chặn B2
Reconcile typecheck hand-written `Type` ↔ schema-generated `Type` thuộc
frontend/middle-end. Schema `Type` có **nợ thiết kế** chặn drop-in, để B1b giải:
- native ints `I8..U64/F64/Pointer` (disc 20-29) — frontend chưa support, nuốt
  vào = dead variant phạm Track-B Rule #2;
- `UserStruct.fields: Vec<StructField>` với `field_type: TypeId` (cú pháp) trộn
  vào type "đã resolve" (ngữ nghĩa) = lỗi phân tầng trong chính schema.
G xác nhận: MirType của B1a dư sức gánh B2; B1b giải sau là chia nhỏ rủi ro đúng.

## 5. Va chạm cần canh (regression watch)
- **A2 INV-4 verifier** đọc `ty` — đổi sang MirType phải giữ INV-4 đỏ khi vi phạm.
- **Fallback-invariant Bậc D** (fat-pointer String ABI) — JIT dispatch theo
  type; chuyển string→enum không được đổi hành vi lowering String.
- **C1 fixture-27** (enum-payload-qua-param) hiện ghim bởi string-match — migrate
  phải giữ fixture 27 xanh hoặc ghi nhận lý do.

## 6. Blueprint Implementation (Strangler, staged)

**Phase-0 Spike (throwaway, mẫu Bậc D):** dựng `MirType` + `Display` + `parse` +
`From`-producer trên nhánh nháp; convert `is_copy` + 5 helper; chứng minh
layout-lookup + Struct/Enum-tách + ordering-semantics giữ nguyên; đo clippy/test
delta. **VỨT.** Không chạm tree production.

**Production stages (mỗi stage = 1 commit, teeth đỏ TRƯỚC khi sang stage kế):**
1. **S1 — Giới thiệu song song.** Thêm `MirType` + `Display` + `parse` shim +
   method (`is_copy`/`is_nullable`/…). `ty: String` GIỮ NGUYÊN. Gate xanh.
2. **S2 — Flip producer.** `lower::type_name` → `lower_type -> MirType`. Field
   `LocalDecl.ty` / field-layout đổi `String→MirType`. Consumer chưa migrate
   tạm dùng `Display`/`parse` bridge. **Xoá `simple_is_copy`** + test cặp đôi.
3. **S3 — Migrate consumer theo cụm** (mir → lower → borrowck → jit). Thay
   `== "String"`/`is_vec_type(s)`/`starts_with('&')` bằng match `MirType`.
4. **S4 — Nhổ răng cưa.** XOÁ `MirType::parse`. Build phải nổ đỏ ở mọi chỗ quên
   migrate → fix đến xanh. `Display` GIỮ (diagnostic). 0 string-dispatch còn lại.

**Tiêu chí done B1a (verify bằng test, không niềm tin):**
- `rg 'parse\(' triet-mir` → 0 hit MirType::parse.
- `rg 'is_vec_type|is_hashmap_type|is_nullable_type|nullable_payload' src` → 0
  (đã thành method/match).
- `simple_is_copy` không còn tồn tại.
- Gate: build 0 · test 0-fail · fixtures 99 · clippy delta justify (baseline 208).
- A2 INV-4 + fixture-27 + String-lowering teeth đỏ khi poison.

## 7. Consequences
- **Tích cực:** xoá ordering-rule ngầm (kết cấu hoá), một-nguồn move/copy, tách
  Struct/Enum bắt lỗi trỏ-sai-bảng, móng vững cho B2. Type-safety từ lúc sinh ra.
- **Tiêu cực:** blast radius ~100+ site (nhưng MỘT tầng, staged). `Display` phải
  round-trip chính xác string-form cũ để fixtures không vỡ diagnostic.
- **Nợ mang sang (không đụng trong B1a):** B1b reconcile typecheck↔schema Type ·
  concat→sret · B3 alias-analysis (thay conservative=true).
