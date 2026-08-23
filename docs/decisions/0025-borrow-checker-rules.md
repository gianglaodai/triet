# ADR 0025 — Borrow Checker Rules

**Status:** **Locked** (promoted via v0.8.x.review 2026-05-28). Sibling to [ADR-0022](0022-trit-balanced-ownership.md) + [ADR-0026](0026-actor-boundary-send-rules.md). v0.8 shipped skeleton diagnostics (E2400/E2402-E2403/E2410-E2411/E2420-E2422/E2430/E2440) per v0.8.10; full NLL enforcement deferred to v0.9 (requires real-world Triet corpus). Locks the compile-time enforcement algorithm for 5 reference forms from ADR-0022 §2. Defines new error code namespace **E24XX** for borrow-related diagnostics.

**Issue:** [ADR-0022](0022-trit-balanced-ownership.md) locks the conceptual model — 5 reference forms, mutability default, linear `&+`, capability-as-unsafe, no-cycle theorem. The remaining *algorithmic* decisions:

- **D4** — Borrow exclusivity using NLL (Non-Lexical Lifetime) or basic lexical?
- **D5** — When lifetime inference fails, allow explicit annotation `<'a>`?
- How to detect use-after-move (E2420, E2421).
- How to perform constructibility termination checks (E2422 from ADR-0022 §6.4).
- Mutability violation enforcement (E2410, E2411 from ADR-0022 §3.4).
- Drop order + custom destructor.
- Default inference algorithm in `usr::` vs `sys::/dev::`.

Author 2026-05-26 finalized D4 = NLL (smart enough to accept code rejected by Rust 2015), D5 = **no `<'a>` syntax** (compile error with refactor suggestion, worst case use capability post-v1.0).

This ADR locks the algorithm for each error category + implementation phasing. Each sub-decision has a dedicated error code in the E24XX namespace (E2400–E2499 reserved for borrow checker).

---

## §1 — Goals & Non-goals

### 1.1 — Goals

1. **Compile-time only** — the borrow checker emits no runtime checks.
2. **Zero-cost** — no changes to memory layout, no insertion of guard code.
3. **90% common patterns work** without explicit annotation.
4. **10% edge cases** $\rightarrow$ compile error with 2-3 concrete refactor suggestions.
5. **Error messages** use the E2400 pattern with a "Suggested fix" block (style finalized via [ADR-0021 §10](0021-trilean-refinement.md)).

### 1.2 — Non-goals (defer post-v1.0)

- **Polonius-level permissive analysis** — Rust's next-gen borrow checker. Triet v1.0 stops at NLL.
- **Two-phase borrows** — a Rust feature allowing certain mutable+immutable overlaps. Deferred until evidence of necessity arises.
- **Generic lifetime variance** — since there is no annotation syntax, there is no variance.
- **Self-referential async future** — deferred until the async/await design is locked (post-v0.8).

### 1.3 — Error code namespace E24XX

Reserved range **E2400–E2499** for the borrow checker. Allocation:

| Range | Category |
|---|---|
| E2400–E2409 | Lifetime inference & elision |
| E2410–E2419 | Mutability violations |
| E2420–E2429 | Move semantics & use-after-move |
| E2430–E2439 | Namespace inference violations |
| E2440–E2449 | Borrow exclusivity (NLL) |
| E2450–E2459 | Reserved (drop order, custom drop) |
| E2460–E2499 | Reserved for future expansion |

Module path: `triet::borrow::E24XX`. CLAUDE.md to be updated when this ADR lands.

### 1.4 — Error message format

All diagnostics in the E24XX namespace (§2-§10) follow the canonical format finalized in [ADR-0027 — Diagnostic Format Standard](0027-diagnostic-format-standard.md). This format applies language-wide, not just to E24XX.

Summary: header `EXXXX ErrorName` + body (1-3 sentences) + optional span block (`--> file:line:col` + caret) + optional `[Fix N]` numbered fix blocks using imperative `Change/Wrap/Use/Add/Replace/Move X to Y`. Pure ASCII, no `-/+` diffs. Detailed spec + rationale: ADR-0027 §2.

