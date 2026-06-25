---
name: campaign_cfg_tail_expression_kickoff
description: ✅ ĐÓNG TRỌN 2026-06-18 — Chiến dịch CFG Tail-Expression (ADR-0055) hạ màn. Lát 1 SIGILL + Lát 2 `= ~0` xong, commit. KẾ TIẾP = Heap-Nullable (recon ở cuối file).
metadata: 
  node_type: memory
  type: project
  originSessionId: 2e8fd692-48b0-4f38-b76d-815d7e054b83
---

**✅ CHIẾN DỊCH CFG TAIL-EXPRESSION HẠ MÀN (G tuyên 2026-06-18).** 4 commit local (chưa push), origin vẫn `667ea24`:
- `4d51faa` fix Lát 1 + `82863ed` docs — **Lát 1 SIGILL 132**: free/trait fn trả flat-struct qua expr-body emit `Return(struct)` by-value → SIGILL. Fix = helper SSOT `emit_struct_sret_copy` (triet-lower/src/lib.rs) route tail-Return qua sret y hệt Stmt::Return; DRY cả hai caller (G mandate "một van một mỏ lết"). Teeth 182/183/184 poison→SIGILL. Chỉ `MirType::Struct` vỡ (String/Vector/heap-Outcome đã đúng vì trùng khuôn `Return[local]`).
- `a0eff46` fix Lát 2 + `de450c6` docs — **Lát 2 A-hẹp `= ~0`**: tail-value đã wire sẵn bởi ADR-0055+0056/0057/0058 (O probe 20+ dạng đều chạy). Còn ĐÚNG MỘT bất đối xứng tail: `= ~0` báo lowerer-error trong khi `return ~0` chạy → mirror null-~0 special-case (Stmt::Return lib.rs:1265-1276) sang tail-path (đầu 807). Guard `!matches!(Outcome)` giữ ternary `~0` đi OutcomeConstructor (fixture 133→100 verify). Fixtures 185-188. Gate cuối `0·0·183·0`.

**Kỷ luật chiến dịch (O áp):** O probe-trước-thiết-kế LẬT 2 tiền đề recon stale ("match-tail trả 0 sai" = SAI trên HEAD; "Lát 2 = việc lớn" = SAI, đã xong 95%). O đâm-lén-sếp-bằng-data 2 lần → G khen + rút phán quyết. Gap #2 (`{ ~0 }`/if-arm null fail y hệt ở return/let — type-propagation, KHÔNG phải tail-asymmetry) → đẩy Heap-Nullable backlog, chống scope-creep. G chốt DRY null-sentinel = ĐỂ INLINE ("a little copying is better than a little dependency" — 3 dòng gán hằng ≠ van sret 15 dòng logic).

## ★ KẾ TIẾP — recon Heap-Nullable campaign (O đào 2026-06-18, đừng đào lại)
Backlog mở lớn duy nhất = **Heap-Nullable saga ~5 lát** (`T?` cho T heap: String/Vector/HashMap/Struct/Enum).
- **Gate hiện tại:** `Body::verify()` triet-mir/src/lib.rs:1440-1464 refuse `HeapNullableNotLowered`. Chokepoint phủ return/local/struct-field/enum-payload; `find_heap_nullable` (1380) recurse Nullable/Reference/Outcome; `is_scalar_nullable_payload` (1362) whitelist Integer/Trit/Tryte/Long/Trilean/Unit/Unknown. Ruling β (G ký): gate ở LOWER không typecheck — vì stdlib khai heap-nullable làm API stub (`env.get`/`fs.read -> String?`); declaration vô hại, chỉ compilation refuse.
- **Scalar `T?` chạy:** sentinel `NULL_SENTINEL = i64::MIN` (triet-mir lib.rs:2334), canary N1 < mọi range scalar.
- **★ Móng repr (a) ptr-sentinel ĐÃ CÓ MỘT PHẦN ở runtime:** heap shims xử `ptr == NULL_SENTINEL` = null/no-op khắp nơi (mir_lower.rs:2198 string, 2470/2693 hashmap, 4024; test `__triet_string_free(NULL_SENTINEL)` no-op @4786, get-OOB/key-miss trả NULL_SENTINEL @2575/2848). → Lát 3 (JIT conditional Drop) đã có móng free-no-op-on-null. Campaign KHÔNG khởi từ 0.
- **TODO 5 lát:** (1) ADR repr (a) ptr-sentinel slot `{ptr,len,cap}`, `ptr==SENTINEL`=null, null-check project `.ptr` không so cả slot · (2) widening String→String? + `~0` materialize ptr-sentinel · (3) JIT conditional Drop · (4) Elvis `?:` + match `~+/~0` heap (project `.ptr`, move payload) · (5) `?+>` map/flatMap heap (Deinit/tombstone tránh double-free) · gỡ gate. **G nghiêng repr (a).** Cần **ADR-first** (lock repr) trước khi gõ vì design-heavy.

[[mentor_o_persona]] [[colleague_d_persona]] [[lang_return_keyword_survives]]
