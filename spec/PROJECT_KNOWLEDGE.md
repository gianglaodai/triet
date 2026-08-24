# Project Knowledge — Triết Compiler

Shared reference for all roles (O, G, D). The single source of truth for project
context, architecture, development rules, language conventions, and workspace standards.

## What this is

Triết is a balanced-ternary-first programming language implemented in Rust.

**Read [VISION.md](../VISION.md) before reasoning about the project's purpose** — it
was rewritten honestly on 2026-06-18 and locks three framings you must not drift
from:
- **OS-capable is a design *constraint*, NOT a destination.** The language must
  never require a mandatory GC / managed runtime and must stay freestanding-
  expressible (Rust-style; binary-HW OS is legitimate). There is NO promise to
  build an OS/microkernel — do not propose "v3.0 kernel" milestones (VISION §7).
- **"AI-first" was REMOVED as a claim (2026-06-22).** Triết makes no AI
  hypothesis — it does not prove, measure, or sell one. The explicit /
  machine-fixable-diagnostic / refuse-over-guess principles exist for
  *correctness + craft*; any benefit to LLM codegen is an unmeasured side-effect,
  never sold. The value anchor is **coherence** — one Ł3 algebra across
  null / logic / capability — NOT turns-to-green (VISION §5 tombstone, §8).
- **Balanced ternary + Ł3 are *identity/inspiration*, NOT a hardware bet.** The
  project does not wait for ternary hardware (VISION §6).

Honesty rules over impressiveness: no "✅ shipped" for deleted code, no "measured"
for any AI benefit (none is claimed), no "OS-capable" for a constraint not yet satisfied
in the implementation (VISION §3).

**The codebase was REBUILT from the backend up on 2026-06-04.** A complete
compiler shipped v0.2-v0.10 and was then **deleted** in a ground-up rewrite.
Read the history honestly so you don't recommend against code that no longer exists.

### What was deleted (the v0.2-v0.10 compiler — gone, not "legacy-but-active")

The shipped compiler ran `parse → modules → typecheck → IR (53-opcode bytecode
VM) → interpreter`, with a delegate-to-VM Cranelift JIT (v0.11 reached 96% JIT
coverage), a self-hosting `compiler/` (~23K LOC Triết-in-Triết), and ~1637 tests.
On 2026-06-04 the backend was **purged**: `triet-ir`, `triet-interpreter`,
`triet-bootstrap`, `triet-cli` crates + 5500 lines of JIT legacy were deleted
permanently (git history retains them). **Do not assume any of these exist.**

> ⚠️ ORPHAN: `compiler/` (the self-host `.tri` sources) was NOT deleted but its
> target IR/VM (`triet-ir`/`triet-interpreter`) WAS. It can no longer bootstrap.
> Treat it as dead weight pending a decision, not as a working self-host.

**`docs/` is the HISTORICAL RECORD of the deleted compiler:**
- `docs/ARCHIVE.md` — **single reference digest** of the deleted v0.2-v0.10
  architecture + a **classified catalog of all 36 ADRs** (LIVE / TOOLING /
  HISTORICAL). The old `docs/ARCHITECTURE.md` + `docs/plans/` were folded into
  this and removed (full text in git history). Read for *intent*, not layout.
- `docs/decisions/` — **44 ADRs**. 0001-0036 belong to the OLD compiler: the ones that
  lock **language semantics** (error codes, diagnostic format, Outcome, Trilean!
  refinement, S6 reference forms, keyword conventions) **remain authoritative**;
  ADRs that describe the deleted *architecture* (VM, bootstrap, old JIT shim ABI)
  are history — see `docs/ARCHIVE.md` §2 for the live/dead tag on each.
  **0037-0044 belong to the NEW compiler** (rewrite-era, all two-mentor-signed):
  0040 heap layout · 0041 nullable PA-3c · 0042 ownership-across-boundary/Deinit ·
  0043 HashMap · 0044 arithmetic range enforcement.

### The current compiler (the rewrite — formerly "Track B")

A single pipeline. Reused frontend + a new backend built from scratch:

