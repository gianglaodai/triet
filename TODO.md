# TODO

Sub-task tracking — short-term work in progress.

- Long-term phasing: [`ROADMAP.md`](ROADMAP.md)
- Architectural decisions: [`docs/decisions/`](docs/decisions/)
- Language semantics: [`SPEC.md`](SPEC.md), [`VISION.md`](VISION.md)

This file tracks the **current phase** only. When a phase finishes, its summary archives to `ROADMAP.md` and detailed checkboxes are deleted from here.

---

## v0.2 — v0.6 archived

All shipped phases now live in [`ROADMAP.md`](ROADMAP.md):

| Phase | ADRs | Final test count |
|---|---|---|
| v0.2.x Module system | 0005, 0006 | 700+ |
| v0.3 Bytecode VM + Stable IR | 0007, 0008 | 835 |
| v0.3.x.cleanup | 0009 | 835 |
| v0.3.x.ternary | 0010 | 838 |
| v0.4 Crate-Pack + Stable ABI | 0011, 0012, 0013 | 867 |
| v0.5 CAS Packaging | 0014, 0015 | 918 |
| v0.5.x.review | 0015 Addendum | 924 |
| v0.6 Capability System | 0016, 0017, 0018 | 1079 |
| v0.6.x.review | 0018 Addendum | 1085 |

---

## v0.7 — Self-hosting Compiler 🔄 in progress

**Quyết định kiến trúc:** [ADR-0019](docs/decisions/0019-self-hosting-compiler-bootstrap.md) (bootstrap + Rust-shim stdlib), [ADR-0020](docs/decisions/0020-outcome-error-handling.md) (Outcome error handling). See ROADMAP.md §v0.7 for deliverables + gates (recalibrated per ADR-0019 §7 — perf parity defers to v0.9).

### Shipped (9 commits, 1085 → 1129 tests)

| Sub-task | Description | Commit |
|---|---|---|
| v0.7.1 | ADR-0019 framework + ROADMAP §v0.7 recalibrate + ADR index update | `277ee7f` |
| v0.7.2 | Canonical emission invariants audit + `triet-bootstrap` skeleton + 3 determinism tests | `cf7eaf4` |
| v0.7.3.1 | `TypeTag::Vector`/`HashMap` + `.triv` wire v3 → v4 + ADR-0019 Addendum §A1-A4 | `5da6234` |
| v0.7.3.2 | Vector builtins (4 ops, wire IDs 8-11) — VM dispatch + 5 tests | `472cc65` |
| v0.7.3.3 | HashMap builtins (5 ops, wire IDs 12-16) + error-model 3-tier lock + 7 tests | `77e5acf` |
| v0.7.3.4 | IO/path/string builtins (10 ops, wire IDs 17-26) — closes v0.7.3 umbrella; 12 tests | `f304e87` |
| v0.7.4.1 | Generic function syntax — parser + AST + typecheck + lowerer (type-erased per Q3-A deviation §A7.1) | `96c92ef` |
| v0.7.4.2 | Stdlib `.tri` stubs (5 new files, Java-aesthetic) + `path_to_builtin` (19 entries) + 5 VM tests | `f6d722f` |
| v0.7.4.3-error (docs) | ADR-0020 Outcome error handling design (10 §s including null/~0 unification) + ADR-0001/0010 Addendums + SPEC §1.5.3/§2.5 updates | `9f8dca6` |
| v0.7.x.audit.1 | CRITICAL fixes (CLAUDE state + ADR-0007/0008 cross-refs) | `46dd59a` |
| v0.7.x.audit.2 | MAJOR fixes (README + TODO + anchors) | `0b2d336` |
| v0.7.x.audit.3 | MINOR fixes (SPEC null→~0 + CLAUDE outcome syntax + audit memory) | `06eff56` |
| v0.7.4.3-error.1 | Lexer + AST + Parser for Outcome syntax (compound tokens, `\|capture\|`, productions) | `c0fe111` |
| v0.7.4.3-error.2 | Typecheck Outcome support — `Type::Outcome` + 9 errors (E1024-E1032) + W2001 NullDeprecated | `d8e5b07` |
| v0.7.4.3-error.3a | IR data plane — `TypeTag::Outcome` + 6 opcodes (0xC1-0xC6) + `.triv` v5 + VM dispatch + E2210 | `f9d1f91` |
| v0.7.4.3-error.3b | Lowerer — AST Outcome → IR opcodes + pattern matching + 10 e2e VM tests | `d03aa66` |
| v0.7.4.3-error.3c | ADR-0021 `Trilean!` refinement + SPEC §7.1.1 fix + ADR-0010 Addendum §C | `f4fa78e` |
| v0.7.4.3-error.3d | `Trilean!` refinement type + E1033 `PossiblyUnknownCondition` + E1034 `TrileanReturnNotRefined` + 15 typecheck tests | `c3eb126` |
| v0.7.4.3-error.3e | Migrate corpus `if (trilean == lit)` → `match` (alu/memory/utils/print) | `6e4db80` |
| v0.7.4.3-error.4a | `triet fmt --migrate-null` tool (lexer-based, idempotent, dry-run-by-default + `--write`) | `e49d389` |
| v0.7.4.3-error.4b | Apply migration to 6 stdlib stubs (`examples/nullable.tri` deferred pending outcome-null runtime unification) | `be7532d` |
| v0.7.4.3-error.6a | Outcome-null runtime unification — ADR-0010 Addendum §D + lowerer + 4 cross-tolerant VM opcodes + 6 tests | `ffcf6de` |
| v0.7.4.3-error.6b | Interpreter parity for `~0` + migrate `examples/nullable.tri` (closes `.4b` deferred) | `a48c275` |
| v0.7.4.3-fix (struct-fields) | Wire `StructDef` field order into lowerer — kills `field_name_to_idx` placeholder | `0d4577e` |
| v0.7.4.3-error.5a | Capstone demo (`demos/05-error-handling/` — 4 `.tri` files + README) | `c139a89` |
| v0.7.4.3-error.5b | Capstone integration tests (4 tests in `error_handling_demo.rs`) | `b5b2abc` |