---

## §2 — Borrow Exclusivity with NLL (D4)

**Decision:** At any program point, for the same object (place), a borrow must satisfy **one of two states**:

- **State A:** Exactly 1 `&0 mutable T` is active. No `&0 T` is active simultaneously.
- **State B:** N $\ge$ 0 `&0 T` are active. No `&0 mutable T` exists.

Prohibited: 2 `&0 mutable` for the same object; or `&0 mutable` + `&0` for the same object.

### 2.1 — "Active" per NLL is not lexical

NLL: a borrow is only "active" from its creation point to its **last use**, not until the end of the block. This allows many patterns rejected by Rust 2015:

```triet
let mutable v = Vector { 1, 2, 3 }
let r1: &0 Vector = &0 v        // borrow start
print(r1.length)                  // last use of r1
v.push(4)                         // OK with NLL — r1 is no longer active from the line above
```

Rust 2015 lexical would reject this because `r1` is "still in scope." NLL accepts it because the last use has passed.

### 2.2 — E2440 BorrowExclusivityViolation

```text
E2440 BorrowExclusivityViolation
    Cannot create `&0 mutable` while `&0` is active on the same place `v`.
    Borrow `r1` is still live at the mutation point because of a later read.
    
    --> src/example.tri:11:5
       |
    10 |     let r1: &0 Vector = &0 v
       |                         ----- &0 borrow created here
    11 |     v.push(4)
       |     ^^^^^^^^^ &0 mutable creation conflicts
    12 |     print(r1.length)
       |           -------- &0 still used here (extends live range above)
    
    Suggested fixes:
    
    [Fix 1] Reorder the read before the mutation (shrinks live range of r1):
    Move `print(r1.length)` to immediately before `v.push(4)`
    
    [Fix 2] Copy the borrowed value out before mutating:
    Replace line 10 with `let len = v.length` and remove the `r1` binding
    
    [Fix 3] Restructure to avoid simultaneous read+write:
    Wrap the mutation behind a method on the owner struct that controls borrow scope internally
```

### 2.3 — Algorithm: live-range analysis on CFG

The compiler builds a control-flow graph (CFG) and assigns each borrow a "live range" from its creation point to its last-use point. Two borrows conflict when their live ranges intersect **and** violate exclusivity. The algorithm has a complexity of O(N·M) where N = borrow count and M = CFG size — practical for realistic codebases.

Implementation phase v0.10 (details in §12).

---

## §3 — Lifetime Elision (3 rules)

**Decision:** The compiler infers borrow scope automatically via 3 sequential rules. If all 3 fail $\rightarrow$ E2400.

### 3.1 — Rule 1: 1 input borrow $\rightarrow$ output ties to input

```triet
function first_word(s: &0 String) -> &0 String {
    // Compiler automatically infers: return scope = `s` scope
}
```

The function has **exactly 1** borrow input (`&0`, `&0 mutable`, or `&-`) and returns a borrow. Output borrow scope = input borrow scope.

### 3.2 — Rule 2: Method with self $\rightarrow$ output ties to self

```triet
public struct Cache {
    public function get(self: &0 Cache, key: String) -> &0 Entry {
        // Compiler automatically infers: return scope = self scope
    }
}
```

A method with receiver `&0 self` or `&0 mutable self` (regardless of other borrow inputs). Output borrow scope = self scope.

### 3.3 — Rule 3: Owned return (no inference needed)

```triet
function parse(s: &0 String) -> &+ ParsedDoc {
    // Return owned — no lifetime relationship needs to be inferred
}
```

The output is `&+ T` (owned). There is no lifetime relationship — the function transfers ownership out. No elision required.

### 3 $\rightarrow$ 3.4 — When all 3 rules fail $\rightarrow$ E2400

```triet
function pick_longer(a: &0 String, b: &0 String) -> &0 String {
    if a.length > b.length { return a } else { return b }
}
```

