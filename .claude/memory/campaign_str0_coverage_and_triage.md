---
name: campaign_str0_coverage_and_triage
description: "✅ CLOSED 2026-07-25(c) — a TRIAGE of the whole debt ledger (burying 2 zombies + 1 phantom) + `&0 String` read-borrow coverage (len/eq/concat). origin/main 5dd2aeb, gate 0·clean·0·458·0."
metadata: 
  node_type: memory
  type: project
  originSessionId: 12b89f5b-2d0d-472a-bab6-868c1c7615ed
  modified: 2026-07-25T07:54:58.625Z
---

## ✅ CLOSED — 3 commits PUSHED (signed by O and G, 2026-07-25(c))

```
5dd2aeb  docs(todo): close &0 String coverage, log WO-JIT-Print debt
31fd2d6  feat(track-c): &0 String read-borrow overload coverage — len/eq/concat
4904c55  docs(mentor): clean the G state — bury the Enum? param zombie and the &+ T phantom, reclassify N1, fix fixture 177
```
Gate `0·clean·0·458·0`, fixtures 455→458 (462/463/464).

## 🔑 THE BIGGEST LESSON: BACKLOG LABELS ARE NOT TRUSTWORTHY — RECON-BEFORE-WO SAVED US TWICE

Giang picked the front "Enum? param SIGILL 132" from the debt ledger. O reconned first (verify-don't-trust):
- **The Enum? param is a ZOMBIE**: already closed by `ccb8db3` (WO-NullableEnumParamABI, 2026-07-19, teeth
  419-427). O built a pristine worktree at `564f0f7` → reproduced exit 132; at HEAD → it runs correctly
  (1/99/42). The debt lived on in the forward list of `MENTOR_G_STATE.md` while being closed in the code and
  in TODO.md.
- Giang ordered a **fresh triage of the ENTIRE debt ledger** → which exposed another one: **the `&+ T`
  borrow params are a PHANTOM**: ADR-0022 §4.1 says `&+` is not a borrow but a unique OWNER, so passing it
  is a MOVE; moving heap values in ALREADY works through a plain param `f(v: Vector)`; and sharing across
  threads is ADR-0026 **BYOS, FROZEN**. It unlocks no capability at all. BURIED.
- The triage also found: **N1 widening** is now a clean `E1120` refusal (an ADR-0065 feature, NOT a
  miscompile pit); and the comment on fixture 177, "tail-expr fat-struct return SIGILL", is STALE (verified:
  a plain free function with an expression body → 30, exit 0).
- **NO entry was still a live soundness or crash pit.** Everything genuinely open is a feature-completeness gap.

🩸 **O's blemish:** it initially overstated "the G state is full of zombies"; a precise grep showed it was
milder. Read displayed a working-tree version that differed from HEAD (abnormal) → O **reverted to the real
HEAD and re-applied each line by hand** (auditable, refusing to ship changes of unknown provenance into G's
boot file — the `3417c4f` clobber precedent). Verify-don't-trust applied to its own output.

## THE `&0 String` COVERAGE FRONT (G chose Option 2: len+eq+concat, via Option A)

**The hole matrix** (real CHECK+RUN probes): Vector/HashMap `&0` reads are FULLY covered; the holes are only
in String, and scattered: `length/contains/is_empty(&0 String)` ✓ but `len(&0 String)`→E1041 and
`concat/eq(&0 String)`→E1003.

**The patch surface is typecheck-only (env.rs); the C shims ALREADY accept `&0 String`** (live proof:
`length(&0)`→5, `contains(&0)`→9):
- **len**: add one `declare_overload("len",(ref_string)->Integer)` next to the ADR-0059 C.2 block
  (`env.rs:747`); the `"len"|"length"` arm (`triet-lower/src/lib.rs:2685`) already strips the Reference →
  ZERO lowering changes.
- **eq**: `declare`→`declare_overload` + 3 combinations `(ref,owned)/(owned,ref)/(ref,ref)`; the JIT's
  `bung_fields` class already has an `is_reference()` branch → ZERO JIT changes.
- **concat**: this one needs the JIT EXTENDED. The `concat_sret` class (`mir_lower.rs:3968-3979`) reads only
  `struct_slots` and lacks the Reference fallback branch that `bung_fields` has (`:3993-4012`). The fix is a
  **verbatim MIRROR** of the `is_reference()` branch (`use_var`→`load {ptr,len}@0/@8`) into concat_sret's
  else. That unifies the marshalling of the two classes.

**Option A (explicit overloads)** — G REJECTED an implicit `&0 T→T` coercion ("silent typecheck garbage").

**print/println were EXCLUDED** (O used the authority G delegated "if needed"): there is no JIT shim
(`grep __triet_print` = empty; even an owned String gives `callee print not found`). Adding a `&0` overload
would be a NO-OP (merely moving E1003 to a JIT not-found). → recorded as the debt **WO-JIT-Print** (its own
I/O front: build the stdout shim and wire it).

## 🦷 BLOOD TEETH (string_ref_overload_free_counting.rs, --test-threads=1, with pointer dedup)
- Healthy: `len(&0 s)` FREE=1, `eq(&0,&0)` FREE=2, `concat(&0,&0)` FREE=3 (2 borrowed + 1 result allocation),
  dup=0.
- Poisoning the SHIM (proving the counter is alive): a leaking shim → FREE 0; a duplicating shim → FREE 2×N
  with dup>0.
- **O poisoned INDEPENDENTLY at 2 layers** (a /tmp cp snapshot, matching md5, NO git checkout):
  removing the len overload → 462 gives E1041; removing the `is_reference` branch from concat_sret → 464
  exits 4 with "concat: String arg without slot". Both are load-bearing.

## ⚖ ROLES (a clean session)
- **D = a Sonnet 5 subagent**: WO-1 (len+eq) STOPPED correctly per LAW 4 at concat (stuck on concat_sret —
  reported to O, reverted cleanly, did NOT widen into mir_lower.rs on its own). WO-2 (concat) was completed
  once G opened the scope, mirroring the specified idiom without inventing anything. 0 fabrications. WIP
  commits, no push.
- **O**: reconned before the WO (burying 2 zombies + 1 phantom before burning effort), issued 2 WOs, verified
  all 3 slices independently with blood (its own gate, its own 2-layer poison), squashed the commits neatly,
  pushed, and confirmed with ls-remote.
- **G**: signed the kill; REJECTED the coercion; demanded a MIR dump for concat and asked about print/println
  (O answered with evidence → excluded).

## 🔴 NEW AND OUTSTANDING DEBTS (awaiting G and Giang to open one)
- **WO-JIT-Print** (NEW, clearly defined): `print`/`println` have no JIT shims
  `__triet_print`/`__triet_println` for the stdout side effect. It needs the shims built, the lowerer wired,
  and JIT declarations. An I/O front, NOT part of `&0` coverage.
- method returns of `Struct?`/`Enum?` (E1100 ConstructNotYetLowered) · get_ref with V=Nullable (E1003, the
  lowerer's `&0 Nullable`) · §15.6 making `Vector<Leaf?>` run (a feature, with sensitive drop glue) · deep
  Clone (a large campaign) · drain (the Iteration ADR) · `&0 Enum` consumption (basic borrowing already
  works; the "consumption" concern is not yet clear) · the mir_lower panic groups B/C (B is a tombstone, C
  is deferred as `D-JIT-OOM`).

[[campaign_shim_meta_spof_adr0085]] [[campaign_typed_collections]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_teeth_never_git_checkout]]
