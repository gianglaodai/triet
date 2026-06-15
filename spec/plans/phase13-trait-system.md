# Trait System Tier 1 — Blueprint thi công (O soạn, G duyệt 2026-06-15)

**HEAD `81979fc`. Gate (working tree, cleanup chờ commit) 0·0·168·0.**
Hiện thực hóa [ADR-0061](../../docs/decisions/0061-trait-system-tier1-static-dispatch.md):
Tier 1 static dispatch. Khách hàng đầu tiên = `Comparable` ([ADR-0038](../../docs/decisions/0038-comparable-trait-deferred.md)).

## 0. Hiện trạng (O khảo sát — đo, không đoán; file:line)

| Tầng | Trạng thái | Bằng chứng |
|---|---|---|
| **Schema** | 🔴 Không có `Item::Trait`/`Item::Implementation`. Chỉ `TypeParameter.bound`. | `triet-schema.yaml` |
| **Lexer** | 🔴 Không keyword `trait`/`implement`. | grep lexer |
| **Parser** | 🔴 Không parse trait/impl. | grep parser |
| **Type enum** | ✅ UserStruct/UserEnum/Function inline, 17 variant. Không Trait variant (đúng — §2.2 dùng registry riêng). | `types.rs:13-117` |
| **env / name_table** | ✅ `collect_declared_types()` build `name_table: HashMap<String,Type>`. Móng để resolve `for_type`. | `check_resolved.rs:136-227` |
| **Method dispatch** | 🟡 `check_method_call` → `builtin_method_type()` bảng cứng. **Chưa lội impl_table.** | `check/methods.rs:12-79`, `check/exprs.rs:1215-1270` |
| **Resolution annotation** | ✅ `EnumVariantResolution` + `ExprResolutions/PatternResolutions` (khuôn cho `MethodResolution`). | `syntax/lib.rs:36-51` |
| **MIR call** | ✅ `CallDispatch{callee_name, target:Jit}` sẵn. Tier 1 không cần thêm. | `mir/lib.rs:777-800` |
| **JIT** | ✅ direct `ins().call` cho `CallTarget::Jit`. **0 cơ chế mới.** | `mir_lower.rs:1583-1624` |
| **match-on-Trit** | 🔴 chưa có path; enum-match đi `SwitchInt` keyed `enum_layouts`. | (ADR-0038 §note) |

→ Tier 1 = **frontend (schema/parser) + typecheck routing + resolution annotation**; MIR/JIT gần
như free.

## Trình tự bất biến (G): Schema → Codegen → Parser → Typecheck → Lower. KHÔNG tay-viết AST node.

---

## T1 — Schema + Codegen (móng AST)

- **T1.1** Thêm `Item::Trait { name, type_parameters, methods: Vec<MethodSignature> }` + `Item::Implementation
  { trait_name, for_type: TypeExpr, methods: Vec<FunctionDefinition> }` + `MethodSignature` vào
  `triet-schema.yaml`. Ownership annotation đầy đủ.
  → **verify:** `python3 spec/schema/codegen.py --validate` pass.
- **T1.2** Regen: `codegen.py --target rust`. Generated AST có `Item::Trait`/`Item::Implementation`.
  → **verify:** `cargo build` xanh; generated file byte-identical reproducible (acid test E1a).

## T2 — Lexer + Parser

- **T2.1** Lexer: thêm keyword `trait`, `implement`, `for`.
  → **verify:** unit test token.
- **T2.2** Parser: parse `trait Name { method_sig... }` + `implement Trait for Type { function... }`.
  → **verify:** parser unit test dựng AST đúng; **poison** (đổi resolve sai) → test đỏ.

## T3 — Typecheck: registries + coherence

- **T3.1** Build `trait_table` + `impl_table` từ `Item::Trait`/`Item::Implementation` (song song
  `collect_declared_types`). Resolve `for_type` qua `name_table`.
- **T3.2** Verify impl khớp chữ ký trait (method name/arity/param/return khớp `MethodSignature`) → E-code
  nếu lệch.
- **T3.3** Coherence: cấm 2 impl trùng `(Type, Trait)` → **E1043** (đề xuất; format ADR-0027).
  → **verify T3:** negative test mỗi E-code; **poison** (bỏ check coherence) → test trùng-impl phải đỏ.

## T4 — Method dispatch routing (CORE)

- **T4.1** `check_method_call` (`check/methods.rs`): sau khi `builtin_method_type()` miss, **lội
  `impl_table`** tra `(receiver_type_name, method)` → tìm trait có method, verify args, trả
  `return_type`.
- **T4.2** Ghi `MethodResolution { concrete_fn: "Type$Trait$method" }` lên `ExprId` (mở rộng
  `ExprResolutions` hoặc map mới song song). Mangling `Type$Trait$method`.
  → **verify T4:** typecheck test resolve đúng concrete_fn; **poison** (resolve sai impl) → assert
  concrete_fn phải đỏ.

## T5 — Lower: impl method = Body + tiêu thụ annotation

- **T5.1** Lower mỗi `Item::Implementation` method thành `Body` bình thường, tên = mangled
  `Type$Trait$method`.
- **T5.2** Lower method-call: đọc `MethodResolution` → `CallDispatch{ callee_name: concrete_fn,
  target: Jit, args: [receiver, ...args] }` (receiver = arg đầu).
  → **verify T5:** MIR Display lộ `Integer$Comparable$compare` (Răng Cưa — G); fixture check-mode
  pass; **poison** (lower sai tên callee) → MIR-assert/run đỏ.

## T6 — match-on-Trit path (cho `compare`)

- **T6.1** Lower `match compare(a,b) { -1_trit => …, 0_trit => …, 1_trit => … }`: match trên **Trit
  literal**, đường riêng (KHÔNG enum SwitchInt). Branch theo giá trị Trit i64.
  → **verify T6:** fixture run ra đúng nhánh; **poison** (sai mapping Trit→nhánh) → run đỏ.

## T7 — End-to-end Comparable + JIT

- **T7.1** `trait Comparable { compare(other) -> Trit }` + `implement Comparable for Integer` (+ String,
  Tryte nếu kịp). JIT direct call (0 cơ chế mới).
- **T7.2** Fixture endgame: `let c = a.compare(b); match c { … }` → RUN ra giá trị đúng.
  → **verify T7:** `triet-driver run` ra kết quả đúng; gate xanh; teeth: poison impl resolution →
  RUN đỏ.

---

## Teeth contract (persona O — poison-phải-đỏ)

Mọi lát "fix cấu trúc" phải có negative test đỏ khi poison logic cốt tử. Đặc biệt:
- T4: poison resolve → sai impl → assert concrete_fn đỏ.
- T5: poison callee_name → MIR/run đỏ.
- T3.3: poison coherence → trùng-impl lọt = đỏ.
Báo cáo mỗi lát = **raw gate 4 dòng** (build/test/fixtures/clippy) trên cây nộp.

## Phong ấn (ADR-0061 §3)

- Tier 2 (generic mono) · Tier 3 (vtable/dynamic) — **đóng băng YAGNI**, mở khi có consumer.
- ADR-0039 (`?+>`) — **không thuộc phase này** (chặn ở Lambda lowering, phase riêng).
- Default method body — defer.
