# ADR 0004 — String multi-line: strip common indentation

**Trạng thái:** Quyết định, ràng buộc spec ngôn ngữ. Implement khi parser ship multi-line literal (đã có lexer rule `"""..."""` ở v0.1 nhưng strip rule chưa apply).

**Issue:** SPEC §13 #4 — `"""..."""` có nên strip indentation chung như Java text blocks (Java 15+) không? Hay giữ nguyên text như Python's triple-quoted strings?

## Quyết định

**Có**, strip common leading whitespace, theo Java 15+ text block + Kotlin `trimIndent()`. Quy tắc:

1. Nếu literal **không có newline** (e.g. `"""xin chào"""`) → giữ nguyên text.
2. Nếu có newline:
   - **Bỏ leading newline duy nhất ngay sau `"""` mở** (cho phép đặt content xuống dòng cho dễ đọc).
   - **Tìm common leading whitespace** — số space hoặc tab giống nhau xuất hiện ở đầu mọi dòng *non-empty* và **dòng chứa `"""` đóng**.
   - **Strip** số ký tự đó từ mỗi dòng.
   - **Bỏ trailing newline** ngay trước `"""` đóng (nếu có).
   - Tab và space được đếm như **ký tự đơn** (không expand). Mix tab+space ở leading whitespace là lỗi compile.

## Ví dụ

### Case 1: code lồng trong block, indent 12 → strip về 0

```triet
fn html_doc() -> String {
    """
    <html>
        <body>hello</body>
    </html>
    """
}
```

Kết quả runtime:
```
<html>
    <body>hello</body>
</html>
```

(Common indent = 4 spaces; closing `"""` ở cột 4. `<body>` giữ 4 space relative.)

### Case 2: single-line vẫn ngang vế

```triet
let s: String = """xin chào"""
// = "xin chào"
```

### Case 3: closing `"""` quyết định strip depth

```triet
let s = """
        line A
        line B
    """
// strip = 4 (theo closing), kết quả:
//     line A
//     line B
```

### Case 4: tab/space mix ở leading → lỗi compile

```triet
let bad = """
    line one     // 4 spaces
	line two     // 1 tab
    """
// LexError: "inconsistent leading whitespace in multi-line string"
```

## Lý do

- **AI-first / regular > exception.** Java/Kotlin convention đã prove được — LLM training data ngập tràn pattern này. Stripping mặc định = code generated trông tự nhiên (indented theo block scope) mà runtime lại sạch.
- **Source nhìn dễ.** Code Triết ưu tiên cú pháp dễ đọc; multi-line string không-strip buộc dev phải đẩy content về cột 0, phá indent flow.
- **Closing-quote-driven.** Closing `"""` quyết định strip — nhất quán với Java spec; LLM gen content rồi đặt closing ở vị trí mong muốn là done.
- **Tab/space mix lỗi rõ.** Triết không cố guess — tránh bug âm thầm khi mix indentation. Dev fix một lần, tooling auto-format giúp.

## Cân nhắc đã loại

- **Python: không strip.** Bị bỏ vì viết multi-line string indented xấu (`"""string\nbreaks layout"""`).
- **Raw escape `r"""..."""` để skip strip.** Đẹp nhưng tăng surface; `r` prefix sau cũng có thể thêm sau cho mục đích khác (regex literal). Để dành.
- **Tab expansion.** Java 15+ ban đầu expand tab thành 4 space khi normalize, sau bỏ. Triết theo Java sau: không expand, strict equality ký tự đơn.

## Hậu quả

- Lexer phải biết phân biệt `"""..."""` với hai `""` liền nhau. Đã có ở v0.1 (multi-line bracket).
- Strip happens **ở lex time**: token literal đã chứa text đã strip. Span theo source gốc, mapping minor complexity nhưng manageable.
- Diagnostic hai loại:
  - "inconsistent leading whitespace" — line bị mix
  - "leading whitespace shorter than common — line inappropriately less-indented"
- Khi dev không muốn strip (e.g. ASCII art): đặt content sát cột 0, hoặc sau này dùng `r"""..."""` raw (deferred).

## Implementation v0.1

Lexer `lex_string_multiline` hiện chưa apply strip rule — token chứa raw text. Cần update để áp dụng quy tắc trên trước khi emit. Test cases:
- single-line vẫn unchanged
- closing-quote-driven strip với 4 case (column 0, deeper, shallower, mixed tab/space)
- inconsistent-whitespace error
