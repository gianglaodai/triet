# TODO Archive — Track C Rewrite (các phần ĐÃ ĐÓNG)

Ledger per-step của backend rewrite, archive khỏi `TODO.md` ngày 2026-06-18 để
`TODO.md` chỉ còn backlog sống. Mỗi mục giữ commit-hash + ADR để truy vết. Lịch
sử đầy đủ (diff từng dòng) ở `git log`; lý do thiết kế ở `docs/decisions/`.

Mốc đóng: origin `96986b4` (2026-06-18).

---

## Phase 4 — Aggregate Type Lowering
- [x] Struct literal lowering (Cranelift StackSlot).
  - [x] **Nested field access (ADR-0060 P2):** `f28d14d` `a82e44c`. Offset-chain + multi-word copy. P1 (Sub-8B packing) vẫn khóa.
- [x] Enum literal lowering (unit + payload, end-to-end).
  - *MIR: EnumAlloc, SetDiscriminant, GetDiscriminant, SwitchInt, Trap, Payload projection.*
  - *Parser: bare-variant resolution là global name-match (lowerer scans enum_layouts). Defer: typecheck annotate variant resolved lên AST.*
  - *construct semantic = COPY (SPEC §10.1). Fixture 28 pin.*
- [x] String literal lowering (Phase 4.3a). Shims alloc/from_bytes/free/concat/eq/len. M1-M4 move semantics.
- [x] Vector support (Phase 4.3b).
- [x] Nullable (`T?`) repr Bậc A — ADR-0041 (PA-3c uniform MIN). NULL_SENTINEL + widening + `~0` + Elvis + get. Fixtures 40-46.
- [x] Match `~+/~0` 2-arm nullable. `b7d1f98` `279e7f2`. E1026 exhaustiveness, E1035 `~-` rejection. Fixtures 48-57.
  - *Debt F6: ĐÓNG bởi A2+A3 `d8e1ba9` (Unreachable terminator → INV-4).*
- [x] B7-lift ownership-across-boundary. `d36244a` `d9b5cf4` `a58693e` `0f9b1d8` `86b7039`. ADR-0042. Deinit tombstone + borrowck M3+. Fixtures 58-65.
- [x] HashMap. `2b72c62` `3951821` `d2e3043` `a08916d` `ed71185` `07da95f` `247a3be`. ADR-0043. 5 shim, insert-or-update, rehash, D2 reject-MIN. Fixtures 66-73 + C9.
- [x] ReturnShape::Struct multi-field returns.
- [x] MIR verifier: enum structural invariants (4i-1→4i-7).

## Phase 5 — Bậc C (borrow params)
- [x] **ADR-0044 trap-on-overflow:** `1fbf6ab`. JIT range check (trapnz SIGILL), E1036 literal overflow, pow checked_mul. 8 N7 tests.
- [x] **ADR-0045 Borrow Params Heap (`&0 T` read-only):** `1cd7635`. Xóa type-erasure `"?"`. Return-borrow CẮT (E1042).
  - [x] B1-B7: type thật reference · lower no push_owned borrow · caller no zero · wire check_body_with · E1042 refuse `-> &0 T` · read-op `length` · TODO+engine.
- [x] **ADR-0046 Return-borrow Elision:** `cfae64d` `034ba0d` `d6e3da0`. `-> &0 T` qua form-gate + reuse E2400 + return_borrow_map. Fixture 84.
- [x] **ADR-0047 Read-ops:** `3259631` `5071be1` `a92e415` `6052509` `3012af8`. `contains` + `is_empty`. 8 fixtures.
- [x] **ADR-0048 Mutable Borrow:** `7390012` `d556f2a` `bdaa5e3`. `clear(&0 mutable String)` in-place. E2440 exclusivity. Fixtures 93-95.
- [x] **ADR-0059 Mũi C (`&0` heap Vector/HashMap):** `bf668fd` `8be0263`. + vá Generic return-bind. `&+`/`&-` phong ấn YAGNI.

## Bậc D — Fat-Pointer ABI (ADR-0049) — ĐÓNG (O+G 2026-06-09)
- [x] **L6-1:** param fat-String by-pointer. `626390c`.
- [x] **L6-2:** return fat-String sret (Lối d). `9caa350`.
- [x] **L6-3+L6-4:** trảm heap len/cap, slot=chân lý duy nhất. `d60eb9b`. Heap `[Header 8B][data…]`. Fallback heap-read → Err (universal-slot invariant).
- [x] **Endgame fixture 100:** String round-trip 5-boundary. `9b28c54`.

## Chiến Dịch Trả Nợ (O+G classified 2026-06-09)
Strategy: A1 → B1 → B2 → B3 → C/D/E.

