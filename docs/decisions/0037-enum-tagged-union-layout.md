# ADR 0037 — Enum tagged-union layout trên Bậc A StackSlot

**Trạng thái:** **Đã duyệt** (Author + Mentor sign-off 2026-06-05).

## Tóm tắt

Đặc tả memory model cho user-defined `enum` trên hạ tầng Bậc A (Cranelift StackSlot,
mọi giá trị i64/8-byte aligned). Ba phần: (1) `EnumLayout` — kích thước + alignment
của tagged union trên stack, (2) các MIR statements/terminators cần thêm để khởi tạo,
đọc discriminant, và dispatch match, (3) ownership semantics: construct-move,
destructure-copy (bất đối xứng, nhưng đều là behavior hiện có của borrowck).

---

## §1 — EnumLayout (Implementer)

### 1.1. Kích thước & alignment

Bậc A constraint: mọi giá trị là i64, alignment 8 byte. Enum áp dụng cùng nguyên tắc:

```
┌──────────────────┬──────────────────────────────────────────┐
│ discriminant     │ payload (union của các variant payload)  │
│ i64 (8 bytes)    │ max_payload_size bytes                   │
│ offset 0         │ offset 8                                 │
└──────────────────┴──────────────────────────────────────────┘
```

- **discriminant:** `i64` tại offset 0, alignment 8.
- **payload:** tại offset 8 (đã 8-byte aligned). Kích thước = max của tất cả variant payload.
  Với variant không payload (unit variant), payload area không được dùng —
  nhưng vẫn chiếm chỗ trong layout (sized-by-max).
- **total_size:** `8 + max_payload_size`, làm tròn lên alignment 8 (thường đã aligned sẵn
  vì max_payload_size cũng là bội của 8 trong Bậc A).
- **alignment:** 8.

**Ví dụ:** `enum Option<T> { Some(T), None }` với `T: Integer` (payload = 8 bytes):
total_size = 8 + 8 = 16 bytes.

**Ví dụ:** `enum Color { Red, Green, Blue }` (toàn unit variants, không payload):
max_payload_size = 0 → total_size = 8 + 0 = 8 bytes (chỉ chứa discriminant).

### 1.2. Discriminant encoding

| Variant | Discriminant (i64) |
|---------|-------------------|
| Thứ nhất | 0 |
| Thứ hai | 1 |
| Thứ n | n-1 |

Đánh số từ 0 theo thứ tự khai báo trong source. Đây là mapping **deterministic**
— cùng source enum → cùng discriminant trên mọi platform (deterministic compilation
per ADR-0012 reproducibility).

**Không dùng Trit encoding cho user-defined enum.** `T?` và `T~E` dùng trit discriminator
vì chúng là built-in sum types với ≤ 3 states. User-defined enum có n variants tùy ý
(n ≥ 2, có thể > 3) → cần integer discriminant. Dùng i64 giữ uniform representation
với Bậc A "everything i64".

**Giá trị discriminant trong MIR:** `ConstValue::Integer(i128)` hoặc literal i64.
JIT sẽ `stack_store(I64, slot, 0)` để ghi tag.

### 1.3. `EnumLayout` struct (đề xuất cho `triet-mir`)

```rust
/// Layout of a user-defined enum on the stack (tagged union).
pub struct EnumLayout {
    /// Enum name (e.g., "Option").
    pub name: String,
    /// Byte offset of the discriminant field (always 0 for Bậc A).
    pub discriminant_offset: usize,
    /// Size of the discriminant in bytes (always 8 for Bậc A — i64).
    pub discriminant_size: usize,
    /// Byte offset of the payload union (always 8 for Bậc A).
    pub payload_offset: usize,
    /// Total size in bytes, rounded up to `alignment`.
    pub total_size: usize,
    /// Required alignment (always 8 for Bậc A).
    pub alignment: usize,
    /// Per-variant metadata.
    pub variants: Vec<VariantLayout>,
}

/// Metadata for one enum variant.
pub struct VariantLayout {
    /// Variant name.
    pub name: String,
    /// Integer discriminant value (0, 1, 2, …).
    pub discriminant_value: i64,
    /// Payload layout, if the variant carries data.
    pub payload: Option<PayloadLayout>,
}

/// Layout of a variant's payload.
pub struct PayloadLayout {
    /// Size of this variant's payload in bytes.
    pub size: usize,
    /// Alignment of this variant's payload.
    pub alignment: usize,
    /// If the payload is a struct/tuple, field layouts keyed by name.
    pub fields: Vec<FieldLayout>,
}
```

