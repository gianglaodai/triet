---
name: campaign_shim_meta_spof_adr0085
description: "✅ ĐÓNG 2026-07-24 (Nhịp 1 + 2a) — ADR-0085 bịt SPOF builtin_shim_meta. Nhịp 1: bảng toàn phần 8-entry + cổng tồn tại Body::verify() discriminator __triet_ (P-exist). Nhịp 2a: self-loan-exclusion M3 mutate-precheck + khôi phục mutates_arg:Some(0) append/clear (E2440-string-mutate). origin/main f6b569f, gate 0·0·452·0. D bác O 2/2 đúng; O luật-18 dính BA lần cùng phiên (poison cứu cả ba). 2026-07-24(b): Nhịp 2b KHAI TỬ (audit lôi ra canary stale; lỗ thật = push_owned-vs-M3 isolation, hạ xuống nợ defense-in-depth sổ đen G)."
metadata:
  node_type: memory
  type: project
  originSessionId: 82e5c422
  modified: 2026-07-24T15:21:23.782Z
---

## ✅ ĐÓNG — Nhịp 1 + 2a, O+G ký, đã PUSH (origin/main `f6b569f`, gate `0·0·452·0`)

```
f6b569f docs(todo): close Nhịp 2a self-loan-exclusion (ADR-0085 AMEND-2)
9a9cb72 fix: WO-N2a-Amend — thay T2 vacuous bằng plain-local mutate-precheck tooth
7c8cbf2 feat: WO-N2a — self-loan-exclusion M3 mutate-precheck + khôi phục mutates_arg:Some(0)
15b5b9a docs(adr): ADR-0085 shim-meta totality + verify-gate (Nhịp 1 close)
63887e4 feat: WO-N1 — bảng toàn phần 8-entry + cổng tồn tại Body::verify()
```

## Bối cảnh — SPOF `builtin_shim_meta` (phát hiện ở [[campaign_nullable_position_and_temp_ownership]], nợ ở [[campaign_forgot_nullable_sweep]])
Bảng tĩnh `triet-mir/src/lib.rs:1076`, đọc bởi **5 read-site / 3 crate** (borrowck ×3, JIT M3 ×1, lowerer `emit_shim_call` ×1), tất cả `if let Some`. Entry thiếu/láo nuốt câm: thiếu-consuming→JIT không zero + lowerer schedule Drop = **double-free câm**; flag-láo tiêu-thụ-thực-mượn→double-free, mượn-thực-tiêu-thụ→leak. KHÔNG defense-in-depth (một nguồn, năm nơi sai cùng chiều).

## Nhịp 1 (P-exist) — `63887e4`+`15b5b9a`
**Bảng TOÀN PHẦN 8-entry** (thêm 7→**8**: `cap_check`/`hashmap_contains`/`pow`/`string_append`/`string_clear`/`string_contains`/`string_hash` + **`vector_contains`**) + **cổng tồn tại `Body::verify()`** discriminator `__triet_`: `CallDispatch` gọi `__triet_*` mà `builtin_shim_meta`→`None` = `MirError::UnknownShim` ở **P3.5 (TRƯỚC borrowck/JIT)**, machine-fixable ADR-0027, KHÔNG silent-miscompile. Option β (single-gate verify) > α (5 bản sao predicate) > γ (registry refactor, blast lớn) > δ (existence canary = bản-sao-thứ-4 oracle-vòng). §append/clear ban đầu `mutates_arg:Some(0)` "vá luôn E2440" — **AMEND-2 hạ về `None`** (xem Nhịp 2a).
🩸 **O poison:** xóa `__triet_vector_push`→fixture 166 nổ `UnknownShim` P3.5, restore md5 `14a6a39`. T2 (`some_user_fn`→Ok)+452 fixture xanh chứng minh discriminator KHÔNG giết user-fn.
⚖ **D bác O 2/2:** ① **bảng 7→8** — O recon `comm` CHỈ JIT-dispatch-names, sót `__triet_vector_contains` emit từ **lowerer** `:2607` (chạy sống fixture 86). ② **`mutates_arg:Some(0)` scope-creep** — wire vào tự bắn 5 fixture E2440.

## Nhịp 2a (self-loan-exclusion) — `7c8cbf2`+`9a9cb72`+`f6b569f`
**MIR-grounded (bác cả giả định D lẫn O):** `clear(&0 mutable m)` lower thành `_1 = &0 mutable _0; Call clear(_1)` — args[0]=`_1`=**loan.dest** (reference), KHÔNG phải `_0`=m. Precheck `checker.rs:1294` so `places_conflict(loan.source=_0, arg=_1, conservative)` → `_1` trỏ `_0` → alias → E2440 với **loan của chính nó**. **Fix = mirror tiền lệ U2 `:1260`:** trace `arg`→`real_place` (loan.dest→loan.source) + `.filter(|l| l.dest != *arg)` loại self-loan. + khôi phục `mutates_arg:Some(0)` append/clear → E2440-string-mutate genuine hoạt động. Blast radius: pop/remove (container Local trần, không borrow-dest) BẤT BIẾN.
⚖ **O tự bắt T2-clear VACUOUS TRƯỚC khi ký:** T2 gốc dùng `clear` + 2 borrow `r`(&0)+`arg`(&0 mut) → E2440 tới từ **borrow-conflict thường** tại tạo `arg` (ExclusiveMutable vs live ReadOnly), nổ TRƯỚC precheck. Poison `.filter(|_l| false)` (mù toàn precheck) → T2 **vẫn pass** = lá chắn giấy. Với reference-arg, genuine-concurrent non-vacuous là BẤT KHẢ (borrow-conflict luôn nổ trước). → G Option A: xóa, thay **T2 plain-local** `pop(v)` né borrow-conflict, đập thẳng precheck.
🩸 **O poison hai lưỡi T2 mới (non-vacuous):** (a) `.filter(false)`→T2 FAILED · (b) over-exclude `source!=real_place`→T2 FAILED (**lưỡi ăn tiền: canh false-negative**). (T3) gỡ dòng `l.dest!=*arg`→fixture 93 nổ E2440 (teeth false-positive, orthogonal T2). Hai teeth / hai mệnh đề / hai poison. restore md5 `4357908d`.

