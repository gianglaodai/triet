# ADR 0022 — Trit-Balanced Ownership

**Status:** **Locked** (promoted via v0.8.x.review 2026-05-28; supersedes 2026-05-22 initial sketch). Foundation for the v0.8 Ownership + Concurrency Model — shipped ObjectHeader (`triet-core::memory`), 5-form lexer tokens, parser AST `ReferenceForm`, type-system resolution transparently per v0.8.3–v0.8.6. Locks semantics of 5-form reference syntax + mutability + aliasing + cycle policy + self-ref capability + Outcome integration. Detailed enforcement algorithm separated into [ADR-0025](0025-borrow-checker-rules.md). Concurrency Send rules separated into [ADR-0026](0026-actor-boundary-send-rules.md).

**Issue:** Triet aims to be OS-capable per [VISION §3.5 + §5](../../VISION.md) — requiring a memory model as rigorous as Rust, but:
1. **Without the `unsafe` keyword** — all unsafe operations route through the [capability system (ADR-0018)](0018-capability-loader-semantics.md), ensuring auditability.
2. **Without lifetime annotations `<'a>`** — viral annotations represent Rust's largest cognitive hurdle.
3. **Ternary identity** — reference syntax must map to trits `{+1, 0, -1}` for consistency with [VISION §5 ternary first-class](../../VISION.md).
4. **Zero runtime overhead** — no runtime refcounting, no cycle collectors, no generational checks.

During the 2026-05-25 design session, the author evaluated 5 scenarios: (S1) Rust-renamed, (S2) Hylo mutable value semantics, (S3) Vale generational references, (S4) Pony reference capabilities, (S5) hybrid gen-refs + actor isolation. On 2026-05-26, the author finalized **S6 — Rust-strict static borrow checking + ternary syntax + capability-as-unsafe**, prioritizing: strictness, compile-time error detection, performance, and AI-friendliness. Generational references (S3/S5) were rejected because they shift errors to runtime and introduce 1–2% overhead, opposing project priorities.

This ADR locks the conceptual model. Implementation in phase v0.8 covers only parser tokens; full enforcement is deferred to v0.9–v1.0 per [ADR-0025](0025-borrow-checker-rules.md) §10.

---

## §1 — Context & Problem Statement

### 1.1 — Systems programming challenges

To write kernels and operating systems, Triet must resolve fundamental systems programming challenges:

| Systems programming problem | Rust solution | Triet objective |
|---|---|---|
| Doubly-linked lists, graph cycles | `Rc<RefCell>` + `Weak` | Equivalent expressiveness WITHOUT the `unsafe` keyword |
| Self-referential structs (parsers, future states) | `Pin` + `unsafe` | Capability `dev::self_ref` replacing `unsafe` |
| MMIO, FFI, raw pointers | `unsafe` blocks | Capability `sys::io.memory` / `dev::ffi` |
| Viral lifetime annotations `<'a, 'b>` | Elision rules cover ~70% | **Completely eliminate annotation syntax** |
| Custom collection internals | `unsafe` extensively | Capability `dev::raw_memory` |

### 1.2 — Author's 4 priorities (Locked 2026-05-26)

1. **Strictness** — refuse-over-guess per VISION §6.
2. **Maximize compile-time error detection** — runtime checks only where strictly unavoidable (e.g. array bounds).
3. **Performance** — zero-cost abstractions, no runtime refcounting in the core language.
4. **AI-friendly** — minimal concepts, explicit syntax, error messages with actionable fix suggestions. Compile-time errors > runtime errors for AI debugging.

Accepted trade-offs: doubly-linked lists / cycles must be broken using `&-`, lacking the seamlessness of Vale gen-refs. Self-referential structs must pass through capability gates. In exchange: zero runtime overhead + 100% compile-time verification.

### 1.3 — Decisions D1–D3 locked in this ADR

| ID | Decision | Rationale |
|---|---|---|
| **D1** | `&+` is **unique/exclusive owner** (no unrestricted cloning in core language) | Zero runtime overhead, compile-time exclusivity verification becomes feasible |
| **D2** | Default **read-only everywhere** (variables, parameters, struct fields). Explicit keyword `mutable` to enable mutation | Fits "stability over speed" brand, aligning with Rust 2018+ defaults |
| **D3** | Self-referential structs **prohibited by default**, unlocked via capability `dev::self_ref` (offset-based pattern) | Refuse-over-guess; eliminates Pin/unsafe complexity |

