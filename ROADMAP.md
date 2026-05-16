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

## Trạng thái hiện tại — v0.5 đã ship ✅

v0.3 ✅ (interpreter + VM + IR) → v0.3.x.cleanup ✅ → v0.3.x.ternary ✅
→ v0.4 ✅ (Crate-Pack + Stable ABI) → **v0.5** ✅ (CAS Packaging).

✅ Tree-walking interpreter end-to-end
✅ Type checker với inference + monomorphization
✅ Struct, enum + generics (G.1)
✅ Łukasiewicz Ł3 + Kleene K3
✅ Nullable subtyping `T ⊂ T?` (bẩm sinh tam phân, không bolt-on)
✅ Diagnostic format (miette, error codes E0000–E2399+)
✅ Module system: hierarchical namespace, explicit `public` export, dot paths, Python-style imports, cycle detection, visibility
✅ Bytecode VM: register SSA IR, 53-opcode dispatch (incl. WitnessCall), balanced ternary semantics
✅ Lowerer: AST → IR cho toàn bộ v0.2 features bao gồm SSA phi cho mutable vars (loops + if), enum payload + variant tag dispatch, tuple/literal pattern match, `.enumerate()` adapter, `?.` / `?:` / `!!` nullable ops, stdlib text builtins
✅ `.triv` binary format v3: ADR-0008 + ADR-0010 (BR_TRILEAN) + ADR-0012 (WITNESS_CALL + witness section)
✅ `BrTrilean` 3-way branch + strict `if` Unknown→panic + Ł3-aware Eq (ADR-0010)
✅ CLI `triet build foo.tri -o foo.triv` + `triet run foo.triv`
✅ Differential tests: **11/11 examples byte-identical VM vs interpreter** (gate ADR-0009 § A đạt)
✅ Benchmark harness: criterion, VM 1.26× interpreter (baseline)
✅ Cargo workspace `version = 0.4.0` đồng bộ với SPEC v0.4 (ADR-0009 § C)
✅ `cargo clippy --workspace --all-targets -- -D warnings` sạch (ADR-0009 § B)
✅ **Crate-Pack format** `.tripack` per ADR-0011 — ABI metadata + IR code section + dedicated linker section IDs
✅ **Witness table dispatch** per ADR-0012 — IR-level support, VM dispatch, `.triv` v3 wire format
✅ **Semver linking policy** per ADR-0013 — E2300-2399 decision matrix, refuse-to-link on major bump, iface_hash drift warnings
✅ **`triet-pack` crate**: write_tripack/read_tripack + plan_link, 26 unit + 7 integration tests
✅ **Stdlib `std.result`**: canonical `Result<T, E>` enum; SPEC §2.5 promotes `T?` as primary nullable
✅ 867 tests workspace-wide ở v0.4, → **918 tests ở v0.5** (0 ignored), snapshot tests cho IR + diagnostics
✅ **CAS Packaging** per ADR-0014/0015 — 3-cấp hash tree (term + module + pkg), package store `~/.triet/store/`, atomic install protocol, mark-and-sweep GC
✅ **Resolver + lockfile** — hash-pinned dep resolution, `triet.lock` line format
✅ **`triet store` CLI** — `import`, `list`, `gc` subcommands
✅ **Shared loading demo** — VISION §3.1 gate hit at iface level (term iface dedup proven; body-level RAM dedup queued behind lowerer per-term split)
✅ **Cross-module enum variant import** — `from std.result import Ok, Err` closed pre-existing v0.2.x gap; E2107 cho aliased variant import
🔜 Tiếp theo: v0.6 — Capability System (`sys.*` / `dev.*` / `usr.*` enforce, Trit-level capability, Ł3 policy hook)

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
- ~~Strict `if`/`while` unknown-as-error check~~ → Closed in v0.3.x.ternary phase below.

---

## v0.3.x.ternary — Ternary-native IR ✅ SHIPPED

**Mục tiêu:** Audit sau cleanup phát hiện 5 chỗ binary-thinking leak ở IR:
BrIf 2-way (Unknown collapse to else), if/if? semantic distinction hardcoded
ở lowerer thay vì IR, EnumTag Trit chỉ dùng 2/3 states, Constant::Null
bolt-on, Eq trên Trilean::Unknown trả False thay vì Unknown. Tất cả violate
VISION §5 "Trit-level capability + Łukasiewicz checking + ternary ABI". Phase
này lock thiết kế tam phân-first ở IR level trước khi v0.4 ABI freeze.

**Quyết định kiến trúc:** [ADR-0010](docs/decisions/0010-ternary-native-ir.md)

**Đã ship:**

