---
name: handoff-2026-06-11-muic-adr0059
description: Mũi C ĐÓNG (C.1+C.2) — ADR-0059 stack-borrow &0 heap Vector/HashMap + vá Generic return-bind; &+ phong ấn YAGNI. HEAD 8be0263.
metadata: 
  node_type: memory
  type: project
  originSessionId: 3ff940f3-c92e-4084-9b38-c8e2a2aa3a3d
---

# Mũi C ĐÓNG TRỌN (C.1+C.2) — ADR-0059. HEAD `8be0263`, gate 0·0·163·201

**2026-06-11.** Sau chuỗi Outcome 0052→0058. Mũi C = stack-borrow `&0` heap Vector/HashMap
+ vá nợ Generic return-bind. `&+` StrongFrozen phong ấn YAGNI (ADR-0059 §5).

## Chuỗi commit (verify-don't-trust, O tự đo từng cái)
| Commit | Việc | Trạng thái |
|---|---|---|
| `03b0655` | ADR-0059 doc (O viết, G commit nguyên văn) | ✅ |
| `bf668fd` | **C.1** Generic arm `lower_type`+`lower_type_simple` (Vector<Integer> return-bind) | ✅ O teeth (poison→`len() on type ?`) |
| `002daca` | fix trùng số fixture 105→166 (lỗi G đặt trùng `105_e2420_branch_move`) | ✅ |
| `c17bd96` | ADR-0059 §8 đính chính teeth + get-scope | ✅ (G ⏳ co-sign §8) |
| `8be0263` | **C.2** `&0` overload len/get Vector/HashMap + get lower-fix | ✅ O CODE ACCEPT |

## Bài học quy trình (3 sự cố G+D, O bắt hết)
1. **G phá cadence ở C.1:** G tự code+commit KHÔNG qua O-teeth-trước. O teeth HỒI TỐ (poison
   Generic-arm → 105/166 regress `len() on type ?`, lưới gate đỏ 1/161). Mã may đúng, nhưng
   commit-trên-niềm-tin = đúng thứ verify-don't-trust cấm. Luật: teeth TRƯỚC commit.
2. **G đặt trùng số fixture 105** → O bắt, đổi 166. Quy ước: số mới = max+1 (D check
   `ls fixtures|grep -oE '^[0-9]+'|sort -n|tail -1`).
3. **D overclaim nhãn crash ở C.2:** D chạy fixture 167 dưới poison-608 → exit **132 (SIGILL)**,
   KHÔNG có dòng double-free, rồi dán nhãn "Double-free, tcache abort, SIGABRT". Sai. 132=SIGILL
   (trap số học), 134=SIGABRT (double-free). 167 làm `n+m`: callee free buffer→`len(xs)` đọc
   rác→`2+rác` overflow→trapnz SIGILL nổ TRƯỚC double-free-Drop. **O tự ép probe tối giản
   (no post-borrow arith) → exit 134 + `free(): double free detected in tcache 2` SẠCH.** Cơ
   chế Vector ĐỒNG NHẤT String (tiên đoán O đúng). Bài học D: **đọc chữ ký crash trước khi gọi
   tên** — 134+"double free detected" mới là double-free; 132 là SIGILL.

## Kiến thức kỹ thuật chốt (cho mặt trận sau)
- **Borrow-param-no-free là HAI lớp độc lập:** (a) lower KHÔNG push_owned ref-param
  (`lib.rs:621-626`); (b) JIT Drop handler **type-gated** — bỏ qua local kiểu `Reference`.
  Poison MỘT lớp bị lớp kia bắt (poison `lib.rs:624` push_owned-guard → exit 0, KHÔNG máu).
  **Chỉ poison type-classification `lib.rs:608` (strip Reference→owned) mới defeat cả hai →
  double-free 134.** (ADR-0059 §8 ghi; bản gốc ghi nhầm 624.)
- `get` lower (`lib.rs:1939`) **không** strip reference như `len` (1733-1737); `is_vec()`=
  `matches!(Vector)` không xuyên Reference. C.2 đã thêm strip cho `get`.
- `&0 String` borrow đã chạy từ trước (fixture 77/84/100); wire backend chung (call-site
  stack_addr, shim nhận pointer-to-slot) — Vector/HashMap dùng lại nguyên.

## Còn treo
- ADR-0059 §8 G co-sign (G ⏳→✅) — pattern như ADR-0058 §9.
- TODO.md cập nhật Mũi C đóng.
- `&+`/`&-` StrongFrozen/Weak: phong ấn YAGNI (ADR §5), mở ADR riêng khi có use-case 2-owner
  + ObjectHeader refcount runtime.
- Mặt trận kế (chờ G/author chốt): nợ khác trong TODO.md.