### 1.4. Tương tác với StructLayout

Enum payload có thể là struct (named fields) hoặc tuple (positional fields) hoặc
scalar đơn. `PayloadLayout.fields` dùng chung `FieldLayout` với `StructLayout` —
cùng cơ chế offset-within-payload, cùng alignment 8 trong Bậc A.

`EnumLayout` và `StructLayout` là **riêng biệt** — enum có discriminant + union payload,
struct có fixed fields. Không kế thừa, không composite pattern. Lý do: borrowck cần
phân biệt enum projection (Downcast→Field) với struct projection (Field trực tiếp).

---

## §2 — MIR Statements & Terminators (Architecture)

### 2.1. Construction pipeline

Để tạo `let a = Enum::Variant(x)`:

```
EnumAlloc(dest: _e, enum_name: "Option")
    → allocate StackSlot kích thước EnumLayout.total_size
SetDiscriminant(dest: _e, value: 0i64)
    → ghi discriminant vào offset 0
Assign(dest: _e.Payload("Some"), source: Place::local(x))
    → copy/move payload vào offset 8 của slot
```

#### `Statement::EnumAlloc`

```rust
EnumAlloc {
    /// Destination local.
    dest: Local,
    /// Enum name — key into `Body::enum_layouts`.
    enum_name: String,
    /// Source location.
    span: Span,
}
```

Tương tự `StructAlloc` — chỉ allocate stack space, không ghi dữ liệu. JIT tạo
`StackSlot` với `total_size` và `alignment` từ `EnumLayout`.

#### `Statement::SetDiscriminant`

```rust
SetDiscriminant {
    /// The enum local to write into.
    dest: Local,
    /// Integer discriminant value (0, 1, 2, …).
    value: i64,
    /// Source location.
    span: Span,
}
```

Ghi `value` vào `enum_slot + discriminant_offset` (offset 0) dưới dạng i64.
Tách riêng khỏi `Assign` vì:
- Discriminant không phải là "place" thông thường — không có địa chỉ độc lập,
  không borrow được.
- `SetDiscriminant` là unconditional write, không đọc source.
- Borrowck không cần track loan trên discriminant (tag không borrow được).

**Tại sao không dùng `Assign` + `Projection::Discriminant`?** Vì `Projection`
được dùng để *theo dõi borrow* ở cấp field. Discriminant không bao giờ được
borrow riêng — nó là metadata của enum, không phải user-accessible field.
Dùng statement riêng giữ borrowck đơn giản.

#### Payload assignment

Dùng `Statement::Assign` hiện có, với source là `Place::local(x)` và dest là
`Place::local(_e)` với projection chain `[Payload("Some")]` (với variant đơn)
hoặc `[Payload("Some"), Field("value")]` (với variant có named struct payload).

`Projection::Payload(String)` mới — xem §2.3.

### 2.2. Match dispatch

Để match `match a { Variant1(x) => bb1, Variant2 => bb2, ... }`:

```
// Đọc discriminant
_disc = discriminant của _a  (đọc từ offset 0 của slot)
// Switch n-way
SwitchInt(discriminant: _disc, cases: [(0, bb_variant1), (1, bb_variant2)], default_bb: bb_trap)
```

#### Terminator mới: `SwitchInt`

```rust
SwitchInt {
    /// Local holding the integer discriminant to branch on.
    discriminant: Local,
    /// (discriminant_value, target_block) pairs.
    cases: Vec<(i64, BasicBlock)>,
    /// Default/fallthrough block for unknown discriminant values.
    /// **Bậc A: always a Cranelift trap block, never Unreachable.**
    default_bb: BasicBlock,
    /// Source location.
    span: Span,
}
```

Khác với `If` terminator (branch trên Trilean condition với +/0/-):
- `SwitchInt` branch trên integer i64 với n targets.
- Không có zero_bb/negative_bb semantic của `If`.
- `default_bb` bắt mọi discriminant không khớp cases.

