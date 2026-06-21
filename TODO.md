# TODO — Triết Backend (Track C)

Backlog sống cho chiến dịch kế. **Chỉ chứa việc CHƯA xong / phong-ấn.**
Ledger các phần ĐÃ đóng (per-step + commit-hash) → [`docs/TODO-ARCHIVE.md`](docs/TODO-ARCHIVE.md) + `git log` + `docs/decisions/`.

Mốc hiện tại: HEAD `6ac3b27` (test) (chờ O ký + push). Gate `0·0·253·0`. **⚰️ Trục B (Heap-in-Aggregate) — nhát 1b: ARG-MOVE qua boundary ĐÓNG (chờ ký).** ADR-0066 KCN-2 — `take(p: Person{name:String})` whole-move: callee free + caller tombstone, FREE_COUNT==1 (no leak, no double-free). **(A)** callee by-pointer drop-glue (struct branch dùng `copy_base_addr` unify slot-local + param; `emit_heap_free_at` address-based) · **(B)** 6 to_zero filter `is_copy(None)`→`ctx_is_copy` (heap-struct arg → Deinit) · **(C)** Deinit walk struct tombstone ptr@field.offset. LUẬT THÉP ATOMIC giữ (Deinit đầu ret_bb). 3 răng đỏ độc lập route-lower (R-callee→0 / R1-deinit→2 / R1-arg→2 double-free). Fixture 258 + counting test `arg_move_struct_frees_once`. **Nhát 1a (trước):** M-1 sizing + M-2 B8-relax + KCN-1 drop-glue + STEP 4 fat-store; 256/257 + 3 unit teeth. **Trước đó:** ADR-0065 §12.8/§12.7 — Chuỗi Nullable Aggregate Trục A HOÀN TẤT. Kế: **1c assign-move** (`let q = p` pseudo-copy, G ghi nợ) · borrowck use-after-move (G: JIT tombstone đủ, ptr=0→SIGSEGV sạch) · Vector/HashMap field · Lát 2 nested/recursive + enum-payload heap.

### 🟡 Sổ nợ Tech-Debt Hạ tầng — counting-test parallel isolation
Các test free-count (`nullable_map_heap_output_counting`, `vector_nullable_drop_counting`, …) dùng process-global `AtomicUsize` + no-mangle shim → flake hiếm dưới `cargo test --workspace` tải nặng (đo: `map_vector_output_freed_once` đỏ 1 lần, xanh 6+ lần isolation/release/re-run). Cần `--test-threads=1` hoặc subprocess isolation (hạ tầng N7 đã có cho một số). KHÔNG chặn nhát 1a (code orthogonal). Ghi nợ theo lệnh G.

---

## 🟢 BACKLOG MỞ

### 🔴 Chiến dịch CFG Tail-Expression — ưu tiên 1 (soundness)
Wire nốt ADR-0055: block tail-expr gánh giá trị cuối hàm.
return-scope đã khóa (ADR-0020 §3.8): `return` = early-exit + cọc-tiêu-mode, KHÔNG phải throw.

- [x] **ĐẬP TRƯỚC TIÊN (soundness):** 🔴 expr-body fat-struct return không route sret → **SIGILL 132**. `4d51faa`
      Free fn `f() -> Point = Point{...}` emit `Return(struct)` by-value thay vì ghi sret slot;
      block-body (`{ return ... }`) chạy đúng. Crash/soundness hole có sẵn, độc lập trait/nợ#2.
      *Soundness trước syntax (G 2026-06-17).* — ADR-0055 lát 1: helper SSOT `emit_struct_sret_copy`
      route tail-Return qua sret y hệt Stmt::Return; teeth 182/183/184 poison→SIGILL.
