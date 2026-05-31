# Triết

> Một ngôn ngữ lập trình **balanced ternary, AI-first**, lấy cảm hứng từ Setun (Liên Xô, 1958).

Triết (Hán-Việt 哲, "triết học") là một ngôn ngữ lập trình production-grade dùng hệ tam phân cân bằng `{-1, 0, +1}` làm nền tảng số học, kết hợp logic 3 giá trị Łukasiewicz Ł3 cho khả năng lập luận với thông tin không chắc chắn — tối ưu cho thời đại AI lập trình.

## Tài liệu

- [`VISION.md`](VISION.md) — Tầm nhìn dài hạn: 5 trụ cột kiến trúc, mục tiêu OS-capable.
- [`ROADMAP.md`](ROADMAP.md) — Lộ trình từ v0.2 tới v3.0+ với gate cho từng phase.
- [`SPEC.md`](SPEC.md) — Đặc tả ngôn ngữ (source of truth cho semantics).
- [`docs/decisions/`](docs/decisions/) — ADRs cho các quyết định kiến trúc.

## Trạng thái

🟢 **Language SPEC v0.10 — implementation v0.10.0 SHIPPED — JIT builtin-shim layer + NLL borrow enforcement + multi-thread Atomic + interpreter parity.** **v0.10 highlights:** (1) JIT builtin-shim layer per [ADR-0032](docs/decisions/0032-builtin-shim-abi.md) — 36/43 builtins JIT-shimmed (I/O, Text, Vector, HashMap, Path, String, Atomic ×10, File I/O ×5), multi-call codegen with per-call failure sentinels, composite ABI (`Rc::into_raw` box / borrow / `drop_arc`); all shims **delegate semantics to the VM's own `dispatch_builtin`** so VM↔JIT divergence is impossible by construction; (the `catch_unwind`-across-JIT error design hit a cranelift-jit 0.132 unwind-table cliff — resolved to a per-call sentinel mechanism, ADR-0032 §4 Addendum); (2) NLL borrow enforcement per [ADR-0025](docs/decisions/0025-borrow-checker-rules.md) — E2440 exclusivity (CFG live-range), E2400 lifetime elision (3 rules), E2411 frozen-to-mutable promotion, E2403 escaping-borrow; (3) Multi-thread Atomic per [ADR-0026 v2](docs/decisions/0026-actor-boundary-send-rules.md) — real `raw_thread.spawn` (OS threads, `.triv` v7), `Atomic<T>` migrated to `Arc<Mutex>` (Send + Sync), cross-thread share validated; (4) interpreter `sys.atomic.*` parity. **v0.11 backlog**: AOT cache (relocating-ELF-loader cliff, [ADR-0033](docs/decisions/0033-aot-cache-cranelift-object.md)) + bootstrap byte-identical gate lift + ≥10× perf bench + varargs shims (`FStringConcat`/`TextConcat`) + `std.concurrency.*`. Pipeline `parse → modules → typecheck → interpret/VM/JIT` end-to-end; bytecode VM với register SSA IR + `.triv` v7 + `.khi` user-facing pack format. **Ternary-native IR** với `BrTrilean` 3-way branch per [ADR-0010](docs/decisions/0010-ternary-native-ir.md). **Self-hosting Compiler** per [ADR-0019](docs/decisions/0019-self-hosting-compiler-bootstrap.md), Outcome error handling per [ADR-0020](docs/decisions/0020-outcome-error-handling.md), Trilean! refinement per [ADR-0021](docs/decisions/0021-trilean-refinement.md), S6 Ownership per [ADR-0022](docs/decisions/0022-trit-balanced-ownership.md), Self-host port policy per [ADR-0029](docs/decisions/0029-self-host-port-policy.md). **1637 tests** pass workspace-wide.

> **Lưu ý — gate hội tụ self-hosting defer sang v0.11.** Bootstrap chain 3-stage đã wired end-to-end và `cmp` gate đã in-tree, nhưng test chứng minh `compiler/main.tri` hội tụ (Stage 2 ≡ Stage 3 byte-identical) đang `#[ignore]` — một lần Stage 2 self-compile `main.tri` mất >15 phút trên VM dev tier. CI hiện chỉ enforce proxy gate `factorial.tri` Stage 1 ≡ Stage 2 byte-identical, đủ để verify canonical-encoding invariants ([ADR-0019 §3](docs/decisions/0019-self-hosting-compiler-bootstrap.md)) nhưng **chưa** verify full self-host convergence claim. Gate lift chained to the JIT AOT cache (warm-cache bootstrap < 10 min); the cache hit a relocating-ELF-loader cliff and defers to v0.11 ([ADR-0033 Addendum](docs/decisions/0033-aot-cache-cranelift-object.md)), so the convergence gate carries one more phase.

