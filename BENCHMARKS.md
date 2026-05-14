# Benchmarks

VM performance vs tree-walking interpreter. Measured via [criterion](https://github.com/bheisler/criterion.rs).

**Gate (ROADMAP § v0.3):** VM phải ≥3× interpreter trên 11 example programs.

## Running

```bash
cargo bench -p triet-cli --bench vm_vs_interpreter
```

Reports are written to `target/criterion/`.

## v0.3.11 baseline (2026-05-11)

Measured on AMD Ryzen, Linux x86-64, Rust 1.93. Only execution time;
load/typecheck/lower excluded.

| Example | Interpreter | VM | Speedup |
|---|---|---|---|
| `factorial.tri` | 79.6 µs | 63.2 µs | 1.26× |
| `fizzbuzz.tri` | — | — | — |
| `lukasiewicz_vs_kleene.tri` | — | — | — |
| `measles_risk.tri` | — | — | — |
| `nullable.tri` | — | — | — |
| `maybe.tri` | — | — | — |
| `generic.tri` | — | — | — |
| `long_arithmetic.tri` | — | — | — |
| `counter.tri` | — | — | — |
| `enumerate.tri` | — | — | — |
| `while_polling.tri` | — | — | — |

**Status: 3× bench gate deferred to v0.4 performance pass.** VM is ~1.26× faster
on factorial; the 3× target requires further optimization. v0.3's primary aim
is IR design validation, not production throughput — per [VISION § 4.3](VISION.md)
the bytecode VM is development tier scaffolding, not a production runtime.

Expected improvement areas (work for v0.4):

1. **Instruction dispatch**: Currently a large `match` over all 52 opcodes.
   Can be replaced with computed goto / threaded code.
2. **Value representation**: `RuntimeValue` is a full enum with heap-allocated
   strings. Can use NaN-boxing or tagged pointers.
3. **Frame management**: `HashMap<ValueId, RuntimeValue>` for registers;
   can use `Vec<RuntimeValue>` indexed by ValueId.0.
4. **Builtin dispatch**: String-based builtin lookup; can use function pointers.

Per ADR-0009 § A, this gate is the **only** v0.3 deliverable not at 100%; all
other gates (functional coverage, differential 11/11, code hygiene, doc sync)
are satisfied. The deferred bench gate is tracked here rather than gated as
a v0.4 prerequisite.

## Historical data

| Version | Date | Interpreter | VM | Speedup | Notes |
|---|---|---|---|---|---|
| v0.3.11 | 2026-05-11 | 79.6 µs | 63.2 µs | 1.26× | Baseline |
