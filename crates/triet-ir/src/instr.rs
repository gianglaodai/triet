//! IR instructions — the flat, register-based operations of the Triết
//! intermediate representation.
//!
//! Per [ADR-0007], every instruction that produces a value writes to an
//! explicit destination `ValueId`. Terminators (branch, return, unreachable)
//! produce no value and must appear last in a basic block.
//!
//! # Doc convention
//!
//! Every variant carries a doc comment; individual fields are
//! self-documenting by name + type and are not separately documented.
//!
//! [ADR-0007]: ../../../docs/decisions/0007-ir-design.md
#![allow(missing_docs)]

use crate::types::{BlockId, ConstId, FuncId, ValueId};

/// An operand to an instruction — either a virtual register value or
/// an inline constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operand {
    /// An SSA virtual register value.
    Value(ValueId),
    /// A compile-time constant from the constant pool.
    Const(ConstId),
}

impl From<ValueId> for Operand {
    fn from(v: ValueId) -> Self {
        Self::Value(v)
    }
}

impl From<ConstId> for Operand {
    fn from(c: ConstId) -> Self {
        Self::Const(c)
    }
}

/// A pair of (value, source block) for phi node incoming edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PhiIncoming {
    /// The value coming from this predecessor.
    pub value: ValueId,
    /// The predecessor block this value arrives from.
    pub block: BlockId,
}

