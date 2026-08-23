# ADR 0003 — Iterator protocol for `for`

**Status:** Shape decided, full implementation in v0.2 (alongside generics). v0.1 hardcodes `Range` + `Enumerate`; refactoring has established the `advance_iterator` helper in the interpreter as the foundation (commit `06025bb`).

**Issue:** SPEC §13 #3 — `Iterator` trait for `for` loops. v0.1 is hardcoded; v0.2 must provide a user-extensible protocol.

## Decision

**Two Rust-style traits** (similar to Mojo, Rust, Swift), where `next()` returns `T?` (a nullable primitive — NOT `Option<T>`).

```triet
trait Iterator<T> {
    fn next(self: mut Self) -> T?
}

trait Iterable<T> {
    fn iter(self) -> Iterator<T>
}
```

`for x in expr { body }` desugaring (compiler-internal):

```triet
let __iter = expr.iter()
loop {
    let __next = __iter.next()
    if? __next == null { break }
    let x = __next!!
    body
}
```

### Why `T?` instead of `Option<T>`?

- `T?` is already a primitive in v0.1, requiring no generics for its definition.
- Using a nullable primitive for the Iterator allows users to define iterators for custom types **before** `Option<T>` (the v0.2 generic) is stabilized.
- There is no semantic requirement for `next()` to distinguish between "value exists, but value is null" and "end of stream" — `T?` is sufficiently unambiguous. (In cases where distinction is required, use wrapping: `Iterator<T?>` for a stream of nullables, where `next()` returns `T??`.)
- Consistent with SPEC §2.5: `T?` = check-and-use, `Option<T>` = pipeline. The `next()` loop in an Iterator is clearly a check-and-use pattern immediately following the call.

### Adapter pattern

`map`, `filter`, `take`, `skip`, `zip`, `chain`, `enumerate` — all are methods on `Iterator<T>` returning `Iterator<U>` (lazy). This will be enabled by v0.2 generics.

`enumerate` in v0.1 is hardcoded within the `Value::Enumerate` enum — it will be refacted into an adapter struct using the trait when v0.2 is shipped.

## Rationale

- **Familiarity.** Rust/Mojo/Swift use this pattern. LLMs are trained on extensive data using this pattern, making it highly suitable for an AI-first approach.
- **Lazy by default.** Iterator chains do not materialize until consumed — efficient for large or infinite sequences.
- **Mutable receiver `mut self`.** Aligns with the Mojo memory convention in SPEC §10.3: stream advancement is a mutation, explicitly denoted by `mut`.
- **Not push-based (visitor).** While `for_each(|t| ...)` is simple, it cannot cleanly implement `break`/`continue` without breaking `for` semantics in §7.2.

## Consequences

- In v0.1, the `Range` and `Enumerate` interpreter dispatch (`advance_iterator`) serves as the internal-only equivalent of `Iterator::next()`. Once the `Iterator` trait lands in v0.2, these `Value` variants will be wrapped in structs implementing `Iterator` → user code remains unchanged.
- `for` desugaring uses `loop { ... break }` — it does not bind the expression value (while `loop` in §7.2 supports break-with-value, `for` does not require it). The compiler can optimize this away once the Cranelift backend arrives in v0.3.
- Separating the `Iterable` trait from `Iterator` allows a collection to be iterated multiple times (`coll.iter()` can be called twice), whereas a raw `Iterator` (once in flight) cannot.

## Implementation roadmap

| Phase | Deliverable | Status (as of v0.7.3.2) |
|---|---|---|
| v0.1 ✅ | Hardcoded `Range`, `Enumerate` via `advance_iterator` (commit `06025bb`) | shipped |
| v0.2 | `Iterator<T>` and `Iterable<T>` traits; refactor `Range`/`Enumerate` into `Iterable` structs; `map`/`filter`/`take`/`zip` adapters | **NOT LANDED** — slipped past v0.2/v0.3/v0.4/v0.5/v0.6 phases. Re-tracked as a deferred item in [ADR-0019 Addendum §A7](0019-self-hosting-compiler-bootstrap.md). Target re-tackle: v0.8 (concurrency model reframes iterator+stream protocols). |
| v0.3 | Performance pass: avoid allocations for adapter chains (state machine fusion) | deferred — depends on v0.2 deliverable landing first |

**v0.7.3.2 implication:** `BuiltinName::VectorIterator` was specified in ADR-0019 §5 but dropped per Q2-A — the `Iterator` trait gap
