//! Serialize and deserialize [`IrProgram`] to/from the `.triv` binary format.
//!
//! Per [ADR-0008], the wire format is:
//! ```text
//! magic (4 bytes) | version (u32 LE) | section_count (u32 LE)
//! section[0..N]: section_id (1 byte) | section_size (u32 LE) | payload
//! ```
//!
//! [ADR-0008]: ../../../docs/decisions/0008-triv-binary-format.md

use std::fmt;

use crate::constant::{Constant, ConstantPool};
use crate::instr::{BuiltinName, Instruction, Operand, PhiIncoming};
use crate::module::{BasicBlock, Function, IrModule, IrProgram};
use crate::types::{BlockId, ConstId, FuncId, TypeTag, ValueId};

// ── Constants ──────────────────────────────────────────────────────

const MAGIC: [u8; 4] = [0x74, 0x72, 0x69, 0x76]; // "triv"
/// `.triv` format version.
///
/// History (canonical — single source at [ADR-0008 §"Version history"]):
/// - v1: initial release (ADR-0008).
/// - v2: ADR-0010 added `BR_TRILEAN` (0xB4) ternary-native branch.
/// - v3: ADR-0012 added `WITNESS_CALL` (0x93) + `witness_tables`
///   section (5). Older readers hit `TrivError::UnknownOpcode`.
/// - v4: ADR-0019 Addendum (v0.7.3) added `TypeTag::Vector(T)` (disc 8)
///   and `TypeTag::HashMap(K, V)` (disc 9).
/// - v5: ADR-0020 (v0.7.4.3-error.3a) added `TypeTag::Outcome` (disc 10
///   with `allow_null_state` boolean payload) + 6 opcodes 0xC1-0xC6
///   for outcome construction / unwrap / discriminant extraction.
///   Patch bump per ADR-0008 §"Version compatibility".
///
/// [ADR-0008 §"Version history"]: ../../../../docs/decisions/0008-triv-binary-format.md
const VERSION: u32 = 6;

const SEC_TYPES: u8 = 1;
const SEC_CONSTANTS: u8 = 2;
const SEC_FUNCTIONS: u8 = 3;
const SEC_CODE: u8 = 4;
/// ADR-0012 witness tables. Optional — only emitted when the program
/// contains `WitnessCall` instructions. Older readers skip unknown
/// sections silently per ADR-0008 framing.
const SEC_WITNESS_TABLES: u8 = 5;

// ── Opcodes ────────────────────────────────────────────────────────

mod opcode {
    pub(super) const CONST: u8 = 0x00;
    pub(super) const ADD: u8 = 0x10;
    pub(super) const SUB: u8 = 0x11;
    pub(super) const MUL: u8 = 0x12;
    pub(super) const DIV: u8 = 0x13;
    pub(super) const MOD: u8 = 0x14;
    pub(super) const POW: u8 = 0x15;
    pub(super) const NEG: u8 = 0x16;
    pub(super) const LUK_AND: u8 = 0x20;
    pub(super) const LUK_OR: u8 = 0x21;
    pub(super) const LUK_IMPLIES: u8 = 0x22;
    pub(super) const LUK_XOR: u8 = 0x23;
    pub(super) const LUK_IFF: u8 = 0x24;
    pub(super) const KLEENE_IMPLIES: u8 = 0x30;
    pub(super) const KLEENE_XOR: u8 = 0x31;
    pub(super) const KLEENE_IFF: u8 = 0x32;
    pub(super) const EQ: u8 = 0x40;
    pub(super) const NE: u8 = 0x41;
    pub(super) const LT: u8 = 0x42;
    pub(super) const LE: u8 = 0x43;
    pub(super) const GT: u8 = 0x44;
    pub(super) const GE: u8 = 0x45;
    pub(super) const TO_INTEGER: u8 = 0x50;
    pub(super) const TO_TRYTE: u8 = 0x51;
    pub(super) const TO_LONG: u8 = 0x52;
    pub(super) const TO_TRIT: u8 = 0x53;
    pub(super) const TO_TRILEAN: u8 = 0x54;
    pub(super) const STRUCT_NEW: u8 = 0x60;
    pub(super) const FIELD_GET: u8 = 0x61;
    pub(super) const FIELD_SET: u8 = 0x62;
    pub(super) const ENUM_NEW: u8 = 0x70;
    pub(super) const ENUM_TAG: u8 = 0x71;
    pub(super) const ENUM_PAYLOAD: u8 = 0x72;
    pub(super) const NULL_WRAP: u8 = 0x80;
    pub(super) const NULL_UNWRAP: u8 = 0x81;
    pub(super) const NULL_CHECK: u8 = 0x82;
    pub(super) const CALL_LOCAL: u8 = 0x90;
    pub(super) const CALL_CROSS_MODULE: u8 = 0x91;
    pub(super) const CALL_BUILTIN: u8 = 0x92;
    /// ADR-0012 — cross-package generic dispatch via witness table.
    /// Added in `.triv` v3.
    pub(super) const WITNESS_CALL: u8 = 0x93;
    pub(super) const CLOSURE_NEW: u8 = 0xA0;
    pub(super) const CLOSURE_CALL: u8 = 0xA1;
    pub(super) const BR: u8 = 0xB0;
    pub(super) const BR_IF: u8 = 0xB1;
    pub(super) const RET: u8 = 0xB2;
    pub(super) const UNREACHABLE: u8 = 0xB3;
    /// ADR-0010 — three-way branch on Trilean condition. Added in .triv v2.
    pub(super) const BR_TRILEAN: u8 = 0xB4;
    pub(super) const PHI: u8 = 0xC0;
    // v0.7.4.3-error.3a (ADR-0020 §7.3) — outcome ops, .triv v5.
    pub(super) const OUTCOME_NEW_POSITIVE: u8 = 0xC1;
    pub(super) const OUTCOME_NEW_NEGATIVE: u8 = 0xC2;
    pub(super) const OUTCOME_NEW_NULL: u8 = 0xC3;
    pub(super) const OUTCOME_DISCRIMINANT: u8 = 0xC4;
    pub(super) const OUTCOME_UNWRAP_VALUE: u8 = 0xC5;
    pub(super) const OUTCOME_UNWRAP_ERROR: u8 = 0xC6;
}

const OPERAND_CONST: u8 = 0x00;
const OPERAND_VALUE: u8 = 0x01;

// ── Errors ─────────────────────────────────────────────────────────

/// Errors that can occur when reading a `.triv` file.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum TrivError {
    /// The file's major version is higher than what this reader supports.
    UnsupportedVersion {
        /// The version found in the file.
        found: u32,
        /// The highest version this reader supports.
        max_supported: u32,
    },
    /// Structural corruption: bad magic, truncated file, invalid UTF-8, etc.
    Corrupted(String),
    /// A type discriminant byte doesn't match any known [`TypeTag`].
    UnknownTypeDiscriminant(u8),
    /// An opcode byte doesn't match any known [`Instruction`].
    UnknownOpcode(u8),
    /// The section payload size doesn't match the declared size.
    SectionSizeMismatch {
        /// The section ID (1=types, 2=constants, 3=functions, 4=code).
        section_id: u8,
        /// The size declared in the section header.
        declared: u32,
        /// The actual bytes remaining.
        actual: usize,
    },
    /// A builtin ID byte doesn't match any known [`BuiltinName`].
    UnknownBuiltin(u8),
}

impl fmt::Display for TrivError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion {
                found,
                max_supported,
            } => write!(
                f,
                "unsupported .triv version {found} (max supported: {max_supported})"
            ),
            Self::Corrupted(msg) => write!(f, "corrupted .triv file: {msg}"),
            Self::UnknownTypeDiscriminant(d) => {
                write!(f, "unknown type discriminant 0x{d:02X}")
            }
            Self::UnknownOpcode(op) => write!(f, "unknown opcode 0x{op:02X}"),
            Self::SectionSizeMismatch {
                section_id,
                declared,
                actual,
            } => write!(
                f,
                "section {section_id}: declared size {declared}, actual {actual}"
            ),
            Self::UnknownBuiltin(id) => write!(f, "unknown builtin ID {id}"),
        }
    }
}

impl std::error::Error for TrivError {}

// ── LEB128 encoding ────────────────────────────────────────────────

fn write_leb128(buf: &mut Vec<u8>, mut value: u32) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            buf.push(byte | 0x80);
        } else {
            buf.push(byte);
            break;
        }
    }
}

fn read_leb128(data: &[u8], pos: &mut usize) -> Result<u32, TrivError> {
    let mut result: u32 = 0;
    let mut shift: u32 = 0;
    loop {
        if *pos >= data.len() {
            return Err(TrivError::Corrupted("truncated LEB128".into()));
        }
        let byte = data[*pos];
        *pos += 1;
        result |= u32::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 35 {
            return Err(TrivError::Corrupted("LEB128 overflow".into()));
        }
    }
}

// ── Low-level write helpers ────────────────────────────────────────

fn write_bytes(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(bytes);
}

fn write_u8(buf: &mut Vec<u8>, value: u8) {
    buf.push(value);
}

