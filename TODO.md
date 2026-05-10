# TODO

Sub-task tracking — short-term work in progress.

- Long-term phasing: [`ROADMAP.md`](ROADMAP.md)
- Architectural decisions: [`docs/decisions/`](docs/decisions/)
- Language semantics: [`SPEC.md`](SPEC.md), [`VISION.md`](VISION.md)

This file is updated as tasks complete. When a phase finishes (e.g. v0.2.x),
the summary is archived into `ROADMAP.md` và detailed checkboxes
removed from here.

---

## v0.3 — Bytecode VM + Stable IR (in progress)

Per [ROADMAP.md § v0.3](ROADMAP.md). Mục tiêu: thiết kế và lock **Triết IR** —
biên giới ngôn ngữ ↔ phần cứng. Bytecode VM ở phase này là **development
tier scaffolding**, không phải production runtime. Production target nhị
phân là AOT native (LLVM, v2.0); production target tam phân là trytecode
native (v∞). Xem [VISION §4](VISION.md).

VM v0.3 tồn tại để: (1) validate IR design qua thực thi trước khi commit
IR vĩnh viễn, (2) cho self-hosting compiler (v0.7) một platform chạy
trước khi LLVM landing, (3) differential test oracle so sánh với
tree-walker (v0.2), (4) phát triển ecosystem trong khi backend production
chưa có.

**Architecture decisions:**

- **[ADR-0007](docs/decisions/0007-ir-design.md)** ✓ — IR là **register-based,
  SSA form, virtual register vô hạn, type-tagged per register**. Map 1:1
  sang LLVM IR (v2.0), Cranelift IR (v0.9), trytecode (v∞).
- **ADR-0008** (sẽ viết ở v0.3.8) — Bytecode binary format `.triv`: magic +
  version, section layout (header / constant pool / function table /
  code), endianness, varint encoding, stable cho v1.0 freeze.

**Gate cho phase (per ROADMAP):**
1. IR spec written (✓) + bytecode format có version field (ADR-0008).
2. Mọi `examples/*.tri` chạy qua bytecode VM với output **byte-identical**
   tree-walking interpreter (incl. diagnostics).
3. Bench ≥3× speedup vs interpreter trên 11 demo programs.
4. IR snapshot tests detect regression khi đổi lowerer.

**Không làm trong v0.3 (deferred):**
- JIT (v0.9, Cranelift backend đọc cùng IR).
- Native AOT compile (v2.0, LLVM backend đọc cùng IR).
- Trytecode backend (v∞ + ternary hardware).
- ABI metadata trong `.triv` (v0.4 — cần IR ổn trước).
- Cross-package linking (v0.4).

### Done

- [x] **v0.3.0** — ADR-0007: IR design — register-based SSA decided `abbd1d9`
  - Survey prior art: LLVM IR, Rust MIR, Swift SIL, Cranelift IR
    (register SSA — adopted); JVM, Wasm, CPython 3.x (stack — rejected).
  - Tradeoff matrix mapped to SPEC §0.3 principles: AI-first, Stability,
    Refuse over guess, Tam phân first-class, multi-backend trajectory.
  - Decision rationale: VM v0.3 là dev tier; production targets là AOT
    native (LLVM v2.0) + trytecode (v∞); IR phải map 1:1 sang LLVM/
    Cranelift/ternary CPU register.
  - Output: [`docs/decisions/0007-ir-design.md`](docs/decisions/0007-ir-design.md)
    (full ADR with alternatives, hậu quả, implementation roadmap).
  - Companion docs updates: VISION §4 (execution model multi-backend),
    SPEC §0.6 (VM as dev tier), ROADMAP § v0.3 (clarify VM is scaffolding).

