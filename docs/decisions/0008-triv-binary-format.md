# ADR 0008 — `.triv` bytecode binary format

**Trạng thái:** Quyết định. Áp dụng cho v0.3.9+ (serializer/deserializer) và mọi
backend đọc `.triv` (CLI, JIT, AOT, trytecode). Là wire format chính thức của
Triết IR theo [ADR-0007 § Wire format](0007-ir-design.md).

**Issue:** ADR-0007 định nghĩa IR shape (register SSA, type-tagged, virtual
registers). Nhưng in-memory Rust types (`enum Instruction`, `struct Function`,
...) không phải là wire format — không có magic bytes, version field, hay cấu
trúc section cho forward compatibility. Cần một binary format:

- **Deterministic**: cùng `IrProgram` → cùng byte sequence (cho CAS hash v0.5).
- **Compact**: varint encoding, constant pool dedup, string interning.
- **Versioned**: magic + version field, additive-only sau v1.0.
- **Self-describing**: section layout cho phép tooling đọc mà không cần
  compiler source.
- **Stable**: v1.0 freeze, backend phải đọc được `.triv` cũ.

ADR này lock binary format trước khi serialize/deserialize implementation (v0.3.9).

## Quyết định

`.triv` là binary format với:

1. **Magic bytes** `0x74 0x72 0x69 0x76` ("triv" ASCII).
2. **32-bit version** (little-endian) — currently `4` (centralized history below).

**Version history (canonical — single source of truth):**

| Version | Phase / ADR | Change | Reader behavior on encounter |
|---|---|---|---|
| `1` | v0.3 initial release ([ADR-0008](0008-triv-binary-format.md), this ADR) | Initial format: magic + version + section_count + 4 sections (types/constants/functions/code). | n/a (oldest readable) |
| `2` | v0.3.x.ternary ([ADR-0010](0010-ternary-native-ir.md)) | Added `BR_TRILEAN` opcode (0xB4). | v1 readers emit `UnknownOpcode` on 0xB4. |
| `3` | v0.4 ([ADR-0012](0012-witness-table-dispatch.md)) | Added `WITNESS_CALL` opcode (0x93) + new `witness_tables` section (5). | v2 readers emit `UnknownOpcode` on 0x93. |
| `4` | v0.7.3.1 ([ADR-0019 Addendum §A1](0019-self-hosting-compiler-bootstrap.md)) | Added type discriminants 8 (Vector) + 9 (HashMap). | v3 readers emit `UnknownTypeDiscriminant` on 8/9. |
| `5` | v0.7.4.3-error ([ADR-0020 §7](0020-outcome-error-handling.md), pending impl) | Added type discriminant 10 (Outcome with `allow_null_state: bool`) + 6 opcodes 0xC1–0xC6 (`OUTCOME_NEW_POSITIVE/NEGATIVE/NULL`, `OUTCOME_DISCRIMINANT`, `OUTCOME_UNWRAP_VALUE/ERROR`). | v4 readers emit `UnknownTypeDiscriminant` on 10 / `UnknownOpcode` on 0xC1–0xC6. |

Each bump is **additive-only** per §"Version compatibility" rules below — no semantic change to existing sections/opcodes. Older readers refuse cleanly on encountering newer features, never silently misinterpret.
3. **Section-based layout** — mỗi section có `section_id` (1 byte) + `section_size` (u32
   LE). Unknown section → skip, không error.
4. **Little-endian** cho multi-byte integers (đồng bộ với CPU target chính:
   x86-64, ARM64, RISC-V).
5. **LEB128 unsigned varint** cho tất cả integer nhỏ (`ValueId`, `BlockId`,
   `FuncId`, `ConstId`, counts, field indices).
6. **Length-prefixed UTF-8** cho strings (LEB128 length + bytes).

### File layout

