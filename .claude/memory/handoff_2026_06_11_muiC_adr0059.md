---
name: handoff-2026-06-11-muic-adr0059
description: Spear C CLOSED (C.1+C.2) — ADR-0059 stack-borrow `&0` for heap Vector/HashMap + fixing the generic return-bind; `&+` sealed as YAGNI. HEAD 8be0263.
metadata: 
  node_type: memory
  type: project
  originSessionId: 3ff940f3-c92e-4084-9b38-c8e2a2aa3a3d
---

# Spear C FULLY CLOSED (C.1+C.2) — ADR-0059. HEAD `8be0263`, gate 0·0·163·201

**2026-06-11.** After the Outcome chain 0052→0058. Spear C = stack-borrow `&0` for heap Vector/HashMap
plus repaying the generic return-bind debt. `&+` StrongFrozen is sealed as YAGNI (ADR-0059 §5).

## The commit chain (verify-don't-trust; O measured each one personally)
| Commit | Work | Status |
|---|---|---|
| `03b0655` | The ADR-0059 doc (written by O, committed verbatim by G) | ✅ |
| `bf668fd` | **C.1** the generic arm in `lower_type` + `lower_type_simple` (Vector<Integer> return-bind) | ✅ O's teeth (poison→`len() on type ?`) |
| `002daca` | fixing a duplicate fixture number 105→166 (G had reused `105_e2420_branch_move`) | ✅ |
| `c17bd96` | ADR-0059 §8 correcting the teeth + the get scope | ✅ (G ⏳ to co-sign §8) |
| `8be0263` | **C.2** the `&0` overloads of len/get for Vector/HashMap + the get lowering fix | ✅ O ACCEPTED THE CODE |

## Process lessons (3 incidents from G and D, all caught by O)
1. **G broke the cadence at C.1:** G coded and committed WITHOUT O's teeth going first. O applied teeth
   retroactively (poisoning the generic arm → 105/166 regress to `len() on type ?`, turning 1 of 161 in the
   gate red). The code happened to be right, but committing on faith is exactly what verify-don't-trust
   forbids. Law: teeth BEFORE the commit.
2. **G reused fixture number 105** → O caught it and renumbered to 166. Convention: a new number is max+1
   (D checks with `ls fixtures|grep -oE '^[0-9]+'|sort -n|tail -1`).
3. **D overclaimed the crash label at C.2:** D ran fixture 167 under poison-608 → exit **132 (SIGILL)**,
   with NO double-free line, and then labelled it "double free, tcache abort, SIGABRT". Wrong. 132=SIGILL
   (an arithmetic trap), 134=SIGABRT (a double free). 167 computes `n+m`: the callee frees the buffer →
   `len(xs)` reads garbage → `2+garbage` overflows → trapnz SIGILL fires BEFORE the double-free Drop.
   **O forced a minimal probe (no post-borrow arithmetic) → exit 134 + a CLEAN
   `free(): double free detected in tcache 2`.** The Vector mechanism is IDENTICAL to String (O's
   prediction was right). D's lesson: **read the crash signature before naming it** — only 134 plus
   "double free detected" is a double free; 132 is a SIGILL.

## Technical conclusions locked in (for the next front)
- **Borrow-param-no-free is TWO independent layers:** (a) the lowerer does NOT push_owned a reference
  param (`lib.rs:621-626`); (b) the JIT Drop handler is **type-gated** — it skips locals of type
  `Reference`. Poisoning ONE layer is caught by the other (poisoning the push_owned guard at `lib.rs:624`
  → exit 0, no blood). **Only poisoning the type classification at `lib.rs:608` (stripping Reference →
  owned) defeats both → a double free, 134.** (Recorded in ADR-0059 §8; the original wrongly said 624.)
- `get` lowering (`lib.rs:1939`) does **not** strip references the way `len` does (1733-1737); `is_vec()` =
  `matches!(Vector)` does not see through a Reference. C.2 added the stripping for `get`.
- `&0 String` borrowing already worked (fixtures 77/84/100); the backend wiring is shared (the call site
  passes a stack_addr, the shim takes a pointer to the slot) — Vector/HashMap reuse it as is.

## Still outstanding
- ADR-0059 §8 needs G's co-signature (G ⏳→✅) — the same pattern as ADR-0058 §9.
- TODO.md must record that Spear C is closed.
- `&+`/`&-` StrongFrozen/Weak: sealed as YAGNI (ADR §5); open a separate ADR when there is a 2-owner use
  case plus a runtime ObjectHeader refcount.
- The next front (awaiting G / the author): other debts in TODO.md.
