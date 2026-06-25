---
name: feedback_verify_semantics_before_asserting
description: Mẫu lặp — author khẳng định ngữ nghĩa compiler bằng phỏng đoán rồi mã hóa vào test; mentor phải đòi chứng cứ thực nghiệm.
metadata: 
  node_type: memory
  type: feedback
  originSessionId: cbfcad37-8830-40cb-a053-1a01523fea6d
---

Trong phiên rewrite 2026-06-04, author lặp lại **ba lần** cùng một lỗi: khẳng
định một điều về ngữ nghĩa compiler/ngôn ngữ **bằng phỏng đoán**, rồi mã hóa
phỏng đoán đó vào test hoặc comment:

1. **Outcome pass-through** — đoán compiler sai (đúng thật, may).
2. **Fixture 20 borrow_fail** — đoán "double `&0 mutable` SHOULD reject E2440";
   thực tế hai mutable borrow KHÔNG dùng là HỢP LỆ theo NLL → compiler ĐÚNG khi
   chấp nhận. Test asserted compiler có bug nó không có → suýt drive false-positive regression.
3. **Fixture 21 drop_while_borrowed** — đoán "latent E2450"; thực tế cấu trúc đó
   về bản chất không thể là E2450 (borrow unused + Copy scalar), VÀ E2450 chết
   end-to-end vì lowerer không emit Drop nào.

**2026-06-09 — LẦN THỨ 4 (A1 is_propagated):** author dán nhãn "future-proof /
MIR không tạo được pattern Drop-before-deref" cho guard is_propagated — hai lần,
cả hai đều sai. O dựng probe MIR chứng minh bom SỐNG reachable: nested-scope
return-borrow tạo Drop(_0) trước length(_2) thật → UAF lọt im lặng. Suýt ship
bom vì nhãn sai.

**Why:** author không phải compiler engineer; trực giác về NLL/S6/Outcome chưa
chắc. Khi đoán sai và mã hóa vào lưới an toàn, test SAI sẽ kéo dự án về phía bug
(người sau "sửa" compiler để thỏa test sai = tạo regression thật).

**How to apply (cho cả hai):**
- Mỗi khi author assert một điều về ngữ nghĩa (borrow rule, type rule, "should
  fire EXXXX"), ĐÒI chứng cứ thực nghiệm TRƯỚC khi chấp nhận: chạy `triet-driver`,
  hoặc diff với example đã chứng minh, hoặc trích SPEC §10.
- **Quy tắc G §2 (REFUSE OVER GUESS — mở rộng 2026-06-09):** Trước khi gọi một
  guard/code-path là "dead", "future-proof", "unreachable", hoặc "MIR không tạo
  được", PHẢI tự tay chèn `panic!("Unreachable")` / `Err(JitError::Unsupported)`
  vào đó và chứng minh không test nào chạm. Nếu không chứng minh được → đó là
  LỖ HỔNG (Hole), không phải Dead Code. Không nhận chữ
  "latent/should/probably/future-proof" trần trụi.
Xem [[feedback_stability_over_speed]].
