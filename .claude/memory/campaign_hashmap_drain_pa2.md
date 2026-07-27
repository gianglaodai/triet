---
name: campaign_hashmap_drain_pa2
description: "✅ ĐÓNG 2026-07-27 — HashMap.drain() LANDED qua PA-2 destructuring-only desugar (ADR-0089 §AMEND-2). PA-1 Tuple hạng nhất bị G đạp chết (729 site MirType). Bất biến: Tuple sống ở front, chết tại lower. Lần đầu move-out đồng thời heap-key × heap-value. 816a729, gate 0·clean·0·522·0."
metadata:
  node_type: memory
  type: project
---

## ✅ ĐÓNG — `816a729` (D) + docs (O co-sign). O✅/G✅/Giang✅ 2026-07-27

Gate `0·clean·0·522·0 CLEAN`. Fixture 511 → **522 file** (số hiệu 520-530).

## 🎯 RECON LẬT CẢ KHUNG CỦA CHÍNH NHÃN

Giang chốt *"Tuple lowering để xử lý HashMap.drain()"*. Nhãn defer cũ
(ADR-0089 §AMEND) ghi **2 bức tường**. Đo lại:

- **Bức tường 2 (không có enumerate-shim key-less) = KHÔNG PHẢI BỨC TƯỜNG.**
  Drop-glue `mir_lower.rs:1940` (free KEY) / `:2038` (free VALUE) / rehash
  `:6583` **ĐÃ walk toàn bộ `cap` key-less, lọc `state==1`** từ lâu. Cơ chế
  CÓ SẴN và đã được kiểm chứng bằng máu — chỉ chưa expose thành shim công khai.
  Nhãn đúng ở mức *shim*, sai ở mức *cơ chế*.
- **Bức tường 1 (Tuple) KHÔNG NÊN PHÁ — nên ĐI VÒNG.**

## 💀 PA-1 (Tuple hạng nhất) BỊ G ĐẠP CHẾT

`MirType::` bị match tại **729 site** (`mir_lower.rs` riêng có 29 match
exhaustive). Thêm 1 variant = **tự gieo lại họ bug "match exact, QUÊN
variant"** mà dự án VỪA tốn trọn một chiến dịch để quét (họ "quên `Nullable`":
6 thành viên, **2 nằm bên trong chính lưới an toàn**). Lại chạm **B-γ
multi-reg return** (defer vô hạn) + kề **B-β sub-8B** (đã đạp chết).

🔑 **Câu hỏi kiến trúc quyết định:** `for (k,v) in m.drain()` cần **HAI BIẾN
trong thân vòng**, KHÔNG cần một **GIÁ TRỊ tuple**. PA-1 xây kiểu hạng nhất
chỉ để lập tức phá ra làm hai — trả 729 site cho một trung gian không ai giữ
lại. G: *"tự châm lửa đốt nhà mình"*.

## 🔒 BẤT BIẾN: Tuple SỐNG ở front, CHẾT tại lower

`MirType::Tuple` = **0** toàn backend (O verify). PA-2 = **0 variant mới,
0-hit trên 729 site cũ**.

⚠️ **TIÊU CHÍ KIỂM SAI — O ăn đòn:** `grep -c Tuple` trần **KHÔNG** phải tiêu
chí. Lowerer **BẮT BUỘC** match `triet_syntax::Pattern::Tuple`
(`triet-lower/src/lib.rs:2036`) để destructure — đó CHÍNH LÀ thiết kế PA-2,
không phải vi phạm. **D bác tiêu chí proxy thô của O bằng số đo — D ĐÚNG.**
Tiêu chí duy nhất đúng: **`MirType::Tuple` = 0**.

## 🔑 CHUỖI 4 BƯỚC MOVE-OUT — một cờ `state` đóng CẢ BA tử huyệt

Shim `__triet_hashmap_drain_next` (`mir_lower.rs:7005+`), mirror
`__triet_hashmap_remove:6824`: surface K+V ra out-ptr → **zero key-cell** →
**`state→2`** → **`len--`** → trả `idx+1`.

Drop-glue **chỉ walk `state==1`** ⇒ ① move-out sound (tombstone miễn
double-free) · ② break-mid (đã-drain `2` bỏ qua, còn-lại `1` drop-glue dọn
nốt) · ③ container-survives (`len--` ⇒ drain trọn `len==0`, re-insert hợp lệ).

**Cursor O(N)** không rescan O(N²): `cap=1000,len=10` → **1000** vs **10.000**
lượt đọc state. Sound-stop `while idx<cap` kiểm ĐK **TRƯỚC** khi đọc byte ⇒
`cap==0` an toàn (fixture 525 canh).