The function has 2 borrow inputs, is not a method, and returns a borrow. The compiler does not know if the output ties to `a` or `b`.

```text
E2400 BorrowLifetimeInferenceFailed
    Cannot infer which input the returned borrow ties to.
    Function has 2 input borrows: `a: &0 String`, `b: &0 String`.
    
    --> src/example.tri:1:62
       |
    1  | function pick_longer(a: &0 String, b: &0 String) -> &0 String {
       |                                                     ^^^^^^^^^ ambiguous return borrow
    
    Suggested fixes:
    
    [Fix 1] Return owned value instead (requires cloning inside body):
    Change `-> &0 String` to `-> &+ String`
    
    [Fix 2] Group inputs into a collection with a single borrow scope:
    Refactor parameter list: change `(a: &0 String, b: &0 String)` to `(items: &0 Vector<String>)`
    
    [Fix 3] Encapsulate inside a struct method (ties return to `self`):
    Wrap logic in `impl StringPair { function longer(self: &0 StringPair) -> &0 String { ... } }`
```

---

## §4 — No Annotation Policy (D5)

**Decision:** Triet **does not have** `<'a>` syntax or its equivalent. When elision fails, the compiler decisively refuses. The author must refactor according to one of the 3 suggestions in E2400.

### 4.1 — Reasons for refusing annotation

1. **Brand fit:** Annotation virality is the number one reason developers leave Rust according to surveys. Triet aims for "Rust rigor + better ergonomics."
2. **AI-friendly:** Annotations require global reasoning about lifetimes — difficult for LLMs to generate correctly. Refuse-with-refactor provides a clearer goal for LLMs.
3. **Long-term simplicity:** Every feature the compiler lacks is a language feature that does not need to be learned, documented, or versioned.

### 4.2 — The 5% case where refactoring is impossible

Some API designs truly require multi-input borrows tied to different lifetimes (rare in practice). Triet policy:

- **v0.8–v1.0:** Complete refusal — the author must refactor.
- **post-v1.0:** Re-evaluate. If concrete evidence arises from a self-hosting compiler or kernel work, open the `dev::explicit_region` capability for this pattern. The capability gate keeps it audit-friendly.

Author 2026-05-26 finalized: prioritize a **clean brand** for v1.0. Worst case: 1-2% of the codebase must refactor — this is acceptable.

---

## §5 — Use-After-Move (E2420, E2421)

**Decision:** Move semantics per ADR-0022 §4.1. The compiler tracks each `&+` binding via dataflow analysis. Access after a move $\rightarrow$ E2420. Attempting self-ownership $\rightarrow$ E2421.

### 5.1 — E2420 UseAfterMove

```triet
let alice: &+ User = create_user("Alice")
take(alice)                  // ownership moves into take()
print(alice.name)            // E2420
```

```text
E2420 UseAfterMove
    Cannot access `alice` after ownership was moved.
    Binding `alice` was consumed by `take()` on line 4.
    
    --> src/example.tri:5:11
       |
    4  |     take(alice)
       |          ----- ownership moved here
    5  |     print(alloc_name)
       |           ^^^^^ used after move
    
    Suggested fixes:
    
    [Fix 1] Borrow instead of move (keeps `alice` usable after the call):
    Change `take(alice)` to `take(&0 alice)` if `take` accepts `&0 User`
    
    [Fix 2] Restructure so the value is only consumed once:
    Move `print(alice.name)` to before `take(alice)`
    
    [Fix 3] Clone before move (only if `User` opts into clone semantics):
    Change `take(alice)` to `take(alice.clone())`
```

### 5.2 — E2421 SelfOwnershipParadox

Specifically for cases attempting to move a variable into a field of itself (see proof in ADR-0022 §6.2):

```triet
a.b_field.a_field = a       // E2421
```

