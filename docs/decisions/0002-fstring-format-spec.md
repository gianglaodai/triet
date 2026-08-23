# ADR 0002 — F-string format spec

**Status:** Decided, language spec constraint. v0.1 only implements `{}` (no format spec). The full spec will be applied incrementally as needed.

**Issue:** SPEC §13 #2 — Should the format syntax in `f"{val:???}"` follow Python (`{val:.2f}`, very rich), Rust (`{val:>5}`, moderate), or be simplified?

## Decision

**A selective subset of the Rust format spec**, adhering to the SPEC §0 philosophy ("regular > exception, low ambiguity > terseness"). Syntax:

```
fstring_part   ::=  "{" expr [":" format_spec] "}"
format_spec    ::=  width? ("." precision)?
width          ::=  ["0"] decimal_digits        # leading 0 = zero-pad number
precision      ::=  decimal_digits              # number of decimal places (deferred — requires Float v0.2+)
```

### Included

- `{n}` — Default Display
- `{n:8}` — width 8, space padding (numbers right-aligned, strings left-aligned)
- `{n:08}` — width 8, zero-padding (numeric types only)
- `{n:.2}` — 2 decimal places **(pending float v0.2+; v0.1 will reject with LexError)**

### Excluded

- Alignment markers `<` `>` `^` (Rust). Reason: width implicitly right-aligns numbers and left-aligns strings — sufficient for 95% of use cases; explicit overrides are not required for v0.1/v0.2.
- Type chars `b` `o` `x` `X` `e` `E` (Rust hex/binary/octal/scientific). The philosophy is **ternary first** (§2.1) — hex/oct/bin are exceptions and must use explicit method calls (e.g., `n.to_hex_string()`). Binary literal `0b...` also does not exist.
- Sign char `+` (Rust). Only add if there is actual demand.
- Optional fill char (`{:*>5}`). Removed because the `*>` syntax is too ad-hoc for LLMs.
- Locale-aware formatting. The default philosophy is canonical decimal; locale is a concern for libraries/runtime, not syntax.

## Rationale

- **Regular: a single grammar.** Width + precision cover 95% of real-world needs. Everything else → explicit method calls, avoiding syntactic noise.
- **AI-first.** Python's full format mini-language (`{val:>+10,.2%}`) is difficult for LLMs to remember precisely — prone to hallucination. The subset above is enumerable within a bulleted list.
- **Ternary first.** Removing hex/bin/oct chars from the format spec clarifies that those bases are exceptions within the Philosophy.
- **Extensible.** The current subset is not closed to a broader spec — alignment characters can be added later without breaking code.

## Implementation v0.1

The lexer mode-stack currently parses `{expr}` correctly (§1.5.4 implementation note). The format spec after the `:` colon is not yet parsed — to be added as needed. When encountering a format spec in v0.1, the lexer/parser will accept it as a pass-through text string internally; the runtime will report a clear error: "format spec X is not supported".

## Consequences

- Fragile strings like `f"price: {price:#.2f} USD"` must wait for float support in v0.2+. For v0.1, developers should write `f"price: {format_money(price)} USD"` — more explicit and clearer.
- The spec is not closed to extensions; only the surface area is limited in v0.1.
