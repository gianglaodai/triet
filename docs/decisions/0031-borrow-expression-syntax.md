# ADR 0031 — Borrow Expression Syntax (call-site `&+`/`&0`/`&-`)

**Status:** **Locked** (v0.9.x.atomic.7a, author sign-off 2026-05-30 "Option A"). Closes [SPEC §10](../../SPEC.md) gap noted in v0.7-era warning ("runtime does not yet expose references; syntax in §10.1—§10.4 reflects design intent, not current compiler behavior"). Enables [ADR-0028 §6](0028-atomic-primitive.md) example `let mutable counter = sys.atomic.new(0); spawn(&+ counter)` to type-check and run end-to-end.

> **Lock context (2026-05-30 audit):** Author review surfaced 4 issues before lock:
> 1. **Operand scope `vec[i]`** — `[i]` array-style index syntax does not exist in the Triet parser (only `TupleIndex` `.0` exists). §2 originally listed `index` speculatively. **Resolution:** removed — IDENT + field-access only for v0.9.
> 2. **Bare `&` form** — CLAUDE.md table phrasing implied a 6th form. SPEC §10.1 + ADR-0022 §2 both explicitly specify "**Five forms** of reference". **Resolution:** §6 confirms 5 forms total; CLAUDE.md receives a clarifying fix in the same commit.
> 3. **Type-only shipping before enforcement** — original §4 deferred ALL borrow checks to v0.9.x.borrow.*. The demo would silently accept double-move and use-after-move patterns, teaching incorrect semantics and breaking in v0.10. **Resolution (Option A):** wire up **real E2420 UseAfterMove errors** in v0.9 as new sub-task `.7d`. NLL (E2440) + lifetime elision (E2400) + `&-` upgrade (E2403) still defer to v0.10 per the ADR-0025 corpus-driven note. §10 captures the v0.10 backlog.
> 4. **Demo multi-worker semantics** — `&+ Atomic<T>` multi-sharing works via a refcount bump at the Send boundary per ADR-0026 v2 §3.2. The single-threaded VM (ADR-0028 §9) has no real Send crossing — a multi-worker demo is inherently v0.10 territory. **Resolution:** scope the `.7e` demo to a single `fetch_add` call, and document multi-worker behavior in comments as part of the v0.10 backlog. This exercises ADR-0028 §9 test gate item 2 ("round-trip correctness, not concurrency") exactly.

**Issue:** v0.9.x.atomic.7 requires an `atomic_counter` demo running live on the VM (3× `fetch_add` $\rightarrow$ counter == 3) per ADR-0028 §9. Stdlib `sys.atomic` signatures require `&+ Atomic<T>` parameters per ADR-0028 §5 (interior mutability — frozen owner). However, `let counter = new(0)` produces a plain `Atomic<Integer>` per ADR-0028 §6 (the constructor returns an owned value, not a borrowed reference). Triet v0.8 could lex `&+`/`&0`/`&-` tokens in a **type expression** context only — there was no expression-level borrow syntax. Consequence: `spawn_worker(counter)` raised an E1003 type mismatch, making call sites impossible beyond type-level paperwork.

Three paths emerged:
1. Implicit T $\rightarrow$ &+ T auto-coercion in `Type::matches()`. Smallest scope, but reverses how Rust handles function calls (ownership move), risking loosening invariants.
2. Change `sys.atomic.new` to return `&+ Atomic<T>` directly. Breaks the ADR-0028 §6 signature; still fails to cover other cases (e.g. sharing an existing `Atomic<T>` value).
3. **Implement expression-level `&FORM expr` borrow syntax** — author's choice on 2026-05-30 (the cleanest answer; aligns with ADR-0022 §4.2 spirit "compiler automatically borrows" + ADR-0028 §6 example syntax).

Open questions ADR-0031 must resolve:

1. **Operand scope** — bare identifier? full expression? bound somewhere in between?
2. **Precedence** — where in the Pratt table?
3. **Semantics** — `&+ x` move-or-copy? Borrow checker enforcement timing?
4. **Form coverage** — all 5 forms (`&+`, `&+ mutable`, `&0`, `&0 mutable`, `&-`) or a v0.9 subset?
5. **Self-host port** — Layer A (lockstep mandatory) per ADR-0029?
6. **Test gate** — minimum requirements to ship v0.9.

---

## §1 — Syntax: prefix `&FORM operand`

**Decision:** Borrow expression has prefix syntax mirroring type-form prefixes:

```
borrow_expr := '&+' ['mutable'] operand    # strong frozen | strong mutable
             | '&0' ['mutable'] operand    # scope read-only | scope exclusive
             | '&-' operand                # weak observer
```

Tokens already exist (the lexer has shipped `AmpersandPlus`/`AmpersandZero`/`AmpersandMinus` since v0.8.x.review.3). The parser disambiguates context: type-expr position uses `try_parse_reference_prefix` (existing); expression position uses the **new** `parse_borrow_prefix` (this ADR).

**Rationale:**

- Mirrors type-form syntax exactly — the author reads `&+ counter` either as a "borrow expression" or an "&+ T type", using the same prefix structure.
- No new tokens needed — incremental over the v0.8 lexer.
- All 5 forms covered uniformly — no asymmetry where one form is expression-only.

---

## §2 — Operand scope: identifier + field access v0.9

**Decision:** v0.9 operand grammar:

```
operand := IDENT                          # &+ counter
         | operand '.' IDENT              # &+ obj.field
```

**Deferred to v0.10+ (corpus-driven; tracked in §10):**

- **Array-style index `vec[i]`** — the Triet parser does NOT currently have a `[i]` index expression (only `TupleIndex` `pair.0` exists). Vector access goes through the `get(vec, i)` method per `triet_chained_get_unwrap.md` conventions. Until `vec[i]` syntax itself ships, `&+ vec[i]` is moot.
- **Function call result borrow** (`&+ make_thing()`) — requires lifetime-extension semantics. Borrow-from-rvalue is a corner case Rust solves via temporary materialization; Triet defers this until a concrete use case emerges.
- **Compound binary expressions** (`&+ (a + b)`) — semantically dubious (borrowing a computed value); refuse-over-guess.
- **Method call result** (`&+ obj.method()`) — same lifetime-extension concern.
- **Nested borrow expression** (`&+ &0 x`) — refused by typecheck (cannot borrow a borrow at the expression level).

**Rationale:**

- Identifier + field-access covers ADR-0028 §6 example syntax + `.7e` demo needs exactly.
- Index operand (`vec[i]`) removed from v0.9 scope per the 2026-05-30 audit — Triet does not have array-index syntax yet.
- Lifetime-extension cases push borrow checker complexity beyond the ADR-0025 staged plan.
- Conservative scope per project philosophy "Refuse over guess" (VISION §6).

---

## §3 — Precedence: prefix unary tier

**Decision:** The borrow prefix sits at the same precedence tier as `!` / `not` / unary `-` / `~+` / `~-` / `~0`. **Right-binding** (consistent with other prefix ops). Lower than postfix `.field`/`[i]`/`(args)`/`?`/`!!` (so `&+ obj.field` parses as `&+ (obj.field)`, NOT `(&+ obj).field`). Higher than every binary operator.

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

**No ambiguity with `&&` (logical AND):** `&&` is a distinct lexer token, not 2×`&`. Longest-match applies per ADR-0022 (S6 lexer rule).

**No ambiguity with type-position prefix:** Parser dispatch by context — type-position uses `try_parse_reference_prefix` after a type keyword, expression-position uses the new prefix rule.

---

## §4 — Semantics per form

**Decision:** Each form produces a `Type::Reference(form, T)` value at the type level. Runtime erases references entirely (refs vanish at the IR/VM level — `RuntimeValue` does not track reference forms).

