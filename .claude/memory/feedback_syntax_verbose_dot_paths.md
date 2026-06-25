---
name: User syntax preferences — verbose names + dot path separator
description: User prefers full/verbose keywords over abbreviations (Java sensibility), and `.` over `::` for path separator. Bias-driven; applies to ALL syntax design decisions for Triết.
type: feedback
originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---
**Rule (đã user xác nhận 2026-05-09):**

1. **Verbose/full keywords > abbreviations.** When choosing a new keyword or naming a syntactic construct, default to the full word.
   - `module` over `mod` ✓
   - Likely `public` over `pub`, `package` over `pkg`, `function` over `fn`, etc. — but check before mass-renaming established keywords.

2. **Path separator is `.` (dot), NOT `::`.** Triết uses `foo.bar.baz` for namespace paths, like Java/Python/JS — never `foo::bar::baz` like Rust/C++.
   - `crate.foo.bar` not `crate::foo::bar`
   - `use std.io.println` not `use std::io::println`
   - `std.io` not `std::io`

**Why:** User is a Java developer with strong aesthetic preference. "viết đầy đủ chẳng có vấn đề gì với tôi" — verbosity is not a cost for them. They explicitly find `module` "nhìn thuận mắt hơn" (more visually pleasing) than `mod`, and `.` "ngắn gọn và đẹp mắt hơn" (cleaner and more beautiful) than `::`.

**How to apply:**
- When designing NEW syntax (path syntax, new keywords): default to verbose form + dot separator. Don't assume Rust convention.
- When proposing decisions in ADRs, frame the verbose form as the recommendation, not as alternative.
- For ALREADY-shipped keywords (`fn`, `let`, `mut`, `const`, `pub`, `enum`, `struct`): don't unilaterally rename — ASK before sweeping changes since user has been working with them already.
- If a Rust prior-art document I've referenced uses `::` or shorthand keywords, mentally translate when applying to Triết.

**Concrete keyword mapping (confirmed 2026-05-09):**
| Shorthand (current) | Verbose (target) |
|---|---|
| `fn` | `function` |
| `pub` | `public` |
| `pub(pkg)` | `public(package)` |
| `mut` (in `let mut`, params) | `mutable` |
| `const` | `constant` |
| `mod` | `module` |

**Path syntax:** `.` everywhere. No `::`. Rust tokens (`Token::ColonColon`) remove entirely.

**Module syntax (Java JPMS-aligned):**
- `module foo` (file-bound submodule declaration)
- `module foo { ... }` (inline submodule)

**Import syntax (Python-aligned, dot path):**
- `import std.io` — imports the module; access via `std.io.X`
- `from std.io import println` — selective; `println` directly in scope
- `from std.io import println, print` — multi
- `from std.io import println as p` — alias

**Concrete impact on existing decisions (must fix):**
- ADR-0005 currently specifies `mod` keyword + `::` path syntax + `use foo::bar` imports. ALL contradict this preference. Major rewrite needed.
- Commit `ab2725a` renamed `module`→`mod` token. Must revert (combined with broader rename).
- 11 example `.tri` files use `fn`, `pub`, etc. — need full sweep.
- All snapshot tests with these keywords need re-baselining.
- Pre-existing `import std.io.println` (current code base) is dot-path imports — already aligned, but the leaf-symbol-imports semantic should switch to Python `from ... import ...`.

**KEEP (not changed):** `let`, `if`, `else`, `match`, `return`, `for`, `while`, `loop`, `break`, `continue`, `in`, `true`, `false`, `unknown`, `null`, `import`, `owned`, `struct`, `enum`, `type`, type primitives. User has been using these without protest; verbose-enough or domain-conventional.

**Logic operators — symbolic preferred (updated 2026-05-10):** Keyword forms (`not`, `and`, `or`, `xor`, `implies`, `iff`, `kleene_implies`, `kleene_xor`, `kleene_iff`) remain valid/reserved but symbolic forms are the primary convention: `!`, `&&`, `||`, `^`, `=>`, `<=>`, `~>`, `~^`, `<~>`. Both forms map to the same AST nodes. The `~` prefix consistently marks Kleene K3 variants. Demo code, examples, and new code should use symbolic form.
