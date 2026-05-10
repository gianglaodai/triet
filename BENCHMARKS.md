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

**Status: Gate not yet met.** VM is ~1.26× faster on factorial but the 3× target
requires further optimization. Expected improvement areas:

1. **Instruction dispatch**: Currently a large `match` over all 46 opcodes.
   Can be replaced with computed goto / threaded code.
2. **Value representation**: `RuntimeValue` is a full enum with heap-allocated
   strings. Can use NaN-boxing or tagged pointers.
3. **Frame management**: `HashMap<ValueId, RuntimeValue>` for registers;
   can use `Vec<RuntimeValue>` indexed by ValueId.0.
4. **Builtin dispatch**: String-based builtin lookup; can use function pointers.

These optimizations are deferred to v0.4 (performance pass) per ROADMAP.
v0.3 focuses on IR design validation, not production throughput.

## Historical data

| Version | Date | Interpreter | VM | Speedup | Notes |
|---|---|---|---|---|---|
| v0.3.11 | 2026-05-11 | 79.6 µs | 63.2 µs | 1.26× | Baseline |
