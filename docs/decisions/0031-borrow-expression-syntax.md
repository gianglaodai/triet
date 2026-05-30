# ADR 0031 — Borrow Expression Syntax (call-site `&+`/`&0`/`&-`)

**Trạng thái:** **Locked** (v0.9.x.atomic.7a, author sign-off 2026-05-30 "Phương án A"). Closes [SPEC §10](../../SPEC.md) gap noted in v0.7-era warning ("runtime chưa expose references; cú pháp §10.1—§10.4 phản ánh design intent, không phải hành vi hiện tại của compiler"). Enables [ADR-0028 §6](0028-atomic-primitive.md) example `let mutable counter = sys.atomic.new(0); spawn(&+ counter)` to type-check and run end-to-end.

> **Lock context (2026-05-30 audit):** Author review surfaced 4 issues before lock:
> 1. **Operand scope `vec[i]`** — `[i]` array-style index syntax doesn't exist in Triết parser (only `TupleIndex` `.0`). §2 originally listed `index` speculatively. **Resolution:** removed — IDENT + field-access only for v0.9.
> 2. **Bare `&` form** — CLAUDE.md table phrasing implied a 6th form. SPEC §10.1 + ADR-0022 §2 both explicit "**Năm dạng** reference". **Resolution:** §6 confirms 5 forms total; CLAUDE.md gets clarification fix same commit.
> 3. **Type-only ship trước enforcement** — original §4 deferred ALL borrow check to v0.9.x.borrow.*. Demo would silently accept double-move + use-after-move patterns, teaching wrong semantics, breaking v0.10. **Resolution (Phương án A):** wire up **E2420 UseAfterMove real fires** in v0.9 as new sub-task `.7d`. NLL (E2440) + lifetime elision (E2400) + `&-` upgrade (E2403) still defer v0.10 per ADR-0025 corpus-driven note. §10 captures v0.10 backlog.
> 4. **Demo multi-worker semantics** — `&+ Atomic<T>` multi-share works via refcount bump on Send boundary per ADR-0026 v2 §3.2. Single-thread VM (ADR-0028 §9) has no real Send crossing — multi-worker demo inherently v0.10 territory. **Resolution:** scope `.7e` demo to single fetch_add call, comment-document multi-worker as v0.10 backlog. Exercise ADR-0028 §9 test gate item 2 ("round-trip correctness, not concurrency") exactly.

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

## §2 — Operand scope: identifier + field access v0.9

**Decision:** v0.9 operand grammar:

```
operand := IDENT                          # &+ counter
         | operand '.' IDENT              # &+ obj.field
```

**Deferred to v0.10+ (corpus-driven; tracked §10):**

- **Array-style index `vec[i]`** — Triết parser does NOT currently have `[i]` index expression (only `TupleIndex` `pair.0` exists). Vector access goes through `get(vec, i)` method per `triet_chained_get_unwrap.md` user memory. Until `vec[i]` syntax itself ships, `&+ vec[i]` is moot.
- **Function call result borrow** (`&+ make_thing()`) — requires lifetime-extension semantics. Borrow-from-rvalue corner case Rust solves via temporary materialization; Triết defers until real use case.
- **Compound binary expressions** (`&+ (a + b)`) — semantically dubious (borrowing a computed value); refuse-over-guess.
- **Method call result** (`&+ obj.method()`) — same lifetime-extension concern.
- **Nested borrow expression** (`&+ &0 x`) — refused by typecheck (cannot borrow a borrow at expression level).

**Rationale:**

- Identifier + field-access covers ADR-0028 §6 example syntax + `.7e` demo needs exactly.
- Index operand (`vec[i]`) removed from v0.9 scope per 2026-05-30 audit — Triết doesn't have array-index syntax yet.
- Lifetime-extension cases push borrow checker complexity beyond ADR-0025 staged plan.
- Conservative scope per project philosophy "Refuse over guess" (VISION §6).

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

**Borrow checker enforcement v0.9 — split per Phương án A (2026-05-30):**

| Rule | v0.9 status | Where |
|---|---|---|
| **E2420 UseAfterMove** (consume-once) | ✅ **SHIPS v0.9 — fires real** | Sub-task `.7d` per §9 |
| E2440 NLL borrow exclusivity | ⏸️ Defers v0.10 (corpus-driven) | §10 backlog |
| E2400 Lifetime elision 3 rules | ⏸️ Defers v0.10 | §10 backlog |
| E2403 `&-` weak observer upgrade | ⏸️ Defers v0.10 | §10 backlog |
| E2410/E2411 Mutability violations | Skeleton only (pre-existing) | §10 backlog |

