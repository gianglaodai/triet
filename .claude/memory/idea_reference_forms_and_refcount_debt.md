---
name: idea_reference_forms_and_refcount_debt
description: "2026-08-25 rulings on the & family: &- is outward flow (Weak DELETED), &+ is still drifting in G's docs, and ADR-0026 §7 locks an atomic refcount on EVERY heap allocation — contradicting the 0-refcount claim"
metadata: 
  node_type: memory
  type: project
  originSessionId: f1274d46-292b-4dbb-b77f-aa8ea2d0f365
---

# Reference forms, and the refcount that is still there

Discussion-only session, Giang & O, 2026-08-25. No code, no ADR. Companion to
[[idea_ternary_placement_syntax]].

## 1. `&-` = OUTWARD FLOW. `Weak` is DELETED from the language. (agreed by Giang, G, and O)

Two rival meanings were in circulation:

- **A (schema `crates/triet-syntax/src/generated/types.rs:93-94` + `ADR-0022:62`):** `WeakObserver`,
  "must be upgraded to `&0` before reading (the upgrade is compile-time verified to ensure the owner
  still exists)", purpose = **break cycles**.
- **B (G, 2026-08-23 onward):** outward flow — returnable from a function, points up to a parent.
  Rust's `&'a` with an inferred region.

**B wins. A is deleted.** Three reasons:

1. **A's job no longer exists.** `Weak` exists because `Rc` cycles leak. No `Rc` ⇒ no `Rc` cycle ⇒
   nothing to break. Cyclic graphs are routed to Arena+NodeID by decree.
2. **A's own description is self-contradictory.** If the compiler *proves* the owner is alive, it is
   not weak — it is an ordinary borrow with a discharged outlives constraint. A genuine weak pointer
   is one whose validity is unknown until runtime, which requires a runtime check (control block or
   generation counter) — not zero-cost.
3. **The truly dynamic case already has a home:** Arena + generational index, where the generation
   counter does the control block's job next to the data, `u32`, cache-friendly. A second mechanism
   for the same job is blocked by Pillar ③.

**B is load-bearing, not cosmetic.** Without it: no zero-copy view returned from a function (must
clone, or return `(offset, len)` and reassemble), and no parent pointers in trees — AST parent links,
UI tree parents, DOM parents are all in the **90%** tree case, so losing them collapses the 90/10
split to roughly 60/40 and guts the Pit-of-Success argument.

With B the `&` family finally has one clean axis — **direction relative to the current frame**:
`&+` toward me (I hold it, I drop it) · `&0` stays here (borrow, may not escape) · `&-` away from me
(returns to caller / points up to parent). G's "Directional Flow" instinct was right; G's enforcement
rule was not (see §3).

**Debt created:** amend `docs/decisions/0022-trit-balanced-ownership.md:62` and the schema variant
`WeakObserver` (through `spec/schema/triet-schema.yaml`, never by hand-editing generated files). Five
forms remain five, but one form's meaning changes.

## 2. `&+` is STILL DRIFTING — caught three times in three messages

Giang's position, stated twice: **`&+` is the strong unique owner, unchanged.** Backed by
`docs/decisions/0022-trit-balanced-ownership.md:58` (LOCKED, `&+ T` ≈ `Box<T>`) and the generated
schema `crates/triet-syntax/src/generated/types.rs:82-86` (`StrongFrozen` / `StrongMutable`).

G's 2026-08-23 docs and follow-up messages repeatedly rewrite it as *"Positive / Universal read-only
shared view"*, *"tham chiếu bất tử / đọc toàn cục (.rodata / Static / Arena)"* — each time without
declaring an ADR amendment. **Verify-don't-trust applies to G.** Nail `&+` = owner before any further
document is written on top of the drifted meaning.

Why G kept reaching for it: G's model had **no pole expressing immortality**, so `&+` got bent to
cover the gap. **`-T` fills that slot properly** (`&0 -T` = immortal shared read), which makes the
drifted `&+` redundant. Keeping both = two names for one thing = Pillar ③ rejection.

Geometry note: G's `&+`/`&0`/`&-` ladder mixes axes — "immortal" is a claim about **time**, while
`&0`/`&-` are claims about **direction**. The three do not lie on one axis in G's version; they do in
Giang's.

## 3. G's flagship `&-` example is UNSOUND as written — and needs a rule nobody has written

```
struct AstNode {
    children: Vector<+AstNode>,
    parent: &- AstNode?,
}
```

G claims the compiler statically guarantees 100% that a child never outlives its parent, on the
grounds that the child sits inside the parent's vector. **The child can leave.**
`crates/triet-typecheck/src/env.rs:384` — `pop_front<T>(Vector<T>) -> T?` moves the first element
out, and `drain` landed too (Slices 2b/2d). So:

```
let c = tree.children.pop_front();   // child leaves the parent
// tree dies here
// c.parent now points into freed memory
```

Reachable with shipped features inside the 581-fixture gate, not hypothetical.

Worse: **an AST with parent pointers is the canonical case where Rust's lifetimes ALSO fail.** Nobody
writes `struct AstNode<'a> { parent: Option<&'a AstNode<'a>> }` in real Rust — it is self-referential,
`'a` binds to itself, and people switch to arenas + indices. G used the hardest case in Rust as proof
that **one polarity bit** solves it. One bit is not enough where a whole region lattice is not enough.