/// An IR instruction.
///
/// Instructions are grouped per [ADR-0007 § Phân nhóm instruction]:
/// constants, arithmetic, logic, comparison, conversion, aggregate,
/// nullable, function calls, closure, control flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Instruction {
    // ── Constants ──────────────────────────────────────────────
    /// Materialize a constant from the pool into a register.
    /// `const Integer 42_integer` → `%v = Const(c42)`.
    Const {
        /// Destination virtual register.
        dest: ValueId,
        /// Constant pool reference.
        constant: ConstId,
    },

    // ── Arithmetic (balanced ternary, all Integer/Tryte/Long) ──
    /// Addition: `%d = Add %lhs, %rhs`.
    Add {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// Subtraction: `%d = Sub %lhs, %rhs`.
    Sub {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// Multiplication: `%d = Mul %lhs, %rhs`.
    Mul {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// Balanced ternary division (rounds toward nearest, no bias): `%d = Div %lhs, %rhs`.
    Div {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// Balanced ternary modulo: `%d = Mod %lhs, %rhs`.
    Mod {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// Exponentiation (right-associative in source): `%d = Pow %base, %exp`.
    Pow {
        dest: ValueId,
        base: Operand,
        exp: Operand,
    },
    /// Negation (unary minus): `%d = Neg %operand`.
    Neg { dest: ValueId, operand: Operand },

    // ── Logic: Łukasiewicz Ł3 (default) ────────────────────────
    /// Ł3 AND (= min): `%d = LukAnd %lhs, %rhs`.
    LukAnd {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// Ł3 OR (= max): `%d = LukOr %lhs, %rhs`.
    LukOr {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// Ł3 implication `=>`: `%d = LukImplies %lhs, %rhs`.
    /// `min(1, 1-a+b)` — unknown=>unknown = true.
    LukImplies {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// Ł3 XOR: `%d = LukXor %lhs, %rhs`.
    LukXor {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// Ł3 biconditional `<=>`: `%d = LukIff %lhs, %rhs`.
    /// `(a => b) && (b => a)` trong Ł3.
    LukIff {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },

    // ── Logic: Kleene K3 (alternative, `~` prefix) ─────────────
    /// K3 implication `~>`: `%d = KleeneImplies %lhs, %rhs`.
    /// `max(1-a, b)` — unknown~>unknown = unknown.
    KleeneImplies {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// K3 XOR `~^`: `%d = KleeneXor %lhs, %rhs`.
    KleeneXor {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// K3 biconditional `<~>`: `%d = KleeneIff %lhs, %rhs`.
    KleeneIff {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },

    // ── Comparison (result: Trilean, always known — true or false) ──
    /// Value equality `==`: `%d = Eq %lhs, %rhs`.
    Eq {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// Value inequality `!=`: `%d = Ne %lhs, %rhs`.
    Ne {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// Less than: `%d = Lt %lhs, %rhs`.
    Lt {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// Less than or equal: `%d = Le %lhs, %rhs`.
    Le {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// Greater than: `%d = Gt %lhs, %rhs`.
    Gt {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },
    /// Greater than or equal: `%d = Ge %lhs, %rhs`.
    Ge {
        dest: ValueId,
        lhs: Operand,
        rhs: Operand,
    },

    // ── Conversion (explicit, per SPEC §2.4) ───────────────────
    /// Widening (no loss): `%d = ToInteger %v`.
    ToInteger { dest: ValueId, operand: Operand },
    /// Narrowing, panic on overflow: `%d = ToTryte %v`.
    ToTryte { dest: ValueId, operand: Operand },
    /// Widening to 81-trit: `%d = ToLong %v`.
    ToLong { dest: ValueId, operand: Operand },
    /// Narrow to 1-trit, panic on overflow: `%d = ToTrit %v`.
    ToTrit { dest: ValueId, operand: Operand },
    /// Convert to Trilean: `%d = ToTrilean %v`.
    ToTrilean { dest: ValueId, operand: Operand },

    // ── Aggregate: struct ──────────────────────────────────────
    /// Allocate a struct: `%d = StructNew [fields]`.
    StructNew {
        dest: ValueId,
        /// Fields in declaration order.
        fields: Vec<Operand>,
    },
    /// Read a named field: `%d = FieldGet %object, %field_idx`.
    FieldGet {
        dest: ValueId,
        object: Operand,
        /// Field index (0-based, in declaration order).
        field_idx: u32,
    },
    /// Write a named field (returns the updated struct): `%d = FieldSet %object, %field_idx, %value`.
    FieldSet {
        dest: ValueId,
        object: Operand,
        field_idx: u32,
        value: Operand,
    },

    // ── Aggregate: enum ────────────────────────────────────────
    /// Construct an enum variant: `%d = EnumNew %variant_idx, [payload]`.
    EnumNew {
        dest: ValueId,
        /// Variant index (0-based).
        variant_idx: u32,
        /// Optional payload value (None for unit variants).
        payload: Option<Operand>,
    },
    /// Get the variant discriminant (Trit): `%d = EnumTag %scrutinee`.
    EnumTag { dest: ValueId, scrutinee: Operand },
    /// Unpack the payload of a known variant: `%d = EnumPayload %scrutinee`.
    /// Panics at runtime if the variant tag doesn't match what the
    /// type-checker expected.
    EnumPayload { dest: ValueId, scrutinee: Operand },

    // ── Nullable ───────────────────────────────────────────────
    /// Wrap a value as nullable: `T → T?`.
    NullWrap { dest: ValueId, value: Operand },
    /// Force-unwrap a nullable value (panic if null): `T? → T`.
    NullUnwrap { dest: ValueId, nullable: Operand },
    /// Check if a nullable is non-null (returns Trit): `T? → Trit`.
    /// - `+1` = non-null
    /// - `0` = null
    NullCheck { dest: ValueId, nullable: Operand },

    // ── Function calls ─────────────────────────────────────────
    /// Local function call (intra-module): `%d = CallLocal @func(%args)`.
    CallLocal {
        dest: Option<ValueId>,
        callee: FuncId,
        args: Vec<Operand>,
    },
    /// Cross-module call with absolute path: `%d = CallCrossModule path(%args)`.
    /// The `AbsolutePath` preserves the capability namespace for v0.6.
    CallCrossModule {
        dest: Option<ValueId>,
        path: triet_modules::AbsolutePath,
        args: Vec<Operand>,
    },
    /// Cross-package generic dispatch via a witness table
    /// (ADR-0012): `%d = WitnessCall path[witness_idx](%args)`.
    ///
    /// Unlike `CallCrossModule`, this opcode carries a witness-table
    /// index so the callee can recover the concrete type arguments
    /// at runtime. The lowerer emits this for any call that targets
    /// a generic export of a separately-compiled package; intra-
    /// package generic calls keep using monomorphization, so they
    /// still emit `CallLocal`. See `IrProgram::witness_tables`.
    WitnessCall {
        dest: Option<ValueId>,
        path: triet_modules::AbsolutePath,
        /// Index into `IrProgram::witness_tables`. Linker dedups so
        /// the same `(path, type_args)` shares one entry.
        witness_idx: u32,
        args: Vec<Operand>,
    },
    /// Builtin dispatch: `%d = CallBuiltin name(%args)`.
    CallBuiltin {
        dest: Option<ValueId>,
        name: BuiltinName,
        args: Vec<Operand>,
    },

    // ── Closure ────────────────────────────────────────────────
    /// Create a closure capturing live variables: `%d = ClosureNew @lambda, [captures]`.
    ClosureNew {
        dest: ValueId,
        lambda: FuncId,
        captures: Vec<ValueId>,
    },
    /// Call a closure: `%d = ClosureCall %closure(%args)`.
    ClosureCall {
        dest: Option<ValueId>,
        closure: Operand,
        args: Vec<Operand>,
    },

    // ── Control flow (terminators — no dest, last in block) ────
    /// Unconditional branch: `Br target`.
    Br { target: BlockId },
    /// Conditional branch on Trilean condition: `BrIf %cond, then, else`.
    /// `true` → `then_block`, `false` or `unknown` → `else_block`.
    ///
    /// **DEPRECATED in favor of `BrTrilean`** per ADR-0010. Retained for
    /// .triv v1 backward compatibility and for genuinely-binary callers
    /// (e.g. branching on a Trit whose third state is statically impossible).
    /// New lowerer code must emit `BrTrilean` instead so the Unknown state
    /// is preserved at the IR level.
    BrIf {
        cond: Operand,
        then_block: BlockId,
        else_block: BlockId,
    },
    /// Three-way branch on a Trilean condition: ADR-0010 ternary-native IR.
    ///
    /// Dispatches directly on the three Ł3 truth values:
    /// - `Trilean::True`    → `true_block`
    /// - `Trilean::Unknown` → `unknown_block`
    /// - `Trilean::False`   → `false_block`
    ///
    /// When two of the three targets are the same the opcode degenerates to
    /// classical two-way branching; the lowerer uses this to express `if?`
    /// (`unknown_block == false_block`) vs plain `if` (`unknown_block`
    /// jumps to an `Unreachable` block, panicking per SPEC §7.1.1).
    BrTrilean {
        cond: Operand,
        true_block: BlockId,
        unknown_block: BlockId,
        false_block: BlockId,
    },
    /// Return from the current function: `Ret [%value]`.
    Ret { value: Option<Operand> },
    /// Unreachable — marks a code path that must never execute.
    /// Emitted after exhaustive match or as a compiler invariant marker.
    Unreachable,

    // ── Phi node (must appear first in block) ──────────────────
    /// SSA φ-node merging values from multiple predecessors.
    /// `%d = Phi [(%v1, b1), (%v2, b2)]`.
    Phi {
        dest: ValueId,
        incoming: Vec<PhiIncoming>,
    },
}

/// Builtin function names recognized by the IR.
///
/// These mirror the stdlib builtins from v0.2 that the interpreter
/// currently dispatches directly. The list will grow with stdlib.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinName {
    /// `println(text)` — print a line.
    Println,
    /// `print(text)` — print without newline.
    Print,
    /// `assert(cond, msg)` — panic if condition is false or unknown.
    Assert,
    /// `assert_eq(a, b)` — panic if not equal.
    AssertEq,
    /// F-string concatenation: convert all args to string and join.
    /// Internal builtin — not user-callable.
    FStringConcat,
    /// `std.text.len(s)` — return UTF-8 char count of a string.
    TextLen,
    /// `std.text.concat(a, b)` — string concatenation.
    TextConcat,
    /// `std.text.from_integer(n)` — integer → decimal string.
    TextFromInteger,
}
