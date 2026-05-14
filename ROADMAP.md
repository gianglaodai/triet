# Triết — Roadmap

> Lộ trình từ interpreter v0.2 tới ngôn ngữ-OS v3.0+.
> **Nguyên tắc:** *stability over speed.* Mỗi version mở khóa version sau.

Xem tầm nhìn dài hạn ở [`VISION.md`](VISION.md).

---

## Triết lý phasing

1. **Mỗi phase có gate rõ ràng.** Không bắt đầu phase N+1 nếu phase N chưa pass gate.
2. **Quyết định kiến trúc có ADR.** Mỗi trụ cột lớn = một ADR ở `docs/decisions/`.
3. **Test bao phủ trước khi tiến.** Workspace tests phải xanh và sample programs chạy được trước khi bump version.
4. **Không skip phase.** v0.5 (CAS) không thể làm trước v0.3 (bytecode) vì hash AST chưa ổn định.

---

## Trạng thái hiện tại — v0.3 đã ship ✅ + cleanup gate ADR-0009 ✅

✅ Tree-walking interpreter end-to-end
✅ Type checker với inference + monomorphization
✅ Struct, enum + generics (G.1)
✅ Łukasiewicz Ł3 + Kleene K3
✅ Nullable subtyping `T ⊂ T?` (bẩm sinh tam phân, không bolt-on)
✅ Diagnostic format (miette, error codes E0000–E2200+)
✅ Module system: hierarchical namespace, explicit `public` export, dot paths, Python-style imports, cycle detection, visibility
✅ Bytecode VM: register SSA IR, 52-opcode dispatch, balanced ternary semantics
✅ Lowerer: AST → IR cho toàn bộ v0.2 features bao gồm SSA phi cho mutable vars (loops + if), enum payload + variant tag dispatch, tuple/literal pattern match, `.enumerate()` adapter, `?.` / `?:` / `!!` nullable ops, stdlib text builtins
✅ `.triv` binary format: ADR-0008, serializer/deserializer (24 round-trip tests)
✅ CLI `triet build foo.tri -o foo.triv` + `triet run foo.triv`
✅ Differential tests: **11/11 examples byte-identical VM vs interpreter** (gate ADR-0009 § A đạt)
✅ Benchmark harness: criterion, VM 1.26× interpreter (baseline)
✅ Cargo workspace `version = 0.3.0` đồng bộ với SPEC v0.3 (ADR-0009 § C)
✅ `cargo clippy --workspace --all-targets -- -D warnings` sạch (ADR-0009 § B)
✅ 835 tests workspace-wide, 0 ignored, snapshot tests cho IR + diagnostics
🔜 Tiếp theo: v0.4 — Crate-Pack + Stable ABI

---

## v0.2.x — Module system cơ bản ✅ SHIPPED

**Mục tiêu:** Cho phép tách codebase thành nhiều file/module với hierarchical namespace, explicit export.

**Quyết định kiến trúc:** [`docs/decisions/0005-module-system.md`](docs/decisions/0005-module-system.md)

**Đã ship:**
- Verbose keywords: `function`, `public`, `public(package)`, `mutable`, `constant`, `module` (ADR-0005).
- Dot path syntax: `crate.foo.bar`, `self.x`, `super.y` (không `::`).
- Python-style imports: `from std.io import println, print as p` + `import std.io.println`; glob bị reject.
- 3-level visibility: `public`, `public(package)`, default private — enforced ở name resolver.
- File resolution: flat (`foo.tri`) → nested fallback (`foo/foo.tri`); inline `module foo { … }` ≡ file-bound cho path resolution.
- Multi-arena `ResolvedProgram`: một arena per parsed file, tránh cross-file ID remapping.
- Cyclic import detection (E2100) với cycle trace `foo → bar → baz → foo`.
- Diagnostic codes E2100–E2106 cho loader/resolver, tất cả implement `miette::Diagnostic`.
- Reserved namespace roots: `std`, `sys`, `dev`, `usr`, `core`, `crate`, `self`, `super` (chưa enforce capability).
- Stdlib reorganized as real filesystem files in `std/` directory: `std.io`, `std.text`, `std.assert`.
- 704-dòng ternary ALU demo across 6 modules (file-bound + nested + inline) — exercises every feature.
- Symbolic operators preferred convention (`!`, `&&`, `||`, `^`, `=>`, `~>`, `~^`, `<=>`, `<~>`); keyword forms vẫn valid.

