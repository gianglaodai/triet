---
name: campaign_jit_print_adr0087
description: WO-JIT-Print / ADR-0087 — print/println stdout hoạt động lần đầu sau rewrite; 4-overload × 4 extern-C shim; 2× LUẬT-4 phơi lỗ recon O
metadata: 
  node_type: memory
  type: project
  originSessionId: 3712b205-e00c-4e74-a480-17f825f1409d
  modified: 2026-07-25T12:11:10.744Z
---

# WO-JIT-Print / ADR-0087 — I/O stdout đầu tiên sau rewrite (2026-07-25)

origin/main feat-head **`25cb2cc`**, gate `0·clean·0·458·0`, 6 commit. O+G ký.
Đây là **stdout-write ĐẦU TIÊN** trên backend rewrite — trước đó `print`/`println`
chỉ `declare` trong typecheck, rớt default-arm lower → JIT `callee not found` exit 4
(feature-gap, KHÔNG silent-miscompile).

## Thiết kế (G ký 5 quyết định, ADR-0087)
- **4 overload:** `print(String)`, `print(&0 String)`, `println(String)`, `println(&0 String)`.
  Owned=MOVE=tiêu thụ; `&0 String`=borrow (Reference là Copy)=tái dùng được sau in.
- **4 extern-C shim** (memory-responsibility hardcode vào TÊN symbol, KHÔNG cờ `is_owned`):
  `__triet_print`/`__triet_println` owned `arity:3 (ptr,len,cap)` `arg_consumes:[true]` → write rồi `__triet_string_free`;
  `__triet_print_ref`/`__triet_println_ref` `arity:2 (ptr,len)` `arg_consumes:[false]` → write only.
  Owned move-in ⇒ shim sở hữu+free; M3 zero caller-slot ⇒ caller Deinit=free(0) no-op ⇒ single free (mẫu `vector_push`).
- **Unit return đàng hoàng:** `emit_shim_call` (`triet-lower/src/lib.rs`) thêm nhánh
  `MirType::Unit` → `dest:vec![]` + `ReturnShape::Unit`, trả **Unit local thật** (`ConstValue::Unit`),
  KHÔNG throwaway-i64-rebound. **G BÁC thẳng trick i64-0 của O** ("nợ kỹ thuật, mọi hàm Unit tương lai lặp rác").
  `ShimSymbol{has_return:false}` (mẫu void đã có sẵn); register ở `triet-driver/src/main.rs`.
- **Cap compile-time only** (`capability_check.rs` E2200/E2201; `std`=ambient, `sys.io`=grant). KHÔNG runtime `__triet_cap_check`. Bám VISION §capability.
- Route arm `triet-lower/src/lib.rs:2661` `"print"|"println"` TRƯỚC default: strip Reference-prefix (mẫu `len`), base phải String, dispatch `(op,is_ref)`→1/4 shim.

## 🔑 BÀI HỌC LỚN: recon-trước-WO của O vẫn SÓT tầng typecheck — D cứu 2 lần qua LUẬT-4
1. **Lỗ recon (a):** WO Task-3 liệt kê thiếu `env.rs`. `print`/`println` khai bằng `env.declare`
   ĐƠN (không `declare_overload`); mà `check_call` chỉ chạy `resolve_overload` khi
   `env.lookup(name).is_none()` (`exprs.rs:879`); và `Type::matches` (`types.rs:274`) KHÔNG
   coerce `Reference(String)`→`String`. ⇒ `println(&0 s)` bị typecheck từ chối TRƯỚC lowerer
   ⇒ T2/T4 (dùng `&0 s`) **không reachable**. D DỪNG hỏi. O verify 4 claim đúng hết → fix:
   đổi CẢ HAI sang `declare_overload` + thêm overload `Reference(BorrowReadOnly,String)` (mirror len/eq/concat).
2. **Lỗ recon (b):** 2 test `flags_call_arity_mismatch`/`flags_call_argument_type_mismatch`
   (`triet-typecheck/src/lib.rs`) mượn tạm `print` để test cơ chế WrongArity/Mismatch chung;
   giờ `print` overload → gọi sai đẻ `NoMatchingOverload` (đúng hành vi mới) → 2 test vỡ.
   D đề (a) đổi sang `to_string` (single-sig). **O chọn (a) NHƯNG dùng USER-DEFINED fn** —
   fix GỐC: tách test khỏi trạng-thái-overload của builtin (to_string sau này overload lại vỡ y hệt);
   user-fn là single-sig điển hình, không bao giờ overload ⇒ giết cả lớp fragility.

**Khắc: recon-trước-WO PHẢI phủ typecheck env (declare vs declare_overload) khi mở overload builtin —
mẫu chuẩn là mọi `&0`-overload đi qua `declare_overload`, regular binding sẽ nuốt resolve_overload.**
Đây là biến thể của [[feedback_verify_producer_before_consumer]]: bản đồ O/Giang cũng là giả định tới khi chạm compiler thật.

## 🦷 Teeth (O verify độc lập, không tin raw D)
File `crates/triet-driver/tests/print_println_overload_subprocess.rs` — subprocess fork-guard
(UB→crash child) + delegating counting-free `__ppo_str_free` (đếm-rồi-free-thật, bắt double-dealloc)
+ assert CẢ stdout content LẪN FREE-count. T1 owned FREE=1 · T2 ref-reuse "x\nx\n" FREE=1 · T3 no-newline · T4 routing "a\nb\na\n" FREE=2.
**🩸 O TỰ re-poison T1:** meta `__triet_println [true]→[false]` → child crash
`free(): double free detected in tcache 2` (UB THẬT glibc), restore cp md5 `bade48f3` khớp, xanh lại.
**T4 failure-mode lệch dự đoán** (O đoán "leak FREE==0"; thực = refuse-compile qua guard marshalling
`arg_ty.is_reference()`) — D báo ĐÚNG thực tế quan sát ([[feedback_failure_mode_precision]]), tooth vẫn đỏ, refuse-over-guess tốt hơn.

## ⚖ D=Sonnet 5 subagent (O spawn theo lệnh G)
2× DỪNG đúng LUẬT-4 (blocker THẬT, không tự chế), khai thật main.rs (file WO O ghi nhầm chỗ)
+ T4-lệch, 0 vết bịa, cp-restore đúng luật (KHÔNG git checkout). O gатекeeper verify máu:
git-state, code review soundness, E2E run thật (stdout hiện + `s` tái dùng qua 2 println_ref, Drop 1 lần),
self-gate, re-poison T1. **HỌNG chốt: O tự chạy gate + tự đóng T1, không ký trên raw D.**

## Nợ defer (đáy sổ, chưa mở)
`read_line` (input) · f-string/format runtime · buffering policy (line vs unbuffered).

→ [[campaign_str0_coverage_and_triage]] (phiên trước, `&0 String` coverage — cùng mẫu declare_overload)
