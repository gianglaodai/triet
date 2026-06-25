---
name: future-sized-ternary-ints
description: Converged design intent cho sized ternary integer family + literal suffixes. Defer tới một type-system ADR riêng (post-v0.11). Đọc khi bàn numeric types / kernel footprint / Integer width.
metadata: 
  node_type: memory
  type: project
  originSessionId: 5ad1339c-d4d2-4c07-9823-7d76ba88d258
---

Bàn 2026-05-31 (giữa lúc mở v0.11). Author muốn kiểu số đủ năng lực viết kernel (không lãng phí tài nguyên như Rust i8/i16/…). **Hội tụ: phần lớn ĐÃ có sẵn — gần như suýt làm thừa.**

## Hiện trạng (SPEC §2.1) — họ tam phân theo lũy thừa 3 trit
- `Trit`(1) · `Tryte`(9=3²,±9841) · `Integer`(27=3³,±3,8×10¹², **mặc định**) · `Long`(81=3⁴,±2,2×10³⁸).
- Balanced ternary đối xứng → **KHÔNG có unsigned** → họ kiểu gọn bằng nửa Rust (không `u`, không bug dấu). Đây là selling point "đơn giản hơn".
- Nhị phân là ngoại lệ phải đánh dấu: `BinaryInteger`(i32) · `BinaryLong`(i64) · `BinaryByte`(u8).

## Quyết định author (chưa implement — chỉ ghi nhận intent)
1. **Không thêm tên kiểu mới.** Giữ Trit/Tryte/Integer/Long + Binary*.
2. **Khoảng trống thật duy nhất = kiểu 3 trit** (3¹, ±13) — chỉ cần TÌM TÊN (TBD; naming = quyền author per [[feedback_implementer_choice]]). Hữu ích cho kernel: cờ/counter nhỏ gói 1 byte thay vì Integer 6 byte.
3. **`t3/t9/t27/t81` = HẬU TỐ ép kiểu literal**, KHÔNG phải tên kiểu. Đối xứng: tam phân dùng `tN`, nhị phân dùng `iN` (i8/i16/…). Mở rộng/điều hoà với hậu tố hiện có `_trit`/`_tryte`/`_long` (SPEC §1.5.1) — chi tiết để ADR sau chốt.
4. **`Integer` GIỮ CỐ ĐỊNH 27 trit + deterministic mọi host.** TUYỆT ĐỐI không làm Integer linh hoạt theo phần cứng (i64 nhị phân / i81 tam phân) — sẽ phá ngữ nghĩa tam phân-first + phá **byte-identical bootstrap gate** (chính gate v0.11 AOT cache đang nâng). Mục tiêu tiết kiệm tài nguyên đạt bằng: sized types nhỏ (footprint) + `Binary*` (tốc độ native nhị phân) — không cần đụng Integer.

## Liên hệ v0.11 (nhẹ)
- ADR-0033 §5 **đã tách cache theo `target_triple`** → kiến trúc đã lường codegen khác theo phần cứng đích; cửa để mở.
- Giữ JIT/AOT ABI (map_type/shim, [[ADR-0032]]) KHÔNG hard-code "chỉ tồn tại Integer=i64". Mở rộng ABI cho sized ints là việc JIT tương lai, không phải v0.11.

## Process
Type-system feature thực sự → **ADR riêng** (vd ADR-0034 "sized ternary integer family + numeric width policy") + **phase riêng (~v0.12 type-system)**. KHÔNG gộp vào v0.11 (AOT cache). Per [[feedback_stability_over_speed]]. Liên quan [[reference_spec]], [[project_vision_os_capable]] (kịch bản phần cứng tam phân là động lực gốc).
