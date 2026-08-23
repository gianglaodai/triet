# ADR 0004 — String multi-line: strip common indentation

**Status:** Decided, language spec constraint. To be implemented when the parser ships multi-line literals (lexer rule `"""..."""` exists in v0.1, but the stripping rule has not yet been applied).

**Issue:** SPEC §13 #4 — Should `"""..."""` strip common indentation like Java text blocks (Java 15+)? Or should it preserve the text as in Python's triple-quoted strings?

## Decision

**Yes**, strip common leading whitespace, following Java 15+ text blocks + Kotlin's `trimIndent()`. Rules:

1. If the literal **does not contain a newline** (e.g., `"""hello"""`) $\rightarrow$ preserve the text as is.
2. If it contains a newline:
   - **Remove the single leading newline immediately following the opening `"""`** (allowing content to be placed on a new line for readability).
   - **Find the common leading whitespace** — the number of identical spaces or tabs appearing at the start of every *non-empty* line and the **line containing the closing `"""`**.
   - **Strip** that number of characters from each line.
   - **Remove the trailing newline** immediately preceding the closing `"""` (if present).
   - Tabs and spaces are counted as **single characters** (no expansion). Mixing tabs and spaces in the leading whitespace results in a compilation error.

## Examples

### Case 1: code nested in a block, indent 12 $\rightarrow$ stripped to 0

```triet
fn html_doc() -> String {
    """
    <html>
        <body>hello</body>
    </html>
    """
}
```

Runtime result:
```
<html>
    <body>hello</body>
</html>
```

(Common indent = 4 spaces; closing `"""` is at column 4. `<body` retains 4 spaces relative to the indent.)

### Case 2: single-line remains unchanged

```triet
let s: String = """hello"""
// = "hello"
```

### Case 3: closing `"""` determines strip depth

```triet
let s = """
        line A
        line B
    """
// strip = 4 (based on closing), result:
//     line A
//     line B
```

### Case 4: mixed tabs/spaces in leading whitespace $\rightarrow$ compilation error

```triet
let bad = """
    line one     // 4 spaces
	line two     // 1 tab
    """
// LexError: "inconsistent leading whitespace in multi-line string"
```

## Rationale

- **AI-first / regular > exception.** Java/Kotlin conventions are proven — LLM training data is saturated with this pattern. Default stripping ensures that generated code looks natural (indented according to block scope) while the runtime value remains clean.
- **Source readability.** Triet prioritizes readable syntax; non-stripping multi-line strings force developers to align content to column 0, breaking the indentation flow.
- **Closing-quote-driven.** The closing `"""` determines the strip depth — consistent with the Java spec; an LLM generates content and simply places the closing quote at the desired position.
- **Explicit errors for mixed tabs/spaces.** Triet does not attempt to guess — avoiding silent bugs when indentation is mixed. Developers fix it once, and tooling can auto-format it.

## Rejected Alternatives

- **Python: no stripping.** Rejected because indented multi-line strings result in poor formatting (`"""string\nbreaks layout"""`).
- **Raw escape `r"""..."""` to skip stripping.** Elegant but increases surface area; the `r` prefix can be added later for other purposes (e.g., regex literals). Deferred.
- **Tab expansion.** Java 15+ initially expanded tabs to 4 spaces during normalization but later removed this. Triet follows the latter Java approach: no expansion, strict single-character equality.

## Consequences

- The lexer must distinguish `"""..."""` from two consecutive `""`. This is already implemented in v0.1 (multi-line bracket).
- Stripping occurs **at lex time**: the literal token already contains the stripped text. The span refers to the original source, introducing minor mapping complexity, but it is manageable.
- Two types of diagnostics:
  - "inconsistent leading whitespace" — line with mixed indentation.
  - "leading whitespace shorter than common — line inappropriately less-indented"
- When developers do not want stripping (e.g., ASCII art): align content to column 0, or use `r"""..."""` raw (deferred).

## Implementation v0.1

The `lex_string_multiline` lexer does not yet apply the stripping rule — the token contains raw text. It must be updated to apply the rules above before emission. Test cases:
- single-line remains unchanged
- closing-quote-driven stripping with 4 cases (column 0, deeper, shallower, mixed tab/space)
- inconsistent-whitespace error