```text
E2421 SelfOwnershipParadance
    Cannot move `a` into a field reachable from `a` itself.
    Linear ownership requires the source and destination to be distinct.
    This is one of the patterns prevented by the no-cycle theorem (ADR-0022 §6).
    
    --> src/example.tri:7:5
       |
    7  |     a.b_field.a_field = a
       |     ^^^^^^^^^^^^^^^^^^^^^ ownership cycle attempt
    
    Suggested fixes:
    
    [Fix 1] Use a weak back-edge (recommended for parent-child or graph back-edges):
    Refactor `a_field` to a weak observer: change `a_field: &+ A` to `a_field: &- A`, then write the assignment as `a.b_field.a_field = &- a`
    
    [Fix 2] Restructure so the back-edge is not owning:
    Replace the back-link with an index or ID that resolves through a registry, removing the reference entirely
```

### 5.3 — Algorithm: move-state tracking

The compiler maintains a 3-state for each `&+` binding:

- **Owned** — binding is valid, access is permitted.
- **Moved** — ownership has been transferred; access $\rightarrow$ E2420.
- **Conditionally moved** — moved on one branch, owned on another (after if/match) $\rightarrow$ merge logic.

Conditional moves are typically resolved by the compiler inserting "drop flags" or forcing re-assignment before the use point. Detailed implementation deferred to the implementation phase.

---

## §6 — Constructibility Termination (E2422)

**Decision:** Per ADR-0022 §6.4, every struct with a recursive `&+ T` reference (T = Self directly or via a chain) must have a **base case** for the constructor to terminate. The compiler checks locally; no global SCC is required.

### 6.1 — Algorithm

For a struct `S` with field `f: F`:

1. Compute the *reaches-self* set: the set of types T that can "reach" S from F through an `&+` ownership chain.
2. If S $\in$ reaches-self(F) $\rightarrow$ S is self-recursive via F.
3. If self-recursive, F must be a **terminable type**: `(&+ T)?`, or `Vector<&+ T>` / `Map<K, &+ T>` / a collection with an empty state, or `&- T` (weak, non-counting).
4. Otherwise $\rightarrow$ E2422.

### 6.2 — E2422 NonTerminatingConstruction

```triet
public struct Node {
    value: Integer,
    next: &+ Node               // E2422 — not terminable
}
```

```text
E2422 NonTerminatingConstruction
    Struct `Node` has recursive ownership through field `next: &+ Node`,
    but the field is not terminable. The constructor would require an
    infinite chain of pre-existing `Node` instances.
    
    --> src/example.tri:3:5
       |
    3  |     next: &+ Node
       |     ^^^^^^^^^^^^^ recursive field has no base case
    
    Suggested fixes:
    
    [Fix 1] Make the field nullable so the chain can terminate with `~0` (most common):
    Change `next: &+ Node` to `next: (&+ Node)?`
    
    [Fix 2] Use a collection that terminates naturally with the empty state:
    Change `next: &+ Node` to `children: Vector<&+ Node>`
    
    [Fix 3] Use a weak reference if this is not the owning chain:
    Change `next: &+ Node` to `next: &- Node`
```

### 6.3 — Not a cycle check

E2422 is a **local property check** (one field, one struct definition at a time). It does not build a type graph or use SCC. It is O(N) relative to the number of fields, not O(N²) like cycle detection.

This is where Triet differs from Rust: Rust checks size finiteness via the `Sized` trait + bounds. Triet checks **constructibility** directly.

---

 $\text{7. Mutability Enforcement (E2410, E2411)}$

**Decision:** ADR-0022 §3.4 declares "frozen owners cannot be promoted." The compiler enforces this via 2 error codes.

### 7.1 — E2410 CannotMutateFrozenOwner

```triet
let owner: &+ User = create_user("Alice")     // frozen owner
owner.name = "Bob"                              // E2410
```