- [x] **v0.3.1** — Scaffold `triet-ir` crate `abbd1d9`
  - Crate `triet-ir` created + added to workspace ✓.
  - Types: `ValueId`, `BlockId`, `FuncId`, `ConstId`, `TypeTag` ✓ (types.rs).
  - Constant pool: `Constant` enum + `ConstantPool` với dedup ✓ (constant.rs).
  - Instruction set: 50+ opcodes grouped per ADR-0007 — const, arith, logic
    Ł3/K3, comparison, conversion, aggregate (struct/enum), nullable,
    function calls (local/cross-module/builtin), closure, control flow
    (br/br_if/ret/unreachable), phi ✓ (instr.rs).
  - Module types: `BasicBlock`, `Function`, `IrModule`, `IrProgram` ✓
    (module.rs).
  - Display formatting cho disassembly ✓ (display.rs).
  - SSA verifier: duplicate definition, undefined value, missing terminator,
    phi order, empty function, invalid phi predecessor ✓ (verify.rs).
  - 21 unit tests: constant pool intern/dedup/types, factorial IR construction,
    function well-formedness, verifier SSA checks, display formatting,
    operand/value extraction, IrProgram ✓ (lib.rs tests).
  - Clippy auto-fix ✓. Workspace tests xanh ✓.

### Pending

- [ ] **v0.3.2** — Lowerer: AST → IR (core expressions + statements) _(uncommitted)_
  - `lower_program(&ResolvedProgram) -> IrProgram` entry point ✓
  - Literals (Integer, Ternary, Trilean, String, Null, FString) ✓
  - Arithmetic (`+`, `-`, `*`, `/`, `%%`, `**`), comparison, symbolic
    logic ops (`!`, `&&`, `||`, `^`, `=>`, `~>`, `~^`, `<=>`, `<~>`) ✓
  - Variable bindings (`let`), assignment, lexical scope tracking ✓
  - Control flow: `if/else`, `while`, `loop`, `break`, `continue`,
    `if?`, `while?` (unknown-as-false handling) ✓
  - Phi nodes ở if-else merge ✓
  - For loop (scaffold, full iterator protocol deferred to v0.3.4) ✓
  - Match expression (simplified, tag checks deferred to v0.3.4) ✓
  - Function calls (local, cross-module, builtin) ✓
  - 51 unit tests (31 lowerer + 13 verifier/IR type + 7 pre-existing) ✓
  - Edge cases: nested blocks/shadowing, nested if/else, nested loops, early
    return, multi-param, forward ref, null ops, struct/enum/tuple literals,
    while?, break with value, cross-module call, empty program, multi-module,
    large/negative integers, all Ł3 ops (9×真理値組み合わせ), all comparison
    ops with equal values, safe field access, elvis, const, method call,
    range, field access, block without final expr, if without else ✓
  - Bug fix: `BlockId` → `BTreeMap` thay vì `Vec` để fix index out of bounds
    khi nested control flow ✓
  - Bug fix: verifier treats function params as implicitly defined ✓
  - `triet-syntax` dependency added to triet-ir ✓
  - `ModuleId`/`ArenaId` fields made `pub` for cross-crate construction ✓

- [ ] **v0.3.5** — VM: execute IR (`triet-ir/src/vm.rs`) _(uncommitted)_
  - `Vm::new(IrProgram)` + `execute(FuncId, args) -> Result<RuntimeValue, VmError>` ✓
  - `RuntimeValue` enum: Trit/Tryte/Integer/Long/Trilean/String/Unit/Null/
    Struct/Enum/Closure ✓
  - `Frame` với register file, block/pc tracking, return info ✓
  - Dispatch loop: tất cả 52 opcodes có handler ✓
  - Arithmetic: type-tag aware (Integer/Long/Tryte/Trit) ✓
  - Trilean Ł3/K3 dispatch (separate opcodes) ✓
  - Comparison (eq/ne/lt/le/gt/ge) → Trilean ✓
  - Conversion (to_integer/to_tryte/to_long/to_trit/to_trilean) ✓
  - Nullable: NullWrap/NullUnwrap/NullCheck ✓
  - Aggregate: StructNew/FieldGet/FieldSet, EnumNew/EnumTag/EnumPayload ✓
  - Function calls: CallLocal (full), CallBuiltin, CallCrossModule (stub) ✓
  - Closure: ClosureNew/ClosureCall ✓
  - Control flow: Br/BrIf/Ret/Unreachable, phi với prev_block tracking ✓
  - Frame stack: call/return với return_block/return_dest ✓
  - Builtins: println, print, assert, assert_eq ✓
  - Diagnostic codes E22XX (8 variants) ✓
  - 12 VM tests: arithmetic, logic Ł3, comparison, conditional, factorial
    recursive, phi after if-else, div by zero, builtin assert ✓

