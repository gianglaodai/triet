---
name: future-comparable-trait-and-monad-gap
description: "Phiên design 2026-06-05: (1) Comparable trait / compare()->Trit — design LOCKED tại ADR-0038, defer chờ Trait system; (2) map/flatMap — ĐÃ ĐÓNG TRỌN tại ADR-0039: ?+> (map+flatMap auto-flatten), ?0> bị giết, ?-> cấm E1041."
metadata:
  node_type: memory
  type: project
  originSessionId: cbfcad37-8830-40cb-a053-1a01523fea6d
---

Phiên design 2026-06-05 (Mentor O + author), hai câu hỏi ngôn ngữ. Chi tiết đầy đủ
trong repo — memory này giữ phần KHÔNG có trong repo + pointer.

## 1. So sánh 3 trạng thái → ĐÃ CHỐT, ghi `docs/decisions/0038-comparable-trait-deferred.md`
- `Comparable` là **trait** (author xác nhận: Triết chắc chắn làm Trait, không
  Interface — hiện CHƯA có trait system: `Item` chỉ Function/Struct/Enum, lexer
  không có keyword `trait`/`impl`).
- `compare() -> Trit` (KHÔNG enum Ordering — i64 discriminant phản bội bản sắc
  tam phân; tên Ordering đã bị atomic chiếm). Tổng thứ tự only; unknown ở lại
  với operator `==`/`<` Ł3.
- **Defer chờ Trait system, KHÔNG build built-in tạm** (Phương án A + bài học
  dead-field). TODO.md có section "Deferred — design locked".
- Đừng mở lại tranh luận này — đọc ADR-0038 trước.

## 2. map/flatMap (Monad) cho `T?` — KẾT LUẬN: gần như đã có sẵn, MỘT gap mở
Author hỏi "cú pháp nào thay map/flatMap của Monad cho T?". Trả lời sau khi
verify SPEC: **Triết ĐÃ thiết kế xong họ này** (ADR-0020, SPEC.md §Outcome
operators quanh dòng 385-407):
- `expr ~+> |val| body` = Functor **map** (SPEC ghi nguyên văn), `~0>` null
  default, `~-> |err| body` transform error.
- **flatMap/bind** = `~-> |e| return …` EARLY-RETURN mode — compiler suy MAP
  vs bind theo sự hiện diện của `return`. Không cần tên "flatMap".
- `T?` thuần: `?.` optional chaining + `?:` default (SPEC.md quanh 339-342).
- `~?`/`~:` cũ đã deprecated, lexer refuses (ADR-0020 §3.7).

**GAP ĐÃ ĐÓNG (2026-06-05, cùng ngày):** chốt tại
`docs/decisions/0039-nullable-operator-family.md` (đề xuất Mentor G, author
duyệt, Mentor O verify):
- **`?+>`** = map + flatMap hợp nhất cho `T?` (auto-flatten `U?`→`U?`, kế thừa
  flatten của `~+>` ADR-0020 §3.1:379 — KHÔNG sinh `T??`).
- **`?0>` bị giết** — `?:` RHS chốt là Expression (Block + Return) nên `?0>` thừa.
- **`?->` cấm vĩnh viễn** — E1041 NullableHasNoErrorState; lexer reserve token
  để diagnostic đẹp.
- Đối xứng tiền tố: họ `?` cho T?, họ `~` cho Outcome — `~+>` KHÔNG áp lên T? thuần.
Bài học phiên này: Mentor O báo động sai "(b) flatten phá đối xứng" vì không mở
lại ADR-0020 §3.1 (flatten đã có sẵn) — verify-trước-khi-phán áp cho cả mentor.
Ngược lại bắt được Mentor G bịa "Throw" (Triết không có exception — chỉ panic).
Advisor ngoài (Mentor G) cũng phải qua verify như author.

Lưu ý chung: các op trên là design-locked nhưng **CHƯA implement** trong backend
rewrite (Bậc A mới tới scalar/struct/enum). SPEC là nguồn đúng khi tới lúc lower.
