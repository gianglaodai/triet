# ADR-0081 — Get-Borrow-Mutable from Container (`get(&0 mutable c, k) → (&0 mutable V)?`)

- **Status:** ❄️ **FROZEN / DEFERRED (Mentor G lệnh 2026-07-04).** WO A2 HỦY. Ném sang **Cụm D (Phase 3: Ownership — sub-path reassign)**. Mở lại CHỈ KHI core xử lý được `deref-assign` (`*ref = new_val`) + cập nhật handle an toàn (drop-in-place qua con trỏ). Lý do đóng băng: §7 dưới.
- **Date:** 2026-07-04
- **Deciders:** Author (Giang) · Mentor O · Mentor G
- **Ghi chú:** Phân tích §1-§6 GIỮ NGUYÊN (có giá trị khi mở lại). Kiến trúc mặt-borrowck (Q1: `returns_borrow_form`; lõi exclusive-loan conflict cả READ) đã đúng — vấn đề là API sẽ VACUOUS trên core functional-mutate hiện tại, không phải sai thiết kế loan.
- **Supersedes / extends:** ADR-0079 (get-borrow READ-ONLY) — this is the mutable twin.
- **Related:** ADR-0022 (S6 5-form reference), ADR-0059 (`&0` stack-borrow), ADR-0077/0078 (typed containers).

> **Đây là ADR-lite: nó KHÔNG mở campaign.** Mục tiêu duy nhất là trả lời 2 câu
> soundness G đặt ra và chốt PHẠM VI P1 trước khi D được cấp WO. A1 (get-borrow
> READ generic-V) đi song song bằng WO thẳng, KHÔNG cần ADR — vì nó không chạm
> lõi borrowck (loan vẫn read-shared, propagated). A2 cần ADR vì nó cắm thẳng vào
> tim borrow-checker: loan phải **exclusive**.

---

## 1. Bối cảnh

ADR-0079 mở read-side borrow: `get(&0 c, k) → (&0 V)?`, loan **read-shared**
(`ReferenceForm::BorrowReadOnly`, `is_propagated: true`) trên toàn container.
Nhiều read đồng thời hợp lệ; chỉ **mutate-while-borrowed** mới E2440.

A2 muốn mở twin **mutable**: `get(&0 mutable c, k) → (&0 mutable V)?` — lấy một
tham chiếu **ghi được** vào slot của phần tử V bên trong container, để mutate
V tại chỗ (vd `push` lên inner Vector borrow ra được) mà không phải move V ra
rồi nhét lại.

## 2. Hai câu hỏi soundness của G (và trả lời)

### Q1 — Bằng cách nào `checker.rs` biết callee trả read hay mutable reference?

**Hiện trạng (đã recon):** `BuiltinShimMeta` (`triet-mir/src/lib.rs:1006`) chỉ có
`returns_borrow_of: Option<usize>` — encode **arg nào** bị borrow, KHÔNG encode
**form**. Form bị **hardcode** ở `checker.rs:1173`:

```rust
state.active_loans.insert(Loan {
    source: real_source,
    dest: ret_temp,
    form: ReferenceForm::BorrowReadOnly,   // ← HARDCODE. Đây là điểm A2 phải sửa.
    is_propagated: true,
    ...
});
```

