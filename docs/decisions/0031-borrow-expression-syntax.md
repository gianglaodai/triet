# ADR 0031 — Borrow Expression Syntax (call-site `&+`/`&0`/`&-`)

**Trạng thái:** **Draft** (v0.9.x.atomic.7a, 2026-05-30). Pending author sign-off. Closes [SPEC §10](../../SPEC.md) gap noted in v0.7-era warning ("runtime chưa expose references; cú pháp §10.1—§10.4 phản ánh design intent, không phải hành vi hiện tại của compiler"). Enables [ADR-0028 §6](0028-atomic-primitive.md) example `let mutable counter = sys.atomic.new(0); spawn(&+ counter)` to type-check and run end-to-end.

**Issue:** v0.9.x.atomic.7 cần demo `atomic_counter` chạy thật trên VM (3× `fetch_add` → counter == 3) per ADR-0028 §9. Stdlib `sys.atomic` signatures bắt buộc `&+ Atomic<T>` parameters per ADR-0028 §5 (interior mutability — frozen owner). Nhưng `let counter = new(0)` produces plain `Atomic<Integer>` per ADR-0028 §6 (constructor returns owned value, not borrowed reference). Triết v0.8 lex được `&+`/`&0`/`&-` tokens trong **type expression** context only — không có expression-level borrow syntax. Hệ quả: `spawn_worker(counter)` raises E1003 mismatch, demo không thể call site nào ngoài type-level paperwork.

3 path xuất hiện:
1. Implicit T → &+ T auto-coerce trong `Type::matches()`. Smallest scope, đảo cách Rust làm function calls (ownership move), risks loosening invariant.
2. Đổi `sys.atomic.new` return `&+ Atomic<T>` direct. Breaks ADR-0028 §6 signature; vẫn không cover trường hợp khác (existing `Atomic<T>` value cần share).
3. **Implement expression-level `&FORM expr` borrow syntax** — author's choice 2026-05-30 (the cleanest answer; aligns with ADR-0022 §4.2 spirit "compiler tự động borrow" + ADR-0028 §6 example syntax).

Open questions ADR-0031 phải lock:

1. **Operand scope** — bare identifier? full expression? bound somewhere middle?
2. **Precedence** — đâu trong pratt table?
3. **Semantics** — `&+ x` move-or-copy? Borrow checker enforcement timing?
4. **Form coverage** — all 5 forms (`&+`, `&+ mutable`, `&0`, `&0 mutable`, `&-`) hay subset v0.9?
5. **Self-host port** — Layer A (lockstep mandatory) per ADR-0029?
6. **Test gate** — minimum to ship v0.9.

---

## §1 — Syntax: prefix `&FORM operand`

**Decision:** Borrow expression has prefix syntax mirroring type-form prefix:

```
borrow_expr := '&+' ['mutable'] operand    # strong frozen | strong mutable
             | '&0' ['mutable'] operand    # scope read-only | scope exclusive
             | '&-' operand                # weak observer
```

Tokens already exist (lexer ships `AmpersandPlus`/`AmpersandZero`/`AmpersandMinus` since v0.8.x.review.3). Parser disambiguates context: type-expr position uses `try_parse_reference_prefix` (existing); expression position uses **new** `parse_borrow_prefix` (this ADR).

**Rationale:**

- Mirrors type-form syntax exactly — author reads `&+ counter` either as "borrow expression" hay "&+ T type", same prefix structure.
- No new tokens needed — incremental over v0.8 lexer.
- All 5 forms covered uniformly — no asymmetry where one form is expression-only.

---

## §2 — Operand scope: identifier + field access + index v0.9

**Decision:** v0.9 operand grammar:

```
operand := IDENT                          # &+ counter
         | operand '.' IDENT              # &+ obj.field
         | operand '[' expression ']'     # &+ vec[i]
```

**Deferred to v0.10+ (corpus-driven):**

