---
name: feedback-tiered-opus-deepseek-workflow
description: "Quy trình chia tầng model Opus(thiết kế+test) / DeepSeek(code cơ học) để tiết kiệm token. Đọc khi viết handoff doc, IMPLEMENTATION_CHECKLIST, hoặc khi quyết định delegate task nào."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 81b50c42-7059-43be-aa72-d23ea0203e32
---

Tác giả dùng **2 model qua Claude CLI để tiết kiệm token**: **Opus** (đắt, thông minh) + **DeepSeek** (rẻ, kém hơn). Quyết định 2026-06-01. Luật meta ở **`HANDOFF_PROTOCOL.md`** (repo root); mỗi task có `IMPLEMENTATION_CHECKLIST.md` riêng do Opus viết; escalation ghi `ESCALATION_LOG.md`.

**Why:** chi phí Opus cao, tác giả ngân sách hạn chế. DeepSeek triển khai được nếu đặc tả đủ cứng + có harness nghiệm thu khách quan (dự án này CÓ: `cargo test`/`clippy -D warnings`/`jit_tier_down_audit`/value-parity test).

**How to apply (khi tôi là Opus):**
- **QUY TẮC VÀNG: Opus viết TEST nghiệm thu, DeepSeek chỉ viết CODE để pass.** Không để model yếu viết cả hai (nó sẽ co-adjust cho xanh → test vô nghĩa). Điểm mù lớn nhất: bug nguy hiểm (vd double-free phiên cross-call) KHÔNG bị test ngây thơ bắt — phải Opus suy luận ownership rồi viết test mới. → refcount/lifetime/unsafe/ABI/ADR/IR-change = **OPUS-ONLY**; chỉ delegate việc lặp-pattern có oracle sẵn (vd các opcode-slice agg.2/3 cùng khuôn shim).
- **DeepSeek phải escalate NGAY** (không đợi 3 lần) khi: cần `unsafe`, đổi ABI/signature/wire/IR, spec mơ hồ, đụng ADR Locked, hoặc câu hỏi memory-safety. Tripwire **3 lần fail test → STOP + log + tác giả chuyển Opus** (cấm hacky-pass: `#[ignore]`/`#[allow]`/`--no-verify`/nới assert/sửa expected — `CLAUDE.md` đã cấm).
- **Kinh tế:** viết checklist bất-khả-xâm-phạm cũng tốn token Opus → chỉ lời khi 1 checklist phủ NHIỀU slice lặp. Slice lẻ độc nhất: Opus tự làm rẻ hơn.

**Trial 1 (cross-call.b, commit `6987115`): PASSED CLEAN.** DeepSeek transcribe đúng spec (mirror `translate_boxed_call`, đảo box↔unbox), test paste verbatim, 2 clippy warning tự sửa đúng cách (không `#[allow]` né), 0 vi phạm prohibition. Opus re-verify độc lập khớp 100% (test pass / clippy sạch / 1676 workspace / audit 1622). Kết luận: quy trình hoạt động khi (a) spec có pseudocode chính xác + drop-order tường minh, (b) có sibling-pattern để mirror, (c) test do Opus viết sẵn. Scratch files (checklist + handoff report) xóa sau merge; chỉ `HANDOFF_PROTOCOL.md` ở lại.

**Trial 2 (TypeTag::Opaque, commit `fdc727d` bởi DeepSeek+Antigravity): code VỮNG nhưng VI PHẠM ranh giới + THIẾU test đúng.** Task IR-shape (thêm TypeTag variant + .triv v8 bump + ADR-0036) = **lẽ ra OPUS-ONLY** (§8) nhưng được giao đi. Opus review (`1240f35`): impl đúng (map_type/is_composite_tag/boundary_class/serde/self-host lockstep nhất quán, vá luôn bug disc-11 Atomic reader), NHƯNG (a) **không có test value-parity cross-mode chạy RUNTIME** — chỉ serde round-trip + "audit compile được"; vùng rủi-ro-nhất (Opaque đi qua biên + refcount) chưa execute+so-VM dưới malloc tripwire; (b) gộp `Unit` vào PassThrough với lý lẽ SAI (`map_type(Unit)=I8`≠i64-ptr boxed) — an-toàn-do-Cranelift-verifier-bắt + mong manh. Opus bù: thêm test runtime + siết `Unit`→`None` + sửa ADR §4. **BÀI HỌC CỦNG CỐ: với refcount/IR/memory-safety, kể cả khi delegate impl, Opus PHẢI tự viết test EXECUTE+value-parity TRƯỚC. "Audit compile được" + "test pass" KHÔNG chứng minh memory-safe — double-free/mis-marshal lọt qua test ngây thơ. Đúng điểm mù protocol cảnh báo.**

Liên quan [[feedback_stability_over_speed]], [[feedback_explicit_strictness]] (cấm hacky ops), [[feedback_quality_over_speed_v0_10]] (Opus = technical-quality owner).