**Sub-task changelog (per-step commits):**

| Sub-task | Description | Commit |
|---|---|---|
| v0.2.x.0 | SPEC.md align với VISION (5 pillars + OS-capable) | — |
| v0.2.x.1 | Drop SIMD/Tensor/DType §10.5 từ SPEC | — |
| v0.2.x.2 | Visibility AST + parser capture (3 levels) | — |
| v0.2.x.3 | Verbose keyword sweep + dot path commitment | — |
| v0.2.x.3.1 | Post-sweep drift cleanup (ADR-0005, SPEC, README) | `fa89622` |
| v0.2.x.4 | Reserved namespace roots validation | — |
| v0.2.x.5 | Module decls + Python import syntax (parser-only) | `e6e7e51` |
| v0.2.x.6 #36.1 | Scaffold `triet-modules` crate | `35dc88f` |
| v0.2.x.6 #36.2 | File loader + multi-arena `ResolvedProgram` | (bundled) |
| v0.2.x.6 #36.3 | Cycle detection (E2100) | `28b0ca3` |
| v0.2.x.6 #36.4 | Name resolution + visibility check | `135342c` |
| v0.2.x.6 #36.5 | Typecheck integration | `075db7d` |
| v0.2.x.6 #36.6 | Interpreter integration | `9b0687c` |
| v0.2.x.6 #36.7 | CLI rewire through loader + integration tests | `5634613` |
| v0.2.x.7 v1 | Stdlib embedded module tree (synthetic) | `d1f1698` |
| v0.2.x.7 v2 | Stdlib reorganize as real filesystem files | `befc59c` |
| v0.2.x.8 v1 | Demo lớn (ALU) + snapshot tests cho E2100–E2103 | `b9d1d0c` |
| v0.2.x.8 v2 | Module system demo snapshot tests (11 modules, 3 tests) | `e356a61` |

**Không làm (deferred theo phase sau):**
- Capability enforcement (đợi v0.6).
- Cross-package linking (đợi v0.4).
- Signature files riêng (compiler tự suy ra từ source).

**Gate đã đạt:** Demo 704-dòng chia thành 6 module type-check + run đúng; visibility/cycle/file-not-found diagnostics chỉ rõ vị trí lỗi; 700+ tests xanh.

---

## v0.3 — Bytecode VM + Stable IR ✅ SHIPPED

**Mục tiêu:** Thiết kế và lock **Triết IR** — biên giới ngôn ngữ ↔ phần cứng. Bytecode VM ở phase này là **development tier scaffolding**, không phải production runtime. Production target nhị phân là AOT native (LLVM, v2.0); production target tam phân là trytecode native (v∞). Xem [VISION §4](VISION.md) cho execution model multi-backend.

**Quyết định kiến trúc:**
- **[ADR-0007](docs/decisions/0007-ir-design.md)** — IR là **register-based, SSA form, virtual register vô hạn, type-tagged per register**.
- **[ADR-0008](docs/decisions/0008-triv-binary-format.md)** — Bytecode binary format `.triv`: magic bytes, version field, section layout, LEB128 varint, little-endian.

**Đã ship:**

| Sub-task | Description | Commit |
|---|---|---|
| v0.3.0 | ADR-0007: IR design — register-based SSA | `abbd1d9` |
| v0.3.1 | Scaffold `triet-ir` crate (types, instr, constant, module, verify, display) | `abbd1d9` |
| v0.3.2 | Lowerer: AST → IR (core expressions + statements, 51 tests) | `2c80c2d` |
| v0.3.3 | Lowerer: items + functions + modules (merged into v0.3.2) | — |
| v0.3.4 | Lowerer: aggregates + match + closures (merged into v0.3.2) | — |
| v0.3.5 | VM: bytecode execution (52 opcodes, 20 tests, balanced ternary) | `cef4119` |
| v0.3.6 | Snapshot tests: IR display output (4 tests, insta) | `0ee2bb9` |
| v0.3.7 | Differential tests: VM vs interpreter (3/11 pass, 8 deferred) | `2c57a50` |
| v0.3.8 | ADR-0008: .triv bytecode binary format | `117c20d` |
| v0.3.9 | Serialize/deserialize: .triv reader/writer (24 tests) | `52cee51` |
| v0.3.10 | CLI: `triet build` + .triv execution + VM CallCrossModule | `3b94bbf` |
| v0.3.11 | Benchmark harness (criterion) + BENCHMARKS.md | `4dab69a` |