**The missing rule (invented this session, must be in the ADR):**

> A struct holding a `&-` back-pointer to its owner **may not be moved out of that owner.**

`+AstNode` inside `children` is pinned in place: no `pop_front`, no `drain`, no move-out for types
carrying a back-pointer. Rust needs `Pin` + `unsafe` to express this; Triết can make it a static
typecheck rule at zero runtime cost — but it is a **real user-facing restriction** and must be stated
plainly, not hidden behind "guaranteed 100% statically".

The rest of the example is correct and cheap: `&- AstNode?` for a root with no parent reuses the
ADR-0062 pointer sentinel, zero extra bytes.

## 4. The one-bit ceiling, and the lazy sound rule

```
function pick(a: &- String, b: &- String) -> &- String
```

One polarity bit cannot say whether the result borrows from `a` or `b`; this is precisely what Rust
needs `'a`/`'b` for. **Rule: a returned `&-` is bounded by the SHORTEST-lived `&-` argument.**
Conservative, always sound, loses precision only in rare cases, and costs zero lifetime syntax —
consistent with the existing `places_conflict(conservative)` discipline.

The defensible slogan is **"drop lifetime SYNTAX, keep lifetime ANALYSIS."** G's proposed enforcement
— compare declaration order in the source — drops the analysis too, and is unsound three ways:
a move can kill the parent early; the child can escape by return or container; source order does not
determine drop order for heap parents. Triết's existing NLL borrowck (`PROJECT_KNOWLEDGE:327`,
E2420/E2440/E2450 with 263/177/112 occurrences) is strictly stronger than that rule — do not
downgrade to it.

## 5. Can Triết drop `move` / `Send` / `Sync` / `Pin`? (Giang asked; G claimed yes to all)

| Keyword | Drop the keyword? | Concept survives? | Condition |
|---|---|---|---|
| `move` | Yes | **Yes, deeply** — move semantics is ADR-0042 Deinit tombstones + E2420 | only while escaping closures stay fenced by **E1122 EscapingClosureSealed** (`PROJECT_KNOWLEDGE:356`) |
| `Pin` | Yes | **Yes** — it becomes the §3 no-move-out rule for `&-` | only while `async`/`spawn` stay on the ADR-0026 v2 §6 refuse-list |
| `Sync` | **The bound: yes. The concept: NO — it does not collapse** | **Yes, with live members TODAY** | ⚠️ O's first answer ("nearly") was wrong — see **§6c**. `Atomic<T>` (ADR-0028, LOCKED) is sanctioned interior mutability, and ADR-0026:161 already writes `!Sync`. What saves the promise is that the type NAME carries the warning, not a bound |
| `Send` | **The bound: yes. The concept: no** | **It is runtime machinery here, not a marker** | drop `T: Send` from 99% of signatures (it is vacuously true); keep a `thread_bound` annotation for FFI handles — see **§6b**. Machinery blocked by ADR-0026 §3.2 + §7 |

`move` is a *closure capture* annotation in Rust, needed because Rust **infers** capture mode. In
Triết the capture mode is already written in the captured thing's reference form, so no annotation is
needed — but the reason it is currently moot is that escaping closures are fenced, not that the design
won.

`Pin` exists for `async` self-referential state machines. ADR-0022 D3 already bans self-referential
structs (escape hatch is offset-based, and offsets survive moves where pointers do not), and async is
refused. But note the concept comes straight back as the §3 immovability rule.

`Send`/`Sync` are pervasive in Rust because of `Rc` vs `Arc` and `Cell`/`RefCell`. Remove those and
the `T: Send + Sync` noise leaves 99% of signatures — but the concept survives as a **small closed
set** of FFI / thread-affine OS handles.

## 6. ⚠️ THE FINDING: Triết already has an atomic refcount on EVERY heap allocation

