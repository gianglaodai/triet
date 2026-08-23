# Triết (哲)

> **A balanced-ternary-first systems programming language engineered for zero-cost memory safety, algebraic 3-valued logic, and bare-metal performance without the cognitive overhead of lifetime annotations.**

Triết (Sino-Vietnamese 哲, *"philosophy"*) is a deterministic systems language designed on the mathematical elegance of **Balanced Ternary arithmetic `{-1, 0, +1}`** and **Łukasiewicz Ł3 three-valued logic**. Its design provides a unified, coherent algebraic foundation across logic, data absence, memory placement, and capabilities—delivering the uncompromising performance and safety of modern systems languages while eliminating their historic design debts.

---

## 🌟 Core Pillars & Breakthroughs

### 1. First-Class 3-Valued Logic (`Trilean` & `Trilean!`)
Unlike classical languages that bolt on missing data via `Option<bool>` (introducing branch misprediction penalties) or SQL's flawed 3VL (where `NULL` causes silent query bugs):
- **Algebraically Closed Ł3/K3 Logic:** Primitive `Trilean` values (`True (+1)`, `Unknown (0)`, `False (-1)`) evaluate with mathematical consistency.
- **Refinement Subtyping (`Trilean!`):** `if` conditions strictly require `Trilean!` (statically proven $\neq Unknown$), catching unhandled ambiguity at compile time via **`E1033`** (ADR-0021).
- **100% Branchless Machine Codegen:** Lowered to native CPU `min`/`max`/`neg` instructions in Cranelift without branching overhead.

### 2. Lifetimes Without the Syntax Matrix (`&0`, `&+`, `&-`)
Triết eliminates Rust's complex named lifetime annotations (`'a`, `'b`, `where 'a: 'b`) in favor of **Directional Flow Polarities**:
- **`&0 T` (Neutral / Local Sink):** Strictly bound to local scope; guaranteed never to escape or be returned.
- **`&- T` (Negative / Outward Flow):** Flow-connected to caller context; permitted to return or back-reference parent nodes.
- **`&+ T` (Positive / Universal Read):** Shared immutable view of ambient or arena memory.

### 3. Balanced-Ternary Memory Placement (`+T`, `T`, `-T`)
Replaces library allocation wrappers like `Box<T>` with zero-cost constructor and type-level placement polarities:
- **`+T{}` / `+T` (Heap - Dynamic):** Unique owning pointer (8-byte allocation, automatically freed on drop).
- **`T{}` / `T` (Stack - Frame):** Flat inline stack slot with zero allocation latency.
- **`-T{}` / `-T` (Static - Immortal):** Application lifetime in `.rodata`, zero allocation, no-op drop.

### 4. Data-Oriented "Pit of Success" (Banishing `Rc<T>`)
- **90% Hierarchical Domain:** Managed cleanly via top-down `+T` ownership and upward `&-` loans with $O(1)$ linear cleanup.
- **10% Non-Hierarchical Graphs / Cycles:** Handled via **Arena Allocation + NodeID (`u32`)**, forcing optimal CPU L1/L2 cache locality and eradicating cyclical memory leaks by construction.

### 5. Two-Tier `unsafe` Capability Protection
- **Macro Level:** Manifest-level capability declarations in `dao.package` (e.g. `dev::raw_memory`, `dev::ffi`) block third-party supply chain attacks.
- **Micro Level:** Explicit `unsafe { ... }` lexical blocks isolate raw memory operations for instant fault localization during debugging.

---

## 🚦 Status — Clean-Architecture Ground-Up Pipeline (v0.1.0-dev)

> **Honest Engineering Status:** The compiler backend underwent a complete ground-up architectural rewrite starting June 2026. The pipeline is lean, robust, and verification-driven (`source → AST → MIR → NLL Borrowck → Cranelift JIT`).

