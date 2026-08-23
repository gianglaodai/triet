# ADR-0056 — Heap Value-Merge: type the if/match merge result so Fat-Pointers survive

- **Status:** 🔒 LOCKED — Approved by G 2026-06-11. Drafted by Mentor O 2026-06-11, grounded from spike if-heap.
- **Date:** 2026-06-11
- **Drafted by:** Mentor O (analyzed `Expr::If`/`Expr::Match` lowering + spike type-result).
- **Signatures:** O ✅ (grounded from probe B1/B2 + spike revert sha-identical) · G ✅ (approved + stamped 2/11/2026).
- **Related:** [ADR-0055 §8](0055-block-body-tail-expression.md) (descope source — exposing 3 branch-merge slots), [ADR-0049](0049-fat-pointer-abi.md) (Fat-Pointer {ptr,len,cap}). **Sealed for [ADR-0057]:** Outcome value-merge + Outcome-let-binding (NOT part of this ADR).

---

## 1. Context — if/match constrains the expressionality of Fat-Pointer data

Expression-based philosophy: `if`/`match` MUST evaluate to a value. Currently, with
**Fat-Pointer** (String/Vector — 24-byte heap `{ptr,len,cap}`), branch values are
**truncated to 1 word (only `ptr`)** when merged into the common result → garbage len/cap:

```triet
function pick(n: Integer) -> String = if n == 0 { "xx" } else { "yyy" }
// length(pick(0)) → 0/garbage instead of 2
```

This was revealed during the ADR-0055 (§8 descope) teeth tests. The bug is independent of ADR-0055 (reproducible in
`= if…/= match…` prefix).

## 2. Root cause — MEASURED FROM CODE + SPIKE, NOT GUESSED

Two constructs share the same flaw (`triet-lower/src/lib.rs`):

| Construct | Result alloc | Branch write |
|---|---|---|
| `Expr::If` merge | `lib.rs:2201` `c.alloc_local()` **untyped** | `Assign{result, then_val}` (2205) · `{result, else_val}` (2221) |
| plain enum `Expr::Match` merge | `lib.rs:3082` `c.alloc_local()` **untyped** | `Assign{result, body_val}` (3179 EnumVariant · 3205 unit · 3243 wildcard) |

**Two flaws:** (1) result local is **UNTYPED** → defaults to scalar i64; (2) written via
`Statement::Assign` 1-local → JIT lowers `_5 = move _4` as **1-word**. Heap loses len/cap.

**SPIKE scope conclusion (O probed and reverted, revert sha-identical):**
- JIT `Assign` IS type-aware: `let y: String = x` → `_1 = move _0` copies full
  `{ptr,len,cap}` → `length=4` ✓. **JIT already knows how to move Fat-Pointers WHEN the local is typed.**
- Type of if-merge result local from `then_val` → B1 if-heap **0/garbage → 2** (correctly "xx").

→ **Fix LOWER-ONLY. JIT branch-codegen DOES NOT need changes.** This is a critical finding:
JIT is already smart enough; the flaw is purely that the Lowerer forgets to attach the type.

## 3. Decision (G finalized scope — APPROVED 2026-06-11)

**LOCKED scope:** only Fat-Pointer (String/Vector) value-merge via **if** + **plain
enum match**. Solution: **inject the correct type for the `result` local from the branch value.**

**Site 1 — `Expr::If` (lib.rs:2201):** result assigned AFTER `then_val` → type at allocation:
```rust
let result = c.alloc_local_ty(c.local_decls[then_val.0].ty.clone());
```

**Site 2 — plain enum `Expr::Match` (lib.rs:3082):** result assigned BEFORE the arm loop →
cannot type at allocation. **Patch `result` type at the first write-site** (idempotent —
typecheck ensures all arms have the same type): at each `Assign{result, body_val}` (3179/3205/
3243) set `c.local_decls[result.0].ty = c.local_decls[body_val.0].ty.clone();` before
pushing the Assign. (Implementer may choose a helper or set-on-every-write — as long as the final type is correct.)

**Four inviolable boundaries:**
1. **ABSOLUTLY FORBIDDEN to touch JIT** (G's order). JIT typed-Assign is already correct — spike proves this.
2. **DO NOT touch nullable-match** (2605/2618 already uses `alloc_local_ty(payload_ty)` — already typed).
3. **DO NOT touch outcome-match** (2862 `Unknown`) — sealed for ADR-0057.
4. **Scalar merge MUST NOT regress** — type-from-branch applies uniformly; scalar remains i64.

## 4. Teeth (mandatory red→green — route-lower, DO NOT hand-build MirBuilder)

| Cell | Form | After fix | Poison-revert (result to untyped) |
|---|---|---|---|
| if-heap String | `= if n==0 {"xx"} else {"yyy"}` → `length` | 2 | 0/garbage 🔴 |
| if-heap Vector | `= if … {vec a} else {vec b}` → `length` | correct len | garbage 🔴 |
| enum-match-heap String | `= match c { Red=>"xx", Blue=>"yyy" }` → `length` | correct arm | garbage 🔴 |
| enum-match-heap Vector | match → Vector per arm | correct len | garbage 🔴 |
| **regression scalar** | if-scalar / match-scalar (fixtures 146/147) | still correct | — (must not break) |
| **regression ADR-0055** | 9 fixtures 143-151 | still green | — |

**NO Outcome cells** — Outcome-merge belongs to ADR-0057, and will remain red after this ADR; if
anyone adds an Outcome cell to teeth 0056 = out of scope, REJECT.

## 5. Implementation Order

1. Write teeth §4 (heap if
