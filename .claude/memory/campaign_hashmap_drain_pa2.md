---
name: campaign_hashmap_drain_pa2
description: "✅ CLOSED 2026-07-27 — HashMap.drain() LANDED via PA-2 destructuring-only desugar (ADR-0089 §AMEND-2). PA-1 first-class Tuple was killed by G (729 MirType sites). Invariant: Tuple lives at the front, dies at lower. First simultaneous move-out of heap-key × heap-value. 816a729, gate 0·clean·0·522·0."
metadata:
  node_type: memory
  type: project
---

## ✅ CLOSED — `816a729` (D) + docs (O co-sign). O✅/G✅/Giang✅ 2026-07-27

Gate `0·clean·0·522·0 CLEAN`. Fixtures 511 → **522 files** (numbers 520-530).

## 🎯 RECON OVERTURNS THE ENTIRE FRAME OF THE LABEL ITSELF

Giang settled on *"Tuple lowering to handle HashMap.drain()"*. The old deferral label
(ADR-0089 §AMEND) recorded **2 walls**. Re-measured:

- **Wall 2 (no key-less enumerate-shim) = NOT A WALL AT ALL.**
  Drop-glue `mir_lower.rs:1940` (free KEY) / `:2038` (free VALUE) / rehash
  `:6583` **already walks the entire key-less `cap`, filtering `state==1`**, for a
  long time. The mechanism EXISTS and has been verified in blood — it just hadn't
  been exposed as a public shim yet.
  The label was right at the *shim* level, wrong at the *mechanism* level.
- **Wall 1 (Tuple) should NOT be broken through — it should be GONE AROUND.**

## 💀 PA-1 (first-class Tuple) KILLED BY G

`MirType::` is matched at **729 sites** (`mir_lower.rs` alone has 29 exhaustive
matches). Adding 1 variant = **re-seeding the exact "match exact, FORGOT the
variant" bug family** the project JUST spent an entire campaign sweeping ("forgot
`Nullable`" family: 6 members, **2 of them inside the safety net itself**). It also
touches **B-γ multi-reg return** (deferred indefinitely) + sits next to **B-β
sub-8B** (already killed).

🔑 **Decisive architectural question:** `for (k,v) in m.drain()` needs **TWO
VARIABLES in the loop body**, NOT a **tuple VALUE**. PA-1 builds a first-class
type only to immediately break it back into two — paying 729 sites for an
intermediary nobody keeps around. G: *"burning down your own house to light a
match."*

## 🔒 INVARIANT: Tuple lives at the front, DIES at lower

`MirType::Tuple` = **0** across the entire backend (O verified). PA-2 = **0 new
variants, 0 hits on the 729 existing sites**.

⚠️ **VERIFICATION CRITERION — O takes a hit:** a bare `grep -c Tuple` is **NOT**
the criterion. The lowerer **MUST** match `triet_syntax::Pattern::Tuple`
(`triet-lower/src/lib.rs:2036`) to destructure — that IS EXACTLY the PA-2 design,
not a violation. **D refutes O's crude proxy criterion with a measurement — D is
RIGHT.** The only correct criterion is: **`MirType::Tuple` = 0**.

## 🔑 THE 4-STEP MOVE-OUT CHAIN — one `state` flag closes ALL THREE fatal weak points

Shim `__triet_hashmap_drain_next` (`mir_lower.rs:7005+`), mirroring
`__triet_hashmap_remove:6824`: surface K+V to out-ptr → **zero the key-cell** →
**`state→2`** → **`len--`** → return `idx+1`.

Drop-glue **only walks `state==1`** ⇒ ① move-out is sound (tombstone exempts
from double-free) · ② break-mid (already-drained `2` is skipped, remaining `1`
is cleaned up by drop-glue) · ③ container survives (`len--` ⇒ a full drain
reaches `len==0`, re-insert is valid).

**O(N) cursor** avoids O(N²) rescans: `cap=1000,len=10` → **1000** vs **10,000**
state-reads. Sound-stop `while idx<cap` checks the condition **BEFORE** reading
the byte ⇒ `cap==0` is safe (fixture 525 guards this).

## 🔑 SENTINEL CONVENTION (G required this be written into the ADR)

| Sentinel | Value | Meaning |
|---|---|---|
| `NULL_SENTINEL` | `i64::MIN` | **absent value** (nullable PA-3c) |
| cursor-stop (new) | **`-1`** | **no more slots to scan** |

