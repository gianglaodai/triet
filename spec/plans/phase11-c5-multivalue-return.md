# C5 — Multi-value Return — Blueprint thăm dò (O spike, G lệnh 2026-06-10)

**HEAD `992311e`. Gate 0·0·102·203.** Spike read-only khảo sát (mẫu Native/B2.0).

## Tóm tắt phán quyết O
C5 **KHÁC Native (tin tốt) + giống B3 (tin xấu):**
- **Premise NHẸ** — KHÔNG có vũng lầy value-model như Native. Móng sẵn (ReturnShape 2-value) + Cranelift multi-return native.
- **0 producer** — không fn nào trả multi-value (Outcome guarded, tuple-return chưa có ngôn ngữ). YAGNI.
→ **Defer YAGNI**, nhưng khi mở DỄ (premise đã sẵn, không phải 3-lát-nền như Native).

## Khảo sát (đo, không đoán)

### Móng ĐÃ SẴN (khác Native — Native thiếu tiền đề)
- **MIR `Return { values: Vec<Local> }`** (mir:677) — đã hỗ trợ >1 value cấu trúc. Display `Return(v1, v2)`.
- **`ReturnShape`** (mir:775) đã có 2-value: `BinaryOutcome` (disc+payload), `TernaryOutcome`. `arity()` trả 0/1/2.
- **Cranelift multi-return NATIVE** — function `sig.returns` nhiều `AbiParam`, caller `inst_results[0..n]`. KHÔNG đụng value-model "single i64" (mỗi value vẫn 1 i64, chỉ nhiều values). **Đây là khác biệt cốt tử với Native** (Native phá value-model; C5 không).

### Điểm CHẶN (1 chỗ, gọn)
- **JIT return path** (jit:1067-1070): `if values.len() > 1 → Err "multi-value return requires Bậc C packed ABI"`. Guard + test (jit:2769-2824 hand-build 2-value → verify Err).
- C5 = gỡ guard + implement return path nhiều values + caller nhận `inst_results[0..n]`.

### 0 PRODUCER (YAGNI như B3)
- `ReturnShape::BinaryOutcome/TernaryOutcome` — **0 producer** (lowerer chưa sinh cho fn nào).
- Outcome ops **guarded** (lower:1124 "~- not in Bậc A scope") — chưa có producer Outcome.
- **Tuple-return** (`-> (A, B)`) — chưa có ngôn ngữ.
- **0 fixture** trả multi-value thật. Nullable match `~+/~0` (48-57) là **single-value** `Integer?`, KHÔNG phải 2-value.

## Blast radius (NHẸ — nếu mở)
| Vùng | Site | Loại |
|------|------|------|
| JIT return guard | jit:1067-1070 | gỡ `len>1` Err |
| JIT return path | jit:1070+ | emit nhiều return values (Cranelift native) |
| JIT caller | inst_results | nhận `[0..n]` thay `[0]` |
| Value-model | — | **KHÔNG đụng** (mỗi value 1 i64, Cranelift trả nhiều) |

## Khuyến nghị O: DEFER YAGNI (nhưng dễ mở)
0 producer → implement multi-value = feature không-ai-dùng (như B3 0-over-reject, Native 0-field). Defer.
**KHÁC Native:** C5 premise NHẸ — khi có producer, mở GỌN (gỡ guard + return path + caller, KHÔNG cần ADR byte-size/value-model). Cranelift làm hết phần ABI.

## ⛔ PHONG ẤN — Nhóm E Deferred (G ký DEFER 2026-06-10)
C5 → Nhóm E cùng Native Layout + Packed Outcome. "Một tính năng dễ làm không có nghĩa là nên làm" (G). 0 producer → không viết JIT cho C5.

## ⛔ Điều kiện mở C5 (Nhóm E Deferred)
1. Có **producer multi-value thật**: Outcome un-guard (`~+ value` construct → T~E, ADR-0020) HOẶC tuple-return ngôn ngữ (`-> (A,B)`).
2. Fixture function trả >1 value (use-case đo được).
→ Khi 1+2 có: C5 mở gọn (gỡ guard jit:1070 + return path nhiều values + caller inst_results). KHÔNG phải đại phẫu.

## Lưu ý quan hệ C4
2-value return chính = **Outcome** (BinaryOutcome). C4 (Packed Outcome ABI) đã phong ấn Nhóm E. C5 + C4 cùng phụ thuộc **Outcome producer**. Mở Outcome producer = tiền đề chung cho cả C4+C5. Cân nhắc gộp khi Outcome active.