D4–D7 are finalized in ADR-0025 and ADR-0026 (borrow checker + thread boundary sending).

---

## §2 — Five Reference Forms (Syntax Locked)

**Lock:** Triet provides exactly 5 reference forms, ordered from strongest to weakest:

| Syntax | Name | Ownership | Mutation Rights | Aliasing | Rust Equivalent |
|---|---|---|---|---|---|
| `&+ T` | Strong owner, frozen | Unique owner | Read-only | No cloning | `Box<T>` (frozen) |
| `&+ mutable T` | Strong owner, mutable | Unique owner | Mutable | No cloning | `Box<T>` |
| `&0 T` | Scope borrow, read-only | Borrow | Read-only | Multiple handles OK | `&T` |
| `&0 mutable T` | Scope borrow, mutable exclusive | Borrow | Mutable | **Exclusive** (1 at a time) | `&mut T` |
| `&- T` | Weak observer | Non-owning | Read-only after upgrade | Multiple handles OK | `Weak<T>` (compile-time) |

### 2.1 — Why both `&+ T` (frozen) and `&+ mutable T` exist

Java analogy: `final User u = new User(...)` (frozen owner) vs `User u = new User(...)` (mutable owner). Both represent unique owners, differing in mutation permissions.

`&+ T` (frozen) exists to **safely cross thread boundaries** (see [ADR-0026](0026-actor-boundary-send-rules.md) §3) — frozen ≡ immutable and shareable. `&+ mutable T` cannot be Sent (shared mutability causes race conditions).

### 2.2 — Why no "shared owner" syntax exists (Arc/Rc equivalent)

Per **D1**, the core language contains no shared ownership primitives. Rationale:

- **Performance:** Rc and Arc impose refcounting overhead. Arc atomic operations are particularly expensive.
- **Compile-time clarity:** Unique ownership enables compile-time exclusivity checking without runtime guards.
- **Brand fit:** Triet accepts greater verbosity than Rust in select patterns in exchange for zero runtime cost and compile-time rigor.

When immutable cross-thread sharing is genuinely required, [ADR-0026](0026-actor-boundary-send-rules.md) enables **implicit automatic refcounting** at thread boundaries — without exposing it to user-facing surface syntax.

### 2.3 — Why no `&+ mutable shared T` exists (Rc<RefCell> equivalent)

Shared mutability is the root cause of data races and iterator invalidation. Rust resolves this via `RefCell` (runtime borrow checking, panicking on violation). Triet rejects this approach because:

- Runtime panics violate the "maximize compile-time error detection" priority.
- 95% of these patterns are better structured using message-passing (encapsulating mutable state within a single thread/context, querying/updating via messages).

The remaining 5% edge cases (single-threaded interior mutability): handled via `Cell<T>` for primitive copy types (planned post-v1.0), or refactored into message-passing architectures.

---

## §3 — Mutability Rules (D2: Read-only by default)

**Lock:** Read-only is the default across 3 positions: variable bindings, function parameters, struct fields. The `mutable` keyword explicitly enables mutation.

### 3.1 — Variable binding

```triet
let x = 10              // immutable binding (rebinding prohibited)
let mutable y = 20      // mutable binding (y = 30 OK; rebinding type not allowed)
```

This pertains strictly to variable bindings, independent of reference types. `mutable` on a binding ≠ `mutable` on a reference.

### 3.2 — Function parameter

```triet
function greet(name: String)            // name: &0 String (read-only borrow, default)
function append(buf: &0 mutable Bytes)  // exclusive mutable borrow
function consume(owned: &+ String)      // take ownership (move semantics)
```

Parameters default to `&0` inference (see §5). Explicit `&+` takes ownership. Explicit `&0 mutable` allows mutation across borrows.

### 3.3 — Struct field

```triet
public struct Process {
    pid: Integer,                          // immutable field (set once at construction)
    mutable state: ProcessState,           // mutable field (modifiable post-construction)
    children: Vector<&+ Process>,          // immutable field, owned children
    mutable parent: &- Process             // mutable field, weak ref to parent
}
```

`mutable` on a struct field permits modifying that field after instantiation. Omitting it means the field is initialized once in the constructor and remains immutable thereafter.

### 3.4 — "Frozen forever" cannot be promoted to mutable

Strict rule: **`&+ T` can NEVER be promoted to `&+ mutable T`.** Frozen means permanently frozen.

