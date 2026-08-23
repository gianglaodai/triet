# Triết (哲)

> A **balanced-ternary-first** programming language with first-class three-valued logic, deterministic memory management, and a native Cranelift compiler backend.

Triết (Sino-Vietnamese 哲, *"philosophy"*) uses the balanced ternary arithmetic system `{-1, 0, +1}` and three-valued Łukasiewicz Ł3 logic as its foundation. Its design anchors on **internal coherence**—a single unified Ł3 algebra across logic and data absence—combined with a strict, panic-free compiler architecture.

---

## Status — Ground-up Rewrite in Progress (v0.1.0-dev)

> **Engineering Reality:** A full compiler shipped v0.2–v0.10. On **2026-06-04, the old backend was deleted** to rebuild a clean, robust architecture from the ground up (`source → AST → MIR → NLL Borrowck → Cranelift JIT`). The compiler is young and strictly disciplined; we document only what is proven by working tests.

### ✅ What Works Today (End-to-End Native Execution)

- **Balanced-Ternary Arithmetic:** `Trit`, `Tryte`, `Integer` (27-trit), and `Long` (81-trit) with symmetric ranges and range-enforced trap-on-overflow (ADR-0044).
- **Three-Valued Logic (3VL):** `Trilean` with native Łukasiewicz Ł3 and Kleene K3 operators (`&&`, `||`, `!`, `^`, `=>`, `<=>`).
- **Compile-Time Refinement (`Trilean!`):** `if cond` strictly requires `Trilean!` (statically proven $\neq Unknown$), catching unhandled ambiguity at compile time with **`E1033`** (ADR-0021). `if?` provides explicit fallback.
- **Native Nullable `T?`:** 1-trit sentinel representation (`i64::MIN`), Elvis operator `?:`, and exhaustive `match ~+ / ~0` (including `Struct?` and `Enum?` per ADR-0041/0065).
- **Control Flow:** `if`/`else`, `while`, recursion, cross-function calls, and external math shims (`pow`).
- **Aggregates & ABI:** `struct` (flat StackSlot + SRet convention per ADR-0066), `enum` (discriminant switch).
- **Heap Types:** `String`, `Vector<T>`, and `HashMap<K, V>` with move-only semantics and inline drop glue.
- **Flat Heap-in-Struct:** Structs containing heap fields (`String`/`Vector`/`HashMap`) with recursive drop-glue and move tombstones (ADR-0066).
- **NLL Borrow Checker:** Flow-sensitive dataflow borrow checking enforcing use-after-move (E2420), aliasing exclusivity (E2440), and drop-while-borrowed (E2450).
- **Native Output:** Cranelift JIT printing via ABI shims (ADR-0087).

### ⏳ Not Yet Rebuilt / In Progress

- Nested/recursive heap-in-aggregate (`Struct { inner: HasHeap }`, ADR-0066 Lát 2).
- Partial field moves (`let s = p.name`).
- Capability loader runtime (ADR-0016/0017/0018).
- Freestanding / no-std AOT binary compiler.
- Package bundling and linker (`triet-pack` wiring).

---

## Language Highlights & Verified Examples

All code examples below reflect verified behavior currently running in the test harness.

### 1. Reasoning Under Uncertainty with Ł3 Logic
```triet
// Logic under uncertainty — Łukasiewicz Ł3 algebra
function risk_assessment(fever: Trilean, rash: Trilean, vaccinated: Trilean) -> Trilean {
    let symptoms = fever && rash;
    // If symptoms are True but vaccination status is Unknown -> Result is Unknown
    return symptoms && !vaccinated;
}

function main() -> Integer {
    let sensor_a: Trilean = true;
    let sensor_b: Trilean = unknown;
    
    let risk: Trilean = risk_assessment(sensor_a, sensor_b, false);
    
    // `if?` provides safe fallback for unrefined Trilean values
    if? risk {
        return 1; // High risk
    } else {
        return 0; // Low risk or Indeterminate
    }
}
```