```
┌──────────────────────────────────────────────────────────────┐
│ .triv file                                                    │
├──────────────┬───────────────────────────────────────────────┤
│ magic        │ 4 bytes: 0x74 0x72 0x69 0x76 ("triv")        │
│ version      │ 4 bytes: u32 LE (= 1)                         │
│ section_count│ 4 bytes: u32 LE                                │
├──────────────┴───────────────────────────────────────────────┤
│ section 0..N                                                  │
│   section_id    1 byte                                        │
│   section_size  4 bytes u32 LE (payload only, excl header)    │
│   payload       section_size bytes                            │
└──────────────────────────────────────────────────────────────┘
```

Section IDs:

| ID | Name | Content | Required |
|----|------|---------|----------|
| 1 | `types` | Type tag table for dedup | Yes |
| 2 | `constants` | Constant pool entries | Yes |
| 3 | `functions` | Function signatures + metadata | Yes |
| 4 | `code` | Function bodies (blocks, instructions) | Yes |
| 5–127 | (reserved) | For future Triết versions | — |
| 128–255 | (custom) | Tooling/backend-specific metadata | No |

Unknown section IDs: skip `section_size` bytes, continue. This is how forward
compatibility works — newer `.triv` files with extra sections are readable by
older tools.

### Type tag table (section 1)

Deduplicated type tags referenced by index from function signatures and
instructions. Encoded recursively:

```
type_count:    LEB128 u32
types[0..N]:
  discriminant: 1 byte
    0 = Trit
    1 = Tryte
    2 = Integer
    3 = Long
    4 = Trilean
    5 = String
    6 = Unit
    7 = Nullable (followed by inner type index: LEB128 u32)
```

Type indices are `u32` (LEB128), stable within the file. The type table is
write-once at serialization time; every reference to a type (function params,
return types, instruction type annotations) uses a type index.

### Constant pool (section 2)

```
constant_count: LEB128 u32
constants[0..N]:
  type_index: LEB128 u32 (into type table)
  value:      type-dependent (see below)
```

Value encoding by type:

| Type | Value encoding |
|------|---------------|
| Trit | 1 byte: `0x00` = Negative, `0x01` = Zero, `0x02` = Positive |
| Tryte | i16 LE (the 9-trit value as i16) |
| Integer | i64 LE (the 27-trit value as i64) |
| Long | LEB128 byte count + i128 LE bytes |
| Trilean | 1 byte: `0x00` = False, `0x01` = Unknown, `0x02` = True |
| String | LEB128 byte count + UTF-8 bytes |
| Unit | 0 bytes |

Constant IDs are assigned in insertion order (0..N-1), matching the in-memory
`ConstantPool` dedup order. A valid reader must reconstruct the same ID ↔ entry
mapping.

### Function table (section 3)

```
function_count: LEB128 u32
functions[0..N]:
  name_length:    LEB128 u32
  name:           UTF-8 bytes (0 bytes if no name)
  param_count:    LEB128 u32
  params[0..N]:
    name_length:  LEB128 u32
    name:         UTF-8 bytes
    type_index:   LEB128 u32
  return_type:    LEB128 u32 (type index)
```

Function IDs are assigned in the order functions appear. The code for function
`fN` is in the code section's N-th function body.

### Code section (section 4)

```
function_count: LEB128 u32
function_bodies[0..N]:
  block_count:   LEB128 u32
  blocks[0..N]:
    block_id:    LEB128 u32
    name_length: LEB128 u32
    name:        UTF-8 bytes (0 bytes if no name)
    instr_count: LEB128 u32
    instructions[0..N]:
      opcode:    1 byte
      operands:  opcode-dependent (see opcode table)
```

Block IDs in the wire format are the logical `BlockId` values. Blocks appear in
an arbitrary order; the entry block is the one with the smallest block_id
(conventionally `b0`).

### Opcode table

Each instruction is a 1-byte opcode followed by operands. Opcodes are grouped
for readability; reserved gaps allow additive extension.

**Constants** (0x00–0x0F):
| Opcode | Mnemonic | Operands |
|--------|----------|----------|
| `0x00` | `const` | `dest: varint, const_id: varint` |

