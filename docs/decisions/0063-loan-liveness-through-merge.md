# ADR-0063 — Borrowck: point-level loan liveness at Drop (UAF qua block-merge)

- **Status:** 🔒 LOCKED — G ký duyệt 2026-06-19. Khởi thảo Mentor O 2026-06-19, recon empirical (3 phương án implement-thử-đo-revert).
- **Date:** 2026-06-19
- **Khởi thảo:** Mentor O (deep-recon Bug A heap-nullable → đào ra UAF pre-existing ở Expr::If/match reference-arm).
- **Liên quan:** [ADR-0046](0046-propagated-loan-liveness.md) (PropagatedLoan return-borrow bounded by dest liveness — ADR này TINH CHỈNH cái Drop-check của nó) · ADR-0045 (Reference = Copy) · CFG-tail Lát 1 (`159fd68`, Bug A block-tail drop escape).

---

## 1. Context — UAF qua block-merge, borrowck câm

`let r = if c { let inner = "hello"; id(&0 inner) } else { … }; length(r)` → **trả 5, KHÔNG E2450** = use-after-free chạy êm. Borrowck bỏ sót. Fixture 102 (cùng pattern, plain block) bắt E2450; bản if-wrapped lọt.

**MIR (then-arm + merge):**
```
Call id(_2) → [_3]      ; PropagatedLoan source=_1(inner) dest=_3
Drop(_1)                ; block-pop drop inner
_4 = move _3            ; If-merge: result _4 = then_val _3
… length(_4) …          ; dùng _4 sau khi _1 chết → UAF
```

**Rễ (checker.rs:780-784, ADR-0046 Drop-check):**
```rust
loan.source.local == *l && (!loan.is_propagated
    || liveness.blocks[block.0].live_out.contains(&loan.dest))
```
Drop-check dùng **block-level `live_out`**. `_3` (loan dest) bị `_4 = move _3` tiêu thụ NGAY trong block → KHÔNG ∈ `live_out` → check trượt. Borrow vẫn sống qua `_4` (live_out) nhưng loan trỏ `_3` → miss.

**Vì sao Block (fixture 102) bắt được:** Lát 1 cho reference-tail **direct-return** (không Assign-to-merge); `_3` LÀ block result, dùng `length(_3)` ngoài block → `_3 ∈ live_out` → E2450. If/match **merge BẮT BUỘC Assign** `_4 = move _3` (CFG hội tụ 2 nhánh) → `_3` chết trong block → miss.

## 2. Phương án bị loại (recon empirical — KHÔNG đoán)

> Framing ban đầu (G): "loan-follow qua reference Assign — Duplicate loan (vì Reference Copy) không Retarget". **Recon BÁC cả hai** bằng thực nghiệm.

- **(a) Duplicate loan ở Assign handler** (dest==source → copy loan với dest mới): **FAIL empirical** — headline vẫn 5. Lý do timing: `Drop(_1)` đứng TRƯỚC `_4 = move _3` trong dataflow order; lúc Drop xử, duplicate chưa xảy ra; loan vẫn dest=_3, vẫn miss. Retarget cùng bệnh.
- **(b) Point-level liveness NAIVE** (dest dùng-sau trong block, TÍNH cả `Drop(dest)`): **2 false-positive** — fixtures 84/101 (return-borrow hợp lệ) nổ E2450 oan. Vì `Drop(msg)` đứng trước `Drop(r)` ở scope-end → `Drop(r)` bị tính là "r dùng sau".

## 3. Decision — point-level READ-after-Drop liveness ở Drop-check

Drop-check bổ sung điều kiện: loan dest **được ĐỌC** (không phải Drop) ở một statement SAU trong CÙNG block:
```rust
let dest_used_after = body.blocks[block.0].statements[stmt_idx+1..].iter().any(|s| match s {
    Assign{source,..} | Borrow{source,..} | GetDiscriminant{source,..} => source.local == dest,
    BinaryOp{left,right,..} => left.local==dest || right.local==dest,
    _ => false,   // Drop(dest) KHÔNG phải use — dest đang chết
});
has_active_loans = loan.source.local == *l && (!loan.is_propagated
    || live_out.contains(&loan.dest) || dest_used_after);
```

**Bất biến khóa:** *Đọc một reference SAU khi borrowed-source của nó bị Drop (trong cùng frame) = UAF — luôn E2450. Drop chính reference đó (cùng chết) = an toàn.* → quy tắc KHÔNG có false-positive **by construction**: không code hợp lệ nào đọc ref sau khi source chết.

**Vì sao đây đúng chỗ (không phải lowerer, không phải loan-follow):**
- Fix nằm ở **Drop-check borrowck**, construct-AGNOSTIC → phủ If + match + MỌI merge tương lai bằng MỘT điểm, KHÔNG đụng lowering (G outline "guard If/match lowering" = 3 chỗ + vỡ merge).
- `live_out OR read-after-same-block` = point-level đầy đủ: cross-block escape (live_out: terminator/successor) + same-block-consume (read-after). Bù đúng khe ADR-0046 bỏ.

## 4. Empirical evidence (cây thử, đã revert)

| Phương án | Headline If-ref | Regression 204 + workspace |
|---|---|---|
| (a) Duplicate loan | ❌ vẫn 5 | — |
| (b) Point-level naive (Drop=use) | ✅ E2450 | ❌ 84/101 false-pos |
| **(c) READ-after, Drop loại** | ✅ E2450 | ✅ **204/204 + workspace 0 FAILED** |

Clean-tree confirm: UAF=5; với fix=E2450 → load-bearing.

## 5. Teeth bắt buộc (khi implement)
- **Headline fixture:** If-reference-arm UAF → E2450. Poison: gỡ `dest_used_after` → trả 5 (UAF về) → RED.
- **Regression cứng:** 84/101 (return-borrow) GIỮ pass (không false-pos); 102/20/21/24 (E2450/borrow) GIỮ đúng; full 204 + workspace.
- **match-arm:** cùng Drop-check nên cùng được vá; UNVERIFIED runtime (match-literal limitation chặn repro) — ghi rủi ro, KHÔNG claim test giả.

## 6. Consequences
- **Tích cực:** bịt UAF class (ref đọc sau source-drop) cho mọi merge; 1 điểm sửa borrowck; 0 regression đo được; KHÔNG đụng lowerer/loan-model (ít rủi ro hơn loan-duplicate).
- **Chi phí:** Drop-check thêm O(statements-còn-lại) scan/Drop — bounded, chấp nhận.
- **Đóng băng:** chỉ same-block read-after; cross-block đã do live_out. Nếu sau này cần point-level toàn diện (per-statement liveness) → ADR riêng.
- **Đính chính ADR-0046:** Drop-check của nó (block live_out) là xấp xỉ; ADR này khóa thêm same-block read-after như điều kiện liveness hợp lệ.

## 7. Chữ ký
- O: ✅ (recon empirical, 3 phương án đo máu, fix grounded MIR + 0-regression)
- G: ✅ (ký duyệt 2026-06-19 — ADR mới đè ADR-0046 [không amendment, lịch sử không xóa]; match-arm giữ UNVERIFIED minh bạch; fix point-level READ-after-Drop ở borrowck Drop-check, không lowerer/loan-follow)