The cursor domain is always `≥0` ⇒ `-1` never collides with the valid range.
**MIXING THE TWO CONCEPTS IS FORBIDDEN.**

## 🩸 O VERIFIES IN BLOOD — 3 INDEPENDENT poison probes

| Probe | Poison | Measured |
|---|---|---|
| **P1** | `state→2` becomes `1u8` | `drain_full` **9 vs 6** · `break_mid` **10 vs 8**, repeated pointer = **REAL double-free** |
| **P2** | drop `len--` | `drain_full_leaves_len_exactly_zero` **3 vs 0** |
| **P3** | fail-**open** guard `if true` | 527-530 go red **+ pre-existing fixture 510 goes red in sympathy**; 520-525 do **not** go red |

**First time in the project's history: simultaneous move-out of heap-key
(String) × heap-value (String/Vector) from the same bucket.** P1 is the tooth
guarding the main minefield. Counting teeth **dedup POINTERS** (`count==N` AND
`dup==0`) — a plain FREE-count is blind to double-free (3 frees could be 3
objects OR 2 objects + 1 duplicate).

## ⚔ TWO POISON LESSONS

**P2 — "not red" must be resolved into (a)/(b) via a REACHABLE PATH.** The drain
loop stops via `state` through the cursor, **NOT via `len`**; re-insert at
`cap=4` never touches the resize threshold ⇒ **(b) weak test**, not (a)
unobservable-in-principle. **D reported HONESTLY and THEN added a tooth
themself** reading `len(m)` directly — not faking a probe just to get past the
gate (exactly the escape hatch O had written into the WO).

**P3 — "removing the guard" means fail-OPEN, NOT fail-closed.** D got the
direction wrong the first time (`if false &&` = tighter ⇒ proves 0), **caught it
themself, fixed to `if true ||`, re-measured, reported both attempts**. Under
the correctly-directed poison, the refuse shapes **do NOT slip through** but are
caught by a different `LowerError` in the lowerer ⇒ **2-layer defense-in-depth**
(typecheck = correct code, lower = final fail-closed backstop) — the same
architecture as ADR-0088 Lane A.

## 🩸 O WRONG ON 2 CRITERIA — D refutes both with measurements

1. **`grep -c Tuple` = 0/0/0** — crude proxy, would reject the very design O
   ordered.
2. **"gate target 530 fixtures"** — confused the **highest fixture number**
   (530) with the **TOTAL FILE COUNT** (522), while **O themself had run
   `ls|wc -l` = 511 in the same session**. The data refuting O was already in
   O's own hands.

Same root cause **"acting before measuring"** — now surfacing at the
*acceptance-criteria* layer, exactly matching the pattern of Rule 16. The "measure
first" discipline is **still not a reflex**.

## Slice 1 fence + open debt

**OPEN:** `K` ∈ {scalar, String} · `V` ∈ {scalar, String, Vector, HashMap}.
**REFUSE E1054:** pattern≠tuple-2 · tuple-3 · aggregate key/value · `V=Nullable`.
**Outside for-guard → E1015** (precedent from Vector 491, keeping for-guard-ONLY).

⚠️ **E1054 now carries 4 MEANINGS** — pattern-shape cases still print `key`/`value`
even though the actual cause is the pattern shape. G temporarily accepts this for
Slice 1; split it later if "one E-code, one contract" is tightened.

**Debt:** aggregate key/value drain (move-out of an aggregate key = new ABI) ·
`V=Nullable` (see 🩸 CORRECTION below) · split E1054 · **PA-1 remains REJECTED**.

## 🩸 CORRECTION 2026-07-27(e) — G REFUTES O's "UB double-free" (reverse verify-don't-trust)

O reconned `V=Nullable drain` (Giang settled this battlefront), widened 2 fence
probes → `HashMap<_,Integer?>`/`HashMap<_,Vector?>` + stored-null `~0` →
**SIGABRT 134**; `String?` clean. O **guessed** "pre-existing double-free in
drop-glue, independent of drain" → **WRONG**. **G refutes with file:line:**
`__triet_hashmap_insert:6620` has `if v == NULL_SENTINEL { abort() }` = **canary
D2 (ADR-0044 Q4)**. `insert(k, ~0)` with V of **stride-8** (Integer?/Vector?/HashMap?)
passes `v = i64::MIN` by-value → hits D2 → **aborts AT INSERT, never reaching
drop**. `String?` slips through because it passes a 24B POINTER (address ≠
MIN); present Integer slips through because it's ≠ MIN; drop for Integer? is
skipped (`aggregate_needs_drop`=false). **One mechanism explains the entire
table — there is NO double-free, NO UB.** This is a **design limit
(fail-closed trap)**, not memory corruption.

