---
name: campaign_shim_meta_spof_adr0085
description: "✅ ĐÓNG TRỌN 2026-07-25 — ADR-0085 bịt SPOF builtin_shim_meta.arg_consumes. Nhịp 1 (P-exist): bảng toàn phần 8-entry + cổng Body::verify() discriminator __triet_. Nhịp 2a: self-loan-exclusion M3 mutate-precheck. Nhịp 2 THREAT-1 canary (WO-SPOF-3, c22a751): shim_arg_consumes_spof_canary.rs pin bảng vs hành vi free C-shim, teeth 2 chiều đo được (push [t,t]→[t,f] FREE 1→2 double-free; pop [f]→[t] FREE 2→1 leak). THREAT-2 (WO-SPOF-2 disable_m3 isolation) CHÔN vĩnh viễn = bất khả kiến trúc (Stmt::Let Drop vô điều kiện → M3 load-bearing named-local tombstone, không cô lập !consumed bằng FREE-count M3-OFF được). O trượt shape Chiều-2 BA lần (verify consumer bỏ producer-gate), D+O tự phanh bằng MIR-dump. gate 0·clean·0·455·0."
metadata:
  node_type: memory
  type: project
  originSessionId: 82e5c422
  modified: 2026-07-25T02:53:16.321Z
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

## ✅ NHỊP 2 THREAT-1 CANARY ĐÓNG + THREAT-2 CHÔN VĨNH VIỄN (2026-07-25, O+G ký, PUSH `c22a751`)
G tách **HAI threat model** O đã gộp nhầm:
- **Threat 1 (bảng khai láo vs C-shim):** teeth-able. Đóng bằng `crates/triet-driver/tests/shim_arg_consumes_spof_canary.rs` — canary **M3-ON** (`with_shims`, như production), counting-shim `__sacs_*` đếm String-free, 3 shape (A push→FREE1, B insert→FREE1, C push×2+pop→FREE2). Teeth 2 chiều **O tự cắm, đo thật** (cp-snapshot mir `ff3d1fd1`, KHÔNG git checkout): **Chiều 1** consume→borrow `__triet_vector_push [true,true]→[true,false]` → A **FREE 1→2** (double-free đếm sạch, KHÔNG SIGABRT vì counting-shim không free thật); **Chiều 2** borrow→consume `__triet_vector_pop [false]→[true]` → C **FREE 2→1** (leak "a"). Đóng nợ "no test canaries arg_consumes itself" (note `triet-lower/src/lib.rs:1661`).
- **Threat 2 (bảng vs Lowerer `!consumed` — cái nợ #1 cũ "push_owned-vs-M3 isolation"): CHÔN = BẤT KHẢ KIẾN TRÚC.** WO-SPOF-2 (thêm `JitOptions.disable_m3` + canary M3-OFF assert FREE==1) **HỦY — premise sai**: `Stmt::Let` đăng ký Drop cho named local **VÔ ĐIỀU KIỆN** → nhánh `!consumed` chỉ cứu **anonymous temp** khỏi Drop thừa; tombstone named-local nằm **HOÀN TOÀN trên M3**. M3 KHÔNG phải "mái che dư thừa" — load-bearing cho việc KHÁC. Tắt M3 → baseline **tự double-free** (comment `triet-lower/src/lib.rs:1641-1647` đã ghi ĐÚNG crash-mode `free(): double free ... tcache 2` SIGABRT từ trước) → không có baseline sạch để cô lập `!consumed`. D chạy canary M3-OFF → tái lập đúng SIGABRT đó → O verify độc lập + đọc lại note 1641-1647 (bằng chứng nằm TRONG recon của chính O) → nhận lỗi, revert sạch (`git checkout` diff D chưa commit + rm canary).

🩸 **VẾT O — trượt shape Chiều-2 BA LẦN cùng gốc "verify consumer path, bỏ producer-gate":** ① `length(Vector<String>)` — không overload (Integer-only), typecheck reject (D bắt). ② `length(s)` owned String — **fast-path** `triet-lower/src/lib.rs:2701-2723` đọc field `len` trực tiếp, bypass `emit_shim_call` (D bắt, MIR-dump). ③ `length(&0 s)` — reference = **Copy** (`mir 629/736`), M3 `!is_copy@4809` + borrowck `@1333` skip → poison **vacuous** (O tự bắt trước re-task). Lối thoát DUY NHẤT verify-reachable: `pop<T>(Vector<String>)` generic → non-Copy handle by-value qua `emit_shim_call:3095`. **Bài học G đóng đinh (lần thứ N): "đừng thiết kế test bằng trí tưởng tượng — mọi thứ phải đập vào kiến trúc thật."** Cả 3 lần D/O tự phanh TRƯỚC khi gõ code (MIR-dump + đọc-thật + is_copy layer).
⚖ **D=Sonnet 5 (subagent):** DỪNG đúng LUẬT-4 **hai lần** (owned-String bypass, Vector<String> no-overload), báo MIR-dump thật, KHÔNG tự mở scope/nới assertion. Nộp gate raw đầu, canary sạch (A=1,B=1,C=2). Vết nhẹ: `length(&0 s)` fix đề xuất còn sót tầng is_copy (O bắt).

## 🔴 NỢ CÒN TREO
1. Carry-over: `mir_lower.rs` panic thay Err (~4 panic + 4 unreachable + 5 unwrap + 38 expect, Track B #1 — cần triage reachable-vs-internal; **NHÓM A đã đóng** → [[campaign_front_b_panic_audit_group_a]]) · lỗ N1 widening · method-call return over-refuse · Deep-Clone · drain · `&0 Enum` tiêu thụ · borrow-params `&+ T` (Bậc C lát 2). **ĐÃ ĐÓNG:** Threat-2 push_owned-vs-M3 (bất khả kiến trúc, section trên) · container-element `Nullable(Struct-heap)` refuse §15.6 → [[campaign_spof_container_nullable]] (WO-SPOF-1) · `LowerError` hệ mã lỗi → [[campaign_front_a_lower_error_codes]].

[[campaign_forgot_nullable_sweep]] [[campaign_nullable_position_and_temp_ownership]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_g_report_protocol]]
