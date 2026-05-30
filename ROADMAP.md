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

## Trạng thái hiện tại — v0.8 đã ship ✅

v0.3 ✅ → v0.4 ✅ → v0.5 ✅ → v0.6 ✅ (Capability System) → **v0.7** ✅ (Self-hosting Compiler) → **v0.8** ✅ (Ownership Foundation + BYOS Concurrency Primitives). Post-release `v0.8.x.review` audit fixes ADR-0009 gate B leftover + namespace mistag + self-host lexer port + doc drift.

✅ Tree-walking interpreter + Bytecode VM (register SSA IR, 53 opcodes incl. `BrTrilean` + `WitnessCall`), `.triv` wire format v5 (ADR-0008 + 0010 + 0012 + 0020)
✅ Type checker với inference + monomorphization + Trilean! refinement (ADR-0021)
✅ Outcome error handling — `T~E` / `T?~E` trit-encoded, `~+`/`~0`/`~-` constructors + `~?`/`~:` postfix ops (ADR-0020)
✅ Łukasiewicz Ł3 + Kleene K3 + symbolic + keyword operator forms
✅ Module system: hierarchical namespace, dot paths, Python-style imports, multi-arena `ResolvedProgram` (ADR-0005)
✅ Crate-Pack `.khi` + cross-package linker với semver decision matrix (ADR-0011/0012/0013)
✅ CAS Packaging — 3-cấp hash tree (term + module + pkg), package store `~/.triet/store/`, atomic install, mark-and-sweep GC, `dao.lock` hash-pinned (ADR-0014/0015)
✅ Capability System — `sys.*`/`dev.*`/`usr.*` 4-state level, `dao.package` + `dao.policy` + `/dev/tty` prompt, E22XX namespace (ADR-0016/0017/0018)
✅ Self-hosting Compiler — `compiler/` 7 `.tri` files (~23K LOC), 3-stage bootstrap chain Stage 1 (Rust) → Stage 2 → Stage 3 byte-identical; main.tri convergence gate `#[ignore]`'d due to VM dev tier, lifts v0.9 (ADR-0019)
✅ S6 Ownership Model — 5-form reference `&+`/`&0`/`&-`/`&` + `owned`, `ObjectHeader` 8-byte binary header với refcount atomic ops, lexer + parser + AST + type-system resolve transparently (ADR-0022)
✅ Concurrency Primitives (BYOS) — Send derivation cho 13 type categories, E2500 fires, capability gates extended với `sys.raw_thread`/`sys.atomic`/`dev.ffi`/etc., Atomic placeholder shipped (ADR-0026 v2)
✅ Borrow Checker skeleton + Diagnostic format AI-first — E24XX namespace (E2400/E2402-E2403/E2410-E2411/E2420-E2422/E2430/E2440) + E25XX (`triet::actor::E2500/E2510/E2520`), `[Fix N]` numbered blocks per ADR-0025/0027. Enforcement defer v0.9
✅ Cargo workspace `version = 0.8.0`, SPEC header v0.8 (S6 §10 + Outcome §1.5.3 + Trilean! locked)
✅ Differential tests: 12/12 v0.7.4.2 examples byte-identical VM vs interpreter; `outcome_propagate.tri` VM-only per ADR-0019 Addendum §A7
✅ `cargo clippy --workspace --all-targets -- -D warnings` sạch + `cargo fmt --all --check` sạch (post-v0.8.x.review.1)
✅ **1425 tests workspace-wide** (3 `#[ignore]` documented per ADR-0019 §7 perf gate)
🔄 v0.9 — wide-phased (in progress): v0.9.0 design ADRs ✅ + **v0.9.x.atomic ✅ shipped 2026-05-30** (Atomic primitive full impl + borrow expression syntax + E2420 enforcement; 1506 tests). Remaining: v0.9.x.jit (Cranelift) + v0.9.final (Stage 2 ≡ Stage 3 main.tri gate lift to CI).

---

## v0.2.x — Module system cơ bản ✅ SHIPPED

**Mục tiêu:** Tách codebase thành nhiều file/module với hierarchical namespace, explicit export. Final: **700+ tests**.

**ADRs:** [ADR-0005](docs/decisions/0005-module-system.md) — verbose keywords (`function`/`module`/`mutable`/`constant`/`public`), dot paths (`crate.foo.bar`), Python-style imports (`from std.io import println`), 3-level visibility, multi-arena `ResolvedProgram`, reserved namespace roots, stdlib as filesystem files. Locked: single-file = crate root (Python/Go); inline ≡ file-bound for path resolution.

