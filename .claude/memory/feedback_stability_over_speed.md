---
name: Stability over speed — kỷ luật quyết định
description: User explicitly đặt stability/certainty cao hơn delivery speed cho các quyết định kiến trúc Triết. Quyết định chậm có ADR > ship đại rồi sửa.
type: feedback
originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---
**Rule:** Mọi quyết định kiến trúc lớn cho Triết phải:
1. Có tài liệu hóa (ADR ở `docs/decisions/`).
2. Tham chiếu prior art cụ thể (Unison, Mojo, Pony, Swift, Genode...).
3. Liệt kê alternatives đã consider và lý do reject.
4. Không bị áp lực ship nhanh.

**Why:** User nói rõ (2026-05-09): "Tôi không cần một giải pháp triển khai nhanh, chúng ta đang làm một thứ điên rồ, chúng ta muốn cho ra một thứ giúp đảo lộn thế giới, một ngôn ngữ nhanh, an toàn, nhưng quá trình triển khai ngôn ngữ thì nên là những quyết định chắc chắn và an toàn, ổn định nên được đặt lên cao nhất."

**How to apply:**
- Trước khi commit kiến trúc mới, viết ADR với context/decision/alternatives/consequences.
- Khi đứng giữa "feature đẹp nhanh" và "feature ít hấp dẫn nhưng đặt nền tảng vững" → chọn nền tảng.
- Khi đứng giữa "phát minh giải pháp riêng" và "adopt prior art tested" → chọn prior art (trừ khi prior art mâu thuẫn với balanced ternary identity của Triết).
- Pace timeline scaled theo 5-10 năm cho v3.0. Không hứa hẹn timeline ngắn.
- Giải thích cho user nếu một feature cần thời gian dài — họ chấp nhận và ưu tiên chất lượng.