- Function call result borrow (`&+ make_thing()`) — requires lifetime-extension semantics. Borrow-from-rvalue corner case Rust solves via temporary materialization; Triết defers until real use case.
- Compound binary expressions (`&+ (a + b)`) — semantically dubious (borrowing a computed value); refuse-over-guess.
- Method call result (`&+ obj.method()`) — same lifetime-extension concern.
- Nested borrow expression (`&+ &0 x`) — refused by typecheck (cannot borrow a borrow at expression level).

**Rationale:**

- Identifier / field / index covers 95%+ of practical borrow sites (per Rust corpus analogy).
- Lifetime-extension cases push borrow checker complexity beyond v0.9 .borrow.* scope.
- Conservative scope per project philosophy "Refuse over guess" (VISION §6).
- Matches author's atomic_counter demo needs exactly (identifier operand only).

---

## §3 — Precedence: prefix unary tier

**Decision:** Borrow prefix sits at the same precedence tier as `!` / `not` / unary `-` / `~+` / `~-` / `~0`. **Right-binding** (consistent with other prefix ops). Lower than postfix `.field`/`[i]`/`(args)`/`?`/`!!` (so `&+ obj.field` parses as `&+ (obj.field)`, NOT `(&+ obj).field`). Higher than every binary operator.

Pratt table position (descending):

```
1.  field access `.`           )
2.  method call `.method(...)` )  postfix tier (left-binding)
3.  index `[...]`              )
4.  call `(...)`               )
5.  Nullable `?` / `!!`        )
6.  Outcome `~?` / `~:`        )
─────────
7.  Unary prefix tier:  `&+`/`&0`/`&-` | `!` | `not` | unary `-` | `~+`/`~-`/`~0`
─────────
8.  multiplicative / additive / shift / comparison / logical / assignment
```

**No ambiguity với `&&` (logical AND):** `&&` is a distinct lexer token, not 2×`&`. Longest-match per ADR-0022 (S6 lexer rule).

**No ambiguity với type-position prefix:** Parser dispatch by context — type-position uses `try_parse_reference_prefix` after type keyword, expression-position uses new prefix rule.

---

## §4 — Semantics per form