| Sub-task | Description | Commit |
|---|---|---|
| v0.3.x.ternary.1 | ADR-0010 — ternary-native IR design | `c944949` |
| v0.3.x.ternary.2 | `BrTrilean` opcode (0xB4) — 3-way branch | `6f00c0a` |
| v0.3.x.ternary.3+5 | Lowerer migrate to `BrTrilean` + strict `if` Unknown→panic (SPEC §7.1.1) | `09cc1e5` |
| v0.3.x.ternary.4 | Match dispatch + pattern test dùng `BrTrilean` (cùng commit trên) | — |
| v0.3.x.ternary.6 | `Eq`/`Ne` propagate Unknown khi operand Trilean::Unknown (Ł3) | `39b6cd6` |
| v0.3.x.ternary.7 | Document `Constant::Null` = Trit::Zero discriminator state | `3b1ef2f` |
| v0.3.x.ternary.8 | Verify gate: 11/11 + 838 tests + clippy clean | this commit |

**Backend mapping (per ADR-0010):**
- **JIT (Cranelift, v0.9)**, **LLVM AOT (v2.0)**: BrTrilean → 2 cmp + 2 branch on binary CPU.
- **Trytecode (v∞)**: BrTrilean → **1 native instruction** trên hardware tam phân. Đây là điểm Triết thắng vĩnh viễn nếu ngày phần cứng tam phân xuất hiện.

**`.triv` wire format bumped v1 → v2** — v1 readers gặp BR_TRILEAN opcode (0xB4) trả `UnknownOpcode` thay vì silently misinterpret.

**Lowerer state sau migration:** 0 emit BrIf, 7 emit BrTrilean. `BrIf` còn lại trong IR enum cho .triv v1 backward decode + cases binary-thực (Trit verified 2-state). Không có new code emit BrIf.

**Không làm:**
- Xoá `BrIf` enum variant (defer — wire format compat).
- Encoding ≥4-variant enum thành Tryte tag (defer — chưa example nào cần).
- Capability `Trilean` dispatch (v0.6 — sẽ build trên BrTrilean infrastructure).

---

## v0.4 — Crate-Pack + Stable ABI ✅ SHIPPED

**Mục tiêu:** Cho phép phân phối binary library, type-safe cross-package linking.

**Quyết định kiến trúc:**
- **[ADR-0011](docs/decisions/0011-abi-metadata-format.md)** — ABI metadata binary format. Hai cấp hash (iface_hash + impl_hash). BLAKE3. Section ID layout cho future-compat.
- **[ADR-0012](docs/decisions/0012-witness-table-dispatch.md)** — Witness table dispatch (Swift-style) cho cross-package generics. Hybrid: monomorphize intra-pkg, witness inter-pkg.
- **[ADR-0013](docs/decisions/0013-semver-linking-policy.md)** — Semver decision matrix. iface_hash là final arbiter. Auto-shim explicitly not promised.

**Đã ship:**

| Sub-task | Description | Commit |
|---|---|---|
| v0.4.1 | ADR-0011 ABI metadata format | `8e9cfce` |
| v0.4.2 | ADR-0012 Witness table dispatch | `d600f73` |
| v0.4.3 | ADR-0013 Semver linking policy | `c76b89c` |
| v0.4.4 | `triet-pack` crate + `.tripack` serde (11 round-trip tests) | `09b155d` |
| v0.4.5 | Cross-package linker + decision matrix (8 tests) | `b1f9f83` |
| v0.4.6 | `WitnessCall` opcode + `.triv` v3 wire format + VM dispatch | `8360036` |
| v0.4.7 | `std.result` + SPEC `T?` primary | `06d7129` |
| v0.4.8 | Cross-package demo (7 integration tests) | `5d61de9` |
| v0.4.9 | Verify gate + Cargo 0.4.0 + docs sync | this commit |

**Gate đạt (ADR-0009):**
- ✅ A — Functional: differential 11/11 byte-identical, 0 `#[ignore]`, 0 `TODO(v0.4...)`.
- ✅ B — Hygiene: 867 tests pass, clippy `-D warnings` clean, `cargo fmt` clean.
- ✅ C — Docs: SPEC v0.4, Cargo `0.4.0`, README updated, 3 ADRs landed.
- ✅ D — Self-consistency: 11/11 examples chạy interpreter + VM, demo cross-pkg pass.

