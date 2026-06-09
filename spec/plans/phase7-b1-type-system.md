# B1a — Rombac MirType (Crusade #3)

**Blueprint chỉnh theo ADR-0050 + CORRECTION §3.1.1/§3.1.2.**
HEAD `58a8519`. Gate baseline **0·0·99·208**.

## Tóm tắt

Bỏ string-match trong backend (MIR/lower/jit/borrowck), thay bằng `enum MirType`
viết tay trong `triet-mir`. Đây là **MÓNG cho B2** (sáp nhập 2 tầng borrowck).

**Quyết định kiến trúc:** ADR-0050 ký O+G 2026-06-09. 3 bất biến G:
① MirType viết-tay-trong-mir · ② TÁCH Struct/Enum (cấm UserType) ·
③ parse shim chết ở commit cuối.

## Khảo sát phạm vi (đồng bộ ADR §2.2)

| Đối tượng | Số site | Ghi chú |
|-----------|---------|---------|
| `is_copy` (consumer) | 49 | |
| `is_nullable_type` | 14 | |
| `nullable_payload` | 14 | |
| `is_vec_type` | 14 | |
| `is_hashmap_type` | 7 | |
| ref-form `starts_with('&')` | 9 | |
| jit type-dispatch | 22 | 20 literal `__triet_*` là tên shim — GIỮ |
| Producer (cổ chai duy nhất) | `type_name` | lower:613 |
| `simple_is_copy` | 12 | BOM — bản sao thứ hai của is_copy |
| `struct_names`/`enum_names` | 35 | HashSet + method check |
| TECH-DEBT comments | 4 | "MIR-type-as-string" |

**Tổng ~180 điểm va chạm.** Mỗi điểm đều là một trong: đổi `&str`→`&MirType`,
thay `starts_with('&')`→`matches!(ty, Reference{..})`, hoặc xóa (helper cũ).

### Producer của type string

`type_name()` ở `triet-lower/src/lib.rs:613` là NGUỒN DUY NHẤT sinh type string:

```rust
fn type_name(arena: &Arena, id: TypeId) -> String {
    match &arena.type_expression(id).node {
        TypeExpr::Named(n) => n.clone(),                    // "Integer", "String", "Point"
        TypeExpr::Nullable(inner) => format!("{inner}?"),   // "Integer?"
        TypeExpr::Reference { form, inner } => {            // "&0 String"
            format!("{prefix}{inner_name}")
        }
        _ => "?".to_string(),                               // fallback
    }
}
```

→ Đổi thành `lower_type(arena, id, struct_names, enum_names) -> MirType`.

## Thiết kế MirType (đồng bộ ADR §3.1 + CORRECTION §3.1.1/§3.1.2)

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirType {
    // Scalars — Copy (SPEC §10.1)
    Integer, Trit, Tryte, Long,
    Trilean,             // TRẦN — refined là gate frontend (CORRECTION §3.1.2)
    Unit,
    Unknown,             // thay sentinel "?"

    // Heap — Move (ADR-0042)
    String,
    Vector,              // TRẦN — 0 consumer đọc element type (CORRECTION §3.1.1)
    HashMap,             // TRẦN — 0 consumer đọc key/value type

    // Modifiers
    Nullable(Box<MirType>),   // KẾT CẤU → giết ordering-rule
    Reference { form: ReferenceForm, inner: Box<MirType> },
      // ^ reuse triet_mir::ReferenceForm (lib.rs:407), KHÔNG dep mới

    // User types — TÁCH theo phán quyết G câu 2
    Struct(String),      // resolve qua body.struct_layouts
    Enum(String),        // resolve qua body.enum_layouts
}
```

**Ghi chú:**
- Tên `MirType` (không `Type`) — tránh va nghĩa với `typecheck::Type` và `generated::Type`.
- `Vector`/`HashMap` TRẦN: O probe trước S1 — 0 consumer backend đọc element/key/value type.
  Khi Bậc C cần generic Vector thật, thêm payload + consumer CÙNG commit (Rule #4).
- `Trilean` TRẦN: refinement là gate frontend (ADR-0021), kiểm xong TRƯỚC MIR.
  0 backend đọc field `refined` → dead field nếu có.
- `Display` round-trip về string-form cũ (chỉ cho diagnostic/fixtures).
- `parse(&str)` transitional — gắn `// TECH-DEBT(B1a): MUST KILL THIS SHIM`, chết ở S4.

