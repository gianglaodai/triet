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

- [x] **v0.7.4.3-debt.7** — EnumTag Integer variant index (parser.tri unblocker) — `730fddc`
  - `EnumTag` opcode: output changed from `Trit(Positive | Negative)` to `Integer(variant_index)`. Pattern::EnumVariant + Variable-as-variant now compare `Eq(tag, Integer(idx))` instead of `Eq(tag, Trit(idx==0?Positive:Negative))`. Pre-fix any enum with 3+ variants collapsed variant 1,2,3,... into indistinguishable Negative; post-fix all variants dispatch correctly. 1247 tests pass; 4-variant enum reproducer `E { A, B, C, D }` now produces `A→1 B→2 C→3 D→4`.

### Deferred (not in debt umbrella)

_None — all 7 workarounds resolved. WA-6 deferral moved to .debt.6 since it surfaced together with struct-field expected-type extension and was cheap to fix in the same commit._

### After v0.7.4.3-debt: remaining v0.7 sub-tasks

- [x] **v0.7.4.4** — `lexer_differential` NDJSON byte-diff test + verify gate (closes v0.7.4 umbrella) — `e1535fd`
  - New `crates/triet-bootstrap/tests/lexer_differential.rs` (20 tests). Adds `dump_ndjson(source: String) -> String` to `compiler/lexer.tri` (NDJSON bridge format per [ADR-0019 §A2]: `{"t":<Kind>,"s":[start,end][,"v":...][,"u":...]}` per token; `{"e":...}` on error). Rust-side mirror converts byte spans → char spans via `byte_to_char_index` so real `examples/*.tri` files with UTF-8 comments (box-drawing) participate in the corpus. Corpus covers keywords, operators (single + compound + outcome), nullable/force-unwrap, ternary + decimal + suffixed integer literals, string/f-string/escape handling, line comments, question-modified keywords (`if?`/`while?`), realistic function signatures, and three example files (factorial / maybe / nullable). 1267 workspace tests pass.
  - **Lowerer fix surfaced by this gate**: `lower_while_loop` now uses `rebind_var` (instead of `bind_var`) when binding the loop-header phi-dest into the live scope. Without this, an `Expr::Block` scope wrapping a match-arm body (the parser wraps `~+ x => { … }` arm bodies as `Expr::Block`) would `pop_scope` and drop the phi-dest mapping before `lower_match_expr`'s post-arm `resolve_var` snapshot, so any variable mutated through a `while` inside a match arm reverted to its pre-match value after the match. The body-scope shadow at line ~1846 still keeps in-body reads/writes pointing at the phi-dest.
  - **VM fix surfaced by this gate**: `NullCheck` no longer classifies `RuntimeValue::Enum { payload: None, .. }` as the null state. The legacy "any payload-less Enum is null" arm collided with bare unit-variant enums (e.g. `LetKw`) flowing through a `T?` slot via the ADR-0010 Addendum §D cross-tolerance — `keyword_for(slice) ?: Identifier(…)` then mis-classified every keyword as null and produced `Identifier` for `let`/`while`/etc. The two canonical null carriers remain `RuntimeValue::Null` and `Outcome { discriminator: Trit::Zero, payload: None }`.
  - **Lexer-port gap surfaced by this gate**: `finish_ident` now peeks past the identifier slice and absorbs a trailing `?` for the `if?` / `while?` compound keywords, mirroring the Rust impl's `#[token("if?")]` / `#[token("while?")]` longest-match.

### v0.7.4 umbrella closed

The 4-sub-commit umbrella from [ADR-0019 §A7.4] is now done end-to-end: v0.7.4.1 generic syntax → v0.7.4.2 stdlib stubs → v0.7.4.3 lexer port + `-debt.{1..7}` cleanup → v0.7.4.4 differential gate. Triết-side and Rust-side lexers agree byte-for-byte on the corpus.

[ADR-0019 §A2]: docs/decisions/0019-self-hosting-compiler-bootstrap.md
[ADR-0019 §A7.4]: docs/decisions/0019-self-hosting-compiler-bootstrap.md

---

## v0.7.5 — `compiler/parser.tri` port (in progress)

Per ADR-0019 §A7.5: port crates/triet-parser/ (~6027 LOC across 9 files) to a Triết-native `compiler/parser.tri`, mirroring crates/triet-syntax/'s arena pattern. Author opted (2026-05-21) for the split-umbrella approach: each sub-task ships an incremental AST + parser slice with its own integration test, rather than a single 7000-LOC commit. Rationale per `feedback_stability_over_speed.md` — debug surface co lại, per-sub-task verify gate matches v0.3 cadence.