```triet
let owner: &+ User = create_user("Alice")
owner.name = "Bob"                      // E2410 CannotMutateFrozenOwner
let mutable_owner: &+ mutable User = owner  // E2411 CannotPromoteFrozenToMutable
```

Rationale: allowing promotion invalidates the frozen guarantee — developers could bypass immutability via temporary promotion. Consequently, authors must decide between frozen and mutable at construction.

Error codes E2410–E2419 are reserved for mutability violations (detailed in ADR-0025 §4).

---

## §4 — Aliasing Rules (D1: Linear/Unique `&+`)

**Lock:** Each heap allocation has **exactly one `&+`** at any given moment. Sharing requires `&0` borrowing.

### 4.1 — Move semantics when passing `&+`

```triet
function take(owned: &+ User) { /* ... */ }

let alice: &+ User = create_user("Alice")
take(alice)            // ownership moves into take(); alice no longer usable
print(alice.name)      // E2420 UseAfterMove
```

Contrast with Rust: Rust moves `Box<T>` but borrows `&T`. Triet is more syntactically explicit — reading `&+` immediately signals that a move will occur.

### 4.2 — Borrowing does not move

```triet
function read(borrowed: &0 User) { print(borrowed.name) }

let alice: &+ User = create_user("Alice")
read(alice)                  // implicit borrow: alice → &0 alice
print(alice.name)            // OK, alice retains ownership
```

The compiler automatically borrows `&+` into `&0` when passing into functions expecting `&0`. No explicit `&` operator is required as in Rust.

### 4.3 — Exclusive mutable borrowing

```triet
function mutate(borrowed: &0 mutable User) { borrowed.name = "Bob" }

let mutable alice: &+ mutable User = create_user("Alice")
mutate(alice)                // temporary exclusive mutable borrow
print(alice.name)            // OK, alice retains ownership, observes "Bob"
```

At any point in time: 1 `&0 mutable` XOR N `&0`. Enforced by the compiler via Non-Lexical Lifetimes (NLL) per [ADR-0025 §2](0025-borrow-checker-rules.md). Violations trigger the E2400 error series.

### 4.4 — Why linear/unique rather than refcounting

Refcounting (Rc/Arc) permits multiple coexisting `&+` handles, incrementing counters on clones. Triet rejects this because:

1. **Runtime cost:** atomic operations for Arc, non-atomic increments for Rc — violating performance priorities.
2. **Cycle vulnerability:** refcounting cannot reclaim cyclic structures without cycle collectors (runtime overhead + non-deterministic reclamation) or requiring explicit Weak handles.
3. **Compile-time predictability:** unique ownership enables static exclusivity checks; refcounting cannot.

Cross-thread immutable sharing is handled in [ADR-0026](0026-actor-boundary-send-rules.md) using **implicit** refcounting, without exposing syntax to user code. Memory layout details (8-byte object header containing a refcount field) are locked in [ADR-0026 §7](0026-actor-boundary-send-rules.md) under Scenario A ("header always present") rather than lazy box-wrapping, as lazy wrapping breaks compile-time invariants of `&-` weak references.

---

## §5 — Default Inference by Namespace

**Lock:** In the `usr::` namespace, the compiler infers reference forms when omitted by the author. In `sys::` / `dev::`, inference is prohibited — explicit annotations are mandatory.

### 5.1 — Inference rules in `usr::`

| Position | Implicit Default | Explicit Override |
|---|---|---|
| Struct field type `field: T` | `&+ T` (owned, immutable) | `&+ mutable T`, `&- T`, `T` (value type only) |
| Function param `param: T` | `&0 T` (borrow, read-only) | `&0 mutable T`, `&+ T`, `T` (value type only) |
| Function return `-> T` | Inferred from body (owned or input-tied borrow) | Explicit `-> &0 T` |
| `let x = expr` | Inferred from expr | Explicit `let x: &+ T = expr` |

Example:

```triet
// usr namespace — implicit refs
module usr.account

public struct Account {
    id: AccountId,             // value type, no ref
    owner: User,               // implicitly &+ User (struct field default)
    balance: Money             // value type, no ref
}

public function transfer(from: Account, to: Account, amount: Money) {
    // from, to implicitly &0 Account (param default)
    // ...
}
```

### 5.2 — Prohibition of inference in `sys::` / `dev::`

