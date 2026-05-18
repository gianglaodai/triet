# ADR 0019 — Self-hosting compiler bootstrap (3-stage chain + canonical emission + Rust-shim stdlib)

**Trạng thái:** Quyết định. Áp dụng cho phase v0.7 — compiler Triết viết bằng Triết. Recalibrate perf gate v0.7 (defer 2× parity sang v0.9 JIT). Không đổi IR shape ([ADR-0007](0007-ir-design.md)), không đổi `.triv` wire format ([ADR-0008](0008-triv-binary-format.md) v3 + [ADR-0010](0010-ternary-native-ir.md) + [ADR-0012](0012-witness-table-dispatch.md)), không đổi `.tripack` ABI ([ADR-0011](0011-abi-metadata-format.md)), không đổi CAS scheme ([ADR-0014](0014-hash-scheme-refinement.md)), không đổi capability semantics ([ADR-0016](0016-capability-type-system.md)/[ADR-0017](0017-trilean-policy-hook.md)/[ADR-0018](0018-capability-loader-semantics.md)). Lock author direction confirmed 2026-05-17 (Q1-B, Q2-B, Q3-A, Q4-A, Q5-C, Q6-C, Q7-defer).

**Issue:** [ROADMAP §v0.7](../../ROADMAP.md) đặt mục tiêu *"Compiler Triết viết bằng Triết. Bootstrap đầy đủ"* với gate *"Bit-identical bootstrap qua 2 vòng tự build"*. Nhưng để hở 7 vùng kiến trúc cần lock TRƯỚC khi viết dòng Triết-compiler nào:

1. **Bootstrap chain shape** — single-stage vs 2-stage vs 3-stage? Mỗi chọn lựa có gate khác nhau.
2. **Component order** — big-bang rewrite hay incremental component-by-component? Cadence sub-task ảnh hưởng trực tiếp.
3. **Version skew handling** — Rust impl emit `.tripack` có thể khác Triết-in-Triết impl emit. Làm sao verify bit-identical?
4. **Gate semantics** — so sánh gì cho "bit-identical"? `.tripack` bytes, IR bytes, hash, hay semantic output?
5. **Stdlib status** — self-host compiler cần `Vec`, `HashMap`, file IO. Hiện stdlib chỉ 32 dòng. Extend toàn diện hay shim?
6. **Testing strategy** — per-component differential, end-to-end, hay bootstrap-loop CI?
7. **Performance gate** — ROADMAP nói "2× parity với Rust impl". Triết-on-VM thực tế 50-200× chậm hơn Rust-native. Recalibrate?