G has repeatedly claimed "0 Reference Counter", "XÓA SỔ". The locked record says otherwise —
`docs/decisions/0026-actor-boundary-send-rules.md:375`:

> **Lock:** Every heap allocation on a binary target contains an 8-byte `ObjectHeader`
> [`refcount: u32 | reserved: u32`]. The refcount is automatically atomically incremented/decremented
> at the `Send` boundary for `&+ T` (frozen).

and `:174` — the refcount increases **atomically** when a closure is `Send`.

So: **an 8-byte header on every heap object whether or not it ever crosses a thread** (only the
4-byte `refcount` half is the Send tax — the `reserved` half carries ADR-0077's Vector stride), and `Send` in
Triết is not a passive marker like Rust's — at `:174`/`§3.2` it *triggers atomic operations at
runtime*. Dropping `Send` from Triết is therefore a **bigger** change than dropping it from Rust,
not smaller.

### The prize

`ADR-0026:395`:

> Negative sentinels: `-1` = static, **`-2` = frozen forever**. Atomic ops check `current < 0` to
> **skip the refcount entirely**.

**ADR-0026 §7 already reserved a slot for "frozen forever" and already specified that it skips all
refcounting — it simply had no syntax to construct such an object.** `-T` is that syntax. Giang's
idea is not a foreign graft; it fills a slot a locked ADR left empty.

Consequence:

> If `-T` becomes the way to share immutable data across threads, nobody uses the `&+ T`-frozen +
> refcount path of ADR-0026 §3.2 any more — which is what would make the "0 refcount" claim *true*.
> Today it is false against the record.

### ⚠️ But NOT "8 bytes back" — O overclaimed, then measured (2026-08-25)

The header is real and live, not spec-only: `crates/triet-core/src/memory.rs:51`
`ObjectHeader { refcount: AtomicU32, reserved: AtomicU32 }`, written at
`crates/triet-jit/src/mir_lower.rs:5694` and `:6092`. **The `reserved` half is NOT spare — ADR-0077
stores the Vector `stride` in it** (`:6092`). So at most the 4-byte refcount could be reclaimed, and
with 8-byte alignment that is realistically **0 bytes** without redesigning the whole layout.

**And the hidden trade:** §3.2 buys a real feature — share ONE frozen object between two threads and
free it when the last one finishes. `-T` replaces it only by **never freeing**: zero-refcount sharing
paid for with permanent memory. Fine for a batch process, wrong for the long-running server (the same
LSP problem as [[idea_ternary_placement_syntax]]'s open questions). **Item M's job is to MEASURE, not
to promise.**

### 🎁 `-T`'s runtime is already built and waiting for a constructor

`crates/triet-core/src/memory.rs:91` — `new_frozen_forever()`, `FROZEN_FOREVER_SENTINEL` =
`u32::MAX - 1`; `increment` (`:99`) and `decrement` (`:109`) **already skip the atomic op** for it.
Measured: **zero callers outside its own unit tests** (`:200`, `:209`). The machinery for `-T` exists,
is tested, and is dead for want of syntax.

**Debt, ranked alongside `+T` itself: re-examine ADR-0026 §3.2 and §7 in the light of `-T`
(TODO.md item M).**

## 6b. `Send`'s user-facing surface → `thread_bound` (Giang ruled 2026-08-25)

For pure Triết data `Send` is **vacuously true** — move ⇒ exclusive (E2420 kills the sender's access),
`-T` ⇒ immutable, no `Rc`, no interior mutability ⇒ nothing can race. A property that is always true
carries no information, so the `T: Send` bound can leave 99% of signatures. What survives is only
**thread-affine OS/FFI handles** (GPU/UI context, a lock guard that must release on its own thread) —
physics and OS, not something a type system can design away.

⛔ **Do NOT spell it `!Send`.** Giang's ruling, and it is right: negating a concept the developer has
never been shown is worse than not naming it — `!Send` reads as "not-what?".

✅ **`thread_bound`**, an annotation on foreign type declarations (`extern type GpuContext:
thread_bound`), **not a core keyword**. The OS-binding author writes it once; an application developer
writes it zero times and reads it zero times. Chosen for being plain, like `function` / `mutable` /
`public`. Rejected: `rooted` / `captive` / `resident` (vague), `pinned` (Rust took it, and it means
"cannot change ADDRESS", a different axis).

**The developer's real contact surface is the DIAGNOSTIC, not the annotation** (per ADR-0027): the
error says "`GpuContext` cannot leave the thread that created it … declared `thread_bound` at
sys/gpu.tri:12", with no `Send` in the sentence. The concept is learned exactly when it matters and
never has to be carried around.

⚠️ **Living condition of the "developers never see it" promise** — stated correctly (O's first
wording was wrong, see §6c): it is **NOT** "no interior mutability" — Triết already has some. It is:

> **No HIDDEN interior mutability. The one sanctioned form must be visible in the TYPE NAME.**

`Atomic<Integer>`, `Channel<User>`, `Actor<T>` announce themselves — a developer reaching for one has
already declared they are doing concurrency, and the name is the warning. No bound needs carrying.
The promise dies the day a type is shared-and-mutable **without saying so in its name** (a lazily
initialised shared cache is the classic offender), or the day counted shared ownership returns.

## 6c. ⚠️ `Sync` does NOT collapse — O's "nearly" was hiding four real things

O first ranked `Sync` as "nearly droppable, needs interior mutability gone". Measured, that premise
was already false in the locked record. Giang's requirement stands and is met — **a developer must
never see `Sync`** — but the concept survives with live members.

1. **`Atomic<T>` exists and is LOCKED.** `docs/decisions/0028-atomic-primitive.md` (Status: Locked,
   v0.9.0.1): `fetch_bitwise_or(self: &+ Atomic<Integer>, mask: Integer, ordering: Ordering)` —
   mutation through a shareable handle, with an explicit `Ordering`. That **is** interior mutability,
   and `Integer` vs `Atomic<Integer>` differ in exactly what `Sync` encodes.
2. **`!Sync` is already written into ADR-0026** at `:161` — "Mirrors Rust `Send + !Sync` types" — with
   a category already attached to it.
3. **The concurrency primitives are themselves shared+mutable**: `Channel.bounded(16)` (`:304-309`),
   `std.concurrency.actor.Actor<T>` (`:354`), locks. BYKS puts them in stdlib (`:13`), but the claim
   "sharing this simultaneously is safe because it synchronises internally" is a **core-language
   safety rule** — `:13` keeps "universal primitives + compile-time safety rules" in the core, and
   stdlib cannot self-certify.
4. **Deleting the refcount deletes the memory fence** — see §6d.

**Verdict:** `Sync` shrinks to a small closed set — `Atomic<T>` · concurrency primitives · thread-affine
FFI handles — but it is load-bearing, not noise. If a spelling is ever needed, follow the
`thread_bound` naming law and name the positive property: **`self_synchronized`**, never `Sync`/`!Sync`.
Not decided yet; nothing forces the decision until threads land.

## 6d. Safe publication — zero refcount also means zero happens-before edge

`Arc` does not merely count. Its atomic operations **also create the happens-before edge**: a thread
that sees the pointer is guaranteed to see the field writes that built the object.

`-T` removes the counter and therefore removes that edge. On a weakly-ordered CPU (ARM), thread B can
observe the `-T` pointer **before** it observes the writes that constructed the object — reading
garbage out of something advertised as immutable and always-safe.

This is the classic **safe publication** problem. Java met it and solved it with **`final`-field freeze
semantics in the JMM**, precisely so immutable objects could be published without synchronisation —
prior art that *supports* the `-T` design, on one condition:

> `-T` needs **one release at the end of construction**, once per object — not per read.

Cheap, but it must be **designed, not assumed**. "Zero atomics" is true of every subsequent read and
**false at the moment of publication**.

### And the constraint that falls out of it

Aliasing-XOR-mutability is enforced by NLL borrowck over **one CFG, i.e. within one thread**. It does
not extend across a spawn boundary on its own: if A lends `&0 x` to B, A's borrowck cannot know when B
finishes, so it cannot know when taking `&0 mutable x` becomes legal again.

⇒ **A `&0 T` to a MORTAL object must not cross a spawn boundary**, unless a scoped-thread construct
makes the join point visible to borrowck. Rust fences this with `'static` bounds on spawned closures.
Without an equivalent rule, "move a unique owner" and "`&0 -T`" stop being the *only* two ways across,
and the whole vacuously-true-`Send` argument collapses with it.

## 7. Unmeasured numbers G put in writing (do not let them into documents)

"10x faster than Rc/Weak" · "clone makes the code 10x slower" · the fabricated error code **`E2401`**
(measured: E2400, E2402, E2403 exist; **2401 does not**).

Related: [[idea_ternary_placement_syntax]] · [[idea_post_rust_architecture_and_ternary_foundations]] ·
[[feedback_verify_producer_before_consumer]] · [[campaign_borrowck_nll_foundation]]
