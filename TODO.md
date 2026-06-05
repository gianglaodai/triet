# TODO (Track B Rewrite)

Sub-task tracking for the current phase (Phase 4 & 5).

## Phase 4 — Aggregate Type Lowering
- [x] Struct literal lowering (using Cranelift StackSlot infrastructure).
  - *Note: JIT hiện tại chưa hỗ trợ nested field access (e.g., `a.b.c`). Cần tính toán offset cộng dồn hoặc stack load chain.*
- [x] Enum literal lowering (unit + payload, end-to-end, 24/24 integration green).
  - *MIR: EnumAlloc, SetDiscriminant, GetDiscriminant, SwitchInt, Trap, Payload projection.*
  - *Parser: bare-variant resolution is global name-match (lowerer scans all enum_layouts). Lowerer tự resolve thay vì tiêu thụ typecheck decision = cross-layer mismatch. Diagnostic khó hiểu khi hai enum có variant trùng tên. Defer: typecheck annotate variant resolved lên AST.*
  - *Known: enum payload qua function param chưa hỗ trợ (JIT: "Payload access on non-enum local"). Cần sret-like by-pointer cho enum params → Bậc B/C.*
  - *Known: construct semantic = COPY (không MOVE) per SPEC §10.1 stack primitives. Fixture 28 pin hành vi này. MIR hiện ghi "move" trong Display nhưng borrowck không enforce (transition Moved→Ended cho phép Return). Latent Bậc B/C: payload heap sẽ cần phân biệt Copy/Move type.*
- [ ] String, Vector, HashMap literal lowering.
- [x] ReturnShape::Struct for multi-field returns in MIR.
- [x] MIR verifier: structural invariants cho enum (4i-1 đến 4i-7).
- [ ] Shim registry for Track B aggregates (`__triet_alloc_struct`, `__triet_set_field`, etc. if fallback is needed, though StackSlot is preferred).

## Phase 5 — Bậc C (Native Layout)
- [ ] Native struct layout (StackSlot with MIR StructLayout sizes).
- [ ] Packed Outcome ABI (bit extraction for discrim/payload).
- [ ] Multi-value return (>1 return value).

## Deferred — design locked, chờ tiền đề (KHÔNG build tạm)
- [ ] **Trait system** (trait decl + impl + dispatch). Author 2026-06-05: Triết chắc chắn làm Trait, không Interface. Phase riêng, chưa xếp lịch.
- [ ] **`Comparable` trait, `compare() -> Trit`** — design lock tại [ADR-0038](docs/decisions/0038-comparable-trait-deferred.md). Chờ Trait system; KHÔNG làm built-in special-case. Trit (không enum Ordering), tổng thứ tự only, unknown ở lại với operator Ł3.
- [ ] **Họ toán tử Nullable `?+>`** (map+flatMap cho `T?`, auto-flatten) + `?:` RHS = Expression + cấm `?->` (E1041) — design lock tại [ADR-0039](docs/decisions/0039-nullable-operator-family.md). Chờ nullable/Outcome lowering (Bậc B/C). SPEC §Elvis cần thêm câu "RHS là Expression" khi sync.

## Integration Test Corpus
- [x] Basic test harness (`cargo test -p triet-driver`).
- [x] `while` loop hang fixed.
- [x] Trilean logic ops fixed in typechecker/JIT.
- [x] Enum fixtures: unit match (color), payload local, payload param error, construct reuse.

## Tech Debt / Cleanup
- [x] Deleted orphaned `compiler/` directory (Track A legacy).
- [ ] Schema unification: fully migrate generated `Type` into typechecker.
- [ ] Alias analysis: replace `conservative=true` band-aid with proper NLL alias analysis.
- [ ] Version bump: `Cargo.toml` 0.10.0 → 0.11.0-dev or 1.0.0-dev.
- [ ] Fix fixture 27: match error-code thay vì match internal JIT string (brittle, rò rỉ representation).
- [ ] Enum exhaustiveness checker (currently non-exhaustive match = runtime Trap).
- [ ] Pattern::Wildcard support trong enum match (Bậc A hiện chỉ handle EnumVariant + Variable patterns).