**Tại sao không reuse `If` terminator?** `If` semantic là Ł3-aware 3-way branch
trên trit (True/Unknown/False). Enum match là n-way branch trên integer
discriminant — khác biệt về kiểu dữ liệu và số lượng target. Tách riêng
giữ IR rõ ràng, borrowck biết chính xác semantic của từng terminator.

#### `default_bb` = Cranelift trap (không Unreachable)

**Quyết định:** `default_bb` luôn là một basic block kết thúc bằng Cranelift
`trap` instruction, **không bao giờ** dùng `Terminator::Unreachable`.

**Lý do:** Typecheck hiện tại chỉ có exhaustiveness check cho Outcome (E1026,
`check_outcome_exhaustiveness`). Không có check exhaustive cho user-defined
enum. Nếu `default_bb = Unreachable` và match thiếu arm → `br_table` rơi vào
unreachable → **undefined behavior** (Cranelift unreachable = UB).

**Hệ quả:**
1. **Non-exhaustive match = runtime error**, không phải compile-time error.
   Đây là khoảng hở semantic — yếu hơn Rust (Rust bắt exhaustive match tại
   compile-time). Ghi nhận trung thực.
2. **TODO:** Enum-exhaustiveness checker (or-pattern + guard + wildcard) là
   task riêng, không nằm trong Phase 4. Khi có exhaustiveness, đổi `default_bb`
   từ trap sang `Unreachable` (và optimization passes có thể cắt dead trap block).
3. **Borrowck:** trap block là reachable, rỗng (không chứa statement hay access
   nào) → vô hại với borrowck.
4. **JIT:** dead-trap cho match đã exhaustive (discriminant luôn khớp cases)
   → vẫn tồn tại trong codegen nhưng không bao giờ thực thi. Chấp nhận được
   cho Bậc A.

#### Cần `Statement` để đọc discriminant

Cần một statement để load discriminant vào một local trước khi SwitchInt.
Hiện tại, `OutcomeDiscriminant` làm việc này cho Outcome (đọc trit tag). Đề xuất
một statement tổng quát hơn:

```rust
GetDiscriminant {
    /// Destination local for the discriminant value (i64).
    dest: Local,
    /// The enum local to read from.
    source: Local,
    /// Source location.
    span: Span,
}
```

JIT: `stack_load(I64, enum_slot, offset=0)` → ghi vào `dest`.

**Borrowck:** `source` được tính là một **use** của enum local (đọc discriminant).
Nếu `source` đã Moved → **E2420 UseAfterMove**. Tương tự `OutcomeDiscriminant`
trong `liveness.rs` hiện tại — `GetDiscriminant` không move enum, chỉ đọc tag.

### 2.3. Projection mới: `Payload`

Để truy cập payload của một variant cụ thể sau khi đã chứng minh enum đang ở
variant đó (qua `SwitchInt`):

```rust
enum Projection {
    // ... existing variants ...
    /// Access the payload of a specific enum variant.
    /// Only valid after the borrowck has proven the enum is in this variant
    /// (via the SwitchInt branch).
    Payload(String),  // variant name
}
```

Dùng kết hợp với `Field` để truy cập field của struct payload:

```
// Truy cập field "value" của variant "Some"
place = Place::local(_e)
    .project(Payload("Some"))
    .project(Field("value"))
```

**Borrowck với `Payload` projection:** `Payload("Some")` là refinement type —
borrowck biết (từ SwitchInt branch) rằng discriminant == 0 (Some). Trong
branch này, access đến `Payload("Some")` là valid. Nếu access `Payload("None")`
(unit variant) trong branch Some → lỗi type (payload không tồn tại, bị typecheck
bắt trước borrowck).

**`places_conflict` cho `Payload` — DEFER Bậc B/C.** Ở Bậc A:
- Không có by-reference bind trong match (§3.4) → không có loan nào trên
  `Payload` projection.
- Destructure dùng copy (§3.2) → không có Partial-Moved tracking.
- → `places_conflict(Payload(..))` không bao giờ được gọi ở Bậc A.

