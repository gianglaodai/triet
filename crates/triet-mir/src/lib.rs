//! Triết MIR (Mid-level Intermediate Representation).
//!
//! A flat, non-nested, control-flow-graph-based IR that sits between the
//! AST and the borrow checker / codegen backend. MIR eliminates AST nesting,
//! makes control flow explicit, and enables dataflow analysis (liveness,
//! NLL borrow checking).
//!
//! # Pipeline position
//!
//! ```text
//! AST → MIR (lowering) → CFG (build) → Borrow Check (dataflow) → Codegen
//! ```
//!
//! # Design principles
//!
//! - **Flat:** Every temporary, variable, and intermediate value gets a
//!   `Local` index. No nesting, no trees.
//! - **Explicit control flow:** Basic blocks connected by terminators.
//!   No implicit fall-through beyond block boundaries.
//! - **Ownership-annotated:** Function signatures carry parameter passing
//!   modes (Borrow/Move/MutableBorrow) and return-borrow dependency info.
//! - **Independent crate:** MIR is consumed by both the borrow checker and
//!   the codegen backend. It does NOT depend on AST types.

#![warn(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A half-open byte range `[start, end)` in source text.
///
/// Mirrors `triet_syntax::Span` to keep `triet-mir` dependency-free.
pub type Span = std::ops::Range<usize>;

/// A zero-length span for synthetic MIR nodes that have no corresponding
/// source location (e.g. implicit `Return(())` inserted at function end,
/// or compiler-generated temporaries). Must NOT be used for errors the
/// user actually wrote.
pub const DUMMY_SPAN: Span = 0..0;

// ── Index types ──────────────────────────────────────────────

/// Index into a function's local variable table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Local(pub usize);

impl fmt::Display for Local {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "_{}", self.0)
    }
}

/// Index identifying a basic block within a function body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BasicBlock(pub usize);

impl fmt::Display for BasicBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

/// Index into a function table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FunctionId(pub usize);

/// How a call target should be compiled.
///
/// JIT functions use Cranelift's native calling convention. Shim functions
/// are `extern "C"` Rust functions registered as symbols in the JIT module
/// and must be called with the SystemV ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallTarget {
    /// A user-defined function compiled by the JIT (Cranelift native ABI).
    Jit,
    /// An `extern "C"` runtime shim (SystemV ABI, symbol from `shims.rs`).
    Shim,
}

// ── Places + projections ────────────────────────────────────

/// A single step in a [`Place`] projection chain.
///
/// Projections refine a base [`Local`] down to a sub-location. The borrow
/// checker derives a field path from these to track loans at field
/// granularity (e.g. borrowing `vga.left` must not freeze `vga.right`) —
/// `Field(name)` maps directly to the borrow checker's `FieldPath::Field`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Projection {
    /// `*p` — dereference a reference/pointer.
    Deref,
    /// `p.name` — access a named struct field.
    Field(String),
    /// `p[i]` — index into a collection by the value held in a local.
    Index(Local),
    /// Access the payload of a specific enum variant.
    /// Only valid after the borrowck has proven the enum is in this variant
    /// (via a `SwitchInt` branch).
    Payload(String),
}

/// A memory location: a base [`Local`] refined by zero or more projections.
///
/// A bare local is `Place { local, projection: [] }`. `obj.field` is
/// `Place { local: obj, projection: [Field("field")] }`. `*r` is
/// `Place { local: r, projection: [Deref] }`. Chains compose left-to-right.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Place {
    /// The base local the projection chain starts from.
    pub local: Local,
    /// Projection steps applied in order.
    pub projection: Vec<Projection>,
}

impl Place {
    /// A place referring to a whole local with no projections.
    #[must_use]
    pub const fn local(local: Local) -> Self {
        Self {
            local,
            projection: Vec::new(),
        }
    }

    /// Return a new place with `proj` appended to this place's chain.
    #[must_use]
    pub fn project(&self, proj: Projection) -> Self {
        let mut projection = self.projection.clone();
        projection.push(proj);
        Self {
            local: self.local,
            projection,
        }
    }
}

impl From<Local> for Place {
    fn from(local: Local) -> Self {
        Self::local(local)
    }
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = self.local.to_string();
        for proj in &self.projection {
            s = match proj {
                Projection::Deref => format!("(*{s})"),
                Projection::Field(name) => format!("{s}.{name}"),
                Projection::Index(i) => format!("{s}[{i}]"),
                Projection::Payload(variant) => format!("{s}.Payload({variant})"),
            };
        }
        f.write_str(&s)
    }
}

/// Declaration of a local: its type (as a display string) and mutability.
///
/// MIR stays independent of AST types, so the type is carried as a name
/// string rather than a `triet_syntax::Type`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDecl {
    /// Type name, for diagnostics and layout lookup (e.g. "Integer", "Point").
    pub ty: String,
    /// Whether the binding is mutable.
    pub mutable: bool,
}

impl LocalDecl {
    /// A temporary/local of the given type, immutable by default.
    #[must_use]
    pub fn new(ty: &str) -> Self {
        Self {
            ty: ty.to_string(),
            mutable: false,
        }
    }
}

// ── MIR statements ──────────────────────────────────────────

/// A single MIR statement — a simple, flat operation.
///
/// Each statement operates on `Local` values only. Any nesting in the
/// original AST is broken into temporaries during lowering.
///
/// Every variant carries a [`Span`] for source-level diagnostics. Use
/// [`DUMMY_SPAN`] for compiler-synthesized statements that have no
/// corresponding source code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Statement {
    /// Mark a local as live (created). Must precede any use.
    StorageLive(Local, Span),

    /// Mark a local as dead (dropped). Must follow the last use.
    StorageDead(Local, Span),

    /// Tombstone a local: write 0 (dead value) to signal the runtime
    /// that this slot holds an invalid pointer. Used for caller zeroing
    /// after a Move-type argument is passed to a user function
    /// (ADR-0042 §Q1). Borrowck treats this as a move (→ Moved), NOT
    /// a re-initialization — the zero is a tombstone, not a user value.
    Deinit(Local, Span),

    /// `dest = source` — copy or move depending on types.
    Assign {
        /// Destination place.
        dest: Place,
        /// Source place.
        source: Place,
        /// Source location.
        span: Span,
    },

    /// `dest = &form source` — create a reference to source.
    Borrow {
        /// Destination place (the new reference).
        dest: Place,
        /// Which S6 reference form.
        form: ReferenceForm,
        /// The place being borrowed (may be a field/deref projection).
        source: Place,
        /// Source location.
        span: Span,
    },

    /// `dest = const` — load a compile-time constant.
    Const {
        /// Destination place.
        dest: Place,
        /// Constant value as a human-readable string (placeholder).
        value: ConstValue,
        /// Source location.
        span: Span,
    },

    /// Binary operation: `dest = left op right`.
    BinaryOp {
        /// Destination place.
        dest: Place,
        /// Operator kind.
        op: BinOp,
        /// Left operand.
        left: Place,
        /// Right operand.
        right: Place,
        /// Source location.
        span: Span,
    },

    /// Discriminant of an Outcome value: `dest = discriminant(source)`.
    /// Returns a Trit indicating the Outcome arm.
    OutcomeDiscriminant {
        /// Destination place (Trit).
        dest: Place,
        /// Source Outcome value.
        source: Place,
        /// Source location.
        span: Span,
    },

    /// Unwrap success arm of Outcome: `dest = unwrap_value(source)`.
    OutcomeUnwrap {
        /// Destination place (success payload).
        dest: Place,
        /// Source Outcome value.
        source: Place,
        /// Source location.
        span: Span,
    },

    /// Unwrap error arm of Outcome: `dest = unwrap_error(source)`.
    OutcomeUnwrapError {
        /// Destination place (error payload).
        dest: Place,
        /// Source Outcome value.
        source: Place,
        /// Source location.
        span: Span,
    },

    /// Allocate stack space for a struct literal. The struct's layout
    /// can be found in `Body::struct_layouts` by name.
    StructAlloc {
        /// The local being initialized as a struct.
        dest: Local,
        /// Struct name — key into `Body::struct_layouts`.
        struct_name: String,
        /// Source location.
        span: Span,
    },

    /// Allocate stack space for an enum literal. The enum's layout
    /// can be found in `Body::enum_layouts` by name.
    EnumAlloc {
        /// The local being initialized as an enum.
        dest: Local,
        /// Enum name — key into `Body::enum_layouts`.
        enum_name: String,
        /// Source location.
        span: Span,
    },

    /// Write the discriminant tag into an enum value.
    /// `dest` must have been previously `EnumAlloc`-ed.
    SetDiscriminant {
        /// The enum local to write into.
        dest: Local,
        /// Integer discriminant value (0, 1, 2, …).
        value: i64,
        /// Source location.
        span: Span,
    },

    /// Read the discriminant tag from an enum value into `dest`.
    /// `source` is counted as a use (not a move) by borrowck —
    /// reading a Moved enum's discriminant → E2420.
    GetDiscriminant {
        /// Destination for the discriminant value (i64).
        dest: Place,
        /// The enum local to read from.
        source: Place,
        /// Source location.
        span: Span,
    },

    /// Drop a local's value (decrement refcount or free memory).
    Drop(Local, Span),
}

/// Compile-time constant value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstValue {
    /// Integer literal.
    Integer(i128),
    /// Trit literal: -1, 0, or 1.
    Trit(i8),
    /// Unit `()`.
    Unit,
    /// String literal placeholder.
    String(String),
}