```text
E2410 CannotMutateFrozenOwner
    Cannot mutate field `name` of frozen owner `owner: &+ User`.
    Frozen owners are read-only for their entire lifetime (ADR-0022 §3.4).
    
    --> src/example.tri:2:5
       |
    1  |     let owner: &+ User = create_user("Alice")
       |                ------- frozen owner declared here
    2  |     owner.name = " $\text{Bob}$"
       |     ^^^^^^^^^^^^^^^^^^ mutation through frozen reference
    
    Suggested fixes:
    
    [Fix 1] Declare the owner as mutable at construction site:
    Change `let owner: &+ User` to `let owner: &+ mutable User`
    
    [Fix 2] Construct a fresh owner with the new value (functional style):
    Replace `owner.name = "Bob"` with a new binding that constructs a fresh `User` with all fields copied explicitly and `name` set to `"Bob"`
```

### 7.2 — E2411 CannotPromoteFrozenToMutable

```triet
let frozen: &+ User = create_user("Alice")
let mutable_handle: &+ mutable User = frozen      // E2411
```

```text
E2411 CannotPromoteFrozenToMutable
    Cannot promote `&+ User` (frozen owner) to `&+ mutable User`.
    Frozen ownership is permanent — promotion would break the
    "safe to share across actor boundary" invariant from ADR-0026 §3.
    
    --> src/example.tri:2:46
       |
    1  |     let frozen: &+ User = create_user("Alice")
       |                 ------- declared frozen here
    2  |     let mutable_handle: &+ mutable User = frozen
       |                                           ^^^^^^ frozen-to-mutable promotion
    
    Suggested fixes:
    
    [Fix 1] Declare as mutable at construction, derive frozen view only when sharing:
    Replace line 1 with `let frozen: &+ mutable User = create_user("Alice")` and remove line 2
    
    [Fix 2] Keep frozen ownership and construct a fresh mutable owner with fields copied explicitly:
    Replace line 2 with a new binding that constructs `&+ mutable User` by reading each field from `frozen`
```

### 7.3 — Field-level mutability granularity

```triet
public struct User {
    id: UserId,                  // immutable field (set once)
    mutable display_name: String  // mutable field
}

let mutable u: &+ mutable User = User { id: ..., display $\text{name}$: "Alice" }
u.display_name = "Bob"            // OK — field is mutable, owner is mutable
u.id = NewId                      // E2410 — field is immutable regardless of owner
```

Field mutability is **independent** of owner mutability. A mutable owner allows general mutation; a field requires the `mutable` keyword if it is to be modifiable.

---

## §8 — `&-` Upgrade & Scope Rules

**Decision:** ADR-0022 §9 declares `.upgrade()` returns `T?`. Compile-time invariant: `&-` only exists when tracing back to $\ge$ 1 live `&+`. Violation $\rightarrow$ E2403.

### 8.1 — E2402 BorrowInStructField (from ADR-0022 §7.1)

```triet
public struct BadIdea {
    cursor: &0 Tryte              // E2402
}
```

```text
E2402 BorrowInStructField
    Field `cursor: &0 Tryte` cannot be stored in struct `BadIdea`.
    Scope borrows (&0) are bound to the calling scope and cannot
    persist as struct fields (ADR-0022 §7.1).
    
    --> src/example.tri:2:5
       |
    1  | public struct BadIdea {
    2  |     cursor: &0 Tryte
       |     ^^^^^^^^^^^^^^^^ scope borrow stored as field
    3  | }
    
    Suggested fixes:
    
    [Fix 1] Use an owned reference (struct takes ownership of the byte):
    Change `cursor: &0 Tryte` to `cursor: &+ Tryte`
    
    [Fix 2] Use a weak reference (observer pattern, ownership stays elsewhere):
    Change `cursor: &0 Tryte` to `cursor: &- Tryint`
    
    [Fix 3] Use an offset-based pattern under `dev::self_ref` capability (ADR-0022 §7.2):
    Change `cursor: &0 Tryte` to `cursor_offset: Integer` and declare `dev::self_ref: grant` in dao.package
```

### 8.2 — E2403 WeakRefOutlivesOwner

```triet
function escape() -> &- Process {
    let p: &+ Process = create_process()
    return &- p                   // E2403 — weak ref outlives the &+ owner
}
```

