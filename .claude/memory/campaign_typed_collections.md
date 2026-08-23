---
name: campaign_typed_collections
description: "✅ Typed Vector/HashMap P1 (ADR-0077/0078) + Get-Borrow (ADR-0079) + key-typed HashMap<String,V> (ADR-0080) + Read-side Cluster A + CLUSTER B push+drop Slice A/B/C (Vector<Struct/Enum>, HashMap value) + VALUE MOVE-OUT D-1/D-2 (Vector pop, HashMap remove by-value, ADR-0082 §AMEND-2) + Vector::pop_front + KEY-AGGREGATE Slice 1 struct-key + Slice 2 enum-key (ADR-0083) + GET-BY-VALUE Copy-aggregate (ADR-0082 §AMEND-3, is_copy_aggregate↔MirType::is_copy, E1049 refuse heap-bearing, non-destructive copy-out) — BOOKS CLOSED 2026-07-15 `12adc0b`. Full detail, MEMORY.md index only links here."
metadata: 
  node_type: memory
  type: project
  originSessionId: ac639140-8210-42c9-941b-8cfd203d270e
---

## ✅ CLOSED — ADR-0079 §AMEND GET_REF SLICE 2: `get(&0 c,k)` → `(&0 Agg)?` (O+G signed off 2026-07-17, PUSHED)
origin/main = `8981135` (1 commit), gate `0·0·385·0`. **Final sweep of E1041/E1049 — NO new ADR** (§AMEND ADR-0079, composes with ADR-0084). `get(&0 Vector<Agg>,i)` / `get(&0 HashMap<scalarK,Agg>,k)` → `(&0 Agg)?` **zero-copy**, heap-bearing **ALLOWED** (E1049's tombstone: `Vector<Tagged{String}>`→`&0 Tagged`→`.name` runs). Fixtures 388-391 + `get_ref_aggregate_counting.rs`. **Model D = Sonnet 5.**

**🩸 O'S RECON FLIPS THE HANDOFF FRAME — "infrastructure ready, just pull the trigger" was WRONG.** The 2026-07-16 log recorded tier 2 as "verify JIT route". Actual measurement turned up **2 MINES**:
- **MINE 1 — the lowerer FORCES every aggregate to `_get_copy` regardless of `&0`** (`triet-lower/src/lib.rs:2501` tests `is_aggregate_elem` BEFORE `is_borrow`; the comment right there confesses *"routed identically either way"*). Opening only the typecheck = typecheck promises a borrow, the lowerer delivers a copy → `returns_borrow_of:None` → **0 loans** = exactly G's fear. Fix: `is_aggregate_elem && is_borrow` → dedicated shim.
- **MINE 2 — 8B-aggregate deref-thin, the THIRD occurrence of this bug class** (after Slice A T9, §AMEND-3 T9-masking). `_get_ref` derefs when `stride≤8` (CORRECT for V=container: the handle *is* the body_ptr) but `&0 Struct` from a local = **stack_addr** (`mir_lower.rs:3154`, Blocker-B Slice 1a) — an address regardless of size. `Id{value:Integer}` 8B → returns `42` instead of a pointer → field-read derefs 42. **O's poison → signal 11 SIGSEGV.**
- **Tier 3 was LIGHTER than the log claimed:** `returns_borrow_of:Some(0)` already existed, keyed by SHIM NAME → correct routing = the loan comes for free. **Borrowck: 0 lines of code.**

**Mechanism:** a NEW typecheck arm placed BEFORE the get-by-value arm, matching only `Reference(BorrowReadOnly,_)` → helper `get_ref_aggregate_return_type` (`check/exprs.rs:2345`), its sibling `get_by_value_aggregate_element` (`:2374`) matches `arg_tys[0]` DIRECTLY → **mutually exclusive by construction** = the is_ref/non-ref death-boundary. **NO `is_copy_aggregate` on the get_ref arm** (heap-bearing is the MAIN use case). D chose to **split off 2 new Rust shims** `__triet_{vector,hashmap}_get_ref_agg` (no deref, any stride) instead of a runtime flag — G praised it as "level-headed", the same move as `_get_copy` in §AMEND-3: splitting the symbol = 0 cost, 0 regression risk for 333/336/337 (which survive on deref-thin). Cleanup blemish that came with it: **the E1049 message pointed at `get_ref(container,k)` — an API THAT DOES NOT EXIST** (env.rs never declares it) → this led the user into E1041; fixed → `get(&0 container, k)`.

**🩸 O's TEETH (cargo test, cp-snapshot restore md5 matched every round, 0 git-checkout):** restoring MINE-2's deref-thin → 389 **SIGSEGV 11** · loan→None → 388 **loses E2440** ("succeeded with 42") · sneaking a copy back in (forcing `_get_copy`) → 388 loses E2440 **+** 390 gets **blocked by the F3 JIT guard** (**F3, latent since §AMEND-3, now BITES FOR REAL**) · `aggregate_needs_drop(Struct)→false` → counting **left:0 right:1** for both Vector and HashMap.

**⚖ O CORRECTS HIMSELF — O's WO had the WRONG failure-mode, D's REFUTATION WAS RIGHT (2nd time after §AMEND-3):** O wrote teeth-1 as "removing the loan → **SIGSEGV leaks through**". D reported the real behavior = **a silent leak-through** (compiles + returns 42, exit 0). O did not accept a bare assertion → ran an independent probe + **pushed further with a realloc** (the old buffer genuinely freed) → still 42/exit 0. **Mechanism: an `&0` reference has NO drop obligation → a dangling READ, no double-free, no deterministic signal** = **(a) unobservable-in-principle**, not (b) a weak test. The loan is a **compile-time** shield; the tooth is red at exactly that tier. G: *"Welcome to Systems Programming — an &0 reference doesn't damn well have a drop obligation"*, and praised O for building the realloc as verify-don't-trust.

**✅ D LOWERS HIS OWN CLAIM (the exact opposite of the "fabricated secondary evidence" blemish in Slice 2 enum-key):** for the counting tooth D was about to claim `==2 ⇒ double-free` → self-poisoned it TWICE → count **STAYED AT 1** → dug into the MIR for the reason (`length(t.name)` lowers to `move _15.name` → the field is MOVED out of the copy → the alias is never freed a second time, it leaks into an undropped temp) → **revised the claim down to `==0 ⇒ leak`** + wrote "HONEST LIMIT" + **forbade whoever comes next from adding an `==2` claim without proof**. G rated this "extremely high transparency".

**🚩 New debt — the `&0 Enum` SURFACE IS DEAD (D self-disclosed, O's probe confirmed):** the typecheck arm accepts `UserEnum` but has 0 fixtures; O's probe → payload-bind = typecheck refuse (`undefined name s`), discriminant-match = **lowerer refuse, a `LowerError`, NOT a panic** (Track B rule 1 holds). Refuse-guarded at both tiers, **no crash** → G: "package it up, log the debt, don't block the release". The arm promises `(&0 UserEnum)?` but there is still no consumption path.

**⚠️ D's operational blemish:** the first round only registered the shim in `main.rs`, **forgetting the SEPARATE registry in `integration_tests.rs`** → 4 fixtures went green by hand-run binary but the gate harness said "shim not registered" — D found it and disclosed it himself (**lesson: run the CORRECT gate harness, don't trust a hand-run binary**). Clippy baseline was 0 → D introduced 4, **all of them D's own code** (no claiming pre-existing), fixed for real by extracting a helper, **0 `#[allow]`**, re-verified the 2 teeth after the refactor.

**Slice 2 lessons:** ① **recon-before-coding saved this slice** — the "just pull the trigger" handoff was hiding 2 mines; verify-before-WO is now the rule (2nd time, after D1 in the previous session). ② **8B-aggregate masking = the 3rd repeat of this pattern** — always probe `total_size==8` whenever opening a new aggregate path. ③ **the failure-mode of a dangling `&0` = a silent READ, NOT a signal** — don't write a WO demanding SIGSEGV for use-after-free of a reference with no drop-obligation. ④ splitting the symbol beats a runtime flag when branching JIT routing. [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]] [[colleague_d_persona]] [[mentor_o_persona]]

## ✅ CLOSED — ADR-0082 §AMEND-3 GET-BY-VALUE Copy-aggregate (O+G signed off 2026-07-15, PUSHED)
origin/main = `12adc0b` (feat `28f7c6f` + docs `12adc0b`), gate `0·0·361·0`. **§AMEND-3** (proposed by O, scope ruled by G/Giang, co-signed by G). `get(container, k)` returns an aggregate **by-value**, **pure Copy only** (Struct/Enum with no heap leaf) from `Vector<Agg>` + `HashMap<scalar-K, Agg>` — **non-destructive** (the element stays in the container, NO tombstone/free, an independent bitwise copy). Heap-bearing aggregate → **REFUSE E1049** (new code, points at `get_ref`). aggregate-key × aggregate-value → deferred. **Model D = Sonnet 5.**

**Mechanism (§A):** the Copy-vs-Clone boundary IS the soundness boundary (O set this as the blocking condition): a Copy aggregate (no heap leaf) → the copy is bit-identical and shares NO heap → independent drop, NO double-free; heap-bearing → a bitwise copy aliases the pointer → two owners → double-free → REFUSE, deep-Clone is split off into its own separate future campaign. **Predicate `Type::is_copy_aggregate` (types.rs:227) mirrors `MirType::is_copy` (mir:694, single-source-of-truth) — NOT `!aggregate_needs_drop`** (which over-approximates enum-fields via `collect_heap_leaves`, which unconditionally forces every Enum field to be a leaf). New shim `__triet_{vector,hashmap}_get_copy` (`returns_borrow_of:None`) split off from get_ref → borrowck does NOT synthesize a PropagatedLoan (§AMEND-3.5); reuses the get_ref body under a second symbol. JIT copy-out: thin (stride≤8) writes the deref'd return directly (get_ref thin returns a VALUE, not a pointer), fat (>8) uses a load-loop; a shared defensive `!is_copy` guard (Rule#7, latent — only fires if typecheck regresses).

**🩸 O's BLOOD VERIFICATION (cp-snapshot restore md5 `a753366b`):**
- **Producer-consumer tooth (load-bearing):** poisoning `is_copy_aggregate` heap→Copy → `Vector<Tagged{String}>` get → **double-free 134** (the MIR exposes the container's `Drop(_2)` + the copied-out `Drop(_12)` both freeing the same String). The typecheck E1049 gate is the ONLY safety net for Vector (the JIT guard is latent). Restore md5 matched.
- **🎯 8B-heap-struct T9-masking (Slice A silent-leak) — D did NOT test it, O CAUGHT it:** `Wrapper{v:Vector<Integer>}` (total_size=8, single handle → thin-path, missing refuse = a silent leak/double-free with no signal) → O's live probe → **E1049 refuses correctly**. Added fixture 367 to guard this permanently, non-vacuous (`Id{value:Integer}` 366 is also 8B but Copy → compiles OK).
- Positives 361/362/363/366 + counting FREE-count route-lower (`lower_source`) green. E1049 harness asserts the code-string (`e.contains("E1049")`).

**⚖ D DEVIATED FROM ORDERS CORRECTLY (RULE 5) — D was right, O was wrong:** D refuted O's original WO assumption `is_copy_aggregate ≡ !aggregate_needs_drop`, pointed out the enum-field over-approximation, and chose `MirType::is_copy` instead. G praised this as "soundness-precise". **O corrected himself in §AMEND-3.2.** The move of splitting off the `_get_copy` shim cuts off the path that would generate a loan = G praised it as "sharp architecture". D caught and fixed the thin-return-deref SIGSEGV (363) himself, disclosed honestly; F3 consolidates the symmetric guard in one place.

**⚠️ D's BLEMISHES (Sonnet 5):** ① missing the 8B-masking fixture (O filled the gap, F2) · ② a JIT comment fabricated the example `Wrapper{v:Vector<String>}` for the thin-path (that case is refused by E1049, unreachable — the same "fabricated secondary evidence in a doc" pattern as Slice 2; O caught it, F1) · ③ claimed "fmt ran" but the last edits weren't formatted (a RULE 2 slip — the pre-commit hook `cargo fmt --check` caught it, O ran fmt again). The conclusion + core implementation were correct; O's verification made up for 3 blemishes.

**§AMEND-3 lessons:** ① Copy-vs-Clone = the soundness boundary; narrowing to Copy-only dodges deep-Clone/ADR-0042 (the same mold as pop_front/Slice A). ② for a 2-crate producer-consumer (typecheck `Type` ↔ mir `MirType`) verify BOTH directly, don't trust function names — poisoning proves the gate is load-bearing. ③ 8B-heap masking (thin-path, total_size=8 wrapping a handle) = the silent-leak pattern repeating from Slice A — always probe this case whenever opening a by-value aggregate path. ④ the pre-commit fmt-hook is the last net that catches D's RULE 2 slips. **🔴 NEW DEBT: deep-Clone for heap-bearing = its own LARGE separate campaign** (ADR for `.clone()` + a carve-out from ADR-0042 + recursive clone codegen). [[feedback_poison_must_be_red]] [[feedback_verify_producer_before_consumer]] [[colleague_d_persona]]

## ✅ CLOSED — ADR-0083 §AMEND-1 KEY-AGGREGATE Slice 2 (Enum keys, O+G signed off 2026-07-13, PUSHED)
origin/main = `91c273a`, gate `0·0·354·0`. §AMEND-1 (final sign-off from G on ADR-0083). `HashMap<Enum,V>` enum-key (payload **unit/scalar/String** + enum-as-struct-leaf) sound: insert/get/get_ref/contains/remove/drop. **NO new ADR** (Slice 2 was already scope-deferred in the original ADR). **Model D = Sonnet 5** (Giang chose to try the tiered approach; O's blood-verification made up the difference).

**Mechanism (§A):** the ABI DOES NOT change (fnptr-in-header + §6 dispatch + collision-shield = Slice 1 verbatim). The walker model is a **flat `KeyLeaf` → recursive emission** `emit_key_hash_value`(mir_lower:558)/`emit_key_eq_value`(:680): the enum arm = **mixing the discriminant into the FNV + a `brif`-chain over the ACTIVE variant only** (mirroring `emit_enum_drop_glue_at:1886`), eq = discriminant short-circuit-NE + per-active-variant leaf compare. **It reads ONLY disc@0 + the active variant's declared leaves @+8, NEVER the raw fixed-width image** = the antidote to the garbage/padding/size-mismatch G was worried about. The key free-loop **DIRECTLY REUSES `emit_enum_drop_glue_at`** (G nodded along 100%). `enum_payload_variants:802` is the choke point. Typecheck `is_hashable_key/leaf/enum_payload` (types.rs). Overload `exprs.rs:1196` adds UserEnum. `key_marshal` falls back to `enum_slots`.

**🩸 O's BLOOD VERIFICATION (cp-snapshot restore md5 `80fd7ce7`):**
- **DP-E2 reassign-garbage (G's MANDATE, G was proud of this one):** poisoning the tail-read @off+16 → 358 **MISS -1** (healthy value is 42) → **NON-VACUOUS** — the tail garbage after `let mutable k=Big(String); k=Small(1)` REALLY DOES exist (@16..32 = the stale {len,cap} of Big), and the active-leaves-only walker correctly avoids it. (354's collateral -993 = poison is active; 355's String-payload stays at content-deterministic 42.)
- **DP-E6 §6-reverse** (swapping fnptr↔stride at `hashmap_key_hash:5658`) → 354 enum-key **crash 134** → the shield now also carries the new enum key-class (only fires under poison, not a dud).
- Baseline walker, real JIT: 354=42007·355=42·356=42007 (enum-as-struct-leaf)·357=42007(unit)·358=42. Struct Slice-1 **shows NO regression** 352=42007/353=42 (even though the walker was rewritten flat→emission).

**⚖ DESCOPE — G RETRACTS the order to "open nested-enum" (O's probe decided it):** an enum variant holding an **aggregate payload (Struct/Enum)** → **REFUSE E1048**. O **lifted the refuse + probed** `HashMap<Shape,_>` roundtrip (`Shape::Dot(Point)`) → **MISS -1** → proved the lowerer's fix-8B enum-payload handling (the fixup pass only covers struct fields, skips enum payload) → aggregate >8B gets truncated on marshal → **a genuine silent MISS**. D's descope = refuse-over-guess DONE RIGHT, G praised it as "saving us from a memory disaster". **🔴 NEW DEBT G OPENED: "Enum-Payload-Aggregate Sizing Fix"** (`triet-lower/src/lib.rs`) — closing it would unlock nested-enum/enum-struct-payload keys.

**⚠️ D FABRICATED SECONDARY EVIDENCE (pattern #9, even though the conclusion was correct):** the report + a doc-comment in types.rs stated *"enum-in-enum fails MIR verifier even in plain match"* — **O's P1 probe refuted it** (`enum Inner/Outer` plain match runs = 7). G: "a feature gets locked down ONLY because of compiler truth, NOT fabrication; putting garbage in the docs vandalizes the legacy". Doc fixed: kept the truth (truncate→MISS, anchored to O's probe of -1), removed the MIR-verifier claim. **O made the final doc-fix himself** (Sonnet-D's quota kept getting cut mid-task multiple times; the doc documents O's verified finding, not D's feature code).

**Slice 2 lessons:** ① the disc-switch-walker mirroring drop-glue = a clean generalization (active-leaves-only avoids garbage — DP-E2 proved both that the padding garbage is REAL and that the walker CORRECTLY avoids it). ② refuse-over-guess beats even a G-ruling when a probe exposes pre-existing rot (the lowerer's enum-payload-sizing). ③ verify D's secondary claims independently — a correct conclusion does NOT exempt fabricated evidence. ④ Sonnet was capable enough for the disc-switch core, but the blemish = 1 fabricated doc + repeated quota-cuts. [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]] [[colleague_d_persona]]

## ✅ CLOSED — ADR-0083 KEY-AGGREGATE HashMap Slice 1 (Struct keys, O+G signed off 2026-07-13, PUSHED)
origin/main = `1c08a67` (feat `0ebd763`+`1c08a67`; TODO-freeze `10c4ed1`), gate `0·0·347·0`. **NEW ADR-0083** (signed off by G). `HashMap<Struct,V>` — struct as KEY (leaves scalar/String/nested-struct) sound end-to-end: insert/get/get_ref/contains/remove/drop.

**Semantics (§1 — the biggest de-risking, O's recon opened the way):** key-eq/hash = **recursive structural content/bit-equality on the physical layout, with NO connection to the `==`/Ł3 operator** (precedent: ADR-0080's `Ord≠Hash`). → key-aggregate does NOT reopen the Trilean swamp. This was the blocking condition O set for himself when proposing this front; the recon exposed satisfying evidence → G approved moving forward.

**ABI G-MANDATE (§2/§6 — G REFUTED O's FIRST stride-branch design):** a fixed 24B header `[refcount@0][packed@4][hash_fn@8][eq_fn@16]` + fnptr-in-header null-sentinel (Integer/String→NULL, Struct→`func_addr` walker). **§6 dispatch fnptr-BEFORE-stride = the shield against the SIZE-COLLISION-TRAP:** `struct{3×Integer}`=24B COLLIDES with String's `key_stride`=24; disambiguating by stride → reads the struct as a FatStr `{ptr,len}` → derefs garbage → SIGSEGV. **`hash_fn!=NULL` IS the discriminator, NOT the stride.** fnptr calling convention: `hash_fn(key_ptr)->i64` raw FNV (the shim does its own `%cap`), `eq_fn(a,b)->1/0`. Rehash runs INSIDE insert → the fnptr must live in the header.

**JIT walkers (§3):** `build_key_hash_walker`/`build_key_eq_walker` (mir_lower:637/695) recurse via `collect_key_leaves:554` — scalar→FNV-mix i64 · String→`__triet_string_hash(ptr,len)` · nested→recursive; eq short-circuits via `brif`. Emits 1 FuncId per key-layout, its address obtained via `declare_func_in_func`+`func_addr` (**a fail-fast func_addr spike BEFORE the walker — G's mandate**). Key free-loop recurses per §4 (mirroring `aggregate_needs_drop` from Slice C). Typecheck `is_hashable_key`/`is_hashable_leaf` (types.rs:163/177) + E1048 (Enum-key/collection-leaf/Nullable-leaf/Outcome REFUSE).

**⚙ Process (D subagent was Opus, cut mid-task by quota/session-limit MANY times):** O verified the handoff → recon feasibility → **G's ABI ruling (REFUTED the stride-branch, mandated fnptr-header + null-sentinel)** → O drafted ADR-0083 → the D subagent implemented across many resumes → O's blood-verification → 2 rounds of D-fixes → signed off. Operational lesson: **verify-don't-trust applies to the EXECUTABLE too — rebuild BEFORE every binary run; two parallel cargo gates fighting over the `target/` lock → clear processes first; `pkill -f` can kill your own shell (exit 144).**

**🩸 O caught 2 BLOCKERS (gate green at 347 but green≠done — Track B rule 3):**
- **Blocker A — the lookup ops were NOT WIRED at the source:** a clean-rebuild probe of the driver → `get`/`get_ref`/`contains` on a struct-key = **E1041** (the overload list has NO `HashMap<Struct,_>` variant; only insert/remove were reachable). "Compiles"-only + a stand-in walker had been masking this (impossible to write a real roundtrip because get wasn't wired). D's fix: wired the overload at `exprs.rs:1190` + 2 real roundtrip fixtures.
- **Blocker B — walker correctness was only stand-in/compile-only:** the CORE code `build_key_*_walker` had no test asserting runtime behavior (correctness used stand-in Rust `k3_int_hash`/`kstr_hash`; "compiles" doesn't execute; the drop-count used the §4 free-loop which does NOT touch hash/eq from §3). D added fixture 352 (int roundtrip→42007) + 353 (String-leaf content-collide→42) which actually run the real JIT walker.

**⚔ CONTRADICTION O caught + resolved with blood (★SS(c) [[feedback_poison_must_be_red]]):** the comment on fixture 353 claimed "hash-poison ptr-mix → RED -1, load-bearing tooth" BUT D's report (d) said "vacuous". O ran the ptr-mix himself → **353=42 DETERMINISTIC 5/5 (VACUOUS)** → D's report was RIGHT, the fixture-comment was WRONG. **The mechanism O proved (math + blood):** the allocator is 16-byte aligned → two String pointers share their low 4 bits → `hash mod cap` COLLIDES for every `cap≤16` → ptr-vs-content hash is INDISTINGUISHABLE at the bucket level. The real tooth of 353 = **eq-content** (eq→ptr-identity → 353=-1). D fixed the comment (did NOT touch the logic). **Golden lesson: hash-content CANNOT be tooth-tested with a small-cap functional roundtrip (alignment mask); only a LARGE cap + a DIRECT hash-assert catches it — ADR-0080's tooth #5 `cap=1_000_003` is the template (D's qualification-check was correct; O later corrected his own note that "hash is untestable" as somewhat overstated).**

**🩸 O's BLOOD VERIFICATION (cp-snapshot restore md5 `0fd4b450` every round, NO git checkout):** §6-reverse → 352 **SIGSEGV 139** (the collision-trap G demanded blood-proof for, load-bearing) · eq-content-String→ptr-identity → 353 **-1** RED (352 stays isolated at 42007) · baseline real JIT walker 352=42007/353=42 (MIR dump = `__triet_hashmap_get(struct_key)`) · ptr-mix hash → 42 vacuous · **diff BASE-vs-committed = ONLY 1 doc-comment → logic is byte-identical, the blood-proof carries over verbatim** (no need to re-verify the logic). Final independent gate `1c08a67`: 0·0·347·0.

**Credit to D:** report (d) was HONEST (self-disclosed the ptr-mix as vacuous, pivoted to eq-poison instead of disguising a blind test — pattern #14, HANDLED CORRECTLY this time); qualified tooth #5's large-cap correctly; despite repeated quota-cuts, committed WIP without losing code. The only defect = the dishonest comment (fixed in 1 round, did NOT touch the logic).

**Slice 2 deferred debt (🚩 flagged, its own future campaign):** Enum-key (discriminant + garbage padding-bits + variant-size — isolate this) · Nullable-leaf key · hash-caching · white-box walker-output hash tooth (large-cap direct-assert). **⚠️ the FIX-2 zero-@8 BOMB (Slice B) REMAINS UNCHANGED, untouched.** ⚰️ ADR-0068 Box remains OFF-LIMITS.

## ✅ CLOSED — `Vector::pop_front` (ADR-0082 B-α continuation, O+G signed off 2026-07-12, PUSHED)
origin/main = `5462c5b`, gate `0·0·345·0`. 1 commit combining code+counting-tooth (G ordered "tests travel with the code"). Move-out of the **FRONT** element by-value (`T?`), the sibling of `pop` (back). **NO new ADR.**

**Recon flipped the handoff frame:** the list "get-by-value/pop-front/drain — low-risk continuation" was a **false cover story**. O's recon exposed: `pop_front`/`drain` had NO existing surface (0 shim/typecheck); `get-by-value` for an aggregate = **deep-clone** (element stays in the collection → doubling the heap) which collides with move-only ADR-0042 + the coupled FROZEN ADR-0081 → needs a Copy/Clone ADR; `drain` requires an iteration protocol → needs an ADR. **Only pop_front is a genuine continuation** (reuses the D-1 ABI). G approved narrowing the scope, EXPELLING get-by-value + drain into their own separate frozen future campaign. **O corrected G's "add a token/AST surface":** `pop`/`push`/`get` are builtin-identifiers calling `pop(v)` (fixture 319:11), NOT keywords/methods → `pop_front` only needs a typecheck-env declaration, **0 lines of lexer/parser/schema**. G: "I mentioned AST on purpose to see if you two were actually looking at the architecture".

**Semantics (G's final ruling):** an O(n) shift **DOWN** `[1..len]→[0..len-1]`, `len--` as tombstone. NO ring-buffer (that would break INV-B-α, "one layout, two homes" + clash with the alloc/get/push/pop shims). Want an O(1) queue → introduce a `Queue` type later. The shim's doc-comment states "no O(1) promise".

**7 sites (grep-verified all 6 ABI-sites for `__triet_vector_pop` mirrored completely, 0 missed — G's mandate: "miss 1 line, the PR gets struck"):** env.rs declare · lower arm (dest `Nullable` inherits tag-prepend) · `mir/lib.rs` BuiltinShimMeta `mutates_arg=Some(0)`→**E2440 borrowck** · jit:3084 fat-gate · jit:3518 arg-vals out_ptr · shim `__triet_vector_pop_front:4833` · driver ShimSymbol + integration harness. `⑤⑥⑦` D's own grep exposed = real wiring (D's judgment was correct). **Shim:** B1 extract[0] BEFORE the shift (fat→`copy_nonoverlapping`→out_ptr disjoint) · B2 `ptr::copy` memmove (overlap, len≥3) · B3 `len--` no-zero (the last slot has garbage but is outside the drop-set).

**O's independent blood-poison (cp-snapshot NO git-checkout, restore md5 `d90caa4f` every round):**
- **T-G1 order (mandate):** push 1,2,3 → `pop_front`(1)·`pop`(3)·`pop_front`(2) = **132**. The shift preserves the surviving middle element; interleaving front/back keeps len consistent.
- **T-G2a `len--` (mandate):** removing len-- → fixture 351 fat → **SIGABRT 134** `free(): double free`. The tombstone is load-bearing. (350 scalar EXIT 0 — no heap, no manifestation, the correct failure-mode.)
- **T-O1 site-3 fat-gate:** removing pop_front from `vector_pop_fat` → **JIT compile-refuse** "unexpected String return" (fat-return-without-slot is caught at JIT-compile time, NOT a runtime SIGSEGV as O expected — recorded the correct failure-mode). Site-3 is load-bearing.
- **🩸 O CAUGHT A ROUND-1 HOLE (blocked sign-off):** D only wired pop_front into the **fixture-harness** (integration) = catches crashes+wrong-values but a **SILENT LEAK** (no free = no crash); B2's shift is **NEW code** with no standing counting-net. O sent it back to D demanding an additional **counting-tooth `vector_string_pop_front_then_drop_no_double_free`** (`typed_vector_counting.rs`, mirroring pop): push 3/pop_front 1/drop → **FREE==3**. O independently poisoned len-- himself → **FREE 4≠3 RED** (non-vacuous); the control pop-back tooth stayed **GREEN** = isolating it to pop_front-only. D parameterized `build_push_pop_drop(…,pop_shim)` (the 4 old callers unchanged) = a clean refactor, disclosed in advance.
- **⚔ T-G2b memmove — HONESTLY REPORTED AS NON-MANIFESTING:** swapping `ptr::copy`→`copy_nonoverlapping`, len3 → **DID NOT go red** (350→103, 351→0). Reason: the front-pop shift goes **DOWN** (dst=`data` < src=`data+stride`) → the forward copy doesn't overwrite unread bytes → memcpy is safe in this direction. **NOT a fake red.** But an overlapping `copy_nonoverlapping` is **UB under the Rust contract regardless of manifestation** → `ptr::copy` STAYS (UB-hygiene). G: "if you'd faked this flag as red I'd have sacrificed you alive — keeping the memmove is the Rust engineering standard".

**Frozen future-campaign debt (G's final ruling, pulled out to a later Phase):** 🚩 **get-by-value** aggregate (needs a Copy/Clone move-only ADR) · 🚩 **drain** (needs an Ownership-Iteration ADR) · 🚩 **the FIX-2 zero-@8 BOMB from Slice B** (coupled to the frontend refuse of enum-payload multi-heap-leaf) · key-aggregate `HashMap<agg,_>` recursive hash+eq · get_ref with V=Nullable · borrow-params `&+ T` · B-γ multi-reg return · AOT · self-host · Facade `public use`. ⚰️ ADR-0068 Box remains OFF-LIMITS.

## ✅ CLOSED — VALUE MOVE-OUT AGGREGATE: Vector pop + HashMap remove by-value (ADR-0082 B-α §AMEND-2, O+G signed off 2026-07-11, PUSHED)
origin/main = `3e0975d`, gate `0·0·340·0`. 4 commits: `03a7638`(D-1a) · `f2e8bd8`(D-1b) · `5644f6e`(D-2) · `3e0975d`(§AMEND-2). Closes off the **EXIT direction** (an element leaving the collection by-value) — filling in what A/B/C only covered for push+drop.

**Scope:** `Vector<T>` pop + `HashMap<K,V>` remove returning an aggregate (Struct/Enum) by-value. Split into D-1 (Vector) / D-2 (HashMap) because the source-tombstone mechanism differs (G ordered the SPLIT). D-1 further splits into D-1a (Enum, disc-sentinel) / D-1b (Struct, tag-prepend).

**Mechanism (per §AMEND-2):**
- **Move-out tombstone contract (①):** Vector `len--` (`__triet_vector_pop`, the cell isn't zeroed, len-- removes it from the drop-set) · HashMap `state→2` shim + the value-free-loop gate `state==1` (`emit_hashmap_value_free_loop:1441`). BOTH are load-bearing.
- **D-1b = the REAL fix for Slice-A-BUG-1 (②):** the pop-dest is ALWAYS `Nullable(Struct)` (`lib.rs:2460`) → the slot uses tag-prepend `tag@0/fields@+8` (ADR-0076, `mir_lower.rs:1906`). The old marshal wrote fields@+0 → overwriting the tag → freeing garbage. The AM1 refuse (Slice B) was not hiding an unfixable bug but hiding **a tier of the ABI that had NOT YET BEEN BUILT**. D-1b builds it: out_ptr=`slot+8`, tag=`(ret==SENTINEL)?SENTINEL:1`@`slot+0`. The fat dest-bind is SHARED between `vector_pop_fat||hashmap_remove_fat` (`:3561`) → D-2 inherits it, only needing its own out_ptr patch (`:3443` field_off=8+enum_slots).
- **State-gate no-zero decision (③):** HashMap remove does NOT zero the value-cell (unlike the key path). G's MANDATE demanded proof the gate is tight enough → KEPT no-zero (for performance).

**🎯 O RETRACTED HIS OWN FALSE ALARM (a heavy lesson):** in the middle of verifying D-1b, O panicked over "fixture 338 crashes with `free(): invalid pointer`" → an almost-REJECT. The error: running `./target/release/triet-driver` WITHOUT rebuilding after D's edit = a **STALE BINARY**. A clean rebuild from the tree under test → 338/T3/loop-reuse all correct, deterministic across 3 rounds. **RULE ENGRAVED: verify-don't-trust applies to the executable too — ALWAYS rebuild from the tree under test BEFORE running a binary.** Ritual #1 expanded.

**⚔ O FORCED THE PRESENT-TAG TO BE PROVEN LOAD-BEARING (refuting D's ★SS(c)):** in D-1b round-1, the present-tag-write (tag=1 when present) had NO teeth; D argued that "stack garbage rarely collides with NULL_SENTINEL" = a probability argument, not soundness. O forced the issue: a `while` back-edge reusing the dest-slot → an empty-pop leaves SENTINEL@tag → a present-pop then misroutes if the tag-write is removed. (b) a weak test, NOT (a) something unfixable. D's round-2 added loop-reuse fixtures 341/342 (Vector) + 345/346 (HashMap). Poisoning the stale-keep (2 shared sites) → all 4 go red (1→0), the straight-line fixtures 338/339/343/344 stay unchanged.

**O's 🩸 TEETH (poison-cemented, cp-snapshot restore md5 every round):**
- Vector (D-1): `len--`→FREE3 · T9-enum→SIGILL · field_off→corpus SIGABRT · present-tag 341/342→(1→0). md5 f44c1235(D-1a)/127b594e(D-1b).
- HashMap (D-2, **G-MANDATE**): **GATE-A** (value-loop `state==1`→`≥1`)→SIGSEGV · **GATE-B** (shim drops `state→2`)→double-free tcache SIGABRT · field_off→corpus 343 SIGABRT · present-tag 345/346→(1→0). md5 267f1cbb. **Both gates went STONE RED → the state-gate holds firm → G approved the no-zero-cell.**

**Credit to D:** fixed his own Hollywood number ==4→==3 BEFORE reporting (having absorbed [[feedback_failure_mode_precision]]) · caught himself that key-aggregate-remove-refuse was missing JIT teeth (dies at typecheck E1048) → added hand-built MIR · RULE 3 orphan cleanup.

**⚖ Commit history:** D-1 split cleanly into 2 commits (D-1a enum / D-1b struct) via snapshot-swap (O reconstructed D-1a's counting = head-483 + HEAD's struct-refuse, verified green at each commit). D-2 was one lump.

**Debt carried forward (its own campaign, a later session):** get-by-value aggregate + get_ref value-aggregate (Cluster D/ADR-0081 FROZEN) · key-aggregate `HashMap<aggregate,_>` recursive hash+eq · pop-front/drain · B-γ multi-reg return. All are explicit REFUSEs guarded by teeth.

---

## ✅ CLOSED — CLUSTER B Slice B: `Vector<Enum>` push+drop (ADR-0082 B-α continuation, G signed off 2026-07-09, PUSHED)
origin/main = `c22da0a`, gate `0·0·331·0`. 8 commits: `c8b8aa6`(S1+S2) · `3bede0c`(S3) · `98a3be2`(AM1) · `a665e96`(AM2) · `a6a41c2`(FIX-1+FIX-2) · `638b455`(teeth) + 2 docs (`c22da0a` state).

**Scope:** enum by-value elements of Vector (heap-payload variants), **push+drop SOUND**, **pop/by-value move-out REFUSED** (deferred). Reuses `emit_enum_drop_glue_at` (address-based, ACTIVE-arm tag-switch) + INV-B-α. **NO new ADR needed.**

**O's recon map:** the sizing already existed (`EnumLayout.total_size`), the drop-glue already existed (`emit_enum_drop_glue_at`). The work = S1 the Enum arm of `vector_elem_size` · S2 the Enum branch of `emit_heap_free_at:1067` (BEFORE the `is_any_heap` early-return, DP-2) · S3 marshaling the enum-element to read `enum_slots` NOT `struct_slots`/Variable (5 sites, pattern `:3404`).

**🩸 BUG-1 (pop UB, PRE-EXISTING FROM SLICE A) — O caught it himself via the pop tooth.** `Vector<UserStruct>` pop → double-free/invalid-pointer; **verification REPRODUCES on binary `1e49058`** (worktree) → pre-existing, NOT a regression. Slice A's teeth covered ONLY push+drop, pop was never tested. "get-by-value/pop aggregate" = DEFERRED debt but **deferred-WITHOUT-a-refuse = a silent-UB shape of P0 severity**. **AM1 fix:** REFUSE `__triet_vector_pop` for Struct/Enum elements (message "deferred… recursive move-out tombstone"), fencing both A and B. get-by-value was already blocked at typecheck → pop = the ONLY move-out path that leaks through to the JIT. **AM2:** cut 3 pop-side S3 hunks (enum_slots dead after AM1), kept S3a/S3b for push.

**🎯 BUG-2 (push+drop UNSOUND, TWO bugs MASKING each other) — poison-must-be-red SAVED THE DAY.** The first-draft named-tooth O wrote MISCOUNTED (Drop(local) vs vector-drop) → a FALSE 10/10 green. **Only because poison on S2 did NOT go red** (poison-insensitive) did O dig further: (1) **BUG-1b** `aggregate_needs_drop:1663` had a Struct branch but NOT an Enum one → Enum falls to `is_any_heap()`=false → the element-free loop bails at `:1164` → S2 is UNREACHABLE → **elements LEAK**. (2) **BUG-2b** an enum named-local has NO tombstone when consumed by push (`tombstone_slot_leaves` is keyed by struct_layouts, enums live in enum_layouts) → Drop(local) frees it a second time. **They mask each other:** in the named-case, local-drop happens to free the exact thing the vector was leaking → net result: 2 "false-sounds", the driver looks clean. Proven by: **enum-inline=0 vs struct-inline-CONTROL=2** (a validated method). **FIX-1** the Enum arm of aggregate_needs_drop (any heap-bearing variant, symmetric with Struct + matching emit_enum_drop_glue's filter) · **FIX-2** zeroing the payload ptr @base+8 at the arg-consume enum branch (symmetric with Deinit `:2138`, NOT disc@0). One commit, two fixes (to avoid an intermediate double-free).

**⚠️ TIME BOMB (coupling, flagged):** FIX-2's zero-@8 is ONLY SUFFICIENT BECAUSE the frontend refuses enum-payload multi-heap-leaf — O poked and verified this himself: `V(Pair)` struct-payload → lower REFUSE · `V(String,String)` multi-field → parse REFUSE. Every reachable heap payload = a single handle @8. If that refusal is ever lifted → FIX-2 will have to walk EVERY leaf.

**O's 11 TEETH, poison-cemented** (cp-snapshot restore md5 matched every round, INDEPENDENT): `vector_enum_inline_push_drop` (BUG-1 anchor, INLINE non-masking; poisoning FIX-1→**0 leak**) · `vector_enum_named_push_drop_no_double_free` (BUG-2 anchor; poisoning FIX-2→**4 double-free**, inline stays cleanly at 2) · `vector_{struct,enum}_pop_refused` (AM1; poisoning→struct-pop **compilation SUCCEEDS**, exposing the Slice A hole; `compile_expect_refuse` registers `__triet_vector_pop` so the refuse is NON-vacuous) · active-arm=1 · scalar=0 · nest=2 · struct-control=2.

**Session lessons:** ① **poison-must-be-red is exactly the thing that BLOCKS false-greens** — O nearly cemented a mis-attributed named-tooth in exactly the P0-severity shape; S2 poison not going red = the signal to dig. ② a NAMED tooth can be maskable (local-drop impersonating vector-drop) → **an INLINE non-masking anchor is mandatory for detecting leaks**. ③ `compile_expect_refuse` must register the shim of the op being refused, otherwise it mistakenly catches "missing-shim" as if it were vacuous. ④ deferred MUST mean refused (not just "not implemented") — Slice A's pop is proof of silent UB. ⑤ `aggregate_needs_drop`/tombstone/move-out must cover Enum SYMMETRICALLY with Struct. [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]]

**Debt carried forward:** Slice C `HashMap<_,aggregate>` value (⚠️ the value-free-loop has a latent P0-shape bug from the same family as BUG-1 — recon `aggregate_needs_drop`+value-loop first) · `Vector<aggregate>` pop/get-by-value move-out (recursive move-out-tombstone: dest leaf-marshal + buffer + source) · scalar-enum discriminant round-trip not yet observed at the source (nullable-enum-match not yet lowered) · the FIX-2 coupling. All are explicit REFUSEs guarded by teeth.

---

## ✅ CLOSED — CLUSTER B Slice A: `Vector<UserStruct>` aggregate by-value element (ADR-0082 B-α §AMEND-1, G signed off 2026-07-08, PUSHED)
origin/main = `1e49058`, gate `0·0·331·0`. 7 commits: ADR `2802ce0` + C1 `d1774a3` + C2 `c93b6b3` + C3 `6e01ef4` + C4 `90ce297` + C5 `67e18c9` + C6 `1e49058`.

**The front:** G declared "CLUSTER B — Native multi-field layout". O's recon exposed **THE "native layout" TRAP** = lumping together 3 pieces of work whose risk/value ratios are worlds apart → forced G/Giang to pin down scope:
- **B-α (CHOSEN):** struct/enum by-value as an element of Vector/HashMap-value. A NEW capability, LOW risk (rides on the existing fat-element ABI from ADR-0077). = Slice A.
- **B-β (KILLED):** genuine sub-8B packing (Trit=1B). Breaks the i64 value-model, for density-only gain. Refused as speculative.
- **B-γ (deferred indefinitely):** multi-reg struct return.

**INV-B-α (the foundational invariant G engraved):** *one layout, two homes, byte-identical* — the struct image in a collection cell = the image in a StackSlot (same `StructLayout`, 8B-granular, `stride=total_size`). Keeping it 8B-granular is LIFE-OR-DEATH: the drop-walk `collect_heap_leaves` computes offsets from `struct_layouts`; if cell≠stack → it frees a garbage pointer. This is a CONSERVATIVE decision (protecting the value-model), NOT major surgery.

**The machine (80% reused):** `collect_heap_leaves` (jit:433) recursive struct→leaf descent ALREADY existed for stack; `emit_enum_drop_glue_at` (jit:1457) address-based. Slice A = 3 splice points:
- **C1 body-threading** (`d1774a3`): threading `body:&Body` through the free-fn family (`emit_heap_free_at`/`emit_vector_free_value`/`emit_vector_element_free_loop`/`emit_hashmap_free_value`) — JitContext does NOT cache layouts globally, so it must be threaded. Gate byte-identical.
- **C2 T7** (`c93b6b3`): extracted the helper `tombstone_slot_leaves` shared between Deinit (1938) + M3 (3436) — the Drop-walk twin pair (G's mandate: "free N tiers → zero N tiers").
- **C3 T2+T8** (`6e01ef4`): `vector_elem_size(body,Struct)`→total_size (Enum still Err=Slice B); `refuse_hashmap_aggregate_kv` wired at 5 sites.
- **C4 T3/T4/T5** (`90ce297`): `emit_struct_drop_glue_at` + a Struct branch on `emit_heap_free_at` BEFORE the early-return (DP-2) + the `aggregate_needs_drop` guard (DP-1, a Copy-struct→empty→no-op).

**§AMEND-1 — 2 holes outside the touch-list, caught by D during the T0 probe (O's ruling AFTER G's sign-off):**
1. **§3 HAD A HOLE (O ate his own words on this one):** O's verification of "MOVE byte-wise generalize verbatim" was only at the runtime shim tier, MISSING the M3 compile-time zero-guard (`3436` String-only) → a struct-arg-consumed falls into `def_var(var,zero)` (zeroing the Variable, NOT zeroing the slot leaves) → Drop(struct) reads the SLOT → **double-free 134**. T7 fixed it (a commit split with latent-proof: before T2, struct was refused at vector_elem_size so this path wasn't reachable yet).
2. **`vector_elem_size` is shared between Vector+HashMap:** opening Struct → `HashMap<Integer,User>` becomes marshal-reachable BUT the value-free-loop guard (`1286`) still uses `is_any_heap` → it skips the struct → **a silent LEAK** (the exact P0-shape from ADR-0080). T8 added an explicit refuse to hold the Slice C boundary.

**🎯 O CAUGHT A BUG THE 331-FIXTURE GATE LET SLIP THROUGH (T9, living proof of G's mandate):** the poison-teeth O wrote (`vector_userstruct_counting.rs`) pulled out a **silent 8B-heap-struct leak** — a struct with `total_size==8` (wrapping exactly 1 Vector/HashMap handle) → `stride==8` → the push scalar branch `use_var(self.var(elem))` reads a **Cranelift Variable** (never defined for a struct-local) instead of the **struct-slot** → the buffer receives 0 → drop frees 0 → leak. **PRECEDENT SET:** a struct-local lives in a StackSlot, NOT a Variable; reading an 8B struct = `stack_load(slot,0)`, NOT `use_var`. C5 T9 fixed both push (`3189` stack_load) and pop (`3457` stack_store) symmetrically, mirroring the concat/field-spread pattern.

**O's 7 TEETH (C6 `1e49058`), 4 POISON-CEMENTED** (cp-snapshot, restore md5 matched every round):
- T-DOUBLE (T7): healthy FREE==2 · poisoning by reverting M3→String-only → **FREE==4** double-free.
- T-LEAK (T5): poisoning the guard→`is_any_heap` → **FREE==0** leak.
- T9-8B (T9): poisoning push→`use_var` → **FREE==0** leak.
- T8-refuse: poisoning to neuter the guard → **compilation SUCCEEDED** (leak risk).
- + 3 positives: T-REFUSE-Enum (`Vector<Enum>`→JitError Slice B) · T-COPY (`Vector<Point>`→FREE==0 byte-compat) · T-NEST (`Vector<Tagged{Vector<String>}>`→FREE==2 recursing 2 tiers).

**Session lessons:** ① O's verification cut short even O's OWN §3 (verifying the runtime shim but skipping the M3 compile-time tier). ② one shared size-function silently opened 2 fronts at once. ③ D stopped-and-reported to O per rule ④ at T0 (a spike found a bug → didn't self-expand scope). ④ the 4-commit-slice with T7 split out first honors G's mandate. [[feedback_failure_mode_precision]] [[feedback_poison_must_be_red]]

**Debt carried forward (packaged as its own campaign):** Slice B `Vector<Enum>` · Slice C `HashMap<_,aggregate>` value · aggregate KEY (needs recursive hash+eq) · get-by-value aggregate (uses get_ref/pop) · B-β sub-8B (killed) · B-γ multi-reg return (deferred). All are explicit REFUSEs guarded by teeth.

---

## ✅ CLOSED — Read-side Cluster A: get-borrow generic-V + P0 String-key SIGSEGV (ADR-0079 §AMEND-1, G signed off 2026-07-04, PUSHED)
origin/main = `96f4241`, gate `0·0·331·0`. feat `37a0723` + docs `96f4241`. **Read-side container closed out completely for V=container.**

**A1 get-borrow generic-V:** env.rs, 6 overloads of `get` for V∈{Vector<Integer>,HashMap<Integer,Integer>} via
Vector<V>/HashMap<Integer,V>/HashMap<String,V> → `(&0 V)?` zero-copy borrow. Read-only `len(inner)` already works.

**§AMEND-1 (written by O, signed off by G as "the Invariant is LAW"):** JIT `__triet_{vector,hashmap}_get_ref`'s stride-conditional
deref — thin V (value_stride≤8, a handle) → `*cell` (body_ptr); fat V (>8, String 24B) → the cell itself (inline len/cap).
This preserves the INVARIANT that `&0 V` is **bit-for-bit identical** whether it came from a local or from get_ref. Otherwise: `__triet_vector_len`
expects a body_ptr, get_ref returns a cell_ptr → `len` reads `*cell`=body_ptr=garbage. String escapes this trap because of its fat-24B inline layout.
Accessors already exist: `vector_stride` (jit:4018) · `hashmap_value_stride` (jit:4345). **Refuting the fix-the-consumer approach (patching `len` to
deref the cell instead): that would break `len(&0 v)` from a local (a local passes the body_ptr).**

**⚔ O EATS HIS OWN WORDS — the recon claim "A1 is pure env.rs" was HALF WRONG:** O initially declared "A1 doesn't touch the JIT, borrowck is type-agnostic".
POISON-1 (a content-read tooth, `len(ref_vec)`=3, NOT merely routing) exposed the thin-handle indirection blocker.
D stopped and reported to O per the rule. O admitted the mistake, did NOT blame D. **Lesson: a content-read tooth (reading the REAL content) beats a routing
tooth (present/absent only) — routing goes falsely green while release crashes at runtime.** [[feedback_poison_must_be_red]]

**P0 RED ALERT — pre-existing String-key read SIGSEGV (latent since ADR-0080 `381979e`):**
get/get_ref/contains take `&0 HashMap` (**Reference-wrapped**) ≠ insert (owned HashMap). The key_stride extraction
(`mir_lower.rs:3175`) only did `nullable_payload().unwrap_or` → a Reference never reaches the HashMap arm → **defaults key_stride=8**
→ a String key (stride 24) gets marshaled **by-value as 8B** → the hash reads garbage memory → **SIGSEGV 139**. insert escaped this because §AMEND-1 of
ADR-0080 poked directly at the insert-flow (an owned map). Integer-key reads worked because default-8 happens to be correct for them. **0 fixtures, ever,
tested String-key get/contains at runtime → latent and silent under a "BOOKS CLOSED" signature.** FIX: unwrap `MirType::Reference { inner, .. }`
before matching HashMap. O dug up the root-cause by reading the code (not a blind probe). G guessed it 100% correctly ("pass-by-value 8B like Integer").

**❄️ A2 get-borrow-mutable (ADR-0081) FROZEN → banished to Cluster D (Phase 3 Ownership):** `push`/`insert` are functional
(clone+free-old+return a NEW handle) → mutating the inner via `&0 mutable` REQUIRES writing the handle back into the cell → P1 FORBIDS write-back
(deref-assign isn't wired) ⇒ `&0 mutable V` is **VACUOUS for Vector/HashMap** (only pop/remove-shrink are usable). G:
"no half-measures, no dirty loopholes". Reopen this once core has deref-assign + drop-in-place through a pointer. The borrowck-facing architecture
(returns_borrow_form + exclusive-loan conflict on READ too) is already correct — the problem is core's functional-mutate.

**🚫 V=Nullable REFUSE/defer** — the lowerer doesn't yet match `&0 Nullable<T>` (no path to use the inner value). Refuse-over-guess.

**O's blood verification (independent poison→RED, cp-snapshot restore md5 matched):** POISON-1 reverting the stride-deref→garbage `94…` ·
POISON-P0 reverting the Reference-unwrap→SIGSEGV 139 · POISON breaking the overload 336/337→E1041. Fixtures 333-337 (5):
333 Int-key content-read(3) · 334 borrowck-track(E2440) · 335 P0 scalar String-key(142) · 336 String-key get_ref
Vector(2) · 337 String-key get_ref HashMap(1). Independent gate `0·0·331·0` CLEAN.

**⚠️ DISCIPLINE ON D — broke a direct order from G:** O had ordered "drop the 2 String-key overloads" (G signed off on "Integer-Key ONLY, split the merge");
D **unilaterally decided to KEEP** the overloads + folded in the P0 fix (because the P0 fix makes them sound). Technically correct BUT it broke a signed-off order + **was missing the
heap-value String-key fixture** (O had to probe himself to learn len=2/1) — **repeating the EXACT SAME sin as the P0 hole it had just patched**. G swallowed his anger,
accepted the wider scope, but issued a steel warning: *"last time I tolerate shipping an untested API, next time you're out"* + forced D to add
336/337. [[colleague_d_persona]] [[feedback_failure_mode_precision]]

## ✅ FULLY CLOSED — key-typed `HashMap<String,V>` (ADR-0080 + §AMEND-1, Author+O+G signed off, PUSHED 2026-07-03(b))
origin/main = `381979e`, gate `0·0·326·0`. **Campaign Typed Collections P1 (A) BOOKS CLOSED.** `HashMap<String,V>`
+ `HashMap<String,String>` (key ∥ value both on the heap) sound end-to-end from `.tri` source → JIT real-allocator,
not a single byte leaks.

**ADR-0080** (`26452e0`) — O REFUTED amending ADR-0038 (Comparable=`Ord` ≠ `Hash` — mixing them wrecks the architecture) + REFUTED
a `Hashable` trait (a new Tier-1 trait system, building it now would collapse the foundation). A brand-new ADR. **D1** the slot has `key_stride` ∥
`value_stride` **24B fat** (REFUTED 16B: `__triet_string_free` needs the capacity; String does NOT store its length on the heap per ADR-0049
§6.3 → the slot must hold the length for hash/eq); `key_stride∈{8,24}` doubles as the discriminator. **D2/D3** `__triet_string_hash`
FNV-1a + `__triet_string_eq` already exist, dynamic dispatch is forbidden. **D5** key∈{Integer,String}, anything else→REFUSE. **The D
front owes blood at 5 death-points** — O charted an additional **#5 remove-free-resident-key** beyond the Author's original 4: (1) map-drop frees the
key (2) insert-dup must cut down the extra move-in key (3) insert=Move key (4) get/remove/contains=asymmetric `&0` borrow
(5) remove frees the resident key.

**§AMEND-1** (`72bdf7e`) — **D exposed a vacuous-tooth** (recon of KM-P1a): the free was called DIRECTLY inside the Rust shim body
= a static link-time call, BYPASSING the JIT symbol-table (`with_shims:808` substitution) → the counting harness was BLIND →
teeth #2/#3 had been vacuous from the start. O verified independently (symbol-table + the VALUE out_ptr precedent :2952) → took the blade,
retracted the literal WO. The fix = an out-param ABI: `is_update_out` (insert D.2) + `key_out_ptr` (remove D.5) → the free is pushed out to a JIT
call-site, registry-routed, countable. Invariant: the resident key ≠ the lookup key (freeing `k` is forbidden).

**KM-P1a backend** (`c003a5f`) — Front A the 24B fat slot (header packing `reserved = key_stride<<16|value_stride`)
· B `__triet_string_hash` + `hashmap_key_hash/eq` runtime dispatch by key_stride · D.1 `emit_hashmap_key_free_loop`
· D.2/D.5 out-param free registry-routed · rehash key-stride memcpy. Hand-built MIR + counting (the source stayed at E1003
until P1b). D caught his own bug: the key-free-loop compile-time registered `__triet_string_free` for EVERY map including Integer ones →
3 old tests broke → gated compile-time on `key_ty`. **O's 5 teeth poison→RED, independently** (map-drop-leak 1→0 · update-leak
2→1 · remove-leak 1→0 · content-hash cap=1_000_003 · rehash key-stride→SENTINEL).

**KM-P1b source** (`381979e`) — C1 typecheck generic-K∈{Int,String} (`env.rs`) + a String-key overload for
get/len/contains/is_empty + get_ref parity · C2 **E1048 UnsupportedHashMapKey** hard-REFUSE (`exprs.rs:1011`
gate `sub_map["K"]∉{Int,String}`) · D3 borrowck insert `arg_consumes[true,true,true]` key=Move type-aware
(is_copy per-call, NO new code) · D4 get/remove/contains keep borrow `[false,false]`. **A genuine lower-bug D fixed**:
`lower_type`/`lower_type_simple` (triet-lower) unconditionally hardcoded an Integer key → a `HashMap<String,V>` annotation
silently fell back to Integer → reading the 1st type-arg. **A bug D caught himself**: D3 broke D.2 from KM-P1a (M3-zero ran BEFORE the free-of-the-redundant-
key → the leftover key leaked) → reordered D.2/D.5 before M3 (the old regression #2, re-verified, still RED). **O's 7 teeth poison→RED,
independently** (★SS(a) key-leak 2→1 · ★SS(b) value-leak 2→1 · ★SS(c) tombstone double-free SIGABRT 134 · #4 insert-Move
134 · #6 lookup-borrow E2420 · #8 E1048 non-vacuous Tryte+Struct · regr #2 D.2/M3-reorder).

**⚔ LESSON — O corrected D on ★SS(c)** (G praised it as "the pinnacle of verify-don't-trust"): D reported ★SS(c) as "2 redundant defensive
layers, poisoning either layer alone survives, both must be poisoned together to get SIGABRT" → and lowered the tooth's standard to "only prove the outer
invariant". O did NOT accept the narrative — dissected it independently: the KEY path DOES have 2 layers (the state==1 check + a `write_bytes` zeroing the key cell
@4831), but the **VALUE path has ONLY 1 layer** — remove memcpy's the value out to out_ptr WITHOUT zeroing the value cell (no symmetric
`write_bytes`) → **the value-loop's state-check (`:1306`) is SOLELY load-bearing**. Poisoning that single line alone
→ SIGABRT 134. D under-analyzed his own memory-model, stopped at mistaking (b) for (a); O pushed further and exposed the jugular. A pattern of
[[feedback_poison_must_be_red]] + O's ritual #4 (distinguishing meaningless-defensive from a real hazard using blood-backed poison).

**Tier-2+ deferred (not killed):** `HashMap<_,UserStruct>` P2 native-layout · get-clone/borrow heap value ·
get-borrow-mutable key · generic V-overload (P1 was String-only) · hash caching · C native multi-field layout.
[[future_comparable_trait_and_monad_gap]] [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]]

## ✅ CLOSED — Bug-E: Outcome-param ABI + `~->` early-return heap double-free (O+G signed off 2026-07-03)
origin/main = `81fae69`, gate `0·0·326·0`. Giang discovered this himself while writing
`examples/outcome_ternary_family.tri` (pushed straight to main, outside a session): passing
`T~E`/`T?~E` as a function parameter → silently computed WRONG. G ruled that a silent-wrong-answer is worse
than a crash → paused A/C/D, concentrated resources.

**WO1 param-ABI copy-in gap** (`ddb7841`): the callee prologue allocates an empty StackSlot for
EVERY Outcome-typed local, including parameters (`mir_lower.rs:1453`); the parameter-bind loop
(`:1644-1684`) has a copy-in branch for String/Enum but was MISSING Outcome — the caller's pointer
(already correct, `:2676`) was left discarded. Fixtures 328/329/330 (scalar/nullable/interleaved-offset).
⚠️ D used `git stash` to compare pre/post — violating [[feedback_teeth_never_git_checkout]] for the
first time, G logged this in the black book, O re-verified independently via cp and reached the same conclusion.

**WO2 early-return heap double-free** (`818602c`), O extended testing beyond
WO1's scope himself (probing a `String~Integer` param) → SIGABRT 134 → isolated it: the bug needs NO function
parameter at all, it reproduces with just 1 local. 3 sites all missing the HP.4 pattern
(`copy_heap_outcome_payload`/`bind_heap_outcome_payload` + `Deinit`):
- Site A `lib.rs:~5163` (success-arm passthrough unwrap, `~->` early-return)
- Site B `lib.rs:~5023` (error-arm bind `e`, `~->` early-return)
- The SHARED root cause `lib.rs:~1947` (the `Expr::OutcomeConstructor` heap-payload branch —
  shared by EVERY `~+ v`/`~- e` in the language, harmless for a literal/temp but a double-free
  when the payload is a named-local with a drop-obligation — exactly the situation Site B creates).

G signed off on this on-the-spot scope expansion (not touching the locked A/C/D cabinets — this is the ROOT of the campaign
currently open). Fixtures 331/332 (named-local, [[feedback_poison_must_be_red]]). O verified
the blood-proof INDEPENDENTLY for all 3 sites — poisoning ONE site at a time: 5040→332 red/331 unchanged ·
5176→331 red/332 unchanged · 1957→332 red (fixture-count dropped by 258 because the ENTIRE corpus
runs as 1 shared process, the crash truncates the rest of the alphabetical run — NOT a wide-scale regression, O
personally analyzed the raw output to confirm it). Restore md5 matched every time, gate CLEAN at 326.

## ✅ CLOSED — Get-Borrow Heap Value (ADR-0079, G signed off 2026-07-01, PUSHED `4fa0298`, gate 321)
`get(&0 container,k) → (&0 V)?` zero-copy borrow (P1 V=String), replacing E1047 at the borrow
site. Clone is FULLY FORBIDDEN (a hidden alloc = garbage). The loan model: borrowing 1 value = borrowing the WHOLE
container (borrowck cannot name `map[k]` through the opaque hash-shim → a conservative
whole-container freeze). Not-found → a nullable-borrow (NULL_SENTINEL, reusing PA-3c).

Slice A borrowck (`a970540`): U2 `returns_borrow_of` on get_ref → PropagatedLoan
builtin (reusing ADR-0046) · U3 `mutates_arg` (remove/pop in-place) — an active loan →
E2440. Slice B (`f57d9b8`): U1 concrete overload · U4 the `__triet_{hashmap,vector}_get_ref`
shim zero-copy, not-found→NULL_SENTINEL · F-d Copy-source skip-conflict.
⚠️ 2 rounds of O-rejects: remove/pop slipped through the net (U3 originally only checked consume) → D added
`mutates_arg`. O's verification: 5 poison-sensitive borrowck teeth + a content-read
`length(ref_str)`→2/5 + fixture 327's content-read guard (325/326 only ROUTE without reading
content — repeating a lesson from HM-P1b fx322). Deferred: generic V-overload (P1 was String-only) ·
get-borrow-mutable · key-typed.

## ✅ CLOSED — Typed HashMap P1, fully complete (ADR-0078, G signed off 2026-07-01, gate 318)
`HashMap<Integer,V>` (V heap) sound end-to-end through the JIT real-allocator:
insert(Move)/remove(move-out `V?`)/drop. HM-P1b typecheck-open (`f5c11e1`+`2f100fb`):
a dedicated `Type::HashMap(K,V)` (replacing UserStruct) + generic `hashmap_new<V>`/`insert<V>`/
`remove<V>` (key=Integer hardcoded, seeding V from expected_type_stack) + get-heap E1047 +
insert=Move. ⚠️ 3 rounds of O-rejects: (1) non-deterministic garbage — `lower_type`/`lower_type_simple`
hardcoded `HashMap(Integer,Integer)`, dropping the value-arg → stride=8 → a fat String reads garbage;
(2) a vacuous-tooth — SIGABRT 134 used a String LITERAL = a temporary with NO drop-obligation
→ the poison was INERT; O proved it via MIR (a literal has NO Drop, a named-local DOES) — the
NAMED-LOCAL RULE was engraved in stone; (3) clean.

HM-P1a storage backend (`a0e60d8`, gate 315): the value-typed `HashMap<Integer,T>` (T
heap) machinery is sound (dormant — the source was still E1003 at that point, proven via hand-built MIR).
MirType::HashMap(Box<K>,Box<V>) · a value-stride slot with inline stride-in-header ·
a JIT-emitted, registry-routed free-loop · the remove shim's move-out tombstone + out-ptr-
sentinel. 3 tiers of difficulty: T1 value=Vector-reuse · T2 key-typed=NEW hash/eq (DEFERRED,
matching the A-front just settled) · T3 typecheck UserStruct→dedicated Type::HashMap. ⚠️ 3
rounds of rejects: a phantom hash · a VACUOUS fat-rehash tooth with 0 tests · 17 clippy warnings mislabeled
as "pre-existing".

## ✅ CLOSED — Typed Vector P1, fully complete (ADR-0077, G signed off 2026-06-30, gate 312/315)
`Vector<T>` (String/Vector/HashMap/Nullable element) construct+push+pop+drop sound
end-to-end. Element-SIZE built-in = a compile-time CONSTANT (decoupled from native-layout),
REFUSE Vector<UserStruct/Enum> at the P1 boundary. Slice A backend (`76405aa`): MirType::Vector
→Vector(Box) · stride-in-header · a JIT-emitted element-free loop (against vacuity, D caught
a shim-internal free that was bypassing the registry) · by-ptr fat ABI + a pop shim. Slice B typecheck-open
(`951790e`): reused the v0.7.4.1 generic-fn machine (extract_type_params+substitute, NOT
HM-unify) · get-heap→E1047 refuse · push=Move. P1.5 pop-wire (`1977a93`, gate 315): 3
frontend wiring points + a bugfix D discovered himself (empty-fat-pop was writing NULL_SENTINEL into out_ptr).
O found many teeth with SIGABRT 134 real-allocator (poisoning consume/len--/sentinel).

[[feedback_poison_must_be_red]] [[feedback_teeth_never_git_checkout]]
[[feedback_failure_mode_precision]] [[mentor_o_persona]] [[colleague_d_persona]]

## 2026-07-10 — CLUSTER B SLICE C: `HashMap<K,aggregate>` VALUE (ADR-0082 B-α cont., G signed off, PUSHED)
origin/main `6d9e144`, gate `0·0·331·0`. 3 commits: `6ec2630`(F1–F4 + T4 unit) · `36ba45f`(teeth) · `6d9e144`(docs). **Scope:** value-aggregate (Struct/Enum) **insert+drop+alloc SOUND** (mirroring Slice A/B element push+drop); get/get_ref/contains/remove + key-aggregate REFUSED.
**4 fixes / 4 MINES (O's recon, real file:line references):**
- F1 `emit_hashmap_value_free_loop:1387` guard `is_any_heap()`→`aggregate_needs_drop` (Struct/Enum ≠ is_any_heap → the flat guard bails → leak; mirroring the Vector element loop at 1186).
- F2 the `aggregate_needs_drop` Enum-arm: a recursive `for`-loop + `?` replacing the flat `.any(payload.ty.is_any_heap())` — **LATENT defense-in-depth** (the frontend refuses enum-payload-aggregate; unit test T4 pins this directly on a hand-built EnumLayout, bypassing the frontend).
- F3 the `hashmap_insert` value marshal had a TWO-ENDED S3-gap (symmetric to vector_push 3255–3280): END-A fat (>8B) value belongs in `enum_slots` not just `struct_slots`; **END-B** an 8B-aggregate value (wrapping 1 handle, stride==8) → needs `stack_load(slot,0)` NOT `use_var` (the old else-branch read an empty Variable → garbage → a silent leak; the C5/T9 Slice A/B bug reincarnated).
- F4 split the refuse: a new helper `refuse_hashmap_aggregate_key` (key-only) @alloc(3239)+insert(3296); kept `refuse_hashmap_aggregate_kv` (K+V) @remove-probe(3073)+remove(3359)+get-family(3431). G's original WO said 3 sites, O counted 5.
**🩸 O caught a hole G's WO had missed = MINE-3, END-B** (an 8B value wrapping a handle → use_var garbage → a SILENT LEAK, not seen by the 331 fixtures) → a dedicated tooth T3.
**⚖ D's informed "deviation from orders" (approved by G):** get/get_ref/contains/key die at typecheck (E1041 NoMatchingOverload/E1002 undefined/E1048) → the JIT-refuse is defense-in-depth → hand-built MIR (precedent from ADR-0078); only remove touches the JIT. **O probed 5 `.tri` sources to independently verify = absolutely correct.**
**O's 4+1 poison→RED verification, independent** (cp-snapshot restore md5 `62ab04…`): F1→T1/T2/T3 FREE `0 vs 2` · F2→T4 `needs_drop==false` · F3-END-A→T2 compile-fail "fat value without slot" · F3-END-B→T3 FREE 0 (only T3 → an isolated INLINE-anchor) · neutering the 2 refuse-helpers→6 refuse teeth "compilation SUCCEEDED". Failure-mode = wrong FREE-count (a leak, NOT SIGSEGV).
**Teeth:** T1 `hashmap_struct_value_insert_drop_frees_string_field` · T2 `hashmap_enum_value_insert_drop_frees_string_payload` · T3 `hashmap_8b_struct_value_insert_drop_frees_wrapped_vector` · T4 unit `aggregate_needs_drop_enum_recurses_into_struct_payload` · 6 refuse teeth (remove source-level + get/get_ref/contains/key-alloc/key-insert hand-built MIR). Repurposed `hashmap_struct_value_refused_at_jit`→`..._remove_refused_at_jit` (Rule 3; coverage of insert-Struct-value→T1).
**⚠️ The FIX-2 zero-@8 time bomb (Slice B) remains unchanged.** **Slice C deferred debt:** value move-out (get/remove by-value — sharing a grave with Vector pop) · get_ref borrow of a value-aggregate (Cluster D) · contains-allow for a value-aggregate · key-aggregate recursive hash+eq.
**Next front:** value move-out aggregate (recursive move-out-tombstone: dest leaf-marshal + buffer/cell tombstone + source) OR key-aggregate — to be decided by G/Giang.