## 🔑 QUY ƯỚC SENTINEL (G bắt ghi vào ADR)

| Sentinel | Giá trị | Nghĩa |
|---|---|---|
| `NULL_SENTINEL` | `i64::MIN` | **giá trị vắng mặt** (nullable PA-3c) |
| cursor-stop (mới) | **`-1`** | **hết slot để quét** |

Miền cursor luôn `≥0` ⇒ `-1` không đụng dải hợp lệ. **CẤM trộn hai khái niệm.**

## 🩸 O VERIFY MÁU — 3 mũi poison ĐỘC LẬP

| Mũi | Poison | Đo được |
|---|---|---|
| **P1** | `state→2` thành `1u8` | `drain_full` **9 vs 6** · `break_mid` **10 vs 8**, con trỏ lặp = **double-free THẬT** |
| **P2** | bỏ `len--` | `drain_full_leaves_len_exactly_zero` **3 vs 0** |
| **P3** | guard fail-**open** `if true` | 527-530 đỏ **+ fixture CŨ 510 đỏ lây**; 520-525 **không** đỏ |

**Lần đầu trong lịch sử dự án: move-out ĐỒNG THỜI heap-key (String) × heap-value
(String/Vector) ra khỏi cùng một bucket.** P1 là răng canh chính bãi mìn đó.
Counting teeth **dedup CON TRỎ** (`count==N` VÀ `dup==0`) — FREE-count đơn
thuần mù trước double-free (3 free có thể là 3 object HOẶC 2 object + 1 trùng).

## ⚔ HAI BÀI HỌC POISON

**P2 — "không đỏ" phải phân định (a)/(b) bằng ĐƯỜNG-CHẠM-ĐƯỢC.** Vòng drain
dừng theo `state` qua cursor, **KHÔNG theo `len`**; re-insert `cap=4` chưa
chạm ngưỡng resize ⇒ **(b) test yếu**, không phải (a) bất-khả-observable.
**D báo TRUNG THỰC rồi TỰ cắm thêm răng** đọc thẳng `len(m)` — không bịa mũi
giả cho nổ để qua cửa (đúng lối thoát O viết sẵn trong WO).

**P3 — "tháo guard" nghĩa là fail-OPEN, KHÔNG phải fail-closed.** D làm sai
hướng lần đầu (`if false &&` = siết chặt hơn ⇒ chứng minh 0), **tự phát hiện,
sửa `if true ||`, đo lại, báo cả hai lần**. Dưới poison đúng hướng các hình
refuse **không lọt** mà bị lowerer chặn bằng `LowerError` khác ⇒ **defense-in-
depth 2 lớp** (typecheck = mã đúng, lower = fail-closed cuối) — cùng kiến trúc
ADR-0088 Lane A.

## 🩸 O SAI 2 TIÊU CHÍ — D bác cả hai bằng số đo

1. **`grep -c Tuple` = 0/0/0** — proxy thô, sẽ reject chính thiết kế O ra lệnh.
2. **"gate mục tiêu 530 fixtures"** — nhầm **số hiệu cao nhất** (530) với
   **TỔNG SỐ FILE** (522), trong khi **chính O đã chạy `ls|wc -l` = 511 cùng
   phiên**. Dữ liệu bác O nằm sẵn trong tay O.

Cùng gốc **"hành động trước khi đo"** — nay lộ ra ở tầng *tiêu chí nghiệm thu*,
đúng vết luật 16. Kỷ luật "đo trước" vẫn **chưa thành phản xạ**.

## Fence lát 1 + nợ mở

**MỞ:** `K` ∈ {scalar, String} · `V` ∈ {scalar, String, Vector, HashMap}.
**REFUSE E1054:** pattern≠tuple-2 · tuple-3 · aggregate key/value · `V=Nullable`.
**Ngoài for-guard → E1015** (tiền lệ Vector 491, giữ for-guard-ONLY).

⚠️ **E1054 nay mang 4 NGHĨA** — ca pattern-shape vẫn in `key`/`value` dù nguyên
nhân là hình pattern. G tạm chấp nhận lát 1; tách sau nếu siết "một E-code một
hợp đồng".

**Nợ:** aggregate key/value drain (move-out key aggregate = ABI mới) ·
`V=Nullable` (drain qua out-param KHÔNG bọc `Nullable` nên lý thuyết an toàn
hơn `Vector<T?>` — **chưa đo** ⇒ giữ refuse) · tách E1054 · **PA-1 vẫn BỊ BÁC**.

[[campaign_iteration_slice2b_drain]] [[campaign_iteration_slice2d_borrow_drain]] [[campaign_adr0088_lane_a_nested_nullable]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]]