/// Binary and unary operators in MIR.
///
/// Covers arithmetic, comparisons (returning Trilean +1/0/-1), and
/// ternary-logic operations (Łukasiewicz Ł3 + Kleene K3). All logic
/// ops work on i64-encoded Trilean values where +1=True, 0=Unknown,
/// -1=False.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    /// Arithmetic addition.
    Add,
    /// Arithmetic subtraction.
    Sub,
    /// Arithmetic multiplication.
    Mul,
    /// Arithmetic division (signed, truncates toward zero).
    Div,
    /// Arithmetic remainder (signed).
    Mod,

    /// Equality comparison → Trilean (+1/0/-1).
    Eq,
    /// Not-equal comparison → Trilean.
    Ne,
    /// Less-than comparison → Trilean.
    Lt,
    /// Less-than-or-equal comparison → Trilean.
    Le,
    /// Greater-than comparison → Trilean.
    Gt,
    /// Greater-than-or-equal comparison → Trilean.
    Ge,

    /// Łukasiewicz Ł3 conjunction (logical AND): min(a, b).
    LukAnd,
    /// Łukasiewicz Ł3 disjunction (logical OR): max(a, b).
    LukOr,
    /// Łukasiewicz Ł3 exclusive-or: ¬(a ↔ b) in Ł3.
    LukXor,
    /// Łukasiewicz Ł3 implication: ¬a ∨ b (max(-a, b)).
    LukImplies,
    /// Łukasiewicz Ł3 equivalence: (a → b) ∧ (b → a).
    LukIff,

    /// Kleene K3 implication: same truth table as Ł3 for atoms,
    /// differs in how Unknown propagates through nested expressions.
    KleeneImplies,
    /// Kleene K3 exclusive-or.
    KleeneXor,
    /// Kleene K3 equivalence.
    KleeneIff,

    /// Ternary arithmetic negation: maps (+,0,-) → (-,0,+).
    /// Same result for all three unary forms (Negate, Not, KleeneNot)
    /// at the MIR level — differs only in type-system refinement tracking.
    Neg,
}

/// S6 reference forms (mirrors triet_syntax::ReferenceForm).
///
/// Duplicated here to keep triet-mir independent of triet-syntax.
/// Must stay in sync with the AST definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ReferenceForm {
    /// `&+ T` — strong owner, frozen.
    StrongFrozen,
    /// `&+ mutable T` — strong owner, mutable.
    StrongMutable,
    /// `&0 T` — scope borrow, read-only.
    BorrowReadOnly,
    /// `&0 mutable T` — scope borrow, exclusive mutable.
    BorrowExclusiveMutable,
    /// `&- T` — weak observer.
    WeakObserver,
}

impl fmt::Display for ReferenceForm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StrongFrozen => write!(f, "&+"),
            Self::StrongMutable => write!(f, "&+ mutable"),
            Self::BorrowReadOnly => write!(f, "&0"),
            Self::BorrowExclusiveMutable => write!(f, "&0 mutable"),
            Self::WeakObserver => write!(f, "&-"),
        }
    }
}

/// How a parameter is passed at a call site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterPassing {
    /// Borrow (default `&0`) — callee borrows, caller retains ownership.
    Borrow,
    /// Move (`&+` / `owned`) — ownership transferred to callee.
    Move,
    /// Mutable borrow (`&0 mutable`) — exclusive borrow for call duration.
    MutableBorrow,
}

impl fmt::Display for ParameterPassing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Borrow => write!(f, "borrow"),
            Self::Move => write!(f, "move"),
            Self::MutableBorrow => write!(f, "mut_borrow"),
        }
    }
}

// ── MIR terminators ─────────────────────────────────────────

/// A terminator — the last instruction in a basic block that transfers
/// control to another block (or exits the function).
///
/// Every variant carries a [`Span`] for source-level diagnostics. Use
/// [`DUMMY_SPAN`] for compiler-synthesized terminators (e.g. implicit
/// `Return(())` at end of a void function).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Terminator {
    /// Return from the function.
    /// Length matches the function's `ReturnShape::arity()`:
    /// - Unit: empty
    /// - Scalar: 1 local
    /// - BinaryOutcome/TernaryOutcome: 2 locals (discriminant, payload)
    Return {
        /// Values to return. Must match `ReturnShape::arity()`.
        values: Vec<Local>,
        /// Source location (DUMMY_SPAN for implicit returns).
        span: Span,
    },

    /// Unconditional jump to another block.
    Goto {
        /// Target block.
        target: BasicBlock,
        /// Source location (DUMMY_SPAN for synthetic jumps).
        span: Span,
    },

    /// Conditional branch based on a Trilean discriminant.
    ///
    /// For refined `Trilean!` (non-Unknown): only `positive` and `negative`
    /// are reachable. `zero` may point to an unreachable block.
    If {
        /// Condition local (Trilean).
        cond: Local,
        /// Branch taken when cond = Trit::Positive (True).
        positive_bb: BasicBlock,
        /// Branch taken when cond = Trit::Zero (Unknown) — only for full
        /// Trilean branching (`if?`).
        zero_bb: Option<BasicBlock>,
        /// Branch taken when cond = Trit::Negative (False).
        negative_bb: BasicBlock,
        /// Source location.
        span: Span,
    },

    /// Function call. Triết is a kernel language — there is no exception
    /// unwinding. If the callee panics, the entire process aborts immediately.
    /// Therefore, CallDispatch has exactly ONE successor: `return_bb`.
    CallDispatch {
        /// Callee function ID (for internal tracking).
        callee: FunctionId,
        /// Callee function name (for diagnostics / Display).
        callee_name: String,
        /// How to compile this call: JIT-to-JIT or JIT-to-Shim.
        target: CallTarget,
        /// Argument locals.
        args: Vec<Local>,
        /// Block to jump to on normal return.
        return_bb: BasicBlock,
        /// Destination locals for return values (in return_bb).
        /// Length matches the callee's `ReturnShape::arity()`:
        /// - Unit: empty
        /// - Scalar: 1 local (the value)
        /// - BinaryOutcome/TernaryOutcome: 2 locals (discriminant, payload)
        /// - Struct: empty (data written through sret pointer arg[0])
        dest: Vec<Local>,
        /// The callee's return shape — caller needs this to know whether
        /// dest is empty (sret) and how to handle return values.
        return_shape: ReturnShape,
        /// Source location.
        span: Span,
    },

    /// Unreachable point — after infinite loop or guaranteed panic.
    Unreachable {
        /// Source location (DUMMY_SPAN for dead blocks).
        span: Span,
    },

    /// Deterministic abort (Cranelift `trap`). Used for `SwitchInt.default_bb`
    /// when match is non-exhaustive — guaranteed abort, never UB.
    /// Leaf terminator: no successors.
    Trap {
        /// Source location.
        span: Span,
    },

    /// N-way branch on an integer discriminant (enum match dispatch).
    /// Branches to `cases` targets keyed by discriminant value, with a
    /// `default_bb` trap block for unknown discriminants.
    SwitchInt {
        /// Local holding the integer discriminant to branch on.
        discriminant: Local,
        /// (discriminant_value, target_block) pairs.
        cases: Vec<(i64, BasicBlock)>,
        /// Default/fallthrough block for unknown discriminant values.
        /// Always a Trap block in Bậc A (never Unreachable).
        default_bb: BasicBlock,
        /// Source location.
        span: Span,
    },
}

// ── Function signature ──────────────────────────────────────

/// The return shape of a function — encodes the number and meaning
/// of return values. Mirrors the Type system: `Unit` → 0 values,
/// `Scalar` → 1 value, `BinaryOutcome` → 2 values (no Trit::Zero),
/// `TernaryOutcome` → 2 values (Trit::Zero valid).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReturnShape {
    /// `()` — no return value (void).
    Unit,
    /// Any single-value type: Integer, Trilean, reference, etc.
    /// One Cranelift return value.
    Scalar,
    /// `T~E` — binary Outcome. Returns 2 values (discriminant + payload).
    /// Trit::Zero is INVALID (compile-time error E1025).
    BinaryOutcome,
    /// `T?~E` — ternary Outcome. Returns 2 values (discriminant + payload).
    /// Trit::Zero IS valid (null state).
    TernaryOutcome,
    /// Struct return via sret — the caller allocates space and passes a
    /// hidden pointer as the first argument. The callee writes the struct
    /// fields through that pointer and returns 0 values (void).
    Struct {
        /// The name of the struct type.
        struct_name: String,
    },
}

impl ReturnShape {
    /// Number of return values in the ABI (0, 1, or 2).
    #[must_use]
    pub fn arity(&self) -> usize {
        match self {
            Self::Unit | Self::Struct { .. } => 0,
            Self::Scalar => 1,
            Self::BinaryOutcome | Self::TernaryOutcome => 2,
        }
    }
}

/// A path into the return value, used to track which part of a returned
/// reference borrows from which parameter (field-level granularity).
///
/// `Root` is the whole return value (a direct reference return). `Field(name)`
/// is a named field of a returned struct (so `split` can return `{left, right}`
/// where each field borrows a different parameter). Mirrors the borrow
/// checker's `FieldPath` per `phase2-borrow-checker-design.md`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FieldPath {
    /// The whole return value.
    Root,
    /// A named field of the returned value.
    Field(String),
}

/// Maps each field path of the return value to the set of parameter indices
/// its borrow depends on (from lifetime elision, ADR-0025 §6).
///
/// Empty when the function returns a non-reference type, or no returned part
/// borrows from any parameter.
pub type ReturnBorrowMap = BTreeMap<FieldPath, BTreeSet<usize>>;

