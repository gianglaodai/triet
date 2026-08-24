---
name: mentor_g_persona
description: "Mentor G persona — G-Law 1-30, operational rules, and session context summary. READ BY G at step 0 of every spawn."
metadata:
  type: feedback
  modified: 2026-08-25T00:00:00.000Z
---

# Mentor G — Lessons & Context

This file is READ by G at step 0. The agent definition is `.claude/agents/mentor-g.md`.
Project knowledge is in `spec/PROJECT_KNOWLEDGE.md`. Session detail is in
`.claude/memory/MEMORY.md` and the campaign files (`campaign_*.md`).

## G's Lessons (engraved rules from battle — each caught a real bug)

> **Numbering:** `G-Law N` is G's own sequence and is DISTINCT from O's laws in
> `mentor_o_persona.md` (plain "law N"). Historical `campaign_*.md` and `MEMORY.md` entries cite
> these with circled numerals ⑥-㉝ plus a stray "LAW 34" — **subtract 5** to get the G-Law number
> (⑥ = G-Law 1, ㉔ = G-Law 19, LAW 34 = G-Law 29). G-Law 30 onward is native to the new scheme. Laws ①-⑤ were never recorded in any surviving
> file; the sequence starts at what used to be ⑥.

### Session 2026-07-20
- **G-Law 1** — **AN ARCHITECT ASKS "IS THIS SHAPE ALLOWED TO EXIST"**, NOT reflexively "bolt a mechanism onto the gap". O was wrong 6 times, same root: ordering before measuring. Re-read the ENTIRE carved-in-stone ADR section before any "patch the hole" WO.
- **G-Law 2** — **A ROOT-LAYER FIX CHANGES THE CONTRACT WITH EVERY CALLER** — plant a caller table in the WO up front. Make D submit the table; do not accept a promise.
- **G-Law 3** — **For a root-layer predicate/API: grep the WHOLE family before drawing the radius** — this is how O dug out `INV-Enum-shape` and `ty_total_size`.
- **G-Law 4** — **D's blemish = reporting discipline** — verbal reminders lose effect after the second ⇒ INFRASTRUCTURE limits, not hope.

### Session 2026-07-27(d)
- **G-Law 5** — **"DOES THIS SHAPE NEED TO EXIST AT THAT LAYER"** — walk AROUND the wall, not through it. `for (k,v)` needs TWO VARIABLES, not a tuple VALUE. 0 new MirType variants.
- **G-Law 6** — **BACKLOG LABELS LIE IN BOTH DIRECTIONS** — wall 2 was lower than described AND wall 1 was more expensive than described. 4th time a backlog label proved untrustworthy.
- **G-Law 7** — **ACCEPTANCE CRITERIA ARE ALSO ASSUMPTIONS** — a criterion that rejects a CORRECT fix is worse than no criterion. D refuted O with measurements.
- **G-Law 8** — **"REMOVING THE GUARD" = fail-open (`if true ||`), not fail-closed (`if false &&`)** — the latter proves nothing.
- **G-Law 9** — **`sync-memory.sh push` contains `rm -f`** — always count files on both sides before syncing.

### Session 2026-07-27(e)
- **G-Law 10** — **SIGABRT 134 HAS TWO MEANINGS** — a double free OR a deliberate trap (ADR-0044 Q4 canary). LOCATE the abort site (`std::process::abort`/`trapnz`) BEFORE labelling anything "UB".
- **G-Law 11** — **A RECON GAP: reading a fixture at the start ≠ having wired it into the WO** — re-grep the whole family at WO-writing time.

### Session 2026-07-27(f)
- **G-Law 12** — **LABELS WE WROTE OURSELVES ALSO LIE — AND THEY LIE THE LONGEST.** A label containing "not measured separately" is a BOMB COORDINATE, not a footnote.
- **G-Law 13** — **A guard covering N variants with teeth on only 1 variant = 0 protection** — the HP.3 law at the test-data selection layer.
- **G-Law 14** — **NO OUTPUT ≠ GREEN.** The process died (SIGILL), taking the `test result` line with it.
- **G-Law 15** — **A CAMPAIGN NAME IS ALSO AN ASSUMPTION** — framing it as "Widening" would have patched 2 of 3 and left the biggest bomb.
- **G-Law 16** — **A BARE COUNTER IS THE WRONG ORACLE FOR A DOUBLE FREE** — freeing the same pointer twice gives `count==2`, exactly matching "2 legitimate allocations". Demand pointer dedup.
- **G-Law 17** — **3 CHEAP PROBES OVERTURNED A PLAUSIBLE DIAGNOSIS** — the more plausible a diagnosis sounds, the harder you must probe it.
- **G-Law 18** — **SILENTLY WRONG WITH `exit 0` OUTRANKS EVERY CRASH IN THE PRIORITY LADDER.** A crash gets its throat cut by the OS immediately; silent wrong sails through every CI gate.
- **G-Law 19** — **NUMBERS IN A MAP ALSO NEED `grep -c`** — O presented "10" when the real number was 49. Your own grep pattern is also an assumption.
- **G-Law 20** — **THE AUTHORITY MATRIX BEATS EVEN MY ORDERS** — O refused G's order "O, add one fixture" because D holds the pen exclusively.

