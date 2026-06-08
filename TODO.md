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
- [x] String literal lowering (Phase 4.3a).
  - *Shims: alloc, from_bytes, free, concat, eq, len — implemented and registered.*
  - *M1-M4: Assign zero, let-Move-type→Assign, CallDispatch consume zero, Return-escape.*
  - *B7/B8: heap types refused at user-fn boundary and aggregate payload/field.*
  - *Deferred: `concat`/`eq` as surface builtin functions — lowerer dispatch code exists (lib.rs:1030-1065), blocked on typechecker prelude signatures. `len` was wired in 4.3b via overload resolution.*
- [x] Vector support (Phase 4.3b).
- [x] Nullable (`T?`) representation Bậc A — ADR-0041 locked (PA-3c uniform MIN). Móng: NULL_SENTINEL + is_nullable_type + is_copy + canary N1/N2. Xây: widening + ~0 + Elvis + get + fixtures 40-46.
- [x] Match `~+/~0` 2-arm cho nullable (Bậc B lát a). `b7d1f98` `279e7f2`. Exhaustiveness E1026, `~-` rejection E1035. Lowering: branch-based sentinel compare (mẫu Elvis) + 3 guard (duplicate arm, wildcard-last, sub-pattern Variable/Wildcard) → slot-model ≡ first-match-wins. Fixtures 48-57 (10 fixtures).
  - *Debt F6 (Mentor O): non-exhaustive match trôi qua MIR verifier+JIT — verifier không bắt block không terminator. Khi gỡ typecheck guard, lowerer emit null_bb rỗng, JIT compile+run im lặng trả 0. Cùng họ Outcome-guard debt: lowerer dựa hoàn toàn vào typecheck. Cần probe riêng xem INV-1 vì sao nuốt block không terminator.*
- [x] B7-lift — ownership-across-boundary (Bậc B lát c). `d36244a` `d9b5cf4` `a58693e` `0f9b1d8` `86b7039`. ADR-0042 ĐÓNG TRỌN (O+G 2026-06-07). Move-only scope. Deinit tombstone + borrowck M3+ CallTarget::Jit check-then-mark + caller zeroing. Fixtures 58-65 (8 fixtures). Acceptance C1-C8 verified.
- [x] HashMap support (Bậc B lát b). `2b72c62` `3951821` `d2e3043` `a08916d` `ed71185` `07da95f` `247a3be`. ADR-0043 ĐÓNG TRỌN (O+G 2026-06-07). 5 shim (alloc/insert/get/len/free), insert-or-update, rehash, D2 reject-MIN, cap.max(4) invariant. 5/5 teeth đỏ (reject-MIN, free-guard, rehash-displaced, insert-update, arg_consumes). Fixtures 66-73 (8 fixtures) + C9 unit test.
- [x] ReturnShape::Struct for multi-field returns in MIR.
- [x] MIR verifier: structural invariants cho enum (4i-1 đến 4i-7).
- [ ] Shim registry for Track B aggregates (`__triet_alloc_struct`, `__triet_set_field`, etc. if fallback is needed, though StackSlot is preferred).

