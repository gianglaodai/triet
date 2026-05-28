# Triết

> Một ngôn ngữ lập trình **balanced ternary, AI-first**, lấy cảm hứng từ Setun (Liên Xô, 1958).

Triết (Hán-Việt 哲, "triết học") là một ngôn ngữ lập trình production-grade dùng hệ tam phân cân bằng `{-1, 0, +1}` làm nền tảng số học, kết hợp logic 3 giá trị Łukasiewicz Ł3 cho khả năng lập luận với thông tin không chắc chắn — tối ưu cho thời đại AI lập trình.

## Tài liệu

- [`VISION.md`](VISION.md) — Tầm nhìn dài hạn: 5 trụ cột kiến trúc, mục tiêu OS-capable.
- [`ROADMAP.md`](ROADMAP.md) — Lộ trình từ v0.2 tới v3.0+ với gate cho từng phase.
- [`SPEC.md`](SPEC.md) — Đặc tả ngôn ngữ (source of truth cho semantics).
- [`docs/decisions/`](docs/decisions/) — ADRs cho các quyết định kiến trúc.

## Trạng thái

🟢 **Language SPEC v0.8 — implementation v0.8.0 SHIPPED — Ownership Foundation + Concurrency Primitives (BYOS).** **v0.8 highlight:** S6 ownership model (5-form reference `&+`/`&0`/`&-`/`&` + `owned`) per [ADR-0022](docs/decisions/0022-trit-balanced-ownership.md), Send-derivation diagnostics (E2500) per [ADR-0026 v2](docs/decisions/0026-actor-boundary-send-rules.md), borrow-checker skeleton (E24XX, enforcement v0.9) per [ADR-0025](docs/decisions/0025-borrow-checker-rules.md), AI-first diagnostic format per [ADR-0027](docs/decisions/0027-diagnostic-format-standard.md). **BYOS — Bring Your Own Scheduler**: Triết core provides primitives + capability gates, scheduler lives in stdlib (v0.10) or external (kernel-mode). Pipeline `parse → modules → typecheck → interpret` end-to-end; bytecode VM với register SSA IR + `.triv` IR binary format (v5 per [ADR-0008 §Version history](docs/decisions/0008-triv-binary-format.md)) + `.khi` user-facing pack format. **Ternary-native IR** với `BrTrilean` 3-way branch + Ł3-aware `Eq` per [ADR-0010](docs/decisions/0010-ternary-native-ir.md). **Crate-pack distribution** + cross-package linker per [ADR-0011](docs/decisions/0011-abi-metadata-format.md)/[0012](docs/decisions/0012-witness-table-dispatch.md)/[0013](docs/decisions/0013-semver-linking-policy.md). **CAS Packaging** per [ADR-0014](docs/decisions/0014-hash-scheme-refinement.md)/[0015](docs/decisions/0015-package-store-layout.md). **Capability System** per [ADR-0016](docs/decisions/0016-capability-type-system.md)/[0017](docs/decisions/0017-trilean-policy-hook.md)/[0018](docs/decisions/0018-capability-loader-semantics.md). **Self-hosting Compiler** per [ADR-0019](docs/decisions/0019-self-hosting-compiler-bootstrap.md) (3-stage bootstrap chain, factorial.tri Stage 2 byte-identical in CI; main.tri convergence gate `#[ignore]`'d, lifts v0.9), Outcome error handling per [ADR-0020](docs/decisions/0020-outcome-error-handling.md), Trilean! refinement per [ADR-0021](docs/decisions/0021-trilean-refinement.md), Identity rename per [ADR-0024](docs/decisions/0024-khi-dao-identity-naming.md). **1425 tests** pass workspace-wide.

> **Lưu ý — gate hội tụ self-hosting defer sang v0.9.** Bootstrap chain 3-stage đã wired end-to-end và `cmp` gate đã in-tree, nhưng test chứng minh `compiler/main.tri` hội tụ (Stage 2 ≡ Stage 3 byte-identical) đang `#[ignore]` — một lần Stage 2 self-compile `main.tri` mất >15 phút trên VM dev tier. CI hiện chỉ enforce proxy gate `factorial.tri` Stage 1 ≡ Stage 2 byte-identical, đủ để verify canonical-encoding invariants ([ADR-0019 §3](docs/decisions/0019-self-hosting-compiler-bootstrap.md)) nhưng **chưa** verify full self-host convergence claim. Cả 2 ignored tests lift lên CI-required ở v0.9 (Cranelift JIT). Rationale: [ADR-0019 Addendum 2026-05-25](docs/decisions/0019-self-hosting-compiler-bootstrap.md).

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
├── examples/              # 14 single-file .tri programs + 1 dir (atomic_counter aspirational)
├── demos/                 # Larger multi-file demos
│   ├── 02-module-system/  # 704-dòng ternary ALU across 6 modules
│   ├── 04-capability-system/  # v0.6 capability gates walkthrough
│   └── 05-error-handling/ # v0.7.4.3-error Outcome capstone (VM-only)
├── docs/decisions/        # 27 ADRs (+ Addendums on ADR-0001, 0010, 0015, 0017, 0018, 0019)
├── SPEC.md                # Đặc tả ngôn ngữ (header v0.8)
├── VISION.md              # Tầm nhìn 5 trụ cột + OS-capable
└── ROADMAP.md             # Phase gates v0.2 → v3.0+ (v0.8 BYOS shipped)
```

## Build

```bash
cargo build              # debug build
cargo build --release    # release build
cargo test --workspace   # run all tests (1425 in v0.8.0)
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
- **v0.9** — JIT (Cranelift)
- **v1.0** — production stability
- **v2.0** — AOT native compile (LLVM)
- **v3.0** — microkernel POC
- **v∞** — backend cho phần cứng tam phân

Chi tiết với gates và ADRs: [`ROADMAP.md`](ROADMAP.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) hoặc [Apache-2.0](LICENSE-APACHE).
