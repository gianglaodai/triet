# B3 — Alias Analysis (Blueprint thăm dò)

**Spike B3.0 2026-06-10.** HEAD `11d11cf`.

## 1. Hiện trạng

### `places_conflict` — conservative=true

```rust
fn places_conflict(a: &Place, b: &Place, conservative: bool) -> bool {
    if a.local != b.local {
        return conservative;  // ← over-reject source
    }
    // Same base: field-level comparison...
}
```

### 2 caller

| Caller | Line | conservative | Ngữ cảnh |
|--------|------|-------------|----------|
| Borrow check | 514 | `may_alias` = true cho `&0`/`&-`, false cho `&+ mutable`/`BorrowExclusiveMutable` | E2440 exclusivity |
| Move check | 608 | `true` (luôn) | E2420 use-after-move |

### Over-reject: khi nào?

2 `let` binding KHÁC nhau, đều được borrow `&0`:
```
let a = new_value();
let b = new_value();
let ra = &0 a;  // loan on a
let rb = &0 b;  // places_conflict(loan_a, source_b, true) → true → E2440
```
Thực tế `a` và `b` là 2 allocation khác → borrow KHÔNG conflict. Nhưng `conservative=true` → từ chối.

### Over-reject: khi nào KHÔNG?

- `&+ mutable` (may_alias=false): S6 đảm bảo no-alias → 2 local khác = disjoint → không conflict. ĐÚNG.
- Move check (conservative=true): different base → always conflict. Có thể over-reject nếu move vào a mà borrow trên b (khác allocation). Nhưng move check rồi cũng cần alias info.

## 2. B3.0 Spike — đo over-reject thật

### Phương pháp

Dựng fixture `// ERROR: E2440` mô phỏng 2-borrow-khác-allocation → xem MIR có từ chối không.

### Kết quả

**Chưa có fixture nào bị over-reject.** Các fixture E2440 hiện tại (`20`, `77`, `79`, `94`, `95`, `98`) đều là conflict THẬT (cùng base, khác form). Không có case "2 allocation khác bị báo conflict".

### Kết luận B3.0

**B3 là tối ưu lý thuyết chưa có người dùng.** 0 fixture over-reject → 0 nhu cầu thực tế. YAGNI.

## 3. Khuyến nghị: DEFER

| Lý do | Chi tiết |
|-------|---------|
| 0 fixture over-reject | Không ai bị ảnh hưởng bởi conservative=true |
| Rủi ro soundness | Alias analysis sai 1 chiều = UB (data race). Khác B2 (gỡ trùng an toàn), B3 nới-lỏng → mỗi nới phải chứng minh |
| Chi phí cao, lợi ích 0 | Xây alias analysis (Point-to graph, escape analysis...) cho 0 lợi ích hiện tại |

**Đề xuất:** Chuyển B3 vào Nhóm E (CLEANUP) với ghi chú "mở lại khi có fixture over-reject thật". Giữ `conservative=true` như defense-in-depth — soundness > precision.

## 4. Nếu mở lại B3 sau này

### ⛔ ĐIỀU KIỆN TIÊN QUYẾT (G mệnh lệnh 2026-06-10 — KHÔNG ĐƯỢC VI PHẠM)
**PHẢI XÂY LƯỚI TEST ALIASING CỐ TÌNH GÂY LỖI (NEGATIVE TESTS) TRƯỚC khi nới `conservative`.**
Lý do: O probe `conservative=false` (nới tối đa) → 101/101 vẫn pass → **lưới fixture hiện
KHÔNG có case "2 ref khác-local CÙNG allocation phải conflict"** (alias thật). Nếu build alias
analysis SAI mà không có lưới này → UB (data race / UAF) lọt âm thầm, 0 test bắt. **Đụng B3 khi
chưa có negative-alias-test = một tội ác (G).** Lưới bắt buộc: fixture `// ERROR: E2440` với 2
local alias nhau (gán chéo / escape / cùng allocation) → phải bị từ chối. Có lưới đỏ TRƯỚC, mới
được nới conservative.

### Điều kiện mở (sau khi lưới tiên quyết đã có)

- Có fixture // ERROR: E2440 bị từ chối do conservative=true, nhưng thực tế 2 allocation khác
- User report: "code này lẽ ra pass mà bị báo lỗi"

### Thiết kế tham khảo

```
Alias analysis bảo thủ-an-toàn:
- 2 local TỪ 2 allocation site KHÁC → disjoint (provable)
- 2 local CÓ gán chéo (a = b) → may-alias
- 1 local ESCAPE (truyền vào hàm, store vào struct) → may-alias
- Default: may-alias (giữ conservative)
```

### Phân pha (nếu mở)

- B3.1: Alias analysis cho trường hợp đơn giản (2 fresh alloc, no escape, no cross-assign)
- B3.2: Escape analysis (tham số hàm, struct field store)
- B3.3: Cross-assign tracking

## 5. Quyết định chờ G

G cần quyết: defer B3 (YAGNI) hay vẫn build phòng xa? O nghiêng defer — 0 fixture over-reject, rủi ro soundness cao, nguồn lực cho C (feature gap) hợp lý hơn.