Implement `places_conflict` cho `Payload` bây giờ là dead code. Khi Bậc B/C
thêm by-reference bind vào match, lúc đó mới cần định nghĩa conflict rule
cho Payload projection (hai variant khác tên = disjoint trong type-level,
nhưng alias bộ nhớ ở offset 8 — conflict rule phải dựa trên variant refinement
đã chứng minh, không phải offset). **Ghi chú ở đây, không implement.**

---

## §3 — Ownership Semantics (Author)

### 3.1. Construction: move vào

```
let x = 42;
let a = Option::Some(x);  // x bị move vào a
```

- `x` được **moved** (consumed) vào payload của `a`.
- Sau dòng 2, `x` không còn valid — compile error nếu dùng lại.
- `a` sở hữu toàn bộ enum value, bao gồm cả discriminant và payload.

**MIR:** `Assign { dest: _a.Payload("Some"), source: _x }`. Source là plain local
(không projection) → `is_field_read = false` → borrowck mark `_x` là `Moved`
(`checker.rs:543`). Đây là behavior hiện có — enum payload hoạt động giống
struct field construct.

> **Known divergence (pre-existing, không phải ADR-0037):** `Integer` là Copy type
> theo SPEC §10.1 row 1 ("Stack primitives … Copy by value, no aliasing"),
> nhưng borrowck mark-Moved mọi plain-source mà không type-aware. `Some(x)` với
> `x: Integer` lẽ ra nên copy — gap này có sẵn từ trước Phase 4, cần borrowck
> biết Copy-ness của type để fix. **Defer Bậc B/C.** Không sửa trong Phase 4.

### 3.2. Destructuring: copy ra

```
match a {
    Option::Some(y) => {
        // y là by-value COPY của payload
        consume(y);
    }
    Option::None => {
        // không có payload
    }
}
// a vẫn valid sau match — payload được copy, không move
```

- `y` bind **by-value copy** payload của `a`.
- Sau arm, `a` **vẫn valid** — payload chưa bị move ra.
- `_a` có thể dùng lại sau match (miễn là không bị move toàn bộ bởi
  một arm capture `a` không qua projection).

**MIR:** `Assign { dest: _y, source: _a.Payload("Some") }`. Source có projection
→ `is_field_read = true` → borrowck **KHÔNG** mark `_a` là Moved
(`checker.rs:515-517`). Đây là behavior hiện có cho struct field read —
enum payload copy cũng hoạt động tương tự.

**Tại sao copy, không move?**
1. SPEC §10.1 row 1: stack primitives (Integer/Trit/Tryte/Long/Trilean/Unit)
   là "Copy by value, no aliasing". Bậc A mọi payload là i64 = stack primitive
   → copy là **spec-mandated**, không phải chỉ "rẻ hơn".
2. Payload i64 không có destructor, Drop là no-op → move vs copy không
   observable ở runtime Bậc A.
3. Behavior hiện có của borrowck (`is_field_read` cho projected source) đã
   là copy — không cần code mới.

### 3.3. Không Partial-Moved trong Bậc A

Vì destructure dùng copy, enum không bao giờ ở trạng thái Partial-Moved.
Borrowck không cần track per-projection-path Moved state cho enum.

- Sau match, `_a` vẫn Owned.
- Không có double-move risk (vì không có move-out).
- Drop cho `_a`: discriminant + toàn bộ payload area vẫn valid (payload
  được copy, không bị move).

Move-out semantics (destructure thực sự chuyển ownership payload ra khỏi enum,
làm enum Partial-Moved, cấm dùng lại payload đã move) → **defer Bậc B/C** khi
heap payload (String/Vector/HashMap) làm move quan-sát-được và có destructor
thực sự.

### 3.4. Bất đối xứng construct-move / destructure-copy

| | Construct (`Some(x)`) | Destructure (`Some(y) =>`) |
|---|---|---|
| Behavior | **MOVE** source vào payload | **COPY** payload ra bind |
| MIR source | Plain local | Projected (`_a.Payload("Some")`) |
| Borrowck | Mark source Moved (`checker.rs:543`) | is_field_read → không mark (`checker.rs:517`) |
| Code mới | 0 | 0 |

Bất đối xứng nhưng nhất quán với behavior hiện có của borrowck. Cả hai đều
dùng `Statement::Assign` hiện tại, không cần logic borrowck mới. Khi SPEC
§match được làm rõ về move-vs-copy cho pattern bind (và khi Bậc B/C có heap
payload), bất đối xứng này sẽ được giải quyết.

