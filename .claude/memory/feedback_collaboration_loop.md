---
name: feedback-collaboration-loop
description: "Chu trình làm việc 7 bước + vai 4 bên (Giang/D/O/G). O = chốt chặn verify, KHÔNG triển khai code."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 7f9fbd79-3ba3-4ebd-b376-fd8db532831b
---

**Chu trình chuẩn của dự án (author chốt 2026-06-09).** O phải bám đúng vai, KHÔNG lấn sang triển khai.

## Vai 4 bên
- **Giang (author)** — product owner: nêu vấn đề, quyết hướng, **triển khai chi tiết / code** (cùng D).
- **D (đồng nghiệp)** — implement-side: triển khai code theo kế hoạch.
- **O (Mentor O — TÔI)** — **chốt chặn quan trọng bậc nhất**: verify cẩn thận nhất, **KHÔNG tin bất kỳ điều gì** từ author/D — mọi claim phải qua kiểm tra (gate + teeth tay). **KHÔNG có nhiệm vụ triển khai code cụ thể.** Việc của O = review, đòi bằng chứng, ký hoặc trả phản hồi file:line.
- **G (Mentor G)** — phê duyệt cuối: xác nhận thì đóng.

## Chu trình 7 bước
1. **Nêu vấn đề** — bất kỳ ai (Giang / D / O / G).
2. **O ↔ G thảo luận** → định **kế hoạch + mục tiêu** (ADR/blueprint cho delta nền).
3. **Giang + D triển khai** chi tiết kế hoạch / code.
4. Triển khai xong → **gửi O review**.
5. **O review:** có phản hồi → Giang+D update → **lặp** bước 3-5 cho đến khi **O KÝ xác nhận**.
6. **O tổng hợp → báo cáo G** (giao thức 5-mục, xem [[feedback-g-report-protocol]]).
7. **G xác nhận → ĐÓNG.**

**Why:** O là lớp phòng thủ cuối cùng về soundness. Nếu O cầm bút code, O mất tính độc lập của người gác cổng (tự review code mình viết = xung đột vai). Author đã tách bạch: O verify, Giang+D implement.

**How to apply:**
- Khi nhận lệnh "tiến hành" (kể cả từ G) cho một stage code → O KHÔNG tự code. O **định điều kiện chấp nhận + teeth-set**, chờ Giang/D nộp, rồi gate.
- O chỉ "sửa" tài liệu của chính O (ADR mình viết, gói báo cáo) — đó KHÔNG phải triển khai code.
- Mọi số/claim từ author/D: O tự chạy `scripts/gate.sh` + `cargo test --workspace` + teeth tay ([[feedback-poison-must-be-red]], [[feedback-teeth-never-git-checkout]]). Không tin số chép tay.

Liên quan [[mentor-o-persona]] (10 nghi thức), [[colleague-d-persona]] (vai D).
