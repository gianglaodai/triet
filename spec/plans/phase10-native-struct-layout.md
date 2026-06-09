# Native Struct Layout — Blueprint thăm dò (O khảo sát, G lệnh 2026-06-10)

**HEAD `0fb8de6`. Gate 0·0·101·203.** Spike khảo sát theo mẫu B2.0/C1.0.

## Tóm tắt phán quyết O
Native struct layout **CHƯA SẴN SÀNG** — thiếu **2 tiền đề nền** + **0 nhu cầu hiện tại**.
Không phải lát "đổi `*8` → `field.offset`" (JIT ĐÃ dùng field.offset). Vấn đề sâu hơn:
value model Bậc A "mọi value = 1 i64" phải nâng cấp, và `MirType→byte-size` chưa định nghĩa.

## 4 câu hỏi G — trả lời (đo, không đoán)

### Q1: StructLayout MIR đã tính size/align thật chưa, hay field×8?
**Infrastructure ĐÚNG, input GIẢ.**
- `StructLayout::compute` (mir:1143) **đã tính offset align-aware thật**: `offset = align_up(offset, align); push FieldLayout{offset, size, align}; offset += size`. `FieldLayout` có đủ `offset`/`size`/`alignment`.
- **NHƯNG** `lower:347` nhét `(f.name, ty, 8, 8)` — **hardcode size=8 align=8 mọi field** bất kể MirType. → compute ra offset đúng theo input, nhưng input = mọi field 8B → offset = index×8 thực tế.
- **Sửa điểm 1:** thay `8, 8` bằng `mir_type_size(ty), mir_type_align(ty)`. **Nhưng 2 hàm đó CHƯA TỒN TẠI** (xem Tiền đề 1).

### Q2: JIT FieldAccess hardcode `index*8`?
**KHÔNG — đã offset-aware.** jit:291/366 `let offset = i32::try_from(field.offset)`. Đọc/ghi qua `field.offset` từ FieldLayout. Đây là phần G lo lắng nhất — **đã sạch**. Spike "đổi *8 → field.offset" G đề xuất = **không cần** (đã làm).

### Q3: Nhét real offset vào JIT có gãy param passing?
**Điểm gãy KHÔNG ở offset — ở LOAD TYPE.** jit:294/369 `stack_load(I64, slot, offset)` / `stack_store(value, ...)` — **I64 CỨNG**. Lý do: value model Bậc A = "every value is a single i64" (jit:186). Nếu native-pack field <8B (Trit 1B) tại offset thật → `stack_load(I64)` đọc 8B → **tràn sang field kế** = UB. Param by-pointer (struct caller stack_addr + callee load fields I64-8B-increment) cũng tràn.
**Blast:** 14 `stack_load(I64)` + 21 `stack_store` + value model nền — phải đổi load/store type theo `field.size` (I8/I16/I32/I64) + sign/zero-extend.

### Q4: Blast radius khi lật cờ Native Layout
| Vùng | Site | Loại |
|------|------|------|
| `lower:347` field size/align | 1 | đổi `8,8` → `mir_type_size/align` (cần Tiền đề 1) |
| JIT field load/store type | 14 load + 21 store | I64-cứng → type-theo-size + extend (Tiền đề 2) |
| Enum payload load (C1) | jit:706-712 | disc@0 + payload 8B-increment → native packed phải đổi |
| Param by-pointer (struct/enum) | callee field-load | I64-8B → packed-aware |
| Value model | jit:186 "single i64" | nâng cấp nền |

## 2 TIỀN ĐỀ NỀN THIẾU (chặn native — không thể spike khi thiếu)

### Tiền đề 1: `MirType → byte-size` CHƯA định nghĩa
`MirType` không có `fn size()`/`fn align()`. SPEC balanced-ternary: Trit=1 trit, Tryte=9 trit, Integer=27 trit, Long=81 trit — **byte-size CHƯA map**. Trit 1 byte? Tryte 2 byte (9 trit)? Đây là **quyết định ABI/encoding nền** — chưa có ADR. Không có size thật thì `lower:347` không có gì để nhét, spike vô nghĩa.

### Tiền đề 2: Value model "every value = 1 i64" (Bậc A) phải nâng cấp
jit:186 model nền: mọi scalar unboxed thành 1 i64 register / 8B slot. Native packing đòi field <8B load/store đúng width (I8/I16/I32) + extend khi đưa vào register i64. Đây là thay đổi value-model, không phải struct-only.

## 0 NHU CẦU HIỆN TẠI (YAGNI như B3)
**Mọi struct field trong corpus là `Integer` (8B).** `rg ':\s*(Trit|Tryte|Long|Trilean)' fixtures/*.tri` trong struct → **0 hit**. Tất cả `struct Point { x: Integer, y: Integer }`, Inner/Outer cũng Integer. → Native packing **= no-op hiện tại** (Integer 8B, offset không đổi). Như B3 (0 over-reject): xây giờ là tối ưu cho field chưa tồn tại.

## Khuyến nghị O
**Hai đường, G quyết:**
- **(A) Defer như B3 (YAGNI).** 0 field non-Integer → native = no-op. Mở khi có struct dùng Trit/Tryte field thật + ADR byte-size. Giữ Bậc A i64-uniform (đơn giản, sound).
- **(B) Làm tiền đề trước (nếu G muốn móng cho Packed ABI/FFI tương lai):** ADR `MirType byte-size encoding` (Trit/Tryte/Int/Long → bytes) TRƯỚC → rồi value-model nâng cấp → rồi native layout. Đây là **3 lát nền**, không phải 1 lát struct.

**O nghiêng (A) defer** — giống B3: 0 nhu cầu thực, rủi ro value-model nền cao, JIT field-offset đã sạch (Q2). Native layout là tối ưu cache/FFI cho tương lai, không phải nợ-bom. Nhưng nếu G coi i64-phình là "di chứng đáng xấu hổ chặn Packed ABI" cần dọn sớm → (B) với ADR byte-size trước.

## ⛔ PHONG ẤN — Nhóm E Deferred (G quyết (A) DEFER TUYỆT ĐỐI 2026-06-10)
Native Struct Layout **+ Packed Outcome ABI** (Outcome cũng pack bit/byte nhỏ) → kho lưu trữ.
Lý do: YAGNI — 0 fixture dùng Trit/Tryte trong struct, đập value-model JIT chỉ để "hoàn hảo
kiến trúc" lúc này = tự sát. JIT field-offset đã sạch (Q2). i64-uniform Bậc A giữ (đơn giản, sound).

### 3 ĐIỀU KIỆN MỞ PHONG ẤN (G — phải đủ CẢ BA)
1. **Có fixture thực tế** dùng `Trit`/`Tryte`/`Long` BÊN TRONG Struct/Enum (native packing có hiệu lực đo được, không no-op).
2. **ADR Byte-size Mapping cho Balanced-Ternary** (vd Trit=1 byte, Tryte=2 bytes, Integer=8, Long=?) — định nghĩa `MirType→bytes`.
3. **Value-Model mới cho JIT** hỗ trợ `stack_load_8`/`stack_load_16`/`sign_extend` (thay I64-cứng, 14 load + 21 store site).