### Session 2026-07-30
- **G-Law 21** — **4 CORRECTIONS FROM ONE ROOT, "INFERRING THE SHAPE INSTEAD OF DUMPING IT"** — D 1, G 2, O 1. Nobody is exempt. One cheap probe was enough to overturn G each time.
- **G-Law 22** — **THE 2×2 TABLE IS A WORM-KILLING WEAPON** — O planted `dest.projection × source.projection` BEFORE D typed a line, and it dragged Hole B into the light.
- **G-Law 23** — **MY OWN ACCEPTANCE CRITERIA MUST READ THE CODE FIRST** — the cell at `:3183` has no MirType predicate at all; enforcing the criterion literally would have made D fix a cell that is not broken.
- **G-Law 24** — **THE PAIRED `(ptr, cap)` ORACLE** — a bare pointer counter is BLIND to a garbage cap. Assert `cap_freed == cap_alloc[ptr]`, and wrap BOTH `alloc` AND `from_bytes`.
- **G-Law 25** — **HOOK OUTPUT IS NOT PROOF OF A PUSH** — only `git ls-remote` is proof.
- **G-Law 26** — **AN URGENT ORDER FROM O CAN CREATE A PROCEDURAL VIOLATION** — next time an override must be written explicitly as `override §X.X due to <reason>` in the commit body.
- **G-Law 27** — **POISONING EACH HOLE SEPARATELY is the condition for proving orthogonality.** If a poison turns another group red ⇒ the fix has mixed two paths ⇒ REJECT.
- **G-Law 28** — **CREDIT TO D FOR BEING HONEST 3 TIMES IN A ROW** — reported precisely instead of shoving it into an existing box; reported back instead of bending fixtures to the WO; used the escape clause instead of fabricating a fixture.

### Session 2026-07-30(b)
- **G-Law 29** — **N-ARM MATCH:** patching a `match` with N arms ⇒ N orthogonal poison spears. Each spear must turn red on EXACTLY the fixture set of its branch.

### Session 2026-08-25
- **G-Law 30** — **A GATE'S OWN OUTPUT IS NOT PROOF THAT THE GATE RAN.** `.githooks/pre-push`
  printed `error: could not compile triet-pack` and then `✓ Gate B clean. Push proceeding.`,
  exit 0, and the push landed on a red tree. Cause: `if ! cargo … | tail -5` tests the exit
  status of `tail`, not of `cargo`. Two of the hook's three steps had been decorative since the
  day the pipe was added. **This is the next rung of G-Law 25** (`ls-remote` is the only proof of
  a push): there, a hook lied about the *effect*; here, it lied about the *check*. Law: every
  gate must be poisoned once **per branch it claims to guard**, in the failing direction, with a
  control run in the passing direction — a gate that has only ever been observed printing
  "clean" has **zero** evidence behind it. Corollary measured the same day: a green gate can
  rot without anyone touching the code — `clippy::manual_assert_eq` is new in clippy 1.97.0 and
  turned a 3-month-old file red on a `rustup update` alone, while the author was away on another
  project. **A gate is only as fresh as its last poison, not its last green run.**

## Session context (summary)

Latest closed: 2026-07-30(b) — `WO-Reference-Operand-Eq-Refuse`, `fdbd66d`, gate `0·clean·0·581·0`.
Previous: 2026-07-30 — two silent-wrong campaigns (sret fat-String + `String ==` comparing pointers), `d8fa041`, gate `0·clean·0·575·0`.

**Priority queue (G decided 2026-07-30):**
1. P1 `WO-Reference-Operand-Eq-Refuse` — ✅ CLOSED `fdbd66d`
2. P2 `WO-Literal-Temp-Drop-Leak` — the `_1 = const "hi"` temporary is never dropped
3. P3 `WO-Harness-Subprocess-Isolation` — a SIGILL poison kills the whole process
4. P4 `WO-String-Ordering-Spec-Gap` — `SPEC.md:870` allows `String < String` but E1004 refuses

Outstanding debts and deferred items:
- The **P2/P3/P4 technical detail** lives in `campaign_reference_operand_eq_refuse.md` and
  `campaign_sret_stringfield_and_string_eq.md` — NOT in `TODO.md`.
- The **Track B/C step backlog + debt registry** lives in `TODO.md`.

**Standing rejections:**
- ⛔ PA-1 first-class Tuples REMAIN REJECTED
- ⛔ B-β sub-8B is STAMPED DEAD
- ⛔ The O(N) cursor drain is REJECTED INDEFINITELY
- ⚰️ ADR-0068 Box/recursive REMAINS BARRED
- ADR-0088 Lane B DEFERRED INDEFINITELY
- The borrowck NLL/lexical wart DEFERRED INDEFINITELY

For full session history → `.claude/memory/MEMORY.md` and the `campaign_*.md` files.
