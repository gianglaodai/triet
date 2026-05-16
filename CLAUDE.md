# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Author–AI collaboration model

The author (**Giang Hoàng**) owns the **goals, vision, direction, and final
technical decisions**. He is not a compiler engineer — he drives the project
as a product owner with a clear philosophical direction (balanced ternary,
AI-first, stability over speed). Technical implementation is delegated to the AI.

**When you propose any technical recommendation:**
1. **Read the source-of-truth docs first** — `SPEC.md` (semantics) and
   `VISION.md` (architectural pillars). The author's intent is recorded there.
   Your recommendations must align with the design philosophy already decided.
2. **Present tradeoffs in terms the author cares about** — not compiler-theory
   jargon, but: "this makes the language simpler for users", "this preserves
   the ternary identity", "this defers risk to a later phase".
3. **Surface which ADR or SPEC section supports your choice.** If none exists,
   propose writing one before implementing.
4. **The author decides.** Present options clearly, recommend one, explain why.
   Don't proceed with architecturally significant changes without alignment.

The author has explicitly stated: *"Tôi không có kiến thức gì về lập trình
1 ngôn ngữ cả"* — but he knows what he wants the language to BE. Bridge
that gap by grounding every recommendation in the project's own documents.

## What this is

Triết is a balanced-ternary, AI-first programming language implemented in Rust. The codebase is a Cargo workspace with a `parse → modules → typecheck → interpret` pipeline, a register-SSA IR + bytecode VM, a crate-pack distribution format (`.tripack`), and a content-addressed package store (`~/.triet/store/`). Long-term aim is OS-capable; **current state is v0.5 — CAS Packaging shipped** (interpreter + VM remain dev tiers per VISION §4; production AOT lands v2.0).

Source-of-truth docs:
- `SPEC.md` — language semantics (authoritative, currently v0.5)
- `VISION.md` — 5 architectural pillars + OS-capable trajectory
- `ROADMAP.md` — phasing v0.2.x → v3.0 with version gates; **next: v0.6 Capability System**
- `TODO.md` — short-term sub-task tracker with commit hashes
- `docs/decisions/` — 15 ADRs for architectural decisions (see `docs/decisions/README.md` for an index)

## Development principles

### 1. Think Before Coding

Don't assume. Don't hide confusion. Surface tradeoffs.

Before implementing:
- **State your assumptions explicitly.** If uncertain, ask.
- **If multiple interpretations exist, present them** — don't pick silently.
- **If a simpler approach exists, say so.** Push back when warranted.
- **If something is unclear, stop.** Name what's confusing. Ask.

### 2. Simplicity First

Minimum code that solves the problem. Nothing speculative.

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: *"Would a senior engineer say this is overcomplicated?"* If yes, simplify.

### 3. Surgical Changes

Touch only what you must. Clean up only your own mess.

When editing existing code:
- **Don't "improve" adjacent code, comments, or formatting.**
- **Don't refactor things that aren't broken.**
- **Match existing style**, even if you'd do it differently.
- **If you notice unrelated dead code, mention it** — don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that **your** changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: every changed line should trace directly to the user's request.

### 4. Goal-Driven Execution

Define success criteria. Loop until verified.

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:

```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

## Common commands

```bash
cargo build                              # debug
cargo build --release                    # release
cargo test --workspace                   # all tests across crates
cargo test -p triet-parser               # one crate
cargo test -p triet-parser test_name     # one test
cargo clippy --workspace --all-targets   # lint (workspace lints are strict — fix every new warning)
cargo fmt --all                          # format

# Run a .tri program (build the binary first)
cargo build --release
./target/release/triet run examples/fizzbuzz.tri
./target/release/triet check examples/fizzbuzz.tri    # parse+typecheck only
./target/release/triet --json run examples/foo.tri    # machine-readable diagnostics
```

Tests must be **green before any commit**. The user's "stability over speed" principle is non-negotiable — do not bypass failing checks with `--no-verify`, `#[allow]`, or `#[ignore]`.

## Architecture

Compilation pipeline (each stage = one crate):

```
.tri source
    │
    ▼  triet-lexer        tokens (logos-based)
    ▼  triet-parser       AST (recursive descent + Pratt)
    ▼  triet-modules      ResolvedProgram (loader + resolver)
    ▼  triet-typecheck    type errors
    ▼  triet-ir           register-SSA IR + lowerer + bytecode VM
    ▼  triet-interpreter  tree-walking runtime values (dev tier)
    ▼  triet-pack         .tripack format + cross-package linker
    ▼  triet-cli          binary, miette diagnostics, JSON output
```

Foundation crates: `triet-core` (Trit/Tryte/Integer/Long arithmetic), `triet-logic` (Trilean Łukasiewicz Ł3 / Kleene K3), `triet-syntax` (AST types + arena).