```text
E2403 WeakRefOutlivesOwner
    Weak reference `&- p` cannot escape the scope where owner `&+ p` lives.
    After this function returns, `p` drops and the weak ref dangles.
    
    --> src/example.tri:3:12
       |
    2  |     let p: &+ Process = create_process()
       |            ----------- owner created in local scope
    3  |     return &- p
       |            ^^^^ weak ref escapes the owner's scope
    
    Suggested fixes:
    
    [Fix 1] Return the owner instead so the caller decides lifetime:
    Change `return &- p` to `return p` and change return type to `&+ Process`
    
 $\text{Fix 2]}$ Accept a long-lived owner from the caller:
    Refactor function signature: add a parameter `owner: &0 Process` and derive `&- owner` inside the body
    
    [Fix 3] Restructure to store the weak ref in a long-lived struct owned by the caller:
    Pass a `&0 mutable Registry` parameter and insert the weak ref into it instead of returning
```

### 8.3 — Algorithm: trace weak to owner

The compiler maintains an "owner trail" for each `&-` — a chain tracing back to the root `&+`. When a `&-` is assigned, stored, or returned, the compiler checks if the owner trail remains valid in the destination scope. Implementation phase v0.10.

### 8.4 — Upgrade pattern

```triet
let weak: &- Process = ...
let result: Process? = weak.upgrade()

match result {
    ~+ proc => use_process(proc),
    ~0      => log("target dropped or never set")
}
```

`.upgrade()` is a built-in method of the type `&- T`. It returns `T?` per ADR-0022 §9. It forces a `match` — there is no silent dereference to `T` directly.

---

## §9 — Default Inference per Namespace

**Decision:** ADR-0022 §5 declares different inference rules between `usr::` and `sys::/dev::`. Detailed algorithm:

### 9.1 — Algorithm

For every location where a reference type can be implicitly declared (struct field, function param, return), the compiler:

1. Checks the namespace of the module containing the declaration.
2. If `usr::` $\rightarrow$ apply default inference (see 9.2).
3. If `tsys::*` or `dev::*` $\rightarrow$ requires explicit `&+`/`&0`/`&-` immediately. No inference.
4. If the namespace rule is violated $\rightarrow$ E2430.

### 9.2 — Default inference table (usr::)

| Location | Type declared | Inferred reference |
|---|---|---|
| Struct field | `field: T` (T is a heap type) | `&+ T` (owned, immutable) |
| Struct field | `field: T` (T is a value type — primitive, tuple) | `T` (no ref) |
| Function param | `param: T` (T is a heap type) | `&0 T` (borrow, read-only) |
| Function param | `param: T` (T is a value type) | `T` (no ref) |
| Function return | inferred from body | Owned `&+ T` if body constructs, borrow if body returns input |
| `let x = expr` | inferred from expr | Matches expr type |

### 9.3 — E2430 ImplicitRefInSystemNamespace

```triet
module sys.kernel.scheduler

public struct Process {
    state: ProcessState,           // E2430 — implicit &+ in sys:: namespace
}
```

```text
E2430 ImplicitRefInSystemNamespace
    Field `state: ProcessState` requires explicit reference form in
    `sys::kernel::scheduler`. Implicit inference is disabled in
    `sys::*` and `dev::*` namespaces (ADR-0022 §5.2).
    
    --> src/sys/kernel/scheduler.tri:4:5
       |
    3  | public struct Process {
    4  |     state: Process $\text{ProcessState}$
       |     ^^^^^^^^^^^^^^^^^^^ missing explicit reference form
    5  | }
    
    Suggested fixes:
    
    [Fix 1] Declare owned immutable reference (most common for kernel state):
    Change `state: ProcessState` to `state: &+ ProcessState`
    
    [Fix 2] Declare owned mutable reference (when scheduler updates state):
    Change `state: ProcessState` to `state: &+ mutable ProcessState`
    
    [Fix 3] Keep as value type (only when `ProcessState` is primitive or tuple):
    Verify `ProcessState` is a primitive, tuple, or one of the stack-allocatable types from SPEC §10.3; if so, leave declaration as-is. If it is a heap struct, use [Fix 1] or [Fix 2] instead.
```

