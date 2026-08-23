# ADR 0071 — Path Separator (`::`) & Module Import (`use`)

**Status:** **🔒 SEALED** (Signed by Mentor G 2026-06-26; blood-verified by O for Slice 1 (4 safeguards) + Slice 2 (5 safeguards) failing independently + byte-identical restore). Applicable to rewrite-era (Tier C). **Supersedes [ADR-0005](0005-module-system.md)** regarding dot-paths + Python-style imports: changes `.`→`::` for static paths, eliminates `import`/`from` in favor of `use`. NO revisionism: ADR-0005 status updated to `Superseded by ADR-0071`, body retained intact.

**Issue:** ADR-0005 chose `.` as the path separator (Java/Python instinct) and INTENTIONALLY rejected `::` on the grounds that "a two-phase resolver does not require lexical syntax differentiation" (ADR-0005 §"Why dot path"). In practice, `.` (`Token::Dot`) became **overloaded** with three distinct roles — path separator (`std.io`), enum variant (`Color.Red`), and field access (`obj.field`) — and the parser CANNOT distinguish between them; it offloaded everything onto **typecheck** heuristics (`expr_resolutions` map recorded `Color.Red`=FieldAccess→variant, `Color.Red(x)`=MethodCall→variant). The AST was ambiguous and lacked semantic clarity. When building Module Resolution + Traits, this ambiguity became severe technical debt.

**Rationale for reversal (G, 2026-06-25):** *"Strictly separate Static Resolution (`::`) and Instance Access (`.`) at the Syntax (Syntax/AST) layer, clearing the path for Traits and Module Resolution."*

---

## Decision

### 1. Two strictly separated access operators in SYNTAX
- **`::` — Static Resolution** (resolved at compile-time, no receiver instance): namespace/module paths, types, **enum variants** (`Color::Red`).
- **`.` — Instance Access** (dynamic receiver present): fields (`obj.vga`), methods (`hw.use_vga()`), tuple-indices (`t.0`), safe-navigation (`obj?.field`).

The parser categorizes AST nodes **solely by token**, without relying on typecheck heuristics. `Color.Red` (dot on a type-variant) → **parse/typecheck error**, no longer "guessed correctly".

### 2. Eliminating `import`/`from`/glob → `use`
`from X import a` split the unified path `X::a` into two fragments separated by keywords. A path must be a **single unified entity**. New syntax:
```triet
use std::io::println;      // import single item
use std::io;               // import whole module (used as std::io::println)
use std::io::{a, b, c};    // brace-group multi-import (LOCKED — replacing from..import a,b,c)
use std::io::out as o;     // rename retains `as` (after path / inside braces)
```
Keywords `import` and `from` **are deleted from the lexer**. Wildcard `*` remains forbidden (ADR-0005 §exclusions retained).

### 3. Enum variants via `::` — REUSING existing nodes (NO new AST nodes)
- `Color::Red` → `Expr::EnumLiteral { name: "Color", variant_name: "Red", payload: None }` (`EnumLiteral` node already exists, now receiving qualified form).
- `Color::Red(x)` → `Expr::EnumLiteral { ..., payload: Some(x) }`.
- Pattern `Color::Red` / `Color::Red(x)` → `Pattern::EnumVariant { name: Some("Color"), variant_name, payload }` (the `name` field was previously always `None`, now populated during parsing).
- **Remove typecheck hack**: `check/exprs.rs` no longer records `expr_resolutions` for `FieldAccess(Type,field)`/`MethodCall(Type,m)`→variant.
- **MANDATORY QUALIFICATION (LOCKED by G+Giang 2026-06-25):** all user-enum variants must be `Type::Variant` (in both expressions and patterns). **Bare un-qualified `Red`/`None` → error.** CLEAN outcome: a bare identifier in pattern position is now **unambiguously a variable-binding**, eliminating variant-vs-binding ambiguity → simplifies `pattern.rs` + pattern typecheck (`Pattern::EnumVariant.name` is always `Some`, eliminating bare-name→variant pathways). *(Scope note: applies only to user-enums; `~0`/`~+`/`~-` (Outcome) and `true`/`false`/`unknown` (Trilean) are literals/operators, NOT variants — untouched.)*

