---
name: doc-highlights-and-ternary-seeds
description: docs/HIGHLIGHTS.md tồn tại — điểm sáng ngôn ngữ + backlog ý tưởng tam phân (Phần III gieo mầm)
metadata: 
  node_type: memory
  type: project
  originSessionId: f7825e3e-8c6b-4106-bea6-d5d9cbe21ecf
---

`docs/HIGHLIGHTS.md` (tạo 2026-06-08) = tài liệu định vị "điểm sáng Triết so với
ngôn ngữ khác", viết cho lập trình viên (không phải compiler theory). Cấu trúc:

- **Phần I (✅ kiểm chứng được hôm nay):** null bẩm sinh (T?), Ł3 refinement
  Trilean!, số học trap-on-overflow, cú pháp AI-first, an-toàn-bộ-nhớ-kiểu-Rust-bỏ-`<'a>`
  (S6 borrow params + E2440 + suy lifetime E2400). Mỗi mục trích fixture đang xanh.
- **Phần II (🎯 design/chưa rebuild):** ABI bẩm sinh, capability-Trit, CAS, IR-tách-backend,
  S6-bỏ-`unsafe`/GC/`Weak`/BYOS.
- **Phần III (🌱 gieo mầm — ý tưởng tam phân chưa cam kết):** bộ lọc gimmick
  ("chỉ nhận domain có điểm trung tính thật") + tầng 1: `compare()->Trit`
  (LOCKED [[future-comparable-trait-and-monad-gap]] ADR-0038), **rounding-không-thiên-lệch**
  (cắt cụt = làm tròn gần nhất, phần phân số ∈[-½,½]), **tri-state config inherit=0**
  (thay Option<bool>); tầng 2: BitNet b1.58 narrative; tầng 3: signum/merge/voting/clamp.
  Ngoài trục tam phân: **học từ Odin — SoA (`#soa[N]T`) + array programming** (tách
  layout khỏi cách viết code; cache/SIMD). Giao thoa: SoA ternary weights nối mầm #4.
  ⚠ Mầm XA — phụ thuộc native multi-field layout (CHƯA CÓ, mọi value còn i64); ADR
  đầu phải chốt ABI-visibility (intra-package only) + aliasing element-ref với S6.

Phần III có **bảng 4 cửa dependency** (thời điểm xem xét mỗi mầm): Cửa A=ngay/thuần
core (#2 rounding, signum); B=Trait system mở (#1 compare→Trit); C=Capability rebuild
(#3 tri-state config = tổng quát CapabilityLevel 4-state); D=sau native multi-field
layout (SoA Odin → BitNet). ⚠ phase≠build order; Trait system + native layout CHƯA
có phase doc trong spec/ (phase 1-6 chỉ design cho cái đã/đang làm; phase 7 namespace
defer). Đường chính HIỆN TẠI = **Chiến Dịch Trả Nợ** (Bậc D fat-pointer ĐÃ ĐÓNG
`58a8519` — HIGHLIGHTS đã sync, dòng 337 cũ "Phase-1 String fat-pointer" đã sửa);
mầm là "tới cửa thì xem", đừng kéo SoA/BitNet (Cửa D) lên sớm.

**MẦM MỚI O đề xuất 2026-06-09 (chưa author-duyệt vào HIGHLIGHTS):** *Outcome
discriminant LÀ Trit.* `T~E`/`T?~E` đã có 3 nhánh `~+ ok / ~0 absent / ~- err`
đối xứng quanh `~0` — tam phân BẢN ĐỊA của Triết, không ép. Insight = cùng họ #1
+ mục-2-Trilean: "discriminant là số, không phải enum" → fold chuỗi Outcome bằng
số học Trit (min-Trit ≡ Ł3-AND "fail nếu bất kỳ fail") thay match lồng. Cửa B/C
(chờ Outcome rebuild — hiện guarded Err, chưa producer). ĐÁNG ghi Tầng 1 cạnh #1.
O verify Phần I 9/9 fixture đúng (43/48/49/07/74/76/94/81/06 — chạy thật, mã khớp).

**Why:** author muốn lưu lại để quay lại sau. **How to apply:** khi author hỏi
tiếp về "điểm sáng", "áp dụng tam phân vào đâu", hoặc "khi nào làm X" — đọc file
này trước, đừng brainstorm lại từ đầu. Vùng động được NGAY mà không tạo nợ = Cửa A
(viết ADR chốt tính chất #2 rounding, KHÔNG implement vội).

⚠ Nhãn trung thực bắt buộc: đừng bán Phần II/III như đã-có. Phần II từng chạy ở
compiler cũ (đã xóa), chưa rebuild. Khi viết, không bán "borrow như Rust" như
điểm sáng — đó là parity; điểm sáng là cái Triết *bỏ được* (`<'a>`, unsafe, GC).
