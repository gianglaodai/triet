# ADR-0046: Return-borrow Elision — Phase 3

**Status:** ACCEPTED — O + G signed 2026-06-08
**Date:** 2026-06-08
**Author:** AI (colleague D, implementer)
**Reviewers:** Mentor O (semantics, soundness) — SIGNED 2026-06/06/08 · Mentor G (layout, ABI, codegen) — SIGNED 2026-06-08
**Scope:** Enable `fn id(s: &0 T) -> &0 T { return s }` — callee returns a borrowed reference from a param, caller's owner is frozen while the ref is alive. Based on the existing PropagatedLoan infrastructure from ADR-0045 §4.

---

## Summary

ADR-0045 §5 CUT `-> &0 T` return type using E1042 to seal the return-borrow phase. This phase reopens it: typecheck allows `-> &0 T`, lower populates `return_borrow_map` (the only broken link), and the PropagatedLoan engine (available in the checker from ADR-0045 §4) re-issues the loan at the caller → owner is frozen while the reference is alive, preventing use-after-free.

Phase-0 empirical probe: 3/4 of the infrastructure is already available. The only remaining vulnerability is the `return_borrow_map` being empty at the lowerer (lib.rs:168). The only code required: populate that map + open the E1042 gate for `&0`.

---

## §0 — Facts

| # | Fact | Location |
|---|------|----------|
| F1 | Elision decision (0/1/multi ref-param) is already available. `check_lifetime_elision` (check.rs:494) emits E2400 when count ≠ 1. | `check.rs:494-551` |
| F2 | Dangling return `&0 <local>` is already caught by E2450. The `storage-end` of the local terminates the loan → returning a reference to the local is rejected. | `borrowck/checker.s` (E2450) |
| F3 | PropagatedLoan engine is already available in the checker (`checker.rs:754-796`), with unit test `returned_reference_extends_source_lifetime` (checker.rs:1384). The engine is LIVE-in-test but DEAD-in-production because `return_borrow_map` is always empty (F5). | `checker.rs:754-796`, `checker.rs:1384` |
| F4 | The driver has wired `callee_sigs` + `check
