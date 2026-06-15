# TODO (Track B Rewrite)

Sub-task tracking for the current phase (Phase 4 & 5).

## Phase 4 — Aggregate Type Lowering
- [x] Struct literal lowering (using Cranelift StackSlot infrastructure).
  - [x] **Nested field access (ADR-0060 P2):** `f28d14d` `a82e44c`. Offset-chain + multi-word copy ĐÓNG. P1 (Sub-8B packing) vẫn khóa.
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
  - *Debt F6 (Mentor O): ✅ ĐÓNG bởi A2+A3 `d8e1ba9`. Block "rỗng" = Unreachable terminator (Body.terminator không Option, default Unreachable) → INV-4 bắt khi referenced (2 unit test: referenced-fail/unreferenced-ok). Non-exhaustive match chặn 2 lớp: A3 E1026 typecheck + A2 INV-4 MIR. Verify O 2026-06-10.*
- [x] B7-lift — ownership-across-boundary (Bậc B lát c). `d36244a` `d9b5cf4` `a58693e` `0f9b1d8` `86b7039`. ADR-0042 ĐÓNG TRỌN (O+G 2026-06-07). Move-only scope. Deinit tombstone + borrowck M3+ CallTarget::Jit check-then-mark + caller zeroing. Fixtures 58-65 (8 fixtures). Acceptance C1-C8 verified.
- [x] HashMap support (Bậc B lát b). `2b72c62` `3951821` `d2e3043` `a08916d` `ed71185` `07da95f` `247a3be`. ADR-0043 ĐÓNG TRỌN (O+G 2026-06-07). 5 shim (alloc/insert/get/len/free), insert-or-update, rehash, D2 reject-MIN, cap.max(4) invariant. 5/5 teeth đỏ (reject-MIN, free-guard, rehash-displaced, insert-update, arg_consumes). Fixtures 66-73 (8 fixtures) + C9 unit test.
- [x] ReturnShape::Struct for multi-field returns in MIR.
- [x] MIR verifier: structural invariants cho enum (4i-1 đến 4i-7).
- [x] ~~Shim registry for Track B aggregates~~ — N/A OBSOLETE. StackSlot đã thắng toàn tuyến (struct/enum/String/Vector/HashMap đều dùng StackSlot + shim chuyên biệt). Chưa từng tồn tại.

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
- [x] **ADR-0059 Mũi C (Borrow Params Heap `&+ T`) — ĐÓNG:** `bf668fd` `8be0263`. Stack-borrow (`&0`) cho Vector/HashMap (C.2) & vá nợ Generic return-bind (C.1). `&+`/`&-` phong ấn YAGNI. Chữ ký O+G đầy đủ, double-free bảo vệ đã verify (SIGABRT 134).
- [ ] **Codegen opt (G, ADR-0044 ack §iii):** range check 1-instruction — `(val−MIN) >ᵤ 2M` unsigned-sub trick; fallback `bor` gộp 2 icmp trước trapnz. Cắt nửa instruction check mỗi Add/Sub.
- [ ] **Constant folding pass (G, ADR-0044 ack §iii):** toán hạng const in-range → tính compile-time, bỏ trap block.
- [~] Native struct layout (StackSlot with MIR StructLayout sizes). → **PHONG ẤN Nhóm E (G defer 2026-06-10).** Spike O: JIT field-offset ĐÃ sạch (dùng field.offset); vấn đề thật = value-model "single i64" + MirType-byte-size CHƯA có (0 fixture Trit/Tryte trong struct). 3 điều kiện mở: fixture Trit/Tryte-in-struct + ADR byte-size mapping + value-model stack_load_8/16+extend. Xem `spec/plans/phase10-native-struct-layout.md`.
- [~] Packed Outcome ABI (bit extraction for discrim/payload). → **PHONG ẤN Nhóm E (đi kèm Native, cùng cần pack byte nhỏ).**
- [ ] Multi-value return (>1 return value).

## Deferred — design locked, chờ tiền đề (KHÔNG build tạm)
- [ ] **Trait system** (trait decl + impl + dispatch). Author 2026-06-05: Triết chắc chắn làm Trait, không Interface. Phase riêng, chưa xếp lịch.
- [ ] **`Comparable` trait, `compare() -> Trit`** — design lock tại [ADR-0038](docs/decisions/0038-comparable-trait-deferred.md). Chờ Trait system; KHÔNG làm built-in special-case. Trit (không enum Ordering), tổng thứ tự only, unknown ở lại với operator Ł3.
- [ ] **Họ toán tử Nullable `?+>`** (map+flatMap cho `T?`, auto-flatten) + `?:` RHS = Expression + cấm `?->` (E1041) — design lock tại [ADR-0039](docs/decisions/0039-nullable-operator-family.md). Chờ nullable/Outcome lowering (Bậc B/C). SPEC §Elvis cần thêm câu "RHS là Expression" khi sync.
- [x] **SPEC append(byte) range:** `__triet_string_append(byte: i64)` cũ cắt byte thấp (`i64_low_byte`). Đã chốt: REJECT — out-of-range `0..=255` → `std::process::abort()` (tinh thần ADR-0044 no-silent-truncation). 2 N7 subprocess teeth (above_255 + negative → SIGABRT). `mir_lower.rs`.