### Sub-tasks

- [x] **v0.7.5.1** — AST + Arena scaffolding (this commit)
  - New `compiler/parser.tri` (~340 LOC). Mirrors Rust `Arena` with four `Vector<Spanned*>` sub-arenas keyed by Integer index. Ships minimal `Expr` / `Pattern` / `TypeExpr` / `Stmt` surfaces (just the variants needed to prove the recursive lookup pattern); subsequent .5.N sub-tasks expand each enum as parser features land. `AllocResult` wraps `(arena, id)` because IR tuple returns are deferred post-v1.0. Smoke `main()` constructs the AST for `1 + 2 * 3`, asserts arena counts + recursive `format_expr` traversal, and exercises all four sub-arenas.
  - **Two pre-existing lowerer/VM bugs surfaced + fixed under this sub-task:**
    - `lowerer.rs`: `bind_pattern_vars` for `Pattern::EnumVariant` never propagated payload-struct identity onto the SSA value bound by the match, so `match e { Variant(p) => p.field }` always read slot 0. Pass 1a.2 now populates `variant_payload_struct` (variant_name → struct_name) which the bind site consults — parallel to the OutcomeArm path covered by debt.2.
    - `vm.rs`: `NullUnwrap` retained a legacy `Enum { variant: 0, payload: Some(p) } → unwrap` arm that was the inverse of the now-unused `NullWrap` emit. Under ADR-0010 Addendum §D unified encoding, `T?` flows as the bare value (or `Null`); the legacy arm only ever hit user enums whose variant-0 carried a payload, so `Vector<Node>.get(...)!!` returned `Integer(10)` instead of `Leaf(10)`. Symmetric to the v0.7.4.4 NullCheck cleanup.
  - 4 integration tests across `parser_arena_smoke.rs` (1) + `struct_field_through_enum_variant.rs` (3) cover the smoke + both bug fixes. 1271 workspace tests pass.

- [x] **v0.7.5.2** — Pratt expressions + atoms (this commit)
  - `compiler/parser.tri` (+1245 LOC, ~1585 total) — `module lexer;` + `from crate.lexer import …` wires the Token stream from `compiler/lexer.tri`. Adds `ParserState` (tokens + cursor + arena) threaded functionally through every parse helper; `ParseStep~ParseError` is the outcome wrapper (no tuple returns yet per SPEC §95). Expr enum grows from 8 to 17 variants — adds `ForceUnwrapExpr`, `CallExpr`, `FieldAccessExpr`, `MethodCallExpr`, `OutcomeConstructorExpr` (+ `OutcomeArm` enum), `OutcomePropagateExpr`, `OutcomeDefaultExpr`, `ElvisOpExpr`, `RangeExpr`. Pratt covers: 20-variant `BinaryOperator` ladder (precedence + assoc per SPEC §12.1), unary `Negate` (3 surface forms: `-x` / `!x` / `not x`), parenthesised grouping, postfix `!!` / `(args)` / `.field` / `.method(args)` / `~?` / `~:`, plus Elvis `?:` and range `..` / `..=`. Smoke `main()` asserts 17 source snippets parse to their expected s-expr shape. Block / If / Match / Lambda / Tuple / FString / StructLiteral / SafeAccess defer to v0.7.5.3+.
  - **Two pre-existing bugs surfaced + fixed under this gate:**
    - `crates/triet-typecheck/src/check_resolved.rs`: Pass 1 collected each module's declared types in isolation, so any field whose annotation referenced a user-defined type from *another* module fell through to `Type::Unknown`. Pass 2 imports then carried Unknown field types, breaking `match spanned.token { Variant(payload) => ... }` (the `bind_pattern` `UserEnum` guard fails on Unknown → payload binding never enters scope → E1002 on every reference). Pass 1 now iterates to a fixed point with a cross-module name table, so user-type references resolve into their full `UserStruct` / `UserEnum` shapes before Pass 2 runs. Two regression tests in `check_resolved.rs` (cross-module struct-field-match + nested struct field access).
    - `crates/triet-ir/src/lowerer.rs`: With (1) fixed, parser.tri parsed cleanly through typecheck but blew up at the VM on chained field access like `step.state.arena` — the intermediate `step.state` SSA value had no `value_struct_types` entry, so the next `.arena` slot resolution fell back to slot 0 and triggered E2201 "expected Unit, got non-struct". Pass 1a now records `struct_field_types: (struct, field) → struct-name-if-field-is-a-named-struct`, and `Expr::FieldAccess` propagates that identity onto the `FieldGet` dest so chained accesses keep tracking through every link. Parallel to the [v0.7.4.3-debt.2] `value_outcome_value_struct` propagation chain.
  - 6 integration tests across `parser_expr_smoke.rs` (1, lex+parse end-to-end via the VM smoke) + `cross_module_field_resolution.rs` (3, covers single-module, nested chained access, and outcome-unwrapped chained access) + 2 typecheck unit tests pinning the cross-module resolution. 1277 workspace tests pass; `cargo clippy --workspace --all-targets` clean.