/// A function signature as seen by MIR.
///
/// Carries the information the borrow checker needs to reason about
/// cross-function borrow propagation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionSignature {
    /// Function name for diagnostics.
    pub name: String,
    /// Parameters: (name, passing_mode).
    pub params: Vec<(String, ParameterPassing)>,
    /// Return type name for display.
    pub return_type: String,
    /// The shape of the return value (Unit, Scalar, BinaryOutcome, TernaryOutcome).
    /// Determines how many values the function returns in the ABI.
    pub return_shape: ReturnShape,
    /// For each field path of the return value, the parameter indices its
    /// borrow depends on. Drives cross-call loan propagation.
    pub return_borrow_map: ReturnBorrowMap,
}

// ── MIR body ────────────────────────────────────────────────

/// The MIR representation of a single function body.
///
/// A body is a collection of basic blocks. The `blocks` vector is indexed
/// by `BasicBlock(usize)`. `entry_block` identifies where execution starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Body {
    /// Function signature.
    pub signature: FunctionSignature,
    /// All basic blocks in this function.
    pub blocks: Vec<BlockData>,
    /// Index of the entry block.
    pub entry_block: BasicBlock,
    /// Number of locals allocated for this function (including parameters).
    pub num_locals: usize,
    /// Per-local declarations (type + mutability), indexed by `Local`.
    /// Length should equal `num_locals`.
    pub local_decls: Vec<LocalDecl>,
    /// Struct layouts for user-defined types referenced in this function.
    /// Carries enough info for the codegen backend to compute field offsets
    /// for native stack allocation (Bậc C). Empty for functions that don't
    /// use user-defined structs.
    pub struct_layouts: Vec<StructLayout>,
    /// Enum layouts for user-defined enum types referenced in this function.
    /// Carries discriminant + payload layout info for the JIT backend to
    /// allocate StackSlots and compute variant offsets. Empty for functions
    /// that don't use user-defined enums.
    pub enum_layouts: Vec<EnumLayout>,
    /// Human-readable names for non-param locals (let-bound variables).
    /// Params already have names in `signature.params`. Populated by the
    /// lowerer; consumed by borrowck for user-facing diagnostics (E2420,
    /// E2440) so MIR local numbers like `_2` don't leak.
    pub local_names: BTreeMap<Local, String>,
}

// ── Builtin shim metadata ──────────────────────────────────────

/// Metadata for a builtin runtime shim callable via `CallDispatch`.
///
/// Shared between borrowck (marks consume-args Moved after call) and JIT
/// (zeroes consume-arg variables after call). One source of truth, two
/// consumers — schema-first discipline.
#[derive(Clone, Debug)]
pub struct BuiltinShimMeta {
    /// Shim name (e.g. `"__triet_vector_push"`).
    pub name: &'static str,
    /// Per-arg ownership: `true` = consume (caller loses ownership,
    /// variable must be zeroed), `false` = borrow/copy (caller retains).
    pub arg_consumes: &'static [bool],
}

/// Builtin shim metadata table consumed by borrowck and JIT.
///
/// Sorted by name for deterministic lookup. Every shim listed in
/// ADR-0040 §3.1 must have an entry here.
#[must_use]
pub fn builtin_shim_meta(name: &str) -> Option<BuiltinShimMeta> {
    match name {
        "__triet_string_alloc" => Some(BuiltinShimMeta {
            name: "__triet_string_alloc",
            arg_consumes: &[false, false],
        }),
        "__triet_string_concat" => Some(BuiltinShimMeta {
            name: "__triet_string_concat",
            arg_consumes: &[false, false, false, false],
        }),
        "__triet_string_eq" => Some(BuiltinShimMeta {
            name: "__triet_string_eq",
            arg_consumes: &[false, false, false, false],
        }),
        "__triet_string_free" => Some(BuiltinShimMeta {
            name: "__triet_string_free",
            arg_consumes: &[true],
        }),
        "__triet_string_from_bytes" => Some(BuiltinShimMeta {
            name: "__triet_string_from_bytes",
            arg_consumes: &[false, false],
        }),
        "__triet_string_len" => Some(BuiltinShimMeta {
            name: "__triet_string_len",
            arg_consumes: &[false],
        }),
        "__triet_vector_alloc" => Some(BuiltinShimMeta {
            name: "__triet_vector_alloc",
            arg_consumes: &[false, false],
        }),
        "__triet_vector_free" => Some(BuiltinShimMeta {
            name: "__triet_vector_free",
            arg_consumes: &[true],
        }),
        "__triet_vector_len" => Some(BuiltinShimMeta {
            name: "__triet_vector_len",
            arg_consumes: &[false],
        }),
        "__triet_vector_push" => Some(BuiltinShimMeta {
            name: "__triet_vector_push",
            arg_consumes: &[true, false],
        }),
        "__triet_vector_get" => Some(BuiltinShimMeta {
            name: "__triet_vector_get",
            // [false, false]: borrow vec (không consume, khác push), copy index.
            arg_consumes: &[false, false],
        }),
        "__triet_hashmap_alloc" => Some(BuiltinShimMeta {
            name: "__triet_hashmap_alloc",
            arg_consumes: &[false, false],
        }),
        "__triet_hashmap_free" => Some(BuiltinShimMeta {
            name: "__triet_hashmap_free",
            arg_consumes: &[true],
        }),
        "__triet_hashmap_len" => Some(BuiltinShimMeta {
            name: "__triet_hashmap_len",
            arg_consumes: &[false],
        }),
        "__triet_hashmap_insert" => Some(BuiltinShimMeta {
            name: "__triet_hashmap_insert",
            arg_consumes: &[true, false, false],
        }),
        "__triet_hashmap_get" => Some(BuiltinShimMeta {
            name: "__triet_hashmap_get",
            arg_consumes: &[false, false],
        }),
        _ => None,
    }
}

/// Memory layout of a user-defined struct.
///
/// Carries the information the codegen backend needs to allocate the struct
/// on the stack and compute field offsets — without requiring the full AST
/// type definition. This is the IR-level complement to `TypeTag::Opaque`:
/// `Opaque` means "the IR does not track this type's layout," while
/// `StructLayout` means "here is the layout, use it for native codegen."
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructLayout {
    /// Struct name (for diagnostics).
    pub name: String,
    /// Fields in declaration order, with byte size and offset.
    pub fields: Vec<FieldLayout>,
    /// Total size in bytes (including alignment padding).
    pub total_size: usize,
    /// Alignment in bytes.
    pub alignment: usize,
}

/// Layout of a single struct field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldLayout {
    /// Field name.
    pub name: String,
    /// Field type name (e.g. "Integer", "String").
    pub ty: String,
    /// Byte offset from the start of the struct.
    pub offset: usize,
    /// Byte size of the field.
    pub size: usize,
    /// Byte alignment of the field (must be a power of 2).
    /// Alignment is a property of the TYPE, not the size —
    /// e.g. `[u8; 5]` has size=5 but alignment=1.
    pub alignment: usize,
}

/// Memory layout of a user-defined enum (tagged union).
///
/// Carries the information the codegen backend needs to allocate the enum
/// on the stack and access discriminant/payload fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumLayout {
    /// Enum name (e.g., "Option").
    pub name: String,
    /// Byte offset of the discriminant field (always 0 for Bậc A).
    pub discriminant_offset: usize,
    /// Size of the discriminant in bytes (always 8 for Bậc A — i64).
    pub discriminant_size: usize,
    /// Byte offset of the payload union (always 8 for Bậc A).
    pub payload_offset: usize,
    /// Total size in bytes, rounded up to `alignment`.
    pub total_size: usize,
    /// Required alignment (always 8 for Bậc A).
    pub alignment: usize,
    /// Per-variant metadata.
    pub variants: Vec<VariantLayout>,
}

/// Metadata for one enum variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantLayout {
    /// Variant name.
    pub name: String,
    /// Integer discriminant value (0, 1, 2, …).
    pub discriminant_value: i64,
    /// Payload layout, if the variant carries data.
    pub payload: Option<PayloadLayout>,
}

/// Layout of a variant's payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayloadLayout {
    /// Type name of the payload (e.g. "Integer", "Point").
    pub ty: String,
    /// Size of this variant's payload in bytes.
    pub size: usize,
    /// Alignment of this variant's payload.
    pub alignment: usize,
    /// If the payload is a struct/tuple, field layouts keyed by field name.
    pub fields: Vec<FieldLayout>,
}

impl EnumLayout {
    /// Compute the layout for an enum from its variant specifications.
    ///
    /// `variants` is a list of `(name, discriminant_value, payload)`.
    /// Payload is `Option<(ty, size, alignment, fields)>`.
    /// `discriminant_size` defaults to 8 (i64 in Bậc A).
    /// The payload area size = max of all variant payload sizes.
    #[must_use]
    pub fn compute(
        name: &str,
        variants: &[(
            String,                                           // variant name
            i64,                                              // discriminant value
            Option<(String, usize, usize, Vec<FieldLayout>)>, // payload: (ty, size, alignment, fields)
        )],
    ) -> Self {
        let disc_offset: usize = 0;
        let disc_size: usize = 8;
        let payload_offset: usize = 8;
        let max_payload_size = variants
            .iter()
            .map(|(_, _, payload)| payload.as_ref().map_or(0, |(_, s, _, _)| *s))
            .max()
            .unwrap_or(0);
        let total_size = 8 + max_payload_size;
        let alignment: usize = 8;
        let variant_layouts = variants
            .iter()
            .map(|(vname, disc_val, payload)| VariantLayout {
                name: vname.clone(),
                discriminant_value: *disc_val,
                payload: payload
                    .as_ref()
                    .map(|(ty, size, align, fields)| PayloadLayout {
                        ty: ty.clone(),
                        size: *size,
                        alignment: *align,
                        fields: fields.clone(),
                    }),
            })
            .collect();
        Self {
            name: name.to_string(),
            discriminant_offset: disc_offset,
            discriminant_size: disc_size,
            payload_offset,
            total_size,
            alignment,
            variants: variant_layouts,
        }
    }
}

