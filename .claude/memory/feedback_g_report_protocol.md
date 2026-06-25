---
name: feedback-g-report-protocol
description: "Khi O duyệt 'gửi G được', O PHẢI soạn sẵn gói báo cáo đầy đủ cho G — author chỉ chuyển tiếp. Lý do: G chỉ thấy lát cắt cuối → suy diễn lấp chỗ trống → 5 lần dữ liệu sai."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 98556ed7-368a-4e9e-8d7a-6c651fbf342e
---

**Author yêu cầu (2026-06-07):** quy trình thực tế là author iterate với Mentor O
nhiều vòng, rồi chỉ gửi G đoạn trao đổi CUỐI — G không thấy diễn biến giữa chừng.
Đó là gốc rễ của chuỗi "dữ liệu sai" phía G (`triet-mir` ×3, `shims.c`, "59
fixtures" — đều là suy diễn lấp chỗ thiếu input, không phải G ẩu). O đã quy nhầm
cho G trong sổ — entry này sửa lại chẩn đoán.

**Why:** người review thiếu context thì điền khoảng trống bằng suy diễn; chất
lượng review của G phụ thuộc trực tiếp vào gói input mà phía này gửi.

**How to apply:** mỗi lần Mentor O kết luận "gửi G được", O soạn luôn GÓI BÁO CÁO
HOÀN CHỈNH trong cùng message (author chỉ copy-chuyển):
1. Mốc cây: HEAD + chuỗi hash từ lần G thấy gần nhất.
2. Gate 4 hàng (build / test / clippy location-set / fixtures) — số O tự đo, nguyên văn.
3. Diễn biến từ lần G review trước: findings từng vòng + cách sửa (cả vòng đỏ).
4. Delta thiết kế so với cái G đã biết hoặc đã ra lệnh — nêu thẳng (tiền lệ wrap→trap).
5. Câu hỏi cụ thể cần G trả lời, đúng scope chữ ký của ông ấy (layout/ABI/codegen).

KHÔNG ĐỔI: thư G về vẫn đối chiếu số-trong-thư vs số-trong-cây trước khi chép
vào hồ sơ ([[mentor_o_persona]] nghi thức 10) — input đầy đủ giảm lỗi, không
miễn kiểm chứng.