**Arithmetic** (0x10–0x1F):
| Opcode | Mnemonic | Operands |
|--------|----------|----------|
| `0x10` | `add` | `dest: varint, lhs: operand, rhs: operand` |
| `0x11` | `sub` | `dest: varint, lhs: operand, rhs: operand` |
| `0x12` | `mul` | `dest: varint, lhs: operand, rhs: operand` |
| `0x13` | `div` | `dest: varint, lhs: operand, rhs: operand` |
| `0x14` | `mod` | `dest: varint, lhs: operand, rhs: operand` |
| `0x15` | `pow` | `dest: varint, base: operand, exp: operand` |
| `0x16` | `neg` | `dest: varint, operand: operand` |

**Ł3 Logic** (0x20–0x2F):
| Opcode | Mnemonic | Operands |
|--------|----------|----------|
| `0x20` | `luk_and` | `dest: varint, lhs: operand, rhs: operand` |
| `0x21` | `luk_or` | `dest: varint, lhs: operand, rhs: operand` |
| `0x22` | `luk_implies` | `dest: varint, lhs: operand, rhs: operand` |
| `0x23` | `luk_xor` | `dest: varint, lhs: operand, rhs: operand` |
| `0x24` | `luk_iff` | `dest: varint, lhs: operand, rhs: operand` |

**K3 Logic** (0x30–0x3F):
| Opcode | Mnemonic | Operands |
|--------|----------|----------|
| `0x30` | `kleene_implies` | `dest: varint, lhs: operand, rhs: operand` |
| `0x31` | `kleene_xor` | `dest: varint, lhs: operand, rhs: operand` |
| `0x32` | `kleene_iff` | `dest: varint, lhs: operand, rhs: operand` |

**Comparison** (0x40–0x4F):
| Opcode | Mnemonic | Operands |
|--------|----------|----------|
| `0x40` | `eq` | `dest: varint, lhs: operand, rhs: operand` |
| `0x41` | `ne` | `dest: varint, lhs: operand, rhs: operand` |
| `0x42` | `lt` | `dest: varint, lhs: operand, rhs: operand` |
| `0x43` | `le` | `dest: varint, lhs: operand, rhs: operand` |
| `0x44` | `gt` | `dest: varint, lhs: operand, rhs: operand` |
| `0x45` | `ge` | `dest: varint, lhs: operand, rhs: operand` |

**Conversion** (0x50–0x5F):
| Opcode | Mnemonic | Operands |
|--------|----------|----------|
| `0x50` | `to_integer` | `dest: varint, operand: operand` |
| `0x51` | `to_tryte` | `dest: varint, operand: operand` |
| `0x52` | `to_long` | `dest: varint, operand: operand` |
| `0x53` | `to_trit` | `dest: varint, operand: operand` |
| `0x54` | `to_trilean` | `dest: varint, operand: operand` |

**Aggregate: struct** (0x60–0x6F):
| Opcode | Mnemonic | Operands |
|--------|----------|----------|
| `0x60` | `struct_new` | `dest: varint, field_count: varint, fields[0..N]: operand` |
| `0x61` | `field_get` | `dest: varint, object: operand, field_idx: varint` |
| `0x62` | `field_set` | `dest: varint, object: operand, field_idx: varint, value: operand` |

**Aggregate: enum** (0x70–0x7F):
| Opcode | Mnemonic | Operands |
|--------|----------|----------|
| `0x70` | `enum_new` | `dest: varint, variant_idx: varint, has_payload: 1 byte, payload?: operand` |
| `0x71` | `enum_tag` | `dest: varint, scrutinee: operand` |
| `0x72` | `enum_payload` | `dest: varint, scrutinee: operand` |

**Nullable** (0x80–0x8F):
| Opcode | Mnemonic | Operands |
|--------|----------|----------|
| `0x80` | `null_wrap` | `dest: varint, value: operand` |
| `0x81` | `null_unwrap` | `dest: varint, nullable: operand` |
| `0x82` | `null_check` | `dest: varint, nullable: operand` |

**Function calls** (0x90–0x9F):
| Opcode | Mnemonic | Operands |
|--------|----------|----------|
| `0x90` | `call_local` | `has_dest: 1 byte, dest?: varint, callee: varint, arg_count: varint, args[0..N]: operand` |
| `0x91` | `call_cross_module` | `has_dest: 1 byte, dest?: varint, path_length: varint, path: UTF-8 bytes, arg_count: varint, args[0..N]: operand` |
| `0x92` | `call_builtin` | `has_dest: 1 byte, dest?: varint, builtin_id: 1 byte, arg_count: varint, args[0..N]: operand` |