### A. BOM (sai im lặng / UB)
- [x] **A1 `is_propagated` nested-scope:** `be37875`. `live_out.contains(dest)`. Fixtures 101/102. Teeth O: poison → 102 đỏ (UAF lọt).
- [x] **A2 F6 MIR verifier INV-4:** `d8e1ba9`. Bắt Unreachable terminator referenced.
- [x] **A3 Enum exhaustiveness:** `d8e1ba9`. E1026. Fixture 103.

### B. NỢ-MÓNG
- [x] **B1 Rombac Type System (ADR-0050):** S1 `76b53cb` · S2 `fe80b8c` · S3 `ec6d32f` · S4 `9af6afd`. MirType 14-variant thay string-match, net −470 dòng. 14 vòng O chặn (bom producer-ngụy-trang).
- [x] **B2 Sáp nhập 2 tầng borrowck (ADR-0051):** `1e6c14e` · `58dfa4e` + cleanup. MIR NLL = cảnh sát duy nhất. E2420+E2440 teeth-isolate.
- [x] **B3 Alias analysis:** DEFER (YAGNI). Spike `11d11cf`: 0 fixture over-reject. `conservative=true` giữ defense-in-depth.

### C. FEATURE GAP
- [x] **C1 Enum payload qua function param:** `0fb8de6`. by-pointer. Khép nợ no-consumer móng B1. Fixture 27→positive 52.
- [x] **C2 Pattern::Wildcard enum match:** `a25fbff`. Arm `_`→default_bb. Fixture 106.
- [x] **C6 concat sret:** `992311e`. `*mut FatStr` writeback.
- (C3 Native / C4 Packed / C5 Multi-value → phong ấn Nhóm E, xem TODO.md.)

### OP. Outcome Producer (ADR-0052) — error-handling core `T~E`
- [x] **OP.1 Typecheck:** `1e980d0`. return-type-match + E1025 + E1024.
- [x] **OP.2 Lower:** `5a127db`. 2-slot {disc:Trit, payload} + BinaryOutcome.
- [x] **OP.3 JIT:** `25e2d38` + `58a7b2d` (StackSlot 16-byte).
- [x] **OP.4 Match/unwrap:** `6c6e612`. OutcomeDiscriminant + Unwrap.
- (Chuỗi heap Outcome ADR-0055→0058 + HP.1-5 — xem git + ADR.)

### E. CLEANUP
- [x] **E1 codegen.py clippy-clean** (208→0, `#[allow(cast_possible_wrap)]` per-site + comment).
- [x] **E2 Fix fixture 27** (error-code thay match JIT string).

### F. DEFERRED-BY-DESIGN (có ADR)
- [x] D1 (ADR-0041 §6.2) arithmetic trap-on-overflow → ADR-0044.
- [x] D1-literal typecheck E1036 range-check.
- [x] D3 shim MIN-input unreachable.

## Deferred → ĐÃ TRIỂN KHAI (2026-06-15→17)
- [x] **Trait system** — ADR-0061 Tier 1 static dispatch + push `594abd9`. `implement T for Type` → `a.method(b)` mangled dispatch + match-on-Trit. Tier 2/3 đóng băng YAGNI.
- [x] **Comparable trait `compare() -> Trit`** — ADR-0038, qua Trait Tier 1 (fixture `174_trait_comparable`).
- [x] **Nullable `?+>`** — ADR-0039, Phase 14 + push `73532b4` (map/flatMap scalar; `?->` → E1046). Heap → backlog Heap-Nullable.
- [x] **SPEC append(byte) range:** REJECT out-of-range `0..=255` → `abort()` (ADR-0044 spirit). 2 N7 teeth. `mir_lower.rs`.

## Chiến dịch Cleanup "Đại Hốt Xà Bần" (2026-06-17→18, origin `96986b4`)
- [x] **Nợ #1 LoweringInput:** `74a33c3`. Bó 8 input bất biến → `struct LoweringInput<'a>`, giết 2 `#[allow(too_many_arguments)]`.
- [x] **Nợ #2 fat-return trait sret:** `3ea619f`. Arm `MethodCall` inline 3 nhánh + refuse NARROW Vector/HashMap/Enum/Reference. Teeth arg-order → SIGSEGV.
- [x] **Nợ #3 heap-nullable LOWER-gate:** `3e4cb02`. `MirError::HeapNullableNotLowered` ở `Body::verify()` (ruling β). 4 scan return/local/struct-field/enum-payload.
- [x] **Hiến pháp return-scope:** `96986b4`. ADR-0020 §3.8 + TODO backlog.

## Integration Test Corpus (móng)
- [x] Basic test harness · while-loop hang fixed · Trilean logic ops · enum fixtures.
