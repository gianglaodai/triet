# Architectural Foundations: The Post-Rust Systems Paradigm for Triết

> **Document Status:** Architectural Exploration & Design Foundations (Pre-ADR / RFC).  
> **Author & Visionary:** Giang (Language Creator & Architect) with Mentor G.  
> **Target Audience:** Future ADR authors, compiler implementers, and reviewers.

---

## 1. Executive Summary & Core Thesis

The Triết programming language is designed as a **next-generation, high-performance, deterministic systems language** that achieves the zero-cost memory safety of Rust while fundamentally eliminating its greatest ergonomic and architectural bottlenecks:

1. **No Lifetime Syntax (`'a`, `'b`):** Replaced by **Directional Reference Forms** (`&0`, `&+`, `&-`).
2. **No `Box<T>` / `Rc<T>` Zoo:** Replaced by **Balanced-Ternary Memory Placement** (`+T`, `T`, `-T`) and **Arena + NodeID** architectures ("The Pit of Success").
3. **No 3VL Semantic Bugs:** Replaced by **Refined 3-Valued Logic** (`Trilean` / `Trilean!`), resolving the historic SQL 3VL disaster via compile-time gates (`E1033`).
4. **No Monolithic Recompilation Drag:** Replaced by **Stable Deterministic ABI** and **Precompiled Binary Packages** (`.so` + `.trimeta`).
5. **No Proc-Macro Bloat:** Replaced by **Compile-Time Execution (`comptime`)**.
6. **No Uncontrolled `unsafe`:** Replaced by **Two-Tier Capability Defense** (Manifest-level `dao.package` grants + Lexical source blocks).
7. **No Bitwise Operator Pollution:** Core language remains purely algebraic/ternary; binary shift operations are lowered mathematically or quarantined to target-specific intrinsics (`sys::binary`).
8. **Data-Oriented SIMD Headroom:** Solving SoA (Structure of Arrays) at zero-cost via `comptime` collections (`SoaVector<T>`).

---

## 2. Three-Valued Logic (`Trilean` & `Trilean!`) vs `Boolean?`

### 2.1 The Historical Disaster of SQL 3VL
SQL conflated two fundamentally distinct concepts under `NULL`:
- **Ontological Absence** (Missing record / no value).
- **Epistemic Indeterminacy** (`UNKNOWN` truth state).

This caused silent logic catastrophes in database queries (e.g., `WHERE col NOT IN ('a', 'b', NULL)` returning 0 rows; `WHERE age >= 18 OR age < 18` dropping null records).

### 2.2 Algebraic Soundness under Ł3 / K3
Triết establishes `Trilean` as a **closed algebraic 3-valued logic system** ($\{+1\ \text{True},\ 0\ \text{Unknown},\ -1\ \text{False}\}$):
- `True || Unknown` $\rightarrow$ `True` (Tautological regardless of unknown).
- `False && Unknown` $\rightarrow$ `False` (Falsifiable regardless of unknown).
- `!Unknown` $\rightarrow$ `Unknown`.

### 2.3 Refinement Subtyping (`Trilean!` via ADR-0021)
To prevent silent control-flow divergence:
- `if cond` requires `cond: Trilean!` (statically proven $\neq Unknown$).
- Unrefined `Trilean` in `if` raises compile-time error **`E1033 PossiblyUnknownCondition`**.
- Explicit fallback is supported via `if?` (treating `Unknown` as false/fallback).

```text
Type Lattice:
       Trilean (True, False, Unknown)
          ▲
          │  Widening (Implicit, safe)
          │
       Trilean! (Statically proven ≠ Unknown: {True, False})
          │
          ▼  Narrowing (Strictly Explicit: match, .assume_known("reason"), if?)
```

### 2.4 Why `Trilean` is a Primitive, Not a Library Enum
1. **Return type of fundamental relational operators (`==`, `!=`, `<`, `>`):** The core grammar cannot depend on a secondary user-space enum.
2. **Native algebraic operators (`&&`, `||`, `!`):** Direct compilation without dynamic method dispatch.
3. **Machine-level ABI:** Scalar `i8` register passing, branchless Cranelift/assembly codegen (`min`, `max`, `neg`).