**Lowerer f-string + for-loop fixes (v0.3.7 cycle):**
- FStringConcat builtin (instr.rs, vm.rs, serde.rs, display.rs)
- For loop with phi-based SSA loop variable
- VM path_index for cross-module dispatch + path_to_builtin fallback

**Gate đã đạt (after v0.3.x.cleanup):**
- ✅ IR spec (ADR-0007) + bytecode format `.triv` có version field (ADR-0008).
- ✅ Differential tests: **11/11 examples byte-identical VM vs interpreter** (closed under v0.3.x.cleanup phase).
- ⚠️ Bench: VM 1.26× interpreter trên factorial (3× gate → defer cho v0.4 performance pass, ghi rõ ở BENCHMARKS.md).
- ✅ IR snapshot tests detect regression khi đổi lowerer (4 tests).

**Không làm trong v0.3:**
- JIT (v0.9), Native AOT (v2.0), Trytecode backend (v∞).
- ABI metadata trong `.triv` (v0.4).
- Cross-package linking (v0.4).

---

## v0.3.x.cleanup — Gate-closing phase ✅ SHIPPED

**Mục tiêu:** Đóng đầy đủ gate cho v0.3 trước khi mở v0.4. Lock policy bằng
[ADR-0009](docs/decisions/0009-version-gate-policy.md) — gate này áp dụng cho
mọi version bump tương lai, không chỉ v0.3 → v0.4.

**Đã ship:**

| Sub-task | Description | Commit |
|---|---|---|
| v0.3.x.cleanup.1 | ADR-0009 — version gate policy | `6a8a6b1` |
| v0.3.x.cleanup.2 | Bump Cargo workspace 0.1.0 → 0.3.0 + `triet info` | `b86b0be` |
| v0.3.x.cleanup.3 | README.md sync với v0.3 status + workspace structure | `a3df90f` |
| v0.3.x.cleanup.4 | Clippy `-D warnings` sạch (109 → 0 warnings) | `84fea6c` |
| v0.3.x.cleanup.5 | Enum payload + variant tag dispatch (maybe, generic) | `e3726c0` |
| v0.3.x.cleanup.6 | SSA loop+if phi cho mutable vars (counter, while_polling, long_arithmetic) | `2ddd046` |
| v0.3.x.cleanup.7 | Iterator `.enumerate()` + nullable ops `?.` `?:` `!!` + stdlib text builtins (nullable, enumerate) | `be9c0c5` |
| v0.3.x.cleanup.8 | Tuple + literal pattern match (fizzbuzz) | `251f954` |

**Gate đã đạt (ADR-0009):**
- ✅ Gate A — Functional: 11/11 differential, 0 `#[ignore]`, 0 `TODO(v0.3...)`.
- ✅ Gate B — Hygiene: 835 tests pass / 0 fail / 0 ignored; clippy `-D warnings` sạch.
- ✅ Gate C — Docs: SPEC v0.3, Cargo 0.3.0, `triet info` đồng bộ, README cập nhật, ADRs 0001–0009.
- ✅ Gate D — Self-consistency: 11/11 examples chạy interpreter & VM, demo 6-file module-system chạy.

**Không làm:**
- Bench 3× (defer cho v0.4 perf pass; ADR-0010 nếu cần revise gate).
- Strict `if`/`while` unknown-as-error check (defer — yêu cầu `trilean_assert_known` opcode).

---

## v0.4 — Crate-Pack + Stable ABI

**Mục tiêu:** Cho phép phân phối binary library, type-safe cross-package linking.

**Deliverables:**
- Crate-Pack format: file nhị phân kèm metadata (`.tripack`).
- Metadata gồm: ABI signatures, dependency declarations, capability claims (placeholder).
- Compiler refuse-to-link với diagnostic khi major-version mismatch.
- Witness table dispatch cho generics qua biên crate-pack.
- Monomorphization vẫn dùng intra-package (tối ưu).
- Linker: hybrid (static cho hot path, dynamic cho shared libs).
- Result/Option đầy đủ trong stdlib (sử dụng generic của G.1).

**Không làm:**
- CAS hash identity (đợi v0.5).
- Auto-shim ABI migration (rejected — không khả thi general case).
- Capability enforcement runtime (đợi v0.6).

**Gate:**
- Phân phối được 1 stdlib package + 1 user package qua crate-pack format.
- Sửa minor version của lib không cần rebuild app.
- Sửa major version mà không update app → linker error rõ ràng.

