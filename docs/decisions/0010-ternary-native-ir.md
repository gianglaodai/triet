# ADR 0010 — Ternary-native IR: BrTrilean, Eq, NullCheck

**Trạng thái:** Quyết định. Áp dụng cho mọi IR opcode mới + lowerer + VM kể từ v0.3.x.ternary phase. Sửa `.triv` wire format (v2) — vẫn backwards-compatible với v1 reader vì opcodes mới là additive.

**Issue:** Sau cleanup v0.3.x, audit lại 8 commit cleanup phát hiện rằng — mặc dù SPEC + VISION cam kết Triết là tam phân first-class — phần lớn lowerer + IR + VM lại **collapse Trilean sang Boolean** tại biên branch. Cụ thể:

1. **`BrIf` là 2-way branch**. Cond là Trilean nhưng VM dùng `is_truthy()` (chỉ trả `true` cho `Trilean::True`). Cả `Unknown` lẫn `False` đều đi cùng nhánh else. Ł3 có 3 giá trị → IR vứt 1/3 thông tin.

2. **`if?` vs `if` collapse cùng `BrIf`**. SPEC §7.1.1 quy định:
   - `if cond` requires definitely-known cond, **panic on Unknown**
   - `if? cond` treats Unknown as False
   
   Hiện tại cả hai đều dùng `BrIf(cond, then, else)` → semantics khác nhau bị hardcode-collapse tại lowerer thay vì express ở IR.

3. **`EnumTag` trả Trit nhưng chỉ dùng 2/3 trạng thái**. Comment trong code: `Positive for variant 0, Negative for variant >=1`. Enum 3 variant (Red/Green/Blue) đáng lẽ dispatch 1 lệnh dựa trên 1 trit; hiện tại sinh N-1 binary BrIf xâu chuỗi.

4. **`Constant::Null` là bolt-on**. Trong tam phân, discriminator `T?` tự nhiên là 1 trit:
   - `+1` = Some (chắc chắn có)
   - `0` = unknown (chưa biết — useful cho async/lazy)
   - `-1` = None (chắc chắn không có)
   
   Tách `Constant::Null` ra coi như binary "null là một thing riêng".

5. **`Eq` trên `Trilean::Unknown` trả `Trilean::False`** thay vì `Trilean::Unknown`. Ł3 nói: hai giá trị unknown không thể khẳng định bằng nhau hay khác nhau → equality phải là Unknown.

VISION §5 liệt kê 3 điều khiến Triết không thể bị thay thế bởi "Rust + Mojo + Nix": *Trit-level capability, Łukasiewicz checking, ternary ABI primitives*. Cả ba đều bị undermine bởi binary-collapse hiện tại.

ADR này lock thiết kế ternary-native trước khi v0.4 ABI freeze — vì sau v0.4, mỗi binary leak trở thành ABI commitment khó sửa.

## Quyết định

### 1. `BrTrilean` thay `BrIf` làm primary branch opcode

```
BrTrilean { cond, true_block, unknown_block, false_block }
```

- Cond là một SSA value với Trilean semantics.
- Runtime dispatch trực tiếp theo giá trị Trilean:
  - `Trilean::True`  → `true_block`
  - `Trilean::Unknown` → `unknown_block`
  - `Trilean::False` → `false_block`

**Lowering**:
| Source construct | true_block | unknown_block | false_block |
|---|---|---|---|
| `if cond { … } else { … }` (plain) | then | **`unreachable_block`** (panic) | else |
| `if? cond { … } else { … }` | then | else | else |
| `while cond { … }` (plain) | body | **`unreachable_block`** | exit |
| `while? cond { … }` | body | exit | exit |
| Match arm test (Eq → Trilean) | arm_body | next_test | next_test |
| Pattern test (tuple/literal) | enter_body | next_test | next_test |

**`BrIf` được giữ lại** cho 2 trường hợp khi binary semantics là đủ:
- Branch trên một `Trit` đã được verify hoàn toàn 2-state (e.g. `NullCheck` hiện tại).
- Backward-compatible decode của `.triv` v1 files.

Lowerer mới (kể từ v0.3.x.ternary) phải emit `BrTrilean` cho mọi nhánh có cond là Trilean. `BrIf` chỉ giữ cho compat.

### 2. `EnumTag` dùng đầy đủ 3 trit values

Đối với enum N variant:

| N | Tag type | Encoding |
|---|---|---|
| 1 | Unit (no tag) | implicit |
| 2 | Trit (1 trit) | `-1, +1` (zero reserved cho future async/lazy variant) |
| 3 | Trit (1 trit) | `-1, 0, +1` — *idiomatic ternary* |
| 4–9 | Tryte (9 trit) | offset từ -4 |
| 10+ | Integer | full range |

Match dispatch với 3-variant enum lowering thành **1 lệnh `BrTrilean`** trên tag, không phải 2 lệnh BrIf xâu chuỗi.

### 3. Nullable discriminator dùng Trit::Zero làm null

Discriminator của `T?` là một Trit:
- `+1` = Some(value)
- `0`  = null (canonical)
- `-1` = reserved (definitely-missing distinct from null — for future "explicit absent" semantics)

**Implementation pragmatism** — `Constant::Null` variant được giữ lại trong
enum cho compact wire encoding (1 byte vs. 1 instruction + operand) và để
`NullCheck` pattern-match được trực tiếp mà không cần inspect payload.
Nhưng **semantics** của nó được document hoá rõ là "Trit::Zero state of the
nullable discriminator", không phải "null là một thing riêng tách khỏi
trit space". Đây là điểm dây neo bản sắc tam phân.