### Closing summary (`v0.7.4.3-error` umbrella)

`v0.7.4.3-error` introduced Outcome error handling (ADR-0020), Trilean! refinement (ADR-0021), and the outcome-null runtime unification (ADR-0010 Addendum §D). The `.5` capstone demo proves all locked features end-to-end through the VM tier. 1221 workspace tests pass.

---

## v0.7.4.3-debt — Lowerer + typecheck cleanup before lexer port (in progress)

Pre-port audit surfaced **7 workaround sites** in the draft `compiler/lexer.tri`. Author opted (2026-05-19) for the no-tech-debt path: fix the underlying compiler bugs FIRST, then commit `lexer.tri` without workarounds. Rationale per [`feedback_stability_over_speed.md`][stab] + the explicit "tránh tăng thêm nợ kỹ thuật" directive.

[stab]: ../home/.claude/projects/-mnt-M2-STORAGE-Work-workspace-gh-rust-triet/memory/feedback_stability_over_speed.md

The draft `compiler/lexer.tri` + `crates/triet-bootstrap/tests/lexer_self_smoke.rs` stay in the working tree (uncommitted) as a regression gate — each debt fix must keep the smoke test passing.

### Debt sub-tasks

- [x] **v0.7.4.3-debt.1** — Trilean! parser support (WA-3 + WA-4) — `123ffa7`
  - Parse `Trilean!` in type-annotation position as `Type::Trilean { refined: true }`
  - Auto-fix WA-4 (refinement preservation through `&&` once helpers can declare `-> Trilean!`)
- [x] **v0.7.4.3-debt.2** — Field access alphabetical bug (WA-2) — (this commit)
  - Lowerer now tracks `value_outcome_value_struct` / `func_return_outcome_value_struct` parallel to the existing struct-typing maps. Propagation lands in `lower_outcome_propagate` (after `OutcomeUnwrapValue` + Phi), `lower_outcome_default` (after success-arm `OutcomeUnwrapValue` + same-struct Phi), and `bind_pattern_vars` for `Pattern::OutcomeArm(Positive)`. 2 integration tests in `triet-bootstrap`.
- [x] **v0.7.4.3-debt.3** — E1025 false positive (WA-5) — (this commit)
  - `Checker::expected_type_stack: Vec<Type>` consulted before `current_return_type` in `check_outcome_constructor_context`. `check_initializer` pushes the let-binding annotation while inferring the value; `with_expected` RAII helper handles push/pop. Local site wins over surrounding return type, so `let x: T? = ~0` inside a `T~E` function is accepted while a bare `return ~0` from the same function still raises E1025. 5 integration tests cover positive + negative paths.
- [x] **v0.7.4.3-debt.4** — Generic chain inference (WA-7) — (this commit)
  - `extract_type_params` switched from `or_insert_with` to a prefer-concrete rule: when the existing `sub_map[T]` is itself a `TypeParam` (poisoned by an upstream un-inferrable generic like `new<T>()`), a subsequent concrete arg replaces it. First-concrete-wins still holds for `f<T>(a: T, b: T)` mismatched calls. 5 integration tests cover the lexer-port `push(new(), x)` chain, nested chains, mismatch detection, and the bare `new()` alone case.