**Why E2420 ships v0.9 (not defer-all):** Without consume-once enforcement, `.7e` atomic_counter demo silently accepts `&+ counter; &+ counter` double-move pattern — teaches wrong semantics, breaks v0.10 when E2420 fires real. Author principle "chậm mà chắc, không ship tạm bợ" (2026-05-30): code shipping in v0.9 must compile + run with same semantics in v0.10. E2420 is the minimum check needed.

**Why NLL/lifetime defer v0.10:** Per ADR-0025 staging: "full NLL enforcement defer v0.9 (cần real-world Triết corpus)". `.7e` demo single-call pattern doesn't exercise NLL (no overlapping borrows) or lifetime elision (no escaping refs). E2440/E2400/E2403 implementation without corpus risks design rework. v0.10 corpus = full self-host + multi-thread stdlib + capability demos.

**Forward-compat guarantee:** Any v0.9 program that compiles via E2420 will continue compiling in v0.10 with the same semantics. NLL adds REJECTION of previously-passing patterns (overlapping borrows that v0.9 didn't catch) — but `.7e` demo doesn't trigger any such pattern.

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

## §6 — Form coverage v0.9: all 5 (no bare `&` exists)

**Decision:** All 5 forms (`&+`, `&+ mutable`, `&0`, `&0 mutable`, `&-`) ship in v0.9 expression syntax. NOT subset.

**No 6th "bare `&`" form.** Per SPEC §10.1 ("**Năm dạng** tham chiếu") + ADR-0022 §2 ("**Năm dạng** reference (lock cú pháp)"), Triết has exactly 5 reference forms. CLAUDE.md table previously contained phrasing `&+ T, &0 T, &- T, & (ownership reference; longest-match before &&)` which could be misread as 6 forms; the standalone `&` was actually noting the lexer longest-match rule against `&&` logical-AND, NOT a separate form. **Action:** CLAUDE.md row rephrased same commit per audit lock context.

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

## §9 — Implementation sub-phase plan (v0.9.x.atomic.7) — revised Phương án A

| Sub-task | Scope | Files |
|---|---|---|
| `.7a` (done) | Design lock — ADR-0031 Locked, scope refinements, v0.10 backlog | `docs/decisions/0031-*.md` + README + by-topic + TODO restructure + CLAUDE.md bare `&` fix |
| `.7b` | Rust impl borrow expression syntax | `triet-syntax/src/expr.rs` (Expr::Borrow AST variant) + `triet-parser/src/expr.rs` (prefix rule per §3 precedence) + `triet-typecheck/src/check/exprs.rs` (Type::Reference emission) + `triet-ir/src/lowerer.rs` (passthrough) + per-crate tests (parser × form × operand-kind; typecheck per form; lowerer passthrough proof) |
| `.7c` | Self-host Layer A port | `compiler/parser/parser.tri` (Expr variant + prefix rule mirroring `.7b`) + bootstrap symmetry test extension via `release-check.sh` |
| `.7d` | **E2420 UseAfterMove real fires** (Phương án A enforcement minimum) | `triet-typecheck/src/check/exprs.rs` move-tracking (CFG walk over function body, mark binding state alive/moved on move site, fire E2420 on use of moved binding) + tests (positive: `&+ x` then `x` use fires; negative: single-use clean; mixed `&0`/`&+` cases) |
| `.7e` | Demo runtime + e2e | `examples/atomic_counter/atomic_counter.tri` single-call scope (let counter = new(0); let prev = fetch_add(&+ counter, 1, Synchronized); println prev) + `crates/triet-cli/tests/atomic_counter_e2e.rs` asserting `Counter previous: 0` output + comment-document multi-worker v0.10 backlog |
| `.8` | Phase verify gate | cargo test + clippy + fmt + release-check.sh + ROADMAP/TODO archive |

Each sub-task = independent commit per CLAUDE.md cadence. `.7b` is the largest (~5 crates × small change each). `.7d` is medium (CFG walk for move tracking, ~400 LOC). `.7e` is small (demo file + 1 e2e test).

---

## §10 — v0.10 backlog revealed by this ADR

Following items surfaced during ADR-0031 design + 2026-05-30 audit. **Tracked here** so v0.10 phase opening picks them up; each item cross-links the source ADR/section that locked it.

### 10.1 — Borrow checker remaining enforcement (per ADR-0025 staged plan)

- **E2440 NLL borrow exclusivity (full CFG live-range)** — per [ADR-0025 §2](0025-borrow-checker-rules.md). Compute borrow-active region from creation to last-use; reject overlapping `&0 mutable` / `&0` / `&+` borrows. Scope: ~1000+ LOC, CFG-based live-range analysis. Trigger: when v0.9 corpus (self-host + atomic + JIT phases) exposes real overlap patterns.
- **E2400 Lifetime elision 3 rules** — per [ADR-0025 §3](0025-borrow-checker-rules.md). Implement quy tắc 1 (single input borrow → output), quy tắc 2 (`self` receiver → output ties self), quy tắc 3 (owned return). Scope: ~300 LOC + tests. Trigger: when function signature corpus produces ambiguous elision cases.
- **E2403 `&-` weak observer upgrade tracking** — per [ADR-0022 §2 row 5](0022-trit-balanced-ownership.md). Deref `&- T` → `T?` (nullable); compile-time tracked. Scope: ~200 LOC. Trigger: when first stdlib needs weak refs (likely concurrency primitives or doubly-linked structures).
- **E2410/E2411 Mutability violation enforcement** — skeletons exist per v0.8.10. Full enforcement (assign-to-frozen, mutate-via-readonly-borrow). Trigger: when `&+ mutable` / `&0 mutable` usage corpus grows.

### 10.2 — Atomic primitive multi-thread completion (per ADR-0028 + ADR-0026 v2)

- **Real `raw_thread.spawn` implementation** — per [ADR-0026 v2 §3](0026-actor-boundary-send-rules.md). Replace v0.9 placeholder `function spawn(work: Integer) -> Handle = Handle { thread_id: 0 }` with real OS thread creation. `Handle.join()` block until real thread terminates.
- **Send boundary refcount-bump codegen** — per [ADR-0026 v2 §3.2](0026-actor-boundary-send-rules.md). When `&+ T` crosses spawn boundary, emit refcount-bump on ObjectHeader (`triet-core::memory`). User-visible: nothing changes; under the hood: multi-share enabled.
- **`&+ Atomic<T>` multi-thread clone semantics** — per [ADR-0028 §5](0028-atomic-primitive.md). Locked: "refcount-mediated share, race conditions resolved by Ordering". v0.10 wire up clone-on-Send-boundary path; single-thread `&+` stays linear move (consume-once) per v0.9 .7d enforcement.
- **`atomic_counter` demo multi-worker upgrade** — per `.7e` v0.9 comment. Reactivate the 3-worker pattern + final `load(&+ counter, ...)` once real spawn ships. Add concurrency assertion (counter eventually consistent ≥ 3 after all join).
- **`std.concurrency.*` stdlib** — per [ROADMAP §v0.10](../../ROADMAP.md). Mutex, Channel, M:N green threads. Built atop `sys.raw_thread` real implementation.

### 10.3 — Borrow expression operand scope expansion (deferred from ADR-0031 §2)

- **Function-call result borrow** (`&+ make_thing()`) — requires lifetime-extension semantics (Rust calls this "temporary materialization"). Decide rules + ADR amendment.
- **Method-call result borrow** (`&+ obj.method()`) — same lifetime concern as above.
- **Array-style index expression `vec[i]`** — independent of this ADR. Vector access currently via `get(vec, i)` only per `triet_chained_get_unwrap.md`. v0.10 (or whenever index syntax ships) extends `&+ vec[i]` operand.
- **Compound binary expressions** (`&+ (a + b)`) — explicitly refused v0.9 (refuse-over-guess). Re-evaluate if corpus surfaces use case; default stays refused.

### 10.4 — Atomic E2530 — Pointer-Relaxed `fetch_*` pattern (deferred from ADR-0028 §10)

- Per [ADR-0028 §10 pattern 2](0028-atomic-primitive.md): `fetch_add/sub/and/or/xor` with `Ordering.Relaxed` on `Atomic<Pointer>` should fire E2530 (Pointer is publish-like; Relaxed publish almost always wrong). **Blocked v0.9**: `Pointer` type doesn't parse. v0.10 when Pointer lands → wire up this E2530 conservative pattern alongside existing `compare_exchange` weaker-success check from `.6`.

### 10.5 — CLAUDE.md normative documentation drift

- **2026-05-30 audit found bare `&` row in CLAUDE.md confusing.** Fixed same commit as `.7a` lock (this commit). Pattern: CLAUDE.md table rows must spell out forms exhaustively, not hint at lexer rules ambiguously. Audit policy: when adding language convention rows to CLAUDE.md, cross-reference SPEC § to confirm form count + spelling.

### 10.6 — Self-host port lag tracking

- `.7c` ports borrow expression parsing to `compiler/parser/parser.tri` (Layer A per ADR-0029 §3 mandatory). v0.10 may discover that `.7d` E2420 enforcement also needs Layer A or Layer B port if self-host typecheck implementation lands. Currently self-host typecheck pass minimal; revisit when self-host typecheck phase opens (post-v0.9).

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