**Không làm:**
- CAS hash identity (defer v0.5 — `iface_hash_pin` prep đã có trong dep table).
- Auto-shim ABI migration (rejected per VISION §3.3 — semantic change không decidable).
- Capability enforcement runtime (defer v0.6 — slot reserved trong ABI metadata).
- CLI `triet link` subcommand (defer v0.5 — API trong `triet-pack` là contract).
- Cross-module enum variant import (`from std.result import Ok, Err`) — pre-existing gap from v0.2.x; ADR text khuyến nghị, implementation ở v0.5.
- Cross-package generic lowerer emit (lowerer chỉ emit `CallCrossModule` ở v0.4; full `WitnessCall` emit cross-package lands ở v0.5 với multi-package compile).

---

## v0.5 — CAS Packaging ✅ SHIPPED

**Mục tiêu:** Định danh package bằng hash, eliminate DLL Hell, prep parallel versions ở RAM level.

**Quyết định kiến trúc:**
- **[ADR-0014](docs/decisions/0014-hash-scheme-refinement.md)** — Hash scheme refinement: 3-cấp hash tree (term + module + package), `abi_version` 1 → 2, domain separation per level.
- **[ADR-0015](docs/decisions/0015-package-store-layout.md)** — Package store layout: `~/.triet/store/{term,mod,pkg,names,roots,tmp}/`, atomic install via tmp + rename, mark-and-sweep GC.

**Đã ship:**

| Sub-task | Description | Commit |
|---|---|---|
| v0.5.1 | ADR-0014 hash scheme refinement | `f876006` |
| v0.5.2 | ADR-0015 package store layout | `f7b49c8` |
| v0.5.3 | 3-cấp hash tree implementation + abi_version 1 → 2 | `b6d170c` |
| v0.5.4 | Package store filesystem + atomic install + GC | `2425e25` |
| v0.5.5 | Hash-based resolver + `triet.lock` format | `2c43e69` |
| v0.5.6 | Shared loading demo + term dir keyed by impl_hash | `6291bc1` |
| v0.5.7 | `triet store {import,list,gc}` CLI | `8b4ce12` |
| v0.5.8 | Cross-module enum variant import (`from X import Variant`) | `07323a1` |
| v0.5.9 | Verify gate (ADR-0009) + bump 0.4.0 → 0.5.0 + docs sync | this commit |

**Gate đạt (ADR-0009):**
- ✅ A — Functional: differential 11/11 byte-identical, 0 `#[ignore]`, 0 `TODO(v0.5...)`.
- ✅ B — Hygiene: 918 tests pass, clippy `-D warnings` clean, `cargo fmt` clean.
- ✅ C — Docs: SPEC v0.5, Cargo `0.5.0`, README updated, 2 ADRs landed (0014, 0015).
- ✅ D — Self-consistency: 11/11 examples chạy interpreter + VM, store CLI smoke OK, variant import e2e OK.

**Không làm (defer khỏi v0.5):**
- **Lowerer emit `WitnessCall` cho cross-package generics** (Item 2 carry-over) — cần package-aware lowering, multi-week milestone. Reschedules cùng multi-package compile path hoặc v0.7 self-hosting.
- **v=1 `.tripack` lossy migration** (ADR-0015 §9) — hiện chưa có v=1 packs trong wild; lands on demand.
- **Body-level RAM dedup** (`term/<hash>/body.bin`) — chờ lowerer per-term IR body split. Iface-level dedup proven; body-level deferred to v0.6+ alongside lowerer work.
- **Distributed registry / network fetch** — local store đủ; defer v1.0+.
- **Auto-GC** — manual `triet store gc` đủ; "refuse over guess" policy.

---

## v0.5.x.review — Pre-v0.6 audit fixes ✅ SHIPPED

Audit window trước v0.6. 1 binary leak + 3 testing gap được bít. Không
thay đổi spec gốc; thêm Addendum vào ADR-0015. 918 → 924 tests.

| Sub-task | Description | Commit |
|---|---|---|
| v0.5.x.review.1 | `Resolution.from_lockfile: bool` → `ResolutionOrigin { Lockfile, IfacePin, Fresh }` 3-state enum | `20076d5` |
| v0.5.x.review.2 | Concurrent install race + E2382 negative + GC corrupt-manifest conservative (+ `GcReport.corrupt_pkgs`) | `d7f1beb` |
| v0.5.x.review.3 | `$TRIET_STORE` fallback chain + multi-root GC invariant tests | `b167717` |
| v0.5.x.review.4 | ADR-0015 Addendum + sync docs | this commit |

**Trigger:** Audit của tôi (AI) trước khi mở v0.6. Author chấp nhận
"hãy fix tất cả" sau khi review findings — duy trì cadence
*proactive tech-debt audit trước version freeze*.

---

## v0.6 — Capability System (`sys.` / `dev.` / `usr.`) ✅ SHIPPED

