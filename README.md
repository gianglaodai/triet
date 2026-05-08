# Triết

> Một ngôn ngữ lập trình **balanced ternary, AI-first**, lấy cảm hứng từ Setun (Liên Xô, 1958).

Triết (Hán-Việt 哲, "triết học") là một ngôn ngữ lập trình production-grade dùng hệ tam phân cân bằng `{-1, 0, +1}` làm nền tảng số học, kết hợp logic 3 giá trị Łukasiewicz Ł3 cho khả năng lập luận với thông tin không chắc chắn — tối ưu cho thời đại AI lập trình.

## Trạng thái

🚧 **Tiền alpha (v0.1)** — đang scaffold, chưa chạy được. Đặc tả ngôn ngữ tại [`SPEC.md`](SPEC.md).

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
├── examples/            # Sample .tt programs
└── SPEC.md              # Đặc tả ngôn ngữ
```

## Build

```bash
cargo build              # debug build
cargo build --release    # release build
cargo test               # run all tests
cargo clippy             # lint
cargo fmt                # format
```

## Roadmap

- **v0.1** — interpreter tree-walking, semantics đầy đủ (đang làm)
- **v0.2** — struct, enum, generics, `Option<T>`, `BinaryInteger`/`BinaryLong` interop, Ł∞ (fuzzy continuous)
- **v0.3** — bytecode VM với JIT (Cranelift)
- **v0.4** — concurrency model
- **v1.0** — production stability, AOT native compile (LLVM/Cranelift)
- **v2.0+** — backend cho phần cứng tam phân giả định

## License

Dual-licensed under [MIT](LICENSE-MIT) hoặc [Apache-2.0](LICENSE-APACHE).
