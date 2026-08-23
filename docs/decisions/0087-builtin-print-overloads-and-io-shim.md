# ADR 0087 — Builtin Print — Overloads & I/lar I/O Shim

**Status:** Approved (Mentor G signed 2026-07-25). Applies to Tier C+.

**Issue:** `print`/`println` are the FIRST stdout-writes designed for the backend rewrite. Typecheck has declared both (`crates/triet-typecheck/src/env.rs:144` for `print`, `:152` for `println`, both `String → Unit`), but the lowerer (`crates/triet-lower/src/lib.rs`) LACKS builtin arms for them: `match callee_name.as_str()` at `:2661` lists `concat`/`len`/`vector_new`/`push`/… but does not include `"print"`/`"println"`, causing calls to fall through to the default arm `_ => { /* fall through to user-defined function dispatch */ }` at `:3241` — the lowerer treats `print`/`println` as user-defined functions, fails to find definitions, and the JIT fails with `callee 'println' not found`, exit 4. This is NOT a silent miscompile (no invariants are silently violated) — it is purely a feature gap: typecheck promises the functionality, but the backend does not lower it.

## Decision

### 1. Four signature overloads

`print(String)`, `print(&0 String)`, `println(String)`, `println(&0 String)`.

- **Owned `String`** (by-value) = MOVE = consumes the value — the caller cannot reuse the variable after the call (mirrors the ownership rules of all existing functions that accept `String` by-value, ADR-0042).
- **`&0 String`** (borrow, read-only reference) = Reference is Copy (S6, SPEC §10) → reusable after printing.

Both forms are required because: printing a literal/temporary expression (natural move, no need to retain) and printing a variable that will be used later (requires borrow) are equally common scenarios in real-world code; supporting only one would force programmers to use artificial `concat`/`arg_clone` just to print to the screen.

### 2. Four separate extern-C shims by symbol name (without passing an `is_owned` flag)

| Signature | Shim | arity | `arg_consumes` | Behavior |
|---|---|---|---|---|
| `print(String)` | `__triet_print` | 3 (ptr, len, cap) | `[true]` | write ptr..len to stdout → `free(ptr, cap)` |
| `print(&0 String)` | `__triet_print_ref` | 2 (ptr, len) | `[false]` | write only, no free |
| `println(String)` | `__triet_println` | 3 | `[true]` | write + `\n` → `free` |
| `println(&0 String)` | `__triet_println_ref` | 2 | `[false]` | write + `\n`, no free |

Memory responsibility is hardcoded into the SYMBOL NAME (4 distinct symbols), rather than passing a runtime `is_owned` flag to a single shared shim to branch between free/no-free.

- **Owned = `arg_consumes: [true]`:** move-in implies the callee (shim) owns the value ⇒ the shim performs `free`. The caller-side slot is zeroed by M3 (move-tracking is already present for all consuming calls) ⇒ Deinit of the caller sees an empty slot ⇒ `free(0)` is a no-op ⇒ exactly one free over the lifetime. This pattern has precedent with `__triet_vector_push` (when pushing an owned `String` into a `Vector<String>`).
- **Ref = `arg_consumes: [false]`:** the owner retains ownership; `free` occurs in the owner's scope as usual (Deinit tombstone, ADR-0042), not in the shim.

### 3. Return `Unit` properly — no throwaway `i64`

Add a branch to handle return-Unit in `emit_
