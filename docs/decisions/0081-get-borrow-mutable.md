# ADR-0081 — Get-Borrow-Mutable from Container (`get(&0 mutable c, k) → (&0 mutable V)?`)

- **Status:** ❄️ **FROZEN / DEFERRED (Mentor G order 2026-07-04).** WO A2 CANCELLED. Moved to **Cluster D (Phase 3: Ownership — sub-path reassign)**. Reopen ONLY IF core handles `deref-assign` (`*ref = new_val`) + safe handle update (drop-in-place via pointer). Reason for freeze: §7 below.
- **Date:** 2026-07-04
- **Deciders:** Author (Giang) · Mentor O · Mentor G
- **Notes:** Analysis §1-§6 remains UNCHANGED (valid upon reopening). The borrowck architecture (Q1: `returns_borrow_form`; core exclusive-loan conflicts even with READ) is correct — the issue is the API will be VACUOUS on the current functional-mutate core, not a design error in the loan.
- **Supersedes / extends:** ADR-0079 (get-borrow READ-ONLY) — this is the mutable twin.
- **Related:** ADR-0022 (S6 5-form reference), ADR-0059 (`&0` stack-borrow), ADR-0077/0078 (typed containers).

> **This is an ADR-lite: it does NOT launch a campaign.** The sole goal is to answer 2 soundness questions posed by G and finalize the P1 SCOPE before D is granted WO. A1 (get-borrow READ generic-V) proceeds in parallel via direct WO, NO ADR required — because it does not touch the borrowck core (loan remains read-shared, propagated). A2 requires an ADR because it directly impacts the heart of the borrow-checker: the loan must be **exclusive**.

---

## 1. Context

ADR-0079 enables read-side borrowing: `get(&0 c, k) → (&0 V)?`, with a **
