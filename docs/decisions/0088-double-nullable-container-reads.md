# ADR 0088 — Double-Nullable Container Reads (`T??` on `get`-family)

**Status:** **Lane A CLOSED** (O✅/G✅/Giang✅ 2026-07-27, `d9b659a` — see
**§AMEND-1**: 2-tier / 2-source-class `E1055` fence + 9 safeguards 511-519).
**Lane B (true `T??` design) DEFERRED INDEFINITELY.** The main body below represents
the original 2026-07-25 draft (deferring `get`-family via E1051) — it ONLY covered `get`/`get_ref`,
and its §Decision contained **an erroneous assertion regarding `contains`** corrected in §AMEND-1
§88A.4. Read §AMEND-1 prior to taking action based on the main body.

**Issue:** `get`/`get_ref` on containers (`Vector<T>`/`HashMap<K,T>`) returns
`T?` — the outer `?` encodes "whether key/index exists". When `T` is itself already
`T0?` (the value stored in the container CAN be null, e.g.
`HashMap<Integer,Integer?>`), the logical result of `get` is **double-nullable**:
`(T0?)?` / `T0??` — outer `?` = "key found", inner `?` = "stored value is
null". Prior to WO-GetRefNullableRefuse, this case fell through to generic
`NoMatchingOverload` (E1041) — a CORRECT refusal (did not compile) for the
WRONG REASON (appeared as "no matching overload exists" rather than "double-nullable
is not yet designed").

Concrete tension revealed during recon: `crates/triet-mir/src/lib.rs:4030` (test comment
`nullable_type_helpers_round_trip`) noted *"Edge case: 'Integer??' — can't
happen (C6: T?? auto-flatten), but helper must be defined"*, and on line
`:4032` the test manually constructed `MirType::Nullable(Box::new(MirType::Nullable(
Box::new(MirType::Integer))))` to prove `is_nullable()`/
`nullable_payload()` do not panic on that shape — meaning **MIR CAN
represent `T??`** despite the comment asserting it "cannot happen". Comparing with
`crates/triet-typecheck/src/lib.rs` (`nullable_flatmap_flattens_nullable_body`,
~line 89): auto-flattening **ONLY occurs at one specific site** — the `?+>`
(flatMap) operator on `T?`, where bodies returning `U?` are flattened to `U?`
(not `U??`) per the operator's design (ADR-0020 §3 ternary map family).
This is NOT a global type-system invariant — it is the local behavior of
ONE operator. Container `get()` does not route through `?+>`, so nothing
automatically flattens it; if not refused, typecheck lowers `(T0?)?` to the
overload table and enters undefined behavior (silent-flattening in some layer,
or double-nullables reaching MIR/JIT where 1-bit sentinel ABI
(`i64::MIN`, ADR-0041 PA-3c) has no representation for 2 independent levels of null).

## Decision

**Explicitly refuse** double-nullable container reads at typecheck
(`E1051`, `crates/triet-typecheck/src/error.rs`), rather than:
(a) silent-flattening (losing distinction between not-found vs stored-null — WRONG result), or
(b) shipping a half-baked `T??` representation sufficient only for `get`-family without
verification across AST/MIR/JIT/match.

Guard placed in `resolve_overload` (`crates/triet-typecheck/src/check/
exprs.rs`), immediately following the existing heap-violation guard (ADR-0077 Slice B / ADR-
0078 P1b) and preceding the aggregate-key arm (ADR-0083 §5): if `name == "get"` and
the container (`Vector<V>`/`HashMap<_,V>`, including via `&0` borrow) has `V =
Nullable(_)` → push `TypeError::GetContainerNullableValueUnsupported` (E1051),
`return Type::Unknown`. The guard DOES NOT block `contains` (which returns `Trilean!`
found/not-found, without generating double-nullables).

### Concrete Form

```
function f(m: &0 HashMap<Integer, Integer?>, k: Integer) -> Integer =
    get(m, k)  // E1051: get()/get_ref cannot return a double-nullable Integer?? value type
```

Refusal occurs AT typecheck — no constructs reach MIR/JIT (avoiding "refusal gaps"
like ADR-0065 §15 / ADR-0085 — sound by construction, not by luck).

## Alternatives Considered

| # | Alternative | Pros | Cons | Conclusion |
|---|-------------|------|------|------------|
| 1 | Silent-flatten `(T?)? → T?` like `?+>` | No new error required, "works" immediately | **Information loss**: cannot distinguish "key not found" vs "key present, value null" — two semantically distinct scenarios merged, producing WRONG results for programs using `HashMap<K, V?>` to explicitly model optional presence | Rejected — worse than refusing |
| 2 | Fully design `T??` (tuple representation `(present, present_inner, value)` or 3-state tag) immediately | Resolves root cause | Touches AST + typecheck + MIR layout + JIT ABI (sentinel currently has only 1 bit for null) + match ergonomics — full standalone campaign, no ADR currently locks 3-state design | Deferred — as ruled by G, avoid half-baked shipping |
| 3 | **Explicit refusal (E1051), deferring design** | Immediate safety, accurate error rationale, preserves future design options | Users temporarily cannot invoke `get` on nullable-valued containers (workarounds: sentinel values or wrapper Structs with explicit presence flags) | **CHOSEN** |

## Consequences

### Positive
- `get`/`get_ref` never returns a type that the pipeline (MIR/JIT ABI) cannot
  correctly represent — refusal occurs as early as possible (typecheck), before
  any MIR is lowered.
- Accurate diagnostic rationale (E1051 instead of ambiguous E1041) — users understand
  EXACTLY why and how to adapt (sentinels / wrapper Structs).
- Preserves future options: when `T??` is fully designed (dedicated campaign), this guard
  is simply removed/narrowed — no external code depends on the absence of `T??`.

### Negative
- `HashMap<K, V?>` / `Vector<V?>` restricted: `get`/`get_ref` unavailable,
  requiring prior `contains` checks or alternative data modeling.

### Risks to Mitigate
- Current guard only catches shallow 1-level `Vector(Nullable(_))`/`HashMap(_, Nullable(_))` —
  if in the future `V` is an aggregate CONTAINING `Nullable` fields (rather than `V` itself
  being `Nullable`), this guard DOES NOT catch it (that belongs to a different domain:
  ADR-0082/0083 aggregate rules, not double-nullables). Do not conflate these two refusal layers
  during code review.

## Effective Date

- No version assigned — this is a backlog/deferral record, not a "locked" decision
  for a shipped feature. E1051 took effect immediately upon merging WO-GetRefNullableRefuse
  (refusal is active code; ADR documents rationale + design deferral).
- Reopened upon dedicated campaign for `T??` semantics (synchronized AST/typecheck/MIR/JIT/
  match) — this ADR serves as reference anchor, not the definitive design.

---

## §AMEND-1 — Lane A: 2-tier / 2-source-class `E1055` Fence + 9 Safeguards

**Status:** Lane A CLOSED (O✅/G✅/Giang✅, 2026-07-27, `d9b659a`).
**Lane B (true `T??` design) DEFERRED INDEFINITELY** — Ruled by G: without real-world
use cases requiring distinction between "key does not exist" vs "stored value is null",
designing 3-state representations builds a bridge across an empty desert.

### §88A.1 — Recon Frame Shift: Main Body ONLY Covered `get`-family

The main body above addressed only `get`/`get_ref`. Recon across 20 probes on release
binaries (2026-07-27) revealed a broader landscape: **DIRECTLY declared `T??`** (outside
`get`-family) had never been measured. Findings: **NO UB occurred** — all paths
failed closed — but diagnostics FRACTURED: the identical concept `T??` triggered **5 distinct
error codes**, including:

- 🔴 `struct S { v: Integer?? }` + match → **E1190** — an **ICE**
  ("please report this as a compiler bug") emitted for syntactically valid user code.
  Violated ADR-0086 error taxonomy (E1190 reserved exclusively for internal compiler bugs).
- ⚠️ local/param/return/`pop`/`pop_front`/`remove`/`!!` → MIR verifier message
  stated *"heap-nullable… ADR-0065 §4 (B8) Struct?/Enum? Copy-only… [Fix 1]
  Remove the heap field from the struct/enum"* — **entirely misleading**:
  `Integer??` has no heap, no struct. Violated ADR-0027 machine-fixable principles.
- ⚠️ `enum E { A(Integer??) }` → E1141 "requires type annotation" (incorrect rationale).
- ⚠️ `Integer??~E` → E0001 parse error (lexer `?~` compound token).

**Mechanism holding prior to Lane A:** `is_lowerable_nullable_payload`
(`crates/triet-mir/src/lib.rs:1796`) was an **allow-list** (scalar / heap /
Enum / Struct / Reference); `Nullable(_)` was absent, so `T??` fell through
→ refused **by default**. This was **structural good fortune**, not an active fence —
and **0 fixtures guarded it** (grepping `??` across the entire codebase = 0 hits).
Adding a `Nullable` arm to that allow-list — the first step Lane B would take —
would allow 7 pathways to reach the JIT simultaneously, **silently, with green test gates**.
This was the exact single-layer SPOF pattern patched in WO-SPOF-1.

### §88A.2 — TWO Source Classes Generating `T??` (Why WO required multiple touchpoints)

| Class | Origin of `Nullable(Nullable(_))` | Coverage |
|---|---|---|
| **A — Declarations** | `resolve_type`, **TWO copies**: `check.rs:1365` + `check_resolved.rs:597` | local annotations · params · returns · struct fields · enum payloads · `!!` on annotated locals |
| **B — Inference** | `check_call`, following `return_type.substitute(&sub_map)` (`check/exprs.rs`) | `pop` / `pop_front` / `remove` on container `<T?>` |

`let x = pop(v)` **carries no annotations** to traverse `resolve_type` — the shape
manifests only AFTER substitution. A guard placed solely in Class A completely misses this path.

⚠️ **Locked Invariant:** Class A contains **2 duplicate copies** of `resolve_type` — mirroring
the 3-copy pattern of `is_fat_ret` (ADR-0065 §14.7). Touching one copy MANDATES grepping for the other.

**Class B Location — D rejected WO's suggestion of `env.rs` correctly:** `env.rs:374/394/506`
declares `pop`/`pop_front`/`remove` ONCE during env initialization, with `T`/`V` remaining
abstract `TypeParameter`s — unaware of what `T` binds to at specific call sites.
`check_call` is the SOLE chokepoint where concrete types exist.
The guard is **not name-gated**: any generic function returning `Nullable(T)` that binds
`T` to a nullable is uniformly caught (consistent with Class A).

### §88A.3 — MIR Layer = Tier 2 (Becomes phantom code without unit tests)

Once Classes A+B block at typecheck, **no `.tri` pathways reach the MIR layer**.
The message at `triet-mir/src/lib.rs` was updated to accurately describe nested-nullables
(removing misleading ADR-0065 heap/Struct? references), with a mandatory **dedicated unit test**
`nested_nullable_refused_with_correct_message` passing `MirType::Nullable(Nullable(Integer))`
directly into the helper. Precedent: N1/N3 in ADR-0083 Option A — N1 blocks all fixture pathways,
requiring N3 to maintain independent teeth.

⚠️ **False-Positive Trap Caught and Fixed by D:** the draft MIR message contained the
string `"(E1055)"`; the test harness checked `.contains(code)`, causing poison tests removing
the typecheck guard to falsely "pass green" (MIR layer triggered, message happened to contain "E1055").
D discovered this independently, removed the error code string from the runtime message, and re-verified
with the actual harness. **MIR runtime messages MUST NOT contain error code strings from other compiler tiers** —
otherwise, cross-tier poison tests are rendered silently ineffective.

### §88A.4 — CORRECTION to Main Body: `contains` WAS NOT "unblocked"

The main body §Decision stated: *"The guard DOES NOT block `contains` (which returns `Trilean!`
found/not-found, without generating double-nullables)"* — **describing non-existent behavior**.
Real measurement (`triet-driver run`, 2026-07-27):

