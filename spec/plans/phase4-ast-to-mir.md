# Phase 4 — AST → MIR Lowering

**Status:** Partial — scalar + control flow + borrow; aggregates Err (2026-06-04)
**See also:** `spec/plans/REPORT-2026-06-04.md` for current-state summary.

**Dependency note:** Phase numbering ≠ build order. This phase (AST→MIR lowering)
is the prerequisite for Phase 2 (borrowck) and Phase 3 (JIT) — both consume MIR
bodies produced by the lowerer. The lowerer was built first; phase numbers reflect
document order, not dependency order.

**Implementation:** `crates/triet-lower/src/lib.rs` (~650 dòng).
**What works:** let, binaryop (18/19 ops), Pow→shim (`CallDispatch` to `__triet_pow`),
if/else, while, call, borrow (`&0`/`&+`/`&-`), field access (→ Place projection),
return, block, literals. Threads AST `Spanned<T>.span` into MIR. 3 tests.
**What does NOT work:** Struct/enum/String/Vector/HashMap literals — all return
`Err(LowerError::unsupported_*(...))`. `Statement::Drop` never emitted (→ E2450 dead).
**Note:** Plan dự đoán file `ast_lower.rs`; thực tế là `lib.rs` với inline tests.
**Phụ thuộc:** `triet-syntax` (AST), `triet-mir` (MIR), `triet-typecheck` (typed AST)

---

## 1. Kiến trúc

```
.tri source
    │
    ▼ triet-lexer → triet-parser       → AST (triet-syntax)
    ▼ triet-typecheck                  → typed AST
    ▼ triet-lower (MỚI)                → MIR (triet-mir)
    ▼ triet-borrowck                   → borrow-checked MIR
    ▼ triet-jit                        → native code
```

**Nguyên tắc:** `triet-lower` là cầu nối AST→MIR. KHÔNG đụng vào borrow checker, KHÔNG đụng vào type checker. Nhận typed AST, sinh MIR.

---

## 2. Lowering strategy

### 2.1 — Expression flattening

Mỗi biểu thức (Expr) được hạ thành 1 hoặc nhiều MIR statements. Biểu thức lồng nhau được "flatten" bằng cách sinh temporary locals:

```
a + b * c
→
_1 = b * c       (BinaryOp)
_2 = a + _1      (BinaryOp)
result = _2
```

### 2.2 — Control flow

- `if/else`: sinh 3 block (cond, then, else) + merge block
- `while`: sinh 3 block (header, body, exit) + back-edge
- `return`: terminator Return
- Block expression `{ ... }`: sequence of statements

### 2.3 — Local allocation

Mỗi variable binding (`let`) và temporary được gán 1 MIR `Local`. Lowerer duy trì 1 counter để cấp phát Local.

---

## 3. Module structure

```
crates/triet-lower/
├── Cargo.toml
├── src/
│   ├── lib.rs          // entry: pub fn lower_function(...) -> Body
│   └── ast_lower.rs    // core: Expr → MIR, Stmt → MIR, CFG builder
└── tests/
    └── integration.rs  // end-to-end: factorial.tri via full pipeline
```

---

## 4. End-to-end test

```triet
// factorial.tri
function factorial(n: Integer) -> Integer {
    if n <= 1 {
        return 1;
    } else {
        return n * factorial(n - 1);
    };
}
```

Pipeline:
```
parse("factorial.tri") → Program
typecheck(Program) → typed AST
lower(typed AST) → MIR Body
borrow check(Body) → pass
JIT compile(Body) → native code
call factorial(5) → 120
```
