---
name: feedback-explicit-strictness
description: API contract design — dangerous (panic-possible) operations MUST be verbose methods; property access MUST be 100% safe. Rust-strict over Java-ergonomic.
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 3bf5f09d-e8ea-4e4f-ab53-a3fac956c547
---

User favors **explicit strictness over ergonomic-but-dangerous APIs**. Two enforced rules:

1. **Property access (field-like) MUST be 100% safe** — no panic-possible field syntax. If runtime panic is possible, it MUST be a method (verbose, visible at call site).
2. **Dangerous operations MUST be verbose methods** — names like `.unwrap_value(message: String)`, `.unwrap_error(message: String)` that include a panic-reason message. The method call's verbosity serves as a reading-time warning sign.

**Why:** explicit confirmation 2026-05-17 during ADR-0020 error handling design. User: *"Tuyệt đối KHÔNG dùng field access (.value / .error) cho các thao tác có thể sinh Panic. Thuộc tính (Property access) phải đảm bảo khế ước 100% safe. Nếu có rủi ro abort VM, hành vi đó phải là một Method rõ ràng để nhắc nhở developer. Chúng ta ưu tiên Explicit Strictness (chuẩn Rust) hơn Ergonomics nguy hiểm (chuẩn Java/C++)."*

**How to apply:**

Designing new API surfaces — when proposing access patterns:

- **Property/field** (e.g. `.is_success`, `.length`, `.name`): MUST be infallible. Returns a value or a nullable, never panics.
- **Method with message** (e.g. `.unwrap_value("config must exist")`, `.expect_present(...)`): allowed to panic, MUST take a message argument explaining the panic condition. Reading the call site reveals the danger.
- **Method without message** (e.g. `.get(key)`): MUST return nullable or Result-like — never silently panics.

Java's `Optional.get()` and Kotlin's `!!` are **anti-patterns** by this rule (short, dangerous). Rust's `Option::unwrap()` is closer (verbose method) but lacks required message — Triết goes stricter: panic methods MUST accept a String message.

**Related precedent:**
- VM error model 3-tier (ADR-0019 Addendum §A7) — bug vs data event split. This rule reinforces by making bug-tier methods visually distinct from data-tier properties.
- `!!` null-unwrap on `T?` (existing, SPEC §2.5) is the historical exception. User accepts it because `T?` is a primitive language type, not a struct API. New struct APIs (Outcome, Vector, HashMap, etc.) follow the stricter rule.

**Tagline anchor:** "Explicit Strictness over dangerous Ergonomics." Use this phrasing in ADRs when invoking the principle.

Connects to [[feedback-no-abbreviations]] (Java spell-out) and [[feedback-stability-over-speed]] (no ship-and-fix), but distinct: this is about **contract clarity at API boundaries**, not naming or pace.
