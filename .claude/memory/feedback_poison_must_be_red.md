---
name: feedback-poison-must-be-red
description: "LUẬT THÉP G (2026-06-09) — mọi test \"fix bug cấu trúc\" phải bị poison-đỏ trước khi nhận; không tin tên test."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 7f9fbd79-3ba3-4ebd-b376-fd8db532831b
---

**LUẬT THÉP (G chính thức hoá 2026-06-09, sau bài học B1a S1 Vòng 4):** Với mọi PR/commit tuyên bố "fix bug cấu trúc / structural-fix", quy trình duyệt của O **BẮT BUỘC** thêm bước: **poison logic cốt lõi → test không đỏ → REJECT thẳng mặt author.**

**Why:** B1a S1 có test `mirtype_structural_fixes_nullable_vec_misclassification` — tên đúng (bảo vệ ordering-bug `Vector<Integer>?`), nhưng input là `Integer?` (= Nullable(Integer)). Integer không bao giờ là vec → assert `!is_vec()` đúng một cách vô nghĩa, KHÔNG chạm case nguy hiểm. O poison `is_vec` match-xuyên-Nullable → 44 pass (không cắn). Test xanh nhạt nhẽo vô giá trị hơn code không test — nó tạo ảo giác an toàn cho đúng cái bug nó tự nhận đã vá.

**How to apply:**
1. Đừng bao giờ tin TÊN test. Bắt nó chứng minh bằng máu (màu đỏ của panic).
2. Teeth tay: cp snapshot /tmp TRƯỚC ([[feedback-teeth-never-git-checkout]]), poison logic cốt lõi, chạy test chỉ định — PHẢI đỏ; khôi phục bằng cp, KHÔNG git checkout.
3. Test structural-fix phải dùng đúng input tái tạo bug (vd Nullable(Vector), không Nullable(Integer)).
4. Áp cho cả claim của author/D "đã teeth verify" — O tự dựng lại, không tin.

Liên quan [[mentor-o-persona]] (verify-don't-trust), [[feedback-verify-semantics-before-asserting]] (author đoán ngữ nghĩa rồi mã hoá vào test).
