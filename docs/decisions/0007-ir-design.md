# ADR 0007 — IR design: register-based SSA, multi-backend substrate

**Trạng thái:** Quyết định. Áp dụng cho v0.3+ và mọi backend sau (v0.9 JIT, v2.0 AOT, v∞ trytecode). Là biên giới ngôn ngữ ↔ phần cứng theo [VISION §4](../../VISION.md).

**Issue:** Triết v0.2 là tree-walking interpreter chạy thẳng AST. Mô hình này không scale lên các trụ cột phía sau:
- **CAS packaging (v0.5)** cần content hash deterministic — AST node IDs thay đổi mỗi commit, không hash được.
- **Stable ABI (v0.4)** cần signature hash trên IR ổn định, không phải AST.
- **Self-hosting compiler (v0.7)** cần một format IR để Triết-compiler-viết-bằng-Triết emit ra.
- **JIT (v0.9, Cranelift)** compile bytecode → machine code; phải có bytecode để bắt đầu.
- **AOT native (v2.0, LLVM)** map từ Triết IR → LLVM IR; mapping càng trực tiếp càng ít công sức.
- **Trytecode native (v∞, ternary hardware)** map từ Triết IR → instructions cho CPU tam phân thật.

Quyết định IR shape *bây giờ* affect toàn bộ chuỗi v0.3 → v∞. Sai ở đây = đập đi viết lại nhiều phase. ADR này lock IR shape cho mọi backend.

## Quyết định

Triết IR là **register-based, SSA form, virtual register count vô hạn, type-tagged per register**. Wire format `.triv` (ADR-0008, sẽ viết khi v0.3.8 bắt đầu) sẽ serialize cùng shape — không tách wire vs in-memory ở v0.3.

### Hình thức cụ thể

3-address SSA instructions với type tag mỗi register:

```
function @factorial(%n : Integer) -> Integer {
entry:
    %is_zero = eq %n, const Integer 0_integer
    br_if %is_zero, base_case, recursive_case

base_case:
    ret const Integer 1_integer

recursive_case:
    %n_minus_1 = sub %n, const Integer 1_integer
    %recurse   = call @factorial(%n_minus_1)
    %result    = mul %n, %recurse
    ret %result
}
```

**Đặc trưng:**
- Mỗi virtual register `%name` được gán **đúng một lần** (SSA invariant).
- Mỗi register mang type tag: `Trit`, `Tryte`, `Integer`, `Long`, `Trilean`, `String`, `Unit`, `T?`, hoặc user-defined struct/enum/closure.
- Function chia thành **basic blocks**, mỗi block kết thúc bằng terminator (`ret`, `br`, `br_if`, `match`, `unreachable`).
- **Phi nodes** ở entry của block hợp nhất giá trị từ nhiều predecessor: `%v = phi [%a from L1], [%b from L2]`.
- **Constants** không tốn register — `const Integer 42_integer` là operand inline.
- Số register virtual không giới hạn; backend (Cranelift, LLVM) tự lo register allocation.

### Phân nhóm instruction (high-level, không exhaustive)

ADR này không liệt kê đầy đủ instruction set — chi tiết sẽ landed ở v0.3.1 (scaffold `triet-ir` crate). Nhóm phân loại:

| Nhóm | Ví dụ opcodes |
|---|---|
| **Constants** | `const Integer 42_integer`, `const String "hello"`, `const Trilean unknown` |
| **Arithmetic** | `add`, `sub`, `mul`, `div`, `mod`, `pow`, `neg` (cho Tryte/Integer/Long) |
| **Trit/Trilean logic Ł3** | `trit_not`, `trit_and`, `trit_or`, `luk_implies`, `luk_xor`, `luk_iff` |
| **Trit/Trilean logic K3** | `kleene_implies`, `kleene_xor`, `kleene_iff` |
| **Comparison** | `eq`, `ne`, `lt`, `le`, `gt`, `ge` (kết quả Trilean nhưng không bao giờ unknown — value equality theo SPEC §4.5) |
| **Conversion** | `to_integer`, `to_tryte`, `to_long`, `to_trit`, `to_trilean` (+ saturating/truncating variants) |
| **Control flow** | `br <label>`, `br_if <cond>, <true_label>, <false_label>`, `match <scrutinee>, [arms]`, `ret`, `unreachable` |
| **Function** | `call @func(%args)`, cross-module qua `AbsolutePath` từ `triet-modules` |
| **Aggregate** | `struct_new`, `field_get`, `field_set`, `enum_new`, `enum_tag`, `enum_payload` |
| **Nullable** | `null_wrap` (T → T?), `null_unwrap` (T? → T, panic), `null_check` (T? → Trit) |
| **Closure** | `closure_new @lambda, [captures]`, `closure_call %c, [args]` |
| **Builtin** | `builtin "<name>", args` (cho `println`, `assert`, ...) |
| **Outcome** *(v0.7.4.3-error+, [ADR-0020](0020-outcome-error-handling.md))* | `outcome_new_positive` (Trit::Positive arm), `outcome_new_negative` (Trit::Negative arm), `outcome_new_null` (Trit::Zero arm, T?~E only), `outcome_discriminant` (extract trit), `outcome_unwrap_value` (panic if not Positive), `outcome_unwrap_error` (panic if not Negative). Wire opcodes 0xC1–0xC6. Cross-references nullable `T?` via shared `Constant::Null` for compile-time null literals (per [ADR-0010 Addendum](0010-ternary-native-ir.md#addendum--v0743-error-null-literal-unification)). |

**Capability annotation:** Cross-module call mang theo namespace tag từ `AbsolutePath` (e.g., `call @sys.print %s` là gọi vào `sys.*`). v0.6 capability check sẽ đọc tag này — không cần thay đổi IR shape khi enforce capability.

### Wire format (defer to ADR-0008)

- v0.3.0–v0.3.7: in-memory IR == wire format. Serialization là Rust `bincode` hoặc tương đương cho nhanh — KHÔNG phải binary format chính thức.
- v0.3.8 (ADR-0008): design `.triv` format chính thức — magic bytes, version field, sections (header / constant pool / function table / code), varint encoding.
- Stable cho v1.0 freeze. Sau v1.0: additive-only, mọi backend phải đọc được IR cũ.

## Lý do

### Mapped to nguyên tắc thiết kế của SPEC

| SPEC § | Nguyên tắc | Áp dụng cho register SSA |
|---|---|---|
| §0.3.1 | **AI-first** — explicit > implicit, low ambiguity > terseness | SSA register IR human-readable: `%result = mul %n, %recurse` rõ data flow. Stack IR ngầm data flow qua stack position — LLM phải simulate stack trong đầu khi đọc. LLM training data dày trên LLVM IR / Rust MIR (cùng kiến trúc) — sinh code đúng ngay. |
| §0.3.4 | **Stability over speed** — ADR-driven, không "ship đại rồi sửa" | SSA register là kiến trúc dominant trong compiler hiện đại sau 2010 (LLVM, Rust MIR, Swift SIL, Cranelift IR, GCC GIMPLE). Conservative choice với prior art mạnh. Stack VM là di sản từ thời "ngôn ngữ chỉ chạy trên VM mãi mãi" (JVM 1995, Wasm 2017) — không áp dụng cho Triết AOT-native. |
| §0.3.5 | **Refuse over guess** — error rõ ràng, không suy luận im lặng | SSA invariant ("each register defined exactly once") là static check mạnh, verifier đơn giản. Bug compiler bắt sớm, không lan xuống backend. |
| §0.3.6 | **Explicit > implicit** — export, capability, dependency tường minh | Register IR explicit operand names. Stack IR implicit data flow qua stack position — sai stack effect bug khó debug. |
| §0.2 | **Tam phân first-class** — Trit/Tryte/Integer/Long là primitive fixed-size | Type tag per register cho phép `%t1 : Trit`, `%t2 : Trilean`, `%t3 : Long` mang theo type info từ AST → IR → backend. Stack IR cào về "stack slot" generic — type info phải lưu metadata phụ, dễ drift. |

### Trajectory multi-backend (VISION §4)

Đây là lý do then chốt. Triết là **AOT native language với multi-backend strategy**, không phải VM-based language. IR phải ổn định và map tốt sang **mọi backend**, không chỉ VM v0.3.

| Backend | Phase | IR target | Mapping từ SSA register |
|---|---|---|---|
| Bytecode VM | v0.3 | (Triết IR thẳng) | Trivial — VM interpret IR trực tiếp |
| JIT Cranelift | v0.9 | Cranelift IR (SSA register) | Map 1:1 — cùng paradigm |
| AOT LLVM | v2.0 | LLVM IR (SSA register) | Map 1:1 — cùng paradigm |
| Trytecode native | v∞ | Ternary CPU instructions (register-based) | Map gần 1:1 — CPU vật lý cũng register-based |

Nếu Triết IR là stack-based, **mỗi backend phải viết stack-to-SSA lifting pass riêng** — công sức 3 lần (Cranelift, LLVM, trytecode). Đây là chi phí permanent kéo dài qua v0.9/v2.0/v∞, không phải tiết kiệm một lần ở v0.3.

### Triết-specific considerations

**Trilean ops phải là first-class opcode**, không phải function call. Ł3 và K3 là ngôn ngữ primitive theo SPEC §4 — interpreter v0.2 dispatch trực tiếp, không qua function. IR phải giữ nguyên semantics: `luk_implies` ≠ `kleene_implies`, dispatched ở IR level. Capability check (v0.6) đọc opcode để biết user intentionally chọn Ł3 hay K3.

**Long arithmetic backend-specific.** Long là 81-trit big-int, không fit trong native register CPU thông thường (>128 bit). Backend lowering:
- VM v0.3: heap-allocated big-int (tương tự `bnum::I256` v0.2 đang dùng).
- AOT LLVM v2.0: i256 hoặc runtime call vào libgmp.
- Trytecode v∞: native 81 trit register (vì hardware tam phân fit).

IR opcode `add` cho Long emit cùng instruction; backend tự lo lowering. Đây là pattern LLVM IR đã chứng minh: high-level ops + backend-specific lowering.

**Capability namespace preserved.** Function call cross-module mang `AbsolutePath` từ `triet-modules`. IR encode `call @std.io.println %s`, KHÔNG strip thành `call @println %s`. Lý do:
- v0.6 capability check phải biết namespace để enforce (`usr.*` không gọi được `dev.*` không capability).
- v0.5 CAS hash phải bao gồm namespace để identity ổn định khi rename intra-module.
- Debug output rõ ràng cho LLM/dev.

**Nullable `T?` discriminator preserved.** SPEC §2.5 + ADR-0001: `T?` là 1-trit discriminator + T payload. IR ops `null_wrap`/`null_unwrap`/`null_check` explicit, không implicit unwrap. Match nullable pattern lower thành `null_check` + `br_if` + `null_unwrap`.

**Pattern matching exhaustive enforced ở IR verifier.** SPEC §7.3 yêu cầu match exhaustive. Lowerer kiểm tra exhaustiveness và emit `match` opcode với danh sách arm đầy đủ; verifier check lại invariant. Lower xuống cascade `br_if` ở backend.

**Memory model deferred to v0.3 implementation.** SPEC §10 nói memory model nailed-down ở v0.4 (ABI). v0.3 IR sẽ giả định Mojo-style ARC như v0.2 đang làm. ADR riêng (sẽ là ADR-0009 hoặc tương đương ở v0.4) sẽ refine: ARC opcodes (`retain`, `release`), borrow check ở IR level. v0.3 IR có thể thêm opcodes này additive sau.

### Error code namespace

Mở rộng namespace theo CLAUDE.md:

| Range | Component |
|---|---|
| `E0000` | Lexer |
| `E000X` | Parser |
| `E10XX` | Typecheck |
| `E20XX` | Interpreter (tree-walking, v0.2) |
| `E21XX` | Modules (loader/resolver) |
| **`E22XX`** | **VM runtime (v0.3)** — out of bounds, stack overflow, type tag mismatch, unwrap of null, etc. |
| `E23XX` | (reserved cho IR verifier khi v0.3.1 scaffold xong) |

## Alternatives considered

### A1. Stack-based bytecode (JVM, Wasm, CPython 3.x, .NET CIL)

**Reject.**

Pro:
- Implementation đơn giản hơn cho VM v0.3 (~30% ít code).
- Wire format compact hơn (mỗi opcode không cần explicit operand).
- Prior art khổng lồ (JVM 30 năm, Wasm là standard hiện đại).

Con (chí mạng):
- **Implicit data flow** vi phạm AI-first §0.3.1 — LLM/dev phải simulate stack trong đầu để đọc IR.
- **Type info bị cào** về "stack slot" — vi phạm Tam phân first-class §0.2.
- **Mọi backend AOT/JIT phải lifting stack → SSA** — công sức permanent qua v0.9/v2.0/v∞.
- **Stack effect verification yếu hơn SSA invariant** — bug khó bắt sớm.
- **Wasm precedent không áp dụng** — Wasm chọn stack vì web sandbox + small wire size cho download. Triết không có constraint đó.
- **JVM precedent không áp dụng** — JVM chọn stack vì 1995 hardware có ít register; modern hardware register-rich.

Quan trọng nhất: Triết end-game là AOT native (v2.0 LLVM) + trytecode native (v∞). VM chỉ là dev tier. Tối ưu IR cho VM = đầu tư sai chỗ.

### A2. Tree-IR / direct AST execution

**Reject.** Đó là v0.2 hiện tại. Không scale lên CAS, ABI, JIT, AOT vì AST không stable, không hash được, không lower được sang machine code.

### A3. CPS (Continuation-Passing Style) IR

**Defer.** Hợp lý cho concurrency model (v0.8 actor) — first-class continuation map tự nhiên xuống green threads. Nhưng:
- Phức tạp impl + LLM khó học.
- Triết v0.3 không có concurrency yet.
- Có thể bổ sung pass CPS-conversion *trên* SSA IR khi v0.8 actor model định hình.

Không reject vĩnh viễn — chỉ chưa đến lúc.

### A4. MLIR-style multi-level IR (multiple dialects)

**Defer.** Lợi ích cho compiler có nhiều domain-specific dialect (Tensor + scalar + GPU). Triết drop SIMD/Tensor (SPEC §10.5 đã loại) → không cần multi-dialect. Có thể adopt ở v2.0+ nếu LLVM backend cần custom optimization passes.

### A5. Direct lowering AST → LLVM IR (skip Triết IR)

**Reject.**

- LLVM lock-in. Trytecode backend (v∞) bị chặn.
- Self-hosting (v0.7) không khả thi — Triết-compiler-viết-bằng-Triết phải emit cái gì đó simpler than LLVM IR.
- JIT (v0.9 Cranelift) cũng phải duplicate lowering — không cùng pipeline với AOT.
- LLVM build dependency khổng lồ — vi phạm "incremental progress, validate IR sớm" của Stability over speed.

Triết IR là **buffer giữa Triết source và mọi backend** — bắt buộc tách layer.

### A6. Continuation-with-block-args (CFG IR không phi nodes, kiểu Cranelift sớm)

**Consider.** Cranelift IR ban đầu dùng block arguments thay phi nodes — lý thuyết tương đương SSA, đôi khi đơn giản hơn impl. Triết có thể adopt nếu trong v0.3.1 thấy phi node phức tạp không cần thiết.

**Quyết định ở v0.3.1**, không lock-in ngay ADR-0007. ADR sẽ update nếu chọn block arguments.

## Hậu quả

**Tích cực:**
- Multi-backend trajectory (VM → JIT → AOT → trytecode) mỗi backend chỉ cần lowerer riêng — không re-design IR.
- IR human-readable cho debug + LLM-readable cho AI assistance (cùng kiến trúc training data).
- SSA invariant catch IR bugs early ở verifier.
- Map 1:1 sang LLVM IR (v2.0) — minimal lowering effort.
- Type tag per register giữ Tam phân first-class suốt pipeline.
- Capability namespace preserved → v0.6 enforce không cần đổi IR.

**Tiêu cực:**
- Implementation effort v0.3 cao hơn ~30% so với stack VM. Accept tradeoff vì mọi phase sau hời hơn nhiều.
- Wire format có thể phình hơn stack bytecode (operand explicit). Mitigation: ADR-0008 sẽ design varint encoding cho compact storage.
- Cần register allocator ở backend mã hóa (v0.9 JIT, v2.0 AOT). Cranelift/LLVM tự lo, Triết KHÔNG phải implement register allocation. v0.3 VM không cần register allocation (virtual register vô hạn, dispatch trực tiếp).

**Migration strategy:**
- v0.2 baseline: tree-walking interpreter. Giữ nguyên cho differential test ở v0.3.7.
- v0.3.1–v0.3.7: scaffold IR + lowerer + VM. Tree-walker là oracle, VM phải match byte-by-byte.
- v0.3.8+: bytecode binary format `.triv`. CLI thêm `triet build` subcommand.
- v0.4+: ABI metadata + CAS hash đọc từ IR/`.triv`.
- v0.7: self-hosting compiler emit cùng IR shape.
- v0.9: Cranelift backend cho JIT — mới, không thay IR.
- v2.0: LLVM backend cho AOT — mới, không thay IR.
- v∞: Trytecode backend — mới, không thay IR. Chỉ thêm opcode lowering rules cho ternary CPU.

**Breaking change ở IR cần ADR riêng** (theo precedent ADR-0005). Sau v1.0: chỉ additive, không phá ngữ nghĩa cũ.

## Implementation roadmap (v0.3.0 → v0.3.7)

Chi tiết sub-tasks ở [TODO.md § v0.3](../../TODO.md). Outline:

1. **v0.3.0 — ADR-0007** (this) ✓ — IR shape decided.
2. **v0.3.1 — Scaffold `triet-ir` crate** — concrete instruction set, constant pool, basic block + function + module types, display formatting, IR verifier.
3. **v0.3.2 — Lowerer AST → IR (core)** — literals, arithmetic, logic Ł3/K3, comparison, control flow.
4. **v0.3.3 — Lowerer items + functions + modules** — function definitions, generics monomorphization, cross-module calls với `AbsolutePath`.
5. **v0.3.4 — Lowerer aggregates + match + closures** — struct/enum/closure/builtins/nullable.
6. **v0.3.5 — VM execution** — interpret IR theo opcode dispatch; cùng output với tree-walker.
7. **v0.3.6 — Snapshot tests** — IR output cho mọi `examples/*.tri`.
8. **v0.3.7 — Differential tests** — VM ≡ tree-walker byte-by-byte cho mọi example.

Sau đó:
- **v0.3.8 — ADR-0008** (sẽ viết) — bytecode binary format `.triv`.
- **v0.3.9 — Serializer/deserializer** — round-trip IR ↔ `.triv`.
- **v0.3.10 — CLI rewire** — `triet build` + `.triv` execution.
- **v0.3.11 — Benchmark + gate verification** — bench ≥3× tree-walker.

## References

- [LLVM IR Reference](https://llvm.org/docs/LangRef.html) — SSA register design, prior art chính.
- [Rust MIR Documentation](https://rustc-dev-guide.rust-lang.org/mir/index.html) — high-level IR cho Rust, prior art gần Triết nhất (similar source language complexity).
- [Swift SIL](https://github.com/apple/swift/blob/main/docs/SIL.rst) — generics + ABI considerations ở IR level (prior art cho v0.4 ABI).
- [Cranelift IR Reference](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/docs/ir.md) — JIT-friendly SSA IR, target backend cho v0.9.
- ["A Look at the Lua 5 Implementation"](https://www.lua.org/doc/jucs05.pdf) — register VM với fixed register count, tham khảo cho VM dispatch.
- [WebAssembly Specification](https://webassembly.github.io/spec/) — stack-based wire format (alternative đã reject).
- [SSA Book — Static Single Assignment Book](http://ssabook.gforge.inria.fr/latest/book.pdf) — lý thuyết SSA, phi nodes, dominance.
- [GHC Cmm](https://gitlab.haskell.org/ghc/ghc/-/wikis/commentary/compiler/cmm-type) — alternative C-- style IR cho functional language (defer reference).

## Liên quan

- [ADR-0005](0005-module-system.md) — Module system: `AbsolutePath` là input cho IR cross-module call.
- ADR-0008 (sẽ viết, v0.3.8): Bytecode binary format `.triv`.
- ADR-0009 (sẽ viết, v0.4): ABI metadata format — đọc từ IR.
- ADR-0012 (sẽ viết, v0.5): CAS hash scheme — `iface_hash` trên IR signature, `impl_hash` trên IR body.
- ADR-0014 (sẽ viết, v0.6): Capability type system — đọc capability tag từ IR.
- [VISION §4](../../VISION.md) — Mô hình thực thi multi-backend.
- [SPEC §0.3](../../SPEC.md) — Nguyên tắc thiết kế (AI-first, Stability over speed, Tam phân first-class).
- [ROADMAP § v0.3](../../ROADMAP.md) — Phase deliverables + gates.

---

*Quyết định này đóng băng IR shape cho Triết. Breaking change ở IR cần ADR riêng. Wire format binary detail (`.triv`) defer to ADR-0008 khi v0.3.8 bắt đầu.*