### 2. Native Nullable `T?` & Elvis Operator
```triet
function get_discount(age: Integer?) -> Integer {
    // Widening from Integer to Integer? is implicit and zero-cost
    let safe_age: Integer = age ?: 0; // Elvis operator: unwrap or fallback to 0
    
    if safe_age >= 65 {
        return 20;
    }
    return 0;
}

function match_discount(age: Integer?) -> Integer {
    // Exhaustive pattern matching on Nullable
    return match age {
        ~+ val => if val >= 65 { 20 } else { 0 },
        ~0     => 5, // Missing age default
    };
}
```

### 3. Balanced-Ternary Arithmetic & Symmetric Overflow Trap
```triet
function main() -> Integer {
    let a: Integer = 1000000;
    let b: Integer = 2;
    // Arithmetic operations trap immediately on overflow instead of silent wrapping
    return a * b;
}
```

---

## Pipeline Architecture

```text
.tri Source
    │
    ├──► triet-lexer       Tokens (logos-based lexer)
    ├──► triet-parser      AST (recursive descent + Pratt expressions)
    ├──► triet-modules     Module loader & explicit name resolution
    ├──► triet-typecheck   Type checking, Ł3 lattice & refinement inference
    │
    ├──► triet-lower       AST → Flat MIR Lowering (panic-free Result)
    ├──► triet-mir         Flat non-nested Control Flow Graph (CFG) + Verifier
    ├──► triet-borrowck    NLL dataflow borrow checker (Affine moves & lifetimes)
    │
    ├──► triet-jit         Cranelift native machine code generator
    └──► triet-driver      Unified CLI pipeline runner (check / run)
```

---

## Workspace Layout

```text
triet/
├── crates/
│   ├── triet-core/        # Trit, Tryte, Integer, Long & balanced-ternary arithmetic
│   ├── triet-logic/       # Trilean, Łukasiewicz Ł3 & Kleene K3 algebra
│   ├── triet-syntax/      # AST definitions & schema-generated AST arena
│   ├── triet-lexer/       # Tokenizer & lexing pipeline
│   ├── triet-parser/      # Recursive descent parser
│   ├── triet-modules/     # Hierarchical module resolution
│   ├── triet-typecheck/   # Type checker & refinement inference
│   ├── triet-mir/         # Flat MIR representation & verifier
│   ├── triet-lower/       # AST-to-MIR lowering engine
│   ├── triet-borrowck/    # Non-Lexical Lifetime (NLL) borrow checker
│   ├── triet-jit/         # Cranelift native code generator
│   ├── triet-driver/      # Pipeline CLI driver (`triet-driver`)
│   └── triet-pack/        # Package bundling & linker
├── spec/                  # Formal language schema & phase plans
├── docs/
│   ├── decisions/         # Architecture Decision Records (ADRs)
│   ├── proposals/         # Pre-ADR design explorations & RFCs
│   ├── HIGHLIGHTS.md      # Verified language highlights & fixtures
│   └── ARCHIVE.md         # History of the deleted v0.2–v0.10 compiler
├── SPEC.md                # Language specification & semantics
└── VISION.md              # Long-term design north star & invariants
```

---

## Quick Start

### Prerequisites
- Stable Rust toolchain via `rustup`.

### Build & Run
```bash
# Build the workspace
cargo build --release

# Run the test suite
cargo test --workspace

# Execute a program via the Cranelift JIT driver
./target/release/triet-driver run examples/hello_jit.tri
```

---

## Design Principles

1. **Explicit > Implicit:** Glob imports, default-public exports, and ambient capabilities are rejected.
2. **Refuse over Guess:** If the compiler is not 100% certain, it emits a clear diagnostic rather than guessing silently.
3. **Coherence over Novelty:** A single Ł3 algebra runs consistently across logic, data absence, and error handling.
4. **Stability over Speed:** Every architectural decision is recorded in an ADR with explicit verification gates.

---

## Documentation

- [`SPEC.md`](SPEC.md) — Authoritative language specification and semantics.
- [`VISION.md`](VISION.md) — Long-term design philosophy, constraints, and invariants.
- [`docs/HIGHLIGHTS.md`](docs/HIGHLIGHTS.md) — Detailed language highlights backed by working fixtures.
- [`docs/decisions/`](docs/decisions/) — Architectural Decision Records (ADRs).
- [`docs/proposals/`](docs/proposals/) — Pre-ADR design explorations and RFCs.

---

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