```
.tri source
    │
    ▼  triet-lexer + triet-parser       AST (arena-based)      [REUSED, well-tested]
    ▼  triet-modules + triet-typecheck  typed AST              [REUSED, well-tested]
    ▼  triet-lower                      AST → MIR              [NEW]
    ▼  triet-mir                        flat non-nested IR     [NEW]
    ▼  triet-borrowck                   NLL dataflow analysis  [NEW]
    ▼  triet-jit                        Cranelift native code  [NEW]
    ▼  triet-driver                     pipeline binary        [NEW]
```

**Maturity (updated 2026-06-08 — Tier A + Tier B complete, Tier C in progress):**
the backend compiles end-to-end: scalars, arithmetic (**range-enforced
trap-on-overflow per ADR-0044** — Add/Sub/Mul + pow shim, E1036 literal check),
logic ops, control flow, recursion, flat structs (StackSlot + sret), enums
(discriminant switch), **String/Vector/HashMap** (heap shims, move-only +
Deinit tombstone per ADR-0042), **nullable `T?`** (PA-3c `i64::MIN` sentinel,
Elvis, `match ~+/~0`), heap values across user-fn boundaries (B7-lift), NLL
borrowck (E2420/E2440/E2450 + M3/M3+ move tracking), MIR verifier (INV-1/2 +
enum invariants). **NOT yet rebuilt:** borrow params for heap types (`&+ T` —
Tier C slice 2, next), Outcome 2-reg ABI, multi-value return, native layout,
self-host, AOT cache. Workspace tests: **~1086** (gate: `scripts/gate.sh`);
integration corpus **72 fixtures** (driver, numbered 01-76 with 16-19 missing) — the
1637-test VM safety net remains deleted; this is the new net.

Design principles of the rewrite:
- **Schema-driven types:** `spec/schema/triet-schema.yaml` is the SINGLE SOURCE
  OF TRUTH for all type/AST/ownership definitions. Codegen produces Rust structs
  in `crates/triet-syntax/src/generated/`. Hand-editing generated files is
  FORBIDDEN.
- **MIR layer:** Flat, non-nested IR with explicit CFG — purpose-built for
  borrow checking and dataflow analysis.
- **NLL borrow checker:** Polonius-style forward+backward dataflow on the CFG.
- **Native JIT:** Cranelift codegen. Every value is a single `i64` (Tier A/B);
  flat structs use `StackSlot` + sret; heap types (String/Vector/HashMap) live
  behind `extern "C"` Rust shims in `triet-jit/src/mir_lower.rs`; arithmetic
  is range-enforced (`trapnz` → SIGILL; shim traps → SIGABRT — two signal
  families per ADR-0044). Native multi-field layout = Tier C future work.
- **Hardware Token capability:** ZST compile-time tokens enforced by the borrow
  checker — zero runtime overhead (design, not yet implemented).

**`spec/` is the DESIGN AUTHORITY for the rewrite:**
- `spec/schema/triet-schema.yaml` — canonical AST + S6 ownership (⚠️ type system spec-only, hand-written in typecheck)
- `spec/schema/codegen.py` — code generator (Rust now, Triết at v1.0)
- `spec/plans/phase2-borrow-checker-design.md` — CFG + NLL dataflow design
- `spec/plans/phase3-cranelift-backend.md` — Cranelift JIT/AOT architecture
- `spec/plans/phase4-ast-to-mir.md` — AST→MIR lowering strategy
- `spec/plans/phase5-s6-integration.md` — S6 ownership pipeline integration
- `spec/plans/phase6-capability-security.md` — Hardware Token ZST pattern

### Source-of-truth docs

- `SPEC.md` — language semantics (authoritative for the LANGUAGE; header still
  reads **v0.10** and describes the deleted compiler's state — the *semantics*
  are current, the *implementation-status* claims are stale).
- `VISION.md` — 5 architectural pillars + OS-capable trajectory.
- `ROADMAP.md` — ⚠️ the v0.2→v3.0 phasing describes the OLD compiler's route; it has not been
  rewritten for the rewrite's Tier A/B/C reality. Read TODO.md for the real backlog.
- `TODO.md` — **the rewrite's live backlog** (Track B/C, per step with the commit
  hash + debt registry D1-D3). Updated every slice.
- `docs/decisions/` — **44 ADRs**; the language-semantics ones are preserved in
  the rewrite, the architecture ones are history (see "What this is").