```bash
cargo build --release

# Tree-walking interpreter (production tier hiện tại)
./target/release/dao run examples/fizzbuzz.tri
./target/release/dao run examples/measles_risk.tri
./target/release/dao run examples/factorial.tri
./target/release/dao run examples/lukasiewicz_vs_kleene.tri
./target/release/dao run examples/counter.tri
./target/release/dao run examples/long_arithmetic.tri
./target/release/dao run examples/enumerate.tri
./target/release/dao run examples/nullable.tri
./target/release/dao run examples/while_polling.tri
./target/release/dao run examples/maybe.tri
./target/release/dao run examples/generic.tri

# Module system demo (704 dòng, 6 module file-bound + nested + inline)
./target/release/dao run demos/02-module-system/main.tri

# Capability system walkthrough (v0.6 — shipped, illustrative manifest + policy)
cat demos/04-capability-system/README.md
cat demos/04-capability-system/dao.package

# Compile → bytecode → VM execution
./target/release/dao build examples/factorial.tri -o /tmp/factorial.triv
./target/release/dao run /tmp/factorial.triv
```

## Triết lý thiết kế

1. **AI-first** — cú pháp và semantics tối ưu cho LLM sinh code đúng ngay lần đầu
2. **Tam phân là first-class** — `Trit`, balanced ternary arithmetic, Łukasiewicz logic là kiểu/phép nguyên thủy
3. **Stability over speed** — mọi quyết định kiến trúc có ADR; gate đóng phase rõ ràng (xem [ADR-0009](docs/decisions/0009-version-gate-policy.md))
4. **IR ≠ runtime** — Triết IR là spec, backend (VM/JIT/AOT/trytecode) là implementation (xem [VISION § 4](VISION.md))

## Ví dụ

```triet
// FizzBuzz
function fizzbuzz(n: Integer) -> String =
    match (n %% 3, n %% 5) {
        (0, 0) => "FizzBuzz",
        (0, _) => "Fizz",
        (_, 0) => "Buzz",
        _      => std.text.from_integer(n),
    }

// Lập luận với missing data — sức mạnh của Łukasiewicz Ł3
function risk_measles(fever: Trilean, rash: Trilean, vaccinated: Trilean) -> Trilean {
    let symptoms = fever && rash
    symptoms && !vaccinated
    // Nếu vaccinated = unknown → kết quả = unknown
    // → "không đủ thông tin, cần xác minh"
}

// Module system — Python-style imports, verbose keywords
from std.io import println
from crate.gates import nand_gate, xor_gate

public function half_adder(a: Trit, b: Trit) -> (Trit, Trit) =
    (xor_gate(a, b), nand_gate(a, b))
```

## Cấu trúc workspace

```
triet/
├── crates/
│   ├── triet-core/        # Trit/Tryte/Integer/Long + arithmetic
│   ├── triet-logic/       # Trilean + Łukasiewicz Ł3 + Kleene K3
│   ├── triet-syntax/      # AST types + arena allocator
│   ├── triet-lexer/       # Tokenizer (logos-based)
│   ├── triet-parser/      # Parser → AST
│   ├── triet-modules/     # Module loader + name resolver
│   ├── triet-ir/          # Register SSA IR + lowerer + bytecode VM
│   ├── triet-typecheck/   # Type checker với inference + monomorphization
│   ├── triet-interpreter/ # Tree-walking interpreter (development tier)
│   └── triet-cli/         # Binary `triet` (run/check/build/info)
├── std/                   # Standard library (.tri files)
│   ├── io.tri, text.tri, assert.tri
├── compiler/              # Triết-in-Triết self-hosting compiler (v0.7 shipped, ~23K LOC)
├── examples/              # 14 single-file .tri programs + 1 dir (atomic_counter capability + ownership demo)
├── demos/                 # Larger multi-file demos
│   ├── 02-module-system/  # 704-dòng ternary ALU across 6 modules
│   ├── 04-capability-system/  # v0.6 capability gates walkthrough
│   └── 05-error-handling/ # v0.7.4.3-error Outcome capstone (VM-only)
├── docs/decisions/        # 33 ADRs (+ Addendums on ADR-0001, 0010, 0015, 0017, 0018, 0019, 0032, 0033)
├── SPEC.md                # Đặc tả ngôn ngữ (header v0.10)
├── VISION.md              # Tầm nhìn 5 trụ cột + OS-capable
└── ROADMAP.md             # Phase gates v0.2 → v3.0+ (v0.10 JIT shim + NLL + multi-thread Atomic shipped)
```

