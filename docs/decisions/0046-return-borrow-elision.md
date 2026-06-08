# ADR-0046: Return-borrow Elision — Bậc C lát 3

**Status:** ACCEPTED — O + G ký 2026-06-08
**Date:** 2026-06-08
**Author:** AI (đồng nghiệp D, implement)
**Reviewers:** Mentor O (semantics, soundness) — KÝ 2026-06-08 · Mentor G (layout, ABI, codegen) — KÝ 2026-06-08
**Scope:** Mở `fn id(s: &0 T) -> &0 T { return s }` — callee trả về reference mượn từ
param, caller giữ owner đóng băng khi ref còn sống. Dựa trên hạ tầng PropagatedLoan
đã có sẵn từ ADR-0045 §4.

---

## Tóm tắt

ADR-0045 §5 CẮT `-> &0 T` return type bằng E1042 để niêm phong lát return-borrow.
Lát này mở lại: typecheck cho phép `-> &0 T`, lower populate `return_borrow_map`
(mắt xích đứt duy nhất), PropagatedLoan engine (có sẵn trong checker từ ADR-0045 §4)
re-issue loan ở caller → owner đóng băng khi reference còn sống, ngăn use-after-free.

Phase-0 đã probe thực nghiệm: 3/4 hạ tầng đã có sẵn. Lỗ hổng duy nhất còn mở là
`return_borrow_map` rỗng ở lower (lib.rs:168). Việc duy nhất cần code: populate
map đó + mở E1042 gate cho `&0`.

---

## §0 — Dữ kiện

| # | Dữ kiện | Vị trí |
|---|---------|--------|
| F1 | Elision decision (0/1/multi ref-param) đã có sẵn. `check_lifetime_elision` (check.rs:494) bắn E2400 khi count ≠ 1. | `check.rs:494-551` |
| F2 | Dangling return `&0 <local>` đã bị E2450 bắt. `storage-end` của local chấm dứt loan → return ref tới local bị từ chối. | `borrowck/checker.rs` (E2450) |
| F3 | PropagatedLoan engine đã có sẵn trong checker (`checker.rs:754-796`), có unit test `returned_reference_extends_source_lifetime` (checker.rs:1384). Engine LIVE-in-test nhưng DEAD-in-production vì `return_borrow_map` luôn rỗng (F5). | `checker.rs:754-796`, `checker.rs:1384` |
| F4 | Driver đã wire `callee_sigs` + `check_body_with` (main.rs:95, 102). Mắt xích thứ hai của PropagatedLoan đã được nối từ ADR-0045. | `main.rs:95-102` |
| F5 | **Lỗ hổng duy nhất:** `return_borrow_map` ở lower luôn `::new()` rỗng (lib.rs:168-170). Không populate → PropagatedLoan không re-issue loan ở caller → `let stolen = m;` sau `id(&0 m)` lọt qua borrowck. | `lower/lib.rs:168-170` |
| F6 | E1042 gate (check.rs:398) đang chặn TẤT CẢ `-> Ref T` bằng `if let Type::Reference(_, _)`. Cần đổi sang match riêng để whitelist `&0`. | `check.rs:398-408` |
| F7 | O đã probe thực nghiệm: tạm gỡ E1042 cho `&0`, test `id(&0 m); let stolen = m; use r` → lọt (không borrow error), exit 0. Chứng minh F5 là lỗ hổng thật. | Probe session 2026-06-08 |

---

## §1 — E1042 form-gate: whitelist `&0`, refuse còn lại

**Quyết định:** Đổi `if let Type::Reference(_, _)` (check.rs:398) thành `match`
riêng `ReferenceForm::BorrowReadOnly` cho qua; giữ E1042 refuse cho
`StrongFrozen`/`StrongMutable`/`BorrowExclusiveMutable`/`WeakObserver`.

**Lý do:** `&0` (shared read-only) là form duy nhất có thể return-borrow an toàn
với hạ tầng hiện tại:
- `&+` (strong owning) → liên quan refcount/ObjectHeader, defer.
- `&+ mutable` → tương tự.
- `&0 mutable` → exclusive borrow cần exclusivity guarantee phức tạp hơn, defer.
- `&-` (weak observer) → chưa có use-case rõ ràng, defer.

