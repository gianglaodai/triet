# TODO — Triết Backend (Track C)

Backlog sống cho chiến dịch kế. **Chỉ chứa việc CHƯA xong / phong-ấn.**
Ledger các phần ĐÃ đóng (per-step + commit-hash) → [`docs/TODO-ARCHIVE.md`](docs/TODO-ARCHIVE.md) + `git log` + `docs/decisions/`.

Mốc hiện tại: origin `96986b4` (2026-06-18). Gate `0·0·176·0`.

---

## 🟢 BACKLOG MỞ

### 🔴 Chiến dịch CFG Tail-Expression — ưu tiên 1 (soundness)
Wire nốt ADR-0055: block tail-expr gánh giá trị cuối hàm.
return-scope đã khóa (ADR-0020 §3.8): `return` = early-exit + cọc-tiêu-mode, KHÔNG phải throw.

- [ ] **ĐẬP TRƯỚC TIÊN (soundness):** 🔴 expr-body fat-struct return không route sret → **SIGILL 132**.
      Free fn `f() -> Point = Point{...}` emit `Return(struct)` by-value thay vì ghi sret slot;
      block-body (`{ return ... }`) chạy đúng. Crash/soundness hole có sẵn, độc lập trait/nợ#2.
      *Soundness trước syntax (G 2026-06-17).*
- [ ] Wire tail-expr gánh giá trị cuối hàm → giảm `return` cuối thân (happy-path). Làm SAU khi SIGILL đóng.

### 🟣 Chiến dịch Heap-Nullable — saga ~5 lát
`T?` cho `T` heap (String/Vector/HashMap/Struct/Enum). Hiện **GATE ở LOWER** bằng
`MirError::HeapNullableNotLowered` (`Body::verify()` refuse — KHÔNG ở typecheck).

**Ruling β (G ký 2026-06-18):** gate ở LOWER, KHÔNG typecheck — vì stdlib khai
heap-nullable làm API (`env.get`/`path.parent`/`text.from_bytes`/`fs.read -> String?`).
Declaration vô hại (stub `= ~0`); chỉ *compilation* mới miscompile (Bậc A nullable =
single-i64 sentinel, không chứa nổi fat-pointer 24B). Nếu sau cần chặn sớm ở Pass-1 →
Option-2 (gate free-fn `resolve_type_expr_with_params`, đổi chữ ký + dedup).

- [ ] 1. **ADR repr — (a) ptr-sentinel** (G nghiêng): slot `{ptr,len,cap}`, `ptr == NULL_SENTINEL` = null; null-check project `.ptr`, không so cả slot.
- [ ] 2. Widening `String → String?` + `~0` materialize ptr-sentinel.
- [ ] 3. JIT conditional Drop (`if ptr != SENTINEL → drop payload`).
- [ ] 4. Elvis `?:` + match `~+/~0` heap (project `.ptr`, move payload).
- [ ] 5. `?+>` map/flatMap heap (unwrap move + Deinit/tombstone tránh double-free).
- [ ] Gỡ gate `HeapNullableNotLowered` (+ `find_heap_nullable`/`is_scalar_nullable_payload` helper ở triet-mir) khi móng landed.

### 🟢 Perf — ADR-0044 §iii (không chặn)
- [ ] **D1 Codegen opt range-check 1-instruction:** `(val−MIN) >ᵤ 2M` unsigned-sub trick + fallback `bor` gộp 2 icmp. Cắt nửa instruction mỗi Add/Sub.
- [ ] **D2 Constant folding pass:** toán hạng const in-range → tính compile-time, bỏ trap block.

### Khác
- [ ] **D2 HashMap reject-MIN** (ADR-0043 Q6): `insert` reject `i64::MIN` — GIỮ defense-in-depth.

---

## 🔒 PHONG ẤN Nhóm E — YAGNI (G defer 2026-06-10)

Mở khi có tiền đề (value-model thoát single-i64 / producer thật). KHÔNG build tạm.

- Native struct multi-field layout — cần value-model + ADR byte-size mapping + fixture Trit/Tryte-in-struct. Spike: `spec/plans/phase10-native-struct-layout.md`.
- Packed Outcome ABI — đi kèm Native.
- Multi-value return (>1 value) — cần producer thật (Outcome/tuple). Spike: `spec/plans/phase11-c5-multivalue-return.md`.
- `&+` / `&-` borrow forms — phong ấn (ADR-0059).

---

## ✅ ĐÃ ĐÓNG — tóm tắt (chi tiết: [`docs/TODO-ARCHIVE.md`](docs/TODO-ARCHIVE.md) + git + ADR)

- **Phase 4 Aggregate:** struct/enum/String/Vector/HashMap (ADR-0042 B7-lift · 0043 HashMap · 0060 nested `a.b.c`).
- **Phase 5 Bậc C borrow:** ADR-0044 trap-overflow · 0045/0046 `&0` borrow + return-elision · 0047 read-ops · 0048 mutable-borrow · 0059 `&0` heap.
- **Bậc D Fat-Pointer ABI:** ADR-0049 (param/return fat-String sret, slot = chân lý duy nhất).
- **Outcome `T~E`:** ADR-0050 MirType · 0051 borrowck-merge · 0052 producer · 0055→0058 CFG/heap sret.
- **Trait Tier 1:** ADR-0061 (`594abd9`) — dispatch + Comparable (ADR-0038) + match-on-Trit.
- **Nullable `?+>` scalar:** ADR-0039 (`73532b4`) — map/flatMap, `?->` → E1046.
- **Chiến dịch Trả Nợ** (2026-06-09→10): A (3 bom) · B1 type-system (ADR-0050) · B2 borrowck-merge (ADR-0051) · C1/C2/C6 feature-gaps · OP Outcome-producer.
- **Chiến dịch Cleanup "Đại Hốt Xà Bần"** (2026-06-17→18): LoweringInput refactor · fat-return trait sret · heap-nullable LOWER-gate · return-scope ADR-0020 §3.8.