## Integration Test Corpus
- [x] Basic test harness (`cargo test -p triet-driver`).
- [x] `while` loop hang fixed.
- [x] Trilean logic ops fixed in typechecker/JIT.
- [x] Enum fixtures: unit match (color), payload local, payload param error, construct reuse.

## Tech Debt — Chiến Dịch Trả Nợ (O+G classified 2026-06-09)

**Strategy (G-approved, reversed): A1 → B1 (móng) → B2 → B3 → C/D/E. A2+A3 chèn bất kỳ lúc nào.**

### 🔴 A. BOM — sai im lặng / UB tiềm tàng (trả TRƯỚC)

- [x] **A1: `is_propagated` nested-scope (Crusade #1).** ✅ ĐÓNG `be37875`. Thay blind-skip bằng `live_out.contains(dest)` — propagated loan chỉ suppress khi dest đã chết; nested scope (Drop(source) trước use returned ref) → fire E2450. Fixtures 101 (nested-return RUN) + 102 (nested-uaf E2450). **Teeth O 2026-06-10: poison live_out→false → fixture 102 ĐỎ "pipeline succeeded" (UAF lọt).**
- [x] **A2: F6 MIR verifier INV-4.** `d8e1ba9`. Bắt block có Unreachable terminator nhưng được tham chiếu (lowerer quên gọi term()). 2 unit test: referenced-fail + unreferenced-ok.
- [x] **A3: Enum exhaustiveness checker.** `d8e1ba9`. Typechecker check_enum_exhaustiveness dùng pattern_resolutions, fire E1026 nếu thiếu variant. Fixture 103 (negative, missing Red).

### 🟡 B. NỢ-MÓNG — sai thiết kế, chặn nợ khác

- [x] **B1: Rombac Type System — bỏ MIR string-match (Crusade #3).** ✅ ĐÓNG TRỌN (O+G ký 2026-06-09). ADR-0050. 4 lát S1-S4, net −470 dòng:
  - `S1 76b53cb` MirType 14 variant song song · `S2 fe80b8c` flip field String→MirType + producer lower_type + xóa simple_is_copy · `S3 ec6d32f` thanh trừng String: TypeKind ItemSymbolTable + triệt 5 free helper + Display-bridge · `S4 9af6afd` trảm parse + 3 From-shim (acid test toàn vẹn, 0 production lén parse).
  - **Đã diệt:** stringly-typed MIR (`ty:String` + ngữ pháp nhúng), ordering-rule ngầm (→kết cấu Nullable), simple_is_copy (bản sao logic), 2 HashSet (→TypeKind map). Gate 0·0·99·203.
  - **14 vòng O chặn** (verify-don't-trust): bom lớn nhất = producer-ngụy-trang (đẻ String rồi parse ngược) → luật verify-producer-trước-consumer + poison-phải-đỏ. Acid test S4: xóa parse → workspace không nổ = migrate thật.
  - **Nợ mang sang:** móng Struct/Enum no-consumer (khép khi C1 enum-payload) · lower_function 9-param → LoweringInput struct (defer, allow justify).
- [x] **B2: Sáp nhập 2 tầng borrowck typecheck+MIR (Crusade #2).** ✅ GỠ TRÙNG HOÀN TẤT (O+G ký 2026-06-10). ADR-0051. 3 lát + cleanup:
  - `1e6c14e B2.1a` gỡ E2420 subsystem (MoveState machine + 6 branch-join call-site + 2 caller) · `58dfa4e B2.1b` nổ borrow_check.rs E2440 (502 dòng module + variant) · `HEAD B2 cleanup` tử hình E2410+E2430 dead variant.
  - E2420+E2440 teeth-isolate được — MIR NLL là cảnh sát duy nhất. B2.2 (E2400/E2410) hủy do dead variant. Gate 0·0·101·203.
- [x] **B3: Alias analysis thật thay `conservative=true`.** → **DEFER (YAGNI).** B3.0 spike `11d11cf`: 0 fixture over-reject thật. `conservative=true` giữ defense-in-depth. Mở lại khi có fixture // ERROR: E2440 bị từ chối do khác-allocation-thật. Xem `spec/plans/phase8-b3-alias-analysis.md`.

### 🟡 C. FEATURE GAP — thiếu, không sai

- [x] **C1: Enum payload qua function param.** ✅ `0fb8de6` (O+G 2026-06-10). by-pointer mẫu struct Bậc D; caller stack_addr + callee tái dựng enum_slot (disc@0/payload@8). `match MirType::Enum(name)` = active consumer đầu móng B1a (khép nợ no-consumer). Fixture 27 rename→positive EXPECT 52. Teeth disc-offset SIGILL + payload-copy FAILED.
- [x] **C2: Pattern::Wildcard trong enum match.** ✅ `a25fbff` (O+G 2026-06-10). Arm-level `_`→default_bb Goto (móng SwitchInt). C2.2 suppress E1026 reuse A3 short-circuit. Nới INV 4i-6 `Trap→Trap|Goto` (Unreachable vẫn reject). Fixture 106. Teeth A3-103-bảo-vệ + wildcard→trap SIGILL.
- [~] **C3: Native struct multi-field layout.** → **PHONG ẤN Nhóm E** (= Native struct layout, G defer 2026-06-10). Xem phase10.
- [~] **C4: Packed Outcome ABI.** → **PHONG ẤN Nhóm E** (đi kèm Native). Outcome ops guarded Err, chưa có producer.
- [~] **C5: Multi-value return (>1 return value).** → **PHONG ẤN Nhóm E (G defer 2026-06-10).** Spike O: premise NHẸ (ReturnShape 2-value sẵn + Cranelift multi-return native, KHÔNG vỡ value-model như Native) nhưng **0 producer** (Outcome guarded, tuple-return chưa có). YAGNI. Điều kiện mở: Outcome-producer HOẶC tuple-return syntax + fixture use-case. C5+C4 cùng phụ thuộc Outcome producer. Xem `spec/plans/phase11-c5-multivalue-return.md`.
- [x] **C6: concat sret.** ✅ `992311e` (O+G 2026-06-10). `*mut FatStr` writeback (mẫu (b) append, KHÔNG (a) Rust-auto-sret). Tàn dư Bậc D sạch.

### 🔵 OP. OUTCOME PRODUCER — error-handling core (ADR-0052, O+G ký 2026-06-10)

Frontend ✅ + Typecheck ✅ + Lower ✅ + JIT ✅ (multi-value mở, StackSlot 16-byte). Payload CHỈ scalar Bậc A (heap defer B/C). Blueprint `spec/plans/phase12-outcome-producer.md`. ĐÓNG HOÀN TOÀN.

- [x] **OP.1 Typecheck:** `1e980d0`. verify+bổ sung return-type-match + E1025 (`~0` on T~E) + E1024 exhaustiveness. Fixtures negative check-mode.
- [x] **OP.2 Lower:** `5a127db`. `~+ v`/`~- e` → 2-slot {disc:Trit, payload} + `ReturnShape::BinaryOutcome` + `Return[disc,payload]`.
- [x] **OP.3 JIT (un-defer C5-cho-Outcome):** `25e2d38` (2-register) + `58a7b2d` (StackSlot 16-byte). gỡ guard jit:1070 CHỈ cho BinaryOutcome/TernaryOutcome.
- [x] **OP.4 Match/unwrap:** `6c6e612`. `match o { ~+ x => .. ~- e => .. }` OutcomeDiscriminant+branch+Unwrap. Fixtures run.

### 🟢 D. PERF (G ack §iii, không chặn)

- [ ] **D1: Codegen opt range-check 1-instruction.** `(val−MIN) >ᵤ 2M` unsigned-sub trick + fallback `bor` gộp 2 icmp.
- [ ] **D2: Constant folding pass.** Toán hạng const in-range → tính compile-time, bỏ trap block.

### ⚪ E. CLEANUP

- [x] **E1: codegen.py clippy-clean.** 208 clippy chủ yếu từ file generated `ast_*.rs` (xong nốt 19 clippy còn lại, có `#[allow(cast_possible_wrap)]` kèm comment).
- [x] **E2: Fix fixture 27.** Thay match JIT string bằng error-code (dính C1, C1 đã biến nó thành positive EXPECT 52).

### ⚫ F. DEFERRED-BY-DESIGN (có ADR, KHÔNG phải nợ)

- [x] **D1 (ADR-0041 §6.2):** Arithmetic fidelity — JIT trap-on-overflow (ADR-0044). ĐÓNG.
- [x] **D1-literal:** Typecheck E1036 range-check Integer/Ternary literal. ĐÓNG.
- [x] **D3:** Shim MIN-input — MIN không còn reachable từ arithmetic. ĐÓNG.
- [ ] **D2 (ADR-0043 Q6):** `HashMap::insert` reject-on-insert `i64::MIN` — GIỮ defense-in-depth.
- [ ] Trait system (ADR-0038 Comparable, ADR-0039 Nullable-op family).

## Bậc D — Fat-Pointer ABI (ADR-0049) — ĐÓNG (O+G 2026-06-09)

- [x] **L6-1:** param fat-String by-pointer. `626390c`.
- [x] **L6-2:** return fat-String sret (Lối d). `9caa350`.
- [x] **L6-3+L6-4:** trảm heap len/cap + rút Lối B. `d60eb9b`.
  - Heap: `[Header 8B][data…]`. Data offset +16→+0. Slot là chân lý duy nhất.
  - Mọi shim cập nhật. Borrow String: `stack_addr` thay heap handle.
  - Fallback heap-read → `Err(JitError::Unsupported)` (universal-slot invariant).
- [x] **Endgame fixture 100:** String round-trip 5-boundary. `9b28c54`.