- [x] **v0.7.5.3** — Statements + bindings (this commit)
  - `compiler/parser.tri` (+1001 LOC, ~2465 total) — ports `crates/triet-parser/src/stmt.rs` (~540 Rust LOC). Adds the `Block` struct (`block_statements: Vector<Integer>` + optional `block_final: Integer?`), grows `Stmt` from one variant (`ExpressionStmt`) to ten — `LetStmt` / `ConstantStmt` / `ReturnStmt` / `BreakStmt` / `ContinueStmt` / `ForStmt` / `WhileStmt` / `LoopStmt` / `AssignStmt` / `ExpressionStmt` — each multi-field variant carrying its own `*Payload` struct because Triết enum variants take a single payload. `Pattern` grows from `WildcardPattern` only to `WildcardPattern | IdentifierPattern(StringPayload)` — the minimum needed for `let name = …` and `for i in …` / `for _ in …`. New step wrappers (`ParseStmtStep`, `ParsePatternStep`, `ParseBlockStep`, `BlockElement` + `BlockElementStep`, `EatFlagStep`) extend the v0.7.5.2 `ParseStep` pattern; the `BlockElement` two-arm enum tells `parse_block` whether the next element is a finished statement or the block's trailing final-expression. Token plumbing: `is_semi` / `is_rbrace` / `is_assign_token` / `is_comma` / `is_mutable_kw` / `is_while_q_kw` / `is_value_terminator` predicates, `eat_semi` / `eat_mutable`, and `expect_assign` / `expect_lbrace` / `expect_rbrace` / `expect_in_kw` / `expect_colon` consume-or-error helpers. `parse_optional_type_annotation` + `parse_type_minimal` ship a single-identifier `NamedType` parser to fill the `let`/`constant` annotation slot — full type grammar lands in v0.7.5.5. The `parse_block` driver is a `mutable` while-loop over `parse_statement_or_final_expr`; the statement-vs-final-expression dispatch follows the Rust impl (assignment when `Expr::Identifier =`, expr-stmt when `;`, final-expression when `}` / EOF, expr-stmt fallback otherwise). Smoke `main()` asserts 20 new block snippets covering every Stmt variant + the empty / final-only / trailing-final paths.
  - One Triết-side gotcha surfaced + recorded: **`mutable` is a reserved keyword**, so the `LetStmtPayload.mutable` field had to rename to `is_mutable`. Rust uses `mutable: bool` freely because Rust has no `mutable` keyword; Triết's `let mutable` syntax (per ADR-0005, verbose keywords) forces the renaming.
  - 1 new integration test (`parser_stmt_smoke.rs`) loads `compiler/parser.tri` end-to-end (load → typecheck → lower → write `.triv` → round-trip read → run `main()` on VM) — same shape as `parser_arena_smoke.rs` (v0.7.5.1) and `parser_expr_smoke.rs` (v0.7.5.2). 1278 workspace tests pass; `cargo clippy --workspace --all-targets` clean.
