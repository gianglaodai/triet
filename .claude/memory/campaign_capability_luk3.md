---
name: campaign_capability_luk3
description: ✅ CAPABILITY Ł3 (ADR-0069) SEALED — the VISION §8 coherence is complete (null / logic / capability under one Ł3 algebra). A ZST token carrying an Ł3 Trit, enforced by borrowck, with a runtime trap for Defer. origin b081184, gate 0·0·273·0. Slices 0-4 + §amend-A + the §5 LOCK. Next in the red book: Partial-move & Struct-ZST + the import `.`→`::` change. READ THIS if you touch capability/mint/ZST tokens/__triet_cap_check/E2211/E2212.
metadata:
  node_type: memory
  type: project
  originSessionId: capability-luk3-campaign
---

**Capability Ł3 (ADR-0069)** is the third leg of the VISION §8 COHERENCE — a single Ł3 algebra spanning
**null (PA-3c) / logic (Trilean) / capability**. It turns the ternary-first mandate (G + Giang, 2026-06-22)
into constitution. The ADR `docs/decisions/0069-zst-capability-token-luk3.md` is 🔒 SEALED (O and G signed
each slice).

## The strategic fork (O's recon → G chose C)
There were TWO capability worlds: (1) the **package manifest** of ADR-0016/0017/0018 — the 4-state Ł3
algebra was ALREADY coded (`triet-pack/types.rs:297` CapabilityLevel) but **orphaned** from the driver
(typecheck's `check_capabilities` was never called by anyone); (2) the **hardware-token ZST** (schema §10) —
capability = ownership + move, coherent with No-Box but "design only". **G chose DIRECTION C, a synthesis:**
bury 0016/0017/0018, rescue the Ł3 algebra, and build it on the ownership/move machine (No-Box) → a ZST
token that **carries** an Ł3 Trit.

## The Ł3 ↔ capability-lifecycle mapping (the heart of the coherence)
| Ł3 | Level | Semantics | Enforcement | Cost |
|---|---|---|---|---|
| `Trit::Positive` | **Grant** | free minting, possession = the right | typecheck + borrowck move/E2420 | 0 bytes |
| `Trit::Zero` | **Ambient** | **receive-only** (M1): minting is E2211, receiving via a param is fine | typecheck | 0 bytes |
| `Trit::Negative` | **Deny** | minting forbidden + possession ABSOLUTELY forbidden (as a param or binding type) | E2211 on mint · **E2212** on possession | — |
| `Trilean::Unknown` | **Defer** | minting goes through a runtime hook; ≤0 traps | the JIT's `__triet_cap_check` + `trapnz user(2)` | 1 check |

## The slices (each verified with blood by O, with independently red teeth, restored byte-identically and NEVER with git checkout)
- **Slice 0 `8b06a28` — the ZST token and forbidding copies.** A `capability X grant` declaration
  (`Item::Capability`, schema-generated) + `mint X` → a 0-byte ZST local. **The soundness pin: `is_copy` on
  an empty struct → `all()` over ∅ → Copy = a silent bypass** (`triet-mir/lib.rs:666`). Forced non-Copy at
  two layers: `MirType::Capability=>false` (mir) + `ctx_is_copy` (the lowerer) — defence in depth, where
  poisoning EITHER one alone still goes red (they cover each other), and poisoning BOTH loses E2420 = the
  bypass. Structs that are empty OF DATA KEEP Copy (a separate short circuit). Plus `public capability` →
  refused (N2, mirroring imports).
- **§amend-A `47eb283` — M1 receive-only** (Giang's syntax: `capability`/`mint` as contextual keywords; G
  buried M2 possession-gating = duplicating a non-Copy token, and M3 call-graph = action at a distance).
  Ambient is pure O-Cap: the token descends from the outer boundary through parameters, "air does not
  spontaneously generate capability".
- **Slice 2 `ca8272e` — the possession check.** `resolve_type` (the chokepoint for every param/let/field/
  return annotation): a deny capability used as a type → **E2212**; ambient and grant are possessable.
  Minting an ambient → E2211 "receive-only".
- **§5 `d84cd24` — G LOCKED the check AT THE MINT SITE** (NOT at guarded operations: a ZST evaporates at
  runtime, so checking at the guarded op means stuffing runtime checks across every use site = killing the
  essence of a ZST).
- **Slice 3 `2dd4d5f` — the Defer runtime hook (the final boss).** `Expr::Mint` with defer →
  `Statement::CapabilityCheck` (a new MIR variant, populated in the lowerer and consumed in the JIT in the
  same commit, per rule #4) → the JIT emits `__triet_cap_check(cap_id)` → `icmp SignedLessThanOrEqual 0` →
  `trapnz unwrap_user(2)` (SIGILL, SEPARATE from arithmetic's user(1)). `CAP_POLICY: AtomicI64` defaults to
  **0 = Unknown = fail-closed**. A subprocess test (`capability_defer_trap.rs`, N7 + the fork-bomb guard
  `_TRIET_CAP`): allow(+1)→exit 0 · deny(−1)→SIGILL · unknown(0)→SIGILL. ⚔ **The fail-closed tooth =
  changing `icmp sle`→`slt` in the Cranelift IR → Unknown(0) slips through → unknown_traps goes red** (the
  `≤` boundary is load-bearing — G's commendation: "do not trust the coder's intentions, trust only the
  CPU's cut").
- **Slice 4 `278`→30 — the demo.** G settled on A2 (capabilities passed as separate params) instead of a
  full struct aggregate: `struct Hardware{vga}` destructure-move requires **partial moves** = the core of
  the borrow checker, which must NOT be stuffed into the capability ADR (scope creep: "you finished the
  heart surgery, do not start on the ligaments"). The demo shows all 4 levels, and running it gives 30.

## New error codes
- **E2211** CapabilityLevelUnsupported — minting a non-grant (deny/ambient/defer).
- **E2212** CapabilityNotPossessable — a deny capability used as a type (param/binding/field).

## 🔴 The red book — 2 independent campaigns NEXT (G supervises personally when they open)
1. **Partial moves & struct ZSTs:** `let v = hw.vga`, field-level move state, is the core monster of the
   borrow checker and memory management (it needs its own ADR + poisons for struct decomposition, half
   moves, and using the other half) plus cleaning up **the B8 gate at `triet-lower/src/lib.rs:72`** (which
   mistakes a ZST capability field for heap → rejecting `struct Hardware{vga}`). It unlocks the canonical
   destructure-move proof of schema §10 and the full Slice 4 that was deferred.
2. **Imports `.` → `::`:** Giang admits he chose `.` out of Python/Java habit; G demands `::` for a clean
   AST. That **REVERSES ADR-0005** (dot paths are LOCKED) → it needs a NEW superseding ADR (no silent
   revisionism). A wide sweep: the lexer, the parser, every example and fixture, the docs (SPEC + the
   CLAUDE.md §Language conventions table).

## The lesson O swallowed (this session)
- **close-session nearly pushed blind:** the machine-local auto-memory was sparse (a 3-line MEMORY.md, since
  the session never ran a pull at open) while the repo was rich (66 lines). `sync-memory.sh push` =
  `rm repo/*.md` + copying the auto-memory over → it would have **clobbered 44 files**. O STOPPED after
  measuring (wc -l + diff), edited the repo directly, and pulled to sync instead of pushing.
  (Look at the target before overwriting it.)

🔒🏁 **CAPABILITY Ł3 IS SEALED — the three-legged coherence stands. The axis is closed.**

[[mentor_o_persona]] [[colleague_d_persona]] [[project_vision_os_capable]] [[campaign_truc_b_heap_in_aggregate]]
