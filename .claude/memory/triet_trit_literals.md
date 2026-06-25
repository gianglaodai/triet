---
name: triet-trit-literals
description: "Trit literals trong Triết — không phải `0t+ / 0t- / 0t0` (đó là Integer balanced ternary), mà là `1_trit / 0_trit / -1_trit` (suffix form)."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 040682f9-3a3a-48c0-88f4-aa3f74fb5af3
---

`0t+`, `0t-`, `0t0` parse thành **Integer** (balanced ternary literal per SPEC §1.5.1), **KHÔNG** phải Trit. Lexer rule là `#[regex(r"0t[+\-0_]+", lex_ternary_integer)]` — sản xuất `IntLiteral` token. Hệ quả: `function f() -> Trit = 0t+` raise `Mismatch { expected: Trit, found: Integer }`.

Trit literals đúng dùng suffix form:
- `1_trit` → Trit::Positive
- `0_trit` → Trit::Zero
- `-1_trit` → Trit::Negative

Pack_writer.tri đã codify pattern này (xem `byte_to_trit` / `trit_to_byte`).

**Why:** CLAUDE.md "Language conventions" table viết "`0t+`, `0t-`, `0t0` (prefix trit literal)" — misleading. Nhầm lẫn surfaced lần đầu khi viết `compiler/main.tri` v0.7.9.4 returning `Trit` exit codes: dùng `0t+ / 0t-` raise 7 lỗi Mismatch trước khi sửa thành `1_trit / -1_trit`. SPEC §1.5.1 (line 165 `1_trit              // Trit`) là source-of-truth, override CLAUDE.md table.

**How to apply:** Khi cần literal `Trit` trong Triết source, dùng `1_trit / 0_trit / -1_trit`. Reserve `0t+ / 0t- / 0t0` cho balanced-ternary Integer literals (rất ít khi cần — Integer literals thông thường dùng decimal). Helper functions `exit_ok() -> Trit = 1_trit` / `exit_err() -> Trit = -1_trit` là pattern tốt khi nhiều call sites cần cùng giá trị.

Liên quan: [[feedback_syntax_verbose_dot_paths]] (Triết verbose-keyword tradition), [[reference_spec]] (SPEC §1.5.1).