- [x] Wire tail-expr gánh giá trị cuối hàm → giảm `return` cuối thân (happy-path). `a0eff46`
      ADR-0055 lát 2 A-hẹp: phần lớn ĐÃ wire bởi ADR-0055+0056/0057/0058 (probe 20+ dạng:
      literal/expr/if/match/nested/while-tail/struct/heap-if/heap-match/outcome/nullable-widen
      đều chạy). Còn đúng MỘT bất đối xứng tail: `= ~0` báo lỗi trong khi `return ~0` chạy →
      mirror null-~0 special-case sang tail-path. Fixtures 185-188. Gap #2 (`{ ~0 }`/if-arm)
      đẩy Heap-Nullable (fail y hệt ở return/let, không phải tail-asymmetry).

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
- [ ] **Gap #2 — expected-type propagation cho `~0`/Outcome-constructor lồng trong block-final/if-arm/match-arm.** `{ ~0 }`, `if…{~0}` fail y hệt ở CẢ `return`/tail/`let` (đã chứng minh, KHÔNG phải tail-asymmetry — Lát-2 A-hẹp chỉ vá expr-body `= ~0`). Cần ADR type-propagation (đẩy expected-type xuống block/if/match arm), KHÔNG chắp vá per-site.

### 🟣 ADR-0065 Nullable Aggregate (`Enum?` & `Struct?`) — 🔒 LOCKED, 2 lát
Bất biến hợp nhất: `tag_cell == i64::MIN ⟺ null`. Rào **B8 §4**: aggregate-nullable CHỈ
chứa Copy field/payload — KHÔNG drop-glue/alloc/free. Value-model i64 KHÔNG đụng.

- [x] **Lát 1 — `Enum?` (disc-sentinel niche, 0 byte).** `1748510` (feat) + `e9bd3e0` (§9.1).
      disc@0 == i64::MIN = null (discriminant thật ∈ {0,1,2,…}); widening no-op. 5 delta:
      A gate(triet-mir 1399) · B slot-alloc(triet-jit) · C walk_proj unwrap · E result-retype
      (lower, idiom ADR-0056) · F `~0` store disc@0. Fixtures 225-230 (present payload-less/
      payload, ~0 null, Elvis, widening, B8 heap refuse). Poison E/B→SIGSEGV139, F→Trap132.
- [x] **Lát 2 — `Struct?` (tag 8B, Phương án A, β).** slot `{tag@0:i64, fields@8…}`,
      total = struct.total_size + 8. tag@0 == i64::MIN = null / +1 = present. **6 delta:**
      Delta 0 lowerer (Struct→Struct? widening → fresh local + Assign, KHÔNG retype in-place —
      §9.2, recon-miss của O vá in-scope) · 1 gate += `Struct(_)` · 2 slot-alloc +8 (skip
      sret/param/String) · 3 walk +8 (helper `nullable_struct_base_offset`) · 4a widening
      tag=1 + copy→+8 · 4b **β** whole-slot N+8 tag-first (`T?→T?` propagate, KHÔNG refuse).
      Lệch-lệnh chuẩn thuận: `is_aggregate` + slot-loop skip `Struct("String")` (borrowck builder
      type String-local thành `Struct("String")` slot-less → tránh deref SIGSEGV). Fixtures
      231-237; **237 = teeth tag-store** (reassign-widen-over-null, slot tái-dùng MIN). O verify
      máu P1-P5 RED: P1 walk→231/234 sai · P2 4a-1word→SIGILL · P3 tag-store→237 đỏ (231 tươi
      KHÔNG bắt) · P4 4b-tag→234/235 đỏ · P5 B8 gate→236+180. Gate `0·0·232·0`.

### 🟢 Perf — ADR-0044 §iii (không chặn)
- [ ] **D1 Codegen opt range-check 1-instruction:** `(val−MIN) >ᵤ 2M` unsigned-sub trick + fallback `bor` gộp 2 icmp. Cắt nửa instruction mỗi Add/Sub.
- [ ] **D2 Constant folding pass:** toán hạng const in-range → tính compile-time, bỏ trap block.

### Khác
- [ ] **D2 HashMap reject-MIN** (ADR-0043 Q6): `insert` reject `i64::MIN` — GIỮ defense-in-depth.
- [ ] **gate.sh giòn — exit 1 giả khi clippy=0** (G ghi sổ 2026-06-18): dòng cuối `clippy … | grep -- "-->" | sort -u | wc -l` dưới `set -o pipefail` → clippy 0 warning ⇒ grep no-match ⇒ exit 1 ⇒ script exit 1 dù output 4 dòng sạch. Đếm log lởm. Vá ở chiến dịch dọn CI (vd `grep -c` hoặc `|| true` có kiểm). KHÔNG ảnh hưởng soundness C-track.

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
