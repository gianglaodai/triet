# ADR-0061 — Trait System (Tier 1): static dispatch qua method-syntax + mangled name

- **Status:** 🔒 LOCKED — O+G đã ký.
- **Date:** 2026-06-15
- **Khởi thảo:** Mentor O (khảo sát-trước-khi-gõ: 3 mặt trận typecheck/MIR/JIT, tách 3 Tier dispatch, bắt mìn ADR-0039-không-thuộc-trait).
- **Chữ ký:** O ✅ · G ✅ (G duyệt blueprint + 5 quyết định 2026-06-15).
- **Tu chỉnh hậu-khóa (2026-06-15, author chỉ đạo):** áp `feedback_no_abbreviations` toàn schema — keyword `implement` (KHÔNG `impl`, hợp ADR-0005 verbose-keyword); tên đầy đủ `MethodSignature`/`TraitDefinition`/`ImplementationDefinition`, field `parameters`/`type_parameters`. `trait` giữ (là từ đầy đủ). Tên Rust nội bộ `Expr`/`Stmt`/`ExprId` GIỮ (không phải identifier Triết).
- **Liên quan:** [phase13-trait-system.md](../../spec/plans/phase13-trait-system.md) (kế hoạch thi công) · [ADR-0038](0038-comparable-trait-deferred.md) (`Comparable`/`compare()->Trit` — khách hàng đầu tiên) · [ADR-0039](0039-nullable-operator-family.md) (`?+>` family — **TÁCH KHỎI** scope này) · [ADR-0037](0037-enum-tagged-union-layout.md) (i64 discriminant) · [ADR-0050](0050-mir-type-enum.md) (MirType).

---

## 1. Context

Author chốt 2026-06-05 (ADR-0038): **Triết chắc chắn có Trait, không Interface.** `Comparable`
(`compare() -> Trit`) là khách hàng đầu tiên, design-lock chờ Trait system land. Đây là ADR mở
cỗ xe Trait ở mức tối thiểu cần thiết.

**Khảo sát hạ tầng (Mentor O 2026-06-15, đọc code thật):**

| Tầng | Sự thật | file:line |
|---|---|---|
| Lexer/Parser/Schema | **Không** keyword `trait`/`implement`, không AST node. Đất trống. | grep parser/lexer; `triet-schema.yaml` chỉ có `TypeParameter.bound` |
| `Type` enum | 17 variant, **không** Trait. UserStruct/UserEnum lưu inline. | `triet-typecheck/src/types.rs:13-117` |
| Method dispatch | **Bảng cứng** `builtin_method_type()`; không có `implement Type{}` của user. | `triet-typecheck/src/check/methods.rs:12-79` |
| Generic bounds | Chỉ `GenericBound::Send`; không trait bound. | `triet-syntax/src/item.rs:9-23`; `check.rs:991-1003` |
| Resolution annotation | `EnumVariantResolution` ghi lên `ExprId/PatternId`, lowerer tiêu thụ. **Khuôn mẫu** cho method-resolution. | `triet-syntax/src/lib.rs:36-51` |
| MIR call | `CallDispatch{callee_name:String, target:CallTarget{Jit\|Shim}}`. Không mono, không func-ptr, không mangling. | `triet-mir/src/lib.rs:777-800` |
| StructLayout/EnumLayout | Chỉ memory layout. Không method/vtable. | `triet-mir/src/lib.rs:1042`, `1082` |
| JIT | **Chỉ** direct `ins().call(func_ref)`. `call_indirect` = 0 occurrence. | `mir_lower.rs:1583/1596/1624`; grep verified `NONE` |
| Lambda | **Chưa lower được** ("NO Lambda handling in lower"). | `triet-lower/src/lib.rs` (Lambda → `unsupported_expr`) |

Móng hiện tại: resolve call bằng **tên string tĩnh → direct call**. Dispatch tĩnh gần như miễn
phí; dispatch động phải xây từ số 0.

## 2. Decision — Tier 1 Static Dispatch

Tách 3 Tier theo chi phí thật. **Làm Tier 1, phong ấn Tier 2 + Tier 3.**

| Tier | Cần | Chi phí hạ tầng mới | Consumer |
|---|---|---|---|
| **Tier 1 — Concrete dispatch** | `implement T for ConcreteType`; gọi method trên kiểu cụ thể → resolve → tên hàm impl → `CallDispatch::Jit`. | **~0** (direct call sẵn). | **ADR-0038 cần đúng cái này** |
| **Tier 2 — Generic mono** | `function f<T: Trait>(...)` → sinh bản chuyên biệt + mangling + mono collection pass. | Pass mới (MIR chưa có generic-arg/mangling: `FunctionId(0)`). | Khi có stdlib generic |
| **Tier 3 — Trait object / dynamic** | `dyn Trait` → vtable + `call_indirect` + runtime func-addr embed. | ~200-400 LOC mới. | **Không ADR nào cần** |

**G chốt (2026-06-15):** Tier 1 đủ; Tier 2 + Tier 3 **đóng băng YAGNI tuyệt đối** — mở khi có
consumer thật.

### 2.1 Trait/Impl AST — schema-first (KHÔNG ngoại lệ)

