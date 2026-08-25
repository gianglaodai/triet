---
name: campaign_placement_polarity_adr
description: "🔴 IN PROGRESS — DESIGN ONLY, no ADR written, no code. Balanced-ternary memory placement (+T/T/-T) replacing Box/Rc + directional reference forms (&0/&-). G BLOCKED with 6 conditions B1-B6. Author's directive authorises reversing ADR-0022 §2, §4.2, §5.1, §6.3, §9 and SPEC §10.1/§10.3. READ THIS before touching placement, -T/immortal, reference forms, is_copy, or the global-constant question."
metadata:
  node_type: memory
  type: project
  originSessionId: d7c43550-45a8-46b9-b24f-057305091954
---

# Placement Polarity `+T`/`T`/`-T` — design campaign (2026-08-25)

> **STATUS: 🔴 DESIGN PHASE. Nothing is committed. No ADR exists. No code was written.**
> Three rounds of O ↔ G architecture review completed. G's verdict: **BLOCKED, conditions B1–B6.**
> Tree at pause: `M .claude/settings.json`, `?? examples/doubly_linked_list.tri` (do NOT commit the example — see below).
> Gate baseline (G, round 1): `0 · clean · 0 · 581 · 0`.

## The Author's directive that opened this (verbatim intent)

1. **Every prior decision and every landed implementation may be reversed.** Old ADRs must NOT constrain this. ADR-0022 §2 (five locked reference forms) and SPEC §10.1 are explicitly in scope.
2. Goal: `Box<>`, `Rc<>`, `Send`, `Sync`, `Pin`, `move` **disappear from the programmer's eyes — permanently**. They need NOT be eliminated as mechanisms; the language default makes them invisible.
3. `T` and `+T` are different types, exactly as `T` and `Box<T>` in Rust. **"We do not want a zoo, but we must not escape it at any cost — we must make it ELEGANT."**
4. **Multithreading is entirely out of scope.** Re-evaluate after this lands.

## LOCKED (Author decided, or G ruled and Author did not object)