**Triển khai:**
```rust
// check.rs:398 — thay if let Type::Reference(_, _) bằng:
match &return_type {
    Type::Reference(form, _) => match form {
        ReferenceForm::BorrowReadOnly => { /* cho qua */ }
        _ => {
            // E1042 cho StrongFrozen / StrongMutable /
            // BorrowExclusiveMutable / WeakObserver
            self.errors.push(TypeError::BorrowReturnNotYetSupported { ... });
        }
    },
    _ => {}
}
```

---

## §2 — Elision = REUSE E2400, KHÔNG đẻ E1043

**Quyết định:** `check_lifetime_elision` (check.rs:494) là source-of-truth duy nhất
cho elision decision. Lower KHÔNG viết lại logic refuse. Không tạo mã lỗi E1043.

**Lý do:** E2400 đã bắt count≠1 với message đẹp + 3 Fix gợi ý (ADR-0025 §3.4).
Đẻ thêm E1043 ở lower là trùng lặp — typecheck đã là fatal gate (driver main.rs:64
`ExitCode::from(3)`). Lower chỉ cần defense-in-depth Err (không panic) khi typecheck
rò rỉ (xem §3).

**Ghi chú:** G từng định đẻ E1043 trong thảo luận ADR — đã bỏ sau khi O chỉ ra
E2400 có sẵn + message đẹp hơn. ADR này ghi rõ lý do để session sau không tái tạo.

---

## §3 — Lower populate `return_borrow_map`

**Quyết định:** Tại `lower/lib.rs:168`, thay `ReturnBorrowMap::new()` bằng logic:
- Đếm ref-param (non-owning `&0`/`&0 mutable`/`&-`).
- `count == 1` → `return_borrow_map[FieldPath::Root] = {param_index}`.
- `count != 1` → `Err(LowerError{...})` **KHÔNG panic!**.

**Lý do Err-không-panic:** Typecheck E2400 là fatal (`ExitCode 3` ở driver
main.rs:64) — khi lower chạy, count≠1 đã được đảm bảo không thể xảy ra. NHƯNG:
- Harness `run_fixture` (integration_tests.rs:64) cố tình chạy-rốn qua type-error
  để test // ERROR annotations — `panic!` sẽ SIGABRT giết toàn bộ 74 fixture.
- `Err` thì harness bắt ở dòng 71-74: `Err(e) => { errors.push(...); return Err(...) }`
  → fixture đó fail sạch (lower error), corpus loop sang fixture kế — KHÔNG SIGABRT
  như panic.

**Mẫu LowerError:**
```rust
_ => return Err(LowerError {
    message: "internal: return-borrow elision expects exactly 1 ref-param \
              (typecheck E2400 should have rejected this)".into(),
    span,
}),
```

**Tiền lệ:** `lib.rs:1200` — `ok_or_else(|| LowerError { ... })` cho enum variant
không tìm thấy. Pattern giống hệt: internal invariant bị phá → Err, không panic.

**TECH-DEBT (O, 2026-06-08):** `is_propagated` skip E2450 ở Drop (checker.rs:692)
dựa trên giả định: Triết chưa có nested block scope + move-sớm đã bị E2440 chặn,
nên propagated ref KHÔNG THỂ outlive owner. Nếu sau này thêm block-scope lồng hoặc
cho phép ref escape rộng hơn (vd capture trong closure, return ref qua nhiều tầng),
phải re-audit cái skip này — propagated ref có thể outlive owner thật trong các
cấu trúc đó. Ghi nhận để không quên khi mở rộng scope system.

---

## §4 — BỎ E1043 (ghi rõ lý do)

**Quyết định:** Không tạo mã lỗi `E1043` cho "return-borrow elision failed at
lowerer".

**Lý do:**
1. E2400 đã cover case này ở typecheck với message đầy đủ hơn (3 Fix suggestions).
2. Typecheck là fatal gate — lower không bao giờ thấy count≠1 trong production.
3. Defense-in-depth ở lower dùng `LowerError` (string message) — không cần mã lỗi
   riêng vì đây là internal invariant, không phải user-facing diagnostic.

Ghi vào ADR này để session sau không tái tạo E1043.

---

## §5 — Teeth (3 nhóm)

### Nhóm A — Caller-hole (load-bearing)