```triet
module sys.kernel.scheduler

public struct Process {
    pid: Integer,              // explicit OK (value type)
    state: &+ mutable ProcessState,    // MANDATORY explicit
    parent: &- Process                 // MANDATORY explicit
}

public function schedule(proc: ProcessHandle) {  // E2208 LayoutNotExplicit
    // sys namespace mandates explicit annotations
}

public function schedule(proc: &+ ProcessHandle) {  // OK
    // ...
}
```

Error code E2430 `ImplicitRefInSystemNamespace` is reserved for this violation (defined in ADR-0025).

### 5.3 — Rationale for tiering

VISION §3.5 defines 3 namespaces with distinct authority levels. `usr::` optimizes for developer ergonomics, while `sys::` / `dev::` enforces strict discipline. This pattern is consistent with [ADR-0018 §1](0018-capability-loader-semantics.md) — capabilities must also be explicitly declared in `sys::` / `dev::`.

---

## §6 — The Acyclicity Theorem of Linear Ownership

**Lock:** Triet **does not require** cycle detection algorithms, cycle collectors, or garbage collection. The unique ownership rule (D1) **makes the runtime creation of `&+` cycles mathematically impossible**. The compiler requires zero lines of code for cycle checking.

### 6.1 — Theorem statement

> **Acyclicity Theorem:** In any valid Triet program, there exists no closed path in the runtime object graph where all edges are `&+`.

This is a direct mathematical consequence of D1 (`&+` unique/linear) + move semantics (§4.1). No compiler search is required — the language syntax itself prevents cycles at compile time via use-after-move detection.

### 6.2 — Proof sketch (move semantics prevent cycle formation at assignment)

Suppose an author attempts to construct a 2-node cycle `A ⇄ B` where both edges are `&+`:

```triet
let a: &+ A = create_a()       // step 1: a owns A. Owner chain: caller → a
let b: &+ B = create_b()       // step 2: b owns B. Owner chain: caller → b
a.b_field = b                  // step 3: move b INTO a.b_field
                               //         Owner chain now: caller → a → b
                               //         Following this step, variable `b` is consumed.
b.a_field = a                  // step 4: ERROR — `b` was moved in step 3.
                               //         E2420 UseAfterMove.
```

Attempting a nested path:

```triet
a.b_field.a_field = a          // step 4': attempt moving `a` into a-field-of-b-field
                               //          RHS is `a`. LHS belongs to ownership tree of a.
                               //          To move `a`, `a` must be a free owner —
                               //          yet the LHS reads through `a` to resolve the address.
                               //          E2421 SelfOwnershipParadox (defined in ADR-0025).
```

Extending to n-node cycles: identical principles apply. Each `&+` edge represents a move, and each move consumes the source variable. Closing a cycle requires moving an already-consumed variable → impossible.

**Conclusion:** Linear ownership is not a heuristic cycle-prevention algorithm. It is a **structural invariant** — cycles consisting entirely of `&+` edges cannot be expressed in the language.

### 6.3 — Practical consequences

All **bidirectional data structures** (doubly-linked lists, trees with parent pointers, graphs with back-edges) **must** use `&-` for reverse edges. This is not a lint recommendation — it is a structural law of the language.

```triet
public struct DListNode {
    value: Integer,
    next: (&+ DListNode)?,      // forward owns next (nullable for tail terminator)
    prev: &- DListNode           // backward weak — structural law
}

public struct TreeNode {
    children: Vector<&+ TreeNode>,
    parent: &- TreeNode          // structural law
}
```

There is no bypass around `&-` for back-edges. Attempting `&+` in both directions triggers E2420 or E2421 — refuse-over-guess.

### 6.4 — Compiler checks exactly one property: constructibility termination

Because `&+ T` is an indirection (pointer-sized), **type sizes are always finite** even with recursive definitions like `struct Node { next: &+ Node }`. There is no infinite-size hazard.

The compiler checks **constructibility termination**: if type T contains an `&+ T` field (directly or transitively), constructor expressions must provide a **base case** to terminate:

| Pattern | Base case | Constructible? |
|---|---|---|
| `struct Node { next: &+ Node }` | None | ❌ E2422 NonTerminatingConstruction — must wrap in nullable |
| `struct Node { next: (&+ Node)? }` | `~0` | ✅ chain terminates with `~0` |
| `struct Node { children: Vector<&+ Node> }` | `empty()` | ✅ Vector can be empty |
| `struct Node { parent: &- Node }` | Yes (weak null) | ✅ weak references possess natural null states |