- [x] **v0.7.5.4a** — Items: top-level driver + functions + const + typedef + visibility (this commit)
  - `compiler/parser.tri` (+876 LOC, ~3341 total) — ports the upper half of `crates/triet-parser/src/item.rs`. Adds the full `Item` enum (all 8 variants scaffolded so the AST + Arena shape stays stable across the `.4a` → `.4b` split): `FunctionItem(FunctionDef)` / `ConstantItem(ConstantItemPayload)` / `TypeAliasItem(TypeAliasPayload)` / `StructItem(StructDef)` / `EnumItem(EnumDef)` / `ImportItem(ImportPath)` / `ImportFromItem(ImportFromPayload)` / `ModuleItem(ModuleDecl)`. New supporting types: `Visibility` (PrivateVis / PublicVis / PackageVis per ADR-0005), `ParameterPassing` (BorrowedParam / MutableParam / OwnedParam per SPEC §10.3), `FunctionParam`, `FunctionBody` (BlockBody / ExpressionBody), `FunctionDef`, `ConstantItemPayload`, `TypeAliasPayload`, plus stub `StructField` / `StructDef` / `EnumVariant` / `EnumDef` / `ImportPath` / `ImportName` / `ImportFromPayload` / `ModuleContent` / `ModuleDecl` for `.4b`. Arena grows from 4 to 5 sub-arenas with `items: Vector<SpannedItem>` keyed by Integer; `Program { arena, item_ids }` is the new root container.
  - Parser surface: `parse_program` (loops over items till EOF, collects IDs in source order), `parse_item` (visibility prefix + keyword dispatch — struct/enum/module/import stubs surface as `UnexpectedTokenErr` until `.4b`), `parse_visibility` (handles `public` / `public(package)` per ADR-0005), `parse_function` (+ `parse_parameter_list` / `parse_parameter` / `parse_generic_params` / `parse_function_body`), `parse_constant_item`, `parse_type_alias`. Helpers: `parse_ident`, `parse_item_name` (+ ADR-0005 reserved-name check for `std` / `sys` / `dev` / `usr` / `core` via new `ReservedItemNameErr`), 11 new token predicates (`is_function_kw` / `is_constant_kw` / `is_type_kw` / `is_struct_kw` / `is_enum_kw` / `is_module_kw` / `is_import_kw` / `is_from_kw` / `is_public_kw` / `is_lparen` / `is_rparen` / `is_lt` / `is_gt` / `is_thin_arrow` / `is_owned_kw`). Step wrappers: `ParseItemStep`, `ParseProgramStep`, `ParseVisibilityStep`, `ParseIdentStep`, `ParseParamsStep`, `ParseParamStep`, `ParseGenericsStep`, `ParseFunctionBodyStep`. `parse_function_body` consumes either `{ … }` (via v0.7.5.3 `parse_block`) or `= expr` (via v0.7.5.2 `parse_expression`). Smoke `main()` asserts 15 new program snippets covering every supported item shape + every visibility / passing combination.
  - **Six pre-existing lowerer struct-tracking gaps surfaced + fixed under this gate** (all in `crates/triet-ir/src/lowerer.rs`):
    1. **While-loop phi.** `let mutable state: T = …; while … { state = step.state }` — the phi at the loop header didn't inherit pre-loop struct identity, so post-loop `state.field` fell back to `field_idx=0`. Fix: pre-propagate the pre-loop value's struct identity onto `phi_dest` BEFORE the body lowers (the user-declared `let mutable name: T` contract guarantees rebinds preserve T), plus the post-body symmetric check.
    2. **Match-arm mutated-var phi.** `match … { … => state = step.state, _ => {} }` — the per-arm mutated-var phi at the merge didn't propagate struct identity when every arm agreed.
    3. **Match-expression merge phi.** `match … { ~+ … => ~+ X, _ => ~+ X }` — the expression `merge_dest` didn't carry `value_struct_types` or `value_outcome_value_struct` when every arm agreed.
    4. **If-expression merge phi.** `if cond { ~+ X } else { ~+ X }` — same two-incoming merge as match, same gap.
    5. **Outcome constructor literal-side propagation.** `~+ StructValue { … }` didn't seed `value_outcome_value_struct[dest]` from the payload's struct identity, so a subsequent `~?` unwrap dropped the identity. Literal-side analogue of the call-site `func_return_outcome_value_struct` seeding.
    6. **Let-with-type-annotation seeding.** `let p: T = get(v, i)!!` — the `Vector<T>` element extraction is opaque to the lowerer (T-generic), so `p` ended up untracked even though the user wrote the type. Fix: seed `p` from the let's `type_annotation` when the value isn't already tracked.
  - Refactored into a single `shared_struct_identity(tracker, value_ids) -> Option<String>` helper to deduplicate the four shared-identity propagation call sites (while-loop phi, match-arm mutated-var phi, match merge_dest, if merge_dest).
  - **One Triết-side bug surfaced + fixed**: an earlier draft of `parse_function`'s optional-return-type block mixed `ParseLetAnnotation` and `~- err` in two arms of the same `match` — invalid since arms must agree on result type. Restructured to thread `ParseLetAnnotation~ParseError` through a `~?` boundary at the end, mirroring `parse_optional_type_annotation` in v0.7.5.3.
  - 6 new integration tests: `parser_item_smoke.rs` (1, full end-to-end on the new `compiler/parser.tri`) + `phi_struct_tracking.rs` (5, one per lowerer fix — pin each independently of the bootstrap source so future regressions land at the right blame line). 1284 workspace tests pass (was 1278, +6); `cargo clippy --workspace --all-targets` clean.
