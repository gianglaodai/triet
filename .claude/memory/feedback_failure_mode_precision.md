---
name: feedback_failure_mode_precision
description: "D dramatized a failure mode (claiming SIGSEGV when it was really a LEAK) — the technical claim must be exactly right, no Hollywood."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: cfce150f-cc26-451d-b933-ca98ee4f57ce
---

**G LOATHES dramatized failure modes. A compiler engineer must know EXACTLY what causes memory corruption versus what causes a leak — a wrong claim is worthless.**

**The specific case (WO-NullableFieldMoveOut, 2026-06-30):** D reported *"removing the Site-3 dest-type propagation → SIGSEGV"* (assuming it behaved like the Phase 2 Struct/Enum fields). O verified independently with blood: removing Site-3 for a heap `T?` (`String?`/`Vector?`/`HashMap?`) gives a **SILENT LEAK (FREE==0), NOT a SIGSEGV**. The technical reason: the destination becomes Unknown → the JIT drop glue goes blind (it does not recognize the heap type) → `Drop(Unknown)` is a no-op → it leaks; there is no memcpy through a garbage address. A SIGSEGV happens only for **Struct/Enum fields** (an aggregate memcpy through a garbage slot), never for a heap scalar or `T?` (an 8B scalar pointer copy into a variable).

**Why:** the site is still load-bearing (a leak is unsound, so the tooth still goes red correctly) and the code was NOT wrong — but claiming the wrong failure mode proves it was never verified with blood, only inferred from a different case. G: *"If you build compilers without knowing what causes a crash and what causes a leak, you are worthless. There is no room for Hollywood."*

**How to apply:** (1) O — do NOT trust the failure mode in D's report; plant your own poison and measure the actual signal (SIGABRT 134 double free vs SIGSEGV 139 bad deref vs FREE==0 leak vs FREE==2 double-free count — four DIFFERENT signals, never conflate them). (2) Distinguish: an aggregate (Struct/Enum) field with no slot → SIGSEGV (memcpy through garbage); a heap-scalar or `T?` field with no slot → a LEAK (blind drop glue). (3) When writing a WO or teeth, state the expected failure mode correctly — if you guess SIGSEGV when it is really a leak, only a counting harness will catch it, a crash fixture will NOT. [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[mentor_o_persona]]