- `spec/schema/triet-schema.yaml` — **canonical AST + S6 ownership** (design authority; ⚠️ type system spec-only, hand-written in typecheck).
- `spec/plans/` — **phase designs** for the rewrite (design authority).
  Live status lives in `TODO.md` + this file §The current compiler (Maturity). (REPORT-2026-06-04.md — the
  three-party author/O/G report from the rewrite moment — was deleted; git history keeps it.)

## Development principles

### 0. Before every non-trivial change

1. Check `spec/schema/triet-schema.yaml` — the single source of truth for AST/ownership types
   (⚠️ the type system is hand-written: see §Schema-first discipline).
2. Check `spec/plans/` — the phase plans for the rewrite.
3. Check `docs/decisions/` — ADRs that are still locked (language semantics, error codes, conventions).
4. If the change touches types/AST/ownership, it MUST start from the schema.
5. Check `crates/triet-syntax/src/generated/` — is there already a generated type you should use
   instead of writing a hand-written duplicate?
6. Check §Track B — non-negotiable rules below — these are enforced in review.

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

## Track B — non-negotiable rules (from mentor review, 2026-06-04)

These rules were learned the hard way. Violating any of them will be called out
in review. They apply to every Track B crate (lower, mir, borrowck, jit, driver).

### 1. Compiler never panics on user input

A compiler that panics is a script, not a compiler. Every function that processes
user input MUST return `Result<T, LowerError>` (or equivalent). Use the error
type to carry a `Span` so the driver can print a diagnostic.

- `panic!()`, `unreachable!()`, `unwrap()`, `expect()` — **forbidden** in any
  code path reachable from user input (lowerer, typecheck, borrowck, JIT).
- Unsupported AST constructs → `Err(LowerError::unsupported_*(...))` with span.
- Internal invariants (e.g., "block must have a terminator") → `Err`, not panic.

### 2. Schema-first means schema MUST be used

Generated code that nobody imports is **dead code**, not "documentation."
Dead generated types are a bug — either wire them into the compiler or remove
them from the schema.

- Every `pub enum` / `pub struct` emitted by `codegen.py` must have at least
  one consumer in the workspace.
- The first type migrated (ReferenceForm, 2026-06-04) proves the pipeline works.
  Future types should follow the same pattern: replace hand-written with
  `pub use crate::generated::types::Foo`, add manual impls for missing traits.
- Before adding a new type to the Rust source, check if the schema already
  defines it. If yes, use the generated version.

### 3. Soundness beats test color

Green tests do not prove the code is correct. A soundness hole with all tests
passing is worse than a failing test — it silently generates wrong code.

- **Adversarial self-audit before claiming "done."** Ask: what invariants should
  hold? What edge cases are untested? What assumptions are undocumented?
- Borrowck specifically: `places_conflict` must be conservative. When uncertain
  whether two places alias, **assume they do** (refuse over guess). Different
  base locals are only provably disjoint for exclusive/strong references.
- Every `conflicts_with` / `places_conflict` decision must trace to an S6 rule
  in SPEC §10 or an ADR.

### 4. No dead fields in MIR

Every field in `Body` and every MIR data structure must be **populated** by
the lowerer and **consumed** by at least one backend pass.

- `struct_layouts: Vec<StructLayout>` was defined but always empty — dead code
  with 4 passing tests. Fixed by populating from `Item::Struct` in the lowerer.
- When adding a field to a MIR type, add the corresponding population logic
  in the lowerer **in the same commit**. No "the backend will use this later"
  without a producer.
- `ReturnShape` must be extended to cover struct returns BEFORE the JIT needs
  multi-value return — not after.

### 5. Every `#[allow(...)]` must justify itself

Suppressing warnings hides problems. The codegen `#![allow(unused_imports, missing_docs)]`
was added because the generated code had unused imports and empty doc comments.
The correct fix is to fix the codegen, not silence the warning.

- `#[allow(...)]` in hand-written code → must have a comment explaining why.
- `#[allow(...)]` in generated code → must be tracked as a codegen bug.
- Goal: 0 warnings from `cargo check --workspace` (achieved 2026-06-04).

## Common commands