### 2.5 Orthogonality of `Trilean?` (4 States)
`Trilean?` (`Nullable<Trilean>`) represents 4 legitimate, distinct states in data collection lookups (e.g., `HashMap<K, Trilean>.get(k)`):
- `~0` (`Null`): Key not found in map (Absence).
- `~+ Unknown`: Key exists, but sensor/evaluator reports indeterminate truth (Present Unknown).
- `~+ True` / `~+ False`: Key exists, value known.

---

## 3. Balanced-Ternary Memory Placement (`+T`, `T`, `-T`)

Instead of library wrappers like `Box<T>`, Triết expresses memory placement as an intrinsic **Type-Level and Constructor Polarity**:

| Notation | Storage Placement | Lifetime & Semantics | Machine Representation |
| :--- | :--- | :--- | :--- |
| **`+T{}` / `+T`** | **Heap (Dynamic)** | Unique Owning Pointer. Dropped at scope exit with `dealloc`. Replaces `Box<T>`. | 8-byte pointer |
| **`T{}` / `T`** | **Stack (Local Frame)** | Local stack frame slot. Zero allocation latency. 100% deterministic. | Inline flat frame slot |
| **`-T{}` / `-T`** | **Static (Immortal)** | Application lifetime. Stored in `.rodata`/`.data`. 0 allocation, no-op drop. | 8-byte static address |

### 3.1 Invariants & Soundness Requirements
1. **Type-Level Manifestation:** `+T` is a distinct 8-byte pointer type. This enables recursive data structures with statically-known finite struct layouts (e.g., `struct Node { next: +Node? }`).
2. **Drop Glue Differentiation:**
   - `T`: Drops fields, does not call deallocator.
   - `+T`: Drops fields, then calls `__triet_free(ptr, layout.total_size)`.
   - `-T`: Complete no-op (calling `free` on `.rodata` is strictly prevented).
3. **Compile-Time Constant Evaluation for `-T`:** All field expressions in `-T{ ... }` must be evaluated at compile time (`comptime` / const eval).

---

## 4. Eliminating Rust Lifetimes via Directional Reference Forms

Rust's explicit lifetime annotations (`'a`, `'b`, `where 'a: 'b`) are a leaky abstraction resulting from exposing internal Region Calculus variables ($\rho$) to the programmer.

Triết replaces continuous lifetime variables with **Discrete Directional Reference Polarities**:

| Reference Form | Polarity & Intent | Escape Rules | Use Case |
| :--- | :--- | :--- | :--- |
| **`&0 T`** | **Neutral / Local Sink** | **Strictly Forbidden to escape** or be returned. | Parameter reads, local inspections, scoped views. |
| **`&- T`** | **Negative / Outward Flow** | **Allowed to escape into return value** or point upward to parent. | Returning substrings/slices, hierarchical back-links. |
| **`&+ T`** | **Positive / Shared Ambient** | Shared immutable access to arena or global data. | Ambient reads, thread-safe shared memory. |

### 4.1 Comparison on Common Functions
* **Rust (Explicit Lifetime Parameters):**
  ```rust
  fn find_substr<'a, 'b>(src: &'a str, query: &'b str) -> &'a str
  ```
* **Triết (Directional Polarity):**
  ```triet
  function find_substr(src: &- String, query: &0 String) -> &- String
  ```
  *The compiler unambiguously knows the return value borrows from `src` (because `src` is `&-` and `query` is `&0`), with zero generic lifetime noise.*

### 4.2 Struct-Level Lifetimes Eradicated
- `struct ScopedView { text: &0 String }` $\rightarrow$ Inherits `&0` non-escaping constraint automatically.
- `struct Node { parent: &- Node }` $\rightarrow$ Inherits `&-` hierarchical ancestor-loan constraint.