### 9.4 — Sys/dev namespace inference exception

The `dev::ergonomic_inference` capability (TBD post-v1.0) may restore inference for the `dev::` namespace if a developer contributor requires it. Default is OFF.

---

## §10 — Drop Order & Custom Drop

**Decision:** Fields drop in **reverse declaration order**. Custom destructor logic requires the `dev::custom_drop` capability.

### 1 $\rightarrow$ 10.1 — Default drop order

```triet
public struct Connection {
    socket: &+ Socket,           // dropped THIRD (last declared first)
    buffer: &+ Buffer,           // dropped SECOND
    log_handle: &+ LogHandle     // dropped FIRST
}
```

Reason for reverse order: resources are often dependent on earlier resources (socket depends on buffer setup, log_handle is independent). Dropping in dependency order minimizes "dropping A while A's resource depends on B that has already dropped."

### 10.2 — Capability `dev::custom_drop`

```triet
// dao.package
capabilities {
    dev::custom_drop: grant
}

// module dev::driver::pci
public struct PciDevice {
    handle: &+ DeviceHandle,
    
    public function on_drop(self: &+ mutable PciDevice) {
        // Custom destructor — sync flush, IRQ disable, etc.
        // Called automatically when self goes out of scope.
    }
}
```

Restrictions on custom drop:
- Cannot access fields already dropped by the default order.
- Cannot move `self`.
- Cannot panic / return an error (use Outcome ADR-0020 internally, log at the boundary).

Detailed E2450 + E2451 (custom drop violations) deferred to the implementation phase.

### 10.3 — Interaction with move

```triet
let conn: &+ Connection = open_connection()
take(conn)                       // moved; on_drop runs INSIDE take() when conn goes out of scope there
print(conn.socket)               // E2420 — not E2410 — indicates "moved" not "frozen"
```

A move transfers drop responsibility along with the value. Custom drop runs in the final scope that owns the value.

---

## §11 — Worked Examples

### 11.1 — 90% case: simple borrow (elision rule 1)

```triet
function uppercase_first(s: &0 String) -> &0 String {
    return s.to_uppercase().first_word()   // Compiler infers return ties to `s`
}
```

✅ Works without annotation. Elision rule 1 applies.

### 11.2 — 90% case: method (elision rule 2)

```triet
public struct Lexer {
    source: String,
    cursor: Integer,
    
    public function peek(self: &0 Lexer) -> &0 Token {
        // Return ties to self
    }
}
```

✅ Works. Elision rule 2.

### 11.3 — Edge case (5%): multi-input $\rightarrow$ E2400

```triet
function pick_longer(a: &0 String, b: &0 String) -> &0 String { /* ... */ }
// E2400 — refactor needed
```

The author refactors using suggestion (a) to return owned, or (b) to wrap inputs.

### 11.4 — NLL accepts what Rust 2015 rejected

```triet
let mutable v = make_vector()
let r = &0 v
let len = r.length          // last use of r
v.push(x)                    // OK with NLL
```

✅ NLL accepts. Lexical rejects. Triet = NLL.

### 11.5 — Borrow exclusivity violation $\rightarrow$ E2440

```triet
let mutable v = make_vector()
let r1 = &0 v
let r2 = &0 mutable v        // E2440 — conflicts with r1 still being active
print(r1.length)
```

### 11.6 — Move semantics $\rightarrow$ E2420

```triet
let alice: &+ User = create_user("Alice")
take(alice)                  // moved
alice.name                   // E $\text{2420}$
```

### 11.7 — Cycle attempt $\rightarrow$ E2421

```triet
let a: &+ A = ...
let b: &+ B = ...
a.b_field = b                // OK — first move
b.a_field = a                // E2420 — b is moved
// Or: a.b_field.a_field = a // E2421 — self-ownership paradox
```

