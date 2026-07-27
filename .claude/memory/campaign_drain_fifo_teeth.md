---
name: campaign_drain_fifo_teeth
description: "✅ ĐÓNG 2026-07-27(f) — WO-Drain-FIFO-Teeth: khắc hợp đồng FIFO cho .drain() + khóa thứ tự cascade E1056/E1054/E1057. G BÁC O(N) cursor-drain VÔ THỜI HẠN (cạnh bb2/bb3). 6 fixture 531-536, ADR-0089 §AMEND-4. 9d59892+04cb5d3, gate 0·clean·0·528·0."
metadata: 
  node_type: memory
  type: project
  originSessionId: f8391b98-1c93-4cb6-a8cf-8e5d66f4073c
  modified: 2026-07-27T17:33:54.746Z
---

## ✅ ĐÓNG — `9d59892` (fixtures) + `04cb5d3` (ADR §AMEND-4). O✅/G✅/Giang✅ 2026-07-27(f)

Gate `0·clean·0·528·0`. Fixture 522 → **528**.

## 🎯 KHỞI ĐIỂM: Giang chốt lane "O(N) cursor-drain cho Vector" → O RECON RỒI TỰ BÁC CHÍNH MÌNH

O khuyến nghị lane này với lý do "mirror shim cursor proven của HashMap.drain()".
**Recon lật ngược khuyến nghị của chính O:**

1. **`pop_front:6210` B2 `ptr::copy(data+stride, data, new_len*stride)`** = memmove
   toàn đuôi MỖI lần pop ⇒ drain N phần tử = **O(N²)** — THẬT.
2. 💀 **Cursor của HashMap KHÔNG mirror được.** HashMap có **state byte mỗi slot**
   ⇒ drop-glue lọc `state==1` = tombstone per-element miễn phí. Vector buffer chỉ
   `{len@0, cap@8, data@16}` — **không có ô trạng thái nào**. Nhãn "mirror proven"
   của O SAI, O rút.
3. 🔑 **Bất biến đang gánh soundness:** `buffer[0..len)` = đúng tập sống **tại mọi
   thời điểm giữa hai bước** ⇒ mọi exit edge sound **MIỄN PHÍ**. O(N²) chính là
   **giá đang trả** cho bất biến đó.
4. 💀 **BÃI MÌN quyết định — `return` giữa vòng là exit edge RIÊNG.** Đo trong
   `fn drain_it` của fixture 534:
   ```
   bb2: { Drop(_1) Drop(_0) Drop(_5) Return(_1) }   // return-mid — CÓ Drop(_5)
   bb3: { Drop(_1) Drop(_0)          Return(_1) }   // exit ext  — KHÔNG Drop(_5)
   bb4: { If(_4) → +:bb3, -:bb2 }
   ```
   **HAI exit edge, TẬP DROP KHÁC NHAU** ⇒ epilogue đặt tại `ext` bị `return`-mid
   **bỏ qua hoàn toàn** ⇒ drop-glue đạp ô đã move-out = **double-free**.

## ⚖ G RULING: HOÃN VÔ THỜI HẠN O(N), FIFO là hợp đồng khắc đá

- **V-C (LIFO, đổi `pop_front`→`pop`) BÁC** — đổi ngữ nghĩa quan sát được.
- **V-D (đảo buffer rồi pop đuôi) BÁC** — survivor của break-mid bị đảo.
- **V-A/V-A′ (cursor+epilogue) BÁC** — đòi đục `Stmt::Return` của lowerer để
  epilogue chạy trên MỌI exit edge. *"Không đem tính đúng đắn của toàn bộ đường
  return ra đánh cược để trả một món nợ hiệu năng chưa ai kêu."*
- 🔑 **O(N²) là NỢ HIỆU NĂNG, KHÔNG phải lỗ soundness.** Tôn chỉ 3.

## 🩸 PHÁT HIỆN PHỤ: CORPUS MÙ THỨ TỰ — và O tự đính chính claim của mình

O tuyên *"lật FIFO→LIFO thì KHÔNG test nào đỏ"*. **SAI.** O tự poison
(`lib.rs:2639` `pop_front`→`pop`) → **1/522 đỏ**: `490_drain_break_continue.tri`.