```bash
cargo build                              # debug
cargo build --release                    # release
cargo test --workspace                   # all tests across crates
cargo test -p triet-parser               # one crate
cargo test -p triet-parser test_name     # one test
cargo clippy --workspace --all-targets   # lint (workspace lints are strict — fix every new warning)
cargo fmt --all                          # format

# GATE — run BEFORE every commit/report; a report means pasting raw output, never hand-copied numbers
bash scripts/gate.sh                     # build warnings + test results + fixtures + clippy location-set

# Run a .tri program (build the binary first)
# The binary is `triet-driver` (the old `dao` CLI was deleted).
# Scalars/structs/enums/String/Vector/HashMap/nullable/match all run;
# arithmetic outside ±(3²⁷−1)/2 traps per ADR-0044.
cargo build --release
./target/release/triet-driver examples/hello_jit.tri        # check: parse→typecheck→lower→borrowck
./target/release/triet-driver run examples/hello_jit.tri    # run:   + JIT compile + execute → 42
```

Tests must be **green before any commit**. The user's "stability over speed" principle is non-negotiable — do not bypass failing checks with `--no-verify`, `#[allow]`, or `#[ignore]`.

## Architecture

### Current pipeline (the only pipeline — the rewrite)

```
.tri source
    │
    ▼  triet-lexer        [REUSED] tokens (logos-based)
    ▼  triet-parser       [REUSED] AST (recursive descent + Pratt)
    ▼  triet-modules      [REUSED] ResolvedProgram (loader + resolver)
    ▼  triet-typecheck    [REUSED] type errors (BLOCKING — fatal on error)
    ▼  triet-lower        [NEW] AST → MIR lowering (Result, 0 panic!())
    ▼  triet-mir          [NEW] flat non-nested IR + CFG
    ▼  triet-borrowck     [NEW] NLL dataflow borrow checker
    ▼  triet-jit          [NEW] Cranelift native code (Tier A: single-i64 ABI)
    ▼  triet-driver       [NEW] pipeline binary (check / run modes)
```

`triet-pack` (`.khi` format + cross-package linker) survives from the old
compiler but is **not yet wired** into the new pipeline. `triet-ir`,
`triet-interpreter`, `triet-bootstrap`, `triet-cli` were **deleted** — do not
reference them.

The 13 live crates: `triet-core`, `triet-logic`, `triet-syntax` (foundation);
`triet-lexer`, `triet-parser`, `triet-modules`, `triet-typecheck` (reused
frontend); `triet-lower`, `triet-mir`, `triet-borrowck`, `triet-jit`,
`triet-driver` (new backend); `triet-pack` (packaging, unwired).

Foundation crates: `triet-core` (Trit/Tryte/Integer/Long arithmetic), `triet-logic` (Trilean Łukasiewicz Ł3 / Kleene K3), `triet-syntax` (AST types + arena, schema-generated types in `src/generated/`).

New backend crates:
- `triet-mir` — flat, non-nested MIR with `Body`, `Statement`, `Terminator`, `ControlFlowGraph`, `StructLayout`. Independent of AST types. Every field populated, no dead data.
- `triet-lower` — AST→MIR lowering bridge. `lower_program() -> Result<Vec<Body>, LowerError>` — **0 panic!()**. Populates `StructLayout` from `Item::Struct`. Consumes `triet-syntax` + `triet-typecheck`.
- `triet-borrowck` — NLL borrow checker with forward/backward dataflow over CFG. Liveness analysis + loan tracking + `places_conflict(conservative)` — conservative alias assumption for `&0`/`&-`. Error codes: E2420, E2440, **E2450**.
- `triet-jit` — Cranelift JIT compiler consuming MIR `Body` directly. Single-i64 ABI; flat struct `StackSlot` + sret; heap shims (`__triet_string_*`/`__triet_vector_*`/`__triet_hashmap_*` — Rust `extern "C"` in `mir_lower.rs`, with NO .c files); arithmetic range-enforced (trapnz SIGILL; shim abort SIGABRT). N7 subprocess test infra (`spawn_n7_child` with `--exact --test-threads=1` — NEVER spawn without a filter; the fork bomb went off once already). Outcome ops are still guarded `Err` (no producer yet).
- `triet-driver` — pipeline binary. `check` mode: parse→typecheck→lower→borrowck. `run` mode: +JIT compile+execute. Handles `Result` from all phases, exits with diagnostic on error.