## Build

```bash
cargo build              # debug build
cargo build --release    # release build
cargo test --workspace   # run all tests (1637 in v0.10.0)
cargo clippy --workspace --all-targets   # lint
cargo fmt --all          # format
```

### Contributing — install git hooks

Sau khi clone, chạy một lần để bật ADR-0009 enforcement (pre-commit fmt + pre-push full gate B):

```bash
bash scripts/install-hooks.sh
```

Sets `core.hooksPath = .githooks` cho clone hiện tại. Hooks ngăn commit/push với dirty fmt/clippy/test state. Xem [ADR-0009 Addendum](docs/decisions/0009-version-gate-policy.md) cho rationale.

Trước khi tag release: `bash scripts/release-check.sh` verify 4-gate matrix.

## Chạy demo

```bash
# Build binary
cargo build --release

# Chạy chương trình .tri (tree-walker)
./target/release/dao run examples/fizzbuzz.tri

# Type-check không thực thi
./target/release/dao check examples/fizzbuzz.tri

# Compile → bytecode → VM
./target/release/dao build examples/fizzbuzz.tri -o /tmp/fizzbuzz.triv
./target/release/dao run /tmp/fizzbuzz.triv

# Thông tin phiên bản
./target/release/dao info
```

## Roadmap (tóm tắt)

Triết hướng tới **ngôn ngữ-OS-capable**: balanced ternary + AI-first + capability-secure, đủ năng lực viết microkernel khi phần cứng tam phân xuất hiện. Pace: stability over speed (5–10 năm).

- **v0.2** — struct, enum, generics ✅
- **v0.2.x** — module system ✅ ([ADR-0005](docs/decisions/0005-module-system.md))
- **v0.3** — bytecode VM + stable IR ✅ ([ADR-0007](docs/decisions/0007-ir-design.md), [ADR-0008](docs/decisions/0008-triv-binary-format.md))
- **v0.3.x.cleanup** — gate-closing phase ✅ ([ADR-0009](docs/decisions/0009-version-gate-policy.md))
- **v0.3.x.ternary** — ternary-native IR ✅ ([ADR-0010](docs/decisions/0010-ternary-native-ir.md))
- **v0.4** — Crate-Pack + stable ABI ✅ ([ADR-0011](docs/decisions/0011-abi-metadata-format.md), [ADR-0012](docs/decisions/0012-witness-table-dispatch.md), [ADR-0013](docs/decisions/0013-semver-linking-policy.md))
- **v0.5** — CAS packaging ✅ ([ADR-0014](docs/decisions/0014-hash-scheme-refinement.md), [ADR-0015](docs/decisions/0015-package-store-layout.md))
- **v0.6** — capability namespaces (`sys.*` / `dev.*` / `usr.*`) ✅ ([ADR-0016](docs/decisions/0016-capability-type-system.md), [ADR-0017](docs/decisions/0017-trilean-policy-hook.md), [ADR-0018](docs/decisions/0018-capability-loader-semantics.md))
- **v0.7** — self-hosting compiler ✅ ([ADR-0019](docs/decisions/0019-self-hosting-compiler-bootstrap.md), [ADR-0020](docs/decisions/0020-outcome-error-handling.md), [ADR-0021](docs/decisions/0021-trilean-refinement.md), [ADR-0024](docs/decisions/0024-khi-dao-identity-naming.md)). Stage 1 → Stage 2 byte-identical for factorial.tri (CI); Stage 2 ≡ Stage 3 gate for main.tri wired manual-promoted (VM dev tier > 15min per compile, defers to v0.9 JIT).
- **v0.8** — Concurrency Primitives & BYOS (Bring Your Own Scheduler) ✅ SHIPPED
- **v0.9** — Atomic Primitive + Borrow Expression + Cranelift JIT (partial) ✅ SHIPPED ([ADR-0028](docs/decisions/0028-atomic-primitive.md), [ADR-0029](docs/decisions/0029-self-host-port-policy.md), [ADR-0030](docs/decisions/0030-jit-cranelift-integration.md), [ADR-0031](docs/decisions/0031-borrow-expression-syntax.md))
- **v0.10** — Full builtin shim layer + AOT cache + NLL enforcement + multi-thread Atomic (in progress)
- **v1.0** — production stability
- **v2.0** — AOT native compile (LLVM)
- **v3.0** — microkernel POC
- **v∞** — backend cho phần cứng tam phân

Chi tiết với gates và ADRs: [`ROADMAP.md`](ROADMAP.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) hoặc [Apache-2.0](LICENSE-APACHE).