VM `NullCheck` returns Trit:
- `RuntimeValue::Null` → `Trit::Zero` (matches discriminator)
- Some-wrapped value → `Trit::Positive`
- Future "definitely missing" → `Trit::Negative` (reserved, không emit hiện tại)

Branch dùng `BrTrilean` trên kết quả NullCheck thay vì BrIf.

Việc xoá hoàn toàn `Constant::Null` (thay bằng `Const(Trit::Zero) + NullWrap`
pattern) là defer — phá `.triv` wire format cho lợi ích thẩm mỹ thuần khiết
mà không thay đổi semantics. Re-visit ở v0.5 (CAS packaging) nếu hash
stability cần consolidate.

### 4. `Eq` / `Ne` Ł3-aware

Khi cả hai operands là `Trilean::Unknown`:
- `Eq` trả `Trilean::Unknown` (không khẳng định)
- `Ne` trả `Trilean::Unknown`

Khi một operand là `Trilean::Unknown` và operand kia là `True`/`False`:
- `Eq` trả `Trilean::Unknown` (không khẳng định bằng/khác)
- `Ne` trả `Trilean::Unknown`

Khi hai operand đều definite:
- Equal → `Trilean::True`, otherwise `Trilean::False`.

Đối với Trit operand: same — Trit::Zero ↔ Unknown propagation.

Đối với Integer/Long/Tryte/String operand (không có Unknown state): semantics 2-valued vẫn đúng — luôn trả True/False.

### 5. `.triv` wire format compatibility

- Opcode IDs mới (BrTrilean) chỉ được thêm vào cuối enum encoding — không phá v1 decoder.
- Bumping `.triv` version field từ 1 → 2 (per ADR-0008) khi format có instruction mới.
- v1 reader gặp BrTrilean trả `TrivError::UnknownOpcode` — không silently misinterpret.

### 6. Reserved Trit semantics ở IR level

Trong toàn bộ IR, một `Trit` không bao giờ được phép "có nghĩa boolean":
- `+1` = positive / yes / present / variant-positive
- `0` = zero / unknown / pending / canonical-null
- `-1` = negative / no / absent / variant-negative

Code lowering hoặc VM nào collapse 1 trong 3 trạng thái phải có comment giải thích **tại sao binary collapse là đúng** ở vị trí đó (e.g. "tag đã được verify 2-state ở pass trước").

## Hệ quả

### Đối với v0.4 (ABI)

- Cross-package call result là Trilean? → witness table dispatch phải biết encode 3-state.
- Capability check (v0.6) đã planned dùng Ł3 Unknown để defer-to-runtime; BrTrilean trở thành **opcode bản sắc** chứ không chỉ implementation detail.

### Đối với backend (v0.9 JIT, v2.0 LLVM, v∞ trytecode)

- **JIT (Cranelift)**: BrTrilean lower thành 2 cmp + 2 branch (binary CPU). Có overhead encoding nhưng vẫn correct.
- **LLVM AOT**: same — 2 cmp + 2 branch.
- **Trytecode**: BrTrilean lower thành **1 instruction** thực sự — đây là điểm Triết thắng phần cứng tam phân vĩnh viễn.

### Đối với SPEC

- §7.1.1 chính thức được implement (plain `if` panic on Unknown). Hiện tại chỉ là comment TODO.
- §1.5.2 (Trilean three-valued logic) consistent end-to-end — không còn chỗ nào collapse silent.

### Pace

- Implement: 1–2 ngày (mostly mechanical lowerer migration, đã có test corpus 11/11 làm regression net).
- Không phá test hiện tại nếu lowering chính xác giữ semantic (Unknown→False cho `if?`/match thường).

## Không làm

- **Xoá `BrIf` hoàn toàn**: defer — vẫn cần cho backward `.triv` decode + cho cases binary thực sự (Trit đã verified 2-state). Một sau-phase optional có thể audit hết và xoá.
- **Encoding 4+ variant enum thành Tryte**: defer — không có example nào cần ngay; ADR chỉ ghi mapping, lowerer hiện chỉ implement cho 2–3 variants.
- **Capability Trilean dispatch** (v0.6 trụ cột #5): defer — sẽ build trên BrTrilean infrastructure.
- **Trytecode backend trên hardware tam phân**: v∞.

## Prior art

- **CMU CCured / Refinement types**: 3-state qualifier propagation (safe/uncheckable/wild). Cùng triết lý "đừng collapse semantics tại IR".
- **Setun (Brusentsov 1958)**: phần cứng 3-way branch native — `JZ negative, zero, positive` instruction. Đây là chỗ Triết đi theo.
- **LLVM `select` vs `br`**: LLVM tách select (data) khỏi br (control). BrTrilean ở Triết là `br` với 3 successors thay vì 2.
- **Anti-pattern**: JVM `IFEQ`/`IFNE` chỉ check zero/non-zero — đã đúc lực trong binary thinking từ năm 1995, không thể sửa mà không phá ABI.

## Tham chiếu

- [SPEC §1.5.2 — Trilean](../../SPEC.md)
- [SPEC §7.1.1 — if/if? semantics](../../SPEC.md)
- [VISION §5 — Bản sắc Triết](../../VISION.md)
- [ADR-0007 — IR design](0007-ir-design.md) (this ADR refines)
- [ADR-0008 — .triv binary format](0008-triv-binary-format.md) (this ADR bumps version)
- [ADR-0009 — Version gate policy](0009-version-gate-policy.md) (this ADR is filed under v0.3.x.ternary phase)