### 3.5. So sánh với borrow-by-reference

Tương lai (Bậc B/C), pattern match có thể hỗ trợ bind-by-reference:

```
match &0 a {
    Option::Some(y) => {  // y: &0 Integer — tham chiếu shared vào payload
        use(y);
    }
}
```

Lúc này `a` không bị move hay copy — `y` là reference tới payload của `a`.
Borrowck tạo loan `{ source: _a.Payload("Some"), dest: _y, form: BorrowReadOnly }`.
Khi `y` còn live, không thể move `a` hoặc mutate `a` (mượn shared).

**Quyết định:** Bậc A chỉ hỗ trợ by-value copy bind trong match. By-reference
bind thêm ở Bậc B/C khi borrowck đã có per-field loan tracking đủ mạnh và
`places_conflict(Payload)` được định nghĩa.

---

## §4 — Lộ trình

| Bước | Nội dung | MIR thay đổi |
|------|----------|-------------|
| 4a | `EnumLayout` + `VariantLayout` trong `triet-mir` | Data structure mới |
| 4b | Lowerer: thu thập enum definitions → `enum_layouts: Vec<EnumLayout>` | `lower_program` |
| 4c | `Statement::EnumAlloc` + `Statement::SetDiscriminant` + `Statement::GetDiscriminant` | 3 statement variants |
| 4d | `Terminator::SwitchInt` + `Terminator::Trap` | 2 terminator variants |
| 4d' | Borrowck + liveness + JIT: `Trap` xử như `Unreachable` (leaf, 0 successor, 0 read), JIT lower thành Cranelift `trap` | plumbing |
| 4e | `Projection::Payload(String)` | 1 projection variant |
| 4f | Lowerer: `Expr::EnumLiteral` → `EnumAlloc` + `SetDiscriminant` + payload `Assign` | AST→MIR |
| 4g | Lowerer: `match` → `GetDiscriminant` + `SwitchInt` + per-arm payload access | AST→MIR |
| 4h | JIT: `EnumAlloc` → StackSlot, `SetDiscriminant`/`GetDiscriminant` → stack access, `SwitchInt` → Cranelift `br_table` + trap default block | JIT codegen |
| 4i | **MIR verifier:** INV coverage cho 5 cấu trúc mới (xem bên dưới) | verifier assertions |

### 4i. MIR verifier invariants

Verifier hiện tại (`triet-mir/src/lib.rs:828`) là **structural** — kiểm block-bounds
(INV-1: mọi block reference tồn tại trong `body.basic_blocks`) và local-bounds
(INV-2: mọi local reference nằm trong `local_decls`). Không có dominator tree,
không có reaching-def analysis, không flow-sensitive.

**Phân loại invariant theo khả năng của verifier hiện tại:**

| Invariant | Loại | Khả thi? | Xử lý |
|-----------|------|-----------|-------|
| `EnumAlloc.dest` có type enum (tra `local_decls[dest].ty`) | structural | ✅ | Assert trong verifier |
| `SetDiscriminant.value` ∈ `[0, n_variants)` | structural | ✅ | Assert trong verifier |
| `GetDiscriminant.source` có type enum | structural | ✅ | Assert trong verifier |
| `SwitchInt`: mọi block trong `cases` + `default_bb` tồn tại | structural (= mở rộng INV-1) | ✅ | Dạy INV-1 duyệt `SwitchInt.cases` + `default_bb` |
| `SwitchInt.default_bb` kết thúc bằng trap, không phải `Unreachable` | structural | ✅ | Assert trong verifier |
| `SetDiscriminant`/`GetDiscriminant`: dest/source đã được `EnumAlloc` | reaching-def, flow-sensitive | ❌ | **Lowerer responsibility** — lowerer sinh MIR đúng; verifier full-dataflow = Bậc C defense-in-depth |
| `Payload(..)`: chỉ xuất hiện trong block dominate bởi case-target tương ứng của `SwitchInt` | dominator tree, flow-sensitive | ❌ | **Lowerer responsibility** — lowerer 4g dựng match → tự sinh Payload đúng block theo cấu trúc nó tạo; verifier dominance = Bậc C |