**Decision:** Each form produces a `Type::Reference(form, T)` value at type level. Runtime erases (refs vanish at IR/VM level — `RuntimeValue` doesn't track form).

| Form | Type result | Ownership effect on operand | Notes |
|---|---|---|---|
| `&+ x` | `Reference(StrongFrozen, T)` | Move (x consumed, E2420 if used after) | Frozen post-borrow |
| `&+ mutable x` | `Reference(StrongMutable, T)` | Move (x consumed) | Owner mutable |
| `&0 x` | `Reference(BorrowReadOnly, T)` | Borrow (x lives, scope-bounded) | Multiple OK |
| `&0 mutable x` | `Reference(BorrowExclusiveMutable, T)` | Borrow (x lives, scope-bounded) | Exclusive — NLL enforced |
| `&- x` | `Reference(WeakObserver, T)` | Track (x's lifetime independent) | Upgrade-on-deref → `T?` |

**Borrow checker enforcement (defers v0.9.x.borrow.*):** v0.9 .atomic.7 ships **typecheck-only** form distinction — Type::Reference correctly emitted, but consume-once / exclusivity / weak-upgrade rules NOT enforced. E2420 / E2440 emission fires real per ADR-0025 sub-phases.

**Why type-only first:** Atomic demo needs Type::Reference correctness so signatures match. Borrow rule enforcement is orthogonal — staged per ADR-0025 explicit defer ("enforcement defers until real-world corpus first").

---

## §5 — Lowerer + VM: passthrough

**Decision:** Lowerer emits IR identical to bare operand. References erase entirely.

```rust
// triet-ir lowerer (pseudocode):
Expr::Borrow { operand, .. } => self.lower_expr(operand)
```

VM treats `&+ counter` exactly like `counter` — both produce the same `RuntimeValue::Atomic(Rc<RefCell>)` instance. Per ADR-0026 v2 §7 ObjectHeader scheme + ADR-0022 §6 acyclic theorem, no runtime distinction needed.

**Implication:** Demo `spawn_worker(&+ counter)` runs as `spawn_worker(counter)` would — both reach VM dispatch with the same atomic cell. Single-thread VM dev tier (ADR-0028 §9) increments correctly: 3× `fetch_add(counter, 1, Synchronized)` → counter == 3.

---

## §6 — Form coverage v0.9: all 5

**Decision:** All 5 forms (`&+`, `&+ mutable`, `&0`, `&0 mutable`, `&-`) ship in v0.9 expression syntax. NOT subset.

**Rationale:**

- Parser cost amortized — single prefix rule handles all 5 (loop dispatch on token + `mutable` lookahead).
- Type system already supports all 5 — no new `Type::Reference` variants needed.
- Demo only exercises `&+` — but rejecting `&0`/`&-` would create asymmetric UX where users wonder why they can write the type but not the expression.
- Test corpus needs only 5-form parser + typecheck tests — small marginal cost vs. delivering full surface.

---

## §7 — Self-host port: Layer A (lockstep)

**Decision:** Layer A per [ADR-0029 §3](0029-self-host-port-policy.md). Parser surface change → `compiler/parser/parser.tri` lockstep port mandatory.

**Files affected (Triết self-host):**

- `compiler/parser/parser.tri` — add `BorrowExpr { form, operand }` payload struct + `Expr::BorrowExpr` variant + prefix rule.
- AST symmetry: `compiler/parser/lexer.tri` already covers tokens per v0.8.x.review.3.

**Port timing:** within same sub-task as Rust impl (v0.9.x.atomic.7c per phasing — `.7b` Rust, `.7c` self-host port + bootstrap gate).

**Bootstrap impact:** Stage 2 ≡ Stage 3 byte-identical gate (per ADR-0019 §7) — port must ship before gate re-armed in v0.9.final.

---

## §8 — Test gate for v0.9 close

1. **Lexer:** existing `&+`/`&0`/`&-` token tests (v0.8.x.review.3) — no change.
2. **Parser:** new test cases per form × per operand kind (identifier / field / index) — 5×3 = 15 minimum, plus negative cases (function call operand refused, compound binary refused, nested borrow refused).
3. **Typecheck:** each form produces correct `Type::Reference(form, T)`; rejects when operand is `Unit` / `Function` / already-Reference; refuses when borrow-of-borrow attempted.
4. **Lowerer:** assert IR emitted matches bare-operand IR (passthrough proof).
5. **VM:** existing atomic dispatch tests (v0.9.x.atomic.3/.4) — no new VM tests needed.
6. **Demo end-to-end:** `atomic_counter` runs, output asserts `Counter after 3 increments: 3`.
7. **Self-host symmetry:** existing `release-check.sh` Token/TypeExpr symmetry gates extend to cover Expr enum.

---

## §9 — Implementation sub-phase plan (v0.9.x.atomic.7)

| Sub-task | Scope | Files (Rust crates) |
|---|---|---|
| `.7a` (this ADR) | Design lock | `docs/decisions/0031-*.md` + `docs/decisions/README.md` + `docs/decisions/by-topic.md` + TODO restructure |
| `.7b` | Rust impl + tests | `triet-syntax/src/expr.rs` (AST variant) + `triet-parser/src/expr.rs` (prefix rule) + `triet-typecheck/src/check/exprs.rs` (Type::Reference emit) + `triet-ir/src/lowerer.rs` (passthrough) + per-crate tests |
| `.7c` | Self-host Layer A port | `compiler/parser/parser.tri` (Expr variant + prefix rule) + self-host symmetry test extension |
| `.7d` | Demo runtime + e2e | `examples/atomic_counter/atomic_counter.tri` upgrade + new `crates/triet-cli/tests/atomic_counter_e2e.rs` |

Each sub-task = independent commit per CLAUDE.md cadence. `.7b` is the largest (~6 crates × small change each).

---

## Hệ quả

**Possible (positive):**

- ADR-0028 §6 example syntax now real code, not aspirational doc.
- Demo `atomic_counter` exercises full VM dispatch path — load/fetch_add round-trip verifiable.
- Future stdlib (Mutex v0.10, std.concurrency.* v0.10+) can use `&+`/`&0`/`&-` cleanly without piecemeal type loosening.
- Closes SPEC §10 warning ("runtime chưa expose references") — refs now first-class at parser+typecheck level.

**Constrained (cost):**

- 6 Rust source files touched (~200 LOC net) + ~150 LOC self-host port.
- Parser test surface expands (~15 new tests).
- Borrow checker NLL enforcement still defers v0.9.x.borrow.* — short window where user can write borrow expressions without consume-once enforcement (acceptable per ADR-0025 staged plan).

**Costly (verify):**

- Pratt precedence ambiguity with `&&`: lexer longest-match should prevent — verify with stress tests.
- Self-host bootstrap performance — Stage 2/3 parser path adds prefix rule, marginal impact.

---

## Không làm (explicitly rejected)

- **Implicit T → &+ T auto-coerce trong `Type::matches()`** (original option 2). Cleaner short-term but blurs borrow rules — refuse-over-guess (VISION §6). Author rejected 2026-05-30.
- **Constructor returns `&+ Atomic<T>` direct** (original option 3). Breaks ADR-0028 §6 signature lock; doesn't cover all share scenarios. Author rejected 2026-05-30.
- **Borrow from function call result / compound expression** (operand scope reduction). Lifetime-extension semantics unresolved; defer until corpus.
- **Per-form precedence variation.** All 5 forms same precedence tier; no PEMDAS-like inconsistency.
- **Auto-deref for borrow ops** (e.g. `(&+ x).field == x.field`). Defer to method dispatch ADR (post-v1.0). v0.9 ships explicit forms only.

---

## Prior art

| Source | What we copy | What we change |
|---|---|---|
| Rust `&x` / `&mut x` | Prefix borrow syntax; precedence | Triết has 5 forms (vs. Rust 2); explicit `&+`/`&0`/`&-` polarity vs. Rust's `&`/`&mut` |
| OCaml `ref x` / `!x` | Distinguished reference vs. deref ops | Triết uses prefix `&FORM`; ref-deref doesn't exist at expression level (auto-handled at type level) |
| Swift `inout` parameter | Mutable borrow at call site | Triết uses `&0 mutable` at borrow site; type carries form |
| C++ `&x` (address-of) vs. `int&` (reference type) | Same symbol, different meaning by position | Triết same — `&+` in type vs. expression position |

**What we invented:**

- **Ternary form polarity in expression position** — `+`/`0`/`-` Trit-aligned reference forms (existing in types) extend to expressions uniformly.
- **All-5-forms at expression level** — Rust has 2 (`&`/`&mut`), Triết keeps full 5 symmetric to type expression.

---

## Tham chiếu

- [ADR-0022](0022-trit-balanced-ownership.md) — S6 ownership model + 5 reference forms (parent design).
- [ADR-0022 §4.2](0022-trit-balanced-ownership.md) — "Compiler tự động borrow `&+` thành `&0`" rationale (implicit borrow direction).
- [ADR-0025](0025-borrow-checker-rules.md) — borrow checker rules (consumes-once enforcement, deferred v0.9.x.borrow.*).
- [ADR-0026 v2 §7](0026-actor-boundary-send-rules.md) — ObjectHeader scheme (references erase to runtime cell).
- [ADR-0028 §5 + §6](0028-atomic-primitive.md) — `&+ Atomic<T>` interior mutability + constructor signature (the trigger for this ADR).
- [ADR-0029 §3](0029-self-host-port-policy.md) — Layer A lockstep rule (parser surface = mandatory port).
- [SPEC §10](../../SPEC.md) — Memory model (references + ownership; warning about v0.7 runtime gap that this ADR closes).
- [VISION §6](../../VISION.md) — "Refuse over guess" philosophy (operand scope conservatism).