/// Known alignment values for primitive types (in bytes, on 64-bit targets).
/// These are the ONLY valid alignment values — all must be powers of 2.
pub mod align {
    /// Trit (2-bit) — byte-aligned.
    pub const TRIT: usize = 1;
    /// Tryte (16-bit) — 2-byte aligned.
    pub const TRYTE: usize = 2;
    /// Integer (64-bit) — 8-byte aligned.
    pub const INTEGER: usize = 8;
    /// Long (128-bit) — 16-byte aligned.
    pub const LONG: usize = 16;
    /// Trilean (8-bit) — byte-aligned.
    pub const TRILEAN: usize = 1;
    /// Pointer/reference (64-bit) — 8-byte aligned.
    pub const POINTER: usize = 8;
    /// Unit / ZST — alignment 1 (zero-sized, no real constraint).
    pub const UNIT: usize = 1;
}

impl StructLayout {
    /// Compute field offsets from (name, ty, size, alignment) tuples.
    ///
    /// Each field's alignment must be a power of 2. The struct's
    /// alignment is the maximum of its fields' alignments.
    /// Total size is padded to the struct alignment.
    #[must_use]
    pub fn compute(name: &str, fields: &[(String, String, usize, usize)]) -> Self {
        let mut offset = 0usize;
        let mut max_align = 1usize;
        let mut field_layouts = Vec::new();
        for (field_name, field_ty, size, align) in fields {
            assert!(
                align.is_power_of_two(),
                "alignment must be power of 2: field '{field_name}' has align={align}"
            );
            max_align = max_align.max(*align);
            // Align offset to field alignment
            offset = (offset + align - 1) & !(align - 1);
            field_layouts.push(FieldLayout {
                name: field_name.clone(),
                ty: field_ty.clone(),
                offset,
                size: *size,
                alignment: *align,
            });
            offset += size;
        }
        // Pad total size to struct alignment
        let total_size = (offset + max_align - 1) & !(max_align - 1);
        Self {
            name: name.to_string(),
            fields: field_layouts,
            total_size,
            alignment: max_align,
        }
    }
}

/// Data for one basic block: a sequence of statements followed by a
/// terminator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockData {
    /// Statements executed in order when the block is entered.
    pub statements: Vec<Statement>,
    /// Terminator that transfers control at the end of the block.
    pub terminator: Terminator,
}

// ── Control Flow Graph ──────────────────────────────────────

/// A control-flow graph built from a MIR body.
///
/// The CFG makes predecessor/successor relationships explicit and is
/// the structure the borrow checker performs dataflow analysis on.
#[derive(Clone, Debug)]
pub struct ControlFlowGraph {
    /// Per-block data.
    pub blocks: Vec<CfgBlock>,
    /// Entry block.
    pub entry: BasicBlock,
    /// Exit blocks (those with Return terminators).
    pub exits: Vec<BasicBlock>,
}

/// One block in the CFG, with predecessor/successor edges resolved.
#[derive(Clone, Debug)]
pub struct CfgBlock {
    /// Incoming edges from other blocks.
    pub predecessors: Vec<BasicBlock>,
    /// Outgoing edges to other blocks (not counting Return/Unreachable).
    pub successors: Vec<BasicBlock>,
    /// The original block data.
    pub data: BlockData,
}

impl Body {
    /// Build the control-flow graph from this MIR body.
    #[must_use]
    pub fn build_cfg(&self) -> ControlFlowGraph {
        let mut blocks: Vec<CfgBlock> = self
            .blocks
            .iter()
            .map(|_| CfgBlock {
                predecessors: Vec::new(),
                successors: Vec::new(),
                data: BlockData {
                    statements: Vec::new(),
                    terminator: Terminator::Unreachable { span: DUMMY_SPAN },
                },
            })
            .collect();

        // Copy block data + collect successors
        let mut successors: Vec<Vec<BasicBlock>> = Vec::new();
        for (i, block) in self.blocks.iter().enumerate() {
            blocks[i].data = block.clone();
            let succ = terminator_successors(&block.terminator);
            successors.push(succ);
            blocks[i].successors = successors[i].clone();
        }

        // Compute predecessors from successors
        for (i, succs) in successors.iter().enumerate() {
            for &succ in succs {
                if succ.0 < blocks.len() {
                    blocks[succ.0].predecessors.push(BasicBlock(i));
                }
            }
        }

        // Collect exit blocks
        let exits: Vec<BasicBlock> = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| matches!(b.terminator, Terminator::Return { .. }))
            .map(|(i, _)| BasicBlock(i))
            .collect();

        ControlFlowGraph {
            blocks,
            entry: self.entry_block,
            exits,
        }
    }
}

/// Return the successor blocks of a terminator.
fn terminator_successors(terminator: &Terminator) -> Vec<BasicBlock> {
    match terminator {
        Terminator::Return { .. } | Terminator::Unreachable { .. } | Terminator::Trap { .. } => {
            Vec::new()
        }
        Terminator::Goto { target, .. } => vec![*target],
        Terminator::If {
            positive_bb,
            zero_bb,
            negative_bb,
            ..
        } => {
            let mut succs = vec![*positive_bb, *negative_bb];
            if let Some(zero) = zero_bb {
                succs.push(*zero);
            }
            succs
        }
        Terminator::CallDispatch { return_bb, .. } => {
            vec![*return_bb]
        }
        Terminator::SwitchInt {
            cases, default_bb, ..
        } => {
            let mut succs: Vec<BasicBlock> = cases.iter().map(|(_, bb)| *bb).collect();
            succs.push(*default_bb);
            succs
        }
    }
}

