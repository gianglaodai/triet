# TODO (Track B Rewrite)

Sub-task tracking for the current phase (Phase 4 & 5).

## Phase 4 — Aggregate Type Lowering
- [ ] Struct literal lowering (using Cranelift StackSlot infrastructure).
- [ ] Enum literal lowering.
- [ ] String, Vector, HashMap literal lowering.
- [ ] ReturnShape::Struct for multi-field returns in MIR.
- [ ] Shim registry for Track B aggregates (`__triet_alloc_struct`, `__triet_set_field`, etc. if fallback is needed, though StackSlot is preferred).

## Phase 5 — Bậc C (Native Layout)
- [ ] Native struct layout (StackSlot with MIR StructLayout sizes).
- [ ] Packed Outcome ABI (bit extraction for discrim/payload).
- [ ] Multi-value return (>1 return value).

## Integration Test Corpus
- [x] Basic test harness (`cargo test -p triet-driver`).
- [x] `while` loop hang fixed.
- [x] Trilean logic ops fixed in typechecker/JIT.
- [ ] Expand corpus as new aggregate types are implemented.

## Tech Debt / Cleanup
- [x] Deleted orphaned `compiler/` directory (Track A legacy).
- [ ] Schema unification: fully migrate generated `Type` into typechecker.
- [ ] Alias analysis: replace `conservative=true` band-aid with proper NLL alias analysis.
- [ ] Version bump: `Cargo.toml` 0.10.0 → 0.11.0-dev or 1.0.0-dev.