**Closure** (0xA0–0xAF):
| Opcode | Mnemonic | Operands |
|--------|----------|----------|
| `0xA0` | `closure_new` | `dest: varint, lambda: varint, capture_count: varint, captures[0..N]: varint` |
| `0xA1` | `closure_call` | `has_dest: 1 byte, dest?: varint, closure: operand, arg_count: varint, args[0..N]: operand` |

**Control flow** (0xB0–0xBF):
| Opcode | Mnemonic | Operands |
|--------|----------|----------|
| `0xB0` | `br` | `target: varint` |
| `0xB1` | `br_if` | `cond: operand, then_block: varint, else_block: varint` |
| `0xB2` | `ret` | `has_value: 1 byte, value?: operand` |
| `0xB3` | `unreachable` | (none) |

**Phi** (0xC0):
| Opcode | Mnemonic | Operands |
|--------|----------|----------|
| `0xC0` | `phi` | `dest: varint, incoming_count: varint, incoming[0..N]: (value: varint, block: varint)` |

Reserved opcode ranges for future expansion:
- `0xC1–0xCF`: future control flow / phi extensions
- `0xD0–0xEF`: reserved (memory model, ABI, concurrency — v0.4+)
- `0xF0–0xFF`: reserved (custom/experimental — never stable)

### Operand encoding

An `operand` is a tagged union on the wire:

```
operand:
  tag: 1 byte
    0x00 = const (followed by const_id: LEB128 varint)
    0x01 = value (followed by value_id: LEB128 varint)
```

### Builtin ID table

| ID | Builtin |
|----|---------|
| 0 | `println` |
| 1 | `print` |
| 2 | `assert` |
| 3 | `assert_eq` |

### Cross-module call path encoding

`AbsolutePath` is encoded as a dot-separated UTF-8 string (e.g.,
`"std.io.println"`), length-prefixed with LEB128. This is consistent with the
ADR-0005 dot-path convention and human-readable in hex dumps.

### Version compatibility

- **Version 1**: format described in this ADR.
- **v1.0 freeze**: after v1.0, all changes are additive-only (new sections, new
  opcodes, new type discriminants). Existing sections/opcodes never change
  semantics.
- **Version bump rules**:
  - **Patch bump** (e.g. 1 → 2): new opcodes or type discriminants added; old
    readers skip unknown opcodes (error at runtime, not at load time).
  - **Minor bump** (e.g. 1 → 256): new section added; old readers skip via
    section_id.
  - **Major bump** (e.g. 1 → 65536): breaking change to existing section layout
    or opcode semantics. Requires new ADR. Never happens after v1.0.
- **Reader behavior on version mismatch**:
  - Version ≤ reader's max supported: read normally.
  - Version > reader's max supported but same major: read, skip unknown
    sections/opcodes.
  - Major version change: error with diagnostic (`triet::modules::E2102`).

### Corruption detection

- Section size mismatch (actual bytes ≠ declared `section_size`): error.
- Truncated file (EOF mid-section): error.
- Invalid UTF-8 in strings: error.
- Unknown type discriminant: error (decoder needs all types to reconstruct
  values).
- Unknown opcode: warn and skip (for forward compatibility within same major).

The reader is strict: any structural corruption is a hard error. There is no
"best effort" recovery mode — per SPEC §0.3.5 "Refuse over guess".

## Lý do

### Mapped to SPEC design principles

| SPEC § | Nguyên tắc | Áp dụng cho `.triv` |
|--------|-----------|---------------------|
| §0.3.1 | **AI-first** — explicit, low ambiguity | Section layout self-describing; hex dump readable with section headers. Varint encoding documented — LLM có thể sinh parser từ spec này. |
| §0.3.4 | **Stability over speed** — ADR-driven | Version field + section skip forward compat; additive-only sau v1.0; breaking change cần ADR mới. |
| §0.3.5 | **Refuse over guess** — error rõ ràng | Corrupted file → hard error với diagnostic code; không "best effort" parse. |
| §0.3.6 | **Explicit > implicit** | Type table dedup tường minh; mỗi function khai báo signature trước code; cross-module call giữ nguyên path string. |
| §0.2 | **Tam phân first-class** | Trit/Tryte/Integer/Long encoding là primitive trong type table + constant pool; không "cào về integer". |