```
contains(m, 1)  with  m : HashMap<Integer, Integer?>
→ E1041 NoMatchingOverload
   available overloads: (String, String) · (Vector<Integer>, Integer)
   · (HashMap<Integer, Integer>, Integer) · (HashMap<String, Integer>, String) · …
```

`contains` **was also unusable** with `V = Integer?` — because the overload table
did not declare generic `V`, NOT because it was permitted through. Not UB,
not a hole; purely a **documentation error**. Anyone reading the main body assuming
`contains` served as a valid workaround for `HashMap<K,V?>` would fail.

### §88A.5 — 9 Safeguards + 2-Prong Poison Verification Protocol

Fixtures **511–519** (7 explicit pathways, `pop`-family split into 3 cases):
`511` local · `512` param · `513` return · `514` pop · `515` pop_front ·
`516` remove · `517` struct field (**forbidding E1190**) · `518` enum payload
(**forbidding E1141**) · `519` `!!`.

**MANDATORY verification protocol for all modifications touching this fence** (Approved
by G after O rejected G's initial 1-prong proposal — 1 prong trapped verifiers into
"poison not failing RED" scenarios requiring fabricated test failures):

| Prong | Poison Modification | Expected Failure Pattern |
|---|---|---|
| 1a | disable Class A guard | **6** fixtures fail RED: 511·512·513·517·518·519; 514·515·516 **pass GREEN** |
| 1b | disable Class B guard | **3** fixtures fail RED: 514·515·516; other 6 fixtures **pass GREEN** |
| 2 | relax MIR allow-list | **ONLY MIR unit test** fails RED; **0 fixtures** fail RED |