🔑 **Fact for the record (G's order):** `HashMap<K, V?>` with V of stride-8
exploding 134 on `insert(k, null)` = **D2 trap ADR-0044 Q4**
(`__triet_hashmap_insert:6620`), NOT drop-glue. Whenever `V=Nullable` for
HashMap is fully opened, an ADR **MUST** exist (amending ADR-0044/0083) to
arbitrate removing/replacing D2 to properly match Nullable semantics.
⚠️ **Latent (gated behind D2, not yet live):** if D2 is removed, `Vector?`
value-drop runs `emit_hashmap_value_free_loop`
(aggregate_needs_drop(Vector)=true) → `emit_heap_free_at` on the sentinel
cell — needs checking that it skips NULL_SENTINEL. Today this is
UNREACHABLE (D2 blocks it at insert) ⇒ NOT a live hole.

**Lesson for O (12th+ instance of "acting/guessing before measuring"):** seeing
134 → immediately jumping to "double-free" instead of "intentional trap", even
though CLAUDE.md explicitly states "ADR-0044 shim traps → SIGABRT". Violates
feedback_failure_mode_precision (134 = double-free OR abort; the site must be
LOCATED before naming it). G's reverse verify-don't-trust of O was correct this
time.

## ✅ CLOSED 2026-07-27(e) — (C) SPLIT E1054's 4 MEANINGS → 3-CODE PA (`d4baf60`)

Front (C) landed. `d4baf60` (D) + ADR §AMEND-3 (O drafted the WO, D pre-filled
the signature). O✅/G✅. Gate `0·clean·0·522·0` (O ran it independently).
**Does NOT touch soundness/JIT — purely diagnostic taxonomy** (ADR-0086
one-code-one-contract).

E1054 crammed 3 axes into a single `if&&` → split into a cascade
**pattern→key→value**:
| Code | Variant | Axis | Fixture |
|---|---|---|---|
| **E1056** | `DrainHashMapPatternUnsupported` | pattern≠`(k,v)` — **message MUST NOT print key/value** | 510,527,528 |
| **E1054** | `DrainHashMapKeyUnsupported` (narrowed, dropped the `value` field) | K aggregate — `HashMap<{key},_>` | 529 |
| **E1057** | `DrainHashMapValueUnsupported` | V nullable/aggregate — `HashMap<_,{value}>` | 530 |

5 typecheck touch points (`check.rs` cascade + `error.rs` 3 variants + 3
`error_span` arms) + 4 fixture headers + ADR. **526 keeps E1015** (drain outside
for-guard).

🩸 **O VERIFIES IN BLOOD:** Poison A (flip 5 headers→5/5 FAIL, "got:" reveals
the REAL code for every fixture — 510/527/528=E1056 no-key/value ·
529=E1054 `<KP,_>` · 530=E1057 `<_,Integer?>`) + live cascade-order probe
(multi-axis all-3-wrong→E1056 pattern wins; key+value both wrong→E1054
key-before-value). Restore byte-identical md5.

⚔ **O'S BLEMISH: recon-gap — forgot to link fixture 510 into the WO** (read 510
at the start of the session then lost track of it). **D STOPPED-and-reported
per LAW 4 correctly**; O verified 510 (bare-var + valid K/V → violates only the
pattern → E1056, 100% mechanical). The half-mapped WO was O's error, D handled
it correctly.

**Light follow-up debt (G approved, NOT blocking):** add 1 multi-axis fixture
(`HashMap<Struct,Integer?>`+`for x in`→E1056) to lock the cascade order into
the corpus (currently only verified live). 0 risk (every axis is fail-closed).

**Debt still OUTSTANDING:** aggregate key/value drain (new ABI) · V=Nullable
drain (needs an ADR to remove/replace the D2 canary) · PA-1 remains REJECTED.

[[campaign_iteration_slice2b_drain]] [[campaign_iteration_slice2d_borrow_drain]] [[campaign_adr0088_lane_a_nested_nullable]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]]
