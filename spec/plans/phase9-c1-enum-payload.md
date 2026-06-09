# C1 — Enum Payload qua Function Param (by-pointer)

**Blueprint 2026-06-10. HEAD `11d11cf`. Gate 0·0·101·203.**

**Quyết định O:** KHÔNG cần ADR mới. C1 tái dùng mẫu by-pointer Bậc D (ADR-0049)
+ enum layout (ADR-0037) — không đổi semantics ngôn ngữ, không ABI mới. Blueprint đủ.

## 1. Gap

### Hiện trạng: enum arg truyền thiếu payload

```
caller:  stack_load(I64, enum_slot, 0)  → chỉ discriminant (8B đầu)
callee:  param không có enum_slots → "Payload access on non-enum local"
```

### Fixture 27 — chết vì C1

```triet
enum OptionInt { SomeInt(Integer), NoneInt }
function unwrap_or_default(opt: OptionInt, default_val: Integer) -> Integer =
    match opt { SomeInt(x) => x, NoneInt => default_val }
function main() -> Integer = {
    let a = SomeInt(52);
    return unwrap_or_default(a, 0);  // ERROR: Payload access on non-enum local
}
```

**Expected:** `52`. **Actual:** JIT error.

### Nguyên nhân

| Tầng | Site | Mã hiện tại | Vấn đề |
|------|------|------------|--------|
| Caller | `jit:1162-1163` | `stack_load(I64, slot, 0)` | Chỉ truyền discriminant (8B), VỨT payload |
| Callee param-entry | `jit:676-694` | Enum param không tạo `enum_slots` | Payload không tái dựng trong callee |
| Payload access | `jit:310/382` | `enum_slots.get(param_local)` → None | → "Payload access on non-enum local" |

## 2. Thiết kế: tái dùng mẫu struct by-pointer

### Precedent: struct param đã by-pointer

```rust
// jit:1159 — struct arg: stack_addr (manual by-pointer, bất kể size)
if let Some((slot, _)) = self.struct_slots.get(a) {
    builder.ins().stack_addr(I64, *slot, 0)  // ← pointer-to-slot
}
```

Enum follow y hệt struct — KHÔNG cần spike ABI sâu. Struct đã chứng minh manual by-pointer
hoạt động với Cranelift SystemV.

### Quyết định: (a) Mọi enum by-pointer (nhất quán)

**Chọn (a):** Mọi enum param dùng by-pointer — kể cả unit-variant (chỉ có discriminant).
Lý do: nhất quán với struct precedent, ít nhánh code, fixture 32 (unit-enum EXPECT 2)
thành regression teeth tự nhiên.

**Không chọn (b):** Phân biệt unit-vs-payload tạo 2 đường code (by-value cho unit,
by-pointer cho payload) — phức tạp hóa không cần thiết.

### Caller

```rust
// CŨ (jit:1162-1163):
// } else if let Some((slot, _)) = self.enum_slots.get(a) {
//     builder.ins().stack_load(I64, *slot, 0)  // ← CHỈ discriminant!
// }

// MỚI: enum arg → stack_addr (giống struct jit:1159)
} else if let Some((slot, _)) = self.enum_slots.get(a) {
    builder.ins().stack_addr(I64, *slot, 0)  // ← pointer-to-slot
}
```

Enum local LUÔN có slot (EnumAlloc tạo enum_slots tại jit:616) → không cần fallback
`use_var`. Nhánh `else { use_var }` cho scalar — enum không vào đó.

### Callee param-entry

Thêm nhánh `MirType::Enum(_)` vào vòng lặp param (sau String ở `jit:683`):