Nhưng đỏ **do TAI NẠN**, không do thiết kế: 490 kiểm break/continue bằng điều kiện
khớp giá trị (`if x == 100 { break }`), LIFO đẩy `100` lên đầu ⇒ break tức thì ⇒
`sum=0`. **Sửa hằng số trong 490 là mất lớp bảo vệ mà không ai hay.**

Phần lõi đứng vững: **6 fixture liên quan thứ tự (486/487/488/505/506/509) XANH
100%** dưới cú lật. `509` mù vì **mọi String đều `length == 1`**.

🦷 **Bài học: test ĐỎ cũng phải hỏi TẠI SAO nó đỏ trước khi tin nó là lính gác.**

## 🦷 RĂNG (O pre-poison TRƯỚC khi soạn WO — luật 21)

Oracle **position-weighted** `acc = acc*10 + v`, **CẤM tổng cộng dồn** (tổng mù thứ tự).

| Fixture | Hình | EXPECT | Dưới poison LIFO |
|---|---|---|---|
| 531 | owned `Vector<Integer>` [1,2,3] | 123 | **321** ✅ |
| 532 | owned `Vector<String>` len 1/2/4 (heap move-out) | 124 | **421** ✅ |
| 533 | `&0 mutable` + **break-mid** + đọc survivor | 1024 | **4012** ✅ |
| 534 | `&0 mutable` + **return-mid** + đọc survivor | 1024 | **4012** ✅ |
| 535 | `HashMap<KP,Integer?>` + `for x in` (sai CẢ 3 trục) | E1056 | key-trước-pattern → **E1054** ✅ |
| 536 | `HashMap<KP,Integer?>` + `for (k,v) in` (sai key+value) | E1054 | value-trước-key → **E1057** ✅ |

🔑 **534 = fixture ĐẦU TIÊN của toàn corpus chạm `return` giữa drain-loop** — chính
cạnh `bb2` đã bác phương án O(N). Từ nay có răng canh.

Poison Lane 1 làm **531-534 đỏ ở TẦNG HARNESS** (corpus 1 đỏ → 5 đỏ), không chỉ tầng
driver. Lane 2 hai mũi **orthogonal**: mũi A chỉ 535 đỏ (510/527/528 vẫn xanh ⇒ 535
là lính gác DUY NHẤT cạnh pattern>key); mũi B chỉ 536 đỏ.

## ⚔ VẾT O TRONG PHIÊN (ba lần, đều bị phép đo bắt)

1. Khuyến nghị lane O(N) → tự bác sau khi đo `bb2`/`bb3`.
2. "Corpus mù thứ tự, 0 test đỏ" → thực 1/522.
3. Suýt báo "nhánh `is_struct_widening` zero coverage" khi corpus **không in gì** —
   **vắng output ≠ xanh**: tiến trình đã chết (SIGILL) mang theo dòng `test result`.
   Luật 15 cứu.

⚔ **Vết O thứ tư — nhãn `bb9` sai nguồn:** O dán số block từ probe `/tmp` của mình
(receiver owned, trong `main`) vào WO; D chép lại. 533/534 hình khác nên số khác
(`bb2`/`bb3`). **O trả D sửa, và bản sửa TỐT HƠN bản gốc** (bằng chứng nằm trong
corpus, tự kiểm chứng được, thay vì trỏ vào probe `/tmp` sẽ biến mất).

## ⚖ D: 0 vết, 1 lần lệch có lý

**LUẬT 5 duyệt:** D đặt §AMEND-4 thành section độc lập cuối ADR thay vì chèn vào
preamble Slice 2b — đúng khuôn §AMEND-2/§AMEND-3, và nội dung phủ CẢ 2b lẫn 2d.
Raw gate đủ ngay vòng 1 (sắc lệnh hạ tầng có tác dụng). Tự dump MIR kiểm chứng
finding của O **trước** khi sửa, không chép mù.

## Nợ ghi sổ theo phán quyết G

- `len()` thiếu overload `Vector<String>` — backlog, **CẤM đụng** trong campaign này.
- Fixture `490` = lớp bảo vệ vô tình — **CẤM sửa** (surgical).
- O(N) cursor-drain — **hoãn vô thời hạn**, không mở lại tới khi có ADR chứng minh
  soundness trên MỌI exit edge.

[[campaign_hashmap_drain_pa2]] [[campaign_iteration_slice2b_drain]] [[campaign_iteration_slice2d_borrow_drain]] [[campaign_aggregate_move_tombstone]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]]