---

## 5. The 90/10 Architecture & "The Pit of Success" (Banishing `Rc`)

### 5.1 The 90% Case: Hierarchical Ownership
90% of real-world domain structures (JSON ASTs, HTML DOM, UI Trees, Business Pipelines) are naturally **Hierarchical Trees**:
- Parent owns child: `child: +Node?` (Top-down ownership).
- Child loans back to parent: `parent: &- Node?` (Upward loan).
- **Result:** 0 memory leaks, 0 reference count overhead, 100% linear deallocation.

### 5.2 The 10% Case: Non-Hierarchical Graphs & Cycles
10% of domain problems (Many-to-Many RBAC, Cyclic State Machines, Reactive Spreadsheets, Network Routing) have **co-dependent, peer-to-peer lifetimes**:
- Triết explicitly **refuses `Rc<T>` / `RefCell<T>`**.
- Triết enforces **Arena Allocation + NodeID (`u32`)**:
  - The entire graph is owned by a single root container (`GraphArena`).
  - Nodes communicate via numeric identifiers (`NodeID: u32`).
- **Benefits ("The Pit of Success"):**
  1. High-Performance Cache Locality (L1/L2 cache saturation vs 200-cycle DRAM stalls from pointer chasing).
  2. Instant Mass Deallocation (`arena.clear()` in $O(1)$ vs recursive tree teardown).
  3. Elimination of cyclical memory leaks by construction.