- [x] **v0.7.5.4b** — Items: struct + enum + module + imports (this commit)
  - `compiler/parser.tri` (+669 LOC, ~4223 total) — ports the lower half of `crates/triet-parser/src/item.rs`. Wires the remaining 5 Item variants .4a scaffolded: `StructItem` / `EnumItem` / `ImportItem` / `ImportFromItem` / `ModuleItem`. Closes the item layer of the umbrella.
  - New parsers: `parse_struct` (+ `parse_struct_field` / `parse_struct_fields`); `parse_enum` (+ `parse_enum_variant` / `parse_enum_variants` — optional tuple-style payload `Some(T)`); `parse_module` — file-bound `module foo` + inline `module foo { items… }` (recursing into `parse_item` via `parse_inline_module_body`, which lets `module data { struct Cell { … } }` work in a single declaration); + ADR-0005 §"File resolution" rejection of `module foo.bar` dotted declarations. `parse_import_item` (`import a.b.c`); `parse_from_import` (`from a.b import x, y as z` per ADR-0005 §"Imports — Python style"); `parse_import_name_list` / `parse_import_name` (+ ADR-0005 glob `*` rejection + optional `as alias`); and the `parse_dot_path` family (root accepts `crate` / `self` / `super` per ADR-0005 §"Path syntax"; subsequent segments must be regular identifiers). Visibility on `import` / `from` is explicitly rejected via `reject_visibility_on_import` (re-exports of imported names are a post-v0.2.x feature).
  - 8 new step wrappers (`ParseStructFieldStep` / `ParseStructFieldsStep` / `ParseEnumVariantStep` / `ParseEnumVariantsStep` / `ParseDotPathStep` / `ParseImportNameStep` / `ParseImportNamesStep` / `ParseModuleContentStep`) + 6 new token predicates (`is_dot` / `is_star` / `is_as_kw` / `is_crate_kw` / `is_self_kw` / `is_super_kw`).
  - Pretty-printer extended for all 5 new Item variants + helper printers `format_struct_fields` / `format_enum_variants` / `format_dot_path` / `format_import_names`. Smoke `main()` adds 15 new assertions covering every shape (incl. generic struct `Box<T>`, generic enum `Option<T>`, dotted-path imports, `as` alias, inline + external modules, `public(package)` on a module).
  - **One Triết-side gotcha recorded**: match arms with a single-statement body need braces — `~0 => should_continue = false,` parses the `=` as a pattern continuation. Wrap as `~0 => { should_continue = false }` instead. Mirrors the v0.7.5.3 `parse_block` pattern.
  - 1284 workspace tests still pass (no new bootstrap test added — the existing `parser_item_smoke.rs` from .4a runs against the same `compiler/parser.tri` and now covers the .4b assertions automatically). `cargo clippy --workspace --all-targets` clean.