This is a local **constructibility check**, not a global cycle search.

### 6.5 — No runtime collector, no GC

Because the theorem in §6.1 precludes runtime cycles, all `&+` objects are deterministically destroyed when their scope exits. There is no need for:

- ❌ Mark-and-sweep collectors (Java/Go)
- ❌ Reference counting with cycle detectors (Python)
- ❌ Generational GC (V8, .NET)

Triet **advances beyond Swift** (ARC + cycle leaks when developers forget `weak`) and **aligns with Rust** (zero-cost) — while eliminating lifetime annotations.

---

## §7 — Self-referential Structs (D3: Capability-gated)

**Lock:** `&0 T` fields in structs are prohibited by default (as borrow scopes cannot outlive structs). Unlocked via capability `dev::self_ref` — allowing offset-based patterns rather than true embedded pointers.

### 7.1 — Why prohibited by default

```triet
public struct Foo {
    data: Vector<Tryte>,
    cursor: &0 Tryte     // E2402 BorrowInStructField
}
```

`&0` is scope-bound — it cannot outlive its scope. Storing it inside a struct causes dangling pointers when the struct outlives the scope. Rust resolves this using `Pin` + `unsafe` (or crates like `ouroboros`).

### 7.2 — Capability `dev::self_ref` unlocks offset-based patterns

```triet
// dao.package
capabilities {
    dev::self_ref: grant
}

// within module sys::network
public struct NetworkPacket {
    buffer: &+ Vector<Tryte>,
    header_offset: Integer,     // OK — stores offset index, not raw pointer
    payload_offset: Integer
}

public function get_header(packet: &0 NetworkPacket) -> &0 Header {
    return slice_at(packet.buffer, packet.header_offset)
}
```

This pattern avoids storing raw `&0` references in structs — storing `Integer` offsets instead. Capability `dev::self_ref` documents intentional usage without introducing unsafe memory operations.

### 7.3 — No Pin equivalent

Rust's `Pin<&mut T>` allows storing true self-references. Triet prohibits this pattern — requiring authors to use offsets or indices. Trade-off: select Future state machine patterns are more verbose than Rust async, but v0.8 BYOS primitives handle the majority of use cases.

---

## §8 — Capabilities Replacing `unsafe` (Philosophy)

**Lock:** Triet **has no `unsafe` keyword**. All behaviors requiring `unsafe` in Rust are reframed as capability declarations in `dao.package` per [ADR-0018](0018-capability-loader-semantics.md).

### 8.1 — Ownership-related capability table

| Operation | Capability | Effect |
|---|---|---|
| Self-ref struct (offset-based) | `dev::self_ref` | Permits §7.2 patterns |
| Custom collection (raw allocation) | `dev::raw_memory` | Bypasses `&+` tracking, manual lifetime |
| Transmute / bit reinterpretation | `dev::reinterpret` | Casts bytes across differing layouts |
| FFI to C/extern | `dev::ffi` | Passes raw pointers to extern functions |
| MMIO / physical addresses | `sys::io.memory` | Reads/writes physical addresses |
| Custom destructor logic | `dev::custom_drop` | User-defined drop functions with order constraints |

All capabilities must be declared in the root `dao.package`. Auditors inspect a single file to evaluate the entire unsafe attack surface of the codebase.

### 8.2 — Why capabilities are superior to `unsafe` blocks

| Aspect | Rust `unsafe` | Triet Capability |
|---|---|---|
| Audit surface | Grep `unsafe {` scattered across crates | Single `dao.package` file |
| Granularity | All unsafe ops unified under 1 keyword | Dedicated capability per operation type |
| Per-package opt-in | Cargo features offer partial mitigation | Capabilities are first-class declarations |
| Runtime audit | Compile-time only | Compile + link + runtime resolver per [ADR-0017](0017-trilean-policy-hook.md) |
| Brand fit | "Memory safety with escape hatch" | "Memory safety with explicit capability" |

Rust's `unsafe` is binary (yes/no). Triet capabilities operate with 4 states per [ADR-0016 CapabilityLevel](0016-capability-type-system.md): `Grant`/`Ambient`/`Deny`/`Defer`, offering nuanced deployment control.

---

## §9 — Integration with Outcome (ADR-0020) — `&-` Dereferencing

