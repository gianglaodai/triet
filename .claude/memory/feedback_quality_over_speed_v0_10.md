---
name: feedback-quality-over-speed-v0-10
description: "User explicit 2026-05-30 mid-v0.10 — speed không phải concern; chất lượng code là contract chính. Tôi (AI) là technical-quality owner, tác giả depend vào năng lực của tôi để verify."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 5ad1339c-d4d2-4c07-9823-7d76ba88d258
---

**Quality bar cho v0.10 (author explicit 2026-05-30 sau v0.10.x.interp.1):**
> "đừng vội vàng, tốc độ của bạn hoàn toàn sẽ kịp. Vấn đề là chúng ta phải đảm bảo chất lượng code nhé. Đến đoạn này, tôi phải depend vào năng lực của bạn rồi"

**Why:** Tác giả không có background compiler engineering ([[user_role_webapp_dev_visionary]]), không thể visual-review từng dòng Rust unsafe / ABI marshaling / borrow checker enforcement. AI là technical-quality owner cho phần implementation v0.10. Trust được trao kèm trách nhiệm — không phải tự do hành động vội mà là tự do siết chuẩn cao hơn.

**How to apply:**

1. **Pre-commit self-audit là MANDATORY** mỗi sub-task. Đọc lại diff fresh trước khi commit; hỏi "senior engineer sẽ approve PR này không?". Surface bất kỳ code smell nào (dead-code workaround, misleading error variant, missing SAFETY comment, untested error path) trước khi commit. Phương án: fix ngay hoặc declare deferred với reason.

2. **Plan trước code cho sub-task có unsafe / ABI** (jit.1 + jit.2 cụ thể). Viết plan ngắn (5-10 bullet) trước khi gõ Rust:
   - File touched + dòng LOC ước tính
   - Test gate trước khi commit (happy + error path)
   - Per unsafe block: SAFETY invariant + audit comment shape (ADR-0032 §5 mandatory)
   - Defer items + lý do explicit

3. **Refuse over guess** (VISION §6) khi ADR chưa cover. Nếu ADR-0032/0033 không locked về 1 detail, **hỏi tác giả** trước khi gõ code; không silent-pick + ship.

4. **Test coverage bar cho mỗi sub-task:**
   - Happy path: ít nhất 1 test cho mỗi op/feature mới
   - Error path: ít nhất 2 test (typical wrong-args, edge cases)
   - Integration: nếu có cross-crate path, viết integration test trong `tests/`
   - Bench / proptest cho ABI-critical paths (per ADR-0032 §7)
   - VM↔JIT parity test cho mỗi của 43 shims (per ADR-0032 §7.2)

5. **Tier-down honestly:** nếu một sub-task LOC ước tính bị blow-out (>2× plan), pause + report tới tác giả với 3 options: (a) tier-down scope, (b) tách thành 2 commits, (c) defer phần. KHÔNG silent inflate scope.

6. **Verify end-to-end** trước khi mark `[x]`: chạy `dao run` với example liên quan, không chỉ unit test. Nếu UI-adjacent (CLI output, error message format), check format khớp ADR-0027.

**Precedent (post-mortem v0.10.x.interp.1):** sau commit `be9e535` tôi tự phát hiện 2 code smell (const _: redundancy + compare_exchange dùng sai error variant). Author không catch được vì là tiếng Rust nội tại. Đây là exactly the failure mode this feedback prevents. Cách xử lý: thừa nhận + fix followup commit `[v0.10.x.interp.1b]`.

Cross-ref: [[feedback_proactive_audit]] (audit window khi gần phase close); [[feedback_stability_over_speed]] (architectural decisions có ADR, không ship đại); [[feedback_implementer_choice]] (delegation precedent — tự do với responsibility).
