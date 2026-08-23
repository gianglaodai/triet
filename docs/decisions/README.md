# ADR Index — Chronological

> **Author & Architect:** Giang Hoàng ([@gianglaodai](https://github.com/gianglaodai))  
> Architectural Decision Records for Triết. All decisions authored and approved by the Language Creator & Architect. Each ADR captures one significant choice — *why* this design over alternatives — so future readers (including AI assistants) can reconstruct the reasoning without spelunking through git history.

ADRs are immutable once "Locked" status is reached. To change a prior decision, write a new ADR that supersedes it.

> **Looking for a rule on X?** Use [`by-topic.md`](by-topic.md) — all ADRs grouped by topic cluster (Language Surface, Type System, Ownership & Borrowing, Module/Package, IR/Wire Format, Capability & Sandboxing, Backend & JIT, Collections & Iteration).

---

## By Phase

### Pre-v0.2 — Language Semantics Groundwork

| ADR | Title | Status |
|---|---|---|
| [0001](0001-nullable-memory-layout.md) | Nullable memory layout (`T?` discriminator) | Locked |
| [0002](0002-fstring-format-spec.md) | F-string format spec | Locked |
| [0003](0003-iterator-protocol.md) | Iterator protocol (superseded by ADR-0089) | Superseded |
| [0004](0004-multiline-string-indent.md) | Multi-line string indentation | Locked |

### v0.2.x — Module System

| ADR | Title | Status |
|---|---|---|
| [0005](0005-module-system.md) | Module system (superseded by ADR-0071) | Superseded |
| [0006](0006-ternary-packaging-vision.md) | Ternary packaging vision (informational) | Informational |

### v0.3 — Bytecode VM + Stable IR

| ADR | Title | Status |
|---|---|---|
| [0007](0007-ir-design.md) | IR design (register-based SSA) | Locked |
| [0008](0008-triv-binary-format.md) | `.triv` bytecode binary format (v5) | Locked |

### v0.3.x.cleanup — Gate-Closing Phase

| ADR | Title | Status |
|---|---|---|
| [0009](0009-version-gate-policy.md) | Version gate policy (4-gate matrix) | Locked |

### v0.3.x.ternary — Ternary-Native IR Refactor

| ADR | Title | Status |
|---|---|---|
| [0010](0010-ternary-native-ir.md) | Ternary-native IR (`BrTrilean`, strict `if`, Ł3-aware `Eq`/`Ne`) | Locked |

### v0.4 — Crate-Pack + Stable ABI

| ADR | Title | Status |
|---|---|---|
| [0011](0011-abi-metadata-format.md) | ABI metadata format (BLAKE3, two-level hash) | Locked |
| [0012](0012-witness-table-dispatch.md) | Witness table dispatch (Swift-style generics) | Locked |
| [0013](0013-semver-linking-policy.md) | Semver linking policy (E2300–E2399) | Locked |

### v0.5 — CAS Packaging

| ADR | Title | Status |
|---|---|---|
| [0014](0014-hash-scheme-refinement.md) | Hash scheme refinement (3-level hash tree) | Locked |
| [0015](0015-package-store-layout.md) | Package store layout (`~/.triet/store/`, atomic install, GC) | Locked |

### v0.6 — Capability System

| ADR | Title | Status |
|---|---|---|
| [0016](0016-capability-type-system.md) | Capability type system (namespaces, Trit-level grants, E22XX) | Locked |
| [0017](0017-trilean-policy-hook.md) | Trilean policy hook protocol (`dao.policy` rules) | Locked |
| [0018](0018-capability-loader-semantics.md) | Capability loader semantics (`dao.package` manifest) | Locked |

### v0.7 — Self-Hosting Compiler

| ADR | Title | Status |
|---|---|---|
| [0019](0019-self-hosting-compiler-bootstrap.md) | Self-hosting compiler bootstrap (3-stage chain) | Locked |
| [0020](0020-outcome-error-handling.md) | Outcome error handling (`T~E` / `T?~E`, trit-encoded fallibility) | Locked |
| [0021](0021-trilean-refinement.md) | Compile-time `Trilean!` refinement for strict `if` (E1033) | Locked |
| [0023](0023-lowerer-ssa-struct-tracking.md) | Lowerer SSA struct-tracking (`ValueKind` enum) | Locked |
| [0024](0024-khi-dao-identity-naming.md) | Khí + Đạo identity naming (Đạo Đức Kinh §28/§42) | Locked |

### v0.8 — Ownership Foundation + Concurrency Primitives (BYOS)

| ADR | Title | Status |
|---|---|---|
| [0022](0022-trit-balanced-ownership.md) | Trit-balanced ownership (S6 5-form reference) | Locked |
| [0025](0025-borrow-checker-rules.md) | Borrow checker rules (E24XX namespace) | Locked |
| [0026](0026-actor-boundary-send-rules.md) | Concurrency primitives & Send rules (BYOS, E25XX) | Locked |
| [0027](0027-diagnostic-format-standard.md) | Diagnostic format standard (canonical error blocks) | Locked |

### v0.9 — JIT + Borrow Enforcement + Atomic

| ADR | Title | Status |
|---|---|---|
| [0028](0028-atomic-primitive.md) | Atomic primitive design (`Atomic<T>`, `Ordering`) | Locked |
| [0029](0029-self-host-port-policy.md) | Self-host port policy (3-layer scope) | Locked |
| [0030](0030-jit-cranelift-integration.md) | JIT integration (Cranelift backend) | Locked |
| [0031](0031-borrow-expression-syntax.md) | Borrow expression syntax (`&FORM operand`, E2420) | Locked |

### v0.10 — Builtin Shims + AOT Cache + NLL

| ADR | Title | Status |
|---|---|---|
| [0032](0032-builtin-shim-abi.md) | Builtin shim ABI (hybrid RuntimeValue, `unsafe_code = "deny"`) | Locked |
| [0033](0033-aot-cache-cranelift-object.md) | AOT cache via `cranelift-object` | Locked |
| [0034](0034-jit-aggregate-coverage.md) | JIT aggregate coverage via delegate shims | Locked |
| [0035](0035-jit-boxed-refcount-discipline.md) | JIT boxed-value refcount discipline | Locked |

### v0.11 — Compiler Pipeline Refinement

| ADR | Title | Status |
|---|---|---|
| [0036](0036-typetag-opaque-aggregate.md) | `TypeTag::Opaque` — disambiguating user aggregates | Locked |

---

### Rewrite Backend (2026-06-04+ — Clean-Architecture JIT Compiler)

> **Milestone:** The legacy v0.2–v0.10 backend was replaced with a clean, verification-driven pipeline (`source → AST → MIR → NLL Borrowck → Cranelift JIT`). All decisions below govern the active compiler.

| ADR | Title | Status |
|---|---|---|
| [0037](0037-enum-tagged-union-layout.md) | Enum tagged-union layout (discriminant + payload StackSlot) | Locked |
| [0038](0038-comparable-trait-deferred.md) | Comparable trait (`compare() -> Trit`, deferred) | Deferred-Locked |
| [0039](0039-nullable-operator-family.md) | Nullable operator family (`?+>`, `?0>`, `?->`) | Locked |
| [0040](0040-heap-aggregate-layout.md) | Heap aggregate layout (String/Vector shims, zero-on-move) | Locked |
| [0041](0041-nullable-representation-bac-a.md) | Nullable `T?` Tier A (`i64::MIN` sentinel, Elvis `?:`, match 2-arm) | Locked |
| [0042](0042-ownership-across-boundary.md) | Ownership across function boundary (move-only, `Deinit` tombstone) | Locked |
| [0043](0043-hashmap-representation.md) | HashMap representation (open addressing 24B slot, D2) | Locked |
| [0044](0044-arithmetic-range-enforcement.md) | Arithmetic range enforcement (trap-on-overflow, smulhi, E1036) | Locked |
| [0045](0045-borrow-params-heap.md) | Borrow parameters for heap types (`&0 String`, `&0 Vector`) | Locked |
| [0046](0046-return-borrow-elision.md) | Return borrow elision (lifetime propagation across calls) | Locked |
| [0047](0047-read-ops-extension.md) | Read ops extension (`len`, `is_empty`, `capacity`) | Locked |
| [0048](0048-mutable-borrow.md) | Mutable borrow (`&0 mutable`, exclusive loan semantics) | Locked |
| [0049](0049-fat-pointer-abi.md) | Fat pointer ABI for slices and references | Locked |
| [0050](0050-mir-type-enum.md) | MIR Type enum restructuring (clean orthogonal representation) | Locked |
| [0051](0051-borrowck-unification-nll-mir.md) | Borrowck unification (NLL dataflow on flat MIR CFG) | Locked |
| [0052](0052-outcome-abi-implementation.md) | Outcome ABI implementation (disc + payload layout) | Locked |
| [0053](0053-heap-payload-outcome.md) | Heap payload in Outcome (`Outcome<String, Error>`) | Locked |
| [0054](0054-borrowck-drop-kills-liveness.md) | Borrowck: drop kills liveness of loans | Locked |
| [0055](0055-block-body-tail-expression.md) | Block body tail expression (Rust-style implicit returns) | Locked |
| [0056](0056-heap-value-merge.md) | Heap value merge across control flow branches | Locked |
| [0057](0057-jit-outcome-slot-move.md) | JIT Outcome slot move and value reconstruction | Locked |
| [0058](0058-heap-outcome-sret-and-merge.md) | Heap Outcome SRet and branch merge | Locked |
| [0059](0059-stack-borrow-heap-vector-hashmap.md) | Stack borrow (`&0`) for heap Vector and HashMap | Locked |
| [0060](0060-nested-aggregate-layout.md) | Nested aggregate layout (struct-in-struct sizing & projection) | Locked |
| [0061](0061-trait-system-tier1-static-dispatch.md) | Trait system (Tier 1): static dispatch via mangled names | Locked |
| [0062](0062-heap-nullable-ptr-sentinel-repr.md) | Heap-Nullable representation (ptr-sentinel for String/Vector/HashMap) | Locked |
| [0063](0063-loan-liveness-through-merge.md) | Point-level loan liveness at Drop across block-merge | Locked |
| [0064](0064-match-exhaustiveness.md) | Match exhaustiveness (scalar literals and patterns) | Locked |
| [0065](0065-aggregate-nullable.md) | Nullable aggregate (`Struct?` & `Enum?` nullable stack-slots) | Locked |
| [0066](0066-heap-in-aggregate-move-drop-glue.md) | Heap-in-aggregate: move & drop-glue (Flat, Slice 1, SRet ABI) | Locked |
| [0067](0067-nested-flat-enum-payload-drop-glue.md) | Nested-flat & enum-payload heap drop-glue (No-Box, Slice 2) | Proposed |
| [0069](0069-zst-capability-token-luk3.md) | ZST capability token with Ł3-Trit (borrowck-enforced) | Proposed |
| [0070](0070-partial-move-field-level-move-state.md) | Partial-move & field-level move-state (ZST/Capability) | Locked |
| [0071](0071-path-separator-and-module-import.md) | Path separator (`::`) & module import (`use`) — supersedes ADR-0005 | Locked |
| [0072](0072-expected-type-propagation-in-lowering.md) | Expected-type propagation in AST→MIR lowering | Locked |
| [0076](0076-heap-nullable-aggregate-field.md) | Heap-Nullable in aggregate fields/payloads (closing B8 barrier) | Locked |
| [0077](0077-typed-vector-p1.md) | Typed Vector P1 (element-type via type erasure) | Draft |
| [0078](0078-typed-hashmap-p1-value.md) | Typed HashMap P1 (value-typed: `HashMap<Integer, T>`) | Locked |
| [0079](0079-get-borrow-heap-value-from-container.md) | Get-borrow heap value from container (`get(&0 c, k) -> (&0 V)?`) | Locked |
| [0080](0080-hashmap-string-key.md) | Key-typed HashMap P1 (`HashMap<String, V>`, content hash/eq) | Locked |
| [0081](0081-get-borrow-mutable.md) | Get-borrow-mutable from container (`get(&0 mut c, k) -> (&0 mut V)?`) | Frozen |
| [0082](0082-aggregate-by-value-collection-element.md) | Aggregate by-value as collection elements | Locked |
| [0083](0083-key-aggregate-hashmap.md) | Key-aggregate HashMap (`HashMap<Struct, V>`, fnptr-in-header) | Locked |
| [0084](0084-ref-field-auto-deref.md) | Field projection through read-only reference (auto-deref) | Locked |
| [0085](0085-shim-meta-totality-verify-gate.md) | Full `builtin_shim_meta` table & verification gate | Locked |
| [0086](0086-lower-error-code-taxonomy.md) | `LowerError` error code taxonomy (`triet::lower::E11XX`) | Locked |
| [0087](0087-builtin-print-overloads-and-io-shim.md) | Builtin print overloads & native I/O shim | Locked |
| [0088](0088-double-nullable-container-reads.md) | Double-nullable container reads (`T??` on `get`-family, E1055) | Locked |
| [0089](0089-concrete-loop-cfg-range-iteration.md) | Concrete loop CFG & range iteration — supersedes ADR-0003 | Locked |

---

## How to Read an ADR

Every ADR follows a standardized structure:

1. **Status** — Locked / Informational / Superseded / Proposed / Draft.
2. **Issue** — What technical problem forced the decision now.
3. **Decision** — The concrete architectural choice.
4. **Rationale** — Why this design was chosen over alternatives.
5. **Consequences** — What becomes possible, constrained, or costly.
6. **Alternatives Considered / Rejected Alternatives** — Explicitly rejected alternatives and why.
7. **References** — Links to SPEC sections, sibling ADRs, and tests.

Search tip: `grep -rn "## Decision" docs/decisions/` lists every decision summary in <100 lines.