- [x] **v0.7.5.5a** — Full type grammar (this commit)
  - `compiler/parser.tri` (+483 LOC, ~4706 total) — expands the v0.7.5.3 placeholder `parse_type_minimal` (single-Identifier `NamedType` only) into the full ADR-0020 + ADR-0021 grammar **in place**. Every existing call site (let / constant / parameter annotation / return type / struct field / enum variant payload / type alias body) inherits the full grammar without touching its signature. New `TypeExpr` variants: `GenericType(GenericTypePayload { name, arguments })`, `TupleType(Vector<Integer>)`, `NullableType(Integer)`, `FunctionType(FunctionTypePayload { parameters, return_type })`, `OutcomeType(OutcomeTypePayload { value_type, error_type, allow_null_state })`, `RefinedTrileanType`.
  - Parser surface restructured as atom + postfix split: `parse_type_atom` handles NamedType (+ optional generic args via `parse_generic_args`) or paren-form (via `parse_paren_type` — disambiguates grouping `(T)`, singleton tuple `(T,)`, multi tuple `(T1, T2 …)`, and function type `(T1, T2) -> R` after-the-fact); `apply_type_postfix` runs ADR-0020 §1.3 precedence — `Trilean!` (only after a bare `Trilean` atom per ADR-0021 §2.7) → `T?~E` (compound `?~`) → `T~E` (bare `~`) → `T?…?` chained nullable via `apply_chained_nullable`. New helper `parse_type_list_until_rparen` returns both the type list AND a trailing-comma flag, letting `parse_paren_type` distinguish `(T)` from `(T,)`.
  - 4 new token predicates: `is_question` / `is_question_tilde` / `is_tilde` / `is_bang`. 1 new step wrapper: `ParseTypeListStep`.
  - `format_type` pretty-printer extended for all 6 new variants + helper formatting for generics, tuples, function-type params, outcome operators. Smoke `main()` adds 16 new type-level assertions covering NamedType, `Trilean!`, single/chained nullable, single/multi/nested generics, tuple (incl. `(T,)` vs `(T)`), function types, and outcome (binary + ternary, with generic value type).
  - 1 new integration test (`parser_type_smoke.rs`) — same end-to-end shape as the v0.7.5.{1,2,3,4} smoke tests. 1285 workspace tests pass (was 1284, +1); `cargo clippy --workspace --all-targets` clean.
- [x] **v0.7.5.5b** — Full pattern grammar (this commit)
  - `compiler/parser.tri` (+~700 LOC) — expands the v0.7.5.3 minimal `Pattern` (Wildcard / Identifier) into the full grammar per `crates/triet-parser/src/pattern.rs`. New AST: `LiteralPattern` sub-enum (Integer / Ternary / String / Trilean) + 4 new payload structs (`LiteralIntegerPattern` / `RangePatternPayload` / `EnumVariantPatternPayload` / `OutcomeArmPatternPayload`). New `Pattern` variants: `LiteralPat`, `TuplePat`, `OrPat`, `RangePat`, `NullPat`, `EnumVariantPat`, `OutcomeArmPat`.
  - Parser surface: `parse_pattern` (top level — handles or-patterns) + `parse_pattern_no_or` (sub-pattern, keeps `|` from binding across structural boundaries — used by tuple elements, enum variant payloads, outcome arm payloads). Atom helpers: `parse_identifier_or_enum_variant_pattern` (Identifier-vs-EnumVariant disambiguated by `(` lookahead), `parse_tuple_pattern` (singleton `(p,)` vs grouping `(p)` distinguished by trailing-comma flag, mirroring the v0.7.5.5a tuple type), `parse_trilean_pattern_atom`, `parse_literal_or_range_pattern`, `parse_negative_literal_pattern` (`-5` / `-9_tryte` shape), `parse_literal_pattern_payload` (shared by start + range-end), `parse_outcome_arm_pattern_with_payload` (the `~+ binding` / `~- binding` cases — `~0` ships inline at parse_pattern_no_or per ADR-0020 §5.2).
  - 6 new token predicates: `is_pipe` / `is_dotdot` / `is_dotdoteq` / `is_underscore` / `is_null_kw` / `is_minus`. 1 new step wrapper: `ParseLiteralPatternStep`.
  - `format_pattern` pretty-printer extended for all 7 new variants + helper `format_literal_pattern` for the literal sub-enum. Smoke `main()` adds 24 new pattern-level assertions covering every variant (incl. negative range bounds, singleton-tuple-vs-grouping distinction, multi-level or-patterns, multi-element variant payload via explicit tuple wrap `Cell((c, d))`).
  - **One Triết grammar reminder recorded**: enum variants take exactly one payload (per syntax SPEC). For tuple-shaped payloads use explicit wrap `Cell((c, d))` — the Rust impl has the same constraint.
  - 1 new integration test (`parser_pattern_smoke.rs`) — same end-to-end shape as v0.7.5.{1..5a}. 1286 workspace tests pass (was 1285, +1); `cargo clippy --workspace --all-targets` clean. Closes v0.7.5.5 — next is `.6` (error recovery + `parser_differential` NDJSON gate, closes the v0.7.5 umbrella).
- [ ] **v0.7.5.6** — Error recovery + `parser_differential` NDJSON gate (closes v0.7.5 umbrella)

### Remaining v0.7 sub-tasks after parser

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
