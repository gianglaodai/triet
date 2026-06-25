---
name: feedback-no-abbreviations
description: "Triết identifiers (types, builtins, stdlib paths, parameter names) must spell out — Java naming convention, never abbreviate. Vec→Vector, len→length, pkg→package, etc."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 3bf5f09d-e8ea-4e4f-ab53-a3fac956c547
---

User is a Java developer and is allergic to abbreviated identifiers. Triết code (types, builtin names, stdlib paths, parameter names, struct fields) must always spell words out fully — follow Java's naming convention.

**Why:** explicit confirmation 2026-05-17 after proposing `Vec` as TypeTag name. User reply: *"tôi rất dị ứng với viết tắt, việc viết tắt Vector thành Vec là không hợp ý tôi. Nguyên tắc đặt tên của java luôn phải được duy trì cho tôi nhé."* Connects to earlier verbose-keyword preference ([[feedback-syntax-verbose-dot-paths]]) — same Java-sensibility principle applied to identifiers, not just keywords.

**How to apply:**

Triết-facing identifiers (`.tri` source, TypeTag variants, BuiltinName variants, stdlib module paths, function names visible to user code):
- `Vector<T>` not `Vec<T>`
- `length` not `len`
- `HashMap<K,V>` is fine (Java has `java.util.HashMap`, not an abbreviation)
- `Iterator<T>` not `Iter`
- `package` not `pkg`
- `metadata` not `meta`
- `function` not `func` / `fn`
- `parameter` / `argument` not `param` / `arg` in public API surface

Rust-internal naming (struct field names inside `crates/*/src`, local variables, helper functions):
- Existing names like `func_table`, `pkg_name`, `meta`, `fd: FunctionDef` — DO NOT retroactively rename. They live in Rust impl side, not Triết user surface. CLAUDE.md "Surgical Changes" principle applies.
- New Rust code I write: lean toward full names but pragmatic per Rust idiom (e.g. `fn` keyword is Rust, can't change).

Where to enforce: TypeTag enum variants, BuiltinName enum variants, stdlib `.tri` function names, user-facing diagnostic message strings, ADR text describing Triết identifiers, ROADMAP/SPEC tables that name Triết entities.

Where NOT to enforce (Rust-internal): `Box<dyn Any>`, `Vec<T>` (Rust's stdlib type), `HashMap` (also Java compatible), `Arc`, `Rc`, `Cell` — Rust idioms inside Rust impl crates.
