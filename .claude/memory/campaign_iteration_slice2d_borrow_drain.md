---
name: campaign_iteration_slice2d_borrow_drain
description: "🏁 ĐÓNG — ADR-0089 Slice 2d: for-item drain qua &0 mutable Vector<T> (borrow-receiver, container-survives). 3 điểm chạm gồm 1 JIT hole D tự bắt ngoài scope. Phiên 2026-07-27, pháo đài #6. SHA 014442e+2dcc9b6 (code) + 75e5c6a (ADR)."
metadata: 
  node_type: memory
  type: project
  modified: 2026-07-26T19:53:53.234Z
  originSessionId: 1687df81-24d3-40a4-91cb-f1662466d6f7
---

# Phiên 2026-07-27 — 🏁 Pháo đài #6: ADR-0089 Slice 2d (`&0 mutable Vector` drain)

origin/main **`75e5c6a`** (synced), gate **`0·clean·0·501·0`** (+5 fixtures 505-509 + break-mid counting).
Code `014442e` (phase 1) + `2dcc9b6` (phase 2), ADR `75e5c6a`. Giang chọn #6 ("đóng hòm cái nhanh gọn").

## 🏁 Cái gì đóng
`for item in <&0 mutable Vector<T>>.drain()` — drain QUA mượn-độc-quyền-mutable, **caller GIỮ container**
(chỉ rỗng buffer), KHÁC BẢN CHẤT Slice 2b (consume owner + drop buffer).
- **Container-Survives (§2d.2):** Vector runtime = buffer-pointer handle single-i64 (`{len@0,cap@8,data@16}`);
  `&0 mutable Vector` reference-value = CÙNG buffer-pointer → `pop_front` `len--` mutate buffer chung → caller
  thấy drain "miễn phí"; buffer giữ (cap), caller own. KHÁC String (len ở stack fat-slot → clear cần slot-ptr).
- **Break-Mid Caller-Drop (§2d.3):** break-mid → buffer.len giảm đúng #popped; caller drop v sau →
  `emit_vector_element_free_loop` quét `0..len` buffer-header → free CHỈ survivors. Tombstone len-- (GOLD 2b)
  nay gánh CẢ caller-later-drop (tương tác mới).
- **Form-aware fence (§2d.4):** chỉ `ReferenceForm::BorrowExclusiveMutable` + T non-nullable; `&0`/`&+`/`&+ mutable`/`&-`
  → E1053, `Vector<T?>` → E1051/E1053 (double-nullable, đợi ADR-0088).

## 🔧 3 ĐIỂM CHẠM (gồm 1 JIT hole D tự bắt ngoài scope phase-1)
1. typecheck `check.rs:759` — refuse mù `matches!(Type::Reference(..))` → form-aware (Type::Reference=**tuple** `(form,box)`).
2. lower `lib.rs:2373` — unwrap `MirType::Reference{form:BorrowExclusiveMutable, inner:Vector}` (MIR=**struct** `{form,inner}`); is_reference tự bỏ drop.
3. **JIT `mir_lower.rs:3909`** — `vector_pop_fat` predicate thiếu unwrap Reference → `&0 mutable Vector<String>` (arg0=`Reference{..}`) rơi `_=>false` → String tưởng thin → codegen fail "unexpected String return". Fix mirror idiom **ĐÃ CÓ SẴN** ở `_get_copy:3967` (`MirType::Reference{inner,..}=>inner.as_ref()`). 3 marshal site (out_ptr/dest-bind) dùng bool sẵn, KHÔNG re-derive → không cần sửa.

## 🩸 Bài học (Mentor O)
1. **D tự bắt lỗ JIT ngoài scope, DỪNG đúng ranh giới WO** (WO cấm đụng jit) → báo O → O verify thật → G duyệt mở
   điểm chạm #3 (phase 2). Kỷ luật gác cổng: lính bác scope có DATA → verify data (nghi thức #18), không ép.
2. **Verify claim G cắt CẢ chiều — bắt lệch shape:** G viết `MirType::Reference(_, inner)` (tuple), thực tế
   **struct** `{form,inner}` (mir:507). Typecheck Type::Reference MỚI là tuple (types.rs:117). Hai tầng KHÁC shape
   → khắc đúng từng tầng vào WO, D không compile-fail. (G cũng sai E2403→E2420 ở Slice 2c — số liệu G phải đo lại.)
3. **★ POISON-KHÔNG-ĐỎ → phát hiện no-drop 2-LỚP (nghi thức #4/#16):** poison-1 (push_owned receiver) KHÔNG đỏ —
   KHÔNG phải "an toàn", mà poison SAI LỚP. No-drop qua `is_copy(Reference)==true` (mir:736) ở CẢ lowerer
   (no push_owned) VÀ JIT Drop:3397 (skip). push_owned bị lớp is_copy bắt. Escalate poison chokepoint (is_copy
   Reference→false) → 506 `Drop for type &0 mutable Vector<String> not supported` **fail-closed** (JIT không có
   drop-glue cho reference) + counting ĐỎ. **Failure-mode container-survives = fail-closed, KHÔNG silent double-free**
   — an toàn hơn G lo. Defense-in-depth như SPOF Slice 2b.
4. **Poison bán kính đúng:** tháo JIT Reference-unwrap → heap 506 ĐỎ nhưng scalar 505 vẫn OK (poison chỉ trúng
   fat/heap path). Chứng minh fix khu trú đúng fat-detection.
5. **Idiom precedent cùng file = fix low-risk:** Reference-unwrap `_get_copy:3967` (cách `&0 Vector` get chạy,
   fixture 168) chứng minh không novel → recommend Phương án A (hoàn tất) thay vì descope nửa vời.

## ⚙️ Quy trình — 5 pha + phase-2 mở scope giữa chừng
Giang chốt #6 → O recon file:line (verify 7/7 sự thật, bác cờ đỏ "borrow-param heap chưa có" bằng fixture 93-99) →
ADR-first §2d → G duyệt + tự đo (E) → WO → D phase-1 (scalar, bắt JIT hole, dừng) → O verify hole THẬT + đo bán
kính → G duyệt Phương án A → SendMessage resume D phase-2 (context nguyên) → O verify máu (3 poison + escalate) →
O✅/G✅/Giang✅ → O commit ADR + push.

## Nợ còn (G phong tỏa, chờ Giang+O chốt)
🔴 ADR-0088 double-nullable T?? (cliff nặng, ADR-first) · HashMap.drain() · Deep-Clone · §15.6 Vector<Leaf?> ·
N1 widening (ADR-0065) · O(N) cursor-drain perf. ⚰️ ADR-0068 Box/recursive CẤM CỬA.

→ [[campaign_iteration_slice2c_force_unwrap]] [[campaign_iteration_slice2b_drain]] [[campaign_iteration_slice1_2a]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]]
