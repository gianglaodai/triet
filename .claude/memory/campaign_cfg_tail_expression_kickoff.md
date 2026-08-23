---
name: campaign_cfg_tail_expression_kickoff
description: ✅ FULLY CLOSED 2026-06-18 — the CFG Tail-Expression campaign (ADR-0055) came down. Slice 1 SIGILL + Slice 2 `= ~0` are done and committed. NEXT = Heap-Nullable (recon at the end of this file).
metadata: 
  node_type: memory
  type: project
  originSessionId: 2e8fd692-48b0-4f38-b76d-815d7e054b83
---

**✅ THE CFG TAIL-EXPRESSION CAMPAIGN CAME DOWN (declared by G, 2026-06-18).** 4 local commits (not yet pushed), origin still at `667ea24`:
- `4d51faa` the Slice 1 fix + `82863ed` docs — **Slice 1, SIGILL 132**: a free or trait function returning a flat struct through an expression body emitted `Return(struct)` by value → SIGILL. The fix = an SSOT helper `emit_struct_sret_copy` (triet-lower/src/lib.rs) routing the tail Return through sret exactly like `Stmt::Return`; both callers were DRY'd (G's mandate: "one valve, one wrench"). Teeth 182/183/184, poison→SIGILL. Only `MirType::Struct` was broken (String/Vector/heap-Outcome were already correct because they share the `Return[local]` template).
- `a0eff46` the Slice 2 fix + `de450c6` docs — **Slice 2, the narrow-A `= ~0` case**: tail values were already wired by ADR-0055+0056/0057/0058 (O probed 20+ shapes, all running). Exactly ONE tail asymmetry remained: `= ~0` reported a lowerer error while `return ~0` ran → mirror the null-`~0` special case (Stmt::Return lib.rs:1265-1276) onto the tail path (starting at 807). The guard `!matches!(Outcome)` keeps the ternary `~0` going through OutcomeConstructor (fixture 133→100 verifies it). Fixtures 185-188. Final gate `0·0·183·0`.

**Campaign discipline (imposed by O):** probing before designing FLIPPED 2 stale recon premises ("a match tail wrongly returns 0" = FALSE on HEAD; "Slice 2 is a big job" = FALSE, it was already 95% done). O stabbed the boss with data twice → G praised it and withdrew the rulings. Gap #2 (`{ ~0 }` / an if-arm null fails identically in return and let positions — a type-propagation issue, NOT a tail asymmetry) → pushed into the Heap-Nullable backlog to prevent scope creep. G ruled that DRY'ing the null sentinel should be LEFT INLINE ("a little copying is better than a little dependency" — 3 lines assigning a constant ≠ a 15-line sret valve).

## ★ NEXT — recon for the Heap-Nullable campaign (dug by O on 2026-06-18; do not dig again)
The only large open backlog item = the **Heap-Nullable saga, ~5 slices** (`T?` where T is heap: String/Vector/HashMap/Struct/Enum).
- **The current gate:** `Body::verify()` triet-mir/src/lib.rs:1440-1464 refuses with `HeapNullableNotLowered`. The chokepoint covers returns, locals, struct fields, and enum payloads; `find_heap_nullable` (1380) recurses through Nullable/Reference/Outcome; `is_scalar_nullable_payload` (1362) whitelists Integer/Trit/Tryte/Long/Trilean/Unit/Unknown. Ruling β (signed by G): the gate lives in the LOWERER, not typecheck — because the stdlib declares heap nullables as API stubs (`env.get`/`fs.read -> String?`); a declaration is harmless, only compilation is refused.
- **Scalar `T?` works:** the sentinel `NULL_SENTINEL = i64::MIN` (triet-mir lib.rs:2334), with the N1 canary below every scalar range.
- **★ The (a) ptr-sentinel foundation PARTLY EXISTS at runtime:** the heap shims treat `ptr == NULL_SENTINEL` as null/no-op everywhere (mir_lower.rs:2198 string, 2470/2693 hashmap, 4024; the test `__triet_string_free(NULL_SENTINEL)` is a no-op at @4786, and get-OOB / key-miss return NULL_SENTINEL at @2575/2848). → Slice 3 (conditional Drop in the JIT) already has a free-is-a-no-op-on-null foundation. The campaign does NOT start from zero.
- **The 5-slice TODO:** (1) an ADR for representation (a), a ptr-sentinel slot `{ptr,len,cap}` where `ptr==SENTINEL` means null, with the null check projecting `.ptr` rather than comparing the whole slot · (2) widening String→String? + `~0` materializing the ptr sentinel · (3) conditional Drop in the JIT · (4) Elvis `?:` + `match ~+/~0` for heap types (project `.ptr`, move the payload) · (5) `?+>` map/flatMap for heap types (Deinit/tombstone to avoid a double free) · then remove the gate. **G leans toward representation (a).** It needs **an ADR first** (locking the representation) before any typing, because it is design-heavy.

[[mentor_o_persona]] [[colleague_d_persona]] [[lang_return_keyword_survives]]
