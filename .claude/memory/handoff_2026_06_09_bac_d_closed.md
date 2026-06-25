---
name: handoff-2026-06-09-bac-d-closed
description: ★★ BẬC D CLOSED + Crusade A1/A2/A3 ĐÓNG. HEAD 58a8519 (2026-06-09). Kế tiếp: B1 Type System.
metadata:
  type: project
  originSessionId: handoff-bac-d-closed
---

# ★★ BẬC D CLOSED — Fat-Pointer ABI (ADR-0049)

**HEAD: `58a8519`** (main). **O+G ký duyệt 2026-06-09.**

## Cây commit Bậc D

| Lát | Commit | Mô tả |
|-----|--------|-------|
| 0 | `5fcf6e2` | Phase-0 spike + ADR §6 approved |
| 1 | `2da4fa4` | String i64→StackSlot 3-field |
| 2 | `d789a81` | Slot-Truth Edict (tombstone Move & Deinit) |
| 3a | `3003de4` | free(ptr,cap) 2-arg + universal String slots |
| 3b | `e0dcca5` | eq/contains/concat 4-arg bung field |
| 5 | `b8851ed` `e1f3dc1` | append + clear *mut FatStr writeback |
| 6.1 | `626390c` | param fat-String by-pointer |
| 6.2 | `9caa350` | return fat-String sret (Lối d) |
| **6.3+6.4** | **`d60eb9b`** | **Trảm heap len/cap + rút Lối B** |
| **Endgame** | **`9b28c54`** | **Fixture 100 — String round-trip 5-boundary** |

## Crusade commits (sau Bậc D)

| Crusade | Commit | Mô tả |
|---------|--------|-------|
| TODO | `a59b60b` | Dọn rác L26+L63 + phân loại nợ O+G (A–F) |
| **A1** | **`be37875`** | **is_propagated UAF fix — live_out thay blind skip** |
| **A2+A3** | **`d8e1ba9`** | **MIR verifier INV-4 + enum exhaustiveness E1026** |
| TODO | `08b0acd` | Mark A2+A3 done |
| Highlights | `58a8519` | Sync HIGHLIGHTS.md (O review) |

## Trạng thái cuối cùng

- **Gate**: 0 build warnings · 0 test failures · 99 fixtures · 208 clippy.
- **Bậc D đóng trọn**: Slot là chân lý duy nhất. Heap `[Header 8B][data…]`.
  Dây heap cắt hai chiều. Mìn fallback hoá lỗi-to-mồm.
- **A1 đóng**: is_propagated blind skip → live_out check. Fixture 101 (positive) + 102 (negative, có răng).
- **A2 đóng**: MIR verifier INV-4 — bắt block Unreachable được tham chiếu. 2 unit test.
- **A3 đóng**: Typechecker enum exhaustiveness — fire E1026 nếu thiếu variant. Fixture 103.
- **Persona đồng nghiệp D**: cập nhật Rule #7 (REFUSE OVER GUESS — không dán nhãn "future-proof" nếu chưa chứng minh bằng panic-probe).

## Kế tiếp — B1 Type System (Crusade #3)

B1 là con thú khác A-block: đại phẫu nền, bỏ string-match MIR (`ty == "String"`, `starts_with('&')`, `is_nullable_type` prefix-match...) thay bằng enum Type thật, migrate generated Type của schema vào typecheck.

**O khuyến cáo**: mở bằng khảo sát phạm vi trước khi code — grep toàn bộ điểm string-match đang sống (MIR + lower + jit + borrowck) để đo bề mặt va chạm. B1 đụng fallback-invariant Bậc D, A2 INV-4, và B2 borrowck merge → blueprint O+G trước khi gõ.

## Nợ treo

1. **concat sret** — backlog.
2. **Fallback-as-Err** — đóng tại đây. Nợ MIR-type-enum tách chiến dịch riêng.
3. **Chiến Dịch Trả Nợ còn**: B1 (Type System) → B2 (borrowck merge) → B3 (alias analysis) → C/D/E.

[[mentor_o_persona]] — persona ACTIVE
[[colleague_d_persona]] — Đồng nghiệp D persona (Rule #7 REFUSE OVER GUESS)
[[feedback_verify_semantics_before_asserting]] — bài học mẫu lặp 4 lần