### Why LEB128 varint (not fixed-width u32)

- **Compactness**: IR có rất nhiều small integers (`ValueId`, `BlockId`,
  `ConstId`, counts). Hầu hết < 127 → 1 byte each thay vì 4.
- **Prior art**: Wasm, DWARF, Android DEX đều dùng LEB128.
- **Tradeoff**: decoding chậm hơn fixed-width ~10-15%. Acceptable vì `.triv`
  không phải hot path (không phải JIT instruction stream). AOT và JIT đọc
  `.triv` một lần lúc load, sau đó chạy từ in-memory IR.

### Why little-endian

- x86-64 và ARM64 (default config) là little-endian. RISC-V cũng LE-default.
- Big-endian hardware không còn phổ biến cho desktop/server.
- Nếu ternary hardware tương lai (v∞) là big-endian: reader thêm `bswap` lúc
  load — cost một lần, không ảnh hưởng runtime.

### Why section-based (not flat stream)

- **Forward compatibility**: reader skip unknown section → tool cũ đọc được
  file mới.
- **Parallel parsing**: constant pool và code section có thể parse riêng (hữu
  ích cho large programs ở v0.7+).
- **Tooling**: hex dump, `triet inspect`, debugger có thể locate section nhanh
  mà không cần parse toàn bộ.
- **Prior art**: ELF, Wasm, Java class file đều là section-based.

### Why separate function table and code section

Chia tách signature và body:

- **CAS hash (v0.5)**: `iface_hash` chỉ hash function table (signature), không
  hash code → thay đổi implementation không invalidate interface hash.
- **Lazy loading (v0.7+)**: load signatures trước để resolve cross-module
  calls, defer code parsing đến khi cần.
- **Tooling**: `triet inspect --signatures` chỉ cần parse section 3, không cần
  decode toàn bộ instructions.

### Why type table dedup

- Nhiều function params/return types trùng nhau → table dedup tránh encode lại
  `Integer` 100 lần.
- Type index là permanent ID cho type trong file — dùng cho ABI hash.

### Why builtin IDs, not full path strings

Builtins (`println`, `assert`, etc.) là finite set, compiler-known. Encoding
dạng 1-byte ID compact hơn string `"std.io.println"`. Khi cần (v0.4 stdlib
expand): thêm builtin IDs additive. User-defined functions gọi qua
`call_cross_module` với path string — không conflate hai mechanism.

## Alternatives considered

### A1. Postcard / bincode / MessagePack (off-the-shelf Rust serde)

**Reject.**

Pro: zero implementation effort (`#[derive(Serialize, Deserialize)]`).

Con:
- **Không có version field** — serde format thay đổi theo struct layout.
  Rename field → breaking change. Versioning phải tự build bên ngoài.
- **Không self-describing** — tooling không thể đọc `.triv` mà không có Rust
  struct definition.
- **Không control được encoding** — Compactness và varint decisions bị
  framework quyết định.
- **Không stable** — `bincode` default không có spec; behavior đổi giữa
  versions.

Triết IR cần ổn định 10+ năm (đến v∞). Coupling wire format vào một serde
library version là unacceptable.

### A2. Flat buffer (same as in-memory Rust layout)

**Reject.**

- Padding không deterministic → không hash được cho CAS.
- Endianness-dependent → không portable.
- Không có magic/version/sections → corrupt file không detect được.

### A3. Text-based (S-expression, JSON, YAML)

**Reject.**

- Quá verbose (10-20× binary size) → không phù hợp cho storage + CAS hash.
- Parse cost cao hơn binary varint.
- JSON không có comment → khó embed debug metadata.
- S-expression có thể dùng cho debug dump (`triet inspect --text`), nhưng
  không phải canonical format.

