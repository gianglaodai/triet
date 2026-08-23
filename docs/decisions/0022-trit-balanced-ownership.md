# ADR 0022 — Trit-Balanced Ownership

**Status:** **Locked** (promoted via v0.8.x.review 2026-05-28; supersedes 2026-05-22 initial sketch). Foundation for v0.8 Ownership + Concurrency Model — has shipped ObjectHeader (`triet-core::memory`), 5-form lexer tokens, parser AST `ReferenceForm`, type-system resolve transparently per v0.8.3–v0.8.6. Locks semantics for 5-form reference syntax + mutability + aliasing + cycle policy + self-ref capability + Outcome integration. Detailed enforcement algorithm extracted to [ADR-0025](0025-borrow-checker-rules.md). Concurrency Send rules extracted to [ADR-0026](0026-actor-boundary-send-rules.md).

**Issue:** Triet aims to be OS-capable per [VISION §3.5 + §5](../../VISION.md) — must have a memory model as strict as Rust but:
1. **No `unsafe` keyword** — all hazards are managed via [capability system (ADR-0018)](0018-capability-loader-semantics.md), making it audit-friendly.
2. **No lifetime annotation `<'a>`** — viral annotations are the largest cognitive barrier in Rust.
3. **Ternary identity** — reference syntax must map to trit `{+1, 0, -1}` to remain consistent with [VISION §5 ternary first-class](../../VISION.md).
4. **Zero runtime overhead** — no runtime refcount, no cycle collector, no generational check.

Author's 2026-05-25 design session with AI assistant evaluated 5 scenarios: (S1) Rust-renamed, (S2) Hylo mutable value semantics, (S3) Vale generational references, (S4) Pony reference capabilities, (S5) hybrid gen-refs + actor isolation. Author finalized on 2026-05-26: **S6 — Rust-strict static borrow check + ternary syntax + capability-as-un-safe**, prioritizing: strictness, compile-time error catching, performance, and AI-friendliness. Generational references (S3/S5) were rejected because they shift errors to runtime + 1-2% overhead, contrary to priorities.

This ADR locks the conceptual model. The v0.8 implementation phase only handles parser tokens; full enforcement is deferred to v0.9-v1.0 per [ADR-0025](0025-borrow-checker-rules.md) §10.

---

## §1 — Context & Problem Solved

### 1.1 — Problems Triet must solve

Triet must be capable of writing kernels/OSs, which implies:

| System programming problem | Rust solution | Where Triet must be better |
|---|---|---|
| Doubly-linked list, graph cycles | `Rc<RefCell>` + `Weak` | Verbose equivalent but WITHOUT the `unsafe` keyword |
| Self-referential struct (parsers, future state) | `Pin` + `unsafe` | Capability `dev::self_ref` instead of `unsafe` |
| MMIO, FFI, raw pointer | `unsafe` block | Capability `sys::io.memory` / `dev::ffi` |
| Viral lifetime annotation `<'a, 'b>` | Elision rules covers ~70% | **Complete removal of annotation syntax** |
| Custom collection internals | Extensive `unsafe` | Capability `dev::raw_memory` |

### 1.2 — Author's 4 priorities (finalized 2026-05-26)

1. **Strict** — refuse-over-guess per VISION §6.
2. **Maximum compile-time error catching** — runtime checks only where truly unavoidable (e.g., array bounds).
3. **Performance** — zero-cost abstraction, no runtime refcount in the core language.
4. **AI-friendly** — minimal concepts, explicit syntax, error messages with fix suggestions. Compile-time errors > runtime errors for AI debugging.

Accepted trade-off: doubly-linked lists / cycles must be broken using `&-`, without the "natural" behavior of Vale gen-refs. Self-ref structs must pass through a capability gate. In exchange: zero runtime overhead + 100% compile-time checking.

### 1.3 — Decisions D1–D3 finalized in this ADR

| ID | Decision | Rationale |
|---|---|---|
| **D1** | `&+` is the **unique/exclusive owner** (no free cloning in the core language) | Zero runtime overhead, compile-time exclusivity check is feasible |
| **D2** | Default **read-only everywhere** (variable, parameter, struct field). Explicit `mutable` keyword to allow mutation | Brand fit "stability over speed", similar to Rust 2018+ default |
| **D3** | Self-ref structs are **denied by default**, unlocked via capability `dev::self_ref` (offset-based pattern) | Refuse-over-guess; avoids Pin/unsafe complexity |

D4–D7 are finalized in ADR-0025 and ADR-0026 (borrow checker + thread boundary send).

---

## §2 — Five Reference Types (Syntax Lock)

**Lock:** Triet has exactly 5 reference types, ordered from strongest to weakest:

| Syntax | Name | Ownership | Write Access | Aliasing | Rust Equivalent |
|---|---|---|---|---|---|
| `&+ T` | Strong owner, frozen | Unique owner | Read-only | No cloning | `Box<T>` (frozen) |
| `&+ mutable T` | Strong owner, mutable | Unique owner | Mutable | No cloning | `Box<T>` |
| `&0 T` | Scope borrow, read-only | Borrow | Read-only | Multiple handles OK | `&T` |
| `&0 mutable T` | Scope borrow, mutable exclusive | Borrow | Mutable | **Exclusive** (1 at a time) | `&mut T` |
| `&- T` | Weak observer | No ownership | Read-only after upgrade | Multiple handles OK | `Weak<T>` (compile-time) |

### 2.1 — Why both `&+ T` (frozen) and `&+ mutable T` exist

Java analogy: `final User u = new User(...)` (frozen owner) vs `User u = new User(...)` (mutable owner). Both are the sole owner, differing only in mutation rights.

`&+ T` (frozen) exists to **safely send across thread boundaries** (see [ADR-0026](0026-actor-boundary-send-rules.md) §3) — frozen ≡ immutable share-able. `&+ mutable T` cannot be Sent (mutable shared = race condition).

### 2.2 — Why there is no syntax for "shared owner" (Arc/Rc equivalent)

Per **D1**, the core language does not have shared ownership. Reasons:

- **Performance:** Both Rc/Arc have refcount overhead. Arc atomic ops are particularly expensive.
- **Compile-time clarity:** Unique ownership allows compile-time exclusivity checks without runtime guards.
- **Brand fit:** Triet accepts being more verbose than Rust in certain patterns in exchange for zero-cost + compile-time rigor.

When it is truly necessary to share an immutable object cross-thread, [ADR-0026](0026-actor-boundary-send-rules.md) will allow an **implicit automatic** refcount at the thread boundary — but this will not be exposed to the user-facing language.

### 2.3 — Why there is no `&+ mutable shared T` (Rc<RefCell> equivalent)

Mutable sharing is the source of data races + iterator invalidation. Rust solves this with `RefCell` (runtime borrow check, panic on violation). Triet rejects this because:

- Runtime panics violate the "compile-time error catching" priority.
- This pattern can be replaced by a message-passing pattern in 95% of cases (encapsulating mutable state in one thread/context, querying/updating via messages).

For the remaining 5% edge case (