> ⚠️ DEBT REGISTRY (updated 2026-06-08): the live debt list is in **TODO.md** (the single source).
> Closed since the old 2026-06-04 note: the MIR verifier EXISTS (INV-1/INV-2 + enum
> invariants — but F6: it does not yet catch a block missing its terminator); Outcome ops are
> guarded `Err` (no more identity copy); D1/D1-literal/D3 were closed by ADR-0044. Still open:
> D2 (reject-MIN = defence), the F6 verifier, typecheck unreachable-arm, Tryte range
> not yet enforced, `execute_main` ignores `main` parameters, heap-nullable producer.

**Historical phase summary** — describes the DELETED v0.2-v0.10 compiler.
Kept for ADR/intent context only; the crates and architecture below **no longer
exist** (deep dive in [`docs/ARCHIVE.md`](../docs/ARCHIVE.md)):

- **Arena-based AST** — `triet-syntax` allocates `Expr`/`Stmt`/`Pattern`/`TypeExpr` in typed sub-arenas. Nodes hold `*Id` handles, not `Box<T>`. Traverse via `arena.expression(id)`; **never fabricate IDs**.
- **v0.2.x Module system** (ADR-0005 locked; import syntax superseded by ADR-0071 — `use std::io::{a, b as c}` with `::` paths, replacing `from std.io import …`) — multi-arena `ResolvedProgram`, stdlib loaded from the filesystem. **Locked rules**: single-file = crate root; inline ≡ file-bound for path resolution.
- **v0.3 IR + Bytecode VM** (ADR-0007/0008/0010) — register-SSA IR 53 opcodes, `.triv` wire format **v5**, `BrTrilean` 3-way branch + Ł3-aware `Eq`/`Ne`. `Constant::Null` = Trit::Zero discriminator. The VM was the **dev tier** per VISION §4.3. Strict `if cond` Unknown handling: compile-time E1033 (primary) + BrTrilean unknown_block (defense-in-depth post-ADR-0021).
- **v0.4 Crate-Pack** (ADR-0011/0012/0013) — `.khi` container, BLAKE3 two-level hash (`iface_hash` + `impl_hash`), cross-package linker `plan_link`, E2300-E2399 semver decision matrix. **Locked rule**: `iface_hash_pin` is the final arbiter; an auto-shim is NOT promised.
- **v0.5 CAS Packaging** (ADR-0014/0015) — a 3-level hash tree (term + module + package) with 16-byte domain separators, `~/.triet/store/`, atomic install (tmp + rename), mark-sweep GC, `dao.lock` hand-rolled line format. `abi_version` v=1 explicitly refused (no shim).
- **v0.6 Capability System** (ADR-0016/0017/0018) — a namespace attribute in `dao.package` (Grant/Ambient/Deny/Defer 4-state), `dao.policy` resolution rules, `/dev/tty` provenance prompt (POSIX), E22XX. **Locked rule**: the root package's manifest is the sole decision-maker, with no path inheritance.
- **v0.7 Self-hosting Compiler** (ADR-0019/0020/0021/0024) — `compiler/` Triết-in-Triết ~23K LOC mirroring crate boundaries; 3-stage bootstrap chain (Stage 1 Rust → 2 → 3 byte-identical gate `#[ignore]`'d, lifts v0.9). Outcome `T~E`/`T?~E` + Trilean! refinement baked into typecheck/lowerer. `khi`/`dao` identity.
- **v0.8 Ownership + BYOS** (ADR-0022/0025/0026 v2/0027) — S6 5-form reference `&+`/`&0`/`&-`/`&` + `owned`, `ObjectHeader` 8-byte refcount header, Send derivation for 13 type categories, capability schema extended with concurrency caps. **Locked rule (BYOS)**: `actor`/`spawn`/`receive`/`send`/`async`/`await` **NOT keywords** — refuse-list ADR-0026 v2 §6. E24XX/E25XX skeleton emitted, full enforcement defer v0.9.

### Error code namespace

- `triet::lex::E0000` — lexer
- `triet::parse::E000X` — parser
- `triet::typecheck::E10XX` — type checker (E1024-E1032 + E1037-E1039 ADR-0020 Outcome; E1033/E1034 ADR-0021 Trilean!; **E1035** NegativeArmOnNullable ADR-0041 §12; **E1036** IntegerLiteralOverflow ADR-0044 Q2; **E1058** EqualityUnsupported — `==`/`!=` refused fail-closed on Struct/Vector/HashMap/payload-carrying enum/`Nullable<String>` per `WO-String-Eq-Content-Compare-And-Aggregate-Refuse`, ADR-0038 §4 defers a general `compare()`; payload-free enum and bare `String` (content-compare) stay allowed)
- `triet::lower::E11XX` — lowering (AST→MIR), 8-code taxonomy per ADR-0086: **E1100** ConstructNotYetLowered / **E1120** NullableEnumPayloadUnsupported / **E1121** NullableStructReturnHeapField / **E1122** EscapingClosureSealed (design fences) / **E1140** UndefinedLocal / **E1141** NullLiteralWithoutExpectedType / **E1142** LiteralOutOfRange (user errors) / **E1190** InternalInvariant (ICE — compiler bug, not user error)
- `triet::runtime::E20XX` — interpreter (DELETED crate; codes reserved, no live emitter)
- `triet::modules::E21XX` — loader / resolver (E2100 cyclic, E2101 file-not-found, …)
- `triet::capability::E22XX` — capability system (E2200-E2208)
- `triet::pack::E23XX` — semver linker (v0.4)
- `triet::borrow::E24XX` — borrow checker (E2400 lifetime / E2410 mutability / E2420 move / E2430 namespace / E2440 NLL / E2450 DropWhileBorrowed) per ADR-0025. E2450 implemented 2026-06-04.
- `triet::actor::E25XX` — actor/concurrency (E2500 Send / E2510 scope-ref / E2520 mutable-share / E2530+ reply/supervision) per ADR-0026

All errors implement `miette::Diagnostic`. (The old `triet-cli` `--json` mapper layer was deleted with the CLI; `triet-driver` prints miette reports directly and has no JSON mode yet. If/when JSON output returns, the error-code mapper discipline applies again.)

**Diagnostic format:** all error/warning text follows the canonical machine-fixable format locked in [ADR-0027](../docs/decisions/0027-diagnostic-format-standard.md) — header `EXXXX ErrorName` + body + optional span block + optional `[Fix N]` numbered fix blocks with the imperative `Change/Wrap/Use/Add/Replace/Move X to Y`. Pure ASCII, no diff `-/+`.

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
| `use std::io::println`, `use std::io::{a, b as c}` | `from std.io import println` (ADR-0005, superseded) | ADR-0071 (`use` + `::` import path) |
| `!a`, `a && b`, `a \|\| b`, `a ^ b`, `a => b` | — | SPEC §4.2 (symbolic preferred) |
| `a ~> b`, `a ~^ b`, `a <=> b`, `a <~> b` | — | SPEC §4.2 (Kleene variants) |
| `1_trit`, `0_trit`, `-1_trit` (suffix-typed Trit literal) | `0t+` as Trit (those `0t...` forms are balanced-ternary **Integer** literals, not Trit) | SPEC §1.5.1 |
| `&+ T`, `&+ mutable T`, `&0 T`, `&0 mutable T`, `&- T` (5 reference forms — lexer longest-match disambiguates `&` from `&&` logical-AND) | bare `&T` (no such form — 5 forms exhaustive per SPEC §10.1) | SPEC §10 + ADR-0022 §2 |
| `unknown` (third Trilean value) | `null` for Trilean | SPEC §1.5.2 |
| `~0` (canonical Trit::Zero literal for `T?` / `T?~E`) | `null` (deprecated v0.7.4.3-error, W2001 → E2002 v1.0) | SPEC §1.5.3 + ADR-0020 §10 |

Reserved namespace roots (cannot be user identifiers): `std`, `sys`, `dev`, `usr`, `core`, `crate`, `self`, `super`.

`Trilean` defaults to **Łukasiewicz Ł3** semantics (not Kleene). Don't substitute Boolean reasoning when working on logic ops. Per ADR-0021, the typecheck distinguishes generic `Trilean` (might be Unknown) from refinement `Trilean!` (statically proven ≠ Unknown). Plain `if cond` requires `Trilean!`; `Trilean` raises E1033. Literals `true`/`false` are `Trilean!`; `unknown` is `Trilean`. Non-nullable primitive comparisons (`Integer == Integer`, etc.) produce `Trilean!`. Łukasiewicz/Kleene ops preserve refinement when both operands are `Trilean!`.

**Logic operators:** Both symbolic (`!`, `&&`, `||`, `^`, `=>`, `~>`, `~^`, `<=>`, `<~>`) and keyword (`not`, `and`, `or`, `xor`, `implies`, `kleene_implies`, `kleene_xor`, `iff`, `kleene_iff`) forms are valid. Symbolic form is preferred per user convention. The `~` prefix consistently marks Kleene K3 variants.

**Outcome operators (v0.7.4.3-error, design locked per [ADR-0020](../docs/decisions/0020-outcome-error-handling.md)):** Constructors (prefix): `~+ value` (Trit::Positive success arm), `~0` (Trit::Zero null arm — `T?` / `T?~E` only), `~- error` (Trit::Negative failure arm). **Ternary map family (postfix, 3-char compound tokens — one per trit arm, symmetric with constructors):** `expr ~+> |v| body` (success-arm map), `expr ~0> body` (null-arm map, `T?~E` only), `expr ~-> |e| body` (error-arm map / propagate). Each operator runs in **2 modes** distinguished by whether `body` uses `return`: MAP mode (no `return`, body auto-wrapped per §3.0 — `~+>`→`~+`, `~0>`→`~+`, `~->`→`~-`) vs EARLY-RETURN mode (`return` present = propagate to caller, the Rust-`?` analog, e.g. `parse(s) ~-> |e| return ~- e`). Propagate requires the enclosing fn to return `T~E`/`T?~E` (E1028). **Deprecated `~?` / `~:`** (early v0.7.4.3 draft postfix forms) — **lexer refuses** them; full migration to the ternary family per ADR-0020 §3 (2026-05-26: brand-clean, trit-symmetric, non-redundant). Force-unwrap NOT available as operator — use verbose methods `.unwrap_value(message)` / `.unwrap_error(message)` per `feedback_explicit_strictness.md` (`!!` force-unwrap exists for nullable `T?` only). Type syntax: `T~E` (2-state binary outcome), `T?~E` (3-state with null) with `?~` as **lexer compound token** at type position (no whitespace within).

## Workspace conventions

- Rust 2024 edition, stable channel (`rust-toolchain.toml`).
- Workspace lints are strict: `unsafe_code = forbid`, `missing_docs = warn`, clippy `pedantic` + `nursery` at `warn`. Internal crates have `#![allow(clippy::redundant_pub_crate)]` at `lib.rs` to balance with `unreachable_pub`.
- All public items need a doc comment (rustdoc-rendered).
- Miette diagnostics: every error variant gets `#[diagnostic(code(triet::<area>::E<code>))]` plus a `#[label]`-bearing `Span`.

## Schema-first discipline

**`spec/schema/triet-schema.yaml` is the single source of truth for AST node
shapes, operators, and S6 ownership semantics.** The generated `ReferenceForm`,
`Visibility`, and all `Expr`/`Stmt`/`Item` AST types are wired into the compiler.

**⚠️ The type system is NOT yet schema-driven (2026-06-04).** The generated
`Type` enum is **spec-only** — the typechecker uses a hand-written `Type` in
`triet-typecheck/src/types.rs`. The schema's `Type` definition is the target
specification; the hand-written typecheck `Type` has diverged from it (different
variant sets, different semantics). Reconcile is a **future phase** — this is a
conscious deferral, not an oversight. See `spec/plans/phase1-schema-s6-model.md`.

Rules (from `spec/schema/README.md`):
1. **Schema first, code after** — for AST nodes and ownership types. For the
   type system itself: schema documents the target; hand-written typecheck Type
   is the current reality. Don't add variants to hand-written Type without
   checking if the schema already defines them.
2. **Generated code is never hand-edited.** If the generated code has issues,
   fix the codegen (`spec/schema/codegen.py`), not the output.
3. **Schema IS documentation.** Every description in the schema must be
   complete enough for a newcomer to understand the semantics.
4. **Ownership annotation on every field.** Every field with a composite type
   must declare `owned`, `borrow`, or `move`.

```bash
# Regenerate Rust sources after schema changes
python3 spec/schema/codegen.py --target rust --schema spec/schema/triet-schema.yaml

# Validate schema consistency
python3 spec/schema/codegen.py --validate spec/schema/triet-schema.yaml
```

If you find yourself wanting to add a field to `Type` or a variant to `Expr`
directly in Rust — STOP. That's the old way. Edit the schema, run codegen,
then update the consumers (parser, typecheck, lowerer).

## Development cadence

The user follows a per-step commit pattern:
1. Pick the next sub-task from `TODO.md`.
2. Implement, run **`bash scripts/gate.sh`** (build + test + fixtures + clippy
   location-set). A report or review means pasting the script's RAW OUTPUT — never
   hand-copied numbers (convention since 2026-06-07, after 3 unmeasured claims).
3. Commit with conventional format: `<type>(<scope>): subject` — current scope: `feat(track-c): …` / `fix(track-c): …` / `docs(adr): …` (previously `track-b`). Examples in `git log`.
4. Push.
5. Update `TODO.md` to mark `[x]` and append the commit short-hash.
6. The `.githooks/post-commit` hook auto-rebuilds the knowledge graph (graphify-out/) in the background after the commit (AST-only, no API cost) — no manual step needed in the normal case. See the **graphify** section below for the freshness check + manual-fallback conditions.

Do not commit, push, or run `gh` commands without an explicit ask. The user reviews each step. Only the user runs `cargo run` against examples in interactive sessions — don't auto-run.

When a decision affects future architecture (module shape, ABI, type system), write an ADR in `docs/decisions/000N-<topic>.md` instead of "ship and fix later".

## Examples

`examples/*.tri` is a MIX of survivors from the deleted VM-era compiler and new
driver smoke tests. **String/Vector/HashMap/Enum/Struct/nullable now run on
`triet-driver`** (Tier B) — but the old examples have not been re-validated in
bulk: they may use builtins or idioms from the old VM that were never wired up
(e.g. `concat`, f-string runtime). The real test net is
`crates/triet-driver/tests/fixtures/` (72 fixtures); examples are only demos. Do not
treat a failed old example as a regression — but if one fails, record it for pruning
or re-validation.

Known-good on the current driver:
```bash
./target/release/triet-driver run examples/hello_jit.tri        # → 42
./target/release/triet-driver run examples/test_pow.tri         # → 1024
./target/release/triet-driver run examples/test_pow_complex.tri # → 1267
./target/release/triet-driver examples/test_borrow.tri          # → E2440 borrow error (miette)
```

Old VM-era examples (`fizzbuzz`, `factorial`, `measles_risk`, `nullable`,
`generic`, `atomic_counter/`, …) and the `demos/` dirs were written for the
deleted interpreter/VM and the byte-identical interpreter-vs-VM differential
harness. They are **stale fixtures** until aggregate lowering is rebuilt —
either re-validate or prune them when that work lands.

> The "Post-v0.5 … Post-v0.8 audit" notes that used to live here documented the
> DELETED compiler's audit history. They are preserved in git history and in
> `docs/`; they no longer describe live code, so they were removed from this file
> to stop misleading fresh sessions.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

**At the start of every new thread/session** (when graphify-out/graph.json exists), orient on the architecture via graphify *before* doing broad source exploration:
1. Skim graphify-out/GRAPH_REPORT.md "Community Hubs" to map the crate/subsystem layout, then `graphify query "<your task area>"` to pull the scoped subgraph for what you're about to touch.
2. **Check freshness:** GRAPH_REPORT.md records `Built from commit:` — compare to `git rev-parse HEAD`. If it lags HEAD, run `graphify update .` first so the graph reflects current code.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<node-name>"` for focused concepts (note: `explain` takes a **node name**, not a community label). These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- **Auto-update is wired:** a `.githooks/post-commit` hook (installed by `graphify hook install`, with `core.hooksPath=.githooks`) rebuilds the graph in the background after every commit — code-only, no API cost — so the graph normally tracks committed code on its own. Verify with the `Built from commit:` freshness check above; only run `graphify update .` manually if you changed code *without* committing, or the background rebuild failed (log: `~/.cache/graphify-rebuild.log`).
- graphify-out/ is generated output (gitignored) — never hand-edit it; regenerate with `graphify update .` if absent or stale.