### 5.3 Local Compile-Time Cycle Prevention
The compiler does not require exponential whole-program graph traversal to reject cyclic value constructions. It catches them via:
1. **Affine Move Uniqueness:** An object cannot be moved into its own descendant while active.
2. **Borrow Invariants (`Aliasing XOR Mutability`):** Cannot mutate a node while it is borrowed.
3. **Type Dependency Resolver (Tarjan's SCC / 3-Color DFS):** Type definition cycles (`struct A { b: B }`, `struct B { a: A }`) are detected in $O(V+E)$ at the declaration pass.

---

## 6. Precompiled Binary Libraries vs Source Recompilation

### 6.1 Why Rust Recompiles from Source
Rust is forced into source-only distribution (`crates.io`) due to:
1. Monomorphization explosion of generic templates.
2. Deliberately unstable ABI (`repr(Rust)` layout scrambling).
3. Cross-crate inlining dependence on LLVM IR.

### 6.2 The Triết Binary Distribution Strategy
Triết achieves fast compilation and modular distribution via:
- **Deterministic Stable ABI:** Fixed scalar sizes, Copy-In (ADR-0066 KCN-1b), and SRet conventions.
- **Binary Packages (`.tripkg`):**
  - Compiled Native Object (`.so` / `.a` / `.dylib`).
  - Lightweight Metadata Interface (`.trimeta` carrying type signatures, struct layouts, and `&0`/`&+`/`&-` flow contracts).
- **Two-Tier Compiler Architecture:**
  - **Dev Mode (Cranelift):** Near-instant 0.5s builds for development iteration.
  - **Release Mode (Full LTO):** Maximum bare-metal optimization for production deployment.

---

## 7. Metaprogramming (`comptime`) & Two-Tier `unsafe`

### 7.1 Compile-Time Execution (`comptime`) over Proc-Macros
To avoid Rust's procedural macro complexity and build degradation:
- Triết replaces proc-macros with **`comptime` execution** (following Zig's proven model).
- Standard Triết functions execute inside the compiler during compilation for reflection, struct field iteration, and type generation.

### 7.2 Two-Tier `unsafe` Architecture
`unsafe` is a physical necessity for MMIO, hardware drivers, and C FFI, but must be strictly governed:

```text
[ TIER 1: MACRO CONTROL (Supply Chain Security in `dao.package`) ]
  capability = ["dev::raw_memory", "dev::ffi"]
  → Blocks unauthorized third-party supply chain attacks at the manifest level.

[ TIER 2: MICRO CONTROL (Lexical Scoping in Source Code) ]
  unsafe {
      // Raw pointer dereferences and FFI calls MUST be lexically enclosed.
  }
  → Preserves mental speedbumps and 5-minute fault localization during SIGSEGV debugging.
```

### 7.3 Milestone Roadmap
- **Phase 1 (Current):** Hardening the Safe Core (`Trilean!`, `+T`/`T`/`-T`, `&0`/`&+`/`&-`, SRet, Arena).
- **Phase 2 (Systems Integration):** Two-Tier `unsafe`, `RawPtr`, and C-ABI FFI.
- **Phase 3 (Ecosystem Maturity):** `comptime` metaprogramming and Binary Package Manager (`.tripkg`).

---

## 8. Hardware Purity: Absence of Binary Bitwise Shift Operators

### 8.1 Why Triết Omits `<<` and `>>`
Binary bit-shifting (`<<`, `>>`) implicitly tethers an entire programming language to base-2 radix arithmetic ($x \times 2^k$).
- On future **Balanced Ternary Hardware**, radix shifting operates in base 3 (Trit-shifting: $x \times 3^k$).
- Triết's core language grammar contains **ZERO binary bitwise shift operators**.

### 8.2 Self-Hosting & Low-Level Binary Encodings
When Triết self-hosts its compiler backend for binary targets (x86_64/ARM/Mach-O):
1. **Mathematical Equivalence:** Writing `mod * 64 + reg * 8 + rm` is lowered by the backend optimizer into hardware `shl` instructions with zero runtime penalty.
2. **Target-Specific Intrinsics (`sys::binary`):** Hardware bit manipulation is isolated to `sys::binary::{shl, shr, bit_and, bit_or}` without polluting the language syntax.

---

## 9. Performance Headroom: Why Triết Can Outperform Rust

Triết is not bounded by Rust's performance ceiling. It gains distinct microarchitectural advantages in three key areas:

1. **Branchless 3-Valued Logic:**
   - Rust uses `Option<bool>` (requiring 2 conditional branch jumps `cmp/jne` with branch misprediction penalties).
   - Triết lowers `Trilean` Ł3/K3 operations (`&&`, `||`, `!`) to **100% Branchless Instructions** (`minsb`, `maxsb`, `cmov`, `neg`), yielding 2x-5x faster logic evaluation.
2. **Zero Unwinding Bloat:**
   - Triết employs a **Trap-on-Fault / Fail-Closed** model, completely eliminating the thousands of landing pad cleanup blocks that pollute the CPU L1 Instruction Cache (I-Cache) in Rust binaries.
3. **Data-Oriented Cache Locality by Default:**
   - Forcing Arena + NodeID eliminates pointer chasing, turning cache-thrashing pointer graphs into contiguous linear memory vectors.

---

## 10. Data-Oriented Array Optimization (The SoA Lesson from Odin)

### 10.1 Odin's `#soa` Innovation
Odin popularized `#soa [N]Struct`, transforming Array-of-Structures (AoS) into Structure-of-Arrays (SoA) in memory to unlock 100% autovectorized AVX-512 SIMD throughput (4x-8x speedup).

### 10.2 Why Rust Cannot Natively Support `#soa`
In Rust, `&T` represents a single contiguous 8-byte pointer in RAM. In an SoA buffer, struct fields are stored in disjoint parallel arrays, making a contiguous `&T` physically impossible without breaking borrow checker and trait dereferencing invariants.

### 10.3 The Triết Architectural Path
Triết will support Structure-of-Arrays via **`comptime` Collections (`SoaVector<T>`)** in Phase 3 (following Zig's `std.MultiArrayList` pattern):
- Preserves 100% SIMD AVX-512 throughput.
- Keeps the core compiler type system sound and free of disjoint proxy reference hacks.
- Fully satisfies the **Zero-Cost Abstraction** principle ("What you don't use, you don't pay for; what you use, you couldn't hand-code any better").