**Lock:** Dereferencing `&- T` returns `T?` (nullable), NOT `T~UseAfterFree` (Outcome). Tracked at compile time without runtime generation checks.

### 9.1 — Upgrade syntax

```triet
function notify_parent(child: &0 Process) {
    let parent: &- Process = child.parent
    match parent.upgrade() {
        ~+ p => p.on_child_event(child),
        ~0 => log("parent never set")
    }
}
```

The `.upgrade()` method returns `T?` (nullable, 3-state per ADR-0001 + ADR-0020). Exhaustive pattern matching is mandatory — silent dereferencing is impossible.

### 9.2 — Why `T?` and not `T~UseAfterFree` (Outcome)

This decision supersedes S5 (initial AI assistant sketch from 2026-05-25). Rationale locked 2026-05-26:

- **Compile-time safety priority:** returning `T~UseAfterFree` would imply use-after-free is a valid runtime concept. Triet S6 locks: **`&-` never experiences use-after-free** because compile-time analysis ensures upgrades are matched prior to dereferencing. `T?` reflects "parent might not be initialized", not "parent was destroyed".
- **Zero runtime overhead:** `T~UseAfterFree` mandates runtime generation checks. Eliminated.
- **Consistency with ADR-0001:** `T?` already defines wire format and nullable semantics. Reused without introducing extraneous concepts.

### 9.3 — `&- T` is never a dangling pointer

Compile-time invariant: `&- T` exists only while at least one `&+ T` remains live in an enclosing scope, enforced by the borrow checker. When `&+` is dropped, all `&-` references to that object are out of scope (compile-time guaranteed) → no dangling references are observable from user code.

Edge case: storing `&-` into a struct that outlives `&+` triggers compile-time error E2403 `WeakRefOutlivesOwner`. Enforcement algorithm detailed in ADR-0025.

### 9.4 — Contrast with Rust `Weak<T>`

Rust's `Weak<T>` carries generation and pointer data at runtime; `.upgrade()` returns `Option<Rc<T>>` via runtime verification. Triet `&-` is a pure compile-time concept — no generation counters, no runtime checks, no runtime upgrade penalties.

Trade-off: Triet `&-` is more constrained than Rust `Weak` — it cannot cross thread boundaries freely (see ADR-0026 §3). In return: zero runtime overhead.

---

## §10 — Prior Art & Rationale for S6

| Language | Approach | Strengths | Weaknesses relative to Triet |
|---|---|---|---|
| Rust | Static borrow + lifetime annotations | Zero-cost, compile-time | Viral `<'a>` annotations |
| Mojo | Borrow conventions (`borrowed`/`inout`/`owned`) | Simple | Does not solve doubly-linked lists; no thread/BYOS story |
| Pony | 6 reference capabilities (iso/trn/ref/val/box/tag) | Concurrency-safe | Extremely steep learning curve |
| Hylo (Val) | Mutable value semantics, no references | No lifetimes | Forces complete restructuring of data-oriented code |
| Vale | Generational references (runtime check) | Solves cycles naturally | 1–2% overhead, runtime errors |
| Swift | ARC | Simple | Implicit refcounting overhead |

**Triet S6 = Rust static checking core + ternary syntax from original ADR-0022 + capabilities from ADR-0018 + Outcome integration from ADR-0020.**

Unique Triet innovations:
1. **Ternary syntax `&+/&0/&-`** mapped to trit identity.
2. **Capability-as-unsafe** — zero `unsafe` keywords.
3. **No lifetime annotation syntax** — compile errors with actionable fix suggestions on elision failure (see ADR-0025 §4).
4. **Frozen owner `&+ T`** distinct from mutable owner `&+ mutable T` — enabling natural cross-thread transfer.

---

## §11 — Out of Scope (Deferred to Sibling ADRs)

ADR-0022 locks the **conceptual model**. The following are deferred:

| Topic | ADR |
|---|---|
| Borrow checker algorithm (NLL, elision, use-after-move, constructibility termination) | [ADR-0025](0025-borrow-checker-rules.md) |
| Drop order, custom destructors, capability `dev::custom_drop` | ADR-0025 §6 |
| Move semantics (use-after-move detection) | ADR-0025 §8 |
| Send rules, cross-thread refcounting | [ADR-0026](0026-actor-boundary-send-rules.md) |
| FFI memory model (raw pointers, alignment, C-boundary ownership) | Future ADR (post-v0.8) |
| Generics and reference interactions (`Vector<&+ T>` vs `Vector<&0 T>`) | ADR-0025 §9 |
| Closure capture semantics (`Fn` vs `FnMut` equivalents) | Future ADR upon closure locking |
| Trait object references (`dyn Trait` equivalents) | Future ADR upon dynamic dispatch locking |

