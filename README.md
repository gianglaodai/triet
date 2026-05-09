# Triết

> Một ngôn ngữ lập trình **balanced ternary, AI-first**, lấy cảm hứng từ Setun (Liên Xô, 1958).

Triết (Hán-Việt 哲, "triết học") là một ngôn ngữ lập trình production-grade dùng hệ tam phân cân bằng `{-1, 0, +1}` làm nền tảng số học, kết hợp logic 3 giá trị Łukasiewicz Ł3 cho khả năng lập luận với thông tin không chắc chắn — tối ưu cho thời đại AI lập trình.

## Tài liệu

- [`VISION.md`](VISION.md) — Tầm nhìn dài hạn: 5 trụ cột kiến trúc, mục tiêu OS-capable.
- [`ROADMAP.md`](ROADMAP.md) — Lộ trình từ v0.2 tới v3.0+ với gate cho từng phase.
- [`SPEC.md`](SPEC.md) — Đặc tả ngôn ngữ (source of truth cho semantics).
- [`docs/decisions/`](docs/decisions/) — ADRs cho các quyết định kiến trúc.

## Trạng thái

🟢 **v0.2 (interpreter + struct/enum/generics) — chạy được end-to-end.** Đặc tả ngôn ngữ tại [`SPEC.md`](SPEC.md).

```bash
cargo build --release
./target/release/triet run examples/fizzbuzz.tri
./target/release/triet run examples/measles_risk.tri
./target/release/triet run examples/factorial.tri
./target/release/triet run examples/lukasiewicz_vs_kleene.tri
./target/release/triet run examples/counter.tri
./target/release/triet run examples/long_arithmetic.tri
./target/release/triet run examples/enumerate.tri
./target/release/triet run examples/nullable.tri
./target/release/triet run examples/while_polling.tri
./target/release/triet run examples/maybe.tri
./target/release/triet run examples/generic.tri
```

Tổng 10 demo programs thực thi thành công. 522 tests pass workspace-wide.

## Triết lý thiết kế

1. **AI-first** — cú pháp và semantics tối ưu cho LLM sinh code đúng ngay lần đầu
2. **Tam phân là first-class** — `Trit`, balanced ternary arithmetic, Łukasiewicz logic là kiểu/phép nguyên thủy
3. **Production-grade ở Ł3, mở rộng được tới Ł∞** — đường tiến hóa tới logic vô hạn giá trị (fuzzy/probabilistic) không đập bỏ semantics hiện tại

## Ví dụ

```triet
// FizzBuzz
fn fizzbuzz(n: Integer) -> String =
    match (n %% 3, n %% 5) {
        (0, 0) => "FizzBuzz",
        (0, _) => "Fizz",
        (_, 0) => "Buzz",
        _      => std.text.from_integer(n),
    }

// Lập luận với missing data — sức mạnh của Łukasiewicz
fn risk_measles(fever: Trilean, rash: Trilean, vaccinated: Trilean) -> Trilean {
    let symptoms = fever && rash
    symptoms && !vaccinated
    // Nếu vaccinated = unknown → kết quả = unknown
    // → "không đủ thông tin, cần xác minh"
}
```

## Cấu trúc workspace

```
triet/
├── crates/
│   ├── triet-core/      # Trit/Tryte/Integer/Long + arithmetic
│   ├── triet-logic/     # Trilean + Łukasiewicz Ł3 + Kleene K3
│   ├── triet-syntax/    # AST types
│   ├── triet-lexer/     # Tokenizer
│   ├── triet-parser/    # Parser → AST
│   ├── triet-typecheck/ # Type checker
│   ├── triet-interpreter/ # Tree-walking interpreter
│   └── triet-cli/       # Binary `triet`
├── examples/            # Sample .tri programs
└── SPEC.md              # Đặc tả ngôn ngữ
```

## Build

```bash
cargo build              # debug build
cargo build --release    # release build
cargo test               # run all tests (526 in v0.2)
cargo clippy             # lint
cargo fmt                # format
```

## Chạy demo

```bash
# Build binary
cargo build --release

# Chạy chương trình .tri
./target/release/triet run examples/fizzbuzz.tri

# Type-check không thực thi
./target/release/triet check examples/fizzbuzz.tri

# Thông tin phiên bản
./target/release/triet info
```

## Roadmap (tóm tắt)

Triết hướng tới **ngôn ngữ-OS-capable**: balanced ternary + AI-first + capability-secure, đủ năng lực viết microkernel khi phần cứng tam phân xuất hiện. Pace: stability over speed (5–10 năm).

- **v0.2** — struct, enum, generics ✅ (đang ở đây)
- **v0.2.x** — module system ([ADR-0005](docs/decisions/0005-module-system.md))
- **v0.3** — bytecode VM + stable IR
- **v0.4** — Crate-Pack + stable ABI
- **v0.5** — CAS packaging (hash-based identity)
- **v0.6** — capability namespaces (`sys::` / `dev::` / `usr::`)
- **v0.7** — self-hosting compiler
- **v0.8** — concurrency model
- **v0.9** — JIT (Cranelift)
- **v1.0** — production stability
- **v2.0** — AOT native compile (LLVM)
- **v3.0** — microkernel POC
- **v∞** — backend cho phần cứng tam phân

Chi tiết với gates và ADRs: [`ROADMAP.md`](ROADMAP.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) hoặc [Apache-2.0](LICENSE-APACHE).
