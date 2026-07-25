---
name: campaign_str0_coverage_and_triage
description: "✅ ĐÓNG 2026-07-25(c) — TRIAGE toàn sổ nợ (chôn 2 zombie + 1 phantom) + &0 String read-borrow coverage (len/eq/concat). origin/main 5dd2aeb, gate 0·clean·0·458·0."
metadata: 
  node_type: memory
  type: project
  originSessionId: 12b89f5b-2d0d-472a-bab6-868c1c7615ed
  modified: 2026-07-25T07:54:58.625Z
---

## ✅ ĐÓNG — 3 commit PUSHED (O+G ký, 2026-07-25(c))

```
5dd2aeb  docs(todo): close &0 String coverage, log WO-JIT-Print debt
31fd2d6  feat(track-c): &0 String read-borrow overload coverage — len/eq/concat
4904c55  docs(mentor): dọn G-state — chôn zombie Enum? param + phantom &+ T, reclassify N1, fix fixture 177
```
Gate `0·clean·0·458·0`, fixtures 455→458 (462/463/464).

## 🔑 BÀI HỌC LỚN NHẤT: NHÃN BACKLOG KHÔNG ĐÁNG TIN — RECON-TRƯỚC-WO CỨU HAI LẦN

Giang chọn front "Enum? param SIGILL 132" từ sổ nợ. O recon-trước (verify-don't-trust):
- **Enum? param = ZOMBIE**: đã đóng `ccb8db3` (WO-NullableEnumParamABI, 2026-07-19, teeth 419-427).
  O dựng worktree pristine `564f0f7` → tái hiện exit 132; HEAD → chạy đúng (1/99/42). Debt sống ở
  forward-list `MENTOR_G_STATE.md` nhưng đã đóng trong code + TODO.md.
- Giang lệnh **triage-tươi TOÀN sổ nợ** → lộ thêm **borrow-params `&+ T` = PHANTOM**: ADR-0022 §4.1
  `&+` KHÔNG phải borrow mà là unique-OWNER, pass = MOVE; move-in heap ĐÃ CÓ qua plain param
  `f(v: Vector)`; share cross-thread = ADR-0026 **BYOS FROZEN**. Không mở khóa năng lực gì. CHÔN.
- Triage cũng thấy: **N1 widening** nay = refuse `E1120` sạch (feature ADR-0065, KHÔNG hố miscompile);
  comment fixture 177 "tail-expr fat-struct return SIGILL" = STALE (verify plain-free-fn expr-body →30 exit 0).
- **KHÔNG entry nào còn là hố soundness/crash sống.** Mọi thứ genuinely-open = feature-completeness gap.

🩸 **Vết O:** ban đầu nói quá "G-state đầy zombie"; grep chính xác thấy nhẹ hơn. Read hiển thị bản
working-tree lệch HEAD (bất thường) → O **revert về HEAD thật rồi tự tay tái áp từng dòng** (auditable,
không ship thay đổi không rõ nguồn vào file boot G — tiền lệ `3417c4f` clobber). Verify-don't-trust áp lên
chính output của mình.

## FRONT &0 String COVERAGE (G chọn Option 2: len+eq+concat, Phương án A)

**Ma trận hố** (probe thật CHECK+RUN): Vector/HashMap `&0`-read phủ KÍN; hố CHỈ ở String, lẻ:
`length/contains/is_empty(&0 String)`✓ nhưng `len(&0 String)`→E1041, `concat/eq(&0 String)`→E1003.

**Bề mặt vá = typecheck-only (env.rs), C-shim ĐÃ nhận `&0 String`** (chứng minh sống: `length(&0)`→5,
`contains(&0)`→9):
- **len**: thêm 1 `declare_overload("len",(ref_string)->Integer)` cạnh khối ADR-0059 C.2 (`env.rs:747`);
  arm `"len"|"length"` (`triet-lower/src/lib.rs:2685`) đã strip Reference → ZERO lowering.
- **eq**: `declare`→`declare_overload` + 3 combo `(ref,owned)/(owned,ref)/(ref,ref)`; JIT class `bung_fields`
  đã có nhánh `is_reference()` → ZERO JIT.
- **concat**: cần MỞ RỘNG JIT. Class `concat_sret` (`mir_lower.rs:3968-3979`) chỉ đọc `struct_slots`,
  thiếu nhánh Reference-fallback như `bung_fields` (`:3993-4012`). Fix = **MIRROR nguyên văn** nhánh
  `is_reference()` (`use_var`→`load {ptr,len}@0/@8`) vào else của concat_sret. Đồng nhất marshaling 2 class.

**Phương án A (explicit overloads)** — G BÁC coercion `&0 T→T` ngầm ("rác câm typecheck").

**print/println LOẠI** (O dùng thẩm quyền G trao "nếu cần"): KHÔNG có JIT shim (`grep __triet_print`=rỗng;
chạy owned String cũng `callee print not found`). Thêm `&0` overload = NO-OP (dời E1003→JIT-not-found).
→ ghi nợ **WO-JIT-Print** (front I/O riêng: dựng stdout shim + wire).

## 🦷 TEETH MÁU (string_ref_overload_free_counting.rs, --test-threads=1, dedup con trỏ)
- Healthy: `len(&0 s)` FREE=1, `eq(&0,&0)` FREE=2, `concat(&0,&0)` FREE=3 (2 mượn + 1 result alloc), dup=0.
- Poison SHIM (chứng minh counter sống): leak-shim→FREE 0; dup-shim→FREE 2×N/dup>0.
- **O tự poison ĐỘC LẬP 2 tầng** (cp-snapshot /tmp, md5 khớp, KHÔNG git checkout):
  gỡ overload len→462 E1041; gỡ nhánh `is_reference` concat_sret→464 exit 4 "concat: String arg without slot".
  Cả hai load-bearing.

## ⚖ VAI (phiên sạch)
- **D=Sonnet 5 subagent**: WO-1 (len+eq) DỪNG đúng LUẬT 4 ở concat (concat_sret bế tắc — báo O, revert sạch,
  KHÔNG tự nới sang mir_lower.rs). WO-2 (concat) hoàn tất khi G mở scope, mirror đúng chỉ định KHÔNG phát minh.
  0 vết bịa. Commit WIP không push.
- **O**: recon-trước-WO (chôn 2 zombie+1 phantom trước khi đốt công), ra 2 WO, verify máu độc lập cả 3 lát
  (tự gate, tự poison 2 tầng), squash commit gọn, push + ls-remote xác nhận.
- **G**: ký kết liễu; BÁC coercion; đòi MIR-dump concat + hỏi print/println (O trả lời có bằng chứng → loại).

## 🔴 NỢ MỚI + CÒN TREO (chờ G+Giang chốt mở)
- **WO-JIT-Print** (MỚI, định nghĩa rõ): `print`/`println` thiếu JIT shim `__triet_print`/`__triet_println`
  stdout side-effect. Cần dựng shim + wire lowerer + JIT declare. Front I/O, KHÔNG thuộc `&0 coverage`.
- method-return `Struct?`/`Enum?` (E1100 ConstructNotYetLowered) · get_ref V=Nullable (E1003, lowerer
  `&0 Nullable`) · §15.6 `Vector<Leaf?>` chạy (feature, drop-glue nhạy) · Deep-Clone (campaign lớn) ·
  drain (ADR Iteration) · `&0 Enum` tiêu thụ (basic borrow đã chạy, "tiêu thụ" concern chưa rõ) ·
  mir_lower panic Nhóm B/C (B bia-mộ, C defer `D-JIT-OOM`).

[[campaign_shim_meta_spof_adr0085]] [[campaign_typed_collections]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_teeth_never_git_checkout]]