- [x] **v0.7.4.3-debt.5** — Match-arm mutation phi + bare-variant pattern dispatch (WA-1) — (this commit)
  - Two interacting bugs found and fixed together. (a) `lower_match_expr` now collects mutated vars across all arms, pre-snapshots, post-snapshots per arm, and emits one phi per mutable at the merge block — mirroring `lower_if_expr`. (b) `lower_pattern_test` + `bind_pattern_vars` rewrite `Pattern::Variable(name)` to an `EnumVariant` tag check when `name` resolves to a known unit variant. Pre-fix the parser bug was latently masked by the static-last-write semantics of bug (a); fixing (a) exposed it.
  - 5 integration tests in `match_arm_mutation_phi.rs`: while+match+push, bare-unit-variant dispatch, mutation-observable-after-match, wildcard-with-mutations, and a `lex_decimal_integer` reproducer.
- [x] **v0.7.4.3-debt.6** — Rewrite `compiler/lexer.tri` + commit lexer port (this commit)
  - Removed all 7 workarounds: helpers now return `Trilean!` (with plain `if` at call sites), `NumericSuffix?` replaces the explicit `NoSuffix` sentinel, `OneToken` declared in natural order, mode-dispatch restored to canonical `match top { NormalMode => …, FStringMode => …, InterpolationMode(state) => … }`. Generic `push(new(), x)` chain reads cleanly. ~1090 LOC Triết.
  - Also fixed two additional gaps surfaced during rewrite: struct-literal field positions need the expected-type push (mirrors `.debt.3`'s let-binding logic); and `OutcomeDiscriminant`/`OutcomeUnwrapValue` now cross-tolerate bare `T` values flowing through a `T?` slot (closes WA-6 — the previously-deferred lowerer cross-tolerance for match-arm dispatch beyond the 4 opcodes proven in `ffcf6de`).
  - Bootstrap regression gate (`lexer_self_smoke.rs`) green; 1247 workspace tests pass.

- [x] **v0.7.4.3-debt.7** — EnumTag Integer variant index (parser.tri unblocker) — (this commit)
  - `EnumTag` opcode: output changed from `Trit(Positive | Negative)` to `Integer(variant_index)`. Pattern::EnumVariant + Variable-as-variant now compare `Eq(tag, Integer(idx))` instead of `Eq(tag, Trit(idx==0?Positive:Negative))`. Pre-fix any enum with 3+ variants collapsed variant 1,2,3,... into indistinguishable Negative; post-fix all variants dispatch correctly. 1247 tests pass; 4-variant enum reproducer `E { A, B, C, D }` now produces `A→1 B→2 C→3 D→4`.

### Deferred (not in debt umbrella)

_None — all 7 workarounds resolved. WA-6 deferral moved to .debt.6 since it surfaced together with struct-field expected-type extension and was cheap to fix in the same commit._

### After v0.7.4.3-debt: remaining v0.7 sub-tasks

- [ ] **v0.7.4.4** — `lexer_differential` NDJSON byte-diff test + verify gate (closes v0.7.4 umbrella)
- [ ] **v0.7.5** — `compiler/parser.tri` + parser_differential test
- [ ] **v0.7.6** — `compiler/modules.tri` + modules_differential test
- [ ] **v0.7.7** — `compiler/typecheck.tri` + typecheck_differential test
- [ ] **v0.7.8** — `compiler/ir_lowerer.tri` + lowerer_differential test
- [ ] **v0.7.9** — `compiler/pack_writer.tri` + `compiler/main.tri` + drop bridges
- [ ] **v0.7.10** — CLI wiring carry-over (project layout + cap-aware build + DevTtyPrompt + E2208.CapabilityDivergence)
- [ ] **v0.7.11** — Stage 1 → Stage 2 bootstrap script + CI integration
- [ ] **v0.7.12** — Stage 2 → Stage 3 + bit-identical gate verify
- [ ] **v0.7.13** — Verify gate ADR-0009 §A/B/C/D + workspace version 0.6.0 → 0.7.0 + SPEC v0.6 → v0.7 + docs sync

---

## How to update this file

- Mark sub-task `[x]` when its commit lands on `main`.
- Add commit short-hash next to completed sub-tasks for quick git reference.
- Keep order: **Shipped** (table format) → **In progress** (checkbox list) → **Pending** (checkbox list).
- When a whole phase ships, archive its summary into `ROADMAP.md` (changelog section) and delete detailed checkboxes here.
- Audit cadence: every 5-10 commits OR before major implementation phase, per `feedback_proactive_audit.md`.