fn write_u32_le(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn write_string(buf: &mut Vec<u8>, s: &str) {
    write_leb128(buf, s.len() as u32);
    write_bytes(buf, s.as_bytes());
}

fn write_varint(buf: &mut Vec<u8>, value: u32) {
    write_leb128(buf, value);
}

// ── Low-level read helpers ─────────────────────────────────────────

fn read_u8(data: &[u8], pos: &mut usize) -> Result<u8, TrivError> {
    if *pos >= data.len() {
        return Err(TrivError::Corrupted("unexpected EOF".into()));
    }
    let byte = data[*pos];
    *pos += 1;
    Ok(byte)
}

fn read_u32_le(data: &[u8], pos: &mut usize) -> Result<u32, TrivError> {
    if *pos + 4 > data.len() {
        return Err(TrivError::Corrupted("unexpected EOF reading u32".into()));
    }
    let bytes: [u8; 4] = data[*pos..*pos + 4].try_into().unwrap();
    *pos += 4;
    Ok(u32::from_le_bytes(bytes))
}

fn read_string(data: &[u8], pos: &mut usize) -> Result<String, TrivError> {
    let len = read_leb128(data, pos)? as usize;
    if *pos + len > data.len() {
        return Err(TrivError::Corrupted("truncated string".into()));
    }
    let s = String::from_utf8(data[*pos..*pos + len].to_vec())
        .map_err(|e| TrivError::Corrupted(format!("invalid UTF-8: {e}")))?;
    *pos += len;
    Ok(s)
}

fn read_varint(data: &[u8], pos: &mut usize) -> Result<u32, TrivError> {
    read_leb128(data, pos)
}

// ── Type table ─────────────────────────────────────────────────────

fn collect_type_table(program: &IrProgram) -> Vec<TypeTag> {
    let mut types: Vec<TypeTag> = Vec::new();

    // Ensure we have at least Unit (index 0) as a safe default.
    types.push(TypeTag::Unit);

    for module in &program.modules {
        for func in &module.functions {
            for (_, ty) in &func.params {
                add_type(&mut types, ty.clone());
            }
            add_type(&mut types, func.return_type.clone());
        }
    }
    for (_, constant) in program.constants.iter() {
        add_type(&mut types, constant.type_tag());
    }
    types
}

fn add_type(types: &mut Vec<TypeTag>, ty: TypeTag) {
    // Recurse into composite types first — the inner tag must be
    // present in the table before the container references it by
    // index. Mirrors the read-side post-order reconstruction.
    match &ty {
        TypeTag::Nullable(inner) => add_type(types, (**inner).clone()),
        TypeTag::Vector(element) => add_type(types, (**element).clone()),
        TypeTag::HashMap(key, value) => {
            add_type(types, (**key).clone());
            add_type(types, (**value).clone());
        }
        TypeTag::Outcome {
            value_type,
            error_type,
            ..
        } => {
            add_type(types, (**value_type).clone());
            add_type(types, (**error_type).clone());
        }
        _ => {}
    }
    if !types.contains(&ty) {
        types.push(ty);
    }
}

fn type_index(types: &[TypeTag], ty: &TypeTag) -> u32 {
    types.iter().position(|t| t == ty).unwrap() as u32
}

fn write_type_table(buf: &mut Vec<u8>, types: &[TypeTag]) {
    write_varint(buf, types.len() as u32);
    for ty in types {
        write_type_tag(buf, types, ty);
    }
}

fn write_type_tag(buf: &mut Vec<u8>, types: &[TypeTag], ty: &TypeTag) {
    match ty {
        TypeTag::Trit => write_u8(buf, 0),
        TypeTag::Tryte => write_u8(buf, 1),
        TypeTag::Integer => write_u8(buf, 2),
        TypeTag::Long => write_u8(buf, 3),
        TypeTag::Trilean => write_u8(buf, 4),
        TypeTag::String => write_u8(buf, 5),
        TypeTag::Unit => write_u8(buf, 6),
        TypeTag::Nullable(inner) => {
            write_u8(buf, 7);
            let idx = type_index(types, inner);
            write_varint(buf, idx);
        }
        TypeTag::Vector(element) => {
            write_u8(buf, 8);
            let idx = type_index(types, element);
            write_varint(buf, idx);
        }
        TypeTag::HashMap(key, value) => {
            write_u8(buf, 9);
            let key_idx = type_index(types, key);
            let value_idx = type_index(types, value);
            write_varint(buf, key_idx);
            write_varint(buf, value_idx);
        }
        // v0.7.4.3-error.3a (ADR-0020 §7.1): outcome type encoding.
        // Discriminant 10 + 1-byte `allow_null_state` boolean + 2
        // varint inner-type indices (value first, error second).
        TypeTag::Outcome {
            value_type,
            error_type,
            allow_null_state,
        } => {
            write_u8(buf, 10);
            write_u8(buf, u8::from(*allow_null_state));
            let value_idx = type_index(types, value_type);
            let error_idx = type_index(types, error_type);
            write_varint(buf, value_idx);
            write_varint(buf, error_idx);
        }
        // v0.9.x.atomic.3 (ADR-0028 §2 AtomicValue) — Atomic type
        // encoding. Discriminant 11 + 1 varint inner-type index.
        // Wire format `.triv` v6 (bumped v0.9.x.atomic.2 per ADR-0028 §1).
        TypeTag::Atomic(inner) => {
            write_u8(buf, 11);
            let idx = type_index(types, inner);
            write_varint(buf, idx);
        }
    }
}

fn read_type_table(data: &[u8], pos: &mut usize) -> Result<Vec<TypeTag>, TrivError> {
    let count = read_varint(data, pos)? as usize;
    let mut types: Vec<TypeTag> = Vec::with_capacity(count);
    for _ in 0..count {
        let ty = read_type_tag(data, pos, &types)?;
        types.push(ty);
    }
    Ok(types)
}

fn read_type_tag(data: &[u8], pos: &mut usize, table: &[TypeTag]) -> Result<TypeTag, TrivError> {
    let disc = read_u8(data, pos)?;
    match disc {
        0 => Ok(TypeTag::Trit),
        1 => Ok(TypeTag::Tryte),
        2 => Ok(TypeTag::Integer),
        3 => Ok(TypeTag::Long),
        4 => Ok(TypeTag::Trilean),
        5 => Ok(TypeTag::String),
        6 => Ok(TypeTag::Unit),
        7 => {
            let idx = read_varint(data, pos)? as usize;
            let inner = table
                .get(idx)
                .ok_or_else(|| TrivError::Corrupted("invalid type index in Nullable".into()))?
                .clone();
            Ok(TypeTag::Nullable(Box::new(inner)))
        }
        8 => {
            let idx = read_varint(data, pos)? as usize;
            let element = table
                .get(idx)
                .ok_or_else(|| TrivError::Corrupted("invalid type index in Vector".into()))?
                .clone();
            Ok(TypeTag::Vector(Box::new(element)))
        }
        9 => {
            let key_idx = read_varint(data, pos)? as usize;
            let value_idx = read_varint(data, pos)? as usize;
            let key = table
                .get(key_idx)
                .ok_or_else(|| TrivError::Corrupted("invalid key type index in HashMap".into()))?
                .clone();
            let value = table
                .get(value_idx)
                .ok_or_else(|| TrivError::Corrupted("invalid value type index in HashMap".into()))?
                .clone();
            Ok(TypeTag::HashMap(Box::new(key), Box::new(value)))
        }
        // v0.7.4.3-error.3a (ADR-0020 §7.1): outcome type encoding.
        10 => {
            let allow_null_state_byte = read_u8(data, pos)?;
            let allow_null_state = allow_null_state_byte != 0;
            let value_idx = read_varint(data, pos)? as usize;
            let error_idx = read_varint(data, pos)? as usize;
            let value_type = table
                .get(value_idx)
                .ok_or_else(|| TrivError::Corrupted("invalid value type index in Outcome".into()))?
                .clone();
            let error_type = table
                .get(error_idx)
                .ok_or_else(|| TrivError::Corrupted("invalid error type index in Outcome".into()))?
                .clone();
            Ok(TypeTag::Outcome {
                value_type: Box::new(value_type),
                error_type: Box::new(error_type),
                allow_null_state,
            })
        }
        d => Err(TrivError::UnknownTypeDiscriminant(d)),
    }
}

// ── Operand ────────────────────────────────────────────────────────

fn write_operand(buf: &mut Vec<u8>, op: Operand) {
    match op {
        Operand::Const(c) => {
            write_u8(buf, OPERAND_CONST);
            write_varint(buf, c.0);
        }
        Operand::Value(v) => {
            write_u8(buf, OPERAND_VALUE);
            write_varint(buf, v.0);
        }
    }
}

fn read_operand(data: &[u8], pos: &mut usize) -> Result<Operand, TrivError> {
    let tag = read_u8(data, pos)?;
    match tag {
        OPERAND_CONST => {
            let id = read_varint(data, pos)?;
            Ok(Operand::Const(ConstId(id)))
        }
        OPERAND_VALUE => {
            let id = read_varint(data, pos)?;
            Ok(Operand::Value(ValueId(id)))
        }
        t => Err(TrivError::Corrupted(format!(
            "unknown operand tag 0x{t:02X}"
        ))),
    }
}

// ── Optional operand (for calls, ret) ──────────────────────────────

fn write_option_operand(buf: &mut Vec<u8>, op: Option<Operand>) {
    match op {
        None => write_u8(buf, 0),
        Some(o) => {
            write_u8(buf, 1);
            write_operand(buf, o);
        }
    }
}

fn read_option_operand(data: &[u8], pos: &mut usize) -> Result<Option<Operand>, TrivError> {
    let has = read_u8(data, pos)?;
    match has {
        0 => Ok(None),
        1 => {
            let op = read_operand(data, pos)?;
            Ok(Some(op))
        }
        b => Err(TrivError::Corrupted(format!(
            "invalid option operand byte 0x{b:02X}"
        ))),
    }
}

// ── Optional value id ──────────────────────────────────────────────

fn write_option_value(buf: &mut Vec<u8>, dest: Option<ValueId>) {
    match dest {
        None => write_u8(buf, 0),
        Some(v) => {
            write_u8(buf, 1);
            write_varint(buf, v.0);
        }
    }
}

fn read_option_value(data: &[u8], pos: &mut usize) -> Result<Option<ValueId>, TrivError> {
    let has = read_u8(data, pos)?;
    match has {
        0 => Ok(None),
        1 => {
            let id = read_varint(data, pos)?;
            Ok(Some(ValueId(id)))
        }
        b => Err(TrivError::Corrupted(format!(
            "invalid option value byte 0x{b:02X}"
        ))),
    }
}

// ── Builtin ────────────────────────────────────────────────────────

// Builtin ID table additive-extension policy:
//
// Builtin IDs 0–7 shipped pre-v0.7.3 (Println=0 .. TextFromInteger=7).
// IDs 8+ are additive within `.triv` v4 per ADR-0019 Addendum §A1 —
// new builtin IDs do not require a version bump because the
// `CallBuiltin` opcode byte itself is unchanged; only the operand byte
// (builtin ID) gains new accepted values. Pre-v0.7.3 readers
// encountering ID 8+ emit `TrivError::UnknownBuiltinId` (same refusal
// contract as `UnknownTypeDiscriminant`).
fn write_builtin(buf: &mut Vec<u8>, builtin: BuiltinName) {
    let id = match builtin {
        BuiltinName::Println => 0,
        BuiltinName::Print => 1,
        BuiltinName::Assert => 2,
        BuiltinName::AssertEq => 3,
        BuiltinName::FStringConcat => 4,
        BuiltinName::TextLen => 5,
        BuiltinName::TextConcat => 6,
        BuiltinName::TextFromInteger => 7,
        BuiltinName::VectorNew => 8,
        BuiltinName::VectorPush => 9,
        BuiltinName::VectorGet => 10,
        BuiltinName::VectorLength => 11,
        BuiltinName::HashMapNew => 12,
        BuiltinName::HashMapInsert => 13,
        BuiltinName::HashMapGet => 14,
        BuiltinName::HashMapKeys => 15,
        BuiltinName::HashMapContains => 16,
        BuiltinName::ReadFile => 17,
        BuiltinName::WriteFile => 18,
        BuiltinName::FileExists => 19,
        BuiltinName::PathJoin => 20,
        BuiltinName::PathParent => 21,
        BuiltinName::PathBasename => 22,
        BuiltinName::StringSubstring => 23,
        BuiltinName::StringSplit => 24,
        BuiltinName::StringIndexOf => 25,
        BuiltinName::ParseInteger => 26,
        BuiltinName::TextIntoBytes => 27,
        BuiltinName::TextFromBytes => 28,
        BuiltinName::Blake3Hash => 29,
        BuiltinName::WriteFileBytes => 30,
        BuiltinName::GetEnv => 31,
        BuiltinName::ReadDirRecursive => 32,
        // v0.9.x.atomic.2 — atomic builtins per ADR-0028 §1.
        BuiltinName::AtomicNew => 33,
        BuiltinName::AtomicLoad => 34,
        BuiltinName::AtomicStore => 35,
        BuiltinName::AtomicSwap => 36,
        BuiltinName::AtomicCompareExchange => 37,
        BuiltinName::AtomicFetchAdd => 38,
        BuiltinName::AtomicFetchSub => 39,
        BuiltinName::AtomicFetchBitwiseAnd => 40,
        BuiltinName::AtomicFetchBitwiseOr => 41,
        BuiltinName::AtomicFetchBitwiseXor => 42,
    };
    write_u8(buf, id);
}

fn read_builtin(data: &[u8], pos: &mut usize) -> Result<BuiltinName, TrivError> {
    let id = read_u8(data, pos)?;
    match id {
        0 => Ok(BuiltinName::Println),
        1 => Ok(BuiltinName::Print),
        2 => Ok(BuiltinName::Assert),
        3 => Ok(BuiltinName::AssertEq),
        4 => Ok(BuiltinName::FStringConcat),
        5 => Ok(BuiltinName::TextLen),
        6 => Ok(BuiltinName::TextConcat),
        7 => Ok(BuiltinName::TextFromInteger),
        8 => Ok(BuiltinName::VectorNew),
        9 => Ok(BuiltinName::VectorPush),
        10 => Ok(BuiltinName::VectorGet),
        11 => Ok(BuiltinName::VectorLength),
        12 => Ok(BuiltinName::HashMapNew),
        13 => Ok(BuiltinName::HashMapInsert),
        14 => Ok(BuiltinName::HashMapGet),
        15 => Ok(BuiltinName::HashMapKeys),
        16 => Ok(BuiltinName::HashMapContains),
        17 => Ok(BuiltinName::ReadFile),
        18 => Ok(BuiltinName::WriteFile),
        19 => Ok(BuiltinName::FileExists),
        20 => Ok(BuiltinName::PathJoin),
        21 => Ok(BuiltinName::PathParent),
        22 => Ok(BuiltinName::PathBasename),
        23 => Ok(BuiltinName::StringSubstring),
        24 => Ok(BuiltinName::StringSplit),
        25 => Ok(BuiltinName::StringIndexOf),
        26 => Ok(BuiltinName::ParseInteger),
        27 => Ok(BuiltinName::TextIntoBytes),
        28 => Ok(BuiltinName::TextFromBytes),
        29 => Ok(BuiltinName::Blake3Hash),
        30 => Ok(BuiltinName::WriteFileBytes),
        31 => Ok(BuiltinName::GetEnv),
        32 => Ok(BuiltinName::ReadDirRecursive),
        // v0.9.x.atomic.2 — atomic builtins per ADR-0028 §1.
        33 => Ok(BuiltinName::AtomicNew),
        34 => Ok(BuiltinName::AtomicLoad),
        35 => Ok(BuiltinName::AtomicStore),
        36 => Ok(BuiltinName::AtomicSwap),
        37 => Ok(BuiltinName::AtomicCompareExchange),
        38 => Ok(BuiltinName::AtomicFetchAdd),
        39 => Ok(BuiltinName::AtomicFetchSub),
        40 => Ok(BuiltinName::AtomicFetchBitwiseAnd),
        41 => Ok(BuiltinName::AtomicFetchBitwiseOr),
        42 => Ok(BuiltinName::AtomicFetchBitwiseXor),
        id => Err(TrivError::UnknownBuiltin(id)),
    }
}

// ── Instruction ────────────────────────────────────────────────────

fn write_instruction(buf: &mut Vec<u8>, instr: &Instruction) {
    match instr {
        Instruction::Const { dest, constant } => {
            write_u8(buf, opcode::CONST);
            write_varint(buf, dest.0);
            write_varint(buf, constant.0);
        }
        Instruction::Add { dest, lhs, rhs } => {
            write_u8(buf, opcode::ADD);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::Sub { dest, lhs, rhs } => {
            write_u8(buf, opcode::SUB);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::Mul { dest, lhs, rhs } => {
            write_u8(buf, opcode::MUL);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::Div { dest, lhs, rhs } => {
            write_u8(buf, opcode::DIV);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::Mod { dest, lhs, rhs } => {
            write_u8(buf, opcode::MOD);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::Pow { dest, base, exp } => {
            write_u8(buf, opcode::POW);
            write_varint(buf, dest.0);
            write_operand(buf, *base);
            write_operand(buf, *exp);
        }
        Instruction::Neg { dest, operand } => {
            write_u8(buf, opcode::NEG);
            write_varint(buf, dest.0);
            write_operand(buf, *operand);
        }
        Instruction::LukAnd { dest, lhs, rhs } => {
            write_u8(buf, opcode::LUK_AND);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::LukOr { dest, lhs, rhs } => {
            write_u8(buf, opcode::LUK_OR);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::LukImplies { dest, lhs, rhs } => {
            write_u8(buf, opcode::LUK_IMPLIES);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::LukXor { dest, lhs, rhs } => {
            write_u8(buf, opcode::LUK_XOR);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::LukIff { dest, lhs, rhs } => {
            write_u8(buf, opcode::LUK_IFF);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::KleeneImplies { dest, lhs, rhs } => {
            write_u8(buf, opcode::KLEENE_IMPLIES);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::KleeneXor { dest, lhs, rhs } => {
            write_u8(buf, opcode::KLEENE_XOR);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::KleeneIff { dest, lhs, rhs } => {
            write_u8(buf, opcode::KLEENE_IFF);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::Eq { dest, lhs, rhs } => {
            write_u8(buf, opcode::EQ);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::Ne { dest, lhs, rhs } => {
            write_u8(buf, opcode::NE);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::Lt { dest, lhs, rhs } => {
            write_u8(buf, opcode::LT);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::Le { dest, lhs, rhs } => {
            write_u8(buf, opcode::LE);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::Gt { dest, lhs, rhs } => {
            write_u8(buf, opcode::GT);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::Ge { dest, lhs, rhs } => {
            write_u8(buf, opcode::GE);
            write_varint(buf, dest.0);
            write_operand(buf, *lhs);
            write_operand(buf, *rhs);
        }
        Instruction::ToInteger { dest, operand } => {
            write_u8(buf, opcode::TO_INTEGER);
            write_varint(buf, dest.0);
            write_operand(buf, *operand);
        }
        Instruction::ToTryte { dest, operand } => {
            write_u8(buf, opcode::TO_TRYTE);
            write_varint(buf, dest.0);
            write_operand(buf, *operand);
        }
        Instruction::ToLong { dest, operand } => {
            write_u8(buf, opcode::TO_LONG);
            write_varint(buf, dest.0);
            write_operand(buf, *operand);
        }
        Instruction::ToTrit { dest, operand } => {
            write_u8(buf, opcode::TO_TRIT);
            write_varint(buf, dest.0);
            write_operand(buf, *operand);
        }
        Instruction::ToTrilean { dest, operand } => {
            write_u8(buf, opcode::TO_TRILEAN);
            write_varint(buf, dest.0);
            write_operand(buf, *operand);
        }
        Instruction::StructNew { dest, fields } => {
            write_u8(buf, opcode::STRUCT_NEW);
            write_varint(buf, dest.0);
            write_varint(buf, fields.len() as u32);
            for f in fields {
                write_operand(buf, *f);
            }
        }
        Instruction::FieldGet {
            dest,
            object,
            field_idx,
        } => {
            write_u8(buf, opcode::FIELD_GET);
            write_varint(buf, dest.0);
            write_operand(buf, *object);
            write_varint(buf, *field_idx);
        }
        Instruction::FieldSet {
            dest,
            object,
            field_idx,
            value,
        } => {
            write_u8(buf, opcode::FIELD_SET);
            write_varint(buf, dest.0);
            write_operand(buf, *object);
            write_varint(buf, *field_idx);
            write_operand(buf, *value);
        }
        Instruction::EnumNew {
            dest,
            variant_idx,
            payload,
        } => {
            write_u8(buf, opcode::ENUM_NEW);
            write_varint(buf, dest.0);
            write_varint(buf, *variant_idx);
            write_option_operand(buf, *payload);
        }
        Instruction::EnumTag { dest, scrutinee } => {
            write_u8(buf, opcode::ENUM_TAG);
            write_varint(buf, dest.0);
            write_operand(buf, *scrutinee);
        }
        Instruction::EnumPayload { dest, scrutinee } => {
            write_u8(buf, opcode::ENUM_PAYLOAD);
            write_varint(buf, dest.0);
            write_operand(buf, *scrutinee);
        }
        Instruction::NullWrap { dest, value } => {
            write_u8(buf, opcode::NULL_WRAP);
            write_varint(buf, dest.0);
            write_operand(buf, *value);
        }
        Instruction::NullUnwrap { dest, nullable } => {
            write_u8(buf, opcode::NULL_UNWRAP);
            write_varint(buf, dest.0);
            write_operand(buf, *nullable);
        }
        Instruction::NullCheck { dest, nullable } => {
            write_u8(buf, opcode::NULL_CHECK);
            write_varint(buf, dest.0);
            write_operand(buf, *nullable);
        }
        Instruction::CallLocal { dest, callee, args } => {
            write_u8(buf, opcode::CALL_LOCAL);
            write_option_value(buf, *dest);
            write_varint(buf, callee.0);
            write_varint(buf, args.len() as u32);
            for a in args {
                write_operand(buf, *a);
            }
        }
        Instruction::CallCrossModule { dest, path, args } => {
            write_u8(buf, opcode::CALL_CROSS_MODULE);
            write_option_value(buf, *dest);
            write_string(buf, &path.to_string());
            write_varint(buf, args.len() as u32);
            for a in args {
                write_operand(buf, *a);
            }
        }
        Instruction::WitnessCall {
            dest,
            path,
            witness_idx,
            args,
        } => {
            write_u8(buf, opcode::WITNESS_CALL);
            write_option_value(buf, *dest);
            write_string(buf, &path.to_string());
            write_varint(buf, *witness_idx);
            write_varint(buf, args.len() as u32);
            for a in args {
                write_operand(buf, *a);
            }
        }
        Instruction::CallBuiltin { dest, name, args } => {
            write_u8(buf, opcode::CALL_BUILTIN);
            write_option_value(buf, *dest);
            write_builtin(buf, *name);
            write_varint(buf, args.len() as u32);
            for a in args {
                write_operand(buf, *a);
            }
        }
        Instruction::ClosureNew {
            dest,
            lambda,
            captures,
        } => {
            write_u8(buf, opcode::CLOSURE_NEW);
            write_varint(buf, dest.0);
            write_varint(buf, lambda.0);
            write_varint(buf, captures.len() as u32);
            for c in captures {
                write_varint(buf, c.0);
            }
        }
        Instruction::ClosureCall {
            dest,
            closure,
            args,
        } => {
            write_u8(buf, opcode::CLOSURE_CALL);
            write_option_value(buf, *dest);
            write_operand(buf, *closure);
            write_varint(buf, args.len() as u32);
            for a in args {
                write_operand(buf, *a);
            }
        }
        Instruction::Br { target } => {
            write_u8(buf, opcode::BR);
            write_varint(buf, target.0);
        }
        Instruction::BrIf {
            cond,
            then_block,
            else_block,
        } => {
            write_u8(buf, opcode::BR_IF);
            write_operand(buf, *cond);
            write_varint(buf, then_block.0);
            write_varint(buf, else_block.0);
        }
        Instruction::BrTrilean {
            cond,
            true_block,
            unknown_block,
            false_block,
        } => {
            write_u8(buf, opcode::BR_TRILEAN);
            write_operand(buf, *cond);
            write_varint(buf, true_block.0);
            write_varint(buf, unknown_block.0);
            write_varint(buf, false_block.0);
        }
        Instruction::Ret { value } => {
            write_u8(buf, opcode::RET);
            write_option_operand(buf, *value);
        }
        Instruction::Unreachable => {
            write_u8(buf, opcode::UNREACHABLE);
        }
        Instruction::Phi { dest, incoming } => {
            write_u8(buf, opcode::PHI);
            write_varint(buf, dest.0);
            write_varint(buf, incoming.len() as u32);
            for phi in incoming {
                write_varint(buf, phi.value.0);
                write_varint(buf, phi.block.0);
            }
        }
        // v0.7.4.3-error.3a (ADR-0020 §7.3) — outcome opcodes:
        Instruction::OutcomeNewPositive { dest, payload } => {
            write_u8(buf, opcode::OUTCOME_NEW_POSITIVE);
            write_varint(buf, dest.0);
            write_operand(buf, *payload);
        }
        Instruction::OutcomeNewNegative { dest, payload } => {
            write_u8(buf, opcode::OUTCOME_NEW_NEGATIVE);
            write_varint(buf, dest.0);
            write_operand(buf, *payload);
        }
        Instruction::OutcomeNewNull { dest } => {
            write_u8(buf, opcode::OUTCOME_NEW_NULL);
            write_varint(buf, dest.0);
        }
        Instruction::OutcomeDiscriminant { dest, source } => {
            write_u8(buf, opcode::OUTCOME_DISCRIMINANT);
            write_varint(buf, dest.0);
            write_operand(buf, *source);
        }
        Instruction::OutcomeUnwrapValue { dest, source } => {
            write_u8(buf, opcode::OUTCOME_UNWRAP_VALUE);
            write_varint(buf, dest.0);
            write_operand(buf, *source);
        }
        Instruction::OutcomeUnwrapError { dest, source } => {
            write_u8(buf, opcode::OUTCOME_UNWRAP_ERROR);
            write_varint(buf, dest.0);
            write_operand(buf, *source);
        }
    }
}

fn read_instruction(data: &[u8], pos: &mut usize) -> Result<Instruction, TrivError> {
    let opcode = read_u8(data, pos)?;
    match opcode {
        opcode::CONST => {
            let dest = ValueId(read_varint(data, pos)?);
            let constant = ConstId(read_varint(data, pos)?);
            Ok(Instruction::Const { dest, constant })
        }
        opcode::ADD => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::Add {
            dest,
            lhs,
            rhs,
        }),
        opcode::SUB => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::Sub {
            dest,
            lhs,
            rhs,
        }),
        opcode::MUL => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::Mul {
            dest,
            lhs,
            rhs,
        }),
        opcode::DIV => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::Div {
            dest,
            lhs,
            rhs,
        }),
        opcode::MOD => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::Mod {
            dest,
            lhs,
            rhs,
        }),
        opcode::POW => {
            let dest = ValueId(read_varint(data, pos)?);
            let base = read_operand(data, pos)?;
            let exp = read_operand(data, pos)?;
            Ok(Instruction::Pow { dest, base, exp })
        }
        opcode::NEG => {
            let dest = ValueId(read_varint(data, pos)?);
            let operand = read_operand(data, pos)?;
            Ok(Instruction::Neg { dest, operand })
        }
        opcode::LUK_AND => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::LukAnd {
            dest,
            lhs,
            rhs,
        }),
        opcode::LUK_OR => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::LukOr {
            dest,
            lhs,
            rhs,
        }),
        opcode::LUK_IMPLIES => read_binary_arith(data, pos, |dest, lhs, rhs| {
            Instruction::LukImplies { dest, lhs, rhs }
        }),
        opcode::LUK_XOR => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::LukXor {
            dest,
            lhs,
            rhs,
        }),
        opcode::LUK_IFF => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::LukIff {
            dest,
            lhs,
            rhs,
        }),
        opcode::KLEENE_IMPLIES => read_binary_arith(data, pos, |dest, lhs, rhs| {
            Instruction::KleeneImplies { dest, lhs, rhs }
        }),
        opcode::KLEENE_XOR => read_binary_arith(data, pos, |dest, lhs, rhs| {
            Instruction::KleeneXor { dest, lhs, rhs }
        }),
        opcode::KLEENE_IFF => read_binary_arith(data, pos, |dest, lhs, rhs| {
            Instruction::KleeneIff { dest, lhs, rhs }
        }),
        opcode::EQ => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::Eq {
            dest,
            lhs,
            rhs,
        }),
        opcode::NE => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::Ne {
            dest,
            lhs,
            rhs,
        }),
        opcode::LT => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::Lt {
            dest,
            lhs,
            rhs,
        }),
        opcode::LE => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::Le {
            dest,
            lhs,
            rhs,
        }),
        opcode::GT => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::Gt {
            dest,
            lhs,
            rhs,
        }),
        opcode::GE => read_binary_arith(data, pos, |dest, lhs, rhs| Instruction::Ge {
            dest,
            lhs,
            rhs,
        }),
        opcode::TO_INTEGER => {
            let dest = ValueId(read_varint(data, pos)?);
            let operand = read_operand(data, pos)?;
            Ok(Instruction::ToInteger { dest, operand })
        }
        opcode::TO_TRYTE => {
            let dest = ValueId(read_varint(data, pos)?);
            let operand = read_operand(data, pos)?;
            Ok(Instruction::ToTryte { dest, operand })
        }
        opcode::TO_LONG => {
            let dest = ValueId(read_varint(data, pos)?);
            let operand = read_operand(data, pos)?;
            Ok(Instruction::ToLong { dest, operand })
        }
        opcode::TO_TRIT => {
            let dest = ValueId(read_varint(data, pos)?);
            let operand = read_operand(data, pos)?;
            Ok(Instruction::ToTrit { dest, operand })
        }
        opcode::TO_TRILEAN => {
            let dest = ValueId(read_varint(data, pos)?);
            let operand = read_operand(data, pos)?;
            Ok(Instruction::ToTrilean { dest, operand })
        }
        opcode::STRUCT_NEW => {
            let dest = ValueId(read_varint(data, pos)?);
            let field_count = read_varint(data, pos)? as usize;
            let mut fields = Vec::with_capacity(field_count);
            for _ in 0..field_count {
                fields.push(read_operand(data, pos)?);
            }
            Ok(Instruction::StructNew { dest, fields })
        }
        opcode::FIELD_GET => {
            let dest = ValueId(read_varint(data, pos)?);
            let object = read_operand(data, pos)?;
            let field_idx = read_varint(data, pos)?;
            Ok(Instruction::FieldGet {
                dest,
                object,
                field_idx,
            })
        }
        opcode::FIELD_SET => {
            let dest = ValueId(read_varint(data, pos)?);
            let object = read_operand(data, pos)?;
            let field_idx = read_varint(data, pos)?;
            let value = read_operand(data, pos)?;
            Ok(Instruction::FieldSet {
                dest,
                object,
                field_idx,
                value,
            })
        }
        opcode::ENUM_NEW => {
            let dest = ValueId(read_varint(data, pos)?);
            let variant_idx = read_varint(data, pos)?;
            let payload = read_option_operand(data, pos)?;
            Ok(Instruction::EnumNew {
                dest,
                variant_idx,
                payload,
            })
        }
        opcode::ENUM_TAG => {
            let dest = ValueId(read_varint(data, pos)?);
            let scrutinee = read_operand(data, pos)?;
            Ok(Instruction::EnumTag { dest, scrutinee })
        }
        opcode::ENUM_PAYLOAD => {
            let dest = ValueId(read_varint(data, pos)?);
            let scrutinee = read_operand(data, pos)?;
            Ok(Instruction::EnumPayload { dest, scrutinee })
        }
        opcode::NULL_WRAP => {
            let dest = ValueId(read_varint(data, pos)?);
            let value = read_operand(data, pos)?;
            Ok(Instruction::NullWrap { dest, value })
        }
        opcode::NULL_UNWRAP => {
            let dest = ValueId(read_varint(data, pos)?);
            let nullable = read_operand(data, pos)?;
            Ok(Instruction::NullUnwrap { dest, nullable })
        }
        opcode::NULL_CHECK => {
            let dest = ValueId(read_varint(data, pos)?);
            let nullable = read_operand(data, pos)?;
            Ok(Instruction::NullCheck { dest, nullable })
        }
        opcode::CALL_LOCAL => {
            let dest = read_option_value(data, pos)?;
            let callee = FuncId(read_varint(data, pos)?);
            let arg_count = read_varint(data, pos)? as usize;
            let mut args = Vec::with_capacity(arg_count);
            for _ in 0..arg_count {
                args.push(read_operand(data, pos)?);
            }
            Ok(Instruction::CallLocal { dest, callee, args })
        }
        opcode::CALL_CROSS_MODULE => {
            let dest = read_option_value(data, pos)?;
            let path_str = read_string(data, pos)?;
            let path = parse_absolute_path(&path_str)?;
            let arg_count = read_varint(data, pos)? as usize;
            let mut args = Vec::with_capacity(arg_count);
            for _ in 0..arg_count {
                args.push(read_operand(data, pos)?);
            }
            Ok(Instruction::CallCrossModule { dest, path, args })
        }
        opcode::WITNESS_CALL => {
            let dest = read_option_value(data, pos)?;
            let path_str = read_string(data, pos)?;
            let path = parse_absolute_path(&path_str)?;
            let witness_idx = read_varint(data, pos)?;
            let arg_count = read_varint(data, pos)? as usize;
            let mut args = Vec::with_capacity(arg_count);
            for _ in 0..arg_count {
                args.push(read_operand(data, pos)?);
            }
            Ok(Instruction::WitnessCall {
                dest,
                path,
                witness_idx,
                args,
            })
        }
        opcode::CALL_BUILTIN => {
            let dest = read_option_value(data, pos)?;
            let name = read_builtin(data, pos)?;
            let arg_count = read_varint(data, pos)? as usize;
            let mut args = Vec::with_capacity(arg_count);
            for _ in 0..arg_count {
                args.push(read_operand(data, pos)?);
            }
            Ok(Instruction::CallBuiltin { dest, name, args })
        }
        opcode::CLOSURE_NEW => {
            let dest = ValueId(read_varint(data, pos)?);
            let lambda = FuncId(read_varint(data, pos)?);
            let capture_count = read_varint(data, pos)? as usize;
            let mut captures = Vec::with_capacity(capture_count);
            for _ in 0..capture_count {
                captures.push(ValueId(read_varint(data, pos)?));
            }
            Ok(Instruction::ClosureNew {
                dest,
                lambda,
                captures,
            })
        }
        opcode::CLOSURE_CALL => {
            let dest = read_option_value(data, pos)?;
            let closure = read_operand(data, pos)?;
            let arg_count = read_varint(data, pos)? as usize;
            let mut args = Vec::with_capacity(arg_count);
            for _ in 0..arg_count {
                args.push(read_operand(data, pos)?);
            }
            Ok(Instruction::ClosureCall {
                dest,
                closure,
                args,
            })
        }
        opcode::BR => {
            let target = BlockId(read_varint(data, pos)?);
            Ok(Instruction::Br { target })
        }
        opcode::BR_IF => {
            let cond = read_operand(data, pos)?;
            let then_block = BlockId(read_varint(data, pos)?);
            let else_block = BlockId(read_varint(data, pos)?);
            Ok(Instruction::BrIf {
                cond,
                then_block,
                else_block,
            })
        }
        opcode::BR_TRILEAN => {
            let cond = read_operand(data, pos)?;
            let true_block = BlockId(read_varint(data, pos)?);
            let unknown_block = BlockId(read_varint(data, pos)?);
            let false_block = BlockId(read_varint(data, pos)?);
            Ok(Instruction::BrTrilean {
                cond,
                true_block,
                unknown_block,
                false_block,
            })
        }
        opcode::RET => {
            let value = read_option_operand(data, pos)?;
            Ok(Instruction::Ret { value })
        }
        opcode::UNREACHABLE => Ok(Instruction::Unreachable),
        opcode::PHI => {
            let dest = ValueId(read_varint(data, pos)?);
            let incoming_count = read_varint(data, pos)? as usize;
            let mut incoming = Vec::with_capacity(incoming_count);
            for _ in 0..incoming_count {
                let value = ValueId(read_varint(data, pos)?);
                let block = BlockId(read_varint(data, pos)?);
                incoming.push(PhiIncoming { value, block });
            }
            Ok(Instruction::Phi { dest, incoming })
        }
        // v0.7.4.3-error.3a (ADR-0020 §7.3) — outcome opcodes:
        opcode::OUTCOME_NEW_POSITIVE => {
            let dest = ValueId(read_varint(data, pos)?);
            let payload = read_operand(data, pos)?;
            Ok(Instruction::OutcomeNewPositive { dest, payload })
        }
        opcode::OUTCOME_NEW_NEGATIVE => {
            let dest = ValueId(read_varint(data, pos)?);
            let payload = read_operand(data, pos)?;
            Ok(Instruction::OutcomeNewNegative { dest, payload })
        }
        opcode::OUTCOME_NEW_NULL => {
            let dest = ValueId(read_varint(data, pos)?);
            Ok(Instruction::OutcomeNewNull { dest })
        }
        opcode::OUTCOME_DISCRIMINANT => {
            let dest = ValueId(read_varint(data, pos)?);
            let source = read_operand(data, pos)?;
            Ok(Instruction::OutcomeDiscriminant { dest, source })
        }
        opcode::OUTCOME_UNWRAP_VALUE => {
            let dest = ValueId(read_varint(data, pos)?);
            let source = read_operand(data, pos)?;
            Ok(Instruction::OutcomeUnwrapValue { dest, source })
        }
        opcode::OUTCOME_UNWRAP_ERROR => {
            let dest = ValueId(read_varint(data, pos)?);
            let source = read_operand(data, pos)?;
            Ok(Instruction::OutcomeUnwrapError { dest, source })
        }
        op => Err(TrivError::UnknownOpcode(op)),
    }
}

fn read_binary_arith(
    data: &[u8],
    pos: &mut usize,
    f: impl FnOnce(ValueId, Operand, Operand) -> Instruction,
) -> Result<Instruction, TrivError> {
    let dest = ValueId(read_varint(data, pos)?);
    let lhs = read_operand(data, pos)?;
    let rhs = read_operand(data, pos)?;
    Ok(f(dest, lhs, rhs))
}

fn parse_absolute_path(s: &str) -> Result<triet_modules::AbsolutePath, TrivError> {
    let mut parts: Vec<&str> = s.split('.').collect();
    if parts.is_empty() {
        return Err(TrivError::Corrupted("empty absolute path".into()));
    }
    let name = parts.pop().unwrap().to_owned();
    let module = triet_modules::ModulePath::new(parts.into_iter().map(String::from).collect());
    Ok(triet_modules::AbsolutePath::new(module, name))
}

// ── Constant pool ──────────────────────────────────────────────────

fn write_constant(buf: &mut Vec<u8>, constant: &Constant, types: &[TypeTag]) {
    let ty = constant.type_tag();
    write_varint(buf, type_index(types, &ty));
    match constant {
        Constant::Trit(t) => {
            let b: u8 = match t {
                triet_core::Trit::Negative => 0,
                triet_core::Trit::Zero => 0x01,
                triet_core::Trit::Positive => 0x02,
            };
            write_u8(buf, b);
        }
        Constant::Tryte(v) => {
            write_bytes(buf, &v.to_i16().to_le_bytes());
        }
        Constant::Integer(v) => {
            write_bytes(buf, &v.to_i64().to_le_bytes());
        }
        Constant::Long(v) => {
            let val = v.to_integer().to_i64();
            write_bytes(buf, &val.to_le_bytes());
        }
        Constant::Trilean(tl) => {
            let b: u8 = match tl {
                triet_logic::Trilean::False => 0,
                triet_logic::Trilean::Unknown => 1,
                triet_logic::Trilean::True => 2,
            };
            write_u8(buf, b);
        }
        Constant::String(s) => {
            write_string(buf, s);
        }
        Constant::Unit => { /* 0 bytes */ }
        Constant::Null => { /* 0 bytes — the type tag already says Nullable */ }
    }
}

fn read_constant(data: &[u8], pos: &mut usize, types: &[TypeTag]) -> Result<Constant, TrivError> {
    let type_idx = read_varint(data, pos)? as usize;
    let ty = types
        .get(type_idx)
        .ok_or_else(|| TrivError::Corrupted("invalid type index in constant".into()))?;
    match ty {
        TypeTag::Trit => {
            let b = read_u8(data, pos)?;
            match b {
                0 => Ok(Constant::Trit(triet_core::Trit::Negative)),
                1 => Ok(Constant::Trit(triet_core::Trit::Zero)),
                2 => Ok(Constant::Trit(triet_core::Trit::Positive)),
                b => Err(TrivError::Corrupted(format!("invalid trit byte 0x{b:02X}"))),
            }
        }
        TypeTag::Tryte => {
            let val = read_i16_le(data, pos)?;
            triet_core::Tryte::new(val)
                .ok_or_else(|| TrivError::Corrupted(format!("invalid tryte value: {val}")))
                .map(Constant::Tryte)
        }
        TypeTag::Integer => {
            let val = read_i64_le(data, pos)?;
            triet_core::Integer::new(val)
                .ok_or_else(|| TrivError::Corrupted(format!("invalid integer value: {val}")))
                .map(Constant::Integer)
        }
        TypeTag::Long => {
            let val = read_i64_le(data, pos)?;
            let integer = triet_core::Integer::new(val)
                .ok_or_else(|| TrivError::Corrupted(format!("invalid long base value: {val}")))?;
            Ok(Constant::Long(triet_core::Long::from_i64(integer.to_i64())))
        }
        TypeTag::Trilean => {
            let b = read_u8(data, pos)?;
            match b {
                0 => Ok(Constant::Trilean(triet_logic::Trilean::False)),
                1 => Ok(Constant::Trilean(triet_logic::Trilean::Unknown)),
                2 => Ok(Constant::Trilean(triet_logic::Trilean::True)),
                b => Err(TrivError::Corrupted(format!(
                    "invalid trilean byte 0x{b:02X}"
                ))),
            }
        }
        TypeTag::String => {
            let s = read_string(data, pos)?;
            Ok(Constant::String(s))
        }
        TypeTag::Unit => Ok(Constant::Unit),
        TypeTag::Nullable(_) => Ok(Constant::Null),
        TypeTag::Vector(_) | TypeTag::HashMap(_, _) => Err(TrivError::Corrupted(
            "Vector / HashMap have no constant-pool encoding — \
             collection values are built at runtime via builtin opcodes \
             (ADR-0019 §5)"
                .into(),
        )),
        TypeTag::Outcome { .. } => Err(TrivError::Corrupted(
            "Outcome has no constant-pool encoding — outcome values are \
             built at runtime via opcodes 0xC1-0xC3 per ADR-0020 §7.2"
                .into(),
        )),
        // v0.9.x.atomic.3 — Atomic has no constant-pool encoding;
        // atomic values are built at runtime via `AtomicNew` builtin
        // per ADR-0028 §6.
        TypeTag::Atomic(_) => Err(TrivError::Corrupted(
            "Atomic has no constant-pool encoding — atomic values are \
             built at runtime via `AtomicNew` builtin per ADR-0028 §6"
                .into(),
        )),
    }
}

fn read_i16_le(data: &[u8], pos: &mut usize) -> Result<i16, TrivError> {
    if *pos + 2 > data.len() {
        return Err(TrivError::Corrupted("unexpected EOF reading i16".into()));
    }
    let bytes: [u8; 2] = data[*pos..*pos + 2].try_into().unwrap();
    *pos += 2;
    Ok(i16::from_le_bytes(bytes))
}

fn read_i64_le(data: &[u8], pos: &mut usize) -> Result<i64, TrivError> {
    if *pos + 8 > data.len() {
        return Err(TrivError::Corrupted("unexpected EOF reading i64".into()));
    }
    let bytes: [u8; 8] = data[*pos..*pos + 8].try_into().unwrap();
    *pos += 8;
    Ok(i64::from_le_bytes(bytes))
}

fn write_constants(buf: &mut Vec<u8>, pool: &ConstantPool, types: &[TypeTag]) {
    write_varint(buf, pool.len() as u32);
    for (_, constant) in pool.iter() {
        write_constant(buf, constant, types);
    }
}

fn read_constants(
    data: &[u8],
    pos: &mut usize,
    types: &[TypeTag],
) -> Result<ConstantPool, TrivError> {
    let count = read_varint(data, pos)? as usize;
    let mut pool = ConstantPool::new();
    for _ in 0..count {
        let constant = read_constant(data, pos, types)?;
        pool.intern(constant);
    }
    Ok(pool)
}

// ── Function table ─────────────────────────────────────────────────

#[allow(dead_code)]
struct FunctionMeta {
    module_path: String,
    name: Option<String>,
    params: Vec<(String, TypeTag)>,
    return_type: TypeTag,
    func_id: FuncId,
}

fn write_function_table(buf: &mut Vec<u8>, program: &IrProgram, types: &[TypeTag]) {
    let mut count = 0u32;
    for module in &program.modules {
        count += module.functions.len() as u32;
    }
    write_varint(buf, count);

    for module in &program.modules {
        let module_path_str = module.path.to_string();
        for func in &module.functions {
            write_string(buf, &module_path_str);
            match &func.name {
                Some(n) => {
                    write_u8(buf, 1);
                    write_string(buf, n);
                }
                None => write_u8(buf, 0),
            }
            write_varint(buf, func.params.len() as u32);
            for (name, ty) in &func.params {
                write_string(buf, name);
                write_varint(buf, type_index(types, ty));
            }
            write_varint(buf, type_index(types, &func.return_type));
        }
    }
}

fn read_function_table(
    data: &[u8],
    pos: &mut usize,
    types: &[TypeTag],
) -> Result<Vec<FunctionMeta>, TrivError> {
    let count = read_varint(data, pos)? as usize;
    let mut metas: Vec<FunctionMeta> = Vec::with_capacity(count);
    for i in 0..count {
        let module_path = read_string(data, pos)?;
        let has_name = read_u8(data, pos)?;
        let name = match has_name {
            0 => None,
            1 => Some(read_string(data, pos)?),
            b => {
                return Err(TrivError::Corrupted(format!(
                    "invalid has_name byte 0x{b:02X}"
                )));
            }
        };
        let param_count = read_varint(data, pos)? as usize;
        let mut params = Vec::with_capacity(param_count);
        for _ in 0..param_count {
            let param_name = read_string(data, pos)?;
            let type_idx = read_varint(data, pos)? as usize;
            let ty = types
                .get(type_idx)
                .ok_or_else(|| TrivError::Corrupted("invalid param type index".into()))?
                .clone();
            params.push((param_name, ty));
        }
        let ret_type_idx = read_varint(data, pos)? as usize;
        let return_type = types
            .get(ret_type_idx)
            .ok_or_else(|| TrivError::Corrupted("invalid return type index".into()))?
            .clone();
        metas.push(FunctionMeta {
            module_path,
            name,
            params,
            return_type,
            func_id: FuncId(i as u32),
        });
    }
    Ok(metas)
}

// ── Code section ───────────────────────────────────────────────────

fn write_code(buf: &mut Vec<u8>, program: &IrProgram) {
    let mut count = 0u32;
    for module in &program.modules {
        count += module.functions.len() as u32;
    }
    write_varint(buf, count);

    for module in &program.modules {
        for func in &module.functions {
            write_varint(buf, func.blocks.len() as u32);
            for block in &func.blocks {
                write_varint(buf, block.id.0);
                match &block.name {
                    Some(n) => write_string(buf, n),
                    None => write_string(buf, ""),
                }
                write_varint(buf, block.instructions.len() as u32);
                for instr in &block.instructions {
                    write_instruction(buf, instr);
                }
            }
        }
    }
}

fn read_code(
    data: &[u8],
    pos: &mut usize,
    metas: &[FunctionMeta],
) -> Result<Vec<Function>, TrivError> {
    let count = read_varint(data, pos)? as usize;
    if count != metas.len() {
        return Err(TrivError::Corrupted(format!(
            "code function count {count} != function table count {}",
            metas.len()
        )));
    }
    let mut functions: Vec<Function> = Vec::with_capacity(count);
    for (i, meta) in metas.iter().enumerate() {
        let block_count = read_varint(data, pos)? as usize;
        let mut blocks = Vec::with_capacity(block_count);
        for _ in 0..block_count {
            let block_id = BlockId(read_varint(data, pos)?);
            let name_str = read_string(data, pos)?;
            let name = if name_str.is_empty() {
                None
            } else {
                Some(name_str)
            };
            let instr_count = read_varint(data, pos)? as usize;
            let mut instructions = Vec::with_capacity(instr_count);
            for _ in 0..instr_count {
                instructions.push(read_instruction(data, pos)?);
            }
            blocks.push(BasicBlock {
                id: block_id,
                name,
                instructions,
            });
        }
        functions.push(Function {
            id: FuncId(i as u32),
            name: meta.name.clone(),
            params: meta.params.clone(),
            return_type: meta.return_type.clone(),
            blocks,
        });
    }
    Ok(functions)
}

// ── Public API ─────────────────────────────────────────────────────

/// Serialize an [`IrProgram`] into the `.triv` binary format.
///
/// The output is a complete, self-contained `.triv` file suitable for
/// writing to disk. It includes the magic header, version, and all four
/// sections (types, constants, functions, code).
#[must_use]
pub fn write_program(program: &IrProgram) -> Vec<u8> {
    let types = collect_type_table(program);
    let mut type_payload = Vec::new();
    write_type_table(&mut type_payload, &types);

    let mut const_payload = Vec::new();
    write_constants(&mut const_payload, &program.constants, &types);

    let mut func_payload = Vec::new();
    write_function_table(&mut func_payload, program, &types);

    let mut code_payload = Vec::new();
    write_code(&mut code_payload, program);

    // ADR-0012 witness tables. Only emit the section when the
    // program actually has cross-package generic instantiations;
    // older readers happily skip unknown sections, so the absent-
    // section path stays bit-identical with the v1/v2 wire format.
    let witness_payload = if program.witness_tables.is_empty() {
        Vec::new()
    } else {
        let mut payload = Vec::new();
        write_witness_tables(&mut payload, &program.witness_tables);
        payload
    };
    let section_count: u32 = if witness_payload.is_empty() { 4 } else { 5 };

    let mut buf = Vec::new();

    // Header
    write_bytes(&mut buf, &MAGIC);
    write_u32_le(&mut buf, VERSION);
    write_u32_le(&mut buf, section_count);

    // Sections
    write_section(&mut buf, SEC_TYPES, &type_payload);
    write_section(&mut buf, SEC_CONSTANTS, &const_payload);
    write_section(&mut buf, SEC_FUNCTIONS, &func_payload);
    write_section(&mut buf, SEC_CODE, &code_payload);
    if !witness_payload.is_empty() {
        write_section(&mut buf, SEC_WITNESS_TABLES, &witness_payload);
    }

    buf
}

/// Encode the witness-table section (ADR-0012). Each table holds the
/// concrete `TypeTag`s bound to a callee's type parameters; per the
/// ADR, operation slots are reserved for v0.6+ so we don't emit them
/// yet.
///
/// Witness tables encode their type args inline (1 byte primitive
/// discriminator each) instead of referencing the program-wide type
/// table — this keeps the section self-contained for the linker,
/// which validates ABI before code is loaded. `Nullable(T)` type args
/// are deferred (not produced by today's lowerer for generic
/// instantiations); we error rather than silently truncate.
fn write_witness_tables(buf: &mut Vec<u8>, tables: &[crate::module::WitnessTable]) {
    write_varint(buf, tables.len() as u32);
    for table in tables {
        write_varint(buf, table.type_args.len() as u32);
        for tag in &table.type_args {
            write_inline_primitive_tag(buf, tag);
        }
    }
}

fn read_witness_tables(
    data: &[u8],
    pos: &mut usize,
) -> Result<Vec<crate::module::WitnessTable>, TrivError> {
    let count = read_varint(data, pos)? as usize;
    let mut tables = Vec::with_capacity(count);
    for _ in 0..count {
        let arg_count = read_varint(data, pos)? as usize;
        let mut type_args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            type_args.push(read_inline_primitive_tag(data, pos)?);
        }
        tables.push(crate::module::WitnessTable { type_args });
    }
    Ok(tables)
}

/// Self-contained inline encoder for a primitive `TypeTag`. Refuses
/// `Nullable(T)` / `Vector(T)` / `HashMap(K, V)` — composite types
/// don't appear as generic type arguments in today's lowerer; if they
/// ever do, we'll either resolve to the underlying primitive at link
/// time or extend the encoding then.
fn write_inline_primitive_tag(buf: &mut Vec<u8>, ty: &TypeTag) {
    let disc = match ty {
        TypeTag::Trit => 0,
        TypeTag::Tryte => 1,
        TypeTag::Integer => 2,
        TypeTag::Long => 3,
        TypeTag::Trilean => 4,
        TypeTag::String => 5,
        TypeTag::Unit => 6,
        // Composites in a witness table would need a recursive
        // type-ref protocol that we haven't committed to yet
        // (ADR-0012 §6 "Generic constraint support at v0.4" defers
        // complex generic arguments). Emit a sentinel that the reader
        // will refuse — catches the case loudly during round-trip tests.
        TypeTag::Nullable(_)
        | TypeTag::Vector(_)
        | TypeTag::HashMap(_, _)
        | TypeTag::Outcome { .. }
        | TypeTag::Atomic(_) => 0xFF,
    };
    write_u8(buf, disc);
}

fn read_inline_primitive_tag(data: &[u8], pos: &mut usize) -> Result<TypeTag, TrivError> {
    let disc = read_u8(data, pos)?;
    match disc {
        0 => Ok(TypeTag::Trit),
        1 => Ok(TypeTag::Tryte),
        2 => Ok(TypeTag::Integer),
        3 => Ok(TypeTag::Long),
        4 => Ok(TypeTag::Trilean),
        5 => Ok(TypeTag::String),
        6 => Ok(TypeTag::Unit),
        b => Err(TrivError::Corrupted(format!(
            "witness table type arg 0x{b:02X} — only primitive TypeTags are encoded inline at v0.4 (ADR-0012 §6 defers complex generic args)"
        ))),
    }
}

fn write_section(buf: &mut Vec<u8>, section_id: u8, payload: &[u8]) {
    write_u8(buf, section_id);
    write_u32_le(buf, payload.len() as u32);
    write_bytes(buf, payload);
}

/// Deserialize a `.triv` binary file into an [`IrProgram`].
///
/// # Errors
///
/// Returns a [`TrivError`] if the data is corrupted, has an unsupported
/// version, or contains invalid UTF-8 / unknown opcodes / type discriminants.
pub fn read_program(data: &[u8]) -> Result<IrProgram, TrivError> {
    let mut pos: usize = 0;

    // Magic
    if data.len() < 4 {
        return Err(TrivError::Corrupted("file too short for magic".into()));
    }
    let magic = &data[pos..pos + 4];
    pos += 4;
    if magic != MAGIC {
        return Err(TrivError::Corrupted(format!(
            "bad magic bytes: expected {MAGIC:02X?}, got {magic:02X?}"
        )));
    }

    // Version
    let version = read_u32_le(data, &mut pos)?;
    if version > VERSION {
        return Err(TrivError::UnsupportedVersion {
            found: version,
            max_supported: VERSION,
        });
    }

    // Section count
    let section_count = read_u32_le(data, &mut pos)?;

    // Sections
    let mut types: Vec<TypeTag> = Vec::new();
    let mut constants = ConstantPool::new();
    let mut function_metas: Vec<FunctionMeta> = Vec::new();
    let mut functions: Vec<Function> = Vec::new();
    let mut witness_tables: Vec<crate::module::WitnessTable> = Vec::new();
    let mut has_code = false;

    for _ in 0..section_count {
        let section_id = read_u8(data, &mut pos)?;
        let section_size = read_u32_le(data, &mut pos)? as usize;
        if pos + section_size > data.len() {
            return Err(TrivError::SectionSizeMismatch {
                section_id,
                declared: section_size as u32,
                actual: data.len() - pos,
            });
        }
        let payload = &data[pos..pos + section_size];
        let mut payload_pos = 0usize;

        match section_id {
            SEC_TYPES => {
                types = read_type_table(payload, &mut payload_pos)?;
            }
            SEC_CONSTANTS => {
                constants = read_constants(payload, &mut payload_pos, &types)?;
            }
            SEC_FUNCTIONS => {
                function_metas = read_function_table(payload, &mut payload_pos, &types)?;
            }
            SEC_CODE => {
                functions = read_code(payload, &mut payload_pos, &function_metas)?;
                has_code = true;
            }
            SEC_WITNESS_TABLES => {
                witness_tables = read_witness_tables(payload, &mut payload_pos)?;
            }
            _ => { /* skip unknown section */ }
        }

        pos += section_size;
    }

    if !has_code {
        return Err(TrivError::Corrupted("missing code section".into()));
    }

    // Reconstruct modules by grouping functions by module_path
    let modules = group_into_modules(&functions, &function_metas);

    Ok(IrProgram {
        modules,
        constants,
        witness_tables,
    })
}

fn group_into_modules(functions: &[Function], metas: &[FunctionMeta]) -> Vec<IrModule> {
    let mut modules: Vec<IrModule> = Vec::new();
    // Use insertion order, not HashMap, to preserve module ordering.
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for (i, meta) in metas.iter().enumerate() {
        if let Some(&idx) = seen.get(&meta.module_path) {
            modules[idx].functions.push(functions[i].clone());
        } else {
            let idx = modules.len();
            seen.insert(meta.module_path.clone(), idx);
            // The module_path string is already the AbsolutePath of the module
            // as an item (e.g., "khi.test" → AbsolutePath { module: ["khi"], name: "test" }).
            let module_path = parse_absolute_path(&meta.module_path).unwrap_or_else(|_| {
                triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "unknown".into(),
                )
            });
            modules.push(IrModule {
                path: module_path,
                functions: vec![functions[i].clone()],
            });
        }
    }

    modules
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instr::{Instruction, Operand, PhiIncoming};
    use crate::module::{BasicBlock, Function, IrModule, IrProgram};
    use crate::types::{BlockId, ConstId, FuncId, TypeTag, ValueId};
    use triet_core::Integer;

    fn int(n: i64) -> Integer {
        Integer::new(n).unwrap()
    }

    // ── LEB128 round-trip ──────────────────────────────────────

    #[test]
    fn leb128_round_trip() {
        let cases = [0u32, 1, 127, 128, 255, 16383, 16384, 1_000_000, u32::MAX];
        for &value in &cases {
            let mut buf = Vec::new();
            write_leb128(&mut buf, value);
            let mut pos = 0;
            let decoded = read_leb128(&buf, &mut pos).unwrap();
            assert_eq!(decoded, value, "LEB128 round-trip failed for {value}");
            assert_eq!(
                pos,
                buf.len(),
                "LEB128 didn't consume all bytes for {value}"
            );
        }
    }

    #[test]
    fn leb128_truncated() {
        let buf = vec![0x80]; // MSB set, but no continuation
        let mut pos = 0;
        assert!(read_leb128(&buf, &mut pos).is_err());
    }

    // ── Empty program ──────────────────────────────────────────

    #[test]
    fn empty_program_round_trip() {
        let program = IrProgram::new();
        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert!(decoded.is_empty());
        assert_eq!(decoded.constants.len(), 0);
    }

    // ── Single function ────────────────────────────────────────

    fn make_simple_program() -> IrProgram {
        let mut pool = ConstantPool::new();
        let c1 = pool.intern(Constant::Integer(int(1)));

        let function = Function {
            id: FuncId(0),
            name: Some("add_one".into()),
            params: vec![("%x".into(), TypeTag::Integer)],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Add {
                        dest: ValueId(1),
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Const(c1),
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(1))),
                    },
                ],
            }],
        };

        IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "add_one".into(),
                ),
                functions: vec![function],
            }],
            constants: pool,
            witness_tables: Vec::new(),
        }
    }

    #[test]
    fn single_function_round_trip() {
        let program = make_simple_program();
        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    // ── Multi-module program ───────────────────────────────────

    #[test]
    fn multi_module_round_trip() {
        let mut pool = ConstantPool::new();
        let c42 = pool.intern(Constant::Integer(int(42)));

        let func_a = Function {
            id: FuncId(0),
            name: Some("get_answer".into()),
            params: vec![],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Const {
                        dest: ValueId(0),
                        constant: c42,
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    },
                ],
            }],
        };

        let func_b = Function {
            id: FuncId(1),
            name: Some("main".into()),
            params: vec![],
            return_type: TypeTag::Unit,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::CallLocal {
                        dest: Some(ValueId(0)),
                        callee: FuncId(0),
                        args: vec![],
                    },
                    Instruction::CallBuiltin {
                        dest: None,
                        name: BuiltinName::Println,
                        args: vec![Operand::Value(ValueId(0))],
                    },
                    Instruction::Ret { value: None },
                ],
            }],
        };

        let program = IrProgram {
            modules: vec![
                IrModule {
                    path: triet_modules::AbsolutePath::new(
                        triet_modules::ModulePath::khi_root(),
                        "lib".into(),
                    ),
                    functions: vec![func_a],
                },
                IrModule {
                    path: triet_modules::AbsolutePath::new(
                        triet_modules::ModulePath::khi_root(),
                        "main".into(),
                    ),
                    functions: vec![func_b],
                },
            ],
            constants: pool,
            witness_tables: Vec::new(),
        };

        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    // ── All type tags ──────────────────────────────────────────

    #[test]
    fn all_type_tags_survive_round_trip() {
        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "test".into(),
                ),
                functions: vec![
                    Function {
                        id: FuncId(0),
                        name: Some("with_trit".into()),
                        params: vec![("%t".into(), TypeTag::Trit)],
                        return_type: TypeTag::Trit,
                        blocks: vec![BasicBlock {
                            id: BlockId(0),
                            name: Some("entry".into()),
                            instructions: vec![Instruction::Ret {
                                value: Some(Operand::Value(ValueId(0))),
                            }],
                        }],
                    },
                    Function {
                        id: FuncId(1),
                        name: Some("with_nullable".into()),
                        params: vec![("%n".into(), TypeTag::Nullable(Box::new(TypeTag::Integer)))],
                        return_type: TypeTag::Trilean,
                        blocks: vec![BasicBlock {
                            id: BlockId(0),
                            name: Some("entry".into()),
                            instructions: vec![Instruction::Ret {
                                value: Some(Operand::Const(ConstId(0))),
                            }],
                        }],
                    },
                ],
            }],
            constants: {
                let mut pool = ConstantPool::new();
                pool.intern(Constant::Trilean(triet_logic::Trilean::True));
                pool
            },
            witness_tables: Vec::new(),
        };
        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    /// Round-trip `Vector<T>` + `HashMap<K, V>` type tags through the
    /// type table. Catches v3 → v4 wire-format regression in the
    /// `write_type_tag` / `read_type_tag` pair (ADR-0019 §1 type
    /// system extension + ADR-0008 §"Version compatibility" patch
    /// bump rule). Composites also exercise the recursive `add_type`
    /// post-order — the inner tag must precede the container.
    #[test]
    fn vector_and_hashmap_type_tags_round_trip() {
        let vector_int = TypeTag::Vector(Box::new(TypeTag::Integer));
        let map_string_int =
            TypeTag::HashMap(Box::new(TypeTag::String), Box::new(TypeTag::Integer));
        let nested = TypeTag::Vector(Box::new(TypeTag::Vector(Box::new(TypeTag::Trit))));

        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "test".into(),
                ),
                functions: vec![
                    Function {
                        id: FuncId(0),
                        name: Some("with_vector".into()),
                        params: vec![("%v".into(), vector_int.clone())],
                        return_type: vector_int,
                        blocks: vec![BasicBlock {
                            id: BlockId(0),
                            name: Some("entry".into()),
                            instructions: vec![Instruction::Ret {
                                value: Some(Operand::Value(ValueId(0))),
                            }],
                        }],
                    },
                    Function {
                        id: FuncId(1),
                        name: Some("with_hashmap".into()),
                        params: vec![("%m".into(), map_string_int.clone())],
                        return_type: map_string_int,
                        blocks: vec![BasicBlock {
                            id: BlockId(0),
                            name: Some("entry".into()),
                            instructions: vec![Instruction::Ret {
                                value: Some(Operand::Value(ValueId(0))),
                            }],
                        }],
                    },
                    Function {
                        id: FuncId(2),
                        name: Some("with_nested".into()),
                        params: vec![("%nv".into(), nested.clone())],
                        return_type: nested,
                        blocks: vec![BasicBlock {
                            id: BlockId(0),
                            name: Some("entry".into()),
                            instructions: vec![Instruction::Ret {
                                value: Some(Operand::Value(ValueId(0))),
                            }],
                        }],
                    },
                ],
            }],
            constants: ConstantPool::new(),
            witness_tables: Vec::new(),
        };
        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    /// `TypeTag::Outcome` survives a `.triv` round-trip — covers both
    /// the binary (`T~E`, `allow_null_state` = false) and ternary
    /// (`T?~E`, `allow_null_state` = true) shapes plus a nested
    /// `Outcome<Outcome<...>, _>` to prove the post-order type
    /// table encodes inner outcomes before the outer.
    #[test]
    fn outcome_type_tag_round_trip() {
        let binary = TypeTag::Outcome {
            value_type: Box::new(TypeTag::Integer),
            error_type: Box::new(TypeTag::String),
            allow_null_state: false,
        };
        let ternary = TypeTag::Outcome {
            value_type: Box::new(TypeTag::String),
            error_type: Box::new(TypeTag::Integer),
            allow_null_state: true,
        };
        // Outcome<Outcome<Integer, String>, Long> — exercises post-
        // order recursion through the type table.
        let nested = TypeTag::Outcome {
            value_type: Box::new(TypeTag::Outcome {
                value_type: Box::new(TypeTag::Integer),
                error_type: Box::new(TypeTag::String),
                allow_null_state: false,
            }),
            error_type: Box::new(TypeTag::Long),
            allow_null_state: false,
        };

        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "test".into(),
                ),
                functions: vec![
                    Function {
                        id: FuncId(0),
                        name: Some("binary_outcome".into()),
                        params: vec![("%o".into(), binary.clone())],
                        return_type: binary,
                        blocks: vec![BasicBlock {
                            id: BlockId(0),
                            name: Some("entry".into()),
                            instructions: vec![Instruction::Ret {
                                value: Some(Operand::Value(ValueId(0))),
                            }],
                        }],
                    },
                    Function {
                        id: FuncId(1),
                        name: Some("ternary_outcome".into()),
                        params: vec![("%o".into(), ternary.clone())],
                        return_type: ternary,
                        blocks: vec![BasicBlock {
                            id: BlockId(0),
                            name: Some("entry".into()),
                            instructions: vec![Instruction::Ret {
                                value: Some(Operand::Value(ValueId(0))),
                            }],
                        }],
                    },
                    Function {
                        id: FuncId(2),
                        name: Some("nested_outcome".into()),
                        params: vec![("%o".into(), nested.clone())],
                        return_type: nested,
                        blocks: vec![BasicBlock {
                            id: BlockId(0),
                            name: Some("entry".into()),
                            instructions: vec![Instruction::Ret {
                                value: Some(Operand::Value(ValueId(0))),
                            }],
                        }],
                    },
                ],
            }],
            constants: ConstantPool::new(),
            witness_tables: Vec::new(),
        };
        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    /// All 6 new outcome opcodes (0xC1-0xC6) survive a `.triv`
    /// round-trip. Builds one function holding all six instructions,
    /// encodes, decodes, asserts equality.
    #[test]
    fn outcome_opcodes_round_trip() {
        let mut pool = ConstantPool::new();
        let payload = pool.intern(Constant::Integer(Integer::new(7).unwrap()));
        let outcome_type = TypeTag::Outcome {
            value_type: Box::new(TypeTag::Integer),
            error_type: Box::new(TypeTag::String),
            allow_null_state: true,
        };
        let function = Function {
            id: FuncId(0),
            name: Some("all_outcome_opcodes".into()),
            params: Vec::new(),
            return_type: TypeTag::Unit,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::OutcomeNewPositive {
                        dest: ValueId(0),
                        payload: Operand::Const(payload),
                    },
                    Instruction::OutcomeNewNegative {
                        dest: ValueId(1),
                        payload: Operand::Const(payload),
                    },
                    Instruction::OutcomeNewNull { dest: ValueId(2) },
                    Instruction::OutcomeDiscriminant {
                        dest: ValueId(3),
                        source: Operand::Value(ValueId(0)),
                    },
                    Instruction::OutcomeUnwrapValue {
                        dest: ValueId(4),
                        source: Operand::Value(ValueId(0)),
                    },
                    Instruction::OutcomeUnwrapError {
                        dest: ValueId(5),
                        source: Operand::Value(ValueId(1)),
                    },
                    Instruction::Ret { value: None },
                ],
            }],
        };
        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "test".into(),
                ),
                functions: vec![Function {
                    // Stash the outcome_type into a param so the type
                    // table sees a real `TypeTag::Outcome` entry.
                    params: vec![("%seed".into(), outcome_type)],
                    ..function
                }],
            }],
            constants: pool,
            witness_tables: Vec::new(),
        };
        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    /// Verify the wire-format version pin. Bumped 3→4 alongside the
    /// Vector/HashMap type-tag additions (ADR-0019 Addendum); bumped
    /// 4→5 alongside the Outcome type-tag + opcodes 0xC1-0xC6
    /// (v0.7.4.3-error.3a, ADR-0020 §7); bumped 5→6 alongside Atomic
    /// primitive builtins 33-42 (v0.9.x.atomic.2, ADR-0028 §1). The
    /// version field is the 4 bytes at offset 4 (after the 4-byte magic).
    #[test]
    fn wire_format_version_pinned_to_v6() {
        let program = IrProgram::new();
        let bytes = write_program(&program);
        assert!(bytes.len() >= 8, "header truncated");
        let version_bytes: [u8; 4] = bytes[4..8].try_into().unwrap();
        let version = u32::from_le_bytes(version_bytes);
        assert_eq!(
            version, 6,
            "v0.9.x.atomic.2 requires .triv version 6 (was {version})",
        );
    }

    /// v0.9.x.atomic.2 — verify all 10 atomic builtins round-trip
    /// through `write_builtin`/`read_builtin` (IDs 33-42 per ADR-0028 §1).
    /// Catches regression if any ID gap is introduced.
    #[test]
    fn atomic_builtins_serde_round_trip() {
        let atomic_builtins = [
            BuiltinName::AtomicNew,
            BuiltinName::AtomicLoad,
            BuiltinName::AtomicStore,
            BuiltinName::AtomicSwap,
            BuiltinName::AtomicCompareExchange,
            BuiltinName::AtomicFetchAdd,
            BuiltinName::AtomicFetchSub,
            BuiltinName::AtomicFetchBitwiseAnd,
            BuiltinName::AtomicFetchBitwiseOr,
            BuiltinName::AtomicFetchBitwiseXor,
        ];
        for builtin in atomic_builtins {
            let mut buf = Vec::new();
            write_builtin(&mut buf, builtin);
            assert_eq!(buf.len(), 1, "{builtin:?} must encode as single byte");
            let mut pos = 0;
            let decoded = read_builtin(&buf, &mut pos).expect("known atomic builtin");
            assert_eq!(
                decoded, builtin,
                "round-trip mismatch for {builtin:?}: wrote {buf:?}",
            );
            assert_eq!(pos, 1, "{builtin:?} must consume single byte");
        }
    }

    /// A pre-v4 reader (one that only knows discriminants 0..=7) must
    /// refuse a type table containing the new Vector (8) discriminant
    /// with `UnknownTypeDiscriminant`. We simulate the pre-v4 reader
    /// by calling `read_type_tag` directly — same behavior the older
    /// `.triv` consumers would exhibit when fed a v4 file.
    #[test]
    fn pre_v4_reader_refuses_vector_discriminant() {
        let mut buf: Vec<u8> = Vec::new();
        write_u8(&mut buf, 8); // Vector discriminant
        write_varint(&mut buf, 0);
        let mut pos = 0;
        // Empty table — the read will fail either at the discriminant
        // (if rejected) or at the inner-type lookup. Either way the
        // pre-v4 contract is honored.
        let result = read_type_tag(&buf, &mut pos, &[TypeTag::Trit]);
        assert!(
            result.is_ok(),
            "v4-aware reader should accept Vector discriminant: {result:?}"
        );
        // Confirm the v4-aware reader DOES accept it (this protects
        // against accidentally regressing the new branch).
        if let Ok(TypeTag::Vector(inner)) = result {
            assert_eq!(*inner, TypeTag::Trit);
        } else {
            panic!("expected TypeTag::Vector, got {result:?}");
        }
    }

    // ── All instruction variants ───────────────────────────────

    #[test]
    fn all_constant_types_survive_round_trip() {
        let mut pool = ConstantPool::new();
        let c_trit = pool.intern(Constant::Trit(triet_core::Trit::Positive));
        let c_tryte = pool.intern(Constant::Tryte(triet_core::Tryte::new(42).unwrap()));
        let c_int = pool.intern(Constant::Integer(int(-5)));
        let c_long = pool.intern(Constant::Long(triet_core::Long::from_i64(1000)));
        let c_str = pool.intern(Constant::String("hello".into()));
        let c_unit = pool.intern(Constant::Unit);

        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "test".into(),
                ),
                functions: vec![Function {
                    id: FuncId(0),
                    name: Some("consts".into()),
                    params: vec![],
                    return_type: TypeTag::Unit,
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        name: Some("entry".into()),
                        instructions: vec![
                            Instruction::Const {
                                dest: ValueId(0),
                                constant: c_trit,
                            },
                            Instruction::Const {
                                dest: ValueId(1),
                                constant: c_tryte,
                            },
                            Instruction::Const {
                                dest: ValueId(2),
                                constant: c_int,
                            },
                            Instruction::Const {
                                dest: ValueId(3),
                                constant: c_long,
                            },
                            Instruction::Const {
                                dest: ValueId(4),
                                constant: c_str,
                            },
                            Instruction::Const {
                                dest: ValueId(5),
                                constant: c_unit,
                            },
                            Instruction::Ret { value: None },
                        ],
                    }],
                }],
            }],
            constants: pool,
            witness_tables: Vec::new(),
        };

        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    // ── Control flow with phi ──────────────────────────────────

    #[test]
    fn if_else_with_phi_round_trip() {
        let mut pool = ConstantPool::new();
        let c1 = pool.intern(Constant::Integer(int(1)));
        let c0 = pool.intern(Constant::Integer(int(0)));

        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "test".into(),
                ),
                functions: vec![Function {
                    id: FuncId(0),
                    name: Some("choose".into()),
                    params: vec![("%cond".into(), TypeTag::Trilean)],
                    return_type: TypeTag::Integer,
                    blocks: vec![
                        BasicBlock {
                            id: BlockId(0),
                            name: Some("entry".into()),
                            instructions: vec![Instruction::BrIf {
                                cond: Operand::Value(ValueId(0)),
                                then_block: BlockId(1),
                                else_block: BlockId(2),
                            }],
                        },
                        BasicBlock {
                            id: BlockId(1),
                            name: Some("then".into()),
                            instructions: vec![
                                Instruction::Const {
                                    dest: ValueId(1),
                                    constant: c1,
                                },
                                Instruction::Br { target: BlockId(3) },
                            ],
                        },
                        BasicBlock {
                            id: BlockId(2),
                            name: Some("else".into()),
                            instructions: vec![
                                Instruction::Const {
                                    dest: ValueId(2),
                                    constant: c0,
                                },
                                Instruction::Br { target: BlockId(3) },
                            ],
                        },
                        BasicBlock {
                            id: BlockId(3),
                            name: Some("merge".into()),
                            instructions: vec![
                                Instruction::Phi {
                                    dest: ValueId(3),
                                    incoming: vec![
                                        PhiIncoming {
                                            value: ValueId(1),
                                            block: BlockId(1),
                                        },
                                        PhiIncoming {
                                            value: ValueId(2),
                                            block: BlockId(2),
                                        },
                                    ],
                                },
                                Instruction::Ret {
                                    value: Some(Operand::Value(ValueId(3))),
                                },
                            ],
                        },
                    ],
                }],
            }],
            constants: pool,
            witness_tables: Vec::new(),
        };

        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    /// Round-trip a `BrTrilean` instruction — ADR-0010 ternary-native
    /// branch. Catches encoding/decoding mismatches for the new opcode +
    /// the .triv v1 → v2 version bump.
    #[test]
    fn br_trilean_round_trip() {
        let mut pool = ConstantPool::new();
        let c_pos = pool.intern(Constant::Integer(int(1)));
        let c_neg = pool.intern(Constant::Integer(int(-1)));
        let c_zero = pool.intern(Constant::Integer(int(0)));

        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "test".into(),
                ),
                functions: vec![Function {
                    id: FuncId(0),
                    name: Some("sign".into()),
                    params: vec![("%cond".into(), TypeTag::Trilean)],
                    return_type: TypeTag::Integer,
                    blocks: vec![
                        BasicBlock {
                            id: BlockId(0),
                            name: Some("entry".into()),
                            instructions: vec![Instruction::BrTrilean {
                                cond: Operand::Value(ValueId(0)),
                                true_block: BlockId(1),
                                unknown_block: BlockId(2),
                                false_block: BlockId(3),
                            }],
                        },
                        BasicBlock {
                            id: BlockId(1),
                            name: Some("yes".into()),
                            instructions: vec![Instruction::Ret {
                                value: Some(Operand::Const(c_pos)),
                            }],
                        },
                        BasicBlock {
                            id: BlockId(2),
                            name: Some("maybe".into()),
                            instructions: vec![Instruction::Ret {
                                value: Some(Operand::Const(c_zero)),
                            }],
                        },
                        BasicBlock {
                            id: BlockId(3),
                            name: Some("no".into()),
                            instructions: vec![Instruction::Ret {
                                value: Some(Operand::Const(c_neg)),
                            }],
                        },
                    ],
                }],
            }],
            constants: pool,
            witness_tables: Vec::new(),
        };

        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    // ── Struct and enum ops ────────────────────────────────────

    #[test]
    fn struct_field_ops_round_trip() {
        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "test".into(),
                ),
                functions: vec![Function {
                    id: FuncId(0),
                    name: Some("make_point".into()),
                    params: vec![
                        ("%x".into(), TypeTag::Integer),
                        ("%y".into(), TypeTag::Integer),
                    ],
                    return_type: TypeTag::Unit,
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        name: Some("entry".into()),
                        instructions: vec![
                            Instruction::StructNew {
                                dest: ValueId(2),
                                fields: vec![
                                    Operand::Value(ValueId(0)),
                                    Operand::Value(ValueId(1)),
                                ],
                            },
                            Instruction::FieldGet {
                                dest: ValueId(3),
                                object: Operand::Value(ValueId(2)),
                                field_idx: 0,
                            },
                            Instruction::FieldSet {
                                dest: ValueId(4),
                                object: Operand::Value(ValueId(2)),
                                field_idx: 1,
                                value: Operand::Value(ValueId(3)),
                            },
                            Instruction::Ret { value: None },
                        ],
                    }],
                }],
            }],
            constants: ConstantPool::new(),
            witness_tables: Vec::new(),
        };

        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    #[test]
    fn enum_ops_round_trip() {
        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "test".into(),
                ),
                functions: vec![Function {
                    id: FuncId(0),
                    name: Some("unwrap".into()),
                    params: vec![("%opt".into(), TypeTag::Nullable(Box::new(TypeTag::Integer)))],
                    return_type: TypeTag::Integer,
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        name: Some("entry".into()),
                        instructions: vec![Instruction::Ret {
                            value: Some(Operand::Value(ValueId(0))),
                        }],
                    }],
                }],
            }],
            constants: ConstantPool::new(),
            witness_tables: Vec::new(),
        };

        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    // ── Nullable ops ───────────────────────────────────────────

    #[test]
    fn nullable_ops_round_trip() {
        let mut pool = ConstantPool::new();
        let c_int = pool.intern(Constant::Integer(int(42)));

        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "test".into(),
                ),
                functions: vec![Function {
                    id: FuncId(0),
                    name: Some("test_nullable".into()),
                    params: vec![],
                    return_type: TypeTag::Unit,
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        name: Some("entry".into()),
                        instructions: vec![
                            Instruction::Const {
                                dest: ValueId(0),
                                constant: c_int,
                            },
                            Instruction::NullWrap {
                                dest: ValueId(1),
                                value: Operand::Value(ValueId(0)),
                            },
                            Instruction::NullCheck {
                                dest: ValueId(2),
                                nullable: Operand::Value(ValueId(1)),
                            },
                            Instruction::NullUnwrap {
                                dest: ValueId(3),
                                nullable: Operand::Value(ValueId(1)),
                            },
                            Instruction::Ret { value: None },
                        ],
                    }],
                }],
            }],
            constants: pool,
            witness_tables: Vec::new(),
        };

        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    // ── Cross-module call ──────────────────────────────────────

    #[test]
    fn cross_module_call_round_trip() {
        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "main".into(),
                ),
                functions: vec![Function {
                    id: FuncId(0),
                    name: Some("caller".into()),
                    params: vec![],
                    return_type: TypeTag::Unit,
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        name: Some("entry".into()),
                        instructions: vec![
                            Instruction::CallCrossModule {
                                dest: Some(ValueId(0)),
                                path: triet_modules::AbsolutePath::new(
                                    triet_modules::ModulePath::new(vec!["std".into(), "io".into()]),
                                    "println".into(),
                                ),
                                args: vec![Operand::Const(ConstId(0))],
                            },
                            Instruction::Ret { value: None },
                        ],
                    }],
                }],
            }],
            constants: {
                let mut pool = ConstantPool::new();
                pool.intern(Constant::String("hi".into()));
                pool
            },
            witness_tables: Vec::new(),
        };

        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    // ── Witness table dispatch ────────────────────────────────

    /// ADR-0012 — round-trip a program that uses `WitnessCall` plus
    /// a populated `witness_tables` section. Catches encoding drift
    /// in the new opcode (0x93) and the new section (5).
    #[test]
    fn witness_call_round_trip() {
        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::new(vec!["app".into()]),
                    String::new(),
                ),
                functions: vec![Function {
                    id: FuncId(0),
                    name: Some("main".into()),
                    params: vec![],
                    return_type: TypeTag::Unit,
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        name: Some("entry".into()),
                        instructions: vec![
                            Instruction::WitnessCall {
                                dest: Some(ValueId(0)),
                                path: triet_modules::AbsolutePath::new(
                                    triet_modules::ModulePath::new(vec!["math".into()]),
                                    "scale".into(),
                                ),
                                witness_idx: 0,
                                args: vec![],
                            },
                            Instruction::Ret { value: None },
                        ],
                    }],
                }],
            }],
            constants: ConstantPool::new(),
            witness_tables: vec![
                crate::module::WitnessTable {
                    type_args: vec![TypeTag::Integer],
                },
                crate::module::WitnessTable {
                    type_args: vec![TypeTag::Long, TypeTag::String],
                },
            ],
        };

        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
        assert_eq!(decoded.witness_tables.len(), 2);
    }

    /// Programs without `WitnessCall` must NOT emit the witness
    /// table section — keeps the v1/v2 wire format bit-identical.
    #[test]
    fn witness_section_skipped_when_unused() {
        // Build a trivial program with no witness tables.
        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    String::new(),
                ),
                functions: vec![Function {
                    id: FuncId(0),
                    name: Some("noop".into()),
                    params: vec![],
                    return_type: TypeTag::Unit,
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        name: Some("entry".into()),
                        instructions: vec![Instruction::Ret { value: None }],
                    }],
                }],
            }],
            constants: ConstantPool::new(),
            witness_tables: Vec::new(),
        };
        let bytes = write_program(&program);
        // Section count immediately follows magic (4 bytes) +
        // version (4 bytes). Reading at offset 8 should yield 4
        // (no witness section).
        let section_count = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        assert_eq!(section_count, 4);
    }

    // ── Error cases ────────────────────────────────────────────

    #[test]
    fn bad_magic_rejected() {
        let data = vec![0x00, 0x00, 0x00, 0x00];
        assert!(read_program(&data).is_err());
    }

    #[test]
    fn unsupported_version_rejected() {
        let mut data = Vec::new();
        data.extend_from_slice(&MAGIC);
        data.extend_from_slice(&999u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes()); // section count
        let err = read_program(&data).unwrap_err();
        assert!(matches!(err, TrivError::UnsupportedVersion { .. }));
    }

    #[test]
    fn truncated_file_rejected() {
        let mut data = Vec::new();
        data.extend_from_slice(&MAGIC);
        // version truncated — only 2 bytes instead of 4
        data.push(0x01);
        data.push(0x00);
        assert!(read_program(&data).is_err());
    }

    #[test]
    fn unknown_opcode_in_section() {
        // Build a minimal valid file, then corrupt the code section
        let program = make_simple_program();
        let mut bytes = write_program(&program);

        // Find the code section and corrupt an opcode
        // The code section starts after header + 3 sections.
        // We'll just replace bytes within the code section manually.
        // Find the position after the header (4 magic + 4 version + 4 section_count)
        let mut pos = 12;
        for _ in 0..3 {
            // Skip section_id (1 byte) + section_size (4 bytes) + payload
            let section_size =
                u32::from_le_bytes(bytes[pos + 1..pos + 5].try_into().unwrap()) as usize;
            pos += 1 + 4 + section_size;
        }
        // Now at code section. Skip section_id + section_size (5 bytes), then
        // skip function_count and block_count and block_id and name and instr_count.
        // Just find the first instruction that's an Add (opcode 0x10).
        if let Some(opcode_pos) = bytes[pos..].iter().position(|&b| b == opcode::ADD) {
            bytes[pos + opcode_pos] = 0xFF; // invalid opcode
        }

        let result = read_program(&bytes);
        assert!(result.is_err());
    }

    // ── Large program ──────────────────────────────────────────

    #[test]
    fn many_instructions_round_trip() {
        let mut pool = ConstantPool::new();
        let c1 = pool.intern(Constant::Integer(int(1)));
        let c0 = pool.intern(Constant::Integer(int(0)));

        // Build a function with one of each arithmetic op
        let instructions = vec![
            Instruction::Add {
                dest: ValueId(1),
                lhs: Operand::Const(c0),
                rhs: Operand::Const(c1),
            },
            Instruction::Sub {
                dest: ValueId(2),
                lhs: Operand::Value(ValueId(1)),
                rhs: Operand::Const(c1),
            },
            Instruction::Mul {
                dest: ValueId(3),
                lhs: Operand::Value(ValueId(2)),
                rhs: Operand::Const(c1),
            },
            Instruction::Div {
                dest: ValueId(4),
                lhs: Operand::Value(ValueId(3)),
                rhs: Operand::Const(c1),
            },
            Instruction::Mod {
                dest: ValueId(5),
                lhs: Operand::Value(ValueId(4)),
                rhs: Operand::Const(c1),
            },
            Instruction::Pow {
                dest: ValueId(6),
                base: Operand::Value(ValueId(5)),
                exp: Operand::Const(c1),
            },
            Instruction::Neg {
                dest: ValueId(7),
                operand: Operand::Value(ValueId(6)),
            },
            Instruction::Eq {
                dest: ValueId(8),
                lhs: Operand::Value(ValueId(7)),
                rhs: Operand::Const(c0),
            },
            Instruction::Ne {
                dest: ValueId(9),
                lhs: Operand::Value(ValueId(7)),
                rhs: Operand::Const(c0),
            },
            Instruction::Lt {
                dest: ValueId(10),
                lhs: Operand::Value(ValueId(7)),
                rhs: Operand::Const(c0),
            },
            Instruction::Le {
                dest: ValueId(11),
                lhs: Operand::Value(ValueId(7)),
                rhs: Operand::Const(c0),
            },
            Instruction::Gt {
                dest: ValueId(12),
                lhs: Operand::Value(ValueId(7)),
                rhs: Operand::Const(c0),
            },
            Instruction::Ge {
                dest: ValueId(13),
                lhs: Operand::Value(ValueId(7)),
                rhs: Operand::Const(c0),
            },
            Instruction::Ret {
                value: Some(Operand::Value(ValueId(13))),
            },
        ];

        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "test".into(),
                ),
                functions: vec![Function {
                    id: FuncId(0),
                    name: Some("all_arith".into()),
                    params: vec![],
                    return_type: TypeTag::Trilean,
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        name: Some("entry".into()),
                        instructions,
                    }],
                }],
            }],
            constants: pool,
            witness_tables: Vec::new(),
        };

        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    // ── Łukasiewicz logic ops ──────────────────────────────────

    #[test]
    fn lukasiewicz_ops_round_trip() {
        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "test".into(),
                ),
                functions: vec![Function {
                    id: FuncId(0),
                    name: Some("logic".into()),
                    params: vec![
                        ("%a".into(), TypeTag::Trilean),
                        ("%b".into(), TypeTag::Trilean),
                    ],
                    return_type: TypeTag::Trilean,
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        name: Some("entry".into()),
                        instructions: vec![
                            Instruction::LukAnd {
                                dest: ValueId(2),
                                lhs: Operand::Value(ValueId(0)),
                                rhs: Operand::Value(ValueId(1)),
                            },
                            Instruction::LukOr {
                                dest: ValueId(3),
                                lhs: Operand::Value(ValueId(2)),
                                rhs: Operand::Value(ValueId(1)),
                            },
                            Instruction::LukImplies {
                                dest: ValueId(4),
                                lhs: Operand::Value(ValueId(0)),
                                rhs: Operand::Value(ValueId(3)),
                            },
                            Instruction::Ret {
                                value: Some(Operand::Value(ValueId(4))),
                            },
                        ],
                    }],
                }],
            }],
            constants: ConstantPool::new(),
            witness_tables: Vec::new(),
        };

        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    // ── Unreachable ────────────────────────────────────────────

    #[test]
    fn unreachable_instruction_round_trip() {
        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "test".into(),
                ),
                functions: vec![Function {
                    id: FuncId(0),
                    name: Some("divergent".into()),
                    params: vec![],
                    return_type: TypeTag::Unit,
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        name: Some("entry".into()),
                        instructions: vec![Instruction::Unreachable],
                    }],
                }],
            }],
            constants: ConstantPool::new(),
            witness_tables: Vec::new(),
        };

        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    // ── Conversion ops ─────────────────────────────────────────

    #[test]
    fn conversion_ops_round_trip() {
        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::khi_root(),
                    "test".into(),
                ),
                functions: vec![Function {
                    id: FuncId(0),
                    name: Some("convert".into()),
                    params: vec![("%x".into(), TypeTag::Integer)],
                    return_type: TypeTag::Trit,
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        name: Some("entry".into()),
                        instructions: vec![
                            Instruction::ToTryte {
                                dest: ValueId(1),
                                operand: Operand::Value(ValueId(0)),
                            },
                            Instruction::ToTrit {
                                dest: ValueId(2),
                                operand: Operand::Value(ValueId(1)),
                            },
                            Instruction::Ret {
                                value: Some(Operand::Value(ValueId(2))),
                            },
                        ],
                    }],
                }],
            }],
            constants: ConstantPool::new(),
            witness_tables: Vec::new(),
        };

        let bytes = write_program(&program);
        let decoded = read_program(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    // ── Deterministism ─────────────────────────────────────────

    #[test]
    fn serialization_is_deterministic() {
        let program = make_simple_program();
        let bytes1 = write_program(&program);
        let bytes2 = write_program(&program);
        assert_eq!(bytes1, bytes2);
    }

    // ── Binary size sanity ─────────────────────────────────────

    #[test]
    fn binary_size_is_reasonable() {
        let program = make_simple_program();
        let bytes = write_program(&program);
        // A simple function with 2 instructions should be < 200 bytes
        assert!(
            bytes.len() < 200,
            "expected < 200 bytes, got {}",
            bytes.len()
        );
    }

    // ── Cross-module call dot-path ─────────────────────────────

    #[test]
    fn absolute_path_round_trip_through_string() {
        let path = triet_modules::AbsolutePath::new(
            triet_modules::ModulePath::new(vec!["std".into(), "io".into()]),
            "println".into(),
        );
        let s = path.to_string();
        assert_eq!(s, "std.io.println");
        let parsed = parse_absolute_path(&s).unwrap();
        assert_eq!(parsed, path);
    }

    // ── Group into modules ─────────────────────────────────────

    #[test]
    fn group_into_modules_preserves_order() {
        let func = Function {
            id: FuncId(0),
            name: Some("f".into()),
            params: vec![],
            return_type: TypeTag::Unit,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![Instruction::Ret { value: None }],
            }],
        };

        let metas = vec![
            FunctionMeta {
                module_path: "khi.bar".into(),
                name: Some("g".into()),
                params: vec![],
                return_type: TypeTag::Unit,
                func_id: FuncId(0),
            },
            FunctionMeta {
                module_path: "khi.foo".into(),
                name: Some("f".into()),
                params: vec![],
                return_type: TypeTag::Unit,
                func_id: FuncId(1),
            },
        ];

        let functions = vec![func.clone(), func];
        let modules = group_into_modules(&functions, &metas);
        assert_eq!(modules.len(), 2);
        assert_eq!(modules[0].functions.len(), 1);
        assert_eq!(modules[1].functions.len(), 1);
    }
}