Verified by O independently on 2026-07-27 across all 3 prongs + bidirectional specificity
(6+3=9, no cross-tier masking); under prong 1a, `517`/`518` **re-exposed the original ICEs**
(`unsupported match pattern` / `requires an expected type`) ⇒ proving the new guard eliminated
E1190/E1141. Safeguards proven at **harness layer** (modifying `// ERROR:` in 514 to `E9999`
→ yields FAIL expected/got line, rule 15). Restored via `cp`-snapshot + matching md5, NO `git checkout`.

**Over-refusal Controls (must remain preserved indefinitely):** 1-level struct `Integer?`
→ `16` · `HashMap<K,Integer?>` insert-store → `5` · `?+>` flatMap 175/212/213 GREEN
(`exprs.rs:361-364` preserves nullable body, **never emitting `U??`**) · 465/466/467
**retain E1051**, not intercepted by E1055 · 468 positive control.

### §88A.6 — Error Code Boundaries (One Error Code, One Contract)

- **E1051** — `get`/`get_ref` on containers with nullable elements/values.
  REMAINS UNCHANGED, Lane A does not modify.
- **E1055** `NestedNullableUnsupported` — nested `T??` across **all other positions**
  (declarations + inferences). New code, registered in `triet-typecheck/src/error.rs`.
- Merging the two codes is forbidden; E1055 must never displace E1051.

### Effective Date §AMEND-1

- Effective from `d9b659a` (2026-07-27). Gate `0 · clean · 0 · 511 · 0 · CLEAN`
  (fixtures 502 → 511).
- Lane B reopens **solely when** practical use cases arise + an ADR defines synchronized
  AST/typecheck/MIR/JIT/match representations. When that occurs, the first step is relaxing
  the MIR allow-list — and the 9 safeguards above will fail RED if Lane B is incomplete.
  That is their explicit purpose.