| Form | Type Result | Ownership Effect on Operand | Notes |
|---|---|---|---|
| `&+ x` | `Reference(StrongFrozen, T)` | Move (x consumed, E2420 if used after) | Frozen post-borrow |
| `&+ mutable x` | `Reference(StrongMutable, T)` | Move (x consumed) | Owner mutable |
| `&0 x` | `Reference(BorrowReadOnly, T)` | Borrow (x lives, scope-bounded) | Multiple OK |
| `&0 mutable x` | `Reference(BorrowExclusiveMutable, T)` | Borrow (x lives, scope-bounded) | Exclusive — NLL enforced |
| `&- x` | `Reference(WeakObserver, T)` | Track (x's lifetime independent) | Upgrade-on-deref $\rightarrow$ `T?` |

**Borrow checker enforcement v0.9 — split per Option A (2026-05-30):**

| Rule | v0.9 Status | Location |
|---|---|---|
| **E2420 UseAfterMove** (consume-once) | ✅ **SHIPS v0.9 — fires real errors** | Sub-task `.7d` per §9 |
| E2440 NLL borrow exclusivity | ⏸️ Defers to v0.10 (corpus-driven) | §10 backlog |
| E2400 Lifetime elision 3 rules | ⏸️ Defers to v0.10 | §10 backlog |
| E2403 `&-` weak observer upgrade | ⏸️ Defers to v0.10 | §10 backlog |
| E2410/E2411 Mutability violations | Skeleton only (pre-existing) | §10 backlog |

**Why E2420 ships in v0.9 (not defer-all):** Without consume-once enforcement, the `.7e` atomic_counter demo silently accepts the `&+ counter; &+ counter` double-move pattern — teaching incorrect semantics and breaking in v0.10 when E2420 fires for real. The author's principle "slow and steady, never ship half-baked solutions" (2026-05-30): code shipping in v0.9 must compile and run with the same semantics in v0.10. E2420 is the minimum check required.

**Why NLL/lifetime defer to v0.10:** Per ADR-0025 staging: "full NLL enforcement defers to v0.9 (requires a real-world Triet corpus)". The `.7e` demo single-call pattern does not exercise NLL (no overlapping borrows) or lifetime elision (no escaping references). Implementing E2440/E2400/E2403 without a corpus risks design rework. The v0.10 corpus will provide full self-host + multi-threaded stdlib + capability demos.

**Forward-compatibility guarantee:** Any v0.9 program that compiles via E2420 will continue compiling in v0.10 with the same semantics. NLL adds REJECTION of previously-passing patterns (overlapping borrows that v0.9 did not catch) — but the `.7e` demo does not trigger any such pattern.

---

## §5 — Lowerer + VM: passthrough

**Decision:** Lowerer emits IR identical to a bare operand. References erase entirely.

```rust
// triet-ir lowerer (pseudocode):
Expr::Borrow { operand, .. } => self.lower_expr(operand)
```

The VM treats `&+ counter` exactly like `counter` — both produce the same `RuntimeValue::Atomic(Rc<RefCell>)` instance. Per ADR-0026 v2 §7 ObjectHeader scheme + ADR-0022 §6 acyclic theorem, no runtime distinction is needed.

**Implication:** Demo `spawn_worker(&+ counter)` runs as `spawn_worker(counter)` would — both reach VM dispatch with the same atomic cell. The single-threaded VM dev tier (ADR-0028 §9) increments correctly: 3× `fetch_add(counter, 1, Synchronized)` $\rightarrow$ counter == 3.

---

## §6 — Form coverage v0.9: all 5 (no bare `&` exists)

**Decision:** All 5 forms (`&+`, `&+ mutable`, `&0`, `&0 mutable`, `&-`) ship in v0.9 expression syntax. Not a subset.

**No 6th "bare `&`" form.** Per SPEC §10.1 ("**Five forms** of reference") + ADR-0022 §2 ("**Five forms** of reference (syntax lock)"), Triet has exactly 5 reference forms. The CLAUDE.md table previously contained phrasing `&+ T, &0 T, &- T, & (ownership reference; longest-match before &&)` which could be misread as 6 forms; the standalone `&` was actually noting the lexer longest-match rule against `&&` logical-AND, NOT a separate form. **Action:** CLAUDE.md row rephrased in the same commit per audit lock context.

**Rationale:**

- Parser cost is amortized — a single prefix rule handles all 5 (loop dispatch on token + `mutable` lookahead).
- The type system already supports all 5 — no new `Type::Reference` variants needed.
- The demo only exercises `&+` — but rejecting `&0`/`&-` would create asymmetric UX where users wonder why they can write the type but not the expression.
- The test corpus needs only 5-form parser + typecheck tests — small marginal cost vs. delivering full surface.

---

## §7 — Self-host port: Layer A (lockstep)

**Decision:** Layer A per [ADR-0029 §3](0029-self-host-port-policy.md). Parser surface change $\rightarrow$ `compiler/parser/parser.tri` lockstep port is mandatory.

**Files affected (Triet self-host):**

- `compiler/parser/parser.tri` — add `BorrowExpr { form, operand }` payload struct + `Expr::BorrowExpr` variant + prefix rule.
- AST symmetry: `compiler/parser/lexer.tri` already covers tokens per v0.8.x.review.3.

**Port timing:** within the same sub-task as the Rust implementation (v0.9.x.atomic.7c per phasing — `.7b` Rust, `.7c` self-host port + bootstrap gate).

**Bootstrap impact:** Stage 2 ≡ Stage 3 byte-identical gate (per ADR-0019 §7) — port must ship before the gate is re-armed in v0.9.final.

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

## §9 — Implementation sub-phase plan (v0.9.x.atomic.7) — revised Option A

| Sub-task | Scope | Files |
|---|---|---|
| `.7a` (done) | Design lock — ADR-0031 Locked, scope refinements, v0.10 backlog | `docs/decisions/0031-*.md` + README + by-topic + TODO restructure + CLAUDE.md bare `&` fix |
| `.7b` | Rust impl borrow expression syntax | `triet-syntax/src/expr.rs` (Expr::Borrow AST variant) + `triet-parser/src/expr.rs` (prefix rule per §3 precedence) + `triet-typecheck/src/check/exprs.rs` (Type::Reference emission) + `triet-ir/src/lowerer.rs` (passthrough) + per-crate tests (parser × form × operand-kind; typecheck per form; lowerer passthrough proof) |
| `.7c` | Self-host Layer A port | `compiler/parser/parser.tri` (Expr variant + prefix rule mirroring `.7b`) + bootstrap symmetry test extension via `release-check.sh` |
| `.7d` | **E2420 UseAfterMove real fires** (Option A enforcement minimum) | `triet-typecheck/src/check/exprs.rs` move-tracking (CFG walk over function body, mark binding state alive/moved on move site, fire E2420 on use of moved binding) + tests (positive: `&+ x` then `x` use fires; negative: single-use clean; mixed `&0`/`&+` cases) |
| `.7e` | Demo runtime + e2e | `examples/atomic_counter/atomic_counter.tri` single-call scope (let counter = new(0); let prev = fetch_add(&+ counter, 1, Synchronized); println prev) + `crates/triet-cli/tests/atomic_counter_e2e.rs` asserting `Counter previous: 0` output + comment-document multi-worker v0.10 backlog |
| `.8` | Phase verify gate | `cargo test` + `clippy` + `fmt` + `release-check.sh` + ROADMAP/TODO archive |

Each sub-task = independent commit per CLAUDE.md cadence. `.7b` is the largest (~5 crates × small change each). `.7d` is medium (CFG walk for move tracking, ~400 LOC). `.7e` is small (demo file + 1 e2e test).

---

## §10 — v0.10 backlog revealed by this ADR

The following items surfaced during ADR-0031 design + the 2026-05-30 audit. **Tracked here** so the v0.10 phase opening picks them up; each item cross-links the source ADR/section that locked it.

### 10.1 — Borrow checker remaining enforcement (per ADR-0025 staged plan)

- **E2440 NLL borrow exclusivity (full CFG live-range)** — per [ADR-0025 §2](0025-borrow-checker-rules.md). Compute borrow-active region from creation to last-use; reject overlapping `&0 mutable` / `&0` / `&+` borrows. Scope: ~1000+ LOC, CFG-based live-range analysis. Trigger: when the v0.9 corpus (self-host + atomic + JIT phases) exposes real overlap patterns.
- **E2400 Lifetime elision 3 rules** — per [ADR-0025 §3](0025-borrow-checker-rules.md). Implement Rule 1 (single input borrow $\rightarrow$ output), Rule 2 (`self` receiver $\rightarrow$ output ties self), Rule 3 (owned return). Scope: ~300 LOC + tests. Trigger: when function signature corpus produces ambiguous elision cases.
- **E2403 `&-` weak observer upgrade tracking** — per [ADR-0022 §2 row 5](0022-trit-balanced-ownership.md). Deref `&- T` $\rightarrow$ `T?` (nullable); compile-time tracked. Scope: ~200 LOC. Trigger: when the first stdlib needs weak refs (likely concurrency primitives or doubly-linked structures).
- **E2410/E2411 Mutability violation enforcement** — skeletons exist per v0.8.10. Full enforcement (assign-to-frozen, mutate-via-readonly-borrow). Trigger: when `&+ mutable` / `&0 mutable` usage corpus grows.

### 10.2 — Atomic primitive multi-thread completion (per ADR-0028 + ADR-0026 v2)

- **Real `raw_thread.spawn` implementation** — per [ADR-0026 v2 §3](0026-actor-boundary-send-rules.md). Replace v0.9 placeholder `function spawn(work: Integer) -> Handle = Handle { thread_id: 0 }` with real OS thread creation. `Handle.join()` blocks until real thread terminates.
- **Send boundary refcount-bump codegen** — per [ADR-0026 v2 §3.2](0026-actor-boundary-send-rules.md). When `&+ T` crosses spawn boundary, emit refcount-bump on ObjectHeader (`triet-core::memory`). User-visible: nothing changes; under the hood: multi-share enabled.
- **`&+ Atomic<T>` multi-thread clone semantics** — per [ADR-0028 §5](0028-atomic-primitive.md). Locked: "refcount-mediated share, race conditions resolved by Ordering". v0.10 wires up clone-on-Send-boundary path; single-thread `&+` stays linear move (consume-once) per v0.9 .7d enforcement.
- **`atomic_counter` demo multi-worker upgrade** — per `.7e` v0.9 comment. Reactivate the 3-worker pattern + final `load(&+ counter, ...)` once real spawn ships. Add concurrency assertion (counter eventually consistent $\ge$ 3 after all join).
- **`std.concurrency.*` stdlib** — per [ROADMAP §v0.10](../../ROADMAP.md). Mutex, Channel, M:N green threads. Built atop `sys.raw_thread` real implementation.

### 10.3 — Borrow expression operand scope expansion (deferred from ADR-0031 §2)

- **Function-call result borrow** (`&+ make_thing()`) — requires lifetime-extension semantics (Rust calls this "temporary materialization"). Decide rules + ADR amendment.
- **Method-call result borrow** (`&+ obj.method()`) — same lifetime concern as above.
- **Array-style index expression `vec[i]`** — independent of this ADR. Vector access currently via `get(vec, i)` only per `triet_chained_get_unwrap.md`. v0.10 (or whenever index syntax ships) extends `&+ vec[i]` operand.
- **Compound binary expressions** (`&+ (a + b)`) — explicitly refused in v0.9 (refuse-over-guess). Re-evaluate if corpus surfaces use case; default stays refused.

### 10.4 — Atomic E2530 — Pointer-Relaxed `fetch_*` pattern (deferred from ADR-0028 §10)

- Per [ADR-0028 §10 pattern 2](0028-atomic-primitive.md): `fetch_add/sub/and/or/xor` with `Ordering.Relaxed` on `Atomic<Pointer>` should fire E2530 (Pointer is publish-like; Relaxed publish almost always wrong). **Blocked v0.9**: `Pointer` type does not parse. v0.10 when Pointer lands $\rightarrow$ wire up this E2530 conservative pattern alongside existing `compare_exchange` weaker-success check from `.6`.

### 10.5 — CLAUDE.md normative documentation drift

- **2026-05-30 audit found bare `&` row in CLAUDE.md confusing.** Fixed in the same commit as `.7a` lock (this commit). Pattern: CLAUDE.md table rows must spell out forms exhaustively, not hint at lexer rules ambiguously. Audit policy: when adding language convention rows to CLAUDE.md, cross-reference SPEC § to confirm form count + spelling.

### 10.6 — Self-host port lag tracking

- `.7c` ports borrow expression parsing to `compiler/parser/parser.tri` (Layer A per ADR-0029 §3 mandatory). v0.10 may discover that `.7d` E2420 enforcement also needs Layer A or Layer B port if self-host typecheck implementation lands. Currently self-host typecheck pass is minimal; revisit when self-host typecheck phase opens (post-v0.9).

### 10.7 — Interpreter parity for `sys.atomic.*` builtins (discovered v0.9.x.atomic.7e)

- **Gap:** `triet-interpreter` (tree-walking dev tier per VISION §4.3) does NOT intercept `sys.atomic.*` builtin paths. Calling `sys.atomic.new(0)` recurses into the stdlib placeholder body (`= new(initial_value)`) $\rightarrow$ stack overflow. Same pattern blocks `sys.atomic.load/store/swap/fetch_*`. Mirrors the existing `outcome_propagate.tri` VM-only precedent per [CLAUDE.md](../../CLAUDE.md) demos note.
- **v0.9 workaround:** `examples/atomic_counter/` is a **VM-only demo**. User runs via `dao build && dao run *.khi`, NOT direct `dao run atomic_counter.tri`. In-tree e2e test exercises in-process VM, not interpreter.
- **v0.10 resolution:** add atomic builtin intercepts to `triet-interpreter::interpret::evaluate_call_expression` (or similar) mirroring the VM's `path_to_builtin` lookup. Implementation: `RuntimeValue::Atomic` already exists in IR layer; interpreter `Value` enum needs parallel `Atomic` variant + per-op dispatch. Scope estimate: ~300 LOC + tests.
- **Forward-compat:** v0.9 demo continues to work in v0.10 without source change — only interpreter implementation extends.

---

## Consequences

**Positive Outcomes:**

- ADR-0028 §6 example syntax is now real code, not aspirational documentation.
- Demo `atomic_counter` exercises full VM dispatch path — load/fetch_add round-trip is verifiable.
- Future stdlib (Mutex v0.10, std.concurrency.* v0.10+) can use `&+`/`&0`/`&-` cleanly without piecemeal type loosening.
- Closes SPEC §10 warning ("runtime does not yet expose references") — references are now first-class at parser and typecheck levels.

**Constraints & Costs:**

- 6 Rust source files touched (~200 LOC net) + ~150 LOC self-host port.
- Parser test surface expands (~15 new tests).
- Borrow checker NLL enforcement still defers to v0.9.x.borrow.* — short window where user can write borrow expressions without consume-once enforcement (acceptable per ADR-0025 staged plan).

**Risks & Verification Needs:**

- Pratt precedence ambiguity with `&&`: lexer longest-match should prevent this — verify with stress tests.
- Self-host bootstrap performance — Stage 2/3 parser path adds a prefix rule, with marginal impact.

---

## Rejected Alternatives

- **Implicit T $\rightarrow$ &+ T auto-coerce in `Type::matches()`** (original option 2). Cleaner short-term but blurs borrow rules — violates refuse-over-guess (VISION §6). Author rejected on 2026-05-30.
- **Constructor returns `&+ Atomic<T>` directly** (original option 3). Breaks ADR-0028 §6 signature lock; does not cover all sharing scenarios. Author rejected on 2026-05-30.
- **Borrow from function call result / compound expression** (operand scope reduction). Lifetime-extension semantics unresolved; defer until corpus provides data.
- **Per-form precedence variation.** All 5 forms occupy the same precedence tier; avoids PEMDAS-like inconsistencies.
- **Auto-deref for borrow ops** (e.g. `(&+ x).field == x.field`). Defer to method dispatch ADR (post-v1.0). v0.9 ships explicit forms only.

---

## Prior Art

| Source | What We Adopted | What We Changed |
|---|---|---|
| Rust `&x` / `&mut x` | Prefix borrow syntax; precedence | Triet has 5 forms (vs. Rust's 2); explicit `&+`/`&0`/`&-` polarity vs. Rust's `&`/`&mut` |
| OCaml `ref x` / `!x` | Distinguished reference vs. deref ops | Triet uses prefix `&FORM`; ref-deref does not exist at expression level (handled automatically at type level) |
| Swift `inout` parameter | Mutable borrow at call site | Triet uses `&0 mutable` at borrow site; type carries form |
| C++ `&x` (address-of) vs. `int&` (reference type) | Same symbol, different meaning by position | Triet does the same — `&+` in type vs. expression position |

**Novel Contributions in Triet:**

- **Ternary form polarity in expression position** — `+`/`0`/`-` Trit-aligned reference forms (existing in types) extend to expressions uniformly.
- **All-5-forms at expression level** — Rust has 2 (`&`/`&mut`), Triet maintains all 5 symmetrically with type expressions.

---

## References

- [ADR-0022](0022-trit-balanced-ownership.md) — S6 ownership model + 5 reference forms (parent design).
- [ADR-0022 §4.2](0022-trit-balanced-ownership.md) — "Compiler automatically borrows `&+` to `&0`" rationale (implicit borrow direction).
- [ADR-0025](0025-borrow-checker-rules.md) — borrow checker rules (consumes-once enforcement, deferred to v0.9.x.borrow.*).
- [ADR-0026 v2 §7](0026-actor-boundary-send-rules.md) — ObjectHeader scheme (references erase to runtime cell).
- [ADR-0028 §5 + §6](0028-atomic-primitive.md) — `&+ Atomic<T>` interior mutability + constructor signature (trigger for this ADR).
- [ADR-0029 §3](0029-self-host-port-policy.md) — Layer A lockstep rule (parser surface = mandatory port).
- [SPEC §10](../../SPEC.md) — Memory model (references + ownership; warning about v0.7 runtime gap that this ADR closes).
- [VISION §6](../../VISION.md) — "Refuse over guess" philosophy (operand scope conservatism).
