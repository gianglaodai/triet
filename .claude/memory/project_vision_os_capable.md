---
name: project-vision-os-capable
description: Triết's 5 architectural pillars + the ternary identity + the cadence policy. Version-agnostic — check ROADMAP for the shipped/next phase.
metadata: 
  node_type: memory
  type: project
  originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---

**Hard commitment:** Triết is not a pure interpreter. The long-term goal is an OS-capable language, able to write a microkernel if ternary hardware ever appears. Paced on a 5–10 year horizon. Stability > speed.

**The 5 architectural pillars (locked in `VISION.md`):**

1. **Module system** — the aesthetics of Java JPMS + Python imports + dot paths. ADR-0005.
2. **A stable IR + bytecode VM** — register-based SSA, the `.triv` wire format, differential testing against the interpreter. ADR-0007/0008/0010.
3. **A stable ABI + Crate-Pack** — witness-table dispatch (Swift style) for cross-package generics; monomorphization intra-package; `.tripack` packaging; semver linking with `iface_hash` as the final arbiter. ADR-0011/0012/0013.
4. **CAS packaging** (Unison-inspired) — hash-based module identity, two levels: `iface_hash`/`impl_hash`. Read ROADMAP when the corresponding phase arrives.
5. **Capability namespaces** — `sys.`/`dev.`/`usr.` enforced by the compiler. Trit-level capabilities (-1/0/+1) + Łukasiewicz `Unknown` = runtime policy.

**The ternary identity (3 non-negotiables):**
- Trit-level capabilities (3-state natively, never emulated)
- Trilean Ł3 by default (no Boolean reasoning in the logic ops)
- A natively stable ternary ABI (no struct padding, no endianness)

**Logic ops:** symbolic preferred (`!`, `&&`, `||`, `^`, `=>`, `~>`, `~^`, `<=>`, `<~>`). ⚠️ STALE: this entry used to call `0t+`/`0t-`/`0t0` "prefix trit" literals — they are balanced-ternary **Integer** literals; the Trit literals are `1_trit`/`0_trit`/`-1_trit` (see [[triet_trit_literals]]). `unknown` is not `null`.

**Cadence:** every architectural decision gets an ADR in `docs/decisions/` before any code. The version gate matrix (ADR-0009) applies to every version bump: spec ✓, tests ✓, bench gate ✓, snapshot ✓.

**Why:** the user's goal is to prove that ternary is not a freak show. If the language cannot write an OS, it will forever be limited to "the oddball in a binary world". The stability commitment is a way of paying up front instead of accumulating debt.

**How to apply:** every technical proposal must be checked against the 5 pillars — cite which pillar it serves. Refuse features that contradict the capability/ABI/CAS roadmap. When the user asks "what is the next phase", read `ROADMAP.md` + `TODO.md` instead of recalling memory. Related: [[feedback-stability-over-speed]], [[project-triet-overview]].