**Quyết định:** thêm trường `returns_borrow_form: ReferenceForm` (hoặc bool
`returns_mutable_borrow` nếu muốn tránh dep `triet-mir → triet-syntax`; encoding
là implementer's-choice, KHÔNG phải quyết định kiến trúc) vào `BuiltinShimMeta`.
- `__triet_{hashmap,vector}_get_ref` → `BorrowReadOnly` (giữ nguyên, byte-compat).
- shim MỚI `__triet_{hashmap,vector}_get_mut_ref` → `BorrowExclusiveMutable`.
- `checker.rs:1173` đọc `meta.returns_borrow_form` thay hardcode.

JIT: shim get_mut_ref **giống hệt** get_ref về codegen — ói ra con trỏ slot
zero-copy. Sự khác biệt read/mutable là **thuần borrowck**, không phải codegen.
(Đây là lý do A1 thuần env.rs còn A2 thuần borrowck-core — hai trục vuông góc.)

### Q2 — Mutable borrow qua slot: được WRITE-BACK (thay nguyên con V) hay chỉ MUTATE IN-PLACE?

Con trỏ trả về trỏ vào **slot 8B** giữ handle/fat-ptr của V bên trong container.
Hai năng lực khác nhau:

| | In-place mutate | Write-back |
|---|---|---|
| Ví dụ | `push(inner, x)` — inner là `&0 mutable Vector` | `*ref = new_v` — thay cả V |
| Chạm slot? | KHÔNG (handle nguyên) — chỉ heap-object của V lớn lên | GHI ĐÈ 8B slot bằng handle mới |
| Đòi hỏi thêm | 0 — chỉ cần loan exclusive giữ slot ổn định | (a) DROP V cũ (không rỉ) · (b) MOVE new_v vào · (c) **deref-assign syntax** |

**Nguy hiểm write-back:** ghi đè slot mà không drop V cũ = LEAK; drop-of-old +
move-in đều đi **qua reference** — một bề mặt move-tracking mới. Thêm nữa,
`*ref = ...` (deref-assign) **chưa được wire** trong ngôn ngữ hiện tại.

**Quyết định P1 (G CHỐT 2026-07-04):** **IN-PLACE MUTATE ONLY. WRITE-BACK CẤM.**
- Lý do: write-back đòi deref-assign (chưa có) + drop-old-through-ref +
  move-in-through-ref = machinery riêng, đáng ADR riêng khi có use-case; allocator
  chưa có + trait system còn què → nhảy vào write-back bây giờ = tự đào mồ.
- In-place phủ nhu cầu thật (mutate inner container/string đang nằm trong
  container cha — `push` vào inner vector, `insert` vào inner map) và **sound chỉ
  nhờ exclusive loan** — không cần cơ chế mới.

> **⚠️ CẢNH BÁO VACUOUS-SCOPE (ADR-0079 §AMEND-1, phát hiện khi làm A1 — resolve TRƯỚC WO A2):**
> `push`/`insert` của Triết là **functional** (clone + free-old + trả handle MỚI),
> KHÔNG in-place thật. Mutate inner Vector/HashMap qua mutable-borrow ⇒ đòi
> **write-back** handle mới vào cell — mà P1 CẤM write-back. ⇒ "in-place only" có
> nguy cơ **VACUOUS cho V=Vector/HashMap**: chỉ `pop`/`remove` (thu nhỏ, in-place
> thật) dùng được; `push`/`insert` (mở rộng) bị chặn. **Phải chốt với G phạm vi
> A2 thực chất trước khi cấp WO** — có thể A2 P1 rút về "mutate-shrink only" hoặc
> mở hé write-back-handle (thin 8B, KHÔNG drop-old-V vì pop/remove đã cắt) như một
> ngoại lệ hẹp. TBD.
>
> **⛔ LIMITATION P1 (G ký):** CẤM write-back qua mutable ref. `*ref = new_val`
> (deref-assign thay nguyên con V) **KHÔNG được phép** trong P1 — parser/typecheck
> chưa wire deref-assign nên tự-nhiên-refuse; nếu về sau wire deref-assign phải
> refuse tường minh trên `(&0 mutable V)?` cho tới khi có ADR write-back riêng.
> `(&0 mutable V)?` P1 CHỈ để gọi hàm in-place trên V.

## 3. Lõi borrowck phải đổi (vì sao A2 cần ADR mà A1 không)

Read-shared loan (A1/ADR-0079) chỉ conflict với **mutate**. Exclusive loan phải
conflict với **MỌI truy cập** container trong lifetime của borrow — kể cả READ.

Neo từ schema (`triet-schema.yaml:426`, `BorrowExclusiveMutable`):
> "Scope-limited exclusive mutable borrow. **ONLY ONE `&0 mutable T`** [tại một
> thời điểm]."

`Loan::conflicts_with` (`checker.rs:117-122`) ĐÃ đúng: `BorrowExclusiveMutable`
conflict với tất cả. Nhưng **use-site check** hiện chỉ bắn ở mutate-site (U3,
`checker.rs:1180-1210`, lọc theo `arg_consumes`/`mutates_arg`). Với loan
exclusive, một **read** (`len(c)`, `get(&0 c,…)` khác, get_mut thứ hai) trên
cùng source cũng phải E2440/E2410.

