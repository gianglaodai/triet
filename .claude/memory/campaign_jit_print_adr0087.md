---
name: campaign_jit_print_adr0087
description: WO-JIT-Print / ADR-0087 — print/println stdout works for the first time since the rewrite; 4 overloads × 4 extern-C shims; LAW 4 exposed holes in O's recon twice
metadata: 
  node_type: memory
  type: project
  originSessionId: 3712b205-e00c-4e74-a480-17f825f1409d
  modified: 2026-07-25T12:11:10.744Z
---

# WO-JIT-Print / ADR-0087 — the first stdout I/O since the rewrite (2026-07-25)

origin/main feature head **`25cb2cc`**, gate `0·clean·0·458·0`, 6 commits. Signed by O and G.
This is the **FIRST stdout write** on the rewritten backend — before it, `print`/`println` were
only `declare`d in typecheck, fell into the lowerer's default arm, and hit the JIT as
`callee not found`, exit 4 (a feature gap, NOT a silent miscompile).

## The design (G signed 5 decisions, ADR-0087)
- **4 overloads:** `print(String)`, `print(&0 String)`, `println(String)`, `println(&0 String)`.
  Owned = MOVE = consuming; `&0 String` = a borrow (a Reference is Copy) = reusable after printing.
- **4 extern-C shims** (memory responsibility hardcoded into the SYMBOL NAME, with no `is_owned` flag):
  `__triet_print`/`__triet_println` for owned values, `arity:3 (ptr,len,cap)`, `arg_consumes:[true]` → write
  then `__triet_string_free`;
  `__triet_print_ref`/`__triet_println_ref`, `arity:2 (ptr,len)`, `arg_consumes:[false]` → write only.
  An owned move-in ⇒ the shim owns and frees it; M3 zeroes the caller's slot ⇒ the caller's Deinit becomes
  free(0), a no-op ⇒ a single free (the `vector_push` pattern).
- **A proper Unit return:** `emit_shim_call` (`triet-lower/src/lib.rs`) gained a `MirType::Unit` branch →
  `dest:vec![]` + `ReturnShape::Unit`, returning a **real Unit local** (`ConstValue::Unit`) instead of a
  throwaway i64 rebind. **G flatly REJECTED O's i64-0 trick** ("technical debt; every future Unit function
  would repeat the garbage"). `ShimSymbol{has_return:false}` (the void pattern already existed); registered in
  `triet-driver/src/main.rs`.
- **Capabilities are compile-time only** (`capability_check.rs` E2200/E2201; `std` is ambient, `sys.io` is
  grant). NO runtime `__triet_cap_check`. This follows VISION §capability.
- The routing arm at `triet-lower/src/lib.rs:2661`, `"print"|"println"`, sits BEFORE the default: strip the
  Reference prefix (the `len` pattern), require a String base, and dispatch `(op,is_ref)` → 1 of the 4 shims.

## 🔑 THE BIG LESSON: O's recon-before-WO still MISSED the typecheck layer — D saved it twice via LAW 4
1. **Recon hole (a):** WO Task 3 omitted `env.rs`. `print`/`println` were declared with a SINGLE
   `env.declare` (not `declare_overload`); and `check_call` only runs `resolve_overload` when
   `env.lookup(name).is_none()` (`exprs.rs:879`); and `Type::matches` (`types.rs:274`) does NOT coerce
   `Reference(String)`→`String`. ⇒ `println(&0 s)` was rejected by typecheck BEFORE the lowerer ⇒ T2/T4
   (which use `&0 s`) were **unreachable**. D STOPPED and asked. O verified all 4 claims as correct → the fix:
   switch BOTH to `declare_overload` + add the `Reference(BorrowReadOnly,String)` overload (mirroring
   len/eq/concat).
2. **Recon hole (b):** 2 tests, `flags_call_arity_mismatch`/`flags_call_argument_type_mismatch`
   (`triet-typecheck/src/lib.rs`), borrowed `print` to test the generic WrongArity/Mismatch mechanism;
   once `print` became overloaded, a bad call produced `NoMatchingOverload` (the correct new behaviour) →
   both tests broke. D proposed (a) switching to `to_string` (single signature). **O chose (a) BUT with a
   USER-DEFINED function** — the ROOT fix: decouple the test from a builtin's overload state (if to_string is
   ever overloaded, it breaks identically); a user function is the canonical single-signature case and will
   never be overloaded ⇒ killing the whole class of fragility.

**Carved: recon-before-WO MUST cover the typecheck env (declare versus declare_overload) whenever a builtin
gains an overload — the standard pattern is that every `&0` overload goes through `declare_overload`, since a
regular binding swallows resolve_overload.**
This is a variant of [[feedback_verify_producer_before_consumer]]: a map drawn by O or Giang is also an
assumption until it touches the real compiler.

## 🦷 Teeth (O verified independently, trusting nothing in D's raw output)
The file `crates/triet-driver/tests/print_println_overload_subprocess.rs` — a subprocess fork guard (UB →
crashes the child) + a delegating counting free `__ppo_str_free` (count then really free, catching a double
dealloc) + assertions on BOTH the stdout content AND the FREE count. T1 owned FREE=1 · T2 ref reuse "x\nx\n"
FREE=1 · T3 no newline · T4 routing "a\nb\na\n" FREE=2.
**🩸 O re-poisoned T1 ITSELF:** the meta `__triet_println [true]→[false]` → the child crashed with
`free(): double free detected in tcache 2` (REAL glibc UB); restored from the cp snapshot with md5
`bade48f3` matching, and it went green again.
**T4's failure mode differed from the prediction** (O guessed "a leak, FREE==0"; the reality was a compile
refusal through the marshalling guard `arg_ty.is_reference()`) — D reported what it actually observed
([[feedback_failure_mode_precision]]), the tooth still went red, and refuse-over-guess is better.

## ⚖ D = a Sonnet 5 subagent (spawned by O on G's order)
It STOPPED correctly per LAW 4 twice (both were REAL blockers, not invented), honestly reported main.rs (the
WO had named the wrong file), reported T4's divergence, produced 0 fabrications, and restored via cp per the
law (NEVER git checkout). O gatekept with blood: the git state, a soundness code review, a real end-to-end run
(stdout appeared, and `s` was reused across 2 println_ref calls with a single Drop), its own gate, and the T1
re-poison. **The decisive point: O ran the gate itself and closed T1 itself, never signing off on D's raw output.**

## Deferred debts (bottom of the ledger, not opened)
`read_line` (input) · f-strings / runtime formatting · the buffering policy (line-buffered versus unbuffered).

→ [[campaign_str0_coverage_and_triage]] (the previous session, `&0 String` coverage — the same
declare_overload pattern)