**ADR cần viết:** ADR-0009 (ABI metadata format), ADR-0010 (witness table dispatch), ADR-0011 (semver linking policy).

---

## v0.5 — CAS Packaging

**Mục tiêu:** Định danh package bằng hash, eliminate DLL Hell, parallel versions.

**Deliverables:**
- Hash scheme: hai cấp `iface_hash` (ABI surface) + `impl_hash` (toàn nội dung).
- Package store: filesystem layout `~/.triet/store/<hash>/`.
- Resolver: `use foo::bar` → resolve theo hash trong manifest.
- Manifest format: lockfile có hash đầy đủ.
- Shared loading: 2+ apps cùng dùng `std@hash_X` → load 1 lần.
- Migration tool: import package từ filesystem path → CAS store.

**Không làm:**
- Distributed registry (defer — local store đủ cho v0.5).
- Garbage collection (defer — manual `triet store gc` đủ).

**Gate:**
- Chạy song song 2 phiên bản incompatible của cùng package trong cùng app, không xung đột.
- Sửa internal của lib không invalidate `iface_hash` → app không rebuild.

**ADR cần viết:** ADR-0012 (hash scheme), ADR-0013 (package store layout).

---

## v0.6 — Capability System (`sys::` / `dev::` / `usr::`)

**Mục tiêu:** Enforce isolation ở compiler level. Đây là trụ cột bản sắc của Triết.

**Deliverables:**
- Top-level namespaces `sys`, `dev`, `usr` enforced.
- `Capability<T>` type ở stdlib: `Capability<Sys::Net>`, `Capability<Dev::Disk>`...
- Trit-level capability: `Trit` hoặc `Trilean` làm capability level.
  - `+1` (Grant) — explicit grant.
  - `0` (Ambient) — inherit từ caller.
  - `-1` (Deny) — compile error nếu cố use.
  - `Trilean::Unknown` — runtime policy resolves.
- Capability propagation rules ở type checker.
- Crate-pack metadata khai báo capability requirements.
- Loader runtime: refuse-to-load nếu không cấp đủ capability.
- Demo: minimal "kernel-style" program với `usr::app` không thể chạm `dev::*`.

**Không làm:**
- Hardware enforcement (cần phần cứng tam phân hoặc bytecode VM sandbox — defer).
- Distributed capability (defer).

**Gate:**
- Compile-time error rõ ràng khi `usr::*` import `dev::*` không có capability.
- Runtime policy hook hoạt động cho `Trilean::Unknown`.
- Demo capability-restricted program chạy được + bị reject khi capability sai.

**ADR cần viết:** ADR-0014 (capability type system), ADR-0015 (Trilean policy hook), ADR-0016 (loader semantics).

---

## v0.7 — Self-hosting Compiler

**Mục tiêu:** Compiler Triết viết bằng Triết. Bootstrap đầy đủ.

**Deliverables:**
- Lexer, parser, typecheck, IR generator viết lại bằng Triết.
- Bootstrap chain: Rust-compiler-v0.6 → Triết-compiler-v0.7 → Triết-compiler-v0.7 (self-build).
- Performance parity với Rust impl trong vòng 2×.

**Không làm:**
- Compile thẳng sang native (vẫn xuất bytecode v0.3).

**Gate:** Bit-identical bootstrap qua 2 vòng tự build.

---

## v0.8 — Concurrency Model

**Mục tiêu:** Triết có model concurrency riêng, có lý thuyết.

**Lựa chọn cần đánh giá ở phase này:**
- Actor model (Pony, Erlang) — tự nhiên ăn nhập với capability namespace.
- Async/await (Rust) — quen thuộc.
- CSP / channels (Go) — đơn giản.
- Structured concurrency (Trio, Project Loom) — modern.

**Quyết định:** ADR-0017 ở giai đoạn vào v0.8. Hiện tại favor **Actor + structured concurrency** vì alignment với capability model.

**Gate:** Demo concurrent program chạy đúng dưới load + race detector pass.

---

## v0.9 — JIT (Cranelift)

**Mục tiêu:** Bytecode VM có JIT tier cho hot code paths.

**Deliverables:**
- Tier 1: bytecode interpreter (v0.3).
- Tier 2: Cranelift JIT cho function chạy thường xuyên (profile-guided).
- AOT cache: lần chạy thứ 2 dùng JIT-output cached.

