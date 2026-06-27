---
name: campaign_expected_type_propagation
description: "ADR-0072 Expected-Type Propagation in AST→MIR lowering — 🔒 SEALED+pushed origin/main 3d7618f. Giết mầm ung thư c.sig.return_type proxy toàn cục, thay bằng expected: Option<&MirType> tường minh. 3 slice. Mở hàm trả T?. ĐỌC nếu đụng lower_expr signature / OutcomeConstructor / NullLiteral / ~+/~0/~- constructor / nullable-return / if-match-block forwarding."
metadata: 
  node_type: memory
  type: project
  originSessionId: bb3e8b29-d3f8-402a-908d-36cd844a8e9a
---

**ADR-0072 — Expected-Type Propagation (🔒 SEALED 2026-06-27, push origin/main `3d7618f`, gate `0·0·303·0`).**

## Khởi nguồn = một chẩn đoán SAI trong sổ bàn giao
Sổ ghi blocker "match-arm bind heap payload move-out → `lowerer does not support Identifier`". O recon (probe matrix) chứng minh SAI hai tầng:
1. **Name collision:** hàm test tên `get` trùng builtin free-fn Vector/HashMap (`lib.rs:2220`); call 0-arg → `unsupported_expr(callee)` in `Identifier{name:"get"}`. Đổi `get`→`fetch` lỗi bốc hơi. **`match` move-out trên Outcome `T~E` VỐN ĐÃ CHẠY** (fixture 113/139/142).
2. **Kẻ thù thật:** hàm trả `T?` (nullable) không hạ được → `OutcomeAlloc on non-Outcome type`. **BÀI HỌC: verify-don't-trust cắt cả recon trong SỔ, không chỉ recon của người khác.**

## Khuyết tật gốc (mầm ung thư G gọi tên)
`OutcomeConstructor`/`NullLiteral` quyết đường-hạ (Outcome-StackSlot vs nullable PA-3c) bằng cách đọc `c.sig.return_type` — **biến TOÀN CỤC làm proxy**. Sai khi context cục bộ ≠ return type (let nullable, field nullable, hàm-trả-T?). 3 redirect bolt-on cũ (let `:1314`, struct-field `:2986`, `~0` is_null `:884`) chỉ là vá cục bộ bằng cách LỘT `~+` trước constructor. **KHÔNG có `NullableAlloc`** — nullable present=identity(scalar)/widening(aggregate), null=NULL_SENTINEL (đính chính framing G).

## Giải pháp (G chọn param tường minh, BÁC context-ẩn)
`lower_expr(expr, expected: Option<&MirType>, arena, c)`. 3 slice, mỗi slice O verify máu ĐỘC LẬP (byte-identical + poison đỏ + structural grep), G co-sign từng lát:
- **Slice 1** `c9a46e6` — thêm param, 61 site=`None`, byte-identical (MIR-diff rỗng toàn corpus, worktree baseline).
- **Slice 2** `2c900fb` — leaf-consumer đọc `expected` (fallback §2.5 chuyển-tiếp `unwrap_or(sig.return_type)`); wire 4 nguồn (body-tail/return/let-init/struct-field); đập 3 redirect (GIỮ widening block). Mở `T?`-return scalar (303/305). 2 poison `OutcomeAlloc on non-Outcome` đỏ. **Defense-in-depth: 2 guard (Nullable-arm + non-wrapper) — gỡ 1 vẫn đỏ.**
- **Slice 3** `3d7618f` — transparent forwarding `expected` xuống Block-tail/If-then-else/13 match-arm-body (KHÔNG scrutinee/condition); **gỡ sạch fallback §2.5**; **nhổ `c.sig.return_type` khỏi input constructor** (chỉ còn 4 nguồn return-position hợp pháp + 1 reference-form check); extract `emit_outcome_zero`. 306/307/308 mở (context≠sig), **309 negative khóa luật "untyped `let r=~+5` BỊ TỪ CHỐI"**, 157 annotated (semantic fix, ý ADR-0055 Bug A giữ). Diagnostic tổng quát (hết nói "~0 null" cho ~+/~-). 3 poison R-fwd đỏ.

## Bằng chứng đóng (kiệt tác)
157 UNTYPED (chạy qua fallback ung thư) vs 157 ANNOTATED (chạy qua nguồn tường minh) → **MIR byte-identical từng byte**. Thay tim, bệnh nhân không hay. Scope-extension D (8→13 arm, 2→4 nguồn) validate bởi byte-identical 299/299 (sai sẽ vỡ >1 fixture).

## Nợ chuyển tiếp (cờ đỏ)
🔴 **heap-nullable-return drop-glue** (`function f()->String?=~+ "hi"` compile+chạy nhưng CHƯA verify FREE==1/double-free). Fixture bonus 304 ĐÃ XOÁ (G: "không poison = false signal, không được nằm trong gate"). Cần **WO chuyên biệt** cắm poison drop-glue mới được mở. [[campaign_truc_b_heap_in_aggregate]]

## Nền đã sạch cho
`match call_returning_T?(){~+ s=>… ~0=>…}`, `if c {~+v} else {~0}`, block-final `{~+v}` ở mọi value-context. Capability Ł3 (ADR-0069 [[campaign_capability_luk3]]) vẫn treo.
[[feedback_verify_producer_before_consumer]] [[mentor_o_persona]] [[colleague_d_persona]]