**Shipped:** Cyclic import detection (E2100), E2100-E2106 loader/resolver namespace, symbolic operators preferred (`!`/`&&`/`||`/`^`/`=>`/`~>`/`~^`/`<=>`/`<~>`), 704-dòng ternary ALU demo (6 modules). Detail prose: [docs/ARCHITECTURE.md §Module system](docs/ARCHITECTURE.md#module-system-shipped-v02x-adr-0005-locked).

**Không làm (defer):**
- Capability enforcement → v0.6.
- Cross-package linking → v0.4.
- Signature files riêng — compiler tự suy ra từ source.

Commit log: `git log --oneline --grep="v0\.2\.x"`.

---

## v0.3 — Bytecode VM + Stable IR ✅ SHIPPED

**Mục tiêu:** Lock **Triết IR** — biên giới ngôn ngữ ↔ phần cứng. VM ở phase này là **development tier scaffolding** per [VISION §4.3](VISION.md); production target AOT (v2.0) + trytecode (v∞). Final: **835 tests** (after v0.3.x.cleanup).

**ADRs:** [ADR-0007](docs/decisions/0007-ir-design.md) — register-based SSA IR, virtual register vô hạn, type-tagged per register. [ADR-0008](docs/decisions/0008-triv-binary-format.md) — `.triv` binary format (magic bytes + version + section layout + LEB128 varint, little-endian).

**Shipped:** `triet-ir` crate (lowerer + 52-opcode VM + serde), `.triv` v1, CLI `dao build` + `dao run`, criterion benchmarks (VM 1.26× interpreter baseline, 3× gate deferred to v0.4 perf pass). Differential tests 11/11 byte-identical closed under v0.3.x.cleanup. Detail: [docs/ARCHITECTURE.md §IR + bytecode VM](docs/ARCHITECTURE.md#ir--bytecode-vm-shipped-v03-adr-000700080010).

**Không làm (defer):**
- JIT → v0.9 Cranelift.
- Native AOT → v2.0 LLVM.
- Trytecode → v∞.
- ABI metadata trong `.triv` → v0.4.

Commit log: `git log --oneline --grep="v0\.3"` (excluding `.cleanup`/`.ternary`).

---

## v0.3.x.cleanup — Gate-closing phase ✅ SHIPPED

Đóng đầy đủ ADR-0009 gate cho v0.3 trước khi mở v0.4. Lock policy 4-gate matrix (Functional/Hygiene/Docs/Self-consistency) áp dụng mọi version bump tương lai. Final: **835 tests**.

**ADR:** [ADR-0009](docs/decisions/0009-version-gate-policy.md) — version gate policy.

**Shipped:** Cargo workspace bump 0.1.0 → 0.3.0, clippy `-D warnings` clean (109 → 0), README sync, enum payload + variant tag dispatch, SSA loop+if phi cho mutable vars, iterator `.enumerate()` + nullable ops `?.`/`?:`/`!!`, tuple + literal pattern match. 11/11 differential examples byte-identical VM vs interpreter (closes v0.3 Gate A).

**Không làm:**
- Bench 3× → defer v0.4 perf pass (BENCHMARKS.md ghi rõ).

Commit log: `git log --oneline --grep="v0\.3\.x\.cleanup"`.

---

## v0.3.x.ternary — Ternary-native IR ✅ SHIPPED

Audit post-cleanup phát hiện 5 binary-thinking leak ở IR (BrIf 2-way Unknown collapse, hardcoded if-vs-if? semantics, EnumTag 2/3 states, Constant::Null bolt-on, Eq trên Unknown trả False). Phase lock thiết kế tam phân-first ở IR level trước khi v0.4 ABI freeze. Final: **838 tests**, `.triv` wire v1 → v2.

**ADR:** [ADR-0010](docs/decisions/0010-ternary-native-ir.md) — `BrTrilean` 3-way branch + Ł3-aware `Eq`/`Ne` + `Constant::Null` = Trit::Zero. Backend mapping: binary CPU 2 cmp + 2 branch; trytecode hardware 1 native instruction (điểm Triết thắng vĩnh viễn).

**Shipped:** `BrTrilean` opcode (0xB4), lowerer migrate (0 emit BrIf, 7 emit BrTrilean), strict `if` Unknown→panic, `Eq`/`Ne` propagate Unknown khi operand Trilean::Unknown. `BrIf` còn lại cho .triv v1 backward decode + 2-state verified Trit cases.

**Không làm (defer):**
- Xoá `BrIf` enum variant — wire format compat.
- ≥4-variant enum encoding thành Tryte tag — chưa example nào cần.
- Capability `Trilean` dispatch → v0.6 (build trên BrTrilean infrastructure).

Commit log: `git log --oneline --grep="v0\.3\.x\.ternary"`.

---

## v0.4 — Crate-Pack + Stable ABI ✅ SHIPPED

**Mục tiêu:** Phân phối binary library + type-safe cross-package linking. Final: **867 tests**, `.triv` wire v2 → v3.

**ADRs:** [ADR-0011](docs/decisions/0011-abi-metadata-format.md) ABI metadata (BLAKE3, 2-level hash `iface_hash` + `impl_hash`, section ID layout); [ADR-0012](docs/decisions/0012-witness-table-dispatch.md) witness table dispatch (Swift-style hybrid: monomorphize intra-pkg, witness inter-pkg); [ADR-0013](docs/decisions/0013-semver-linking-policy.md) semver decision matrix (`iface_hash_pin` final arbiter, auto-shim NOT promised).

**Shipped:** `triet-pack` crate, `.khi` serde (11 round-trip tests), cross-package linker + decision matrix E2300-E2399 (8 tests), `WitnessCall` opcode + `.triv` v3 wire format + VM dispatch, `std.result` canonical + SPEC `T?` primary nullable, cross-package demo (7 integration tests). Detail: [docs/ARCHITECTURE.md §Crate-Pack distribution](docs/ARCHITECTURE.md#crate-pack-distribution-shipped-v04-adr-001100120013).

**Không làm (defer):**
- CAS hash identity → v0.5 (`iface_hash_pin` prep đã có).
- Auto-shim ABI migration — rejected per [VISION §3.3](VISION.md) (semantic change không decidable).
- Capability enforcement runtime → v0.6 (slot reserved trong ABI metadata).
- CLI `triet link` subcommand → v0.5 (API trong `triet-pack` là contract).
- Cross-module enum variant import (pre-existing v0.2.x gap) → v0.5.
- Cross-package generic lowerer emit (`WitnessCall`) → v0.5+ multi-package compile.

Commit log: `git log --oneline --grep="v0\.4"`.

---

## v0.5 — CAS Packaging ✅ SHIPPED

**Mục tiêu:** Định danh package bằng hash, eliminate DLL Hell, prep parallel versions ở RAM level. Final: **918 tests**, `abi_version` 1 → 2 (v=1 refused, no shim).

**ADRs:** [ADR-0014](docs/decisions/0014-hash-scheme-refinement.md) 3-cấp hash tree (term + module + pkg) với 16-byte domain separators per level; [ADR-0015](docs/decisions/0015-package-store-layout.md) `~/.triet/store/{term,mod,pkg,names,roots,tmp}/`, atomic install via tmp + `rename()` (POSIX), mark-sweep GC.

**Shipped:** Hash-based resolver + `dao.lock` hand-rolled line format (sort-by-name canonical, no serde dep), `dao store {import,list,gc}` CLI, shared loading demo (term iface dedup proven via `tests/shared_loading.rs`), cross-module enum variant import (`from std.result import Ok, Err` — closes v0.2.x gap), E2107 cho aliased variant import. Detail: [docs/ARCHITECTURE.md §CAS Packaging](docs/ARCHITECTURE.md#cas-packaging-shipped-v05-adr-00140015).

**Không làm (defer):**
- Lowerer emit `WitnessCall` cross-package generics → v0.7 self-host.
- v=1 `.khi` lossy migration → on-demand khi v=1 packs xuất hiện in wild.
- Body-level RAM dedup → v0.6+ alongside lowerer per-term split.
- Distributed registry / network fetch → v1.0+ (local store đủ).
- Auto-GC — manual đủ ("refuse over guess").

Commit log: `git log --oneline --grep="v0\.5\."` (excluding `.review`).

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

Trụ cột bản sắc #5 ([VISION §3.5 + §5](VISION.md)). Capability is namespace attribute trong per-package `dao.package` manifest; runtime `Defer` slots resolve via `dao.policy` + optional TTY prompt. Final: **1079 tests**, `abi_version` stays `2` (ADR-0016 §4 populate-not-bump promise).

**ADRs:** [ADR-0016](docs/decisions/0016-capability-type-system.md) capability type system (4-state level Grant/Ambient/Deny/Defer + Trilean::Unknown; wire reuses caps section since v0.4 ABI metadata; root authority sole decision-maker, no path inheritance); [ADR-0017](docs/decisions/0017-trilean-policy-hook.md) Trilean policy hook (`dao.policy` rules + TTY fallback, per-session cache, monotonicity invariant; *+ Addendum: parser strict + `/dev/tty` source + Abstain errata*); [ADR-0018](docs/decisions/0018-capability-loader-semantics.md) loader semantics (`dao.package` grammar, eager Step 6a refuse, TTY provenance prompt, `CapabilityClaim` Rust struct; *+ v0.6.x.review Addendum: monotonicity-under-mutation, policy round-trip, requester sort, strict_parser contracts*).

**Shipped:** Compile-time E2200/E2201 fire khi `usr.*` imports `dev.*`/`sys.*` không cap claim, runtime policy hook + TTY prompt (`/dev/tty` paired I/O POSIX, anti-spoofing, ASCII `!!` markers, G/D permanent write), E22XX fully populated E2200–E2208 across parse/compile/link/runtime. Capstone test `capability_pipeline.rs` (12 integration tests) + `demos/04-capability-system/` illustrative. Detail: [docs/ARCHITECTURE.md §Capability System](docs/ARCHITECTURE.md#capability-system-shipped-v06-adr-001600170018).

**Không làm (defer):**
- CLI wiring (project layout discovery, cap-aware build emitting caps section, `DevTtyPrompt` integration) → v0.7 self-host.
- E2208.PreV06Reader → future `abi_version` bump.
- E2208.CapabilityDivergence → khi lowerer populates caps section from `dao.package`.
- Per-function cap granularity → post-v1.0 (ADR-0016 "Không làm").
- Wildcard claims in manifest — refuse-over-guess.
- Windows ConPTY for TTY prompt → POSIX-first.
- ANSI colour + Unicode TTY → post-security-floor.
- `Capability<T>` stdlib type — superseded by namespace-level claims (ADR-0016 §1 phương án C).
- Hardware enforcement — cần tam phân hardware hoặc bytecode VM sandbox.
- Distributed capability → v1.0+.

Commit log: `git log --oneline --grep="v0\.6\."` (excluding `.review`).

---

## v0.6.x.review — Pre-v0.7 audit fixes ✅ SHIPPED

Audit window trước v0.7. 6 net-new tests across 4 layers (resolver, policy, linker, strict_parser). Audit listed 10 gaps; 5 already covered, 1 deferred (CLI wiring → v0.7), 4 partial/real → 6 net-new. Không thay đổi spec; thêm Addendum vào ADR-0018. 1079 → 1085 tests.

| Sub-task | Description | Commit |
|---|---|---|
| v0.6.x.review.1 | Layer 1 (code core) — monotonicity-under-mutation, `upsert_then_save_round_trip`, requesters sort with non-alphabetical insertion | `d56c518` |
| v0.6.x.review.2 | Layer 2 (boundary/UI) — `strict_parser` positional contract pins: empty file, BOM mid-file, CR mid-line | `b6bde0c` |
| v0.6.x.review.3 | ADR-0018 Addendum + sync docs | this commit |

**Trigger:** Audit của tôi (AI) trước khi mở v0.7. Author chấp nhận transparency về 5 audit miss → ship 6 net-new tests filling distinct invariants. Duy trì cadence *proactive tech-debt audit trước version freeze*.

---

## v0.7 — Self-hosting Compiler ✅ SHIPPED

**Mục tiêu:** Compiler Triết viết bằng Triết. 3-stage chain với fixed-point hội tụ là gate.

**Closed 2026-05-25** với 30+ commits, 1085 → **1345 tests** (+260 net). Self-host compiler ships as 7 `.tri` files (~23K LOC) under `compiler/`. `dao build` đi qua filesystem-aware pipeline; main.tri biên dịch chính mình qua VM. Bootstrap gate Stage 2 ≡ Stage 3 byte-identical wired (`bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical`) nhưng `#[ignore]` per ADR-0019 §7 Addendum — VM dev tier > 15 min per main.tri compile, lifts CI-required ở v0.9 JIT.

**ADRs:** [ADR-0019](docs/decisions/0019-self-hosting-compiler-bootstrap.md) bootstrap chain + canonical emission invariants + Rust-shim stdlib (builtins 4–26) + perf gate recalibration; [ADR-0020](docs/decisions/0020-outcome-error-handling.md) Outcome `T~E`/`T?~E` trit-encoded fallibility + `~+`/`~0`/`~-` constructors + `~?`/`~:` postfix ops + verbose force-unwrap methods + `.triv` v4 → v5 patch bump; [ADR-0021](docs/decisions/0021-trilean-refinement.md) compile-time `Trilean!` refinement (E1033 `PossiblyUnknownCondition` / E1034 `TrileanReturnNotRefined`); [ADR-0023](docs/decisions/0023-lowerer-ssa-struct-tracking.md) lowerer SSA struct-tracking unified `ValueKind` enum (closed v0.7 review finding); [ADR-0024](docs/decisions/0024-khi-dao-identity-naming.md) Khí + Đạo identity (`.tri.bin` → `.khi`, CLI `triet` → `dao`, manifest `dao.package`, lockfile `dao.lock`; source `.tri` + IR `.triv` + name "Triết" giữ).

**Shipped:** Three-layer testing (per-component differential + e2e semantic regression + bootstrap-loop CI gate), canonical emission determinism CI (`examples/*.tri` × 10 builds byte-identical), CLI wiring carry-over v0.6 (project layout discovery, cap-aware build, `E2208.CapabilityDivergence` fires). Examples 14/14 dao check + 13/13 dao build (`while_true_loop.tri` infinite-loop fixture skipped); 12 interpreter run + 1 VM-only `outcome_propagate.tri` per ADR-0019 Addendum §A7 parity gap. Detail: [docs/ARCHITECTURE.md §Self-hosting Compiler](docs/ARCHITECTURE.md#self-hosting-compiler-shipped-v07-adr-001900200021024).

**Không làm (defer per ADR-0019 §Không làm):**
- Native AOT → v2.0.
- JIT → v0.9 Cranelift.
- Triết-native `std.collections`/`std.io.fs` wrappers → v0.8+ scope (Rust-shim builtins đủ).
- Macro / metaprogramming, cross-compile, incremental cache, parallel compilation → post-v1.0+.
- Triết-impl divergent semantics from Rust — goal là 1:1 reimplementation.
- Full 3-stage bootstrap loop < 10 phút → v0.9 JIT (ADR-0019 §7 Addendum; empirical ≥15 min per main.tri compile on VM dev tier).

Commit log: `git log --oneline --grep="v0\.7"`.

---

## v0.8 — Ownership Foundation + Concurrency Primitives (BYOS) ✅ SHIPPED

**Closed 2026-05-28** với 14 sub-tasks (v0.8.8–v0.8.13 bundled trong release commit `78f2402`). Post-release `v0.8.x.review` audit closed gate B Hygiene leftover + namespace mistag + self-host lexer port + doc drift. Final: **1425 tests** (+80 net from v0.7 close).

**Mục tiêu:** Lock memory model (S6) + concurrency Send rules primitives. **KHÔNG mục tiêu:** scheduler/runtime trong core language (BYOS — Bring Your Own Scheduler per ADR-0026 v2). NLL enforcement → v0.9. stdlib `std.concurrency.*` → v0.10.

**ADRs:** [ADR-0022](docs/decisions/0022-trit-balanced-ownership.md) S6 5-form reference (`&+` strong / `&0` neutral / `&-` weak / `&` bare / `owned` transfer) + định lý vô-chu-trình + capability-as-unsafe; [ADR-0025](docs/decisions/0025-borrow-checker-rules.md) borrow checker algorithm (NLL + 3-rule elision + no annotation policy + E24XX); [ADR-0026 v2](docs/decisions/0026-actor-boundary-send-rules.md) **BYOS** — Send rules universal + Atomic primitives + capability gates + refuse `actor`/`spawn`/`receive`/`send`/`async`/`await` keywords + E25XX; [ADR-0027](docs/decisions/0027-diagnostic-format-standard.md) AI-first diagnostic format (header `EXXXX ErrorName` + body + `[Fix N]` numbered blocks, ASCII, no diff `-/+`).

**Shipped:** `triet-core::memory::ObjectHeader` (8-byte binary, 54-trit ternary, refcount atomic ops, sentinels), 5-form lexer tokens + parser AST `ReferenceForm`, Send derivation cho 13 type categories per ADR-0026 v2 §2.1 (E2500 fires), capability schema mở rộng (concurrency caps `sys.raw_thread`/`sys.atomic`/`dev.ffi`/`dev.raw_memory`/`dev.reinterpret`; ownership caps `dev.self_ref`/`dev.custom_drop`), E24XX/E25XX skeleton diagnostics AI-first format. Demo `examples/atomic_counter/` works end-to-end (parses + typechecks + runs main println); Atomic operations declaration-only per ADR-0026 §3 — full impl v0.9 ADR-0028. Detail: [docs/ARCHITECTURE.md §Ownership + BYOS](docs/ARCHITECTURE.md#ownership-foundation--concurrency-primitives-byos-shipped-v08-adr-002200250026-v20027).

**Pivot 2026-05-26 (ADR-0026 v1 → v2):** Original plan included `actor`/`receive`/`send`/`spawn` keywords + actor demo. Author raised kernel-writability concern (Linux Rust modules cannot use async runtime — must defer to C scheduler; same problem applies to v1 Triết). v2 BYOS reframes: core provides primitives + capability gates, scheduler stdlib/external. Same compile-time safety regardless of scheduler. Test estimate scaled 245 → 150 → 80 actual (BYOS revert removed actor demo + lexer + integration scope).

**Gate (ADR-0009):** ✅ A — 0 `TODO(v0.8)`, 3 `#[ignore]` documented. ✅ B — 1425 tests, clippy `-D warnings` + fmt clean (post `v0.8.x.review.1` closure `e8d797a`). ✅ C — Cargo 0.7.0 → 0.8.0, SPEC v0.8 header, README synced (post `v0.8.x.review.4` `ebdbd15`). ✅ D — 12/13 examples interp+VM; `outcome_propagate.tri` VM-only per ADR-0019 Addendum §A7. Perf gate N/A (parser/typecheck only).

**Không làm (defer):**
- NLL borrow checker enforcement (E2440 fires) → v0.9 (cần real-world corpus).
- Lifetime elision 3 rules (E2400 fires) → v0.9 (cần monomorphization infra).
- `&-` upgrade tracking (E2403 fires) → v0.9 (cần escape analysis).
- Drop order + `dev.custom_drop` (E2450+) → separate ADR.
- Move semantics hard error (E2420) → v0.10 (currently stub).
- Atomic primitive implementation → v0.9 ADR-0028 (full memory ordering).
- stdlib `std.concurrency.*` reference impl (green-thread, channel, scope, actor as struct) → v0.10.
- Multiple alternative scheduler crates (triet-rtos, triet-linux) → v1.0+ community.
- Self-hosting compiler uses ownership keywords → Stage 2+ post-v0.8.
- Full auto-wrap lowerer (ADR-0020 §3.0) → v0.7.4.3-error.5.
- Custom scheduler examples (Linux kthread, RTOS) → post-v1.0.

Commit log: `git log --oneline --grep="v0\.8"` (excluding `.review`/`.docs-reorg`).

---

## v0.8.x.review — Post-v0.8 audit fixes ✅ SHIPPED

Audit window sau Release v0.8.0 commit `78f2402`. Whole-project review phát hiện gate B Hygiene leftover + E25XX namespace mistag + v0.8.12 paperwork-vs-reality gap (self-host lexer thiếu `&` token) + widespread doc drift. 5 sub-tasks fix all findings; không thay đổi spec.

| Sub-task | Description | Commit |
|---|---|---|
| v0.8.x.review.1 | Close ADR-0009 gate B leftover — 3 clippy errors trong `resolver.rs` (collapsible-if + manual-let-else + single-match-else trên ambient-module fallback) + 21 `cargo fmt --all` files | `e8d797a` |
| v0.8.x.review.2 | E25XX namespace correction `triet::borrow::E25XX` → `triet::actor::E25XX` per ADR-0026 v2 + CLAUDE.md namespace table (6 chỗ ở `error.rs` + `cli/main.rs`) | `fcc18fd` |
| v0.8.x.review.3 | Port ownership reference tokens to self-host lexer — `compiler/parser/lexer.tri` thêm `AmpersandPlus/AmpersandZero/AmpersandMinus/Ampersand` Token variants + dispatch (longest-match precedes `&&`) + smoke check `check_count("ops_ownership", "&+ &0 &- &", 4)` | `46c8722` |
| v0.8.x.review.4 | Doc sync — CLAUDE.md (state + 2 arch sections + anchor + trit table + cadence + examples + audit history), README.md (v0.8 highlight + structure + tests), docs/decisions/README.md (§v0.8 add 0022/0025/0026/0027, remove "Future research"), ADR status Draft → Locked × 4 | `ebdbd15` |
| v0.8.x.review.5 | ROADMAP §v0.8 SHIPPED marker + archive sub-tasks + add this v0.8.x.review section + TODO.md v0.8 archive | this commit |

**Trigger:** Whole-project audit sau v0.8.0 release. Author confirmed "tất cả các lựa chọn phải tuân thủ chặt chẽ stability over speed" — phase mở để fix tất cả audit findings trước khi v0.9 mở. Duy trì cadence *proactive tech-debt audit trước version freeze* (per v0.5.x.review / v0.6.x.review precedent).

---

## v0.8.x.docs-reorg — Pre-v0.9 doc reorganization ✅ SHIPPED

Author đặt vấn đề: docs đã đầy đủ trước v1.0, có nên reorg để (a) tiết kiệm AI token mỗi conversation, (b) categorize 27 ADRs theo topic? Audit phân tích tradeoffs → recommend doc-only reorg (compact CLAUDE.md + ADR thematic index + ROADMAP compression + close ADR↔SPEC gaps), defer "promote ADRs to technical docs" approach vì sẽ tạo drift surface 3 lớp.

8 sub-tasks fix all findings; không thay đổi spec semantics — chỉ expose existing semantics tới SPEC.

| Sub-task | Description | Commit |
|---|---|---|
| v0.8.x.docs-reorg.1 | Extract CLAUDE.md §Architecture (95 dòng phase prose) ra `docs/ARCHITECTURE.md`. CLAUDE.md 3353 → 2356 từ (-30% tokens/turn). | `e6fbc7a` |
| v0.8.x.docs-reorg.2 | Thematic ADR index `docs/decisions/by-topic.md` — 8 topic clusters cho 27 ADRs. Chronological `README.md` giữ. Không duplicate content. | `2dea4a0` |
| v0.8.x.docs-reorg.3 | Compress ROADMAP shipped-phase prose (v0.2.x → v0.8). Drop sub-task changelog tables, per-phase gate enumeration, phase-specific tangent sections. 623 → 384 dòng (-40%). Fixed stray garbage line 62. | `291fd5b` |
| v0.8.x.docs-reorg.4.1 | Fix SPEC §4.5 drift — Trilean equality propagates Unknown per Ł3 (was "never produces unknown" v0.2-era text; ADR-0010 §4 + ADR-0021 corrected since v0.3.x.ternary but SPEC never synced). BLOCKER. | `5e9fa0e` |
| v0.8.x.docs-reorg.4.2 | Add SPEC §2.5 Outcome operators + methods — ternary family `~+>`/`~0>`/`~->`, auto-wrap rule, MAP vs EARLY-RETURN, safe properties (`.is_success`/`.is_null`/`.is_error`), safe extraction (`.try_value`/`.try_error`), dangerous methods (`.unwrap_value(msg)`/`.unwrap_error(msg)`), pattern exhaustiveness, T~E? rejection. | `48a33ab` |
| v0.8.x.docs-reorg.4.3 | Add SPEC §10.6 Send rules + BYOS + Atomic surface — 13-row Send derivation table per ADR-0026 §2.1, BYOS philosophy với refuse-list, Atomic primitive placeholder, capability gates table, refcount-mediated share cross-boundary. | `6a05bfc` |
| v0.8.x.docs-reorg.4.4 | Add SPEC §6.4-6.7 Module system surface — module declaration (file-bound + inline), import statements (Python-style + whole-module), visibility ladder (3 levels), reserved namespace roots, file resolution convention, cyclic refusal. | `65f7ed9` |
| v0.8.x.docs-reorg.4.5 | Add SPEC §11 Capability system — 4-state level, `dao.package` grammar, `dao.policy` grammar, root authority, monotonicity invariant, TTY prompt fail-closed, E22XX table. Renumber §12-15 (Examples / EBNF / Open issues / Roadmap). | `d6f036f` |

**Token economy delivered:**
- CLAUDE.md: 284 → 234 dòng / 3353 → 2356 từ (~-30% per-turn).
- ROADMAP.md: 623 → 384 dòng (~-40%).
- ADRs: thematic index added, ADR content unchanged (immutable rule honored).

**SPEC.md additions** (semantics đã shipped trong code, commit này expose tới authoritative doc):
- §2.5 Outcome operators (~71 lines)
- §6.4-6.7 Module system (~73 lines)
- §10.6 Concurrency boundary + Send rules (~48 lines)
- §11 Capability system (~120 lines)
- §4.5 BLOCKER fix (Trilean equality drift)

Defer khỏi reorg (per stability-over-speed scope discipline):
- "Promote ADRs to technical docs by category" approach — refuse (would tạo 3-lớp drift surface: SPEC + tech-doc + ADR).
- F-string format subset detail in §1.5.5 (ADR-0002), multi-line string indent rules in §1.5.4 (ADR-0004), iterator protocol §9 (ADR-0003), full atomic API §10/§9 (ADR-0028 TBD v0.9), EBNF §13 refresh — all MED priority, defer v0.9.

**Trigger:** Author yêu cầu pre-v0.9 doc maturation: "chúng ta đã gần như hoàn thành mọi thứ trước version 1 rồi, chỉ còn 0.9. Bạn có nghĩ chúng ta nên chỉnh lý lại doc". Phase mở để rapid-fire close all doc-related debt trước version 1.0 freeze window.

---

## v0.8.x.cadence-fix — Process enforcement automation ✅ SHIPPED

Audit retrospective phát hiện v0.8 release commit `78f2402` vi phạm ADR-0009 gate B (3 clippy errors + 21 unformatted files). Author confirmed 2026-05-28: "không cố ý, không nhận ra cadence slip" — root cause = **policy có, automation không**. ADR-0009 §B references "CI" làm enforcement mechanism từ 2025 nhưng repo chưa setup CI; gates verify bằng mắt người.

Phase này codify enforcement tools để v0.8-style slip không lặp lại trong v0.9+.

| Sub-task | Description | Commit |
|---|---|---|
| v0.8.x.cadence-fix.1 | `scripts/release-check.sh` — single command verify ADR-0009 4-gate matrix + drift checks (version sync, TODO consistency, ADR status sanity). Exit 0 = safe to tag; exit 1 = refuse release. | `4e48703` |
| v0.8.x.cadence-fix.2 | Git hooks (`.githooks/pre-commit` + `.githooks/pre-push`) + `scripts/install-hooks.sh`. pre-commit fmt --check (~0.5s) + pre-push full gate B (~1 min). `core.hooksPath = .githooks` per-clone setup. | `31cb1c7` |
| v0.8.x.cadence-fix.3 | ADR-0009 Addendum — codify release-check.sh + hooks + mandatory pre-version audit window. "Ship rồi audit" = explicit cadence violation. | `594641b` |
| v0.8.x.cadence-fix.4 | Archive phase to ROADMAP + TODO + README install-hooks instruction. | this commit |

**Slip protection model:**
- **Commit-time guard** (pre-commit fmt) — catches v0.8.x.review.1-style fmt issues immediately.
- **Push-time guard** (pre-push full gate B) — last guard trước remote.
- **Release-time guard** (`release-check.sh`) — verify 4-gate + drift, refuse tag nếu fail.
- **Policy guard** (ADR-0009 Addendum) — codify "ship rồi audit" = violation.

Each guard independent — bypass one (e.g., `--no-verify` for WIP) doesn't compromise others. Author can still force-push to private fork; bypass on main = explicit violation.

**Trigger:** Author 2026-05-28 sau audit retrospective: "Chúng ta cần sửa chữa các vấn đề này trước khi sang 0.9." Phase mở để fix root cause (automation absence) chứ không chỉ symptom (v0.8 cleanup).

---

## v0.8.x.completion — Trục 2 implementation gap closure ✅ SHIPPED

Author 2026-05-29: "không làm 0.9 và làm post 0.8. Những vấn đề ở các trục 2, 3. Hãy sửa chữa chúng ở post 0.8". Trục 3 đã đóng tại v0.8.x.cadence-fix. Trục 2 (implementation reality): audit re-verification phát hiện 1 false alarm + 1 real gap + test coverage thin.

| Sub-task | Description | Commit |
|---|---|---|
| v0.8.x.completion.1 | Fix `examples/atomic_counter/dao.package` (deny → grant + add sys.atomic) + sync 4 stale doc files claiming demo "doesn't parse" (CLAUDE.md, README.md, ROADMAP.md, ARCHITECTURE.md). Demo works end-to-end: parses, typechecks, `dao run` outputs "Atomic counter demo (capabilities granted!)". Audit false-alarm: Rust impl parser actually has ReferenceForm fully wired (`type_expr.rs` 30+ refs + 8 dedicated tests). | `5ae0d08` |
| v0.8.x.completion.2 | Port S6 ownership ReferenceForm to self-host `compiler/parser/parser.tri`. Add: ReferenceForm enum (5 variants), ReferencePayload struct, Reference variant trong TypeExpr enum, format_type Reference arm (compact `&+T`/`&+mutT`/`&0T`/`&0mutT`/`&-T`), try_parse_reference_prefix helper, parse_type_minimal dispatch. 9 new smoke assertions in parser_type_smoke. Mirror Rust impl pattern. Closes v0.8.12 paperwork gap fully (lexer port v0.8.x.review.3 + parser AST port v0.8.x.completion.2 = symmetric coverage). | `3ad4874` |
| v0.8.x.completion.3 | Test top-up — close coverage gaps from ADR-0026 §2.1 + ADR-0027. +7 Send derivation tests (String/T?/&+mut/&0mut/&0/strong-mut-propagate/Unit). +4 diagnostic namespace verification tests guarding E24XX = `triet::borrow::` và E25XX = `triet::actor::` (v0.8.x.review.2 regression protection). 1425 → 1436 tests. | `3fe9e00` |
| v0.8.x.completion.4 | Archive phase to ROADMAP/TODO. | this commit |

**Audit retrospective lessons learned:**

1. **Previous audit had false-alarms.** "atomic_counter parser-side ReferenceForm not yet ported" propagated across 4 docs in v0.8.x.docs-reorg, but actual code (`triet-parser/src/type_expr.rs`) had it fully wired. v0.8.x.completion.1 traces this to agent who flagged without verifying against actual code; subsequent docs trusted the unverified claim. **Mitigation:** future audits MUST cite file:line evidence for each "missing" claim — if can't cite, can't claim. Codified ở [v0.8.x.cadence-fix.3 ADR-0009 Addendum](docs/decisions/0009-version-gate-policy.md) §C audit-window contract (release-check.sh runs full verification before tag).

2. **Self-host port lag is real and recurring** — lexer (v0.8.x.review.3) + parser (v0.8.x.completion.2) ported separately, both retroactive. v0.8 ship of Rust impl outpaced self-host. **Policy decision needed** (deferred to v0.9 plan): lockstep (each Rust impl change → self-host port same phase) OR freeze self-host at versioned snapshot. Currently mid-state.

**Structurally v0.9, NOT post-0.8 (per ADR-0025/ADR-0026 explicit defer):**

- NLL borrow checker enforcement (E2440 fires real, not skeleton) — ADR-0025 explicitly defers v0.9 awaiting real-world Triết corpus.
- Lifetime elision 3 rules (E2400 fires) — needs monomorphization infra (v0.9 scope).
- `&-` upgrade tracking (E2403 fires) — needs escape analysis (v0.9).
- Atomic primitive full implementation + memory ordering — ADR-0028 not written yet, needs design ADR; type placeholder shipped v0.8.
- Cross-cutting Send categories untested: Tuple mixed, Closure with captures, T~E outcome cross-thread, Heap struct with mutable non-Send. Deferred — v0.9 corpus + ADR-0028 will exercise.
- stdlib `std.concurrency.*` reference scheduler — v0.10.

**Trigger:** Author wants trục 2 implementation reality + trục 3 process discipline cùng đóng before v0.9 mở. Trục 3 closed v0.8.x.cadence-fix; trục 2 closed here.

Final: **1436 tests workspace-wide** (+11 từ v0.8.x.review/docs-reorg close; +91 net từ v0.7 close 1345).

---

## v0.9 — Wide-phased: JIT + Borrow Enforcement + Atomic + Self-host policy 🔄 in progress

**Scope decision 2026-05-29:** Author chose wide-phased option to match all "defer v0.9" promises locked across ADR-0025 (borrow enforcement), ADR-0026 v2 (Atomic), and ROADMAP §v0.9 (JIT). Phase v0.9 is multi-workstream — design ADRs first, then implementation sub-phases roughly parallel.

**Mục tiêu** (4 workstreams):

1. **JIT (Cranelift)** — production tier 2 cho hot code paths.
2. **Borrow checker enforcement** — E2400/E2403/E2440 fire real (not skeleton); NLL + 3-rule lifetime elision + `&-` upgrade tracking.
3. **Atomic primitive implementation** — full memory ordering enforcement + operation set per ADR-0028 (refined from ADR-0026 v2 §4 placeholder).
4. **Self-host port policy** — per v0.8.x.completion.4 retrospective lesson: lockstep vs freeze decision codified in ADR-0029.

### Quyết định kiến trúc (ADRs to write — v0.9.0 design phase)

- [ADR-0028](docs/decisions/0028-atomic-primitive.md) — Atomic primitive design. Refines ADR-0026 v2 §4 placeholder. Locks: implementation pattern (VM opcodes vs builtin shims per ADR-0019 §5), Ordering↔Trit mapping, full operation set (load/store/swap/compare_exchange + fetch_add/sub/and/or/xor), constructor + drop, E2530 fire conditions, interaction với `&+ mutable` ownership exclusivity.
- [ADR-0029](docs/decisions/0029-self-host-port-policy.md) — Self-host port policy. Per v0.8.x.completion.4 lessons learned: v0.8.x.review.3 (lexer) + v0.8.x.completion.2 (parser AST) both retroactive ports. Locks: lockstep (mỗi Rust impl change → port self-host cùng phase) vs freeze (snapshot self-host tại version, gap acknowledged publicly).
- [ADR-0030](docs/decisions/0030-jit-cranelift-integration.md) — JIT integration. Cranelift backend choices, tier-2 dispatch trigger (call count threshold), AOT cache layout, profile-guided heuristics. Original ROADMAP §v0.9 deliverables go here.

### Sub-phase ordering

```
v0.9.0 — Design phase (3 ADRs)
   ↓
v0.9.x.atomic — depends on ADR-0028
v0.9.x.borrow — independent of others (ADR-0025 design đã lock)
v0.9.x.jit    — depends on ADR-0030
   ↓ (any order after design)
v0.9.final    — release: version bump, SPEC v0.9 header, gate verify
```

### Deliverables (carry forward từ ROADMAP nguyên thủy §v0.9 + new wide scope)

**JIT subsystem (per ADR-0030 lock):**
- Tier 1: bytecode interpreter (v0.3) ✅ exists.
- Tier 2: Cranelift JIT cho function chạy thường xuyên (profile-guided).
- AOT cache: lần chạy thứ 2 dùng JIT-output cached.

**Borrow checker enforcement subsystem (per ADR-0025 deferred):**
- E2400 LifetimeInferenceFailed real fires (not skeleton).
- E2403 EscapingBorrow real fires.
- E2440 BorrowExclusivityViolation NLL enforcement.
- 3-rule lifetime elision algorithm (single-input → output; method self → output; owned return).
- `&-` weak observer upgrade tracking.

**Atomic subsystem (per ADR-0028, refined từ ADR-0026 v2 §4):**
- `Atomic<T>` runtime values trong `triet-ir` VM + `triet-interpreter`.
- `Ordering` enum (Relaxed/Synchronized/Strict mapped vào Trit).
- Operations: `load`, `store`, `swap`, `compare_exchange`, `fetch_add`, `fetch_sub`, `fetch_and`, `fetch_or`, `fetch_xor`.
- Stdlib module `sys.atomic.*` với capability gate.
- E2530 InvalidAtomicOrdering real fires.

**Self-host port (per ADR-0029):**
- Sync mechanism enforced per chosen policy.
- Current gaps reconciled (atomic_counter.tri can compile through self-host if policy is lockstep).

### Gate (ADR-0009 §A/B/C/D applied at v0.9.final)

**Functional (A):**
- 0 `TODO(v0.9)` markers in source.
- Bench ≥10× v0.3 bytecode baseline on numeric-heavy programs.
- `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` lifts from `#[ignore]` → CI-required (carry-forward functional gate per [ADR-0019 Addendum](docs/decisions/0019-self-hosting-compiler-bootstrap.md#addendum--v0713-perf-gate--10-ph%C3%BAt-deferral)).
- Full 3-stage bootstrap loop < 10 phút trên dev hardware (carry-forward perf gate).
- Self-hosted compiler bootstrap loop ≤ 2× Rust impl runtime trên same hardware (carry-forward).
- All deferred E24XX / E25XX errors fire real (no skeleton remaining).

**Hygiene (B):**
- `scripts/release-check.sh` ✓ via ADR-0009 Addendum §A protocol.
- Pre-commit + pre-push hooks active (per ADR-0009 Addendum §B).

**Docs (C):**
- Cargo workspace.package.version 0.8.0 → 0.9.0.
- SPEC.md header v0.8 → v0.9.
- README + ARCHITECTURE.md sync.
- ADR-0028/0029/0030 status: Locked (no Draft references in SPEC).

**Self-consistency (D):**
- All `examples/*.tri` typecheck + build OK including atomic_counter via self-host (per ADR-0029 lockstep choice).
- Send derivation tests cover all 13 categories (close gap từ v0.8.x.completion.3 partial coverage).

### Pace expectation

Per ROADMAP §"Pace expectations": v0.8-v0.9 = 6-12 tháng mỗi phase. Wide scope = realistic 12-18 tháng. ADR-0009 + Addendum §C enforce pre-version audit window mandatory → no "ship rồi audit" anti-pattern.

### Pivot history

Original ROADMAP §v0.9 (pre-v0.8.x.completion) = JIT-only. Author 2026-05-29 chose wide-phased after v0.8.x.completion.4 retrospective identified 5+ deferred items targeting v0.9: NLL enforcement, lifetime elision, `&-` upgrade tracking, Atomic full impl, self-host policy. Rather than re-defer to v0.10, close all in v0.9 before v1.0 freeze.

### Sub-phase status (2026-05-30)

| Sub-phase | ADRs | Status | Final test count |
|---|---|---|---|
| v0.9.0 Design phase | 0028, 0029, 0030 | ✅ SHIPPED (3 ADRs Locked) | 1436 |
| v0.9.x.atomic Atomic Primitive Implementation | 0028, 0031 | ✅ SHIPPED (see below) | **1506** |
| v0.9.x.borrow Borrow checker enforcement | 0025 | ⏸️ Folded — E2420 ships in atomic.7d; NLL/lifetime/`&-` upgrade defer v0.10 per ADR-0031 §10.1 | — |
| v0.9.x.jit Cranelift JIT backend | 0030 | 🔜 pending | — |
| v0.9.final release | — | 🔜 pending | — |

---

## v0.9.x.atomic — Atomic Primitive Implementation ✅ SHIPPED

**Closed 2026-05-30** với 12 sub-tasks across 13 commits. Final: **1506 tests** (+70 from v0.9.0 design close at 1436).

**Mục tiêu:** Refine ADR-0026 v2 §4 placeholder into full Atomic primitive support per ADR-0028 — type-level + IR builtins + VM dispatch + capability + stdlib + borrow expression syntax + use-after-move enforcement + runtime demo. Closes the `&+ Atomic<T>` SPEC §10 v0.7 warning gap end-to-end.

**ADRs locked during phase:**
- [ADR-0028](docs/decisions/0028-atomic-primitive.md) Atomic primitive design (Locked v0.9.0.1, ships through .1–.6).
- [ADR-0031](docs/decisions/0031-borrow-expression-syntax.md) Borrow expression syntax (Locked v0.9.x.atomic.7a Phương án A — closes ADR-0028 §6 example syntax gap).

**Shipped (12 sub-tasks):**

| Sub-task | Description | Commit |
|---|---|---|
| .1 | `AtomicValue` typecheck enforcement (E1040) — `Type::is_atomic_value()` + validation at construction site | `6788d1c` |
| .2 | IR builtin declaration (10 atomic builtins) + `.triv` v5→v6 + VM placeholders + path lookup + self-host TRIV_VERSION lockstep port | `d898760` |
| .3 | VM dispatch universal ops (AtomicNew + Load/Store/Swap/CompareExchange) | `a90caa6` |
| .4 | VM dispatch arithmetic + bitwise (fetch_add/sub + fetch_bitwise_and/or/xor) | `0d3f9fa` |
| .5a | Loader synthetic `sys` root infrastructure | `8131d1e` |
| .5b | Stdlib `std/sys/atomic.tri` shipped (Ordering enum + 10 function signatures) | `6f6b92e` |
| .5c | Stdlib `std/sys/raw_thread.tri` placeholder + cross-module Atomic typecheck closed | `97e30d6` |
| .6 | E2530 `InvalidAtomicOrdering` conservative fire (compare_exchange success<failure) | `0922936` |
| .7a | ADR-0031 Borrow Expression Syntax Draft + Locked Phương án A + v0.10 backlog | `86e59b8` / `6fd3a6e` |
| .7b | Rust impl borrow expression syntax (AST + parser + typecheck + lowerer + interpreter) | `d2ab072` |
| .7c | Self-host Layer A port — `compiler/parser/parser.tri` lockstep | `9e0e10c` |
| .7d | E2420 `UseAfterMove` real fires (CFG move-tracking + branch-aware join) | `bb92aa5` |
| .7e | `atomic_counter` demo single-call + VM e2e | `8296895` |

**Pivot 2026-05-30 (Phương án A):** During .7 design, author surfaced concern: shipping borrow expression syntax + multi-worker demo without consume-once enforcement would teach E2420-failing patterns that v0.10 NLL rejects. Per "chậm mà chắc, không ship tạm bợ" — split .7 into 5 sub-tasks (.7a–.7e), add E2420 enforcement in .7d (was originally deferred v0.9.x.borrow), scope demo down to single-call (.7e). Forward-compat guarantee: any v0.9-passing program continues passing v0.10 NLL.

**Gate (ADR-0009 §A/B/C/D applied):**
- ✅ A — 0 `TODO(v0.9.x.atomic)` markers; demo + e2e proves end-to-end pipeline.
- ✅ B — 1506 tests, clippy `-D warnings` + fmt clean.
- ✅ C — ADRs 0028 + 0031 Locked, README + by-topic index updated; Cargo version stays 0.8.0 until v0.9.final.
- ✅ D — release-check.sh PASSED; self-host Token/TypeExpr symmetry preserved; atomic_counter demo typechecks via `dao check` + runs via in-process VM.

**Không làm (defer v0.10 — captured in [ADR-0031 §10 backlog](docs/decisions/0031-borrow-expression-syntax.md)):**
- §10.1 NLL enforcement (E2440 full CFG live-range), lifetime elision 3 rules (E2400), `&-` upgrade (E2403), mutability violations (E2410/11) — corpus-driven defer per ADR-0025.
- §10.2 Real `raw_thread.spawn` + Send-boundary refcount-bump codegen + `&+ Atomic<T>` multi-thread clone + multi-worker demo + `std.concurrency.*` stdlib.
- §10.3 Borrow expression operand scope expansion (function-call result, method-call result, index `vec[i]`, compound binary).
- §10.4 E2530 Pointer-Relaxed `fetch_*` pattern — blocked by Pointer type.
- §10.5 CLAUDE.md normative documentation drift policy.
- §10.6 Self-host typecheck-layer port lag tracking.
- §10.7 `triet-interpreter` atomic builtin intercepts (discovered .7e — currently VM-only demo per outcome_propagate.tri precedent).

Commit log: `git log --oneline --grep="v0\.9\.x\.atomic"`.

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
- **Syscall opcode design** trong IR (VISION §4.4 "OS-friendly properties — syscall opcodes / FFI primitives"). Lock encoding khi LLVM AOT landing — trước đó VM dev tier không cần.

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