---

## §12 — Examples (4 Patterns)

### 12.1 — Process tree (back-edges mandate `&-` per §6.3)

```triet
public struct Process {
    pid: Integer,
    mutable state: ProcessState,
    children: Vector<&+ Process>,    // forward owns (Vector natural terminator)
    mutable parent: &- Process,      // back-edge — structural law
    mutable next_in_queue: &- Process
}

public function add_child(parent: &0 mutable Process, child: &+ Process) {
    push(parent.children, child)
}

public function notify_parent(p: &0 Process) {
    match p.parent.upgrade() {
        ~+ parent => parent.on_child_event(p),
        ~0 => log("root or unset parent")
    }
}
```

### 12.2 — Doubly-linked list (initialization syntax for `&-` parallels type position)

```triet
public struct DListNode {
    value: Integer,
    mutable next: (&+ DListNode)?,    // (&+ T)? — nullable tail terminator (§6.4)
    mutable prev: &- DListNode         // weak back-edge (§6.3)
}

// Initialization syntax for &- mirrors type syntax: uses operator &-.
// Maps 1:1 with declaration `prev: &- DListNode`.
public function example_link(current_tail: &0 DListNode, value: Integer) -> &+ DListNode {
    return DListNode {
        value: value,
        next: ~0,                      // no successor yet — null terminator
        prev: &- current_tail           // operator &- creates weak ref
    }
}
```

Note: `&- expr` in expression positions is the **constructor operator** for weak references, symmetric to `&- T` in type positions. There is no `weak()` function call — the `&-` symbol is a first-class language operator, consistent with explicit `&0 expr` borrows.

### 12.3 — Network packet (self-ref via capability)

```triet
// dao.package
capabilities {
    dev::self_ref: grant  // certifies intentional usage of offset pattern
}

// module sys::network
public struct NetworkPacket {
    buffer: &+ Vector<Tryte>,
    header_offset: Integer,
    payload_offset: Integer
}

public function get_header(packet: &0 NetworkPacket) -> &0 Header {
    return slice_at(packet.buffer, packet.header_offset)
}
```

### 12.4 — MMIO blink LED (capability gates kernel access)

```triet
// dao.package
capabilities {
    sys::io.memory: grant
}

// module sys::driver::led
public function blink(reg: &0 mutable HardwareRegister) {
    // sys::io.memory capability gates runtime check
    sys.io.memory.write(reg.address, 0xFF)
}
```

Note: §12.4 demonstrates ADR-0018 capability integration rather than ownership mechanics — ownership here is standard `&0 mutable HardwareRegister`. The example is retained to illustrate how ownership and capabilities combine.

---

## §13 — References

- [SPEC §10 — Memory model](../../SPEC.md) (rewritten in sync with ADR-0025 + ADR-0026)
- [VISION §3.5 — Capability + namespace](../../VISION.md)
- [VISION §5 — Triet Identity: ternary first-class](../../VISION.md)
- [VISION §6 — Refuse over guess](../../VISION.md)
- [ROADMAP §v0.8 — Concurrency Model](../../ROADMAP.md)
- [ADR-0001 — Nullable memory layout](0001-nullable-memory-layout.md) (`T?` discriminator reuse for `&- T` upgrade in §9)
- [ADR-0016 — Capability type system](0016-capability-type-system.md) (4-state levels for memory capabilities)
- [ADR-0017 — Trilean policy hook](0017-trilean-policy-hook.md) (capability resolver path for `dev::self_ref` etc.)
- [ADR-0018 — Capability loader semantics](0018-capability-loader-semantics.md) (`dao.package` declaration model)
- [ADR-0020 — Outcome error handling](0020-outcome-error-handling.md) (`T?` syntax used for `&-` upgrades)
- [ADR-0021 — Trilean refinement](0021-trilean-refinement.md) (refuse-with-fix-suggestions pattern for E2400 series)
- [ADR-0025 — Borrow Checker Rules](0025-borrow-checker-rules.md) (enforcement algorithm)
- [ADR-0026 — Actor Boundary Send Rules](0026-actor-boundary-send-rules.md) (concurrency interplay)