### A4. Wasm-style type section + code section

**Consider (partial adopt).**

Wasm dùng type section cho function signatures và code section cho bodies —
cùng pattern tách signature/body ta đang chọn. Nhưng Wasm type system là
value types (i32, i64, f32, f64) — đơn giản hơn Triết (Trit, Tryte, Integer,
Long, Trilean, String, Unit, Nullable, struct, enum, closure). Wasm không có
Ł3/K3 logic ops.

Kết luận: adopt section layout pattern từ Wasm (section id + size, skip
unknown), nhưng design type encoding riêng cho Triết.

## Hậu quả

**Tích cực:**
- `.triv` là canonical format cho Triết IR — mọi backend đọc cùng format.
- Version field + section skip cho forward compatibility từ ngày đầu.
- LEB128 varint compact — typical function body < 100 bytes.
- Type table dedup + constant pool dedup tránh bloat.
- Tách function table / code → CAS hash (v0.5) + lazy loading (v0.7) khả thi.
- Magic bytes `triv` cho `file(1)` detection.

**Tiêu cực:**
- Binary format không human-readable → cần `triet inspect` tool cho debugging.
- LEB128 decode cost (dù một lần lúc load).
- Opcode table cần maintained khi thêm instructions (additive thôi, nhưng vẫn
  phải giữ order).

**Implementation plan:**
- v0.3.9: `triet-ir` module `serde.rs` — `write_program(&IrProgram) →
  Vec<u8>`, `read_program(&[u8]) -> Result<IrProgram, TrivError>`.
- v0.3.10: CLI `triet build` — parse + lower + serialize → `.triv` file;
  `triet run foo.triv` — deserialize + execute.
- v0.3.11: Round-trip tests: mọi `examples/*.tri` parse → lower → serialize →
  deserialize → execute → same output.

**Breaking change policy:**
- Trước v1.0: opcode renumber có thể xảy ra (cần update reader/writer). Sau
  v0.3.9: mỗi thay đổi phải update cả serializer + deserializer + snapshot
  tests.
- Sau v1.0: chỉ additive — thêm opcodes, thêm section, thêm type discriminant.
  Không thay đổi ngữ nghĩa opcode hiện có.

## Error code namespace

| Code | Description |
|------|-------------|
| `E2102` | Unsupported `.triv` major version |
| `E2103` | Corrupted `.triv` file (truncated, bad magic, invalid UTF-8) |
| `E2104` | Unknown type discriminant in type table |
| `E2105` | Unknown opcode in code section |
| `E2106` | Section size mismatch |

## References

- [ADR-0007](0007-ir-design.md) — IR design (register SSA).
- [ADR-0005](0005-module-system.md) — Module system, `AbsolutePath` dot-path
  convention.
- [WebAssembly Binary Format](https://webassembly.github.io/spec/core/binary/) —
  section layout, LEB128 varint, type section prior art.
- [LEB128 encoding](https://en.wikipedia.org/wiki/LEB128) — DWARF/ Wasm varint
  spec.
- [Java Class File Format](https://docs.oracle.com/javase/specs/jvms/se24/html/jvms-4.html) —
  section-based binary format prior art, 30+ year track record.
- [ELF Specification](https://refspecs.linuxfoundation.org/elf/elf.pdf) —
  section-based binary format, magic + endianness convention.
- [LLVM Bitcode Format](https://llvm.org/docs/BitCodeFormat.html) — alternative
  binary IR format (prior art).

## Liên quan

- [ADR-0007](0007-ir-design.md) — IR shape (register SSA).
- ADR-0009 (sẽ viết, v0.4): ABI metadata — đọc từ `.triv` function table.
- ADR-0012 (sẽ viết, v0.5): CAS hash — `iface_hash` trên function table,
  `impl_hash` trên code section.
- [ROADMAP § v0.3](../../ROADMAP.md) — Phase deliverables + gates.

---

*Quyết định này đóng băng wire format cho `.triv`. Serializer/deserializer
implementation ở v0.3.9. Breaking change ở binary format cần ADR riêng.*