```rust
// Mới: Enum param — tạo enum_slots + load từ caller's pointer
if let MirType::Enum(enum_name) = &body.local_decls[local.0].ty {
    // Lấy layout (đã có trong body.enum_layouts từ lowerer)
    let layout = body.enum_layouts.iter()
        .find(|e| e.name == *enum_name)
        .ok_or_else(|| JitError::Unsupported(
            format!("enum layout not found: {enum_name}")
        ))?;
    // Tạo slot riêng trong callee
    let slot = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        layout.total_size as u32,
        layout.alignment.ilog2() as u8,
    ));
    self.enum_slots.insert(local, (slot, layout.clone()));
    // Load [disc][payload] từ caller's pointer
    let mem_flags = cranelift_codegen::ir::MemFlags::new();
    let disc = builder.ins().load(I64, mem_flags, param_val, 0);
    builder.ins().stack_store(disc, *slot, 0);
    // Payload area: copy remaining bytes (8B increments)
    let payload_size = layout.total_size - 8;
    for off in (8..layout.total_size).step_by(8) {
        let field = builder.ins().load(I64, mem_flags, param_val, off as i32);
        builder.ins().stack_store(field, *slot, off as i32);
    }
    // (Nếu payload_size = 0 — unit variant — loop không chạy, chỉ disc được copy)
}
```

### Móng B1a cắn

Quyết định by-pointer qua `MirType::Enum(name)` — `name` bind từ destructure.
Active consumer đầu tiên của `MirType::Enum`. Khép nợ "móng Struct/Enum no-consumer" từ B1a.

## 3. Kế hoạch triển khai (3 lát)

### C1.0 — Spike: xác nhận enum follow struct by-pointer

- Struct param đã by-pointer (jit:1156-1160) — precedent
- Enum layout cùng pattern ([disc][payload]) — chỉ khác nội dung slot
- **Spike nhẹ (inline):** đổi `stack_load` → `stack_addr` ở caller + thêm enum_slots ở
  callee → chạy fixture 27. Không cần nhánh throwaway (precedent rõ).
- **cp /tmp snapshot** trước khi sửa.

### C1.1 — Caller + callee

- `jit:1162-1163`: `stack_load` → `stack_addr` cho enum
- `jit:676-694`: thêm nhánh `MirType::Enum(name)` → tạo slot + load từ pointer
- Xóa fallback `use_var` cho enum (luôn có slot từ EnumAlloc)
- Verify: fixture 27 chạy ra 52, fixture 32 giữ EXPECT 2

### C1.2 — Fixture 27 positive + cleanup

- `27_enum_payload_param_error.tri`: `// ERROR: Payload access` → `// EXPECT: 52`
- Đập string-match `jit:314` (giờ không còn đường nào vào)
- Verify multi-variant: fixture 32 (nonfirst-variant, EXPECT 2)

### C1.3 — Teeth

| Teeth | Cách poison | Kết quả mong đợi |
|-------|------------|-------------------|
| Caller bỏ payload | `stack_load(I64, slot, 0)` thay `stack_addr` | fixture 27 sai giá trị |
| Match Enum dùng string | `ty.to_string() == "Color"` thay `MirType::Enum(_)` | "non-enum" error trở lại |
| Fixture 32 regression | (a) by-pointer mọi enum | EXPECT 2 vẫn đúng |
| Fixture 25/26 | Không đụng | Giữ xanh |

## 4. Rủi ro

| Rủi ro | Mitigation |
|--------|-----------|
| Fixture 32 unit-variant vỡ khi đổi sang by-pointer | Regression teeth — giữ EXPECT 2 |
| Enum layout >16B (multi-payload tương lai) | By-pointer không phụ thuộc size |
| Break struct param | Struct đã stack_addr — enum follow cùng pattern |

## 5. Tiêu chí done

```
fixture 27: EXPECT 52 (positive — không còn ERROR)
fixture 32: EXPECT 2 (unit-enum — giữ nguyên, regression check)
fixture 25/26: EXPECT 1/52 (unit/payload local — không đụng)
gate: 0 build · 0 test · fixtures ≥101 · clippy ≤203
cargo test --workspace → exit 0 (RAW)
```

## 6. Phụ lục: ADR reference

- **ADR-0049** (Bậc D fat-pointer ABI): String by-pointer precedent
- **ADR-0037** (enum layout): `EnumLayout` với `total_size`, `alignment`, `discriminant_offset`
- **ADR-0050** (MirType): `MirType::Enum(String)` — active consumer đầu tiên