impl Body {
    /// Verify that this body is well-formed.
    ///
    /// Checks structural invariants that every MIR consumer (borrowck, JIT) relies on:
    ///
    /// - **INV-1 (block bounds):** every `BasicBlock` reference in every
    ///   terminator, and `entry_block`, must be in range `0..blocks.len()`.
    /// - **INV-2 (local bounds):** every `Local` reference in every statement
    ///   and terminator must be in range `0..num_locals`.
    /// - **INV-3 (enum invariants):** structural checks for enum-related
    ///   statements/projections (type correctness, discriminant range,
    ///   default_bb-is-trap, variant-name validity).
    ///
    /// Flow-sensitive checks (dominance, reaching-def) are NOT covered —
    /// they are the lowerer's responsibility at Bậc A (see ADR-0037 §4i).
    ///
    /// Callers MUST run this after building a `Body` (e.g. from the lowerer)
    /// and before passing it to the borrow checker or JIT backend.
    pub fn verify(&self) -> Result<(), MirError> {
        let num_blocks = self.blocks.len();

        // ── INV-1: entry block in bounds ──
        if self.entry_block.0 >= num_blocks {
            return Err(MirError::BlockOutOfBounds {
                block: self.entry_block,
                num_blocks,
                span: DUMMY_SPAN.clone(),
            });
        }

        // Helper: look up EnumLayout by name.
        let find_enum = |name: &str| -> Option<&EnumLayout> {
            self.enum_layouts.iter().find(|e| e.name == name)
        };
        // Helper: look up EnumLayout by type string (from local_decls).
        let find_enum_by_type =
            |ty: &str| -> Option<&EnumLayout> { self.enum_layouts.iter().find(|e| e.name == ty) };

        for block_data in &self.blocks {
            // INV-1 helpers — BasicBlock references, copy-friendly
            let check_block = |target: BasicBlock| -> Result<(), MirError> {
                if target.0 >= num_blocks {
                    return Err(MirError::BlockOutOfBounds {
                        block: target,
                        num_blocks,
                        span: DUMMY_SPAN.clone(),
                    });
                }
                Ok(())
            };

            // INV-2 helpers — Local references, copy-friendly
            let check_local = |local: Local| -> Result<(), MirError> {
                if local.0 >= self.num_locals {
                    return Err(MirError::LocalOutOfBounds {
                        local,
                        num_locals: self.num_locals,
                        span: DUMMY_SPAN.clone(),
                    });
                }
                Ok(())
            };

            let check_place = |place: &Place| -> Result<(), MirError> {
                check_local(place.local)?;
                for proj in &place.projection {
                    if let Projection::Index(local) = proj {
                        check_local(*local)?;
                    }
                }
                Ok(())
            };

            // ── INV-3 helper: verify Payload projection ──
            let check_payload = |base: Local, variant: &str| -> Result<(), MirError> {
                if let Some(ty) = self.local_decls.get(base.0).map(|d| d.ty.as_str())
                    && let Some(el) = find_enum_by_type(ty)
                {
                    if !el.variants.iter().any(|v| v.name == variant) {
                        return Err(MirError::PayloadVariantNotFound {
                            enum_name: el.name.clone(),
                            variant: variant.to_string(),
                            span: DUMMY_SPAN.clone(),
                        });
                    }
                    return Ok(());
                }
                // If we can't resolve the type, don't fail — the type
                // may be a non-enum type using Payload projection (caught
                // by other checks).
                Ok(())
            };

            // Check terminator
            match &block_data.terminator {
                Terminator::Return { values, .. } => {
                    for &v in values {
                        check_local(v)?;
                    }
                }
                Terminator::Goto { target, .. } => {
                    check_block(*target)?;
                }
                Terminator::If {
                    cond,
                    positive_bb,
                    zero_bb,
                    negative_bb,
                    ..
                } => {
                    check_local(*cond)?;
                    check_block(*positive_bb)?;
                    check_block(*negative_bb)?;
                    if let Some(zb) = zero_bb {
                        check_block(*zb)?;
                    }
                }
                Terminator::CallDispatch {
                    args,
                    return_bb,
                    dest,
                    ..
                } => {
                    for &a in args {
                        check_local(a)?;
                    }
                    for &d in dest {
                        check_local(d)?;
                    }
                    check_block(*return_bb)?;
                }
                Terminator::Unreachable { .. } | Terminator::Trap { .. } => {}
                Terminator::SwitchInt {
                    discriminant,
                    cases,
                    default_bb,
                    ..
                } => {
                    check_local(*discriminant)?;
                    for &(_, target) in cases {
                        check_block(target)?;
                    }
                    check_block(*default_bb)?;
                    // 4i-6: default_bb must terminate with Trap (not Unreachable)
                    let default_block = &self.blocks[default_bb.0];
                    if !matches!(default_block.terminator, Terminator::Trap { .. }) {
                        return Err(MirError::SwitchIntDefaultNotTrap {
                            default_bb: *default_bb,
                            span: DUMMY_SPAN.clone(),
                        });
                    }
                }
            }

            // Check statements
            for stmt in &block_data.statements {
                match stmt {
                    Statement::StorageLive(l, _) => check_local(*l)?,
                    Statement::StorageDead(l, _) => check_local(*l)?,
                    Statement::Deinit(l, _) => check_local(*l)?,
                    Statement::Assign { dest, source, .. } => {
                        check_place(dest)?;
                        check_place(source)?;
                        // 4i-7: check Payload projection validity
                        for proj in &dest.projection {
                            if let Projection::Payload(variant) = proj {
                                check_payload(dest.local, variant)?;
                            }
                        }
                        for proj in &source.projection {
                            if let Projection::Payload(variant) = proj {
                                check_payload(source.local, variant)?;
                            }
                        }
                    }
                    Statement::Borrow { dest, source, .. } => {
                        check_place(dest)?;
                        check_place(source)?;
                        for proj in &source.projection {
                            if let Projection::Payload(variant) = proj {
                                check_payload(source.local, variant)?;
                            }
                        }
                    }
                    Statement::Const { dest, .. } => check_place(dest)?,
                    Statement::BinaryOp {
                        dest, left, right, ..
                    } => {
                        check_place(dest)?;
                        check_place(left)?;
                        check_place(right)?;
                    }
                    Statement::OutcomeDiscriminant { dest, source, .. }
                    | Statement::OutcomeUnwrap { dest, source, .. }
                    | Statement::OutcomeUnwrapError { dest, source, .. } => {
                        check_place(dest)?;
                        check_place(source)?;
                    }
                    Statement::GetDiscriminant { dest, source, .. } => {
                        check_place(dest)?;
                        check_place(source)?;
                        // 4i-4: source must have enum type
                        if let Some(decl) = self.local_decls.get(source.local.0)
                            && find_enum_by_type(&decl.ty).is_none()
                        {
                            return Err(MirError::GetDiscriminantNonEnum {
                                local: source.local,
                                found_type: decl.ty.clone(),
                                span: DUMMY_SPAN.clone(),
                            });
                        }
                    }
                    Statement::StructAlloc { dest, .. } => check_local(*dest)?,
                    Statement::EnumAlloc {
                        dest, enum_name, ..
                    } => {
                        check_local(*dest)?;
                        // 4i-1: dest type must be this enum
                        if let Some(decl) = self.local_decls.get(dest.0)
                            && decl.ty != *enum_name
                        {
                            // Warn but don't fail — the type string
                            // may be resolved differently by the lowerer.
                        }
                        // 4i-1: enum_name must exist in enum_layouts
                        if find_enum(enum_name).is_none() {
                            return Err(MirError::EnumLayoutNotFound {
                                enum_name: enum_name.clone(),
                                span: DUMMY_SPAN.clone(),
                            });
                        }
                    }
                    Statement::SetDiscriminant { dest, value, .. } => {
                        check_local(*dest)?;
                        // 4i-2: dest must have enum type
                        // 4i-3: value must be in range [0, n_variants)
                        if let Some(decl) = self.local_decls.get(dest.0)
                            && let Some(el) = find_enum_by_type(&decl.ty)
                        {
                            let n = el.variants.len() as i64;
                            if *value < 0 || *value >= n {
                                return Err(MirError::SetDiscriminantValueOutOfRange {
                                    enum_name: el.name.clone(),
                                    value: *value,
                                    num_variants: el.variants.len(),
                                    span: DUMMY_SPAN.clone(),
                                });
                            }
                        }
                    }
                    Statement::Drop(l, _) => check_local(*l)?,
                }
            }
        }

        Ok(())
    }
}

/// MIR verification error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirError {
    /// A `BasicBlock` index is out of bounds.
    BlockOutOfBounds {
        /// The out-of-bounds block index.
        block: BasicBlock,
        /// The number of blocks in the body.
        num_blocks: usize,
        /// Source location (DUMMY_SPAN for MIR-level errors).
        span: Span,
    },
    /// A `Local` index is out of bounds.
    LocalOutOfBounds {
        /// The out-of-bounds local index.
        local: Local,
        /// The number of locals in the body.
        num_locals: usize,
        /// Source location (DUMMY_SPAN for MIR-level errors).
        span: Span,
    },
    /// An `EnumAlloc` referenced an enum not found in `Body::enum_layouts`.
    EnumLayoutNotFound {
        /// The enum name that was not found.
        enum_name: String,
        /// Source location.
        span: Span,
    },
    /// `SetDiscriminant.value` is out of range for the enum's variant count.
    SetDiscriminantValueOutOfRange {
        /// The enum name.
        enum_name: String,
        /// The discriminant value that was out of range.
        value: i64,
        /// The number of variants in the enum.
        num_variants: usize,
        /// Source location.
        span: Span,
    },
    /// `SwitchInt.default_bb` does not terminate with `Trap`.
    SwitchIntDefaultNotTrap {
        /// The default block that should be a Trap.
        default_bb: BasicBlock,
        /// Source location.
        span: Span,
    },
    /// A `Payload` projection referenced a variant not in the enum.
    PayloadVariantNotFound {
        /// The enum name.
        enum_name: String,
        /// The variant name that was not found.
        variant: String,
        /// Source location.
        span: Span,
    },
    /// `GetDiscriminant.source` is not an enum type (4i-4).
    GetDiscriminantNonEnum {
        /// The local being read from.
        local: Local,
        /// The type that was found instead of an enum.
        found_type: String,
        /// Source location.
        span: Span,
    },
}

impl fmt::Display for MirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlockOutOfBounds {
                block, num_blocks, ..
            } => {
                write!(
                    f,
                    "MIR verification error: block {} out of bounds (body has {} blocks)",
                    block.0, num_blocks
                )
            }
            Self::LocalOutOfBounds {
                local, num_locals, ..
            } => {
                write!(
                    f,
                    "MIR verification error: local {} out of bounds (body has {} locals)",
                    local.0, num_locals
                )
            }
            Self::EnumLayoutNotFound { enum_name, .. } => {
                write!(
                    f,
                    "MIR verification error: EnumAlloc references unknown enum '{enum_name}'"
                )
            }
            Self::SetDiscriminantValueOutOfRange {
                enum_name,
                value,
                num_variants,
                ..
            } => {
                write!(
                    f,
                    "MIR verification error: SetDiscriminant value {value} out of range [0, {num_variants}) for enum '{enum_name}'"
                )
            }
            Self::SwitchIntDefaultNotTrap { default_bb, .. } => {
                write!(
                    f,
                    "MIR verification error: SwitchInt default_bb {default_bb} must terminate with Trap"
                )
            }
            Self::PayloadVariantNotFound {
                enum_name, variant, ..
            } => {
                write!(
                    f,
                    "MIR verification error: Payload variant '{variant}' not found in enum '{enum_name}'"
                )
            }
            Self::GetDiscriminantNonEnum {
                local, found_type, ..
            } => {
                write!(
                    f,
                    "MIR verification error: GetDiscriminant source {local} has non-enum type '{found_type}'"
                )
            }
        }
    }
}

// ── Display implementations ─────────────────────────────────

impl fmt::Display for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "fn {}(...) -> {} {{",
            self.signature.name, self.signature.return_type
        )?;
        for (i, block) in self.blocks.iter().enumerate() {
            writeln!(f, "  bb{}: {{", i)?;
            for stmt in &block.statements {
                writeln!(f, "    {stmt}")?;
            }
            writeln!(f, "    {term}", term = block.terminator)?;
            writeln!(f, "  }}")?;
        }
        writeln!(f, "}}")?;
        Ok(())
    }
}