### Measured Scope (recon file:line)
| Layer | Location | Slice |
|---|---|---|
| Lexer | `token.rs:151/155/159` keywords import/from/as; add `ColonColon` next to `Colon:369`/`Dot:377` | 1+2 |
| Parser import | `item.rs:48/72` dispatch · `parse_import:451` · `parse_from_import:473` · `parse_dot_path:571` · `parse_import_name:509` (`as`) | 1 |
| AST Item | `Item::Import`/`Item::ImportFrom` (schema-gen `ast_item.rs:159/175`) → unified into `Item::Use` (schema-first) | 1 |
| Resolver | `resolver.rs:145 collect_imports` · `208 resolve_whole_import` · `293 resolve_from_import` | 1 |
| Parser expr | `expr.rs:725 parse_postfix` · `879 parse_dot_postfix` (FieldAccess/MethodCall) | 2 |
| Typecheck variant-hack | `check/exprs.rs:182-201` (MethodCall→variant) · `1567-1601` (FieldAccess→variant) | 2 |
| Parser pattern | `pattern.rs:63-84` (populate `EnumVariant.name`) | 2 |
| `.tri` sweep | import: 1 example + 22 fixtures · `Type.Variant`: sweep entire corpus | 1+2 |
| Docs | SPEC.md, CLAUDE.md §Language convention table, ADR-0005 status | 1+2 |

### Slices (each slice blood-verified by O + signed by G)
- **Slice 1 — `use` + `::` import path.** ✅ CLOSED (`4a7da96`). Lexer `::`+`use`/delete import/from; parser use-path; AST `Item::Use` (schema→codegen→consumers); resolver; sweep import `.tri`+docs.
- **Slice 2 — Expr/Pattern static `::` (READING A — ruled by G 2026-06-26: complete removal without exception).** `Color::Red`(+payload) via EnumLiteral; pattern `name:Some`; **eliminate ALL THREE implicit bare/dot-variant mechanisms**; bare unqualified → error EVERYWHERE; sweep entire corpus.

#### §2.A — Slice 2 Reading A: Elimination of three implicit variant mechanisms
Triet previously accepted user-enum variants through THREE implicit pathways — Reading A eliminates all three, leaving only `::` qualified + `use`-imports:
1. **Pattern guess-hack** (`check.rs:892-918`): bare `Red` in match arm → guessed based on *scrutinee*. → ELIMINATED. Thereafter, bare-in-pattern = **variable binding 100%, unconditionally**.
2. **Expr in-scope-enum-scan** (`check/exprs.rs:1216 resolve_enum_variant` + 2 call sites line 101 Identifier, 145-150 Call-bare): bare `Green` → *scans ALL in-scope enums* to find matching variant names. → ELIMINATED. This is implicit magic (G: "landfill thinking"). Import-bound symbols (`use X::{Ok}`) DO NOT use this path — they resolve via `env.lookup` (exprs.rs:91) FIRST, so eliminating the scan does not break import bindings.
3. **Dot-variant hacks** (3 sites): `check/exprs.rs:182-201` MethodCall→variant (`Color.Red(x)`) · `check_field_access` FieldAccess→variant (`Color.Red`) · `exprs.rs:152-172` Call-FieldAccess (`CD.SomeInt(5)`). → ELIMINATE all three. `.`-variant = error (FieldAccess on enum-TYPE, which has no fields).

**Consequence — enum-match accepts Variable-arm as catch-all:** eliminating ① exposed a hidden gap: "Variable-binding on enum-match" had never executed (the heuristic had previously intercepted every bare identifier arm as a variant). For "bare = binding 100%" to hold, enum-match must accept `Pattern::Variable` as catch-all (binding both scrutinee + default-case) — **symmetrical with `has_scalar_catch_all` already present in scalar-match (ADR-0064 §8)**. This is the *execution* of Reading A, NOT a new feature. Patched in 2 locations: `check_enum_exhaustiveness` short-circuit (adding Variable alongside Wildcard) + lower enum-match else-branch (default-case + bind). A bare identifier SHARING a variant name (`match c { Green => }`) now binds the entire `c`, NOT matching variant Green.

**Consequence — E1018 AmbiguousEnumVariant RETIRED:** E1018 was emitted exclusively from `resolve_enum_variant` (pathway ②). Eliminating ② leaves E1018 obsolete (bare identifiers are illegal → ambiguity cannot occur). Removed variant `TypeError::AmbiguousEnumVariant` + emitter + help-text (no dead code). E1018 retired — code not reused. Following Reading A: **every user-variant reference EXPLICITLY NAMES the enum** (`Color::Red`) OR is an import-bound symbol (`use`); no third pathway exists.

