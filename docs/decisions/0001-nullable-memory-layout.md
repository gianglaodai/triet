# ADR 0001 — Memory layout của `T?`

**Trạng thái:** Quyết định, ràng buộc spec ngôn ngữ. Áp dụng từ v0.1; revisit ở v0.3 nếu packing thực sự bottleneck.

**Issue:** SPEC §13 #1 — `T?` lưu thế nào trong bộ nhớ: discriminator riêng (1 trit/byte cho biết null hay không) hay sentinel value (chiếm một biểu diễn "không dùng" của `T`)?

## Quyết định

**Discriminator** (1 trit phụ) cho mọi `T?`. Layout chuẩn:

```
T?  ::=  is_null: 1 trit  +  payload: T
         (-1 = null, +1 = present, 0 reserved cho v0.2 "uninitialized")
```

## Lý do

- **Đối xứng tam phân.** Triết là balanced-ternary first — mọi biểu diễn n-trit của một type đều có ngữ nghĩa. `Trilean` dùng cả 3 giá trị, `Tryte`/`Integer`/`Long` dùng đối xứng quanh 0. Không có "sentinel slot" thừa để tận dụng → sentinel approach buộc phải cắt bớt phạm vi của `T`, vi phạm guarantee §3.2 ("phạm vi đối xứng").
- **AI-first / regular > exception.** Discriminator thống nhất cho mọi `T`. LLM hoặc dev không phải nhớ bảng "type X có sentinel ở đâu". Layout cố định = generation đúng ngay lần đầu.
- **Overhead thực sự nhỏ.** 1 trit ≈ 1.585 bit. Với packing 5-trit/byte (§3.4), `T?` thường tốn thêm 1 byte (round up) nhưng có thể nhồi với discriminator của các `T?` khác kế bên ở backend tối ưu (v0.3+).
- **`?T` 3-state mở rộng tự nhiên.** Discriminator đã dùng 3 giá trị balanced ternary → `is_null` có thể mở thêm trạng thái thứ ba ("uninitialized" / "moved-from") cho v0.2 borrow checker mà không phá layout.

## Hậu quả

- Mỗi field `T?` trong struct (v0.2) phải dự trữ thêm 1 trit. Compiler có thể gộp nhiều discriminator vào ít byte hơn (như Rust niche optimization) khi `T` đã có "natural sentinel" — nhưng đó là optimization v0.3+, **không thay đổi semantic**.
- Backend hardware tam phân giả định (v2.0+) hưởng lợi: discriminator là một trit thật, không phải hack qua bit-packing nhị phân.
- Nullable composition (`T??`) **không** flatten — `T??` là `(is_null₂, (is_null₁, T))`, hai tầng phân biệt được. SPEC §2.5 đã ngầm cấm composition này (ưu tiên `Option<Option<T>>` v0.2 nếu thực sự cần) nên không phát sinh thêm.

## Implementation v0.1

Interpreter dùng `Value::Null` enum variant của Rust — tương đương semantically với discriminator (Rust enum tag = discriminator). Layout vật lý sẽ được commit ở v0.3 khi bytecode VM đến.

---

## Addendum — v0.7.4.3-error (null literal unification)

Per [ADR-0020 §10](0020-outcome-error-handling.md) (2026-05-17), the source-level literal for the Trit::Zero discriminator state is unified across the language:

| Pre-v0.7.4.3 | v0.7.4.3+ (canonical) | v1.0+ |
|---|---|---|
| `null` (deprecated synonym, W2001 warning) | `~0` | `~0` (only — `null` removed, E2002) |

**No change to memory layout.** The 1-trit discriminator + payload union encoding locked in this ADR is unchanged. Only the source syntax for the Trit::Zero state literal changes:

```triet
// Pre-v0.7.4.3 (still works through v1.0 with W2001 warning):
let user: User? = null

// v0.7.4.3+ canonical (no warning):
let user: User? = ~0
```

The lowerer accepts both source forms and emits the same `Constant::Null` IR opcode (see [ADR-0010 Addendum — v0.7.4.3-error](0010-ternary-native-ir.md#addendum--v0743-error-null-literal-unification)). No IR / wire-format change.

**Pattern match implications:** patterns for `T?` types must use explicit `~+ binding` for the success arm (no implicit T ⊂ T? widening in pattern position per ADR-0020 §10.4):

```triet
// Pre-v0.7.4.3 widening match (still works with W2001 if `null` used):
match maybe_user {
    user => greet(user),       // implicit T ⊂ T? widening — DEPRECATED in patterns
    null => prompt_login(),
}

// v0.7.4.3+ canonical:
match maybe_user {
    ~+ user => greet(user),
    ~0      => prompt_login(),
}
```

**Migration:** `triet fmt --fix --migrate-null` auto-rewrites both literal and pattern occurrences. See [ADR-0020 §10.5](0020-outcome-error-handling.md) for tool specification.
