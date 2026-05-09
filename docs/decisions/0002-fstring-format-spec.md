# ADR 0002 — F-string format spec

**Trạng thái:** Quyết định, ràng buộc spec ngôn ngữ. v0.1 chỉ implement `{}` (không có format spec). Spec đầy đủ áp dụng dần khi dùng đến.

**Issue:** SPEC §13 #2 — Cú pháp format trong `f"{val:???}"` lấy theo Python (`{val:.2f}`, rất rich) hay Rust (`{val:>5}`, vừa) hay đơn giản hóa?

## Quyết định

**Subset có chọn lọc của Rust format spec**, theo đúng triết lý SPEC §0 ("regular > exception, low ambiguity > terseness"). Cú pháp:

```
fstring_part   ::=  "{" expr [":" format_spec] "}"
format_spec    ::=  width? ("." precision)?
width          ::=  ["0"] decimal_digits        # leading 0 = zero-pad số
precision      ::=  decimal_digits              # số chữ số thập phân (deferred — cần Float v0.2+)
```

### Có

- `{n}` — Display mặc định
- `{n:8}` — width 8, pad space (số căn phải, chuỗi căn trái)
- `{n:08}` — width 8, zero-pad (chỉ dành cho numeric type)
- `{n:.2}` — 2 chữ số thập phân **(chờ float v0.2+; v0.1 reject với LexError)**

### Không có

- Alignment markers `<` `>` `^` (Rust). Lý do: width đã ngầm right-align cho số, left-align cho chuỗi — đủ cho 95% use case; explicit override không cần ở v0.1/v0.2.
- Type chars `b` `o` `x` `X` `e` `E` (Rust hex/binary/octal/scientific). Triết là **tam phân first** (§2.1) — hex/oct/bin là ngoại lệ, phải dùng method gọi rõ (e.g., `n.to_hex_string()`). Binary literal `0b...` cũng không tồn tại.
- Sign char `+` (Rust). Chỉ thêm khi có demand thực tế.
- Fill char tùy chọn (`{:*>5}`). Loại bỏ vì cú pháp `*>` quá ad-hoc cho LLM.
- Locale-aware formatting. Triết default là canonical decimal; locale là concern thư viện/runtime, không syntax.

## Lý do

- **Regular: một grammar duy nhất.** Width + precision cover 95% nhu cầu thực. Mọi thứ khác → method call rõ tên, không syntactic noise.
- **AI-first.** Python's full format mini-language (`{val:>+10,.2%}`) khó cho LLM nhớ chính xác — dễ hallucinate. Subset trên là enumerable trong một bullet list.
- **Tam phân first.** Loại bỏ hex/bin/oct chars khỏi format spec làm rõ rằng những hệ cơ số đó là ngoại lệ trong Triết.
- **Mở rộng được.** Subset hiện tại không đóng cửa với spec rộng hơn — có thể thêm alignment chars sau mà không break code.

## Implementation v0.1

Lexer mode-stack hiện đã parse `{expr}` đúng (§1.5.4 implementation note). Format spec sau dấu `:` chưa parse — thêm khi cần. Khi gặp format spec ở v0.1, lexer/parser chấp nhận pass-through như chuỗi text trong nội bộ; runtime báo lỗi rõ "format spec X chưa hỗ trợ".

## Hậu quả

- Các string mong manh kiểu `f"giá: {price:#.2f} USD"` cần đợi float landing ở v0.2+. Cho v0.1, dev viết `f"giá: {format_money(price)} USD"` — explicit hơn, rõ hơn.
- Spec không đóng cửa với extension; chỉ giới hạn surface ở v0.1.