**Việc lõi:** khi active loan có form `BorrowExclusiveMutable`, mở rộng check để
**bất kỳ use nào** của source (read hoặc write) = conflict, không chỉ mutate.
Đây là điểm "chệch một li là alias banh xác" G cảnh báo.

## 4. Phạm vi P1 (G CHỐT 2026-07-04)

1. **Value-type — MIRROR TOÀN BỘ A1.** A2 phủ ĐÚNG bộ V mà A1 giao thật:
   V ∈ {String, Vector, HashMap} + Nullable **nếu A1 giao được Nullable** (nếu
   A1 defer Nullable vì không construct được → A2 cũng defer Nullable, đối xứng).
   KHÔNG được tạo API què lúc nhớ lúc quên — "đã lên ngai là thống nhất toàn cõi".
2. **Container-form:** `&0 mutable Vector<V>` + `&0 mutable HashMap<K,V>`, với
   **K ∈ {Integer, String}** — **key-parity BẮT BUỘC**: `HashMap<String,V>` hưởng
   quyền get_mut_ref y hệt `HashMap<Integer,V>` (JIT `__triet_string_hash` đã thông,
   không có lý do bắt nhịn — cú ★SS(c) máu chưa khô).
3. **In-place mutate only** (Q2) — write-back CẤM (§2 LIMITATION).
4. **Return:** `(&0 mutable V)?` — present slot / `~0` not-found.

## 5. Teeth bắt buộc (O sẽ verify máu khi có WO)

- **T1 exclusive-read-conflict:** `let r = get(&0 mutable c, k); let n = len(c);`
  (read container khi mutable-borrow sống) → **E2440**. Poison: hạ form về
  `BorrowReadOnly` → mất E2440 → RED.
- **T2 double-mut:** hai `get(&0 mutable c,·)` sống chồng → **E2440** (ONLY ONE).
- **T3 mutate-through-ref sound:** `push(inner, x)` với `inner` borrow từ
  `get_mut` → run đúng, container không double-free, không leak (counting).
- **T4 negative:** mutable-borrow chết TRƯỚC khi read container → sạch (no error).

## 6. Ba chốt của G (RESOLVED 2026-07-04)

1. **In-place-only P1, write-back CẤM** → ✅ DUYỆT (§2 LIMITATION).
2. **Value-set = MIRROR TOÀN BỘ A1** (String/Vector/HashMap + Nullable-nếu-A1) →
   ✅ CHỐT. Không API què.
3. **`&0 mutable HashMap<String,V>` key-parity** → ✅ **BẮT BUỘC CÓ** (100% parity
   với `HashMap<Integer,V>`).

---

**Trạng thái:** APPROVED mặt kiến trúc. WO A2 **gate sau A1 merge** — vì §4.1 mirror
đúng V-set A1 giao thật (biết Nullable sống/chết mới khóa được overload-set A2, tránh
lệch). O cấp WO A2 ngay khi A1 land + O verify + G ký.

---

## §7 — Lý do ĐÓNG BĂNG (Mentor G lệnh 2026-07-04)

Khi làm A1 (ADR-0079 §AMEND-1) lộ ra: `push`/`insert` của core là
**functional-style** (clone → mutate → trả handle MỚI, free-old). Lấy
`(&0 mutable V)?` ra mà KHÔNG write-back được handle mới đè lên cell của
container mẹ thì mutable-borrow **VACUOUS** với Vector/HashMap (chỉ `pop`/`remove`
shrink dùng được — vô dụng). Mà P1 CẤM write-back vì machinery `deref-assign`
(`*ref = new_val`) + drop-in-place qua con trỏ **chưa dựng**.

**G phán:** không chấp nhận feature nửa vời (chỉ xài được String) hay "lỗ ngách
nhượng bộ bẩn". A2 **đóng băng** cho tới khi core xử lý triệt để `deref-assign`
+ cập nhật handle an toàn. Chuyển hạng mục sang **Cụm D (Phase 3 Ownership —
sub-path reassign)** — cùng họ vấn đề "ghi qua reference/path". Mở lại từ đó.

**Điều kiện mở lại (definition-of-ready cho A2 v2):**
1. `deref-assign` (`*ref = new_val`) wired ở parser + typecheck.
2. Drop-in-place qua con trỏ (drop-old-V trước khi ghi handle mới) — an toàn,
   có counting-tooth.
3. Khi đó A2 mới sound cho V=Vector/HashMap (write-back handle mới vào cell).