## Kế hoạch triển khai (4 stage, mỗi stage 1 commit XANH)

### Nguyên tắc: MỖI COMMIT GATE XANH

Không commit nào được phép gate đỏ. Dùng Display-bridge: consumer chưa migrate
gọi `ty.to_string()` để giữ hành vi cũ trong cùng commit field type đổi.

### S1 — Giới thiệu song song (gate xanh, +0 hành vi)

**File:** `triet-mir/src/lib.rs`

- Thêm `enum MirType` + `impl Display` + `MirType::parse(&str) -> MirType`
  (gắn `// TECH-DEBT(B1a): MUST KILL THIS SHIM`)
- Thêm method: `is_nullable()`, `nullable_payload()`, `is_reference()`,
  `is_vec()`, `is_hashmap()`, `is_copy(&self, body: Option<&Body>)`
- `is_copy` là MỘT logic duy nhất:
  - `is_copy(None)`: classify theo variant (không recurse layout) — dùng
    trong lowerer trước khi Body tồn tại
  - `is_copy(Some(&body))`: classify + recurse vào struct/enum layout —
    dùng trong borrowck + jit
- `pub ty: String` GIỮ NGUYÊN ở cả 3 field site (LocalDecl/FieldLayout/PayloadLayout)
- Chưa consumer nào đổi — MirType sống song song
- Viết unit test port từ test cũ (is_copy primitives/reference/heap/struct/enum/nullable)

**Verify:** `cargo test -p triet-mir` xanh. Gate xanh.

### S2 — Flip producer + field type (gate XANH — Display-bridge)

**Files:** `triet-mir/src/lib.rs`, `triet-lower/src/lib.rs`

- Đổi 3 field `pub ty: String` → `pub ty: MirType` (LocalDecl:165, FieldLayout:794,
  PayloadLayout:842)
- `lower::type_name(arena,id)` → `lower::lower_type(arena,id,struct_names,enum_names) -> MirType`:
  - `TypeExpr::Named(n)` → map literal: "Integer"→Integer, "String"→String,
    "Vector"→Vector, "HashMap"→HashMap, "Trilean"→Trilean, v.v.
  - User-defined: lookup struct_names/enum_names → `Struct(name)` / `Enum(name)`
  - `Nullable(inner)` → `MirType::Nullable(Box::new(lower_type(...)))`
  - `Reference{form,inner}` → `MirType::Reference{form, inner: Box::new(...)}`
  - Fallback → `MirType::Unknown`
- **Consumer chưa migrate: bridge tạm bằng `.ty.to_string()`** tại mỗi site
  đọc `.ty` — Display sinh ra string-form cũ → hành vi không đổi
- **XOÁ `simple_is_copy`** (lower:652) + test `simple_is_copy_agrees_with_canonical`
  (lower:2906). Mọi call `simple_is_copy(ty, ...)` → `ty.is_copy(None)` (MỘT logic,
  không còn bản sao)
- `struct_names`/`enum_names` HashSet: GIỮ trong lowerer (cần cho `lower_type`
  phân biệt Struct vs Enum). Sẽ xóa ở S3 khi consumer migrate xong.

**Verify:** `cargo test --workspace` xanh. `bash scripts/gate.sh` — gate xanh.
Đây là stage "nguy hiểm nhất" — Display-bridge đảm bảo gate không vỡ khi field
type thay đổi.

### S3 — Migrate consumer theo cụm

Thứ tự: **mir → lower → borrowck → jit** (theo chiều dependency).

Trong mỗi crate, thay thế:

| Pattern cũ | Pattern mới |
|-----------|-------------|
| `ty == "String"` | `matches!(ty, MirType::String)` |
| `is_vec_type(s)` / `is_hashmap_type(s)` | `matches!(ty, MirType::Vector)` / `matches!(ty, MirType::HashMap)` |
| `is_nullable_type(s)` | `ty.is_nullable()` |
| `nullable_payload(s)` | `ty.nullable_payload()` |
| `starts_with('&')` | `matches!(ty, MirType::Reference{..})` hoặc `ty.is_reference()` |
| `is_copy(s, body)` | `ty.is_copy(Some(body))` |
| `layout.name == "String"` | `matches!(body.local_decls[local].ty, MirType::String)` |
| `struct_names.contains(name)` | `matches!(ty, MirType::Struct(_))` |
| `Ctx::is_fat_type(name)` | `matches!(ty, MirType::Struct(_) \| MirType::String)` |