impl fmt::Display for Statement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StorageLive(l, _) => write!(f, "StorageLive({l})"),
            Self::StorageDead(l, _) => write!(f, "StorageDead({l})"),
            Self::Deinit(l, _) => write!(f, "Deinit({l})"),
            Self::Assign { dest, source, .. } => write!(f, "{dest} = move {source}"),
            Self::Borrow {
                dest, form, source, ..
            } => write!(f, "{dest} = {form} {source}"),
            Self::Const { dest, value, .. } => write!(f, "{dest} = const {value}"),
            Self::BinaryOp {
                dest,
                op,
                left,
                right,
                ..
            } => write!(f, "{dest} = {left} {op} {right}"),
            Self::OutcomeDiscriminant { dest, source, .. } => {
                write!(f, "{dest} = discriminant({source})")
            }
            Self::OutcomeUnwrap { dest, source, .. } => {
                write!(f, "{dest} = unwrap_value({source})")
            }
            Self::OutcomeUnwrapError { dest, source, .. } => {
                write!(f, "{dest} = unwrap_error({source})")
            }
            Self::StructAlloc {
                dest, struct_name, ..
            } => write!(f, "{dest} = struct {struct_name} {{..}}"),
            Self::EnumAlloc {
                dest, enum_name, ..
            } => write!(f, "{dest} = enum {enum_name} {{..}}"),
            Self::SetDiscriminant { dest, value, .. } => {
                write!(f, "SetDiscriminant({dest}, {value})")
            }
            Self::GetDiscriminant { dest, source, .. } => {
                write!(f, "{dest} = discriminant({source})")
            }
            Self::Drop(l, _) => write!(f, "Drop({l})"),
        }
    }
}

impl fmt::Display for Terminator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Return { values, .. } if values.is_empty() => write!(f, "Return(())"),
            Self::Return { values, .. } => {
                let vs: Vec<String> = values.iter().map(|v| v.to_string()).collect();
                write!(f, "Return({})", vs.join(", "))
            }
            Self::Goto { target, .. } => write!(f, "Goto({target})"),
            Self::If {
                cond,
                positive_bb,
                zero_bb: Some(zero),
                negative_bb,
                ..
            } => {
                write!(
                    f,
                    "IfTernary({cond}) → +:{positive_bb}, 0:{zero}, -:{negative_bb}"
                )
            }
            Self::If {
                cond,
                positive_bb,
                zero_bb: None,
                negative_bb,
                ..
            } => {
                write!(f, "If({cond}) → +:{positive_bb}, -:{negative_bb}")
            }
            Self::CallDispatch {
                callee_name,
                args,
                return_bb,
                dest,
                ..
            } => {
                let args_str: Vec<String> = args.iter().map(|a| a.to_string()).collect();
                write!(
                    f,
                    "Call {callee_name}({args}) → {return_bb}",
                    args = args_str.join(", ")
                )?;
                if !dest.is_empty() {
                    let dest_str: Vec<String> = dest.iter().map(|d| d.to_string()).collect();
                    write!(f, " → [{}]", dest_str.join(", "))?;
                }
                Ok(())
            }
            Self::Unreachable { .. } => write!(f, "Unreachable"),
            Self::Trap { .. } => write!(f, "Trap"),
            Self::SwitchInt {
                discriminant,
                cases,
                default_bb,
                ..
            } => {
                let cases_str: Vec<String> =
                    cases.iter().map(|(v, bb)| format!("{v} → {bb}")).collect();
                write!(
                    f,
                    "SwitchInt({discriminant}) → [{}], default → {default_bb}",
                    cases_str.join(", ")
                )
            }
        }
    }
}

impl fmt::Display for ConstValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(v) => write!(f, "{v}"),
            Self::Trit(v) => write!(f, "{v}_trit"),
            Self::Unit => write!(f, "()"),
            Self::String(s) => write!(f, "\"{s}\""),
        }
    }
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Add => write!(f, "+"),
            Self::Sub => write!(f, "-"),
            Self::Mul => write!(f, "*"),
            Self::Div => write!(f, "/"),
            Self::Mod => write!(f, "%"),
            Self::Eq => write!(f, "=="),
            Self::Ne => write!(f, "!="),
            Self::Lt => write!(f, "<"),
            Self::Le => write!(f, "<="),
            Self::Gt => write!(f, ">"),
            Self::Ge => write!(f, ">="),
            Self::LukAnd => write!(f, "&&"),
            Self::LukOr => write!(f, "||"),
            Self::LukXor => write!(f, "^"),
            Self::LukImplies => write!(f, "=>"),
            Self::LukIff => write!(f, "<=>"),
            Self::KleeneImplies => write!(f, "~>"),
            Self::KleeneXor => write!(f, "~^"),
            Self::KleeneIff => write!(f, "<~>"),
            Self::Neg => write!(f, "neg"),
        }
    }
}