Fixture mới: `id(&0 m); let stolen = m; use r` — move `m` khi `r` (mượn từ `m`)
còn sống → borrowck phải bắt (mã lỗi xác nhận bằng RUN lúc implement, dự kiến
E2440 hoặc E2450).

| Fixture | Directive | Kỳ vọng (guard CÓ — populate hoạt động) | Teeth (gỡ populate → `::new()` rỗng) |
|---------|-----------|------------------------------------------|--------------------------------------|
| `79_return_borrow_caller_freeze.tri` | `// ERROR: E24xx` (negative fixture) | Borrow error fired → test PASS | Lọt, exit 0, không error → fixture ĐỎ |

**Quan trọng:** Đây là negative fixture — directive `// ERROR:`, không phải
`// EXPECT:`. Khi guard có, borrowck bắn lỗi → fixture pass. Khi gỡ populate,
code biên dịch + chạy exit 0 (use-after-free latent) → fixture đỏ vì EXPECT
error nhưng không có error. Đây là regression test chính cho lát 3 — nếu ai đó
xóa populate, test này PHẢI đỏ, chứng minh guard thật sự bảo vệ.

### Nhóm B — Return-local

Fixture: `fn f(s: &0 String) -> &0 String { let x; return &0 x }` → E2450.

Đã sống (E2450 hoạt động độc lập với return_borrow_map). Chỉ fixture-hoá.

### Nhóm C — 0/multi ref-param

| Test | Input | Kỳ vọng |
|------|-------|---------|
| `81_return_borrow_0_param.tri` | 0 ref-param → `fn f() -> &0 String` | E2400 |
| `82_return_borrow_multi_param.tri` | 2+ ref-param → `fn f(a: &0 String, b: &0 String) -> &0 String` | E2400 |

Đã sống (E2400 hoạt động độc lập). Chỉ fixture-hoá.

---

## Kế hoạch triển khai

| # | Việc | File chính | Teeth |
|---|------|-----------|-------|
| 1 | ADR → commit | `docs/decisions/0046-return-borrow-elision.md` | O rà line-ref trước khi code |
| 2 | §1 E1042 form-gate | `typecheck/src/check.rs:398` | Fixture: `-> &+ T` vẫn E1042; `-> &0 T` qua typecheck |
| 3 | §3 lower populate `return_borrow_map` | `lower/src/lib.rs:168` | Teeth Nhóm A: gỡ populate → lọt lại |
| 4 | count≠1 → Err(LowerError) | `lower/src/lib.rs:168` | Fixture multi-param: harness không SIGABRT, vẫn ra E2400 |
| 5 | Fixture return-local → E2450 | Fixtures | Đã sống, fixture-hoá |
| 6 | Fixture 0/multi param → E2400 | Fixtures | Đã sống, fixture-hoá |
| 7 | Báo cáo ĐÓNG | `scripts/gate.sh` raw | O tự chạy lại, tự teeth |

Gate `scripts/gate.sh` raw sau mỗi bước. Mỗi fixture mới vào corpus.

---

## Q&A

### O-Q1: Vì sao không panic ở lower khi count≠1?

Harness `run_fixture` chạy-rốn qua type-error (integration_tests.rs:64). `panic!`
→ SIGABRT giết toàn bộ 74 fixture. `Err(LowerError)` → harness bắt và continue.
Tiền lệ: `lib.rs:1200`.

### O-Q2: Vì sao không đẻ E1043?

E2400 đã có sẵn, message đẹp hơn (3 Fix suggestions), và là fatal gate (typecheck
abort `ExitCode::from(3)` ở main.rs:64). E1043 trùng lặp. G từng đề xuất rồi bỏ —
ghi vào ADR để không tái tạo. (§4)

### G-Q1: PropagatedLoan engine có thay đổi gì không?

Không. Engine (`checker.rs:754-796`) đã hoạt động đúng trong unit test từ
ADR-0045 §4. Lát này chỉ populate `return_borrow_map` ở lower — engine tự động
re-issue loan khi thấy map không rỗng.

### G-Q2: ABI có thay đổi không?

Không. Return-borrow vẫn truyền handle i64 by-value như cũ. ABI đồng nhất owned
và borrow — khác biệt thuần ngữ nghĩa, quản lý bởi borrowck + lower.

### G-Q3: `&0 mutable` / `&+` return-borrow?

Defer. Chỉ `&0` shared read-only trong lát này. (§1)