**Gate:** Bench ≥10× so với v0.3 bytecode trên numeric-heavy programs.

---

## v1.0 — Production Stability

**Mục tiêu:** Đóng băng spec ngôn ngữ ở tầng v1.0. Backwards-compat policy có hiệu lực.

**Deliverables:**
- SPEC.md đóng băng (chỉ thêm, không phá ngữ nghĩa cũ).
- Stable ABI vĩnh viễn cho v1.x.
- LTS branch.
- Toolchain installer (giống `rustup`).
- Documentation đầy đủ + tutorial sách-style.

**Gate:**
- 100+ public crate-pack ngoài stdlib.
- 3+ ứng dụng production thực.

---

## v2.0 — Native AOT Compile (LLVM)

**Mục tiêu:** Sản xuất binary native cho x86-64, ARM64, RISC-V.

**Deliverables:**
- Backend LLVM (Cranelift đã quen từ v0.9 — LLVM cho production codegen).
- Cross-compile toolchain.
- Debug symbol format (DWARF compat).
- FFI ổn định với C ABI (cho legacy interop).

**Gate:** Triết-compiled binary perf ≥80% so với equivalent Rust binary.

---

## v3.0 — Microkernel POC

**Mục tiêu:** Chứng minh Triết viết được OS. Đây là **mục tiêu lý tưởng** đặt cọc tầm nhìn.

**Deliverables:**
- Microkernel boot trên x86-64 (QEMU đầu tiên, sau đó hardware thực).
- `sys::` namespace là syscall thực.
- `dev::` driver tối thiểu (UART, disk, ethernet).
- `usr::` chạy được 1–2 ứng dụng (shell + 1 demo).
- Capability enforcement runtime ở loader.

**Gate:**
- Boot tới shell prompt.
- App `usr::*` không thể chạm hardware không có capability — kernel test xác nhận.

**Đây không phải production OS.** Đây là chứng minh: ngôn ngữ Triết có đủ năng lực ngữ nghĩa và ABI để implement OS. Production OS là dự án riêng.

---

## v∞ — Phần cứng tam phân

**Khi phần cứng tam phân xuất hiện** (Setun-style modern, hoặc memristor-based ternary):
- Backend native cho ternary CPU (không phải emulate trên binary).
- Trit là unit hardware thực.
- Discriminator của `T?` là một trit thật, không phải bit-packed.

Triết đã sẵn sàng — semantics tam phân đã có từ v0.1, không cần thay đổi ngôn ngữ. Chỉ cần backend codegen mới.

---

## Decision log: Cái KHÔNG làm và lý do

| Đề xuất | Quyết định | Lý do |
|---|---|---|
| Java-style strict filesystem mapping | **Reject** | Java đã từ bỏ với JPMS. Refactor unfriendly. |
| Auto-shim ABI migration | **Reject** | Decidable detection nhưng undecidable adaptation. Misleading promise. |
| Glob imports `use foo::*` | **Reject ở v0.2.x** | Phá ABI clarity. Có thể revisit v1.0+. |
| GC | **Reject** | Triết là system language. Memory model tham khảo Rust borrow checker (defer thiết kế đến v0.4+). |
| Macro hệ thống lớn (Rust-style) | **Defer to v0.7+** | Tăng surface area của ngôn ngữ. Compile-time const eval ưu tiên hơn. |
| Backwards-compat shim cho v0.x | **No** | Trước v1.0, breaking changes free. Sau v1.0, bound chặt. |
| Phát minh CAS scheme riêng | **No** | Dùng prior art (Unison/Nix-inspired), bất biến + Triết-specific 2 cấp hash. |

---

## Pace expectations

| Phase | Realistic timeline (small team) |
|---|---|
| v0.2.x | 1–3 tháng |
| v0.3 (bytecode) | 6–12 tháng |
| v0.4 (ABI) | 6–12 tháng |
| v0.5 (CAS) | 4–8 tháng |
| v0.6 (capability) | 8–12 tháng |
| v0.7 (self-host) | 12+ tháng |
| v0.8–v0.9 | 6–12 tháng mỗi phase |
| v1.0 | tích lũy, release window |
| v2.0 (LLVM) | 12+ tháng |
| v3.0 (kernel) | 24+ tháng |

**Tổng:** 5–10 năm cho v3.0 với một team nhỏ hoặc 1 người. Dự án được scale theo realistic, không hứa hẹn.

> Stability over speed. Đây là tính năng.