Plus: **carry-over từ v0.6** — CLI wiring (`triet check` đọc `triet.package`, `triet build` populate caps section, loader integration với `DevTtyPrompt`) defer khỏi v0.6 với note *"lands cleaner với v0.7 self-hosting"* ([SPEC §0.7 non-goals](../../SPEC.md#07-non-goals-của-v06)). ADR-0019 fold carry-over này vào scope v0.7.

ADR này lock 7 vùng + carry-over, đóng frame cho sub-task v0.7.2 trở đi.

## §1 — Bootstrap chain shape: 3-stage chain

**Quyết định:** 3-stage bootstrap, gate là Stage 2 ≡ Stage 3 byte-identical.

```
Stage 1  (Rust impl, v0.6)
  └─ input: compiler-source/*.tri (Triết-compiler-in-Triết source)
  └─ output: compiler-stage1-built.tripack

Stage 2  (Triết-in-Triết, built by Stage 1)
  └─ input: compiler-source/*.tri (SAME source)
  └─ output: compiler-stage2.tripack

Stage 3  (Triết-in-Triết, built by Stage 2)
  └─ input: compiler-source/*.tri (SAME source)
  └─ output: compiler-stage3.tripack

GATE: cmp compiler-stage2.tripack compiler-stage3.tripack → exit 0
```

**Lý do:**

- **Fixed-point hội tụ là proof toán học.** Nếu Stage 2 ≡ Stage 3, compiler đã hội tụ — output không phụ thuộc bộ build dùng để build nó. Stage 1 chỉ là bootstrap loader, không nằm trong gate.
- **Prior art:** rustc bootstrap (Stage 0/1/2), OCaml `boot/`, GCC `stage1/2/3/4-gcc`. Pattern này đã track 30+ năm.
- **Webapp analogy:** Build Docker image từ Dockerfile twice. Image digest phải match. Nếu khác → nondeterminism cần fix.
- **Cost:** ~1 lần compile thừa (~vài phút). Trade cho gate toán học chặt chẽ.

**Lock decisions:**

| Aspect | Decision | Lý do |
|---|---|---|
| Stage count | 3 (1 Rust + 2 Triết-in-Triết) | Fixed-point proof requires ≥2 Triết-in-Triết stages |
| Gate operator | `cmp` (byte equality) | Strongest valid equality cho `.tripack` |
| Stage 1 status | Bootstrap loader, NOT in gate | Stage 1 có thể có bug compatibility nhưng Stage 2 ≡ Stage 3 vẫn proof Triết-impl converged |
| Stage 3 → Stage 4 sanity (optional) | Run nếu Stage 2 ≢ Stage 3 fails | Debug aid — narrow down nondeterminism source |

**Compiler source layout** (locked):

```
compiler/                       # Triết-in-Triết compiler source
├── lexer.tri                   # 1:1 with Rust triet-lexer
├── parser.tri                  # 1:1 with Rust triet-parser
├── modules.tri                 # 1:1 with Rust triet-modules
├── typecheck.tri               # 1:1 with Rust triet-typecheck
├── ir_lowerer.tri              # 1:1 with Rust triet-ir lowerer
├── pack_writer.tri             # 1:1 with Rust triet-pack writer
└── main.tri                    # CLI driver (parse args, dispatch)
```

Mirror Rust crate boundaries. KHÔNG monolithic file — easier diff với Rust source, easier sub-task split.

## §2 — Component order: bottom-up incremental

**Quyết định:** Viết Triết-in-Triết component-by-component, từ thấp lên cao (lexer → parser → modules → typecheck → lowerer). Mỗi component khi land có differential test riêng (Triết-impl ≡ Rust-impl). Bridge tạm thời qua file IO khi mixed-stage (component nào đã Triết-native thì dump output ra file, component sau đọc lại).

**Order chốt:**

```
v0.7.4  lexer.tri        → emit token stream JSON, diff vs Rust lexer
v0.7.5  parser.tri       → emit AST snapshot, diff vs Rust parser
v0.7.6  modules.tri      → emit ResolvedProgram snapshot, diff vs Rust modules
v0.7.7  typecheck.tri    → emit type errors / OK signal, diff vs Rust typecheck
v0.7.8  ir_lowerer.tri   → emit .triv bytes, diff vs Rust lowerer
v0.7.9  pack_writer.tri + main.tri → wire all in Triết, drop bridges
```

**Bridge format (transient, NOT shipped as canonical):**

- Token stream: NDJSON `{type, span, lexeme}` per line.
- AST: insta-style snapshot text (already used by Rust impl).
- ResolvedProgram: JSON dump (single file output).
- Type errors: miette diagnostic plain text.
- `.triv`: canonical wire format (ADR-0008) — already byte-stable.

Bridges chỉ tồn tại trong sub-task v0.7.4–v0.7.8 transitional period. v0.7.9 drops all bridges; Triết-side data flows in-memory.

**Lý do bottom-up:**

- **Match cadence v0.3** (lowerer ship per sub-task v0.3.2/v0.3.3/v0.3.4). Author quen pattern.
- **Bug bắt sớm.** Triết-lexer bug → Triết-parser hỏng. Test lexer xong rồi mới parser — debug surface co lại.
- **Per-sub-task verify gate** match [ADR-0009 §A](0009-version-gate-policy.md) functional check.
- **Big-bang rewrite vi phạm Stability over speed** ([VISION §6](../../VISION.md)) — 5K LOC unintegrated = không thể test, không thể commit ở per-step pattern.

**Anti-prior-art:** rustc 2010 big-bang rewrite — 4 tháng debug post-switch. ADR-0019 explicitly rejects pattern này.

## §3 — Canonical emission invariants (deterministic output)

**Quyết định:** Lock canonical-emission invariants ngay trong Rust impl TRƯỚC khi viết Triết-compiler. Audit + fix mọi nondeterminism source. Add CI test `bootstrap_determinism` rebuild `examples/*.tri` × 10 lần, all bytes identical.

**Invariants required:**

1. **No HashMap iteration in output path.** Replace với `BTreeMap` HOẶC sort-before-serialize. Hiện ADR-0011 §6 đã lock sort-by-name cho ABI metadata; áp dụng cùng nguyên tắc cho IR body emission.
2. **No timestamps anywhere** trong `.tripack` / `.triv` output. Compile time, file mtime → forbid.
3. **No random / process-state-dependent IDs.** `ValueId` / `BlockId` / `FuncId` deterministic per source structure.
4. **No env-var leak.** `$PWD`, `$USER`, `$HOSTNAME` không bao giờ ảnh hưởng output.
5. **File scan order: sorted by path.** Module loader walks filesystem → sort entries by name BEFORE process.
6. **Constant pool insertion order = canonical.** Hiện đã lock per ADR-0008 §Constant pool; verify Rust impl preserve.

**Audit task (v0.7.2 scope):**

```
1. grep HashMap entire workspace, identify output-path uses, replace với BTreeMap hoặc sort.
2. grep SystemTime/Instant entire workspace, verify zero uses trong emit.
3. cargo test bootstrap_determinism — build 11/11 examples × 10 lần, byte-cmp all results.
4. CI gate added: every commit must pass determinism test.
```

**Lý do:** [ADR-0014 §4](0014-hash-scheme-refinement.md) đã promise canonical encoding cho CAS hash stability. Self-hosting bootstrap = stricter test cùng invariant. Nếu Stage 2 ≢ Stage 3 fails, **chắc chắn** là nondeterminism somewhere trong emission path — debug khó vì compiler-in-Triết và compiler-in-Rust chia sẻ rất ít code.

**Webapp analogy:** "Microservice API response phải reproducible — không timestamp, không random UUID, không server hostname trong payload". Đây là cùng nguyên tắc cho `.tripack`.

## §4 — Bit-identical gate semantics: full `.tripack` bytes

**Quyết định:** Gate = `cmp compiler-stage2.tripack compiler-stage3.tripack` byte-identical. Không loosen, không hash-only, không semantic-equivalence fallback.

**Lý do:**

- **ADR-0011 §6 đã promise canonical encoding** cho ABI metadata + dep table + caps section. ADR-0008 đã promise canonical IR body emission. Mọi precondition cho byte-identical gate đã ship.
- **`cmp` là test trivial** — không cần custom harness, không cần parse.
- **Stricter test bắt nondeterminism mà hash collision có thể miss.** (Hash collision đơn lẻ unlikely cho BLAKE3, nhưng compound errors khi compiler emit khác bytes nhưng cùng hash — possible nếu canonical encoding refactor sai.)
- **Webapp analogy:** "Same input → same output. Bytes are the contract."

**Failure modes & debug path:**

| Failure | Likely root cause | Debug action |
|---|---|---|
| Stage 2 ≢ Stage 3 differs in 1-2 bytes | Single nondeterminism source (HashMap iter, etc.) | Run `xxd diff` on first 1KB; binary search for diverging offset |
| Stage 2 ≢ Stage 3 differs in many bytes | Triết-impl logic bug (lowerer emit wrong opcode for some construct) | Compare smaller test programs first (single function .tri) |
| Stage 2 ≡ Stage 3 but `examples/*.tri` regress | Compiler converged on a wrong fixed-point | Test compiler output against Rust reference for known examples |

**Not in gate (per Q4 decision):**

- Hash comparison (`iface_hash` / `impl_hash`) — looser, allowed nhưng không sufficient.
- IR section-only comparison — không catch caps section bugs.
- Semantic equivalence (run compiler outputs vs reference) — looser, allowed as supplementary test nhưng không primary gate.

## §5 — Stdlib status: Rust-shim builtin approach

**Quyết định:** Self-host compiler dùng **Rust-side builtin opcodes** expose qua `call_builtin <id>` ([ADR-0008 §Builtin ID table](0008-triv-binary-format.md)). KHÔNG viết Triết-native `std.collections.HashMap`/`std.io.fs` etc. cho v0.7. Triết stdlib expansion defer sang v0.8+ hoặc v0.7.x.review nếu cần.

**Builtin IDs reserved cho v0.7** (additive — ADR-0008 §Builtin ID table extends):

| ID | Builtin | Signature |
|---|---|---|
| 4 | `vec_new` | `() -> Vec<T>` |
| 5 | `vec_push` | `(Vec<T>, T) -> Unit` |
| 6 | `vec_get` | `(Vec<T>, Integer) -> T?` |
| 7 | `vec_len` | `(Vec<T>) -> Integer` |
| 8 | `vec_pop` | `(Vec<T>) -> T?` |
| 9 | `vec_iter` | `(Vec<T>) -> Iterator<T>` |
| 10 | `hashmap_new` | `() -> HashMap<K, V>` |
| 11 | `hashmap_insert` | `(HashMap<K, V>, K, V) -> V?` |
| 12 | `hashmap_get` | `(HashMap<K, V>, K) -> V?` |
| 13 | `hashmap_keys` | `(HashMap<K, V>) -> Vec<K>` |
| 14 | `hashmap_contains` | `(HashMap<K, V>, K) -> Trilean` |
| 15 | `read_file` | `(String) -> String?` (None nếu I/O error) |
| 16 | `write_file` | `(String, String) -> Trilean` (True = OK) |
| 17 | `file_exists` | `(String) -> Trilean` |
| 18 | `path_join` | `(String, String) -> String` |
| 19 | `path_parent` | `(String) -> String?` |
| 20 | `path_basename` | `(String) -> String` |
| 21 | `string_substring` | `(String, Integer, Integer) -> String` |
| 22 | `string_split` | `(String, String) -> Vec<String>` |
| 23 | `string_push` | `(String, String) -> String` |
| 24 | `string_index_of` | `(String, String) -> Integer?` (-1 → None) |
| 25 | `parse_integer` | `(String) -> Integer?` |
| 26 | `integer_to_string` | `(Integer) -> String` |

26 builtins. Implement trong `crates/triet-ir/src/vm.rs` `dispatch_builtin()`. Generic-aware (Vec/HashMap parametric trong VM dispatch — Rust impl side dùng `Box<dyn Any>` pattern hiện có).

**Stdlib `.tri` wrappers (optional, defer):** trong v0.7 KHÔNG cần `std.collections.tri` wrapper file. Triết-compiler-in-Triết gọi thẳng `__builtin_vec_new()` etc. Post-v0.7 wrappers ship cùng v0.8 concurrency phase nếu lúc đó stdlib API design xong.

**Lý do (Q5-C):**

- **Scope discipline.** v0.7 deliverable = self-host compiler logic. Self-host stdlib = separate concern. Bundle vào v0.7 → multi-month explosion + 2× debug surface.
- **Implementation symmetry.** Triết-compiler dùng builtin gọi Rust impl. Khi v0.9 JIT lands, builtin lift native → Triết-compiler auto-fast. Khi v2.0 AOT, builtin compile native → same. KHÔNG cần rewrite stdlib.
- **Anti-pattern avoided:** Rust 2014–2015 cố ship `Vec` rewrite cùng self-host → 1 năm regression. ADR-0019 không lặp lại.

**Trade-off accepted:**

- `compiler/*.tri` không "pure" Triết (gọi `__builtin_*`). OK — dev tool, không phải production library. Acceptable.
- Triết stdlib `std.collections` không tồn tại cho user code v0.7. User Triết app vẫn dùng pattern hiện có (function-level, không generic collections). Stdlib gap rõ ràng, document trong [SPEC §0.7 non-goals](../../SPEC.md) khi v0.7 ship.

## §6 — Testing strategy: 3-layer

**Quyết định:** Three concurrent test layers — per-component differential + end-to-end semantic + bootstrap loop. Mỗi layer độc lập catch bug class khác nhau.

### Layer 1 — Per-component differential test

Cho mỗi sub-task v0.7.4 → v0.7.8 (lexer/parser/modules/typecheck/lowerer), thêm test crate riêng:

```
crates/triet-bootstrap/tests/
├── lexer_differential.rs       # Triết-lexer.tripack vs Rust triet-lexer
├── parser_differential.rs      # Triết-parser.tripack vs Rust triet-parser
├── modules_differential.rs     # Triết-modules.tripack vs Rust triet-modules
├── typecheck_differential.rs   # Triết-typecheck.tripack vs Rust triet-typecheck
└── lowerer_differential.rs     # Triết-lowerer.tripack vs Rust triet-ir lowerer
```

Mỗi test:
1. Build Triết-component qua Stage 1 → `.tripack`.
2. Run `.tripack` via VM on every `examples/*.tri` + module-system demo + v0.6 capability test fixtures.
3. Compare output (token stream / AST / type errors / `.triv` bytes) với Rust impl reference.
4. Pass iff byte-identical (cho `.triv`) hoặc structurally equal (token/AST/error).

### Layer 2 — End-to-end semantic test (regression)

Mỗi `examples/*.tri` compile-and-run via Triết-compiler-in-Triết, output ≡ Rust-compiler output. Reuses existing `examples_differential.rs` infrastructure (already 11/11 pass cho interpreter vs VM).

### Layer 3 — Bootstrap loop CI test

`crates/triet-bootstrap/tests/bootstrap_loop.rs`:
1. Stage 1 (Rust) build `compiler/*.tri` → `compiler-stage2.tripack`.
2. Stage 2 (`compiler-stage2.tripack` on VM) build `compiler/*.tri` → `compiler-stage3.tripack`.
3. `cmp compiler-stage2.tripack compiler-stage3.tripack` → must exit 0.

Run in CI on every commit từ sub-task v0.7.11 trở đi. Earlier sub-tasks (v0.7.4–v0.7.10) chạy được nhưng KHÔNG gate ở Layer 3 vì compiler chưa complete.

**Cost:** Bootstrap test takes ~10 min (per Q7 gate). CI runtime increases nhưng acceptable — gate quá quan trọng.

**Lý do (Q6-C):**

- Match cadence v0.3 (per-sub-task differential) + v0.5 (cross-pkg integration) + v0.6 (capability_pipeline.rs capstone).
- Early detection (Layer 1) tránh "v0.7.11 mới phát hiện Stage 2 ≠ Stage 3".
- Three layers catch three failure classes: component bug (Layer 1), semantic regression (Layer 2), nondeterminism (Layer 3).

## §7 — Performance gate recalibration

**Quyết định:** [ROADMAP §v0.7 perf gate](../../ROADMAP.md) *"Performance parity với Rust impl trong vòng 2×"* **defer sang v0.9 (JIT, Cranelift)**. v0.7 gate mới: full Stage 1 → Stage 2 → Stage 3 bootstrap loop hoàn thành **< 10 phút** trên developer hardware (modern laptop, 8-core CPU).

**Lý do recalibrate:**

- Rust impl chạy native (compile-to-machine-code).
- Triết-compiler-in-Triết chạy trên Triết VM, là **development tier** ([VISION §4.3](../../VISION.md)) — bench hiện tại 1.26× tree-walker (KHÔNG phải 1.26× Rust-native). Realistic Triết-on-VM ≈ 50–200× chậm hơn Rust-native cho compiler workload.
- 2× parity gate **không khả thi với VM backend hiện có**. JIT (v0.9 Cranelift) là solution thực — đọc cùng IR, emit machine code → close performance gap.
- Honest expectation > impossible gate.

**v0.7 new gate phrasing (sẽ commit vào ROADMAP.md):**

> *"Self-hosted compiler complete cả 3 stages (Rust → Triết-built-by-Rust → Triết-built-by-Triết) trong < 10 phút trên developer hardware. Bit-identical Stage 2 ≡ Stage 3. Tất cả `examples/*.tri` + module demos + capability tests pass via self-hosted compiler."*

**2× parity gate moves to v0.9:**

> *"Self-hosted compiler + Cranelift JIT backend: bootstrap loop ≤ 2× Rust impl runtime trên same hardware."*

[ROADMAP.md §v0.7](../../ROADMAP.md) + §v0.9 cập nhật trong sub-task v0.7.1 commit.

## §8 — Carry-over từ v0.6: CLI wiring integration

[SPEC §0.7 non-goals của v0.6](../../SPEC.md#07-non-goals-của-v06) defer CLI wiring với note "lands cleaner với v0.7 self-hosting". ADR-0019 fold vào v0.7 scope cụ thể:

| Carry-over item | Sub-task placement |
|---|---|
| `triet check` đọc `triet.package` từ project root | v0.7.10 (CLI integration) |
| `triet build` populate `.tripack` caps section từ manifest | v0.7.10 (CLI integration) |
| Loader integration với `DevTtyPrompt` | v0.7.10 (CLI integration) |
| `E2208.CapabilityDivergence` — fires khi lowerer populate caps section | v0.7.10 (cùng pipeline) |

**Lý do fold v0.7.10:** Triết-compiler-in-Triết phải đọc `triet.package` (chính nó là project!) → manifest discovery convention ép phải decide ở v0.7. Lý do v0.6 defer = exactly đây. Sub-task v0.7.10 chốt convention + ship trong Rust impl side trước, sau đó Triết-side dùng cùng convention.

**Project layout convention** (locked):

```
<project-root>/
├── triet.package           # ADR-0018 §1 source manifest (REQUIRED for build)
├── triet.lock              # ADR-0015 §6 lockfile (REQUIRED for build, auto-generated)
├── triet.policy            # ADR-0017 §3 policy rules (OPTIONAL — fallback to default)
├── src/
│   ├── main.tri            # entry point
│   └── ...
└── ...
```

`triet check` / `triet build` / `triet run` walk upward từ `cwd` tìm `triet.package` (mirrors `cargo` discovery pattern). Nếu không có → error E2208.ManifestMissing (new sub-variant, additive to E2208).

## Hệ quả

### Cho ADR-0007 (IR design)

Không đổi. Self-hosting verifies IR shape stable — Triết-impl emit cùng IR, Rust-impl emit cùng IR, both decode cùng VM.

### Cho ADR-0008 (`.triv` wire format)

Builtin ID table extends additively (4–26 added per §5). Wire format `v3` unchanged. v3 reader sees new builtin IDs → unknown builtin error E2105 hiện có handles gracefully.

### Cho ADR-0011 (ABI metadata)

Không đổi. Canonical encoding (§6) đã promise sort-by-name → self-hosting test verifies invariant. ADR-0019 §3 is stricter version of same promise.

### Cho ADR-0014/0015 (CAS)

Không đổi. CAS scheme đã canonical → bootstrap byte-identical gate compatible by construction.

### Cho ADR-0016/0017/0018 (capability)

Self-hosted parser cho `triet.package` + `triet.policy` phải emit byte-identical errors với Rust impl per ADR-0018 §3 format table. Đã locked ở ADR-0018; v0.7 verifies it.

### Cho `triet-cli`

Project layout discovery (§8) lands trong v0.7.10. Subcommands `triet check` / `triet build` / `triet run` walk-upward-find-manifest convention.

### Cho stdlib expansion

Defer-rest-of-stdlib sang post-v0.7. v0.8 concurrency hoặc v0.7.x.review picks up `std.collections` Triết-native wrapper nếu cần. Builtin opcodes (§5) là contract — wrappers thin layer trên builtin, không re-design API.

### Cho v0.9 JIT

ADR-0019 §7 perf gate "2× parity" defer to v0.9. v0.9 phase open với clear target: Cranelift JIT đọc cùng IR, emit machine code, bootstrap loop ≤ 2× Rust impl runtime.

### Cho v2.0 AOT (LLVM)

AOT backend cũng đọc cùng IR. Self-hosted compiler ở v0.7 = source-of-truth cho IR emit. v2.0 LLVM backend integration: thay Cranelift bằng LLVM ở compile path; KHÔNG đụng Triết-in-Triết compiler logic.

### Cho v3.0 microkernel

Self-hosted compiler = prerequisite cho microkernel POC. Khi v3.0 needs Triết kernel code compile to native, compiler-in-Triết đã exists từ v0.7 → recompile chính nó qua v2.0 AOT backend → kernel binary.

## Không làm

- **Native AOT emit ở v0.7.** ROADMAP §v0.7 đã promise "vẫn xuất bytecode v0.3" → giữ nguyên. LLVM backend là v2.0.
- **JIT integration ở v0.7.** Cranelift = v0.9. ADR-0019 §7 perf gate recalibrated accordingly.
- **Triết-native `std.collections`/`std.io.fs`** ở v0.7. Builtin opcodes (§5) là solution. Stdlib expansion = v0.8+ scope.
- **Macro / metaprogramming.** Tăng surface area + delay self-host. Defer post-v1.0.
- **Cross-compile.** Triết-on-VM hardware-independent. AOT cross-compile = v2.0.
- **Incremental compilation cache.** Useful nhưng orthogonal. Defer v0.9+.
- **Parallel compilation.** Threading = v0.8 concurrency model. v0.7 single-threaded.
- **Stage 4 sanity** as gate. Only debug aid if Stage 2 ≢ Stage 3 fails.
- **Triết-impl divergent from Rust-impl semantics.** Goal là 1:1 reimplementation. KHÔNG "improve" lexer / parser / etc. while rewriting. Refactor lands separately post-v0.7.
- **Big-bang rewrite.** §2 explicitly rejects. Bottom-up incremental.
- **Removing Rust impl post-v0.7.** Rust impl stays as Stage 1 bootstrap loader cho future bootstrap loops (especially when v2.0 AOT backend lands). Rust impl tier = "boot ROM" cho Triết compiler ecosystem.
- **Loosen bit-identical gate to hash-only.** Q4 đã chọn full bytes. Hash collision unlikely nhưng gate là contract.
- **English-only error messages requirement.** Triết-impl phải emit byte-identical error strings as Rust-impl per ADR-0018 §3 format. ADR-0019 không re-decide format.

## Sub-task plan v0.7.1 → v0.7.13

Outline. Per-sub-task design questions (3-5 A/B/C) lands khi sub-task open per author cadence.

| Sub-task | Description | Crate(s) touched |
|---|---|---|
| **v0.7.1** | ADR-0019 land + ROADMAP §v0.7 recalibrate + ADR index update | `docs/`, `ROADMAP.md` only |
| **v0.7.2** | Canonical emission invariants audit + lock + CI test `bootstrap_determinism` | Rust impl audit; new `crates/triet-bootstrap/` skeleton |
| **v0.7.3** | Builtin opcodes 4–26 land trong VM dispatcher (Rust-shim) | `triet-ir` (VM + serde), `triet-cli` for testing |
| **v0.7.4** | `compiler/lexer.tri` + lexer_differential test (umbrella: 4 sub-commits — see §A7.4 v0.7.4 sub-task breakdown below) | New `compiler/` dir, new `crates/triet-bootstrap/tests/lexer_differential.rs` |
| **v0.7.5** | `compiler/parser.tri` + parser_differential test | `compiler/parser.tri`, parser_differential.rs |
| **v0.7.6** | `compiler/modules.tri` + modules_differential test | `compiler/modules.tri`, modules_differential.rs |
| **v0.7.7** | `compiler/typecheck.tri` + typecheck_differential test | `compiler/typecheck.tri`, typecheck_differential.rs |
| **v0.7.8** | `compiler/ir_lowerer.tri` + lowerer_differential test | `compiler/ir_lowerer.tri`, lowerer_differential.rs |
| **v0.7.9** | `compiler/pack_writer.tri` + `compiler/main.tri` + wire all components in Triết (drop bridges) | `compiler/`, end-to-end test |
| **v0.7.10** | CLI wiring carry-over: project layout discovery + `triet check/build/run` cap-aware + DevTtyPrompt loader integration + E2208.CapabilityDivergence fires | `triet-cli`, `triet-pack` (loader) |
| **v0.7.11** | Stage 1 → Stage 2 bootstrap script + CI integration | `crates/triet-bootstrap/tests/bootstrap_loop.rs` Stage 2 only |
| **v0.7.12** | Stage 2 → Stage 3 + bit-identical gate verify in CI | `bootstrap_loop.rs` full 3-stage + `cmp` assertion |
| **v0.7.13** | Verify gate (ADR-0009 §A/B/C/D) + bump 0.6.0 → 0.7.0 + docs sync (SPEC v0.7, README, CLAUDE.md) | Version + docs |

Estimated cadence: 12+ tháng (matches [ROADMAP §Pace expectations](../../ROADMAP.md)).

## Prior art

- **[rustc bootstrap](https://rustc-dev-guide.rust-lang.org/building/bootstrapping/intro.html)** — Stage 0/1/2 model. Direct inspiration cho §1 3-stage chain. rustc Stage 0 = previous stable rustc binary; Stage 1 = compiler built by Stage 0; Stage 2 = compiler built by Stage 1; gate = Stage 1 ≡ Stage 2 (skip Stage 3 in their model). Triết mirrors but explicit Stage 3 since Stage 1 Rust impl is permanent loader, not previous-stable-Triết.
- **[OCaml bootstrap (`boot/ocamlc`)](https://github.com/ocaml/ocaml/tree/trunk/boot)** — Committed bootstrap compiler in repo. Closer precedent — Stage 0 binary committed. Triết Stage 0 = Rust impl (always exists in repo), không cần commit binary.
- **[GCC bootstrap (`make bootstrap`)](https://gcc.gnu.org/install/build.html)** — 3+ stage with bit-identical Stage 2 ≡ Stage 3 gate. Direct precedent cho §1 + §4.
- **[Go bootstrap](https://go.dev/blog/rebuild)** — Go 1.5+ self-hosted via Go 1.4 bootstrap binary. Pattern: previous-stable-as-loader. Similar to rustc.
- **[TinyCC self-compile](http://savannah.nongnu.org/projects/tinycc)** — Single-stage simplicity. Anti-prior-art: too lax for production quality gate.
- **[Rust 2014 stdlib rewrite alongside self-host](https://github.com/rust-lang/rust/issues/15046)** — Anti-prior-art. Big-bang rewrite + stdlib expansion concurrent → 12+ months regression. ADR-0019 §5 explicitly rejects this pattern (Q5-C decision).

**Anti-prior-art:**

- **CPython 3.x self-host attempts via PyPy** — Performance gate (2× CPython) drove design compromise. ADR-0019 §7 explicitly defers perf gate to v0.9 to avoid this.
- **GraalVM Native Image polyglot** — Multi-language interop scope creep. ADR-0019 single-target (Triết only) keep scope tight.

## Tham chiếu

- [VISION §4 (multi-backend trajectory)](../../VISION.md) — IR is the contract, backend là implementation. Self-hosting verifies IR stability.
- [VISION §6 (Stability over speed)](../../VISION.md) — drives bottom-up incremental (§2) + bit-identical gate (§4).
- [SPEC §0.7 non-goals của v0.6](../../SPEC.md#07-non-goals-của-v06) — CLI wiring carry-over justification.
- [ROADMAP §v0.7](../../ROADMAP.md) — original deliverables + gate (recalibrated by ADR-0019 §7).
- [ROADMAP §Pace expectations](../../ROADMAP.md) — 12+ tháng estimate.
- [ADR-0007](0007-ir-design.md) — IR shape (unchanged).
- [ADR-0008](0008-triv-binary-format.md) — `.triv` wire format (builtin IDs extended additively per §5).
- [ADR-0009](0009-version-gate-policy.md) — version gate policy applied to v0.7 (§A/B/C/D in sub-task v0.7.13).
- [ADR-0011](0011-abi-metadata-format.md) — ABI metadata canonical encoding (precondition cho §3 + §4).
- [ADR-0014](0014-hash-scheme-refinement.md) — CAS canonical encoding (precondition).
- [ADR-0016](0016-capability-type-system.md) / [ADR-0017](0017-trilean-policy-hook.md) / [ADR-0018](0018-capability-loader-semantics.md) — capability semantics preserved; CLI wiring carry-over folds in §8.
- TODO.md (will track v0.7.1 → v0.7.13 sub-tasks as they open).

---

*Quyết định này lock bootstrap chain + emission invariants + stdlib strategy + testing strategy + perf gate cho phase v0.7. Breaking change ở bất kỳ §1–§8 cần ADR mới supersede. Sub-task v0.7.2+ implements decisions; mỗi sub-task có per-step design questions theo author cadence.*

---

## Addendum — v0.7.3 (Rust-shim builtin scaffolding)

Lock 4 decisions surfaced during v0.7.3.1 sub-task open, mirror precedent [ADR-0015 Addendum](0015-package-store-layout.md#addendum--v05xreview-pre-v06-audit) + [ADR-0018 Addendum](0018-capability-loader-semantics.md#addendum--v06xreview-pre-v07-audit). Plus author naming-convention constraint locked 2026-05-17.

### A1 — Collection types are first-class `TypeTag` variants

Original ADR §5 promised "wire format v3 unchanged" + only opaque struct shells for Vec/HashMap. Sub-task v0.7.3.1 design discovery (Q1) reframed: opaque shells would force VM to special-case path strings (`std.collections.Vec`) and break the abstraction promise of [ADR-0007](0007-ir-design.md) — every IR value carries an explicit `TypeTag`. Promotion to first-class variants reuses existing generic-function machinery (proven via `examples/generic.tri`).

**Lock:** `TypeTag::Vector(Box<TypeTag>)` + `TypeTag::HashMap(Box<TypeTag>, Box<TypeTag>)`. Wire format discriminants `8` (Vector, post-order single inner ref) + `9` (HashMap, post-order key + value refs).

**Wire format bump: v3 → v4.** This is a **patch bump** per [ADR-0008 §"Version bump rules"](0008-triv-binary-format.md): *"new opcodes or type discriminants added; old readers skip unknown opcodes (error at runtime, not at load time)"*. Pre-v4 readers encountering discriminant 8 or 9 emit `TrivError::UnknownTypeDiscriminant` (mapped to E2104). No ADR-0008 rewrite required — the existing patch-bump rule explicitly covers additive type-table extension.

### A2 — `RuntimeValue` collection variants + `RuntimeMapKey` discipline

VM-side mirror: `RuntimeValue::Vector(Vec<Self>)` + `RuntimeValue::HashMap(BTreeMap<RuntimeMapKey, Self>)`. **`BTreeMap`** (not `HashMap`) — aligns with [ADR-0019 §3 canonical emission principle](#3--canonical-emission-invariants-deterministic-output): deterministic iteration order is mandatory once the self-host compiler starts serializing collection contents. Bonus: ordering enables future content-hashing.

`RuntimeMapKey` enum restricts map keys to hashable primitives (Trit/Tryte/Integer/Long/String). Vector/HashMap/Struct/Enum/Closure/Unit/Null/Trilean **cannot** be keys at v0.7.3 — refuse-over-guess. Trilean specifically excluded because Ł3 `Unknown` semantics make equality undecidable; allowing it as key would silently coerce. Future ADR may revisit once concurrency model (v0.8) decides equality discipline.

### A3 — `vec_*` → `vector_*`, `vec_len` → `vector_length` (no abbreviations)

**Author constraint locked 2026-05-17:** Triết-facing identifiers (TypeTag variants, BuiltinName variants, stdlib `.tri` paths, parameter names) spell out fully — Java naming convention, never abbreviate. Rationale: *"tôi rất dị ứng với viết tắt"* — Java developer mental model carries through SPEC §0.3 AI-first principle (explicit > terse).

**Rename table (overrides §5 original ADR-0019 spec):**

| ADR-0019 §5 (original) | v0.7.3 Addendum (corrected) |
|---|---|
| `vec_new` | `vector_new` |
| `vec_push` | `vector_push` |
| `vec_get` | `vector_get` |
| `vec_len` | **`vector_length`** (len → length) |
| `vec_pop` | `vector_pop` |
| `vec_iter` | **`vector_iterator`** (iter → iterator) |
| `Vec(Box<TypeTag>)` | **`Vector(Box<TypeTag>)`** |
| `BuiltinName::VecNew` | `BuiltinName::VectorNew` |

`HashMap` retains its name (Java `java.util.HashMap` — not an abbreviation). `string_*` / `path_*` / `parse_*` keep verbal form (not abbreviations).

**Rust-internal code excluded:** `Vec<T>` (Rust stdlib), `Box<>`, `Arc<>`, `Rc<>` — Rust idioms in Rust impl crates stay. ADR-0019 §3 audit (v0.7.2) did **not** retroactively rename `func_table` / `pkg_name` / `meta` — CLAUDE.md "Surgical Changes" applies.

### A4 — Drop duplicate builtin IDs 23 + 26 (Q5-A) + Vector mutation + iterator (v0.7.3.2 Q1-A / Q2-A)

Discovery during sub-task open: ADR-0019 §5 IDs 23 `string_push` + 26 `integer_to_string` duplicate existing stdlib builtins (`std.text.concat` + `std.text.from_integer`). Triết strings are immutable — `string_push` semantically ≡ `concat`. Triết stdlib stays minimal per [VISION §6 "explicit > implicit"](../../VISION.md).

**v0.7.3.2 design review added two more drops:**

- **`vector_pop` dropped (Q1-A consequence).** Q1-A picked functional return-new for `vector_push` (SSA-safe, parallelism-friendly). `vector_pop`'s natural signature `(Vector<T>) -> (Vector<T>, T?)` requires tuple return — Triết IR doesn't have first-class tuples in opcodes yet. Self-host compiler doesn't need pop (symbol tables grow monotonically). Defer post-v1.0 alongside slice support.
- **`vector_iterator` dropped (Q2-A).** ADR-0003 Iterator trait specced but not implemented at Triết level. Self-host compiler uses explicit `for i in 0..vector_length(v) { vector_get(v, i)!! }` pattern. Iterator trait scoping ⇒ separate ADR when concurrency model (v0.8) reframes adapter chains.

**Lock (final v0.7.3 dropped list):**

| Dropped | Reason | Use instead |
|---|---|---|
| ID 8 `vector_pop` | Q1-A functional semantic requires tuple return; Triết IR lacks tuple opcodes | Defer post-v1.0 |
| ID 9 `vector_iterator` | Q2-A — ADR-0003 trait not implemented; explicit index loop suffices | `for i in 0..length(v) { get(v, i)!! }` |
| ID 23 `string_push` | Strings immutable → ≡ `concat` | Existing `std.text.concat` |
| ID 26 `integer_to_string` | Duplicate of existing | Existing `std.text.from_integer` |

Self-host compiler (v0.7.4+) consumes existing stdlib paths — no source-side change in `compiler/*.tri`. **19 net-new builtins** (was 23 in original spec; 21 after string/integer dedup; 19 after vector mutation/iterator drop).

### A4.1 — Wire format builtin ID assignments (v0.7.3.2 actual)

ADR-0019 §5 builtin ID table had wire-ID conflicts with pre-existing extensions (`FStringConcat`=4, `TextLen`=5, `TextConcat`=6, `TextFromInteger`=7 had already shipped pre-v0.7.3 — original ADR-0019 §5 mistakenly assumed IDs 4–26 were available). Corrected assignments below; pre-v0.7.3.2 readers encountering ID 8+ emit `TrivError::UnknownBuiltin` (no `.triv` version bump per ADR-0008 §"Version compatibility" — `CallBuiltin` opcode byte unchanged, only operand-byte values grow additively).

| ID | Builtin | Phase |
|---|---|---|
| 0–7 | pre-existing (Println..TextFromInteger) | pre-v0.7.3 |
| **8** | `VectorNew` | **v0.7.3.2 (shipped)** |
| **9** | `VectorPush` | **v0.7.3.2 (shipped)** |
| **10** | `VectorGet` | **v0.7.3.2 (shipped)** |
| **11** | `VectorLength` | **v0.7.3.2 (shipped)** |
| **12** | `HashMapNew` | **v0.7.3.3 (shipped)** |
| **13** | `HashMapInsert` (functional return-new per Q1-A) | **v0.7.3.3 (shipped)** |
| **14** | `HashMapGet` | **v0.7.3.3 (shipped)** |
| **15** | `HashMapKeys` (sorted per Q4-A) | **v0.7.3.3 (shipped)** |
| **16** | `HashMapContains` (strict 2-state Trilean per Q3-A) | **v0.7.3.3 (shipped)** |
| **17** | `ReadFile` (data tier: missing/I-O-error → Null) | **v0.7.3.4 (shipped)** |
| **18** | `WriteFile` (Q4-A strict 2-state Trilean) | **v0.7.3.4 (shipped)** |
| **19** | `FileExists` (strict 2-state) | **v0.7.3.4 (shipped)** |
| **20** | `PathJoin` (Q2-A POSIX `/`, deterministic) | **v0.7.3.4 (shipped)** |
| **21** | `PathParent` (Null if root/empty/no-sep) | **v0.7.3.4 (shipped)** |
| **22** | `PathBasename` | **v0.7.3.4 (shipped)** |
| **23** | `StringSubstring` (Q3-A char-index, OOB panic E2206) | **v0.7.3.4 (shipped)** |
| **24** | `StringSplit` (empty-separator returns single-element `[s]`) | **v0.7.3.4 (shipped)** |
| **25** | `StringIndexOf` (char-offset, Null on miss) | **v0.7.3.4 (shipped)** |
| **26** | `ParseInteger` (strict decimal, Null on any failure) | **v0.7.3.4 (shipped)** |

### A5 — Sub-task split v0.7.3.1 → v0.7.3.4

Per Q2-B (4-sub-commit cadence for the v0.7.3 umbrella):

| Sub-task | Scope | Status |
|---|---|---|
| **v0.7.3.1** | TypeTag + RuntimeValue + wire format v4 + this Addendum | shipped |
| **v0.7.3.2** | Vector builtins (IDs 8–11, 4 ops post-Q1A/Q2A drops) — VM dispatch + smoke + composition test. Stdlib stubs + path_to_builtin defer until generic-function syntax lands (v0.7.4+). | shipped |
| **v0.7.3.3** | HashMap builtins (IDs 12–16, 5 ops) — VM dispatch + smoke + composition + invalid-key panic test. ADR-0019 Addendum §A4.1 IDs marked shipped. Locks error-model 3-tier discipline (lookup miss = data event, invalid key = bug panic). | shipped |
| **v0.7.3.4** | IO + path + string builtins (IDs 17–26, 10 ops post-dedup). Q1-A capability gating deferred §A7 (lands v0.7.10 CLI wiring). Q2-A POSIX-only path semantic (deterministic for bootstrap byte-identical gate). Q3-A char-index string slicing with OOB panic. Q4-A `tempfile` crate for IO tests. **Closes v0.7.3 umbrella — 19 net-new builtins total across 4 sub-tasks.** | shipped |

### A6 — IO Trilean shape (Q4-A): strict 2-state

`file_exists` / `write_file` builtins return `Trilean` per ADR-0019 §5 signatures. **Locked semantic:** strict 2-state — `True` / `False` only; `Unknown` never returned from I/O builtins. Permission denied, race condition, EBUSY → collapse to `False` (matches spec "None nếu I/O error"). Rationale: I/O semantically binary; Triết-native Ł3 philosophy doesn't apply cleanly to "did syscall succeed".

Future-compat: if v0.8 actor model introduces async-pending I/O state, that's a new ADR. v0.7.3 IO builtins stay strict.

### Test coverage scorecard

**v0.7.3.1 (shipped):**

| Layer | Test |
|---|---|
| TypeTag display | `vector_type_display` + `hashmap_type_display` + `collection_equality` |
| Wire format | `wire_format_version_bumped_to_v4` |
| Round-trip | `vector_and_hashmap_type_tags_round_trip` (nested Vector + flat HashMap) |
| Forward-compat | `pre_v4_reader_refuses_vector_discriminant` (v4-aware reader accepts; documents pre-v4 refusal contract) |

**v0.7.3.2 (shipped):**

| Layer | Test |
|---|---|
| Smoke | `vm_vector_new_returns_empty_vector` |
| Smoke | `vm_vector_push_appends_and_returns_new_vector` (functional return-new) |
| Smoke | `vm_vector_get_in_range_returns_element_out_of_range_returns_null` (covers in-range + over-length + negative — Q3-A strict bounds) |
| Smoke | `vm_vector_length_returns_element_count` (covers length=3 + length=0 empty) |
| Composition | `vm_vector_compose_push_length_get_round_trip` — build 3-element vector, sum get(0)+get(2)=400 |

**v0.7.3.3 (shipped):**

| Layer | Test |
|---|---|
| Smoke | `vm_hashmap_new_returns_empty_map` |
| Smoke | `vm_hashmap_insert_returns_new_map_with_pair` (Q1-A functional return-new) |
| Smoke | `vm_hashmap_get_hit_returns_value_miss_returns_null` (data tier: miss → Null, not panic) |
| Smoke | `vm_hashmap_keys_returns_sorted_vector` (Q4-A BTreeMap natural order) |
| Smoke | `vm_hashmap_contains_returns_strict_trilean` (Q3-A strict 2-state — hit→True, miss→False, never Unknown) |
| Error model | `vm_hashmap_invalid_key_type_panics_with_type_mismatch` (Q2-B: Vector as key → E2201 panic, NOT silent Null. Locks bug-tier vs data-tier discipline.) |
| Composition | `vm_hashmap_compose_insert_contains_get_keys_round_trip` — build 3-entry map, get("middle") = 300 |

**v0.7.3.4 (shipped):**

| Layer | Test |
|---|---|
| Smoke (IO) | `vm_read_file_write_file_round_trip` (write → read invariant + Q4-A True confirmation) |
| Smoke (IO) | `vm_read_file_missing_path_returns_null` (data tier: missing file = Null, not panic) |
| Smoke (IO) | `vm_file_exists_strict_trilean` (present → True, missing → False, never Unknown) |
| Smoke (path) | `vm_path_join_posix_semantic` (3 cases: normal / trailing slash / empty base) |
| Smoke (path) | `vm_path_parent_returns_parent_or_null` (3 cases: normal / root `/` / no-sep) |
| Smoke (path) | `vm_path_basename_last_segment` (3 cases: normal / trailing slash / no-sep) |
| Smoke (string) | `vm_string_substring_char_index_multibyte_safe` (ASCII + Vietnamese "Việt" + empty range) |
| Error contract | `vm_string_substring_out_of_bounds_panics` (Q3-A: end>length, negative start, start>end all panic E2206) |
| Smoke (string) | `vm_string_split_returns_vector` (normal + empty-separator refuse-over-guess) |
| Smoke (string) | `vm_string_index_of_char_offset_or_null` (found ASCII + Vietnamese codepoint offset + miss → Null + empty needle) |
| Smoke (string) | `vm_parse_integer_strict_decimal` (positive + negative + empty + non-digit + leading whitespace refuse) |
| Composition | `vm_compose_read_split_parse_accumulate` — lexer-like flow: read tempfile → split by `\n` → parse each line → accumulate to Vec |

**v0.7.3 umbrella totals:** 1088 → 1118 tests (30 net-new across v0.7.3.1+2+3+4: 6 + 5 + 7 + 12), clippy `-D warnings` clean. **19 net-new builtins** ship across IDs 8–26. Phase closes with all 4 sub-tasks complete.

### A7 — Deferred items log (technical debt surfaced by v0.7.3)

Consolidated list of every item punted by v0.7.3.1 + v0.7.3.2 decisions, with target re-tackle phase. Mirrors precedent ([ADR-0015 §9 lossy migration log](0015-package-store-layout.md), [SPEC §0.7 non-goals của v0.6](../../SPEC.md#07-non-goals-của-v06)). Future contributors checking "what's missing" land here.

| Deferred item | Reason | Target phase |
|---|---|---|
| ~~**Stdlib `.tri` stubs for Vector builtins**~~ (`std/collections/vector.tri`, `std/collections/hashmap.tri`, `std/io/fs.tri`, `std/path.tri`, `std/string.tri`) | ~~Unblocked by v0.7.4.1.~~ | **Shipped in v0.7.4.2.** 5 new stdlib files + `std/text.tri` extended with `parse_integer`. Java-aesthetic per-namespace organization (no module-name repetition in function names). |
| ~~**`path_to_builtin` entries for Vector/HashMap/IO/path/string ops**~~ | ~~Unblocked by v0.7.4.1.~~ | **Shipped in v0.7.4.2.** 19 entries added to `vm.rs::path_to_builtin`. |
| **Interpreter parity for v0.7.3 builtins** (`Vector`/`HashMap`/IO/path/string ops not callable via tree-walking interpreter) | `triet-interpreter::builtins::install` only registers the v0.2 prelude (print/println/length/assert/...). The 19 v0.7.3 builtins are VM-dispatched via `path_to_builtin`; interpreter has no equivalent intercept. v0.7.4.2 stdlib stubs work via `triet build` + `triet run .triv` (VM path) but `triet run file.tri` (interpreter path) fails with `UndefinedName`. | **post-v0.7** — VISION §4.3 marks interpreter as development tier; self-host compiler doesn't need it once VM path covers all examples. Bridging would require duplicating 19 builtin implementations in `triet-interpreter::builtins.rs`. Either ship that parity in `v0.7.x.review`, OR drop interpreter entirely when v0.9 JIT lands (faster than tree-walker anyway). |
| ~~**Generic function syntax in AST/parser**~~ (`function vector_new<T>() -> Vector<T>`) | ~~`FunctionDef` struct lacks `type_params` field; parser does not consume `<T>`.~~ | **Shipped in v0.7.4.1** (this sub-task). Parser + AST + typecheck (Rust-style inference per Q2-A) + lowerer all wired. **Deviation from Q3-A locked in §A7.1 below: lowerer uses type erasure** (TypeTag::Unit for generic param slots) instead of clone-per-instantiation. |
| **`vector_pop(v) -> (Vector<T>, T?)`** | Functional return-new semantic (Q1-A) requires tuple return; Triết IR lacks first-class tuple opcodes. Self-host compiler doesn't need pop (symbol tables grow monotonically). | post-v1.0 — alongside tuple opcode + slice support |
| **Tuple opcodes in IR** (`TupleNew`, `TupleGet`, `TupleLength`) | Triết has tuple values in SPEC §8 but no IR opcodes — current lowerer represents tuples via struct workaround. Blocks `vector_pop`, multi-return functions, structural pattern matching. | post-v1.0 (post-self-host, when language stability allows IR additions) |
| **`vector_iterator(v) -> Iterator<T>`** | ADR-0003 Iterator trait specced but never implemented at Triết level (v0.2 deliverable did not land; see ADR-0003 *Implementation roadmap* table). | Lands with ADR-0003 implementation — likely v0.8 (concurrency model reframes iterator+stream protocols) |
| **`Iterator<T>` / `Iterable<T>` traits in stdlib + user-extensible iterator protocol** | ADR-0003 status: locked but not implemented. v0.1 hardcoded `Range`+`Enumerate` still in use; refactor to trait pending. | v0.8 (revisit alongside concurrency primitives) or earlier if v0.7.x sub-task forces it |
| **`vector_iterator` adapter chains** (`map`/`filter`/`take`/`zip`) | Depends on Iterator trait. | Same as Iterator trait above |
| **Error handling primitive — recovery / try-catch / supervisor** | Triết currently has **no mechanism** for user code to catch runtime panics. `VmError` (E22XX) aborts execution; only domain errors (`T?`, `Result<T, E>`, `Trilean::Unknown`) are recoverable. v0.7.3.3 surfaced this via Q2-B: invalid HashMap key types → panic, not recovery. Decision locked because self-host compiler doesn't need recovery (bugs are bugs). But future application code, actor supervisors (v0.8), and microkernel boundary (v3.0) will. | **Future ADR-0020 candidate** — "Error handling discipline: panic vs Result vs Trilean, recovery story". Likely v0.8 alongside concurrency model (actor supervisor patterns force the question). Write ADR-0020 when v0.8 phase opens or when an earlier sub-task demands recovery. |
| **IO builtin capability gating** (`sys.fs.read`, `sys.fs.write` etc.) | v0.7.3.4 Q1-A: `ReadFile`/`WriteFile`/`FileExists` dispatch directly via `std::fs::*` with no `CapabilityResolver` consultation. Self-host compiler bootstrap context is trusted, but future user code calling these builtins must go through v0.6 capability machinery. | **v0.7.10** — paired with CLI wiring carry-over (ADR-0019 §8 project layout discovery). `triet run` flow will resolve `sys.fs.*` capabilities against root manifest before instantiating the VM. |
| **Windows path semantics** | v0.7.3.4 Q2-A: `PathJoin`/`PathParent`/`PathBasename` hardcode POSIX `/` separator for byte-identical bootstrap determinism. Windows backslash + drive-letter handling deferred. Matches existing POSIX-first precedent ([ADR-0018 DevTtyPrompt POSIX-only](0018-capability-loader-semantics.md)). | **post-v1.0** — alongside Windows ConPTY TTY support and broader cross-platform pass. Currently no Triết user has demanded it; bootstrap loop must be byte-identical, and per-OS-variant output would break that. |
| **IO write atomicity** (`WriteFile` is not atomic — partial writes possible on crash mid-write) | v0.7.3.4 uses `std::fs::write` which is **not** atomic. Crash between truncate and full write leaves a truncated/empty file. Self-host compiler bootstrap doesn't need atomicity (any crash invalidates the build run). | **post-v0.7** — when first user-facing application demands it. Pattern would mirror [ADR-0015 §5 atomic install protocol](0015-package-store-layout.md) (write to tmp + rename). |
| **`StringSubstring` byte-index variant** | v0.7.3.4 Q3-A: only char-index version shipped (Vietnamese-safe). Byte-index could be ~100× faster for ASCII-heavy code paths. Self-host compiler doesn't benchmark that hot yet. | **post-v0.9 JIT** — measure self-host compiler hot path; add `StringSubstringBytes` builtin only if profiled bottleneck. Refuse-over-guess: don't add until evidence demands. |
| **Outcome type implementation** (`T~E` / `T?~E` parser + AST + typecheck + lowerer + VM dispatch + tests + SPEC §2.5 rewrite) | ADR-0020 docs-only at v0.7.4.3-error. Full implementation lifts the design from ADR to working code: lexer accepts `~+`/`~0`/`~-`/`?~` compound tokens, parser builds `TypeTag::Outcome` + new AST nodes, typecheck enforces exhaustiveness + closure-capture form, lowerer emits opcodes 0xC1–0xC6, VM dispatches outcome ops with mandatory deallocation contract. | **v0.7.4.3-error sub-task** (next sub-commit in v0.7.4.3 umbrella) — implementation phase of ADR-0020. Estimated 2-3 weeks. |
| **`null` keyword deprecation (W2001) + `~0` literal acceptance** | ADR-0020 §10 unifies `null` and `~0` source syntax. Lexer accepts both tokens; parser produces same AST node; typecheck emits W2001 for every `null` site with fix-hint. Removal scheduled v1.0 (W2001 → E2002). | **v0.7.4.3-error sub-task** (same sub-commit as outcome impl above) |
| **`triet fmt --fix --migrate-null` migration tool** | Token-level rewrite of `null` → `~0` recursively. Idempotent, in-place by default with `--dry-run` option, respects .gitignore. ADR-0020 §10.5 spec. | **v0.7.4.3-error sub-task** (same sub-commit) |

**Maintenance rule (per author 2026-05-17 feedback):** When future v0.7.x.review audits identify additional deferred items, append to this table. When a deferred item ships, mark with strikethrough + commit hash rather than removing — preserves the history of *what was once missing* for future readers.

This Addendum also commits ADR-0019 §3 emission-determinism scope to mention the Vector/HashMap collection types: `add_type` now recurses into `Vector`/`HashMap` for post-order encoding, preserving canonical type-table layout across rebuilds. The v0.7.2 `bootstrap_determinism` test continues to cover this transparently (Vector/HashMap not yet exercised by `examples/*.tri` — when v0.7.4+ adds `compiler/*.tri` source, the test will gain coverage automatically).

### A7.1 — v0.7.4.1 deviation from Q3-A (lowerer monomorphization strategy)

Original v0.7.4.1 design Q3-A locked **clone-per-instantiation** for generic functions: emit separate IR `FuncId`s per unique `(function, type_args)` tuple, mirror Rust monomorphization. v0.7.4.1 implementation **deviates to type erasure at IR level** — `TypeTag::Unit` placeholder for generic param slots, single `FuncId` shared across all instantiations.

**Reasons for deviation:**

- True clone-per-instantiation requires the lowerer to re-do typecheck's call-site inference (re-extract type_params from arg types). Typecheck currently doesn't pipe inferred concrete types to the lowerer, so duplicating logic would couple the two passes invasively.
- For v0.7.4.1's primary use case (unblock stdlib stubs + self-host compiler), type erasure produces semantically correct programs: builtins (`Vector*`/`HashMap*` etc.) are VM-dispatched on name, not `TypeTag`; user-defined generic functions like `identity<T>(x) = x` flow values through registers without depending on TypeTag.
- `RuntimeValue::Vector(Vec<Self>)` is heterogeneous at runtime anyway — element TypeTag is erased post-lowering.
- Determinism is preserved: same source → same erased FuncIds → byte-identical IR (in fact erasure is *more* deterministic than monomorphization, which would need careful hash-stable cache iteration order).

**What this costs:**

- IR loses static type info for generic param slots (placeholder `TypeTag::Unit`). VM doesn't care; future LLVM AOT (v2.0) might benefit from concrete types for optimization.
- IR verifier sees `TypeTag::Unit` where the source had `T` — looks weird in `triet inspect` dumps for generic functions.
- Cannot specialize per-instantiation optimizations (e.g. inlining `vector_new<Integer>` differently from `vector_new<String>`).

**Re-tackle path:**

True clone-per-instantiation lands when **v2.0 LLVM AOT** backend demands it for inlining. At that point we'll need to:

1. Pipe typecheck's call-site inferences (concrete type args per CallExpr span) through to the lowerer.
2. Refactor lowerer's Pass 1 to defer generic-function FuncId allocation.
3. Add `(AbsolutePath, Vec<TypeTag>) -> FuncId` cache + on-demand body instantiation with type substitution.

Tracked in §A7 deferred items log under a future "Generic function monomorphization (Q3-A true semantics)" entry — added below for transparency.

| **Generic function clone-per-instantiation** (Q3-A true monomorphization) | v0.7.4.1 ships type-erased generic functions (§A7.1). Acceptable for VM dev tier but loses static type info for backend optimization. | **v2.0 LLVM AOT** (when concrete types matter for inlining + specialization) |

### A7.2 — v0.7.4.1 test scorecard

| Layer | Test |
|---|---|
| Parser | `parses_function_with_single_type_param` (`function identity<T>(x: T) -> T = x`) |
| Parser | `parses_function_with_multiple_type_params` (`function pair<K, V>(k: K, v: V) -> K`) |
| Parser | `parses_function_without_type_params_has_empty_type_params` (regression guard) |
| Typecheck | `checks_generic_identity_function` (single `T`, inferred via Integer + String call contexts) |
| Typecheck | `checks_generic_function_with_two_params` (`K`/`V` independent inference) |
| End-to-end | `diff_generic_function` — `examples/generic_function.tri` parses → typechecks → lowers → runs byte-identical VM vs interpreter (joins existing 11 examples → 12) |

1118 → 1124 tests (6 net-new across parse/typecheck/end-to-end), clippy `-D warnings` clean.

### A7.3 — v0.7.4.2 test scorecard

| Layer | Test |
|---|---|
| Stdlib stubs | `vector_stdlib_path_dispatches_correctly` (new/push/length/get round-trip via `from std.collections.vector import …`) |
| Stdlib stubs | `hashmap_stdlib_path_dispatches_correctly` (new/insert/get/contains, BTreeMap deterministic key order via Q4-A) |
| Stdlib stubs | `path_stdlib_dispatches_correctly` (join/basename/parent — POSIX semantic per Q2-A) |
| Stdlib stubs | `string_and_parse_integer_dispatch_correctly` (substring char-index UTF-8 + index_of + parse_integer; mixed `std.string` + `std.text` imports) |
| Stdlib stubs | `io_fs_dispatch_with_tempfile_round_trip` (write → exists → read via `tempfile` fixture) |

Tests live in `crates/triet-bootstrap/tests/stdlib_stubs_vm.rs`. **VM path only** — interpreter parity deferred §A7 (`triet-interpreter::builtins::install` covers v0.2 prelude; 19 v0.7.3 builtins are VM-dispatched via `path_to_builtin`, no interpreter equivalent). Subprocess capture deferred — assertions inside `.tri` source surface dispatch bugs as test panics.

**A7.2 update — `diff_generic_function` interpreter parity** worked in v0.7.4.1 because `examples/generic_function.tri` uses only the existing v0.2 prelude (`println`, no stdlib stub imports). v0.7.4.2 stdlib stubs cross the threshold — interpreter doesn't dispatch them. Future `v0.7.x.review` may bridge or drop interpreter; see §A7 deferred item.

1124 → 1129 tests (5 net-new stdlib stub VM tests), clippy `-D warnings` clean.

### A7.4 — v0.7.4 umbrella sub-task breakdown

Original ADR-0019 §8 plan v0.7.4 was a single sub-task ("`compiler/lexer.tri` + lexer_differential test"). v0.7.4.1 design review surfaced that 3 blockers must land first — split into 4-sub-commit umbrella mirroring v0.7.3 cadence:

| Sub-task | Scope | Status |
|---|---|---|
| **v0.7.4.1** | Generic function syntax — parser + AST + typecheck (Rust-style inference) + lowerer (type-erased per §A7.1). Unblocks stdlib stubs. | shipped |
| **v0.7.4.2** | Stdlib `.tri` stubs (5 new files: `std/collections.tri` + `std/collections/{vector,hashmap}.tri` + `std/io/fs.tri` + `std/path.tri` + `std/string.tri`) + `std/text.tri::parse_integer` extension + 19 `path_to_builtin` entries + pseudo-struct shells for `Vector<T>`/`HashMap<K, V>` in typecheck. Java-naming per author convention (no module-name repetition in function names). | shipped |
| **v0.7.4.3** | `compiler/lexer.tri` — hand-rolled scanner port per Q4-A (~1090 LOC Triết — `73590fc`). Followed by `-debt.{1..7}` umbrella that drained all 7 workarounds first (lands `123ffa7..730fddc`). | shipped |
| **v0.7.4.4** | `lexer_differential` integration test (NDJSON byte-diff per Q5-A, 20 corpus entries) + verify gate. Closes v0.7.4 umbrella. Surfaced + fixed two pre-existing bugs (`lower_while_loop` declaring-scope phi via `rebind_var`; `NullCheck` mis-classifying unit-variant enums) plus one Triết-port gap (`if?` / `while?` compound keyword handling). | shipped |