### Arena-based AST
`triet-syntax` allocates recursive nodes (`Expr`, `Stmt`, `Pattern`, `TypeExpr`) in typed sub-arenas inside `Arena`. AST nodes hold `*Id` handles (`u32`-sized) instead of `Box<T>`. Always go through `arena.expression(id)` etc. — never fabricate IDs.

### Module system (shipped v0.2.x; ADR-0005 locked)
`triet-modules` produces `ResolvedProgram` instead of a bare `Program`:
- **Multi-arena**: `Vec<Arena>` — one arena per parsed source file. Inline `module foo { … }` shares the parent's arena via `arena_id`; file-bound `module foo` gets a fresh arena. This sidesteps cross-file ID remapping.
- **Flat module list**: `Vec<Module>` indexed by `ModuleId`. Each `Module` has `bindings: HashMap<String, AbsolutePath>` populated by name resolution.
- **Stdlib as real files**: `std/io.tri`, `std/text.tri`, `std/assert.tri`, `std/result.tri` resolved from filesystem (loader walks from `CARGO_MANIFEST_DIR/../../std` or `./std`). Earlier "synthetic registry" approach replaced in v0.2.x.7.
- **Locked architecture decisions** (per ADR-0005, do not change):
  - Single-file = crate root (Python/Go pattern)
  - Inline ≡ file-bound for path resolution (Rust/OCaml precedent)

### IR + bytecode VM (shipped v0.3; ADR-0007/0008/0010)
`triet-ir` lowers AST to a register-SSA IR (53 opcodes) and runs it on a stack-of-frames VM. `.triv` is the wire format (currently v3 — bumped at ADR-0010 for `BR_TRILEAN` and ADR-0012 for `WITNESS_CALL`). The VM is **development tier only** per VISION §4.3; production target is AOT (v2.0) and trytecode (v∞).

ADR-0010 ternary-native IR locks: `BrTrilean` 3-way branch, strict `if cond` panics on Unknown (SPEC §7.1.1), `Eq`/`Ne` propagate Trilean::Unknown per Ł3, `Constant::Null` is the canonical encoding of `Trit::Zero` discriminator (not a separate "thing").

### Crate-Pack distribution (shipped v0.4; ADR-0011/0012/0013)
`triet-pack` defines `.tripack` (container: ABI metadata + IR code + reserved sections for witness tables + manifest) and the cross-package linker (`plan_link`). Two-level hash at pack level: `iface_hash` (ABI surface) + `impl_hash` (covers code bytes). BLAKE3, canonicalized via sort-by-name so identical surfaces produce identical bytes.

Linker decisions land in the E2300–E2399 namespace: `MajorVersionMismatch` (E2320), `VersionBelowMinimum` (E2321), `IfaceHashDrift` (E2310 advisory). `iface_hash_pin` is the final arbiter — semver triple is *declaration*, hash is *proof*. Auto-shim is explicitly NOT promised.

### CAS Packaging (shipped v0.5; ADR-0014/0015)
Extends the pack-level hash from v0.4 into a **3-cấp hash tree**: term + module + package. Each level has its own `iface_hash` (signature-only) + `impl_hash` (covers body bytes), with 16-byte ASCII domain separators per level to prevent cross-level collisions. `abi_version` bumped 1 → 2 (additive — `.tripack` v=1 explicitly refused per ADR-0014 §5, no shim).

Package store lives at `~/.triet/store/` (override via `$TRIET_STORE`). Three branches mirror the hash tree: `term/<impl_hash>/{iface.bin, body.bin}`, `mod/<impl_hash>/index.bin`, `pkg/<impl_hash>/{pack.tripack, manifest.bin}`. Plus `names/<pkg>/<semver>.link` (symbolic alias → hash), `roots/<project_id>.root` (GC roots), `tmp/<uid>/` (atomic install staging). Atomic install protocol: write to tmp dir → `rename()` (POSIX atomic; EEXIST = race-lost = success). Manual `triet store gc` (mark-and-sweep). E2360–E2382 namespace covers store I/O + lockfile + resolver errors.

`triet.lock` hand-rolled line format (`format_version 1` + `pkg <name> <maj>.<min>.<pat> <iface_hex> <impl_hash_hex>`) — sort-by-name canonical, diff-friendly, no serde dep. `Resolver` (lockfile authoritative when present + still in store; dep `iface_hash_pin` overrides cache).

CLI: `triet store {import,list,gc}` (lossy v=1 migration deferred until v=1 packs exist in the wild). Body-level RAM dedup (`body.bin`) chờ lowerer per-term IR body split — iface-level dedup proven via `tests/shared_loading.rs`.

