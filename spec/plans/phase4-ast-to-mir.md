# Phase 4 — AST → MIR Lowering

**Status:** Draft — đang triển khai
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
