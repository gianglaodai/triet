---
name: project_rewrite_reality_2026_06_04
description: ĐỌC ĐẦU TIÊN — thực tại repo sau cú rewrite 2026-06-04; mọi handoff/state cũ trước mốc này đã LỖI THỜI.
metadata: 
  node_type: memory
  type: project
  originSessionId: cbfcad37-8830-40cb-a053-1a01523fea6d
---

**ĐỌC FILE NÀY TRƯỚC MỌI HANDOFF CŨ.** Ngày 2026-06-04, author **xóa vĩnh viễn
backend của compiler đã ship v0.2–v0.10** và bắt đầu lại từ backend (cái trước
đây gọi là "Track B"). Mọi memory/handoff/state ghi TRƯỚC mốc này (v0.11 jit.4
96%, AOT cache, two-track, 1637 tests, self-host sống) **mô tả thế giới đã chết**
— chỉ đọc làm lịch sử, ĐỪNG khuyến nghị dựa trên chúng.

## Đã xóa (HEAD `6a6bd93`)
Crate: `triet-ir`, `triet-interpreter`, `triet-bootstrap`, `triet-cli` + 5500 dòng
JIT legacy. Git history giữ lại. Bộ 1637-test safety net biến mất theo VM.

## Còn sống — 13 crate
Foundation: core, logic, syntax. Frontend reused (well-tested): lexer, parser,
modules, typecheck. Backend mới: lower, mir, borrowck, jit, driver. Packaging
(chưa wire vào pipeline mới): pack.

## Độ chín THỰC (đừng tự nhận 96%) — cập nhật 2026-06-06
**Sau mốc dưới đây, đã đóng thêm:** Phase 4.3a String + 4.3b Vector + 4.3c
(heap Bậc A, M1-M4, BuiltinShimMeta) và **ADR-0041 Nullable `T?` Bậc A**
(PA-3c uniform `i64::MIN`, widening/`~0`/Elvis/`get`, trap-on-0). HEAD `28c1a5f`,
43 fixtures, 1070 tests.

### Mốc 2026-06-05 (giữ làm lịch sử)
**Phase 3 (Cranelift backend) ĐÓNG ở mốc "Bậc-A complete":** scalar + arithmetic
+ logic-op + control flow + call + **flat struct native** (StackSlot + sret +
by-pointer field access; Gate A) + NLL borrowck + MIR verifier (INV-1/INV-2).
Refuse sạch (defense-in-depth): nested field `a.b.c`, Deref/Index (provably-
unreachable — lower chỉ emit `Projection::Field`), Outcome ops, multi-value return.
**CHƯA dựng:** aggregate literals (String/Vector/HashMap/Enum/`match` → `Err` ở
lowerer = **phase 4** job, KHÔNG phải backend); Outcome 2-reg ABI + multi-return
= **Bậc C** defer; self-host; AOT cache. 16-fixture integration corpus (driver) =
lưới an toàn thay 1637-test oracle đã xóa. Báo cáo: `spec/plans/REPORT-2026-06-04.md`
+ phase status lines (đã de-inflate, trung thực).

## Mìn — ĐÃ XỬ LÝ (cập nhật 2026-06-05)
1. ✅ **`compiler/` mồ côi → XÓA** (10/10 file frontend reject, 23.4K dòng).
2. ✅ **Version → `0.1.0-dev`** (dòng mới, thừa nhận khởi động lại); ROADMAP sync.
3. ✅ **TODO.md → ghi đè thành backlog Track B.**
4. ✅ **JIT Outcome miscompile → GUARD** (`mir_lower.rs`: 3 op trả `Err` +
   test bắt-regression đỏ-khi-gỡ-guard). Provably-unreachable (lower chưa sinh Outcome).
5. ✅ **docs/ legacy → `docs/ARCHIVE.md`** (digest + catalog 36 ADR LIVE/TOOLING/
   HISTORICAL); ADR ngữ nghĩa giữ sống. README viết lại tiếng Anh.
6. ✅ **spec/plans status de-inflate** (reconcile pass): phase1-6 status trung thực.
7. ⚠️ **Schema `Type` enum = DEAD** (typecheck dùng hand-written Type riêng) — đã
   tag spec-only + hạ tuyên ngôn SSOT; migrate Type→schema = **conscious deferral**,
   phase backlog. examples/+demos/ VM-era = stale fixture, chưa prune.

## Mẫu lặp đáng nhớ (coaching)
Author báo "done/xanh" hay sót đúng-một-chỗ, lộ khi mentor grep: fixture-21 (premise
sai), SSOT (sót 2 chỗ), Gate A (warning `ReturnShape` 2×), "build xanh" sai 2 lần.
Nguyên nhân: tuyên-bố-trước-khi-chạy-lệnh-gate. Chữa: chạy CHÍNH lệnh sẽ chấm
(`cargo build|grep warning:`, test mới phải tồn tại) TRƯỚC khi gõ "done".
Xem [[feedback_verify_semantics_before_asserting]].

## Bài học mentor đã nêu cho author
Sai lầm không phải QUYẾT ĐỊNH rewrite (kiến trúc MIR+NLL+native sạch hơn JIT
delegate-to-VM cũ — hợp lý). Sai lầm là **THỨ TỰ**: xóa VM oracle (1637 test)
TRƯỚC khi JIT mới chạm parity. Đúng quy trình: dựng song song, giữ differential
oracle, đẩy tới parity, rồi mới xóa. Đây là lần 2 author theo pattern "dựng tới
~90% rồi đập đi làm lại" và tự reframe phần vứt thành "bản nháp". Xem [[feedback_stability_over_speed]].