**Quy tắc Bậc A:** Lowerer chịu trách nhiệm sinh MIR đúng cho Payload placement
và EnumAlloc-before-use. Verifier chỉ check structural invariants (type enum,
range discriminant, block existence, trap default_bb). Dominator analysis +
reaching-def là defense-in-depth cho Bậc C, không nằm trong scope Phase 4.

**Các invariant structural cụ thể (implement trong 4i):**

| # | Invariant | Cơ chế |
|---|-----------|--------|
| 4i-1 | `EnumAlloc.dest`: `local_decls[dest].ty` là enum type (có entry trong `enum_layouts`) | structural: type lookup |
| 4i-2 | `SetDiscriminant.dest`: `local_decls[dest].ty` là enum type | structural: type lookup |
| 4i-3 | `SetDiscriminant.value` ∈ `[0, enum_layout.variants.len())` | structural: range check |
| 4i-4 | `GetDiscriminant.source`: `local_decls[source].ty` là enum type | structural: type lookup |
| 4i-5 | `SwitchInt`: mọi `(_, bb)` trong `cases` + `default_bb` tồn tại trong `body.basic_blocks` | structural: mở rộng INV-1 |
| 4i-6 | `SwitchInt.default_bb` kết thúc bằng `Terminator::Trap` (không phải `Unreachable`) | structural: terminator check |
| 4i-7 | `Payload(name)`: `name` là variant có trong `enum_layout` của base local | structural: variant lookup |

---

## Không làm

- **Trit discriminant cho user enum.** `T?`/`T~E` dùng trit tag vì ≤ 3 states.
  User enum có n bất kỳ → i64 đơn giản, uniform, dễ codegen. Trit packing
  (3 trit = 1 tryte, 9 trit = 3 tryte...) để dành cho Bậc C nếu cần tối ưu
  kích thước.
- **Nested enum flattening / niche optimization.** `Option<Option<Integer>>`
  lãng phí 2 discriminant. Rust dùng niche optimization (Option<&T> = pointer
  với null sentinel). Triết Bậc A **không làm** — uniform layout đơn giản,
  oracle tier. Bậc C có thể thêm.
- **Tag+payload amalgamation cho unit variant.** Enum toàn unit variant
  (như `Color { Red, Green, Blue }`) không cần payload area. Hiện tại vẫn
  allocate 8 byte payload (max_payload_size = 0 → total = 8). OK cho Bậc A.
- **`Payload` làm việc với `Index`/`Deref`.** `Payload` chỉ valid sau
  `Downcast` logic (chứng minh variant). Kết hợp `Payload` với `Index` hoặc
  `Deref` không có use case hiện tại và bị JIT từ chối (giống nested
  projection hiện tại).
- **`places_conflict(Payload)` — defer Bậc B/C.** Ở Bậc A không có loan trên
  Payload (toàn copy + không by-reference bind) → implement giờ là dead code.
- **Enum exhaustiveness checker — defer, TODO riêng.** Hiện tại non-exhaustive
  match = runtime trap. Exhaustiveness check (or-pattern + guard + wildcard)
  là task riêng, không nằm trong Phase 4. Khi có, `default_bb` được đổi từ
  trap sang `Unreachable`.
- **Partial-Moved tracking cho enum — defer Bậc B/C.** Destructure dùng copy
  ở Bậc A → không cần. Khi heap payload có destructor thực sự, move-out
  semantics mới có giá trị.

## Tham chiếu

- [ADR-0034](0034-jit-aggregate-coverage.md) — Bậc A uniform boxing, enum opcode delegate-to-VM (cũ, triet-ir).
- [ADR-0036](0036-typetag-opaque-aggregate.md) — `TypeTag::Opaque` cho aggregate (cũ, triet-ir).
- [ADR-0020](0020-outcome-error-handling.md) — Outcome `T~E`/`T?~E` với trit discriminator (built-in sum types).
- [SPEC.md](../../SPEC.md) §match — exhaustive match semantics, enum variant patterns.
- `spec/schema/triet-schema.yaml` — `Expr::EnumLiteral`, `Type::UserEnum`, `EnumVariant`.
- `spec/plans/phase3-cranelift-backend.md` — Rổ C: enum là Phase 4 scope.
