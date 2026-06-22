# Triết

> A **balanced-ternary-first** programming language, inspired by the Soviet
> Setun computer (1958), implemented in Rust.

Triết (Sino-Vietnamese 哲, "philosophy") uses the balanced ternary system
`{-1, 0, +1}` as its arithmetic foundation, combined with three-valued
Łukasiewicz Ł3 logic for reasoning under uncertainty. Its value is anchored in
**coherence** — a single Ł3 algebra across null / logic / capability — not in any
AI hypothesis (the "AI-first" label was removed 2026-06-22; see [VISION.md](VISION.md) §5).
The long-term aim keeps the door open to an OS-capable language.

## Status — ground-up rewrite in progress (v0.1.0-dev)

> **Read this honestly.** A full compiler shipped v0.2–v0.10 (bytecode VM +
> tree-walking interpreter + a delegate-to-VM Cranelift JIT + a self-hosting
> compiler, ~1637 tests). On **2026-06-04 that backend was deleted** and the
> project restarted from the backend up with a clean architecture. The current
> compiler is **young**: do not mistake it for the shipped v0.10.

**What works today** (end-to-end, `source → MIR → Cranelift → native code`):
- **Scalars** — balanced-ternary arithmetic (range-enforced, trap-on-overflow per
  ADR-0044), comparisons, Łukasiewicz Ł3 / Kleene K3 logic.
- **Control flow** — `if`/`else`, `while`, recursion, cross-function calls, shim
  calls (`pow`).
- **Aggregates** — `struct` (StackSlot + sret), `enum` (discriminant switch), and
  **heap types** `String` / `Vector<T>` / `HashMap<K,V>` (move-only, inline
  drop-glue).
- **Nullable `T?`** — 1-trit sentinel, Elvis `?:`, `match ~+ / ~0`, including
  **nullable aggregates** `Struct?` / `Enum?` (ADR-0065).
- **`match` on literals** — Integer / Trilean / Trit / Tryte / Long,
  exhaustiveness-checked (ADR-0064); trait static dispatch (Tier 1, ADR-0061);
  `Outcome` `T~E` / `T?~E` error handling.
- **Heap-in-struct (FLAT, ADR-0066 Lát 1)** — a struct holding a
  `String`/`Vector`/`HashMap` field: construct, move (function boundary +
  assignment), recursive-walk drop-glue, tombstone-on-move.
- **NLL borrow checker** — E2420 (use-after-move) / E2440 / E2450
  (drop-while-borrowed).

**Not yet rebuilt:** nested/recursive heap-in-aggregate (`Struct { inner: HasHeap }`)
and enum-payload heap (ADR-0066 Lát 2), partial field-move (`let s = p.name`), the
capability runtime (ADR-0016/0017/0018 — strategic priority after the heap-in-struct
campaign), the self-hosting compiler, the AOT cache, `triet-pack` wiring. The
**language semantics are unchanged** — the rewrite swaps compiler internals, not the
language (see the ADRs).

```bash
cargo build --release

# The driver binary is `triet-driver` (the old `dao` CLI was deleted).
# Scalars, structs, enums, String/Vector/HashMap, nullable T?, and match all run;
# nested/recursive heap-in-aggregate is the next frontier (ADR-0066 Lát 2).
./target/release/triet-driver run examples/hello_jit.tri        # → 42
./target/release/triet-driver run examples/test_pow.tri         # → 1024
./target/release/triet-driver run examples/test_pow_complex.tri # → 1267
./target/release/triet-driver examples/test_borrow.tri          # → E2440 borrow error
```

## Pipeline

```
.tri source
  ├─ triet-lexer        tokens (logos-based)            [reused]
  ├─ triet-parser       AST (recursive descent + Pratt) [reused]
  ├─ triet-modules      name resolution                 [reused]
  ├─ triet-typecheck    type errors (blocking)          [reused]
  ├─ triet-lower        AST → MIR (Result, no panics)   [new]
  ├─ triet-mir          flat non-nested IR + CFG + verifier [new]
  ├─ triet-borrowck     NLL dataflow borrow checker     [new]
  ├─ triet-jit          Cranelift native code (Bậc A: single-i64 ABI) [new]
  └─ triet-driver       pipeline binary (check / run)   [new]
```