### 11.8 — Constructibility termination $\rightarrow$ E2422

```triet
public struct Node {
    next: &+ Node           // E2422 — refactor to (&+ Node)? or Vector
}
```

### 11.9 — Mutability violation $\rightarrow$ E2410

```triet
let owner: &+ User = create_user("Alice")
owner.name = "Bob"          // E2410 — frozen
```

### 11.10 — Self-ref capability case (5%)

```triet
// dao.package
capabilities { dev::self_ref: grant }

public struct NetworkPacket {
    buffer: &+ Vector<Tryte>,
    header_offset: Integer       // offset-based, not real &0
}
```

✅ Capability gate documents intent. The pattern is offset-based, with no actual `&0` stored.

---

## §12 — Implementation Phasing

| Version | Scope |
|---|---|
| **v0.8** | Parser tokens `&+`, `&0`, `&-`, `mutable`. AST nodes. No enforcement. Examples typecheck with a relaxed type system. |
| **v0.9** | Simple enforcement: E2420 use-after-move (linear `&+` tracking). E2422 constructibility termination. E2410/E2411 mutability frozen. E2430 namespace inference. |
| **v0.10** | NLL borrow exclusivity (E2440). Lifetime elision 3 rules (E2400). E2402 borrow in struct field. |
| **v0.11** | `&-` upgrade tracking (E2403). Default inference per namespace fully working. Drop order. |
| **v1.0** | Capability `dev::custom_drop` (E2450, E2451). All E2400–E2459 stable. Self-hosting compiler uses borrow check. |
| **post-v1.0** | Evaluate `dev::explicit_region` need. Evaluate Polonius adoption. |

The self-hosting compiler bootstrap chain (v0.7) does not yet expose references in the stdlib — therefore, the sub-tasks for v0.8+ to port the lexer/parser/typecheck to "Triet-in-Triet" will be the **first real clients** of the borrow checker. The bootstrap loop acts as the functional gate for each version.

---

 $\text{13. Out of Scope}$

- **Polonius adoption** — post-v1.0, evaluate based on evidence.
- **Two-phase borrows** — defer until concrete patterns surface.
- **Generic lifetime variance** — no annotation $\rightarrow$ no variance.
- **Async/await self-borrow** — defer to the concurrency runtime ADR (post-v0.8).
- **Trait object lifetimes** — defer to the dynamic dispatch ADR.
- **Closure capture rules** — defer to the closure ADR (planned post-v0.8).
- **FFI memory ownership** — defer to the FFI ADR.
- **Reborrow patterns** (`&0 mutable T` $\rightarrow$ `&0 T` temporary downgrade) — implementation detail, to be decided when building §2.3 live-range analysis.

---

## §14 — References

- [ADR-0022 — Trit-Balanced Ownership](0022-trit-balanced-ownership.md) (parent — locks 5 reference forms, this ADR enforces them)
- [ADR-0026 — Actor Boundary & Send Rules](0026-actor-boundary-send-rules.md) (sibling, TODO — Send derivation depends on §7 frozen invariant)
- [ADR-0001 — Nullable memory layout](0001-nullable-memory-layout.md) (`T?` reuse for `.upgrade()` return)
- [ADR-0018 — Capability loader semantics](0018-capability-loader-semantics.md) (capability declaration model)
- [ADR-0020 — Outcome error handling](0020-outcome-error-handling.md) (`T?` 3-state semantic)
- [ADR-0021 — Trilean refinement](0021-trilean-refinement.md) (error message + refactor suggestion pattern reuse for E2400 series)
- [SPEC §10 — Memory model](../../SPEC.md) (will be rewritten in sync when ADR-0022 + ADR-0025 + ADR-0026 all land)
- [ROADMAP §v0.8](../../ROADMAP.md) (concurrency phase — depends on this ADR landing first)
- [CLAUDE.md — Error code namespace](../../CLAUDE.md) (update E24XX `triet::borrow::*` when ADR lands)