- [ ] **v0.3.3** — Lowerer: items + functions + modules
  - (Đa số đã làm trong v0.3.2; còn lại generics monomorphization cần typechecker)
  - Function definitions + signatures + parameter binding.
  - Generics monomorphization (cùng pattern typecheck đã làm trong G.1c).
  - Cross-module calls qua `AbsolutePath` từ `triet-modules` (capability
    namespace preserved cho v0.6).
  - Function table indexing strategy.

- [ ] **v0.3.4** — Lowerer: aggregates + match + closures
  - Struct literal + field access + field assignment.
  - Enum literal + pattern destructuring (`match`, `if let`).
  - Match exhaustiveness verifier check at IR level.
  - Closure capture (by-value cho v0.3; mutable capture defer).
  - Builtin call dispatch (`println`, `assert`, `assert_eq`, ...).
  - Nullable ops (`?.`, `?:`, `!!`) → `null_check`/`null_unwrap`/`null_wrap`.

- [ ] **v0.3.5** — VM: execute IR (`triet-vm` crate hoặc trong `triet-ir`)
  - Opcode dispatch loop với type-tag aware operations.
  - Trilean Ł3/K3 dispatch (separate opcodes, không cào).
  - Long arithmetic dùng heap-allocated big-int (như v0.2 `bnum::I256`).
  - Function call/return, frame allocation.
  - Pattern match evaluator.
  - Builtin call dispatch.
  - Diagnostic codes E22XX cho VM runtime errors (out of bounds, type
    tag mismatch, unwrap of null, etc.).

- [ ] **v0.3.6** — Snapshot tests: IR output cho `examples/*.tri`
  - Insta snapshots cho lowered IR mỗi example file.
  - Regression detection khi đổi lowerer hoặc instruction set.

- [ ] **v0.3.7** — Differential tests: VM ≡ tree-walking interpreter
  - Mỗi `examples/*.tri`: run qua cả hai, so sánh stdout + exit code
    byte-by-byte.
  - Diagnostics cho program lỗi: phải identical (cùng error code, cùng
    span, cùng message).
  - Cover cả demo `demos/02-module-system/main.tri` (704-line ALU).

- [ ] **v0.3.8** — ADR-0008: bytecode binary format `.triv`
  - Magic bytes (`0x74 0x72 0x69 0x76` = "triv"?), version field.
  - Section layout: header / constant pool / function table / code.
  - Endianness (little-endian giả định), alignment, varint cho
    instruction operands.
  - Stable cho v1.0 freeze (additive-only sau v1.0).
  - Companion ADR cho ADR-0007 — chia tách "IR shape" và "wire format".

- [ ] **v0.3.9** — Serialize/deserialize: `.triv` reader/writer
  - Writer: `IrProgram → Vec<u8>`.
  - Reader: `&[u8] → IrProgram` với version check + corruption detection.
  - Round-trip tests cho mọi example: parse → lower → serialize →
    deserialize → run → so sánh output.

- [ ] **v0.3.10** — CLI: `triet build` subcommand + `.triv` execution
  - `triet build foo.tri -o foo.triv` — parse + typecheck + lower + serialize.
  - `triet run foo.triv` — auto-detect bytecode vs source theo extension.
  - Backward-compat: `triet run foo.tri` vẫn work (lower + run in-memory).

- [ ] **v0.3.11** — Benchmark harness (criterion) + gate verification
  - Bench cho 11 demo programs: bytecode VM vs tree-walking interpreter.
  - Gate: ≥3× speedup theo ROADMAP.
  - Document baseline numbers in `BENCHMARKS.md` (mới).
  - Nếu không đạt 3×: profile, optimize (instruction dispatch, value
    representation), iterate trước khi đóng phase.

---

## How to update this file

- Mark a task `[x]` and move it to **Done** when its commit lands on `main`.
- Add the commit short-hash next to completed tasks for quick git reference.
- Keep the order: **Done** → **In progress** → **Pending**.
- When a whole phase (e.g. v0.2.x) ships, archive its summary into
  `ROADMAP.md` (under the changelog section) and delete the detailed
  checkboxes from this file.