Thêm vào `spec/schema/triet-schema.yaml` **trước khi gõ Rust** (G: "tự ý thọc tay viết AST node
không qua schema → băm nát code"):

- `Item::Trait { name, type_parameters, methods: Vec<MethodSignature> }` — `MethodSignature` = chữ ký không thân
  (name, params, return_type).
- `Item::Implementation { trait_name, for_type: TypeExpr, methods: Vec<FunctionDefinition> }`.

Trình tự bất biến: **Schema → Codegen → Parser → Typecheck → Lower.**

### 2.2 Lưu trữ — KHÔNG nhét vào `Type` enum

Trait/impl là **quan hệ**, không phải "kiểu của một giá trị". Hai registry mới trong typecheck,
song song `name_table` (không sửa `Type::UserStruct/UserEnum`):

```
trait_table: HashMap<String, TraitDefinition>                  // tên trait → chữ ký method
impl_table:  HashMap<(TypeName, TraitName), ImplInfo>    // (Integer, Comparable) → { compare → "Integer$Comparable$compare" }
```

`ImplInfo` map method-name → **tên hàm mangled cụ thể**. Mỗi method trong `implement` lower thành một
`Body` bình thường với tên mangled — không có gì đặc biệt ở MIR/JIT.

**Bind impl vào UserStruct/UserEnum:** khi typecheck thấy `implement Comparable for Point`, resolve
`Point` qua `name_table` (đã có), kiểm tra method khớp chữ ký trait, ghi vào `impl_table`.
**Coherence tối thiểu:** cấm 2 impl trùng `(Type, Trait)` → E-code mới (đề xuất E1043).

### 2.3 Dispatch — method-syntax `a.compare(b)` (G chốt)

**G chốt:** trait method gọi bằng **method-syntax `a.compare(b)`**, KHÔNG free-function. Lý do
(G): "ra dáng ngôn ngữ có tổ chức thay vì C-style hầm bà lằng".

Luồng:
1. Typecheck `check_method_call(receiver, method, args)` (`check/methods.rs`) — sau khi thử
   `builtin_method_type()` thất bại, **lội vào `impl_table`**: tra `(receiver_type_name, *)` tìm
   trait có method tên đó, verify arity + kiểu args khớp `MethodSignature`.
2. Ghi `MethodResolution { concrete_fn: "Integer$Comparable$compare" }` lên `ExprId`
   (khuôn `EnumVariantResolution`, `triet-syntax/src/lib.rs:36-51`), trả `return_type` của method.
3. Lower tiêu thụ annotation → `CallDispatch { callee_name: "Integer$Comparable$compare",
   target: CallTarget::Jit, args: [receiver, ...args] }`. Receiver thành arg đầu.
4. JIT: **không cơ chế mới** — direct call sẵn sàng (`mir_lower.rs:1583-1624`).

### 2.4 Mangling — `Type$Trait$method` (là Răng Cưa, không phải bug)

**G chốt:** lộ tên mangled `Integer$Comparable$compare` trong MIR Display là **tính năng**. Test +
MIR verifier assert được typecheck resolve đúng impl hay chưa. Cứ phơi ra ở MIR.

### 2.5 ADR-0038 (`Comparable`) — khách hàng Tier 1 + một việc phụ thật

- `trait Comparable { compare(other) -> Trit }` + `implement Comparable for Integer/String/Tryte`.
- Dispatch qua §2.3.
- **Việc phụ riêng (không thuộc dispatch):** ADR-0038 §note ghi `match compare(a,b) { -1_trit =>
  …, 0_trit => …, 1_trit => … }` là **match trên Trit literal — đường lowering KHÁC enum
  SwitchInt** (`enum_layouts`). Cần thêm **match-on-Trit path** nhỏ. Tách thành sub-task trong
  plan, không gộp vào dispatch.

## 3. Không làm (phong ấn)

- **Tier 2 (Generic monomorphization)** — đóng băng. `function f<T: Comparable>(v)` chưa có
  consumer; MIR chưa có generic-arg/mangling. Mở khi stdlib generic land.
- **Tier 3 (Trait object / vtable / dynamic dispatch)** — đóng băng YAGNI. Cần `call_indirect`
  (0 occurrence hiện tại) + runtime func-addr embed (~200-400 LOC). Không ADR nào cần.
- **ADR-0039 (`?+>` nullable-operator family)** — **TÁCH KHỎI Trait System** (G chốt). Đó là
  desugar operator trên `T?`, không dispatch đa hình. Bị chặn thật ở **Lambda/closure lowering**
  (verified "NO Lambda handling in lower"), không phải ở trait. Thuộc phase riêng "Closure/Lambda
  Lowering". Gộp vào trait = đẻ cơ chế quái thai không giải gốc.
- **Default method body trong trait** — chưa cam kết (Tier 1 yêu cầu impl khai đủ). Defer.

## 4. Hệ quả

- Tái dùng đúng đường `EnumVariantResolution` + `CallDispatch::Jit` đã chạy → ít nợ, không đụng
  value-model i64, không đụng ABI, không thêm cơ chế JIT.
- Hợp "stability over speed": không xây vtable cho nhu cầu chưa tồn tại (tránh skeleton mục như
  bài học `enum_layouts` dead-field).
- Hợp bản sắc tam phân: Comparable đi thẳng direct dispatch + `match` trên Trit.

## 5. Tham chiếu

- [ADR-0038](0038-comparable-trait-deferred.md) — `Comparable`/`compare()->Trit` design-lock.
- [ADR-0039](0039-nullable-operator-family.md) — `?+>` family (tách khỏi scope này).
- [ADR-0037](0037-enum-tagged-union-layout.md) — i64 discriminant (lý do dùng Trit, không enum Ordering).
- [ADR-0027](0027-diagnostic-format-standard.md) — format E1043 (coherence conflict).
- [phase13-trait-system.md](../../spec/plans/phase13-trait-system.md) — kế hoạch thi công từng bước.
- Khảo sát file:line: `types.rs:13-117`, `check/methods.rs:12-79`, `syntax/lib.rs:36-51`,
  `mir/lib.rs:777-800`, `mir_lower.rs:1583-1624`.