| # | Decision |
|---|---|
| **L1** | **One-line rule: `&` present = borrow. `&` absent = own. The placement sigil (`+`/`-`/none) is orthogonal — it says only where the bytes live.** ADR-0022's 5-form table conflated the ownership axis with the placement axis; this untangles them. |
| **L2** | **`&0 T` is a PLACEMENT ERASER.** A borrow of stack `T`, heap `+T`, static `-T` is the same 8-byte address ⇒ ~90% of signatures (reads) are placement-blind for free. The zoo appears only in signatures that take ownership, where explicitness is correct. **Erasure is DEPTH-1 ONLY** — `Vector<+Point>` ≠ `Vector<Point>` (different strides). |
| **L3** | **Q1 — a bare parameter `f(s: String)` is OWNED.** NOT a reversal: probe P2 shows `Drop(_0)` inside the callee and `E2420 use after move` at the caller. **Bare parameters have been owned for the entire life of the rewrite; SPEC §10.3 and ADR-0022 §5.1 are documentation that never matched the compiler.** Zero migration cost. |
| **L4** | **Q2 — call-site auto-decay to `&0`.** Already Locked by **ADR-0022:169 §4.2** ("The compiler automatically borrows `&+` into `&0`… No explicit `&` operator is required"), merely unimplemented (fixture 582:29 still writes `&0` by hand). **G's rule, final wording:** *"Coercion happens against a DECLARED target type, never an inferred one. It may only ERASE placement into a borrow. It may NEVER INTRODUCE placement. `+` is a placement on every type; `-` is a placement only where there is drop glue to suppress, elsewhere it is a rendering."* **A user-extensible `Deref` trait is PERMANENTLY BARRED.** Memorable form: **borrowing is implicit; allocating never is.** |
| **L5** | **Q3 — `&+` / `&+ mutable` deleted from the surface syntax**, IR variant kept. `&+`'s job (ownership transfer at a parameter) IS the visible `move` we are hiding; under L3 a bare `T` already says it. Frees the `&+` sigil for the deferred concurrency question. |
| **L6** | **`&-` = signature positions ONLY** (parameters, return types). **May NOT be a struct field.** Back-edges in hierarchical/cyclic structures are **arena indices**. This SUPERSEDES ADR-0022 §6.3's "structural law". Rationale: `&-`-as-field needs a runtime validity flag (= `Rc`/`Weak` through the back door); O's alternative (forbid moving out a field something back-links to) is a **whole-program aliasing property** and unimplementable. |
| **L7** | **No `+T{}` / `-T{}` expression-position constructors.** Placement sigils live in **TYPE POSITION ONLY**. Placement comes from the destination's declared type (ADR-0072 expected-type propagation, already landed) ⇒ `let n: +Node = Node{…}` is the ONLY place an allocation occurs, visibly marked on the same line ⇒ Pillar ② ("0 hidden allocations") holds absolutely. Also kills the `-Point{}` vs unary-minus ambiguity by deletion rather than by a parser rule. |
| **L8** | **`match` on `+Enum` REFUSED in v1.** ADR-0070's move-state paths (`borrowck/checker.rs:158+`) are keyed on `Local` + field path with **no `Deref` step**; extending through a deref is an unbudgeted foundation change. Costs nothing today. |
| **L9** | **H3 — `+T` gets a NEW `MirType` variant, polarity-carrying.** The Tuple precedent does NOT transfer: Tuple was an intermediate nobody keeps (dies at lowering); `+T` is a terminal representation that must reach the backend. See "The `is_copy` problem" below for the decisive measurement. |
| **L10** | **🔑 `-T` = IMMORTAL, not `.rodata`.** The proposal welded a semantic property to an implementation. Under the agency axis the defining property is *"nobody decides when it dies"*. `.rodata` is one way; **heap-allocated-and-never-freed is another**, equally sound. |
| **L11** | **`-T` has TWO constructors. C2 ships v1; C1 is DEFERRED INDEFINITELY.** **C1** = comptime const-eval → `.rodata`; needs comptime **AND** transitive const-placement of every interior allocation — **do not promise a phase**. **C2** = one-way promotion `+T → -T` via `immortal`; needs nothing, ships v1. This is `Box::leak` — prior art. C2 **dissolves the transitive-`.rodata` problem** (allocate normally, suppress drop glue recursively). |
| **L12** | **`-T` is Copy** (immutable + immortal + no drop glue). It is `&'static T`. This is the answer to "thread it through 8 frames": threading costs **zero at runtime**. |
| **L13** | **`-T` may NOT be mutably borrowed — PERMANENTLY REFUSED, on single-threaded aliasing grounds, NOT deferred to concurrency.** `-T` is Copy ⇒ `let b = a` gives two locals, one allocation ⇒ borrowck sees **different base locals** and treats them as disjoint (Track B rule 3) ⇒ aliasing-XOR-mutability violated and borrowck **structurally cannot see it** (`is_copy → true` switches move-tracking off at all 6 gates). **State it as a LINKAGE: `-T` is Copy BECAUSE it is immutable; the immutability is what pays for the Copy.** ⚠️ If this is ever overruled, `-T` cycles become constructible and **ADR-0022 §6's acyclicity theorem — the reason this language needs no GC — silently stops holding.** Needs a tooth. |
| **L14** | **`immortal <expr>` — one-word contextual keyword**, mirroring `mint` (ADR-0069). **G overruled O's `.into_immortal("reason")`:** ADR-0020's mandatory-message rule is about operations that **kill the process at runtime** — the message is the **epitaph** shown when the trap fires. **Promotion cannot fail** ⇒ a mandatory message has no consumer ⇒ dead syntax. Also, a method spelling for a compiler intrinsic that flips a type's polarity is **hidden magic** (no user can write one) = a lie in the syntax. (The Author briefly read `immortal raw` as a two-word keyword; `raw` was just a variable name.) |
| **L15** | **Position rule: `immortal` may appear ONLY as a top-level statement of the root module's `main`. Not inside any nested block, however unconditional. `main` may not be called.** Libraries and helpers build and return `+T`; only the entry point promotes. **The OPERAND may branch freely** (`let raw = if c { a() } else { b() }` then promote) — state this explicitly or a D will fence it. |
| **L16** | **⚠️ The Author and O had TWO DIFFERENT RATIONALES for L15 and neither noticed.** Author's: *"created conditionally ⇒ cases where it is not created ⇒ the variable becomes dangerous"* = **definite-initialisation**, already covered by ordinary `let` rules. O's: **bounding the number of leaks** — the only load-bearing one. **The ADR must state the leak-bound rationale and record the definite-init reading as SUPERSEDED**, or the first engineer to add definite-init analysis will relax the rule for the Author's stated reason and silently delete the property that makes C2 sound. |
| **L17** | Two properties L15 buys: ① **every immortal allocation SITE is statically enumerable by reading `main`, and each runs exactly once** (not the bytes — a `-Vector` from a config file has runtime size; O oversold this and G corrected it). ② **🥇 The decision to leak always belongs to the APPLICATION, never to a library.** A dependency cannot silently leak on you; it can only hand back a `+T`. Rust has no such property. **Lead the ADR with ②.** |
| **L18** | **No RUNTIME INITIALISATION AT MODULE LEVEL.** Not "no globals" — the constraint is about *when*, not *whether*. Two axes are orthogonal: **lifetime** (global — YES, spelled `-T`) vs **scope** (ambient — restricted). Module-level `constant` with a **comptime** value gets an ambient name (Java's `public static final`; `Item::Constant` already carries visibility). Runtime-built immortal data does not — born in `main`, threaded. |
| **L19** | **🏅 PROJECTION PROPAGATION LAW — the Author found this by intuition and it is load-bearing.** Projecting out of a `-T` must yield something itself immortal, **transitively**. Not merely cognitive: if `c.name` yielded an OWNED `String`, the copy's drop glue would free a heap buffer **shared with the immortal original** ⇒ double free. **MIRROR ADR-0084 VERBATIM** (SIGNED 2026-07-26, O✅G✅ — its core-principle block is the Author's argument word for word): *`f` scalar → scalar value (terminal); `f` aggregate OR heap-leaf → **`-F`** zero-copy place projection, transitively. Never copy or move an aggregate or heap value out of a `-T`.* ⚠️ O's own split ("field carrying an allocation") **diverges from ADR-0084 at the bare-Copy-aggregate cell** — do not ship two nearly-identical projection rules. |
| **L20** | **`-Scalar` is a RENDERING, not a TYPE.** The Author asked: *"I want `-Integer`, and the IDE must also report `-Integer`… I don't actually care how memory is managed underneath — I want to see the intuition."* O translated a **display** request into a **type** and paid two costs for it. **G's ruling:** `-Integer` is **not parseable in type position** (fix-it diagnostic: *"`-Integer` is identical to `Integer`; remove the `-`"*); the compiler and IDE **display** `-Integer` wherever the value's provenance is a projection from a `-T`; `-T` is a real type only where there is drop glue to suppress. **This restores synonym count 1 → 0, needs NO Leak C relaxation, and needs NO carve-out in the L4 coercion rule.** It is also *more correct*: `let m = c.max; m = m + 1` is obviously fine (your own copy) and the type reading would have refused it. `c.max + 1` is `Integer`, always — cosmetics, not semantics. |
| **L21** | **Grid refusals (Leak C) — 2 cells refused of 9.** `-T → &0 mutable T` (L13). `-T → T` move/copy-out (the owned copy's drop glue would free interior pointers shared with the immortal original = double free). O's "all three borrow cells are the SAME type" was **false** — state the refusals or a D will implement the uniform version and generate live UB. |
| **L22** | **Leak B — mutable whole-slot replacement is INEXPRESSIBLE** (`f(slot: &0 mutable (+Node)?, new: +Node)` — Rust's `&mut Box<T>` vs `&mut *box`). Soundness-neutral expressiveness hole, and precisely the linked-list-splice idiom. **Refuse in v1 with a dedicated error code, defer.** Do NOT ship a skeleton (`feedback_cham_ma_chac_pattern`). |
| **L23** | **`context` (Odin ambient) gets its OWN ADR, AFTER concurrency has a shape.** Not now. G's reason, which O did not have: **`context`'s semantics are thread-dependent** — each thread needs its own, and the propagation rule at a spawn boundary IS the design. Pulling forward a mechanism whose core semantics are decided by a deliberately-deferred subsystem is designing blind, and is exactly what the Author's directive #4 exists to prevent. Also Pillar ① — an implicit parameter is magic at its most direct, and the Author was willing to forgo ambient access entirely before O talked him into needing it. |
| **L24** | **`-T` cycles are NOT CONSTRUCTIBLE** (derived): building a pointer cycle needs post-construction mutation → needs `&0 mutable` on `-T` → refused by L13; building it as `+T` first is blocked by ADR-0022 §6.1. Good — the immortal door does not smuggle cycles past the acyclicity theorem. **But it is a DERIVED property resting on L13.** Record the dependency; give it a tooth. |
| **L25** | **`&-` inversion is a REPAIR, not churn.** SPEC.md:1038 says `&- T` *"deref returns `T?`, compile-time tracked, **no runtime gen check**"* — the two halves contradict. `examples/doubly_linked_list.tri` (the Author's own, following ADR-0022 §12.2's template) calls `list.tail.upgrade()`, an API with **0 implementations** that cannot exist at zero runtime cost, and produces a **dangling `&-`** that ADR-0022 §9.3 declares impossible. **Keep it OUT of `examples/`; quote it in the ADR as the disproof of `&-`-as-weak-observer.** |
| **L26** | **HONESTY, enforced by G as a sign-off condition.** ① Lifetimes are **NOT eliminated — they are REFUSED**: `check_lifetime_elision` (`typecheck/check.rs:530-570` + mirror `lower/lib.rs:1329-1367`) requires exactly **0 or 1** input borrow params; `longest(a: &0 String, b: &0 String) -> &0 String` → **E2400**. Write: *"Triết does not eliminate region variables; it refuses the programs that need more than one."* ② **`Rc` is not eliminated** — pushed out of the 90%, reshaped in the 10% into `arena.get(id) -> T?` which the programmer still sees. ③ **`Send`/`Sync` vanish ONLY while no shared-mutable primitive exists**; banishing `Rc` pre-pays for it. **Record ②③ as DEBT, not victory.** |

## THE OPEN QUESTION — for the Author, first thing next session

**Global config access: thread by hand, or the fourth shape?**

O gave the Author a **trilemma** and G proved it **not exhaustive**. The Author is owed the fourth row before he decides.

| | Nameable anywhere | Runtime-init | No reachability analysis | No runtime check |
|---|---|---|---|---|
| `constant` comptime (C1) | ✅ | ❌ | ✅ | ✅ |
| `lazy` + `init` (Author's proposal) | ✅ | ✅ | ❌ | ✅ |
| `context` (vaulted, L23) | ❌ (implicit param) | ✅ | ✅ | ✅ |
| **nullable global + `!!`** ← **the missed row** | ✅ | ✅ | ✅ | ❌ *(one compare, visible)* |

```triet
constant config: -Config? = ~0                    // comptime declaration → NO init-order problem
function main() -> Integer {
    config = immortal parse_config("app.toml")    // an ordinary statement
    return run()
}
function helper() -> Integer = config!!.max       // one compare + trap, fail-closed
```

Rests entirely on **landed** machinery: `T?` PA-3c sentinel (ADR-0041) + `!!` ForceUnwrap (`lower/lib.rs:4818`, Slice 2c `c77d674`). The *declaration* is comptime (`~0`); the *assignment* is a statement in `main` ⇒ C++'s fiasco (declarations executing code at load time) does not apply.

- **G does NOT recommend it** over threading, and named the real tension: the Author wants `-Integer` so that *"seeing it, I feel reassured — it's absolutely safe"*, and this shape makes **every access a possibly-trapping operation. The badge says safe; the access says maybe.**
- **O still leans thread-by-hand** — `-T` is Copy so the cost is characters, not cycles; bundle one `-AppContext`; adding a dependency later means editing **one struct**, not N signatures.
- **The Author decides.**

### Also unresolved (Author's call, lower stakes)
- Nothing else. L1–L26 are settled.

### Rejected, with reasons recorded (do not relitigate without new information)
- **Banning `-T` in parameter position** (Author proposed). Refused. Both argument-position gates are Copy-gated (`borrowck/checker.rs:1333`, `:1367`) ⇒ passing a `-Config` is a register copy with **zero** borrowck obligation. The ban forbids the *common* case (a local `-Config` only `main` can name) to prevent the *never* case (passing a globally-nameable constant) — **a cage around the wrong animal**. It also kills `-T` struct fields indirectly: any struct bearing one could only ever be built inside `main`. ⚠️ It is the **lifetime-vs-scope conflation the Author had corrected in O two turns earlier**, recurring in his own reasoning.
- **`lazy` + `init`** (Author proposed). Refused. Break: `helper()` reading `config` before `init` needs call-graph reachability. **The kill that holds is G's EMPTY MIDDLE:** *restrict the initialiser grammar enough for the compiler to see it all ⇒ it becomes comptime-evaluable ⇒ **it IS C1**, needing no `lazy` and no `init`; loosen it enough to admit `parse_config("app.toml")` ⇒ unrestricted reachability. There is no middle.* 🩸 **O's original argument — separate compilation (`.tripkg`/`.trimeta`) — was HIDING BEHIND A COMMITMENT and must be STRICKEN**: `ARCHIVE.md:87,89` classifies ADR-0011/0013 as **TOOLING** for the **v0.4 Crate-Pack architecture deleted 2026-06-04**, `.tripkg` has no ADR at all, and `triet-pack` has **0 hits** in `triet-driver` (Cargo.toml and src). O flagged the risk himself and the measurement confirmed it.

## G's SIGN-OFF: 🔴 BLOCKED — B1–B6

| | Condition |
|---|---|
| **B1** | RECON-0 (recursive type names, see below) measured. **DEMOTED to a parallel track — no longer gates this ADR**, because `-T` needs `+T`, `+T` needs the new `MirType` variant, and **neither needs recursive types**. The Author's config story ships without it. |
| **B2** | The H3 measurements exist as numbers — **including per-polarity `is_copy` across the 6 borrowck gates, the aggregate field walk, and the fail-open default**, with **orthogonal poison per polarity**. |
| **B3** | The ADR itself (not a Work Order) contains: the 2 refused grid cells (L21) · depth-1 erasure (L2) · the `&` rule (L1) · the coercion rule in one sentence (L4) · placement in type position only (L7) · `&-` signatures-only with ADR-0022 §6.3 marked SUPERSEDED (L6) · **the `T` vs `+T` DOCTRINE paragraph (mandatory, below)** · the honesty statements (L26) · a SUPERSESSION TABLE (ADR-0022 §2, §4.2, §5.1, §6.3, §9; SPEC §10.1, §10.3, §10.4, §10.5) · a DEBT section · the agency table as the axis **definition** with legal promotions **derived** from it · the Copy↔immutability linkage (L13) · the leak-bound rationale with definite-init marked superseded (L16) · "the operand may branch" · "`main` may not be called" · C1 deferred **indefinitely, no phase** · the `-T`-cycles derivation and its dependency on L13 (L24) · H-d (multiple entry points when tests land) in the debt section · **`-` is a rendering on drop-glue-free types (L20)** · the projection law mirroring ADR-0084 verbatim (L19) · **the five teeth (below)** · the four-row trilemma table. |
| **B4** | Three claims corrected before they reach the Author: ① Q1 is a **doc correction**, not a reversal. ② Q2 is already **Locked** by ADR-0022 §4.2, merely unimplemented. ③ **`unwrap_value` does NOT compile** — see BUG-3. |
| **B5** | **The entry point is not a language concept.** Measure the cost of a root-module entry-point notion in **typecheck** before the ADR promises L15. |
| **B6** | **`is_copy`'s aggregate arm and its fail-open default.** Plus: write down the **full ordering-constraint set** for that match as a single documented invariant. |

### The mandatory DOCTRINE paragraph (G made this a hard condition of sign-off)

> **Default to `T`. Reach for `+T` only when (a) the type is recursive, (b) the value must outlive the frame that created it, or (c) it is large and moved often. Reach for `-T` only for immortal data. If none of (a)/(b)/(c) applies, `+T` is wrong.**

Without it, the Author's "does the zoo expand?" worry is correct and **G would reject**. With it, `T` vs `+T` is Rust's `T` vs `Box<T>` — universally understood, one obvious way in every concrete case.

### The zoo count, stated honestly (G's §3.7 — do NOT sell this as a reduction)

> **The axis count goes from 1 tangled to 2 orthogonal. The synonym count goes from 1 to 0. The concept count goes UP BY ONE. That is a net win, and pretending it is a reduction is the lie that lets the zoo expand later.**

At a parameter site today six spellings are legal and **`T` and `&0 T` are synonyms** (SPEC §10.3 says a bare `T` *means* `&0 T`) — a live Pillar ③ violation in the current spec. After: six spellings, **no two mean the same thing**; each answers a different question ("do I own it?" / "where does it live?").

### The five teeth for the projection law (G-Law 13: teeth on 1 of 5 is 0 protection)

| # | Shape | Must observe |
|---|---|---|
| 1 | `-T` → heap-leaf field | yields `-String`; **FREE count == 0** |
| 2 | `-T` → Copy-scalar field | value copy; *displayed* `-Integer`; no move-track |
| 3 | `-T` → nested aggregate with heap leaf | yields `-Inner`, transitive; FREE == 0 |
| **4** | **`-T` field inside a `+T`** — `+Session { config: -Config }` | dropping the Session frees **the Session only**; FREE == 1, config's buffer survives |
| **5** | **`+T` field inside a `-T`** — `-App { cache: +Cache }` | promotion suppresses drop glue **recursively**; FREE == 0 |

(4) and (5) are the boundary cells a form-blind drop-glue walk gets exactly wrong, and (5) is the tooth for the **currently-asserted-and-unproven** claim that suppression is recursive.

## MEASUREMENTS TAKEN — do not re-measure

| Fact | Value | Where |
|---|---|---|
| `&-` (WeakObserver) in real `.tri` code | **0 fixtures** | only a comment in fixture 582 + SPEC/lexer/parser unit tests |
| `.upgrade()` in the compiler | **0 implementations** | 2 comments in `triet-pack`, 1 doc-string on the generated enum |
| `&+` in `.tri` | **9 files**, 5 are REFUSE fixtures | 83 E1042 · 104/105 E2420 · 265 self_ref · 507 drain |
| `ReferenceForm` | 181 sites / 5 variants; load-bearing at 2 places, **both exhaustive** (a 6th variant fails to compile — good hygiene) | `syntax/type_ast.rs:29-52` · `borrowck/checker.rs:121-127` |
| `MirType::` textual occurrences | 864 — **THE WRONG ORACLE** (all 16 variants, incl. constructions, 751 of them in 3 files). O used it; G refuted it. | — |
| **`MirType::Reference` sites** | **50 total, 47 FORM-BLIND, 3 form-aware** ← the real risk surface | `jit/mir_lower.rs:367` states the invariant in a comment |
| **`MirType::is_copy` gates in borrowck** | **6** (`:484 :851 :959 :975 :1333 :1367`); `:795` is a **comment** — G said 7, **O corrected G** | `borrowck/checker.rs` |
| **ADR-0068 / 0073 / 0074 / 0075** | **DO NOT EXIST** — 4 phantom ADR numbers. "ADR-0068 bars Box" was in MEMORY and in **G's own standing-rejections list**, cited against a file never written. G-Law 12 caught G. | `ls docs/decisions/` = 87 files |
| What `&+` means TODAY | **ownership transfer at a parameter (move-in), NOT heap placement** ⇒ `+T` is genuinely new machinery, not a rename | `lower/lib.rs:1342` |
| `LazyLock`/`OnceLock`/`once_cell`/`lazy_static` in the repo | **0** | — |
| `static` in production (`crates/*/src/`) | **2** — `CAP_POLICY` (ADR-0069 hook, deliberate) + `TMP_COUNTER` | The compiler itself has lived 15 months with zero lazy-global init |
| `triet-pack` wired into the driver | **0 hits** (Cargo.toml + src) | — |
| ADR-0011 / ADR-0013 | classified **TOOLING**, v0.4 Crate-Pack, **architecture deleted 2026-06-04** | `ARCHIVE.md:40,87,89` |

## BUGS FOUND IN PASSING — separate Work Orders, not this ADR's scope

| # | Bug | Evidence |
|---|---|---|
| **BUG-1 (RECON-0)** | **Recursive type names do not resolve.** `struct Node { next: (&+ Node)? }` → **E1001 unknown type `Node`** (identical with `&-`). Needs **two-pass type declaration** (declare all nominal names, then resolve field types). Collateral: **not one of ADR-0022 §12's four examples compiles**; §6.4's whole constructibility table (`E2422`) describes machinery that cannot be reached. | `typecheck/check.rs:1283-1314` |
| **BUG-2** | **`constant` PARSES but NEVER LOWERS.** `constant MAX: Integer = 42` → `E1140 undefined local variable: MAX`. **Wrong code too** — E1140 is a USER-ERROR code; this is a compiler-completeness gap ⇒ **E1100**. Emitting a user-error code for a compiler gap blames the user. **G: this is NOT a side item — it is the ambient-name half of L18's answer, a prerequisite for C1, the cheapest thing in the area, and should be TAKEN FIRST.** | `parser/item.rs:395` (parses, with visibility) |
| **BUG-3** | **`unwrap_value` is specified and typechecked but NOT LOWERED.** `get().unwrap_value("…")` → **E1100**. 3 hits, all in `typecheck/check/methods.rs` (`:96 :99 :116`); **0** in lower/mir/jit. ⇒ **O's claimed config example does not compile**, and "needs no new mechanism" was FALSE. Cheap (one arm, trap terminator, mirrors `!!` at `lower/lib.rs:4818`) but not free. | ADR-0020:650 specifies it; its own example message is literally about config |
| **BUG-4** | **`match` on a string literal fires the ICE code.** `match s { "function" => … }` → **E1190** *"unsupported match pattern (expected enum variant)"* on syntactically valid input = an ADR-0086 taxonomy violation, same family as the ADR-0088 campaign. **Out of scope for placement — its own WO on Track B rule #1 grounds.** Note: fixing it solves ~90% of the "compile-time lookup table" need (`match` beats a HashMap: no allocation, no hashing, a decision tree) and would remove most of C1's motivation. | — |

## SEQUENCING (G signed this)

```
WO-A   `constant` lowering + E1140→E1100 (BUG-2)      ← cheapest, real value, TAKE FIRST
WO-B   RECON-B5: root-module entry-point notion in typecheck
WO-C   RECON-1 (B2): per-polarity is_copy across 6 gates + aggregate walk + fail-open default
       (RECON-0 / BUG-1 runs in PARALLEL — does not gate)
   ▼
ADR-XXXX   ONE document, ZERO implementation
   ▼
WO-1 … WO-n   Implementation slices, each gated normally
```

**G REJECTED O's A/B split** (ADR-A reference forms → ADR-B placement): ADR-A lands, ADR-B slips ⇒ a language with three reference forms and **no way to own anything on the heap** — strictly worse than today. And the thesis being sold is *"the two axes are orthogonal and only make sense together"* ⇒ splitting the decision across two documents contradicts the decision.

## LESSONS

- 🩸 **O over-CONCEDED where he feared over-refusing.** He asked G "am I over-refusing?" (twice refusing the Author) and G's verdict: *"You over-refused nowhere, you under-enumerated once, and you over-CONCEDED once. The over-concession is the more expensive of the two, and it is the one you did not ask me about."* The Author asked to **SEE** `-Integer`; O gave him a **TYPE** and paid two costs for a translation nobody requested. **Read what was asked for, not what it implies.**
- 🩸 **O hid behind a commitment.** The separate-compilation argument against `lazy` felt strongest and was weakest — TOOLING ADRs for a deleted architecture in an unwired crate. **O flagged the risk himself and told G to measure it; that is the only reason it was caught.** Name your own weak beams.
- 🩸 **O presented a FALSE TRILEMMA to the person who owns the decision.** G: *"presenting a false trilemma to the person who owns the decision is worse than recommending the wrong option."* Enumerate before you frame.
- ⚖️ **The Author found a soundness law by intuition** (L19) and it had a **signed precedent** (ADR-0084) neither O nor G had connected. His error was only in the *vehicle* (a type); O's was in accepting the vehicle instead of the cargo.
- ⚖️ **The Author made the exact error he had corrected in O two turns earlier** (lifetime-vs-scope, in the `-T`-parameter ban). Naming the pattern is worth more than winning the point.
- ⚖️ **O corrected G once, cleanly** (7 `is_copy` gates → 6; G had counted a comment line — the same G-Law 19 failure G had charged O with over the `864`). G conceded immediately.
- 🏅 **G's agency axis (round 1) made CORRECT PREDICTIONS in round 2** — `+T→-T` legal (surrendering agency), `-T→+T` illegal (cannot reclaim), `T→-T` illegal (a frame-bound value has no agency to surrender) — and the type system derives the same three answers independently. **Two independent derivations agreeing on all three cases.** This retires G's 2026-07-10 objection #1 outright rather than by concession, and it is what earns the ternary claim instead of borrowing it. **The Author's instinct was right; his first framing (heap/stack/static as the axis) was wrong.**
- ⚠️ **G now runs on the same model as O** — the model boundary that used to guarantee independence is gone. G re-derived every number from the repo and said so. Across 3 rounds: **G overruled O on 4 points, confirmed O on 4, and was corrected by O once.** Keep demanding derivations, not agreement.

[[campaign_capability_luk3]] [[idea_post_rust_architecture_and_ternary_foundations]] [[idea_ternary_placement_syntax]] [[idea_post_self_host_odin_gems_and_comptime]] [[feedback_cham_ma_chac_pattern]] [[mentor_o_persona]] [[mentor_g_persona]]
