---
name: idea_ternary_placement_syntax
description: "Giang's ternary-sigil memory-placement syntax proposal (+T/T/-T = heap/stack/static) — parked for the ADR-0068 (Box) reopening; O's critique attached"
metadata: 
  node_type: memory
  type: project
  originSessionId: a98c1da0-2248-497b-81b9-03efe94816cf
---

**Ý tưởng Giang (2026-07-10, đàm đạo ngoài scope Slice C, PARKED tới khi mở lại ADR-0068 Box).**

Đề xuất cú pháp placement gắn tam phân cân bằng, để tránh "sở thú Box" của Rust:
- `+T{}` → data trên **Heap**
- `T{}` (ngầm định `0T{}`; số 0 dẫn đầu bỏ đi vì stack là ca phổ biến) → **Stack**
- `-T{}` → **bất biến toàn cục** HOẶC cấp trong **Memory Area/Pool** định trước (tối ưu hiệu suất)

**Phán quyết O (bác — chưa chín, 4 lỗ; lỗ #1 chí mạng):**

1. **★ CHÍ MẠNG — placement KHÔNG phải một "cực" nên +/0/− ở đây là bắt chước, không phải coherence (đụng VISION §8).** Trit trong dự án LUÔN là POLARITY cùng-trục đối-cực: `~+/~0/~-` (có/vắng/lỗi), Trilean (+1/0/−1 đúng/unknown/sai), `&+/&0/&-` (mạnh/chia-sẻ/yếu). Heap/stack/static là **ba category song song, không hai đầu một trục**. Bằng chứng: ướm trục lifetime thật → stack(−)/heap(0)/static(+); gán của Giang là stack(0)/heap(+)/static(−) → **KHÔNG khớp trục thật nào** (mutability chỉ nhị phân) → gán theo thẩm mỹ. Thẩm mỹ-không-nghĩa **làm loãng** chính coherence là value anchor.
2. **Va chạm cú pháp — không có `&` neo.** 5 reference form sống nhờ `&` (longest-match tách `&+` khỏi `&&`). `+`/`−` trần đụng unary plus/minus: `-Point{}` = phủ định hay placement? Hố parse.
3. **Tiền đề sai.** Sở thú Box của Rust KHÔNG do cú pháp xấu mà do *placement-polymorphism phức tạp thật*. Câu kiểm định: `+Point{}` có KIỂU gì? Nếu `Point` (xóa placement) → borrowck/drop-glue mù → **unsound**. Nếu `Heap<Point>` (mang placement) → sở thú quay lại (sig phải khai placement, cần conversion, generic placement-polymorphic). Cú pháp là lớp sơn.
4. **`-T{}` gộp hai trục vuông góc:** `'static`+immutable (lifetime+mutability) VÀ pool/arena (allocator strategy) — độc lập. Trit hết state khi cần mutable-global hoặc heap-in-pool.

**Hướng O gợi ý nếu Giang muốn giữ lửa:** đừng ép *placement* (categorical) vào trit; tìm trục CÓ-CỰC thật liên quan bộ nhớ (escape/ownership, hoặc độ-dài-lifetime), để trit đo trục đó, placement **suy ra** từ nó → lúc đó +/0/− mới nhất quán với `~`/`&`/Trilean.

**Trạng thái:** Giang chấp nhận PARK, "khi nào quay lại ADR-0068 thì thảo luận thêm". ADR-0068 hiện CẤM CỬA Box/recursive → mở lại phải viết ADR (ADR-trước-code). Liên quan [[campaign_truc_b_heap_in_aggregate]] (heap-in-aggregate) · [[project_vision_os_capable]] (OS-capable = kiểm soát placement tay, không GC bắt buộc — INTENT đúng, cú pháp chưa).