**20 literal `__triet_*` shim name GIỮ NGUYÊN** — dispatch dựa trên tên shim,
không phải type.

Sau mỗi cụm: `cargo test --workspace` + 3 fixture canary (INV-4, fixture-27,
fixture-100 String round-trip) phải xanh.

Khi tất cả consumer đã migrate: xóa `struct_names`/`enum_names` HashSet khỏi
`Ctx` (lower) — thông tin đã được encode trong `MirType::Struct`/`MirType::Enum`.

**Verify:** Gate xanh sau mỗi cụm. Cuối S3: 0 call site dùng helper cũ
(nhưng `parse` shim vẫn tồn tại trong code — chưa xóa).

### S4 — Nhổ răng cưa (commit cuối B1a)

- **XOÁ `MirType::parse`**. Build sẽ nổ đỏ ở mọi chỗ còn dùng parse → fix đến
  xanh (mỗi chỗ nổ = một site quên migrate ở S3).
- `Display` GIỮ (chỉ cho diagnostic).
- Xóa 4 TECH-DEBT comments "MIR-type-as-string" (lower:623,661, MIR:2369 + parse shim).
- Verify tiêu chí done (dưới).

**Verify:** Gate 0·0·99·208. Teeth bắt buộc ĐỎ khi poison.

## Regression watch (ADR §5)

| Canary | File | Cách verify |
|--------|------|-------------|
| A2 INV-4 verifier | `mir/lib.rs` | Poison verifier → test INV-4 phải ĐỎ |
| Bậc D fat-pointer | `jit/mir_lower.rs` | Fixture 100 String round-trip 5-boundary |
| C1 fixture-27 | `jit/` | Enum-payload-qua-param phải giữ xanh |
| ordering-rule fix | MIR | `MirType::Nullable(Vector).is_vec() == false` (đúng) |

## Rủi ro & mitigation

### R1 (đã tan — CORRECTION §3.1.1): Vector/HashMap generic parsing

Vector/HashMap TRẦN → producer `lower_type` chỉ-đọc-arena, KHÔNG đụng typecheck
`type_map`. Rủi ro giải thể.

### R2 (đã tan — CORRECTION §3.1.1): struct_names/enum_names lookup

Giữ nguyên struct_names/enum_names HashSet trong lowerer cho `lower_type` phân biệt
Struct vs Enum. Xóa sau S3 khi consumer migrate xong. Không cần lookup mới.

### R3: JIT `layout.name == "String"` pattern

~15 chỗ JIT check `layout.name == "String"` qua struct_slots. Sau migration,
thay bằng `body.local_decls[local].ty` → `matches!(ty, MirType::String)`.

### R4: Display format phải giữ nguyên

Mọi diagnostic message dùng type name. `Display for MirType` sinh ra format
giống hệt cũ. Có test pin round-trip cho từng variant.

### R5: Bậc D regression

Mọi `== "String"` → `matches!(ty, MirType::String)` phải review từng dòng.
Fixture 100 là canary.

### R6: Diff lớn, staged

~180 điểm trên 4 crate. Mỗi stage 1 commit xanh, revert được từng bước.
Không commit monolithic.

## Tiêu chí done B1a (O verify bằng lệnh, không niềm tin)

```
rg 'fn parse' crates/triet-mir/src          → 0 hit MirType::parse
rg 'is_vec_type|is_hashmap_type|is_nullable_type|nullable_payload' crates/*/src → 0
rg 'simple_is_copy' crates/triet-lower/src  → 0
rg "starts_with\\('&'\\)" crates/*/src       → 0
bash scripts/gate.sh   → build 0 · test 0-fail · fixtures 99 · clippy justify (baseline 208)
cargo test --workspace → exit 0, no panicked/error[
```

Teeth bắt buộc trước khi đóng (poison từng cái → test chỉ định ĐỎ):
- String → is_copy=true ⇒ move-test đỏ.
- Nullable wrap bỏ ⇒ ordering-test (Vector<Integer>?) đỏ.
- Struct("X") tra nhầm bảng enum ⇒ resolve đỏ (lợi ích G yêu cầu tách).
- A2 INV-4 + fixture-27 đỏ khi poison verifier/enum-payload.

## Ranh giới — KHÔNG đụng trong B1a

B1b (typecheck↔schema Type) · concat→sret · B2 sáp nhập borrowck ·
B3 alias-analysis · C1 enum-payload mở rộng.
