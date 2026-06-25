---
name: feedback-implementer-choice
description: "Author delegates implementation decisions when they don't affect Triết language syntax. AI picks; author reviews only if language-visible."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: cce4839a-27d6-47a3-b80a-e44188e9d404
---

Author explicit 2026-05-30 (v0.9.x.jit.4): *"ồ tôi chỉ có thể đưa ra ý kiến nếu sự thay đổi đó ảnh hưởng đến cú pháp của ngôn ngữ. Nếu đứng ở vai trò coder mà điều này không ảnh hưởng đến cú pháp code thì hãy triển khai theo recommend của bạn"*.

**Rule:** AI decides implementation-internal choices autonomously when the decision doesn't change what Triết source code looks like or behaves like (from a user's perspective). Author intervenes for syntax/semantics decisions.

**Why:** Author is webapp dev with vision-driven role (per [[user_role_webapp_dev_visionary]]), not compiler engineer. Implementation backend choices (e.g. Cranelift JIT internals, ABI marshaling shape, AOT cache filesystem layout) consume his time without his expertise adding value. Explicit delegation lets AI move fast on the parts that don't need product-owner input.

**How to apply:**

1. **Before asking "which approach?", classify the decision:**
   - Affects Triết source code shape, runtime semantics, error visible to user, or feature scope → **ask author**.
   - Internal to a Rust crate (codegen, dispatch, optimization, ABI mechanics, file formats) → **AI picks**.
2. **Still write the rationale.** Even when AI picks, document in ADR or commit message which alternatives were considered + why this one. Author may review later.
3. **Recommend if you mention alternatives.** When presenting options to the author, lead with one explicit recommendation + reasoning. Don't pass undifferentiated menus.
4. **The "Recommended" marker.** Use it on the option you'd pick if author delegated. Helps signal where the implementer's-choice line falls.

**Examples from v0.9 where this applied:**

- v0.9.x.jit.4 builtin shim deferral (defer / 1-fn POC / full / bundle 4+5) — author said pick + execute. AI chose Option A (defer + diagnostic). NO syntax impact.
- v0.9.x.jit.6 AOT cache deferral — AI chose Option A again, applied [[feedback_cham_ma_chac_pattern]]. NO syntax impact.
- v0.9.x.jit.7+.8 collective defer — AI chose to fold into single commit. NO syntax impact.

**Examples where this DOESN'T apply (asked author):**

- v0.9.x.atomic.7 borrow expression scope cliff — affects Triết source syntax (`spawn_worker(&+ counter)`). Author chose Phương án A.
- ADR-0031 §2 operand grammar — affects what user can write. Author confirmed scope reduction.

**Token-efficiency note:** This delegation pattern means thread-spanning conversations stay focused on author-attention items. AI can implement + commit without round-trips on internal mechanics, only checking in for review or for language-visible decisions.
