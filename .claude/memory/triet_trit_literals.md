---
name: triet-trit-literals
description: "Trit literals in Triết — NOT `0t+ / 0t- / 0t0` (those are balanced-ternary Integers) but `1_trit / 0_trit / -1_trit` (the suffix form)."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 040682f9-3a3a-48c0-88f4-aa3f74fb5af3
---

`0t+`, `0t-`, `0t0` parse as an **Integer** (a balanced-ternary literal per SPEC §1.5.1), **NOT** a Trit. The lexer rule is `#[regex(r"0t[+\-0_]+", lex_ternary_integer)]` — it produces an `IntLiteral` token. Consequence: `function f() -> Trit = 0t+` raises `Mismatch { expected: Trit, found: Integer }`.

The correct Trit literals use the suffix form:
- `1_trit` → Trit::Positive
- `0_trit` → Trit::Zero
- `-1_trit` → Trit::Negative

pack_writer.tri already codifies this pattern (see `byte_to_trit` / `trit_to_byte`).

**Why:** the CLAUDE.md "Language conventions" table used to say "`0t+`, `0t-`, `0t0` (prefix trit literal)" — misleading. The confusion first surfaced while writing `compiler/main.tri` v0.7.9.4 returning `Trit` exit codes: using `0t+ / 0t-` raised 7 Mismatch errors before being corrected to `1_trit / -1_trit`. SPEC §1.5.1 (line 165, `1_trit              // Trit`) is the source of truth and overrides the CLAUDE.md table.

**How to apply:** when a `Trit` literal is needed in Triết source, use `1_trit / 0_trit / -1_trit`. Reserve `0t+ / 0t- / 0t0` for balanced-ternary Integer literals (rarely needed — ordinary Integer literals are decimal). Helper functions like `exit_ok() -> Trit = 1_trit` / `exit_err() -> Trit = -1_trit` are a good pattern when many call sites need the same value.

Related: [[feedback_syntax_verbose_dot_paths]] (the Triết verbose-keyword tradition), [[reference_spec]] (SPEC §1.5.1).
