---
name: campaign_iteration_slice2b_drain
description: "🏁 ĐÓNG — ADR-0089 Slice 2b: for-item-in-Vector.drain() consuming move-out (heap mở), zero-shim desugar pop_front. GOLD: Tombstone DOUBLY LOAD-BEARING. Phiên 2026-07-26 (b), pháo đài thứ 4. SHA 780e081."
metadata: 
  node_type: memory
  type: project
  originSessionId: e83f091f-aeba-435d-b4e9-a103c024fd1c
  modified: 2026-07-26T15:51:00.082Z
---

# Phiên 2026-07-26(b) — 🏁 Pháo đài #4: ADR-0089 Slice 2b (Drain & Consuming Iteration)

origin/main **`780e081`** (synced), gate **`0·clean·0·488·0`** (+9 fixtures 486-494, +4 counting teeth).
Nối tiếp phiên 3-pháo-đài (adfe8f9): Slice 1 + Slice 2a → nay Slice 2b khép chiều **tiêu thụ**.

## 🏁 ADR-0089 Slice 2b (`780e081`) — `for item in v.drain()` move-out
`for item in v.drain()` tiêu thụ `Vector<T>` phần tử-một, **move-out by-value** MỌI T (kể cả
**heap** — `Vector<String>`, `Vector<User{String}>` — điểm KHÁC Slice 2a refuse E1053 copy=alias:
drain chuyển sở hữu → hết alias). Container drop **buffer-only** sau vòng.
- **Zero-shim:** desugar về vòng `pop_front` — 100% mảnh proven (fixture 347/351/338). KHÔNG chạm
  JIT/borrowck/schema. Kiến trúc G duyệt: chấp nhận **O(N²)** (pop_front shift) correctness-first,
  O(N) cursor-drain = nợ perf tương lai.
- **2 điểm chạm:** (1) typecheck `check.rs check_for_stmt` — `.drain()` là for-guard-ONLY pseudo-
  method, match TRƯỚC, infer CHỈ receiver (standalone `v.drain()` vẫn E1041). (2) lower `lib.rs
  Stmt::For` drain arm — hdr `pop_front` + present-test `If(Eq opt,NULL_SENTINEL)` + PA-3c identity
  unwrap; break→ext, continue→hdr (KHÔNG step block, pop_front tự advance).
- **error.rs +62:** 2 variant `DrainNullableElement`/`DrainBorrowedReceiver` — CÙNG code E1053
  (drain-context message), KHÔNG mã mới.

## 🥇 PHÁT HIỆN VÀNG — Tombstone DOUBLY LOAD-BEARING (O poison đo)
Tháo `len--` khỏi `__triet_vector_pop_front` (`mir_lower:6162`) gây **HAI failure-mode phân biệt**:
- (a) **full-drain HANG VÔ HẠN** — pop_front không bao giờ báo empty (len đứng nguyên) → present-test
  không bao giờ dừng vòng. Tức `len--` là **điều kiện DỪNG** của CFG loop.
- (b) **break-giữa-chừng FAILED** — Drop re-walk slot đã move-out → survivor re-free mismatch (double-
  free). Tức `len--` cũng là **chốt chống double-free**.
→ `len--` mang **TẢI TRỌNG KÉP** (khắc vào ADR §2b.5). Teeth `drain_iter_counting.rs` canh cả hai.

## 6 điều kiện thép (G mandate) — ✅ verify máu
491 standalone→E1041 · 492 `&0 Vector`→E1053 · 493 `Vector<T?>`→E1053 (double-nullable defer) ·
494 method-lạ→E1052 · 487 `Vector<String>`+488 `Vector<User{String}>` allocator THẬT total=5 ·
counting rvalue FREE=1 + break-mid=5 + container=1.
Bonus O tự loại: **sentinel-collision** (`Vector<Integer>` chứa `i64::MIN`) BẤT KHẢ — PA-3c sentinel
ngoài dải Integer hợp lệ (ADR-0044/E1036). Deinit(opt_local)=**zero KHÔNG free** (JIT `mir_lower:2928`).

## ⚙️ Quy trình phiên — 5 pha đầy đủ (recon→ADR→WO→D→verify)
O recon file:line → trình bản đồ (drain=proven-pieces) → G duyệt scope Vector-only (BÁC HashMap.drain
"từng pháo đài một") + 6 điều kiện thép → Giang ký ban hành + **O spawn D (Sonnet 5)** → verify máu.

## 🩸 Bài học (Mentor O)
1. **D bị cắt ngang mid-poison-verify** (`<result>`="I'll stop here and wait for background task...")
   — KHÔNG phải submit-giả. Để lại code-poison trong cây (`mir_lower:6164` tombstone comment-out) =
   trạng thái verify dở, KHÔNG malice. O restore về HEAD `da3a0d80` (KHÔNG vào commit).
2. **Docstring giả-thuyết-sai của D:** claim poison→`STR_FREES==6`, THỰC TẾ = hang vô hạn (loop không
   bao giờ tới Drop để đếm 6). O sửa docstring về đo thật. → Giả thuyết viết-trước-khi-chạy ≠ verify.
   O CHỈ chạm COMMENT test D, KHÔNG đụng code logic D (check.rs/lib.rs/error.rs nguyên vẹn — "không sửa hộ").
3. **Infinite-loop LÀ tín hiệu poison** cho drain (khác double-free của single-pop). Giang tự bắt "chạy
   quá lâu" → O phải **timeout-bound + đo giờ** mọi lệnh có thể hang (poison làm loop vô hạn). Luật mới:
   poison cấu-trúc-loop → wrap `timeout` cứng, exit 124=hang.
4. **verify-don't-trust cứu lần nữa:** O tự chạy gate + tự cắm poison độc lập (snapshot cp→poison→đo
   đỏ→restore md5 khớp), KHÔNG tin lời D. Bóc được 2 loose-end (left-poison + false-docstring).
5. G rule "bằng chứng là vua, không thờ cúng thủ tục" — code sound + O verify độc lập xong → ký+commit,
   KHÔNG recall D làm nghi thức thừa.

## Nợ còn (campaign riêng, chờ recon-đầu-ca-tươi)
🔴 ADR-0088 double-nullable `T??` (`Vector<T?>` drain hiện E1053) · HashMap.drain() (bucket state-gate
riêng) · `&mutable Vector` drain (borrow-receiver, hiện E1053) · O(N) cursor-drain perf · Deep-Clone ·
§15.6 Vector<Leaf?> · N1 widening · `!!` ForceUnwrap (Slice 2c — G gợi ý mũi kế).

→ [[campaign_iteration_slice1_2a]] [[campaign_typed_collections]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]]