### Error code namespace
- `triet::lex::E0000` — lexer
- `triet::parse::E000X` — parser
- `triet::typecheck::E10XX` — type checker
- `triet::runtime::E20XX` — interpreter
- `triet::modules::E21XX` — loader / resolver (E2100 = cyclic, E2101 = file-not-found, etc.)

All errors implement `miette::Diagnostic`. The CLI's `--json` flag also needs each variant in `parse_error_code` / `type_error_code` / `runtime_error_code` mappers in `crates/triet-cli/src/main.rs` — keep them in sync when adding variants.

## Language conventions (don't get these wrong)

These are decisions locked by ADRs. Code generation, examples, error messages, and doc comments must match.

| Use | Don't use | ADR |
|---|---|---|
| `function` | `fn` | ADR-0005 (verbose keywords) |
| `public` / `public(package)` | `pub` / `pub(crate)` | ADR-0005 |
| `mutable` | `mut` | ADR-0005 |
| `constant` | `const` | ADR-0005 |
| `module` | `mod` | ADR-0005 |
| `crate.foo.bar` | `crate::foo::bar` | ADR-0005 (dot paths) |
| `from std.io import println` | `use std::io::println` | ADR-0005 |
| `!a`, `a && b`, `a \|\| b`, `a ^ b`, `a => b` | — | SPEC §4.2 (symbolic preferred) |
| `a ~> b`, `a ~^ b`, `a <=> b`, `a <~> b` | — | SPEC §4.2 (Kleene variants) |
| `0t+`, `0t-`, `0t0` (prefix trit literal) | `0T` (capital T), suffix `_trit` | SPEC §1.5.1 |
| `unknown` (third Trilean value) | `null` | SPEC §1.5.2 |

Reserved namespace roots (cannot be user identifiers): `std`, `sys`, `dev`, `usr`, `core`, `crate`, `self`, `super`.

`Trilean` defaults to **Łukasiewicz Ł3** semantics (not Kleene). Don't substitute Boolean reasoning when working on logic ops.

**Logic operators:** Both symbolic (`!`, `&&`, `||`, `^`, `=>`, `~>`, `~^`, `<=>`, `<~>`) and keyword (`not`, `and`, `or`, `xor`, `implies`, `kleene_implies`, `kleene_xor`, `iff`, `kleene_iff`) forms are valid. Symbolic form is preferred per user convention. The `~` prefix consistently marks Kleene K3 variants.

## Workspace conventions

- Rust 2024 edition, stable channel (`rust-toolchain.toml`).
- Workspace lints are strict: `unsafe_code = forbid`, `missing_docs = warn`, clippy `pedantic` + `nursery` at `warn`. Internal crates have `#![allow(clippy::redundant_pub_crate)]` at `lib.rs` to balance with `unreachable_pub`.
- All public items need a doc comment (rustdoc-rendered).
- Miette diagnostics: every error variant gets `#[diagnostic(code(triet::<area>::E<code>))]` plus a `#[label]`-bearing `Span`.

## Development cadence

The user follows a per-step commit pattern:
1. Pick the next sub-task from `TODO.md`.
2. Implement, run `cargo test --workspace` and `cargo clippy --workspace`.
3. Commit with conventional format: `<type>(<scope>): subject` — examples in `git log`. The most recent scope pattern is `feat(v0.5.N): …` / `docs(v0.5.N): …`. Next phase will use `feat(v0.6.N): …`.
4. Push.
5. Update `TODO.md` to mark `[x]` and append the commit short-hash.

Do not commit, push, or run `gh` commands without an explicit ask. The user reviews each step. Only the user runs `cargo run` against examples in interactive sessions — don't auto-run.

When a decision affects future architecture (module shape, ABI, type system), write an ADR in `docs/decisions/000N-<topic>.md` instead of "ship and fix later".

## Examples

Sample programs in `examples/*.tri` exercise specific features. Useful as smoke tests when changing parser/typecheck/interpreter:

```bash
for f in examples/*.tri; do ./target/release/triet run "$f" || echo "FAILED: $f"; done
```

Demos shipped through v0.5: 11 single-file examples in `examples/` (fizzbuzz, factorial, measles_risk, lukasiewicz_vs_kleene, counter, long_arithmetic, enumerate, nullable, while_polling, maybe, generic — all 11/11 byte-identical interpreter vs VM). 1 multi-file module demo in `demos/02-module-system/` (704-line ternary ALU). 1 cross-package linker demo (`crates/triet-pack/tests/cross_package_demo.rs` — 7 integration tests covering accept/refuse/drift cases). 1 shared-loading demo (`crates/triet-pack/tests/shared_loading.rs` — 4 integration tests proving CAS dedup at term + module level). 1 store CLI smoke test suite (`crates/triet-cli/tests/store_cli.rs` — 6 integration tests).