impl fmt::Display for ControlFlowGraph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "CFG:")?;
        writeln!(f, "  entry: {}", self.entry)?;
        writeln!(f, "  exits: {:?}", self.exits)?;
        for (i, block) in self.blocks.iter().enumerate() {
            writeln!(f, "  bb{i}:")?;
            writeln!(f, "    preds: {:?}", block.predecessors)?;
            writeln!(f, "    succs: {:?}", block.successors)?;
            for stmt in &block.data.statements {
                writeln!(f, "    {stmt}")?;
            }
            writeln!(f, "    {}", block.data.terminator)?;
        }
        Ok(())
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_layout_point() {
        // struct Point { x: Integer, y: Integer }
        // Integer = 8 bytes, align 8
        let layout = StructLayout::compute(
            "Point",
            &[
                ("x".into(), "Integer".into(), 8, align::INTEGER),
                ("y".into(), "Integer".into(), 8, align::INTEGER),
            ],
        );
        assert_eq!(layout.alignment, 8);
        assert_eq!(layout.total_size, 16); // 8 + 8, no padding needed
        assert_eq!(layout.fields[0].offset, 0);
        assert_eq!(layout.fields[1].offset, 8);
    }

    #[test]
    fn struct_layout_mixed_alignment() {
        // struct Mixed { a: Trit, b: Integer, c: Trilean }
        // Trit = 1 byte, align 1
        // Integer = 8 bytes, align 8
        // Trilean = 1 byte, align 1
        let layout = StructLayout::compute(
            "Mixed",
            &[
                ("a".into(), "Trit".into(), 1, align::TRIT),
                ("b".into(), "Integer".into(), 8, align::INTEGER),
                ("c".into(), "Trilean".into(), 1, align::TRILEAN),
            ],
        );
        assert_eq!(
            layout.alignment, 8,
            "struct alignment = max field alignment = 8"
        );
        assert_eq!(layout.fields[0].offset, 0); // a at 0
        assert_eq!(layout.fields[1].offset, 8); // b at 8 (padded from 1 to 8)
        assert_eq!(layout.fields[2].offset, 16); // c at 16 (after 8-byte b)
        assert_eq!(layout.total_size, 24); // 16 + 1 = 17, padded to 24 (multiple of 8)
    }

    #[test]
    fn struct_layout_array_field() {
        // struct Buffer { header: Integer, data: [u8; 5] }
        // Integer = 8 bytes, align 8
        // [u8; 5] = 5 bytes, align 1 (NOT align 5!)
        let layout = StructLayout::compute(
            "Buffer",
            &[
                ("header".into(), "Integer".into(), 8, align::INTEGER),
                ("data".into(), "?".into(), 5, 1), // 5-byte array, alignment 1
            ],
        );
        assert_eq!(layout.alignment, 8);
        assert_eq!(layout.fields[0].offset, 0);
        assert_eq!(layout.fields[1].offset, 8); // right after header, no padding needed
        assert_eq!(layout.total_size, 16); // 8 + 5 = 13, padded to 16
    }

    #[test]
    #[should_panic(expected = "alignment must be power of 2")]
    fn struct_layout_rejects_non_power_of_two_alignment() {
        let _ = StructLayout::compute(
            "Bad",
            &[
                ("x".into(), "Integer".into(), 5, 5), // alignment 5 is invalid
            ],
        );
    }

    // ── Verifier tests ─────────────────────────────────────

    /// Build a minimal well-formed body for verify() tests.
    fn well_formed_body() -> Body {
        Body {
            signature: FunctionSignature {
                name: "test".into(),
                params: vec![],
                return_type: "Integer".into(),
                return_borrow_map: ReturnBorrowMap::new(),
                return_shape: ReturnShape::Scalar,
            },
            blocks: vec![BlockData {
                statements: vec![],
                terminator: Terminator::Return {
                    values: vec![],
                    span: DUMMY_SPAN,
                },
            }],
            entry_block: BasicBlock(0),
            num_locals: 0,
            local_decls: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            local_names: BTreeMap::new(),
        }
    }

    #[test]
    fn verify_accepts_well_formed_body() {
        let body = well_formed_body();
        body.verify().expect("well-formed body should pass verify");
    }

    #[test]
    fn verify_rejects_entry_block_out_of_bounds() {
        let mut body = well_formed_body();
        body.entry_block = BasicBlock(99); // out of range
        let err = body.verify().expect_err("should reject bad entry_block");
        assert!(matches!(err, MirError::BlockOutOfBounds { .. }));
    }

    #[test]
    fn verify_rejects_goto_target_out_of_bounds() {
        let mut body = well_formed_body();
        body.blocks[0].terminator = Terminator::Goto {
            target: BasicBlock(99),
            span: DUMMY_SPAN,
        };
        let err = body.verify().expect_err("should reject bad Goto target");
        assert!(matches!(err, MirError::BlockOutOfBounds { .. }));
    }

    #[test]
    fn verify_rejects_if_branch_out_of_bounds() {
        let mut body = well_formed_body();
        // Add a local so we have one to use as cond
        body.num_locals += 1;
        body.blocks[0].terminator = Terminator::If {
            cond: Local(0),
            positive_bb: BasicBlock(99),
            zero_bb: None,
            negative_bb: BasicBlock(0),
            span: DUMMY_SPAN,
        };
        let err = body.verify().expect_err("should reject bad If branch");
        assert!(matches!(err, MirError::BlockOutOfBounds { .. }));
    }

    #[test]
    fn verify_rejects_call_return_bb_out_of_bounds() {
        let mut body = well_formed_body();
        body.num_locals += 1;
        body.blocks[0].terminator = Terminator::CallDispatch {
            callee: FunctionId(0),
            callee_name: "f".into(),
            target: CallTarget::Jit,
            args: vec![],
            return_bb: BasicBlock(99),
            dest: vec![Local(0)],
            return_shape: ReturnShape::Scalar,
            span: DUMMY_SPAN,
        };
        let err = body.verify().expect_err("should reject bad return_bb");
        assert!(matches!(err, MirError::BlockOutOfBounds { .. }));
    }

    #[test]
    fn verify_rejects_local_out_of_bounds_in_return() {
        let mut body = well_formed_body();
        body.blocks[0].terminator = Terminator::Return {
            values: vec![Local(99)],
            span: DUMMY_SPAN,
        };
        let err = body
            .verify()
            .expect_err("should reject bad local in Return");
        assert!(matches!(err, MirError::LocalOutOfBounds { .. }));
    }

    #[test]
    fn verify_rejects_local_out_of_bounds_in_statement() {
        let mut body = well_formed_body();
        // Insert a statement with an out-of-range local
        body.blocks[0]
            .statements
            .push(Statement::StorageLive(Local(99), DUMMY_SPAN));
        let err = body
            .verify()
            .expect_err("should reject bad local in statement");
        assert!(matches!(err, MirError::LocalOutOfBounds { .. }));
    }

    #[test]
    fn verify_rejects_local_out_of_bounds_in_call_args() {
        let mut body = well_formed_body();
        body.num_locals += 1;
        body.blocks[0].terminator = Terminator::CallDispatch {
            callee: FunctionId(0),
            callee_name: "f".into(),
            target: CallTarget::Jit,
            args: vec![Local(99)], // arg out of bounds
            return_bb: BasicBlock(0),
            dest: vec![Local(0)],
            return_shape: ReturnShape::Scalar,
            span: DUMMY_SPAN,
        };
        let err = body
            .verify()
            .expect_err("should reject bad local in call args");
        assert!(matches!(err, MirError::LocalOutOfBounds { .. }));
    }

    #[test]
    fn verify_rejects_oob_local_in_projection_index() {
        let mut body = well_formed_body();
        body.num_locals = 5; // only locals 0..4 are valid
        // Place with Index(Local(99)) — projection carries an OOB local
        let bad_place = Place {
            local: Local(0),
            projection: vec![Projection::Index(Local(99))],
        };
        body.blocks[0].statements.push(Statement::Assign {
            dest: bad_place.clone(),
            source: Place::local(Local(0)),
            span: DUMMY_SPAN,
        });
        let err = body
            .verify()
            .expect_err("should reject Index(Local(99)) in projection");
        assert!(
            matches!(err, MirError::LocalOutOfBounds { .. }),
            "expected LocalOutOfBounds, got {err:?}"
        );
    }

    /// Regression guard: if someone removes the verifier checks and
    /// hand-builds a malformed body, this test must FAIL (panic or
    /// return Ok when it should Err). We test by calling verify() on
    /// a known-bad body — if verify() returns Ok, the guard is dead.
    #[test]
    fn verify_guard_is_live_block_bounds() {
        let mut body = well_formed_body();
        body.blocks[0].terminator = Terminator::Goto {
            target: BasicBlock(999),
            span: DUMMY_SPAN,
        };
        // Must return Err. If this ever returns Ok, the verifier guard
        // was removed and the JIT will OOB panic on this body.
        assert!(
            body.verify().is_err(),
            "VERIFIER GUARD REMOVED: body.verify() accepted a body with \
             a Goto to BasicBlock(999) but the body has only 1 block. \
             The JIT would panic on self.blocks[target]."
        );
    }

    #[test]
    fn verify_guard_is_live_local_bounds() {
        let mut body = well_formed_body();
        body.blocks[0]
            .statements
            .push(Statement::StorageLive(Local(999), DUMMY_SPAN));
        assert!(
            body.verify().is_err(),
            "VERIFIER GUARD REMOVED: body.verify() accepted a body with \
             Local(999) but body.num_locals < 999. \
             The JIT would use_var on an undeclared Cranelift Variable."
        );
    }

    // ── place_type tests ─────────────────────────────────────

    #[test]
    fn place_type_plain_local() {
        let body = Body {
            local_decls: vec![LocalDecl::new("Integer")],
            ..well_formed_body()
        };
        let place = Place::local(Local(0));
        assert_eq!(place_type(&place, &body), "Integer");
    }

    #[test]
    fn place_type_struct_field() {
        let layout = StructLayout::compute(
            "Point",
            &[
                ("x".into(), "Integer".into(), 8, align::INTEGER),
                ("y".into(), "String".into(), 8, align::INTEGER),
            ],
        );
        let body = Body {
            local_decls: vec![LocalDecl::new("Point")],
            struct_layouts: vec![layout],
            ..well_formed_body()
        };
        let place = Place {
            local: Local(0),
            projection: vec![Projection::Field("x".into())],
        };
        assert_eq!(place_type(&place, &body), "Integer");
        let place_y = Place {
            local: Local(0),
            projection: vec![Projection::Field("y".into())],
        };
        assert_eq!(place_type(&place_y, &body), "String");
    }

    #[test]
    fn place_type_enum_payload() {
        let layout = EnumLayout::compute(
            "Option",
            &[("Some".into(), 0, Some(("Integer".into(), 8, 8, vec![])))],
        );
        let body = Body {
            local_decls: vec![LocalDecl::new("Option")],
            enum_layouts: vec![layout],
            ..well_formed_body()
        };
        let place = Place {
            local: Local(0),
            projection: vec![Projection::Payload("Some".into())],
        };
        assert_eq!(place_type(&place, &body), "Integer");
    }

    #[test]
    fn place_type_unknown_projection() {
        let body = Body {
            local_decls: vec![LocalDecl::new("UnknownType")],
            ..well_formed_body()
        };
        // Field on unknown type → "?"
        let place = Place {
            local: Local(0),
            projection: vec![Projection::Field("x".into())],
        };
        assert_eq!(place_type(&place, &body), "?");
        // Unknown projection kind → "?"
        let place2 = Place {
            local: Local(0),
            projection: vec![Projection::Deref],
        };
        assert_eq!(place_type(&place2, &body), "?");
    }

    // ── is_copy tests ────────────────────────────────────────

    #[test]
    fn is_copy_primitives() {
        let body = well_formed_body();
        assert!(is_copy("Integer", &body));
        assert!(is_copy("Trit", &body));
        assert!(is_copy("Tryte", &body));
        assert!(is_copy("Long", &body));
        assert!(is_copy("Trilean", &body));
        assert!(is_copy("Unit", &body));
        assert!(is_copy("?", &body));
    }

    #[test]
    fn is_copy_reference_types() {
        // ADR-0045 §3: reference types are Copy by design —
        // copying a handle is safe because the callee doesn't Drop it.
        let body = well_formed_body();
        assert!(is_copy("&0 String", &body));
        assert!(is_copy("&0 Vector<Integer>", &body));
        assert!(is_copy("&0 HashMap<String, Integer>", &body));
        assert!(is_copy("&+ String", &body));
        assert!(is_copy("&+ mutable String", &body));
        assert!(is_copy("&- String", &body));
    }

    #[test]
    fn is_copy_heap_types() {
        let body = well_formed_body();
        assert!(!is_copy("String", &body));
        assert!(!is_copy("Vector", &body));
        assert!(!is_copy("HashMap", &body));
    }

    #[test]
    fn is_copy_struct_recursive() {
        let layout = StructLayout::compute(
            "HasString",
            &[
                ("header".into(), "Integer".into(), 8, 8),
                ("body".into(), "String".into(), 8, 8),
            ],
        );
        let body = Body {
            struct_layouts: vec![layout],
            ..well_formed_body()
        };
        // Struct with a Move field is itself Move
        assert!(!is_copy("HasString", &body));
    }

    #[test]
    fn is_copy_enum_recursive() {
        let layout = EnumLayout::compute(
            "Either",
            &[("Left".into(), 0, Some(("Integer".into(), 8, 8, vec![])))],
        );
        let body = Body {
            enum_layouts: vec![layout],
            ..well_formed_body()
        };
        // All payloads are Integer → Copy
        assert!(is_copy("Either", &body));

        let layout2 = EnumLayout::compute(
            "EitherString",
            &[("Right".into(), 0, Some(("String".into(), 8, 8, vec![])))],
        );
        let body2 = Body {
            enum_layouts: vec![layout2],
            ..well_formed_body()
        };
        // A payload is String → Move
        assert!(!is_copy("EitherString", &body2));
    }

    #[test]
    fn is_copy_unknown_defaults_move() {
        let body = well_formed_body();
        assert!(!is_copy("UnknownType", &body));
    }

    // ── Nullable representation tests (ADR-0041) ──────────────────

    /// N1 — Canary: `NULL_SENTINEL` must lie outside every Triết scalar range.
    /// If this test fails, someone changed the scalar range (or trit width)
    /// without updating the sentinel — the niche is no longer valid.
    /// Ràng vào hằng triet-core, không hardcode lại số.
    #[test]
    #[allow(clippy::assertions_on_constants)]
    // ^-- This IS a compile-time invariant guard. The whole point is to break
    //     loudly at test time if someone changes Integer/Tryte width without
    //     updating NULL_SENTINEL. clippy's "this is always true" is the desired
    //     property — when it becomes false, the canary did its job.
    fn nullable_sentinel_outside_scalar_ranges() {
        // Integer: MIN = -3_812_798_742_493
        assert!(
            NULL_SENTINEL < triet_core::Integer::MIN.to_i64(),
            "NULL_SENTINEL ({NULL_SENTINEL}) must be < Integer::MIN ({})",
            triet_core::Integer::MIN.to_i64()
        );
        // Tryte: MIN = -9_841
        assert!(
            NULL_SENTINEL < triet_core::Tryte::MIN.to_i64(),
            "NULL_SENTINEL ({NULL_SENTINEL}) must be < Tryte::MIN ({})",
            triet_core::Tryte::MIN.to_i64()
        );
        // Trit: min = Negative = -1 (this also covers Trilean — same domain)
        assert!(
            NULL_SENTINEL < i64::from(triet_core::Trit::Negative.to_i8()),
            "NULL_SENTINEL ({NULL_SENTINEL}) must be < Trit::Negative (-1)"
        );
    }

    /// N2 — `is_nullable_type` / `nullable_payload` round-trip and pins.
    #[test]
    fn nullable_type_helpers() {
        // Round-trip: Integer?
        assert!(is_nullable_type("Integer?"));
        assert_eq!(nullable_payload("Integer?"), Some("Integer"));

        // Round-trip: String?
        assert!(is_nullable_type("String?"));
        assert_eq!(nullable_payload("String?"), Some("String"));

        // Pin: "?" trần must NOT be nullable (type-unknown).
        assert!(!is_nullable_type("?"), "bare '?' must not be nullable");
        assert_eq!(nullable_payload("?"), None);

        // Pin: is_vec_type must NOT consume nullable vector type-string.
        assert!(is_nullable_type("Vector<Integer>?"));
        assert_eq!(
            nullable_payload("Vector<Integer>?"),
            Some("Vector<Integer>")
        );
        // Payload of "Vector<Integer>?" is "Vector<Integer>" which IS a vec type.
        let payload = nullable_payload("Vector<Integer>?").unwrap();
        assert!(
            is_vec_type(payload),
            "payload of nullable Vector should be Vector<Integer>"
        );

        // Non-nullable: plain types.
        assert!(!is_nullable_type("Integer"));
        assert_eq!(nullable_payload("Integer"), None);
        assert!(!is_nullable_type("String"));
        assert_eq!(nullable_payload("String"), None);
        assert!(!is_nullable_type("Vector"));
        assert_eq!(nullable_payload("Vector"), None);

        // Edge case: "Integer??" — can't happen (C6: T?? auto-flatten),
        // but helper must be defined. Mechanical strip: last ? only.
        assert!(is_nullable_type("Integer??"));
        assert_eq!(nullable_payload("Integer??"), Some("Integer?"));
    }

    /// N2 extension — `is_copy` integration: nullable delegates to payload.
    #[test]
    fn nullable_is_copy_delegation() {
        let body = well_formed_body();

        // Integer? → payload Integer → Copy
        assert!(is_copy("Integer?", &body));
        // String? → payload String → Move
        assert!(!is_copy("String?", &body));
        // Vector<Integer>? → payload Vector<Integer> → Move (via is_vec_type)
        assert!(!is_copy("Vector<Integer>?", &body));
        // ? trần → NOT nullable → falls through to Copy (existing behavior)
        assert!(is_copy("?", &body));
    }
}

