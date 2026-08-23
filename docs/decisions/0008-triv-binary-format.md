# ADR 0008 — `.triv` bytecode binary format

**Status:** Decided. Applies to v0.3.9+ (serializer/deserializer) and all `.triv` readers (CLI, JIT, AOT, trytecode). It is the official wire format for Triết IR according to [ADR-0007 § Wire format](0007-ir-design.md).

**Issue:** ADR-0007 defines the IR shape (register SSA, type-tagged, virtual registers). However, in-memory Rust types (`enum Instruction`, `struct Function`, ...) are not a wire format — they lack magic bytes, a version field, or a section structure for forward compatibility. A binary format is required:

- **Deterministic**: same `IrProgram` → same byte sequence (for CAS hash v0.5).
- **Compact**: varint encoding, constant pool deduplication, string interning.
- **Versioned**: magic + version field, additive-only after v1.0.
- **Self-describing**: section layout allows tooling to read the file without the compiler source.
- **Stable**: v1.0 freeze; backends must be able to read older `.triv` files.

This ADR locks the binary format before the serialization/deserialization implementation (v0.3.9).

## Decision

`.triv` is a binary format with:

1. **Magic bytes** `0x74 0x72 0x69 0x76` ("triv" ASCII).
2. **32-bit version** (little-endian) — currently `4` (centralized history below).

**Version history (canonical — single source of truth):**

| Version | Phase / ADR | Change | Reader behavior on encounter |
|---|---|---|---|
| `1` | v0.3 initial release ([ADR-0008](0008-triv-binary-format.md), this ADR) | Initial format: magic + version + section_count + 4 sections (types/constants/functions/code). | n/a (oldest readable) |
| `2` | v0.3.x.ternary ([ADR-0010](0010-ternary-native-ir.md)) | Added `BR_TRILEAN` opcode (0xB4). | v1 readers emit `UnknownOpcode` on 0xB4. |
| `3` | v0.4 ([ADR-0012](0012-witness-table-dispatch.md)) | Added `WITNESS_CALL` opcode (0x93) + new `witness_tables` section (5). | v2 readers emit `UnknownOpcode` on 0x93. |
| `4` | v0.7.3.1 ([ADR-0019 Addendum §A1](0019-self-hosting-compiler-bootstrap.md)) | Added type discriminants 8 (Vector) + 9 (HashMap). | v3 readers emit `UnknownTypeDiscriminant` on 8/9. |
| `5` | v0.7.4.3-error ([ADR-0020 §7](0020-outcome-error-handling.md), pending impl) | Added type discriminant 10 (Outcome with `allow_null_lag: bool`) + 6 opcodes 0xC1–0xC6 (`OUTCOME_NEW_POSITIVE/NEGATIVE/NULL`, `OUTCOME_DISCRIMINANT`, `OUTCOME_UNWRAP_VALUE/ERROR`). | v4 readers emit `UnknownTypeDiscriminant` on 10 / `UnknownOpcode` on 0xC1–0xC6. |
| `6` | v0.9.x.atomic.2 ([ADR-0028 §1](0028-atomic-primitive.md)) | Added `TypeTag::Atomic(T)` (disc 11) + 10 atomic builtin IDs 33-42. | v5 readers emit `UnknownTypeDiscriminant` on 11. |
| `7` | v0.10.x.thread.1 ([ADR-0026 v2 §3](0016-concentration-byos-actor-model-v2.md)) | Added 2 raw-thread builtins IDs 43-44 (`RawThreadSpawn`, `RawThreadJoin`). | v6 readers emit `UnknownBuiltin(43/44)`. |
| `8` | v0.11.x.jit.4.agg.opaque ([ADR-0036](0036-typetag-opaque-aggregate.md)) | Added `TypeTag::Opaque` (disc 12) for user-defined aggregates (struct/enum/generic), disambiguating them from true `TypeTag::Unit`. Also retroactively added the missing disc 11 (`TypeTag::Atomic`) reader arm. Patch bump per §"Version compatibility." | v7 readers emit `UnknownTypeDiscriminant` on 12. |

Each bump is **additive-only** per the §"Version compatibility" rules below — no semantic changes to existing sections/opcodes. Older readers refuse cleanly when encountering newer features, never silently misinterpreting them.
3. **Section-based layout** — each section contains a `section_id` (1 byte) + `section_size` (u32 LE). Unknown section → skip, without error.
4. **Little-endian** for multi-byte integers (synchronized with primary target CPUs: x86-64, ARM64, RISC-V).
5. **LEB128 unsigned varint** for all small integers (`ValueId`, `BlockId`, `FuncId`, `ConstId`, counts, field indices).
6. **Length-prefixed UTF-8** for strings (LEB128 length + bytes).

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
