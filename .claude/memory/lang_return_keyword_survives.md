---
name: lang_return_keyword_survives
description: "Quyết định ngôn ngữ 2026-06-17 — Triết GIỮ keyword `return` (không trảm). Đọc trước khi ai hỏi lại 'có cần return không'."
metadata: 
  node_type: memory
  type: project
  originSessionId: 2e8fd692-48b0-4f38-b76d-815d7e054b83
---

**2026-06-17 — G hỏi O "Triết có cần keyword `return` khi đã có `~+`/`~0`/`~-`?". G nghiêng về TRẢM HOÀN TOÀN return (expression-oriented thuần). O phản biện có máu → G HỦY phán quyết trảm. `return` SỐNG.**

Ngộ nhận phải bóc: `~+`/`~0`/`~-` là **constructor** (trục VALUE — tạo Outcome value, gắn discriminant Trit), KHÔNG phải control-flow. `return` là **control-flow** (điểm thoát + đưa value cho caller). Hai trục TRỰC GIAO — trio không thay được return.

Ba đòn O bác G (file:line, không suông):
1. **`~?` đã CHẾT** — G lôi toán tử propagate `~?` làm trụ đỡ "đã chiếm mặt trận early-exit", nhưng `~?`/`~:` bị khai tử ở commit `d6e8680` (Phase 14.5, ADR-0020 §3.7). E1030/E1031 DELETED (typecheck/error.rs:476), no OutcomePropagate node (parser tests.rs:778). G đánh trận với quân ma.
2. **`~->` (cơ chế SỐNG) DÙNG `return`** — fixture 115/116: `~+ (succeed() ~-> |e| return ~- e)`. SPEC §397: compiler suy ra MAP vs EARLY-RETURN mode **dựa vào sự hiện diện của `return`**. `return` = token phân biệt mode trong chính hệ Outcome. Trảm return = mở lại ADR-0020 §3.0 + mint token mới.
3. **"đã tiến hóa vượt return" là tương lai, không hiện tại** — nợ CFG Tail-Expr (ADR-0055) CHƯA wire (`function f()->Int{match…}` trả 0 sai, workaround `let r=match…; return r`); **136/174 fixture dùng return**. Early-exit hàm non-Outcome bằng if/else expression thuần → kim tự tháp lồng.

**Tách hai nghĩa "bỏ return" (G đã trộn):**
- **(i) bỏ return CUỐI HÀM (happy path):** CẢ HAI đồng thuận = chân lý Triết. Đường đúng = **đóng nợ CFG Tail-Expression (ADR-0055)** → chiến dịch riêng G sẽ mở. Tail-expr gánh giá trị cuối.
- **(ii) trảm return HOÀN TOÀN:** **HỦY BỎ chính thức** (lệnh G 2026-06-17). Giá quá đắt: va ADR-0020 đã ký + 136 fixture. Điều kiện tiên quyết nếu tái xét: phải có ADR mới thiết kế mode-inference của `~->` KHÔNG-dùng-`return`.

G chốt: "`return` không còn là rác C/Java — nó là chốt an toàn định hướng `~->` + gánh early-return hàm non-Outcome." Bài học G tự rút: ADR + đo đạc thực tế bóp chết lý thuyết suông, kể cả từ mồm G. [[mentor_o_persona]] [[reference_spec]]
