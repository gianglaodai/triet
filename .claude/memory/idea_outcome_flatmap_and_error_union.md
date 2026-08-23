---
name: idea_outcome_flatmap_and_error_union
description: "Giang's Outcome monadic pipeline operators (-+>, -->, ->>) and explicit error union (E1 | E2 | E3) design — to be implemented before Self-Hosting"
metadata: 
  node_type: memory
  type: project
  originSessionId: 02eb3cca-b058-4b22-bbed-4dbc03274003
---

# Outcome Monadic Pipeline Operators (`->>`) & Explicit Error Unions `(E1 | E2 | E3)`

> **Author & Visionary:** Giang Hoàng ([@gianglaodai](https://github.com/gianglaodai)) & Mentor G  
> **Status:** 💡 **PLANNED FOR PRE-SELF-HOSTING CAMPAIGN**  
> **Core Tenet:** Zero Error Swallowing (Không nuốt lỗi) + Pure Algebraic Monadic Pipelines.

---

## 1. The Triplet Pipeline Operators

Triết establishes a clean, mathematically symmetric operator trinity for `T ~ E` (Outcome):

| Operator | Type Signature | Meaning | Description |
|---|---|---|---|
| **`- + >`** | `(T1 ~ E) -+> (T1 -> T2) = T2 ~ E` | **Map Success** | Single-layer value transformation on the positive channel (`+`). |
| **`- - >`** | `(T1 ~ E1) --> (E1 -> E2) = T1 ~ E2` | **Map Error** | Single-layer error transformation on the negative channel (`-`). |
| **`- > >`** | `(T1 ~ E1) ->> (T1 -> T2 ~ E2)` | **FlatMap / Bind** | Monadic bind: unwrap, apply fallible function, and flatten. |

> **Syntactic Pruning:** `->>` is used instead of `-+>>` because `>>` is the universal symbol for monadic bind/flattening in computer science (Haskell `>>=`, Clojure `->>`, Shell `>>`), and error-channel flatmap (`-->>`) is unnecessary due to native `if?` / `?:` fallback constructs.

---

## 2. Error Channel Algebra: Non-Swallowing Error Unions `(E1 | E2 | E3)`

When chaining multiple fallible operations with `->>`:

$$\mathbf{(T_1 \sim E_1) \ ->> \ (T_2 \sim E_2) \ ->> \ (T_3 \sim E_3) \ = \ T_3 \sim (E_1 \ | \ E_2 \ | \ E_3)}$$

### Architectural Invariants:
1. **Zero Error Swallowing:** The compiler never implicitly coerces or drops errors into a generic catch-all (avoiding Java's `catch(Exception)` and Rust's `anyhow::Result`).
2. **Explicit User Mapping:** If a function does not want a wide `(E1 | E2 | E3)` signature, the developer explicitly transforms/collapses errors using `-->`:

```triet
// Staged local mapping:
function compile_staged(path: &String) -> Binary ~ CompilerError {
    return read_file(path)   --> CompilerError::Io
        ->> parse_ast        --> CompilerError::Syntax
        ->> typecheck        --> CompilerError::Type
        ->> generate_code    --> CompilerError::Codegen;
}

// Terminal union mapping with exhaustiveness check:
function compile_terminal(path: &String) -> Binary ~ CompilerError {
    return (read_file(path) ->> parse_ast ->> typecheck)
        --> match {
            ~- e: IoError    => CompilerError::Io(e),
            ~- e: ParseError => CompilerError::Syntax(e),
            ~- e: TypeError  => CompilerError::Type(e),
        };
}
```

---

## 3. Ergonomic Ground State Defaults (To be finalized in Self-Hosting)

- **`&T`** $\equiv$ **`&0 T`**: Neutral downward borrow is the unmarked default ground state ($0$).
- **`T{}`** $\equiv$ **`0T{}`**: Stack placement is the unmarked default ground state ($0$).
- **`+T{}`**: Heap allocation (marked $+1$).
- **`-T{}`**: Static `.rodata` allocation (marked $-1$).
- **`~0`**: Null / Void literal.