### ✅ Running End-to-End Today:
- **Balanced-Ternary Arithmetic:** Full symmetric integer range with range-enforced trap-on-overflow (ADR-0044).
- **Łukasiewicz Ł3 / Kleene K3 Logic:** Native operators (`&&`, `||`, `!`, `^`, `=>`, `<=>`) and `Trilean!` type refinement.
- **Control Flow:** `if`/`else`, `if?` fallback, `while`, recursion, cross-function calls, and external math shims.
- **Memory & Aggregates:** `struct` (StackSlot + sret ABI per ADR-0066), `enum` (discriminant switch).
- **Native Nullable `T?`:** 1-trit sentinel (`i64::MIN`), Elvis operator `?:`, and exhaustive `match ~+ / ~0` (ADR-0041/0065).
- **Heap Types:** `String`, `Vector<T>`, `HashMap<K, V>` (move-only semantics with inline drop glue).
- **Flat Heap-in-Struct:** Structs holding heap fields with recursive-walk drop-glue and move tombstones.
- **NLL Borrow Checker:** Flow-sensitive borrow checking catching use-after-move (E2420), aliasing violations (E2440), and drop-while-borrowed (E2450).
- **Native Standard I/O:** Cranelift JIT printing via ABI shims (ADR-0087).

### 🎯 Active Frontiers (On the Roadmap):
- Nested/recursive heap-in-aggregate (`Struct { inner: HasHeap }`, ADR-0066 Lát 2).
- `comptime` metaprogramming and Data-Oriented `SoaVector<T>` collections.
- Binary Package Distribution (`.tripkg` = `.so` + `.trimeta`).
- Pure freestanding/no-std native AOT backend.

---

## 💻 Code Examples

### 1. Reasoning Under Uncertainty with Ł3 Logic
```triet
// Evaluating diagnostic risk with incomplete sensor data
function evaluate_risk(fever: Trilean, rash: Trilean, vaccinated: Trilean) -> Trilean {
    let symptoms = fever && rash;
    // If symptoms are True but vaccination status is Unknown -> Result is Unknown
    return symptoms && !vaccinated;
}

function main() -> Integer {
    let sensor_a: Trilean = true;
    let sensor_b: Trilean = unknown;
    
    // Typecheck enforces refinement: `if` requires `Trilean!`
    let is_safe: Trilean = evaluate_risk(sensor_a, sensor_b, false);
    
    // Using `if?` for explicit fallback on Unknown:
    if? is_safe {
        return 1; // Safe
    } else {
        return 0; // Hazard or Indeterminate
    }
}
```

### 2. Native Nullable `T?` & Pattern Matching
```triet
function compute_discount(age: Integer?) -> Integer {
    // Widening from Integer to Integer? is 100% implicit and zero-cost
    return match age {
        ~+ val => if val >= 65 { 20 } else { 0 },
        ~0     => 5, // Default discount when age is missing
    };
}
```

### 3. Directional Reference Flow (No `<'a>` Lifetimes)
```triet
// The compiler statically knows the return value borrows from `src`, not `query`
function find_keyword(src: &- String, query: &0 String) -> &- String {
    // &0 guarantees `query` cannot escape
    // &- connects `src` directly to caller context
    return src;
}
```

---

## 🏗️ Compiler Architecture & Pipeline

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
    ├──► triet-jit         Cranelift native machine code generator (JIT Tier 1/2)
    └──► triet-driver      Unified CLI pipeline runner (check / run)
```

---

## 📦 Workspace Layout

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
│   ├── proposals/         # Pre-ADR design foundations & RFCs
│   └── HIGHLIGHTS.md      # In-depth language highlights & verified fixtures
├── SPEC.md                # Language specification & semantics
└── VISION.md              # Long-term design north star & invariants
```

---

## 🚀 Getting Started

### Prerequisites
- **Rust Toolchain:** Stable Rust (edition 2021/2024) via `rustup`.

### Build & Run
```bash
# Build the workspace in release mode
cargo build --release

# Run end-to-end tests across all crates
cargo test --workspace

# Execute a Triết program using the native JIT driver
./target/release/triet-driver run examples/hello_jit.tri
```

---

## 📖 Deep References

- [`VISION.md`](VISION.md) — The long-term design philosophy, invariants, and OS-capable constraint.
- [`SPEC.md`](SPEC.md) — Authoritative language specification and semantics.
- [`docs/proposals/post_rust_architecture_foundations.md`](docs/proposals/post_rust_architecture_foundations.md) — In-depth architectural treatise on 3VL, Memory Placement, Reference Polarities, and Two-Tier Safety.
- [`docs/decisions/`](docs/decisions/) — 36+ Architectural Decision Records (ADRs).

---

## 📜 License

Dual-licensed under either:
- **MIT License** ([LICENSE-MIT](LICENSE-MIT))
- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE))