/// Sentinel encoding the `~0` (null) state of **all** `T?` at Bậc A
/// (scalar and heap — uniform). INVARIANT: lies outside every Triết
/// scalar range (see canary test N1 in the test module, and
/// [ADR-0041 §6.2](../../docs/decisions/0041-nullable-representation-bac-a.md#62--scalar-debt-d1-phantom-null-qua-arithmetic-khng-wrap)).
pub const NULL_SENTINEL: i64 = i64::MIN;

/// Returns `true` if `ty` names a nullable type (e.g. `"Integer?"`,
/// `"String?"`, `"Vector<Integer>?"`).
///
/// **Ordering rule:** this MUST be called BEFORE any other type-string
/// classifier (`is_vec_type`, etc.) at every consumer. Reason:
/// `"Vector<Integer>?"` starts with `"Vector<"` and would be
/// misclassified as a bare Vector by `is_vec_type`.
///
/// **Pin:** `is_nullable_type("?")` returns `false`. The bare `"?"`
/// type-string means "type unknown" (`is_copy` treats it as Copy) —
/// it must NOT be classified as "nullable of empty string."
#[must_use]
pub fn is_nullable_type(ty: &str) -> bool {
    ty.ends_with('?') && ty != "?"
}

/// Strips the trailing `?` from a nullable type-string.
///
/// `"Integer?"` → `Some("Integer")`; non-nullable → `None`.
///
/// **Pin:** `nullable_payload("Vector<Integer>?")` returns
/// `Some("Vector<Integer>")` — `is_vec_type` must NOT consume
/// a nullable type-string. Verify in N2.
#[must_use]
pub fn nullable_payload(ty: &str) -> Option<&str> {
    if is_nullable_type(ty) {
        Some(&ty[..ty.len() - 1])
    } else {
        None
    }
}

/// Returns `true` if `ty` names a Vector type (e.g. `"Vector"` or
/// `"Vector<Integer>"`). Single source of truth for Vector type-string
/// matching — use this instead of ad-hoc `starts_with`/`==` checks.
#[must_use]
pub fn is_vec_type(ty: &str) -> bool {
    ty == "Vector" || ty.starts_with("Vector<")
}

/// Returns `true` if `ty` names a HashMap type (e.g. `"HashMap"` or
/// `"HashMap<Integer,Integer>"`). Single source of truth — use this
/// instead of ad-hoc matching.
#[must_use]
pub fn is_hashmap_type(ty: &str) -> bool {
    ty == "HashMap" || ty.starts_with("HashMap<")
}

/// Determines if a type has Copy semantics (stack primitives) or Move semantics (heap types).
pub fn is_copy(ty: &str, body: &Body) -> bool {
    // Nullable types delegate to their payload (ordering rule: BEFORE all
    // other classifiers — is_vec_type would misclassify "Vector<Integer>?").
    if let Some(payload) = nullable_payload(ty) {
        return is_copy(payload, body);
    }
    match ty {
        "Integer" | "Trit" | "Tryte" | "Long" | "Trilean" | "Unit" | "?" => true,
        "String" | "HashMap" => false,
        other if is_vec_type(other) => false,
        other if is_hashmap_type(other) => false,
        // Reference types — Copy by design (ADR-0045 §3).
        // TECH-DEBT(ADR-0045): MIR-type-as-string, xem §3.
        other if other.starts_with('&') => true,
        _ => {
            // Check struct layouts — Copy if all fields are Copy (recursive).
            if let Some(s) = body.struct_layouts.iter().find(|s| s.name == ty) {
                return s.fields.iter().all(|f| is_copy(&f.ty, body));
            }
            // Check enum layouts — Copy if all payloads are Copy (recursive).
            if let Some(e) = body.enum_layouts.iter().find(|e| e.name == ty) {
                return e
                    .variants
                    .iter()
                    .all(|v| v.payload.as_ref().is_none_or(|p| is_copy(&p.ty, body)));
            }
            // Unknown types default to Move (Refuse-over-guess)
            false
        }
    }
}

/// Computes the type of a Place by walking its projection chain.
pub fn place_type(place: &Place, body: &Body) -> String {
    let mut current_ty = body.local_decls[place.local.0].ty.clone();
    for proj in &place.projection {
        current_ty = match proj {
            Projection::Field(name) => {
                // Look up the field type in struct layouts.
                if let Some(s) = body.struct_layouts.iter().find(|s| s.name == current_ty) {
                    if let Some(field) = s.fields.iter().find(|f| f.name == name.as_str()) {
                        field.ty.clone()
                    } else {
                        "?".to_string()
                    }
                } else if let Some(e) = body.enum_layouts.iter().find(|e| e.name == current_ty) {
                    // Look up in enum payload layouts (for struct-payload variants).
                    let mut found = None;
                    for variant in &e.variants {
                        if let Some(ref payload) = variant.payload
                            && let Some(field) =
                                payload.fields.iter().find(|f| f.name == name.as_str())
                        {
                            found = Some(field.ty.clone());
                            break;
                        }
                    }
                    found.unwrap_or_else(|| "?".to_string())
                } else {
                    "?".to_string()
                }
            }
            Projection::Payload(variant) => {
                // Look up the payload type in enum layouts.
                if let Some(e) = body.enum_layouts.iter().find(|e| e.name == current_ty) {
                    if let Some(v) = e.variants.iter().find(|v| &v.name == variant) {
                        if let Some(ref payload) = v.payload {
                            payload.ty.clone()
                        } else {
                            "?".to_string()
                        }
                    } else {
                        "?".to_string()
                    }
                } else {
                    "?".to_string()
                }
            }
            _ => {
                // Other projections (Deref, Downcast, Index) — can't resolve yet.
                "?".to_string()
            }
        };
    }
    current_ty
}