## 🦷 LUẬT 18 DÍNH O **BA LẦN** MỘT PHIÊN (kỷ lục xấu) — cùng gốc "thiết kế bằng trí tưởng tượng, không nhìn memory model"
1. **Bảng 7→8:** recon nửa vời (`comm` một cành JIT, sót lowerer-emit). Vi phạm CHÍNH luật 19 O tự viết vào WO ("grep TOÀN BỘ họ").
2. **`mutates_arg:Some(0)` scope-creep:** đắp cơ chế bắt-E2440 không hợp calling-convention `&0 mutable` (self-loan). "Một mũi tên trúng hai đích" → tự bắn 5 fixture.
3. **T2 vacuous:** thiết kế spec-test dùng reference-arg → E2440 từ tầng khác, test luôn xanh = nói dối.
🔑 **Cả BA quy trình poison/D-đo bắt được TRƯỚC khi ký.** G đóng đinh: *"Con người đầy lối mòn — chỉ quy trình paranoid, poison testing, constraint phần cứng mới cứu dự án."* **Sắc lệnh G mới:** mặt trận sau, mọi cơ chế phải kèm **map trace MIR đính kèm** — hết thời thiết kế bằng thơ ca.

## Ghi chú vai
**D (Sonnet 5):** bác O 2/2 đúng (wire+đo thực địa, không nhắm mắt chép bảng O), dùng poison ĐÚNG không mắc bẫy (O cảnh báo hai-poison trong WO). Vết: **kỷ luật báo cáo — tóm tắt log 3 lần** (WO-N1 ×2, WO-N2a ×1) → G ra **SẮC LỆNH HẠ TẦNG THƯỜNG TRỰC: mọi WO đóng cứng foreground+timeout600000+raw, reject-thẳng-tay nếu tóm tắt** (thực thi lần cuối, D ói raw). Chạy gate nền + không commit WIP (round 1) → O bắt commit trước.

## ⚰ NHỊP 2b KHAI TỬ (2026-07-24(b), G ký) — audit gác-cổng lôi ra bản chất thật
O recon toàn diện trước khi ra WO (Giang chốt "kiểm coverage-gap trước"): **canary "per consuming shim" là STALE** — cả 3 cờ heap-consume (push elem, insert key, insert value) ĐÃ có tooth free-once + poison ghi sẵn trong corpus `*_counting.rs` (typed_vector #1, typed_hashmap #1/#4, vector_userstruct T-DOUBLE/T-LEAK). Dựng canary mới = rơi vào bẫy **δ** G đã bác. **LỖ THẬT DUY NHẤT** (comment prior-O `triet-lower/src/lib.rs:1490-1509` tự thú): mọi tooth test **pipeline HỢP NHẤT** (lowerer `push_owned` + JIT M3 cùng bật); vì **M3 một mình đã đủ** (zero slot → Drop no-op), tầng `push_owned` là **phòng thủ dư thừa BỊ CHE** — sai hoàn toàn mà test vẫn xanh. Tooth thật phải **fire với M3 TẮT** → cần hạ tầng disable-M3 JIT chưa tồn tại, canh một tầng hôm-nay-không-load-bearing. G phán: **lãng phí tài nguyên chiến tranh, Simplicity First** → hạ Nhịp 2b từ "campaign khẩn cấp" xuống **nợ defense-in-depth `push_owned`-vs-M3-isolation** trong sổ đen. 🩸 O suýt đuổi nhầm mục tiêu (canary-fixture) — bằng chứng producer+consumer + comment prior-O cứu; đúng sắc lệnh G "DUMP MIR/đọc source, không thơ ca".

## 🔴 NỢ CÒN TREO
1. **push_owned-vs-M3 isolation** (defense-in-depth, sổ đen G): tooth chứng minh `push_owned` free-once VỚI M3 tắt — cần feature-flag/JitContext option disable-M3. Không khẩn (M3 đang đủ, không hở bộ nhớ).
2. Carry-over: `mir_lower.rs` panic thay Err (~4 panic + 4 unreachable + 5 unwrap + 38 expect, Track B #1 — cần triage reachable-vs-internal) · container-element `Nullable(Struct-heap)` refuse (§15.6) · lỗ N1 widening · method-call return over-refuse · Deep-Clone · drain · `&0 Enum` tiêu thụ · borrow-params `&+ T` (Bậc C lát 2). **ĐÃ ĐÓNG:** `LowerError` hệ mã lỗi → [[campaign_front_a_lower_error_codes]].

[[campaign_forgot_nullable_sweep]] [[campaign_nullable_position_and_temp_ownership]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_g_report_protocol]]
