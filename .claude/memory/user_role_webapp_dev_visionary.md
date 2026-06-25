---
name: User — webapp dev, vision-driven, defers technical decisions
description: User là lập trình viên webapp, không có background system-level. Đưa tầm nhìn và mong muốn; ủy quyền toàn bộ quyết định kỹ thuật cho assistant.
type: user
originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---
User profile (từ self-disclosure 2026-05-09):
- Background: lập trình viên phần mềm chủ yếu webapp.
- Không có kinh nghiệm sâu về system languages, ABI, OS internals, compiler internals.
- Đưa được vision và requirements rõ ràng (e.g., "ngôn ngữ phải viết được OS").
- **Ủy quyền toàn bộ technical/architectural decisions cho assistant** với phạm vi: triển khai ngôn ngữ Triết.

**How to apply:**
- Không hỏi user "should we use witness tables or monomorphization?" — đó là quyết định của tôi.
- HỎI khi: user-facing UX, philosophy, scope, priorities, hoặc trade-off mà chỉ user biết (ví dụ: tên ngôn ngữ, cú pháp lựa chọn aesthetic).
- KHÔNG HỎI khi: implementation strategy, prior art selection, ADR content, kỹ thuật nội bộ.
- Tài liệu hóa quyết định ở ADR — user đọc được nhưng không phải duyệt từng dòng.
- Khi giải thích, dùng analogy với webapp (Java/Spring, npm packages, REST API) hơn là deep system internals — user sẽ hiểu "DLL Hell" qua "version conflict in node_modules" tốt hơn.