## Phase 5 — Bậc C
- [x] **ADR-0044 trap-on-overflow:** `1fbf6ab`. JIT range check (Add/Sub/Mul trapnz SIGILL), E1036 literal overflow, pow checked_mul+range. D1/D1-literal/D3 ĐÓNG. D2 giữ defense-in-depth. 8 N7 subprocess tests, 4/4 teeth đỏ. `scripts/gate.sh`.
- [x] **ADR-0045 Borrow Params Heap — O+G ký 2026-06-08:** `1cd7635`. Scope `&0 T` shared read-only. Móng: xóa type-erasure `"?"`. Return-borrow CẮT (E1042). PropagatedLoan giữ + TODO. 7-bước implementation.
- [x] **B1 (§3) Type thật cho reference:** `type_name` render `&0 String`, không `"?"`. `is_copy` prefix-match → `true`.
- [x] **B2 (§2 callee) Lower không push_owned cho borrow param:** `push_owned` chỉ Move + non-ref. Callee MIR không `Drop(_0)`.
- [x] **B3 (§2 caller) Caller không zero borrow arg:** `to_zero` skip arg có type `starts_with('&')`. Checker M3+ skip.
- [x] **B4 (§4 driver) Wire check_body_with:** driver collect sigs → `check_body_with`. Mắt xích (b) của F4.
- [x] **B5 (§5) Typecheck refuse `-> &0 T`:** E1042 `BorrowReturnNotYetSupported`. Đóng accepted-wrong.
- [x] **B6 (§8) Mở read-op `length`:** wire `length` alias `len`, strip `&0 ` prefix. `length(&0 s)` RUN.
- [x] **B7 (§4) TODO + giữ engine:** TODO tại checker.rs:754, lower/lib.rs:168.
- [x] **ADR-0046 Return-borrow Elision — O+G ký 2026-06-08:** `cfae64d` `034ba0d` `d6e3da0`. Mở `-> &0 T` return type qua 3 bước: §1 E1042 form-gate whitelist BorrowReadOnly, §2 reuse E2400 elision, §3 lower populate return_borrow_map (đếm theo type-string &, count≠1→Err). Blocker fixes: mixed-param false Err (đếm theo type, không ParameterPassing) + E2450 false-positive (is_propagated skip + dest-loan removal). Positive fixture 84 RUN ra 5. TECH-DEBT: is_propagated skip dựa trên giả định không nested block scope.
- [x] **ADR-0047 Read-ops Extension — O+G ký 2026-06-08:** `3259631` `5071be1` `a92e415` `6052509` `3012af8`. `contains` (3 shim String/Vector/HashMap, trả 1/-1 Trilean! encoding) + `is_empty` (derive len==0 qua BinOp::Eq). 8 fixture positive RUN. Slice TÁCH (ref-view vi phạm ADR-0046 Q3, copy-view feature riêng). Clippy cleanup 204→200.
- [x] **ADR-0048 Mutable Borrow — O+G ký 2026-06-08:** `7390012` `d556f2a` `bdaa5e3`. `clear(&0 mutable String)` set len=0 in-place (no realloc — append CẮT vì realloc mìn → Bậc D). E2440 exclusivity REUSE (hai tầng typecheck+MIR). Return-mut `-> &0 mutable T` CẮT (E1042). 3 fixtures: 93 clear RUN, 94/95 exclusivity E2440. TECH-DEBT: hai tầng borrowck song song (typecheck ADR-0025 + MIR) — hợp nhất sau.
- [ ] **Codegen opt (G, ADR-0044 ack §iii):** range check 1-instruction — `(val−MIN) >ᵤ 2M` unsigned-sub trick; fallback `bor` gộp 2 icmp trước trapnz. Cắt nửa instruction check mỗi Add/Sub.
- [ ] **Constant folding pass (G, ADR-0044 ack §iii):** toán hạng const in-range → tính compile-time, bỏ trap block.
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
- [ ] codegen.py emit clippy-clean output — codegen bug
- [ ] Alias analysis: replace `conservative=true` band-aid with proper NLL alias analysis.
- [ ] Version bump: `Cargo.toml` 0.10.0 → 0.11.0-dev or 1.0.0-dev.
- [ ] Fix fixture 27: match error-code thay vì match internal JIT string (brittle, rò rỉ representation).
- [ ] Enum exhaustiveness checker (currently non-exhaustive match = runtime Trap).
- [ ] Pattern::Wildcard support trong enum match (Bậc A hiện chỉ handle EnumVariant + Variable patterns).
- [x] **D1 (ADR-0041 §6.2):** Arithmetic fidelity — JIT trap-on-overflow (ADR-0044). `1fbf6ab`. ĐÓNG.
- [x] **D1-literal (họ D1):** Typecheck E1036 range-check Integer/Ternary literal (ADR-0044 Q2). ĐÓNG.
- [ ] **D2 (ADR-0043 Q6):** `HashMap::insert` reject-on-insert giá trị `i64::MIN` (defense-in-depth, ADR-0044 Q4). GIỮ — gỡ khi arithmetic wrap mod-3²⁷ (cùng điều kiện D1).
- [x] **D3 (họ D1):** Shim MIN-input — MIN không còn reachable từ arithmetic. ĐÓNG.