## Design philosophy

1. **Regular & low-ambiguity** — syntax and semantics tuned for correctness and
   minimal ambiguity (explicit > implicit, regular > exception, keyword over
   symbol when ambiguous). Any benefit to LLM codegen is an unmeasured
   side-effect, never a claim (see [VISION.md](VISION.md) §5).
2. **Ternary is first-class** — `Trit`, balanced-ternary arithmetic, and
   Łukasiewicz logic are primitive types and operators, not library add-ons.
3. **Stability over speed** — every architectural decision has an ADR; phases
   close on explicit gates (5–10 year horizon).
4. **IR ≠ runtime** — the Triết IR is a spec; the backend (JIT/AOT) is an
   implementation detail.

## Language examples

The language itself is unchanged from the shipped spec. (Some of these use
aggregate types the current driver does not yet lower — they illustrate
*syntax*, not what runs today.)

```triet
// Reasoning with missing data — the power of Łukasiewicz Ł3
function risk_measles(fever: Trilean, rash: Trilean, vaccinated: Trilean) -> Trilean {
    let symptoms = fever && rash
    symptoms && !vaccinated
    // If vaccinated is unknown → the result is unknown ("not enough information")
}

// Module system — Python-style imports, verbose keywords
from std.io import println
from crate.gates import nand_gate, xor_gate

public function half_adder(a: Trit, b: Trit) -> (Trit, Trit) =
    (xor_gate(a, b), nand_gate(a, b))
```

## Workspace

13 crates:

```
triet/
├── crates/
│   ├── triet-core/        # Trit/Tryte/Integer/Long + arithmetic   [foundation]
│   ├── triet-logic/       # Trilean + Łukasiewicz Ł3 + Kleene K3    [foundation]
│   ├── triet-syntax/      # AST types + arena + schema-generated    [foundation]
│   ├── triet-lexer/       # Tokenizer (logos-based)                 [reused frontend]
│   ├── triet-parser/      # Parser → AST                            [reused frontend]
│   ├── triet-modules/     # Module loader + name resolver           [reused frontend]
│   ├── triet-typecheck/   # Type checker + inference                [reused frontend]
│   ├── triet-mir/         # Flat MIR + CFG + verifier               [new backend]
│   ├── triet-lower/       # AST → MIR lowering                      [new backend]
│   ├── triet-borrowck/    # NLL dataflow borrow checker             [new backend]
│   ├── triet-jit/         # Cranelift native codegen                [new backend]
│   ├── triet-driver/      # Pipeline binary                         [new backend]
│   └── triet-pack/        # .khi packaging + linker                 [survives, unwired]
├── examples/              # .tri programs (mix of new + stale VM-era fixtures)
├── spec/                  # design authority: schema + phase plans
│   ├── schema/triet-schema.yaml   # single source of truth for types/AST/ownership
│   └── plans/                     # phase designs (rewrite Bậc A/B/C)
├── docs/
│   ├── decisions/         # 36 ADRs (language-semantics ones remain authoritative)
│   └── ARCHIVE.md         # digest of the deleted v0.2–v0.10 compiler + ADR catalog
├── SPEC.md                # language semantics (authoritative for the language)
├── VISION.md              # 5 architectural pillars + OS-capable trajectory
└── ROADMAP.md             # phase roadmap (being reconciled to the rewrite)
```

> `triet-ir`, `triet-interpreter`, `triet-bootstrap`, `triet-cli`, and the
> `compiler/` self-host sources were **deleted** in the rewrite. Don't expect them.

## Documentation

- [`SPEC.md`](SPEC.md) — language semantics (authoritative for the language).
- [`VISION.md`](VISION.md) — long-term vision: 5 architectural pillars, OS-capable goal.
- [`spec/`](spec/) — the rewrite's design authority (schema + phase plans).
- [`docs/decisions/`](docs/decisions/) — ADRs; the language-semantics ones are still binding.
- [`docs/ARCHIVE.md`](docs/ARCHIVE.md) — history of the deleted v0.2–v0.10 compiler.

## Build

```bash
cargo build                              # debug
cargo build --release                    # release
cargo test --workspace                   # all tests
cargo clippy --workspace --all-targets   # lint (strict)
cargo fmt --all                          # format
```

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