Trụ cột bản sắc #5 (VISION §3.5 + §5). Capability is a namespace
attribute declared in the per-package `triet.package` manifest;
runtime `Defer` slots resolve via `triet.policy` rules + optional
TTY prompt.

**3 ADRs locked + 1 Addendum:**

| ADR | Title | Status |
|---|---|---|
| [0016](docs/decisions/0016-capability-type-system.md) | Capability type system (namespace + manifest, Trit + Trilean::Unknown) | Locked |
| [0017](docs/decisions/0017-trilean-policy-hook.md) | Trilean policy hook protocol (`triet.policy` rules + TTY fallback) — *+ Addendum: parser strict + `/dev/tty` source + Abstain errata* | Locked |
| [0018](docs/decisions/0018-capability-loader-semantics.md) | Loader semantics (`triet.package` grammar, eager Step 6a refuse, TTY provenance prompt, `CapabilityClaim`) | Locked |

**11 sub-tasks (v0.6.1–v0.6.11):**

| Sub-task | Description | Commit |
|---|---|---|
| v0.6.1 | ADR-0016 land | `cd65127` |
| v0.6.2 | ADR-0017 land | `0e6e94a` |
| v0.6.2.addendum | ADR-0017 Addendum (parser strict + `/dev/tty` + Abstain errata) | `d6d0aa3` |
| v0.6.3 | ADR-0018 land | `6742948` |
| v0.6.4 | `CapabilityClaim` + `CapabilityLevel` + wire format (ADR-0016 §4) | `22151a4` |
| v0.6.5 | `triet.package` parser (ADR-0018 §1) | `cb8aa7b` |
| v0.6.6 | `triet.policy` parser + shared `strict_parser` (ADR-0017 §3) | `2a3a6c6` |
| v0.6.7 | Type-check cross-root cap check (ADR-0016 §5 rules 1+2) | `b41d47e` |
| v0.6.8 | Link-time cap check (ADR-0018 §2 Step 6a) | `24c34c3` |
| v0.6.9 | Runtime resolver + per-session cache (ADR-0017 §4, ADR-0018 §2 Step 6b) | `6151399` |
| v0.6.10 | TTY prompt UX (`/dev/tty` + provenance + G/D permanent write, ADR-0018 §4 + ADR-0017 Addendum §B) | `40f8cf4` |
| v0.6.11 | Demo + capability pipeline integration test | this commit |

**Gate đạt:**
- ✅ Compile-time E2200/E2201 fire khi `usr.*` imports `dev.*`/`sys.*` không cap claim — proven by [`compile_*` tests in `capability_pipeline.rs`](crates/triet-typecheck/tests/capability_pipeline.rs).
- ✅ Runtime policy hook resolves `Trilean::Unknown` via `triet.policy` rules + TTY prompt — proven by `resolve_*` tests.
- ✅ Demo capability-restricted program: accept path + refuse path both proven by `full_pipeline_capstone_*` tests + `demos/04-capability-system/` illustrative files.
- ✅ E22XX namespace fully populated: E2200–E2208 (+ sub-variants) across parse/compile/link/runtime stages.
- ✅ 924 → 1079 tests, clippy `-D warnings` clean, `abi_version` stays `2` (ADR-0016 §4 promise honored).

**Không làm (defer khỏi v0.6):**
- **CLI wiring** (`triet check` reading `triet.package` from project root, cap-aware build pipeline emitting `.tripack` with caps section populated, loader integration with `DevTtyPrompt`) — needs project-layout discovery convention; lands cleaner with v0.7 self-hosting.
- **E2208.PreV06Reader** — gated by future `abi_version` bump.
- **E2208.CapabilityDivergence** — fires when lowerer actually populates caps section from `triet.package`; defer with lowerer work.
- **Per-function cap granularity** — defer post-v1.0 (ADR-0016 "Không làm").
- **Wildcard claims** in manifest — refuse-over-guess (ADR-0016 "Không làm").
- **Windows ConPTY** for TTY prompt — POSIX-first; Windows defer.
- **ANSI colour theming** + box-drawing Unicode in TTY prompt — usability win, defer post-security-floor.
- **`Capability<T>` stdlib type** (old roadmap wording) — superseded by namespace-level claims (ADR-0016 §1 picked phương án C over A/B/D).
- Hardware enforcement (cần phần cứng tam phân hoặc bytecode VM sandbox).
- Distributed capability (defer v1.0+).

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

**Quyết định:** ADR ở giai đoạn vào v0.8 (next ADR-NNNN khi land). Hiện tại favor **Actor + structured concurrency** vì alignment với capability model.

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