### Measured Scope (recon file:line)
| Layer | Location | Slice |
|---|---|---|
| Lexer | `token.rs` keywords import/from/as; `ColonColon`+`use` | 1 ✅ |
| Parser import | `item.rs` dispatch · `parse_use*` replacing `parse_import/from/dot_path` | 1 ✅ |
| AST Item | `Item::Import`/`ImportFrom` → `Item::Use` (schema-first) | 1 ✅ |
| Resolver | `resolver.rs collect_imports` routes Item::Use | 1 ✅ |
| Parser expr/pattern | `::` primary-level → EnumLiteral / Pattern::EnumVariant{name:Some} | 2 |
| Typecheck eliminate ① | `check.rs:892-918` pattern guess-hack | 2 |
| Typecheck eliminate ② | `check/exprs.rs:1216 resolve_enum_variant` + call sites 101/145 + E1018 retire | 2 |
| Typecheck eliminate ③ | `check/exprs.rs:182-201` + `check_field_access` + `152-172` | 2 |
| `.tri` sweep | bare/dot variants ~25 fixtures + ~13 expr-constructions + examples | 2 |
| Docs | SPEC enum, CLAUDE.md table (`Color.Red`→`::`), E1018 retire | 2 |

---

## Alternatives Considered

| # | Alternative | Pros | Cons | Conclusion |
|---|-------------|------|------|------------|
| 1 | **Option B Rust-model: `::` static / `.` instance + `use`** (chosen) | Parser categorizes AST via tokens; removes typecheck hacks; clears path for Traits/Modules | Broad corpus sweep; reverses locked ADR | **CHOSEN** (G lock) |
| 2 | Option A: `::` only in `import`, call-site retains `.` | Narrower sweep | `std.io.x` still conflates paths/fields → parser still guesses = NOT clean | Rejected (G: "compromised junk") |
| 3 | Add generalized multi-segment `Expr::Path` node | Generalizes future call paths | Qualified calls NOT YET implemented → YAGNI; new node adds heavy schema+lower+typecheck burden | Deferred until qualified calls needed |
| 4 | Keep `from..import` alongside `use` | Non-breaking | Dual syntax = ambiguous AST as before | Rejected (G: complete removal) |

---

## Consequences

### Positive
- AST reflects true semantics: static nodes (EnumLiteral/Use) vs instance nodes (FieldAccess/MethodCall) distinguished at parse time.
- Removes `expr_resolutions` typecheck hacks for dot-variants → cleaner, leaner typechecker.
- Paths are unified `::` blocks → clean foundation for future Module Resolution + Trait paths.

### Negative
- Broad sweep across `.tri` corpus + documentation (foundational syntax change).
- Reverses a LOCKED ADR — requires explicit supersession, updating all docs referencing "dot paths".

### Risks to Mitigate
- **`::` vs `:` in lexer**: longest-match rule, `:` (type annotation) must not be greedily consumed. Safeguard: `let x: Integer` parses correctly.
- **Instance-access regression**: `obj.field`/`obj.method()`/`t.0`/`obj?.field` MUST retain `.`. Regression safeguards required.
- **Schema-first integrity**: `Item::Use` modified via schema+codegen, NEVER hand-editing generated files.

---

## Poison-teeth matrix (G mandate)

| Safeguard | Input | Expected Result |
|---|---|---|
| T-dot-path | `use std.io::x` (dot inside path) | **parse error** |
| T-old-import | `import std::io` / `from std::io import x` | **parse error** (keywords deleted) |
| T-dot-variant | `Color.Red` (expr) | **error** (no longer resolves to variant) |
| T-colon-variant | `Color::Red` / `Color::Red(x)` | parses → EnumLiteral → executes |
| T-bare-variant | bare `Red` (user-enum, unqualified) | **error** (qualification mandatory) |
| T-use-ok | `use std::io::println;` | parses OK + resolves |
| T-brace-use | `use std::io::{a, b};` | parses 2 bindings |
| R-field (regression) | `obj.field`, `hw.use_vga()`, `t.0`, `obj?.field` | retains `.`, executes unchanged |
| R-bind (regression) | bare ident in pattern (`match x { y => }`) | variable-binding, NOT variant |
| R-colon-annot (regression) | `let x: Integer = 1` | `:` annotation parses correctly |

---

## Effective Date
- Rewrite-era Tier C — activated slice-by-slice upon WO closure (O verification + G signature).
- **Supersedes ADR-0005** §dot-path + §Python-import: ADR-0005 status updated to `Superseded by ADR-0071`, body retained (historical record).
- Applies to all new `.tri` code; existing corpus swept in campaign. NO backward-compatibility mode for `.`-paths (complete removal).

## Supplementary Decisions (Finalized by G+Giang 2026-06-25)
1. **Brace-groups LOCKED:** `use a::b::{x, y};` supports multi-imports (replacing `from..import a,b,c`). Renaming inside braces: `{x, y as z}`.
2. **Mandatory qualification LOCKED:** all user-enum variants must be `Type::Variant`; bare `Red`→error (see §3). Broader sweep (qualifying all match arms) yielding cleaner pattern parsing (bare identifiers are guaranteed bindings).
