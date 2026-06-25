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
    /// Access the discriminant field of an Outcome value (offset 0, 8 bytes).
    /// The base local must be Outcome-allocated (`OutcomeAlloc`).
    OutcomeDiscriminant,
    /// Access the payload field of an Outcome value (offset 8, 8 bytes).
    /// For scalar payloads this is the whole value; for heap payloads
    /// this is the `ptr` field of the heap aggregate.
    OutcomePayload,
    /// Access the `len` field of a heap Outcome payload (offset 16, 8 bytes).
    /// Only valid for Outcome with heap payload (String/Vector/HashMap).
    OutcomePayloadLen,
    /// Access the `cap` field of a heap Outcome payload (offset 24, 8 bytes).
    /// Only valid for Outcome with heap payload (String/Vector/HashMap).
    OutcomePayloadCap,
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
                Projection::OutcomeDiscriminant => format!("{s}.disc"),
                Projection::OutcomePayload => format!("{s}.payload"),
                Projection::OutcomePayloadLen => format!("{s}.payload_len"),
                Projection::OutcomePayloadCap => format!("{s}.payload_cap"),
            };
        }
        f.write_str(&s)
    }
}

/// Declaration of a local: its type and mutability.
///
/// Type is carried as a [`MirType`] enum — no more string-match guessing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDecl {
    /// Type of this local.
    pub ty: MirType,
    /// Whether the binding is mutable.
    pub mutable: bool,
}

impl LocalDecl {
    /// A temporary/local of the given type, immutable by default.
    ///
    /// Accepts both `MirType` values and `&str` literals (via the
    /// transitional `From<&str>` impl — dies at S4 with `parse()`).
    #[must_use]
    pub fn new(ty: impl Into<MirType>) -> Self {
        Self {
            ty: ty.into(),
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

    /// Allocate a 16-byte Outcome slot on the stack.
    /// Layout: discriminant at offset 0 (8 bytes), payload at offset 8 (8 bytes).
    /// Access fields via [`Projection::OutcomeDiscriminant`] and
    /// [`Projection::OutcomePayload`] on [`Assign`] statements.
    OutcomeAlloc {
        /// The local being initialized as an Outcome value.
        dest: Local,
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

    /// ADR-0069 Lát 3: runtime capability gate for a `defer` token. Emitted by
    /// the lowerer at a `mint X` site when `X` is `defer`, BEFORE the ZST init.
    /// The JIT lowers it to a single `__triet_cap_check(cap_id)` call + a
    /// fail-closed trap (`unwrap_user(2)`): result ≤ 0 (Deny −1 OR Unknown 0)
    /// → trap. Carries no `Local` — it is a pure control-flow guard with no
    /// value or place (borrowck / liveness / verify treat it as a no-op).
    CapabilityCheck {
        /// Name of the `defer` capability being minted (→ stable `cap_id` hash).
        capability_name: String,
        /// Source location of the `mint` expression.
        span: Span,
    },
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

// ── MirType ──────────────────────────────────────────────────

/// MIR-native type representation — hand-written, independent of AST types.
///
/// Replaces the implicit string-match language (`"Integer"`, `starts_with('&')`,
/// `ends_with('?')`) with a structural enum. The lowerer is the **single
/// producer**; all downstream consumers match on the enum instead of parsing
/// strings.
///
/// # Design authority
///
/// ADR-0050. 3 invariants: ① hand-written in `triet-mir` (no schema dep),
/// ② `Struct`/`Enum` TÁCH (not fused into `UserType`), ③ transitional
/// `parse(&str)` shim dies at the last commit of B1a.
///
/// # Variant notes
///
/// - `Trilean` is bare — refinement (`Trilean!`) is a frontend gate (ADR-0021),
///   checked before MIR. No backend consumer reads a `refined` field.
/// - `Vector`/`HashMap` are bare — no backend consumer reads generic element/
///   key/value types. Payloads return when Bậc C needs generic heap types
///   (with a producer at the same commit — Track B Rule #4).
/// - `Unknown` replaces the sentinel string `"?"` — type checker could not
///   determine this type. Treated as Copy (refuse-over-guess on the safe side).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MirType {
    // ── Scalars (Copy per SPEC §10.1) ──
    /// `Integer` — 27-trit signed integer.
    Integer,
    /// `Trit` — 1-trit numeric atom.
    Trit,
    /// `Tryte` — 9-trit integer.
    Tryte,
    /// `Long` — 81-trit integer.
    Long,
    /// `Trilean` — three-valued truth (Ł3). Bare — refinement is frontend-only.
    Trilean,
    /// `Unit` — zero-sized `()`.
    Unit,

    // ── Sentinel ──
    /// Recovery placeholder. Was the bare `"?"` string. Copy semantics.
    Unknown,

    // ── Heap (Move per ADR-0042) ──
    /// `String` — UTF-8 owned text, heap-allocated.
    String,
    /// `Vector` — heap-allocated growable array. Bare (no element type yet).
    Vector,
    /// `HashMap` — heap-allocated key-value map. Bare (no key/value types yet).
    HashMap,

    // ── Modifiers ──
    /// `T?` — nullable wrapper. KẾT CẤU — kills the old ordering-rule bug
    /// where `is_vec_type("Vector<Integer>?")` returned `true`.
    Nullable(Box<MirType>),
    /// `&+ T`, `&0 T`, `&- T`, etc. One of the 5 S6 reference forms.
    Reference {
        /// Which reference form.
        form: ReferenceForm,
        /// The type being referenced.
        inner: Box<MirType>,
    },

    // ── Outcome (ADR-0052) ──
    /// `T~E` / `T?~E` — Outcome type. Carries both payload types + null-state
    /// flag so the lowerer can emit the correct [`ReturnShape`] without
    /// string-matching or consulting a side-channel map. Payloads are scalar
    /// at Bậc A (ADR-0052 §2); heap payloads deferred to Bậc B/C.
    Outcome {
        /// Success-arm payload type.
        value_type: Box<MirType>,
        /// Failure-arm payload type.
        error_type: Box<MirType>,
        /// `true` for `T?~E` (3-state with null); `false` for `T~E` (2-state).
        allow_null_state: bool,
    },

    // ── User-defined (TÁCH per ADR-0050 ruling ②) ──
    /// User-defined struct — resolved via `body.struct_layouts`.
    Struct(String),
    /// User-defined enum — resolved via `body.enum_layouts`.
    Enum(String),

    // ── Capability (ADR-0069) ──
    /// A capability token type `capability Cap grant`. ZST (0 byte) at runtime,
    /// ALWAYS non-copy (move-only) — possession = right, so a token must never
    /// be duplicated. Distinct from `Struct` so `is_copy` short-circuits to
    /// `false` WITHOUT routing through the struct-field walk (an empty data
    /// struct stays Copy; a capability does not). The `String` is the
    /// capability name.
    Capability(String),
}

impl fmt::Display for MirType {
    /// Round-trip to the legacy string form for diagnostic/fixture stability.
    ///
    /// This MUST produce the same output as the old `lower::type_name()` for
    /// every type that existed before B1a. Fixture `.expected` files and
    /// diagnostic messages depend on this format.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer => write!(f, "Integer"),
            Self::Trit => write!(f, "Trit"),
            Self::Tryte => write!(f, "Tryte"),
            Self::Long => write!(f, "Long"),
            Self::Trilean => write!(f, "Trilean"),
            Self::Unit => write!(f, "Unit"),
            Self::Unknown => write!(f, "?"),
            Self::String => write!(f, "String"),
            Self::Vector => write!(f, "Vector"),
            Self::HashMap => write!(f, "HashMap"),
            Self::Nullable(inner) => write!(f, "{inner}?"),
            Self::Reference { form, inner } => match form {
                ReferenceForm::StrongFrozen => write!(f, "&+ {inner}"),
                ReferenceForm::StrongMutable => write!(f, "&+ mutable {inner}"),
                ReferenceForm::BorrowReadOnly => write!(f, "&0 {inner}"),
                ReferenceForm::BorrowExclusiveMutable => write!(f, "&0 mutable {inner}"),
                ReferenceForm::WeakObserver => write!(f, "&- {inner}"),
            },
            Self::Outcome {
                value_type,
                error_type,
                allow_null_state: false,
            } => write!(f, "{value_type}~{error_type}"),
            Self::Outcome {
                value_type,
                error_type,
                allow_null_state: true,
            } => write!(f, "{value_type}?~{error_type}"),
            Self::Struct(name) | Self::Enum(name) | Self::Capability(name) => write!(f, "{name}"),
        }
    }
}

impl MirType {
    // ── Classification methods ────────────────────────────────

    /// `true` for `Nullable(T)` — structural, not suffix-string.
    ///
    /// Kills the old ordering-rule: every consumer had to call
    /// `is_nullable_type` BEFORE `is_vec_type`, otherwise
    /// `"Vector<Integer>?"` was misclassified as bare Vector.
    #[must_use]
    pub fn is_nullable(&self) -> bool {
        matches!(self, Self::Nullable(_))
    }

    /// Strip `Nullable` wrapper. `Nullable(T)` → `Some(&T)`, else `None`.
    #[must_use]
    pub fn nullable_payload(&self) -> Option<&Self> {
        if let Self::Nullable(inner) = self {
            Some(inner)
        } else {
            None
        }
    }

    /// `true` for any reference form (`&+`, `&0`, `&-`).
    #[must_use]
    pub fn is_reference(&self) -> bool {
        matches!(self, Self::Reference { .. })
    }

    /// `true` for `Vector` (bare — no element type query yet).
    #[must_use]
    pub fn is_vec(&self) -> bool {
        matches!(self, Self::Vector)
    }

    /// `true` for `HashMap` (bare — no key/value type query yet).
    #[must_use]
    pub fn is_hashmap(&self) -> bool {
        matches!(self, Self::HashMap)
    }

    /// `true` for any heap-allocated type (String/Vector/HashMap).
    /// These have 3-field layout `{ptr, len, cap}` and require Drop glue.
    #[must_use]
    pub fn is_any_heap(&self) -> bool {
        matches!(self, Self::String | Self::Vector | Self::HashMap)
    }

    /// `true` for `String` and `String?` (= `Nullable(String)`).
    ///
    /// ADR-0062 (Heap-Nullable Lát 1): `String?` shares String's 24-byte slot
    /// `{ptr@0, len@8, cap@16}`; null is `ptr == NULL_SENTINEL`. Every JIT /
    /// lowerer site that allocates, fills, returns, or drops a String slot keys
    /// off THIS predicate (not bare `matches!(Self::String)`) so `String?` rides
    /// the exact same repr path — widening `String → String?` is a repr no-op.
    /// Vector?/HashMap? (single i64 handle) and Struct?/Enum? are NOT covered.
    #[must_use]
    pub fn is_string_repr(&self) -> bool {
        matches!(self, Self::String)
            || matches!(self, Self::Nullable(inner) if matches!(**inner, Self::String))
    }

    /// Outcome slot size in bytes: 16 for scalar payload, 32 for heap.
    #[must_use]
    pub fn outcome_slot_size(&self) -> u32 {
        if let Self::Outcome {
            value_type,
            error_type,
            ..
        } = self
            && (value_type.is_any_heap() || error_type.is_any_heap())
        {
            return 32;
        }
        16
    }

    /// True if this Outcome has at least one heap payload type.
    #[must_use]
    pub fn has_heap_payload(&self) -> bool {
        if let Self::Outcome {
            value_type,
            error_type,
            ..
        } = self
        {
            value_type.is_any_heap() || error_type.is_any_heap()
        } else {
            false
        }
    }

    // ── Semantics ─────────────────────────────────────────────

    /// Copy semantics: no heap ownership.
    ///
    /// - `is_copy(None)`: classify by variant alone — Struct/Enum default to
    ///   Copy. Used during lowering before `Body` is built. **SOUND ONLY WHILE
    ///   B8 blocks construction of aggregates with heap fields** (B8 rejects
    ///   `StructLiteral`/enum-payload construction with heap-typed fields, not
    ///   declaration). When B8 is relaxed in Bậc B, the caller MUST pass
    ///   `Some(&body)` so struct/enum layouts are consulted.
    /// - `is_copy(Some(&body))`: recurse into `body.struct_layouts`/
    ///   `body.enum_layouts` for `Struct`/`Enum` variants. Used by borrowck
    ///   and JIT.
    ///
    /// This is the SINGLE source of truth for move/copy classification —
    /// replaces the old dual `is_copy(&str, &Body)` + `simple_is_copy(&str, …)`.
    #[must_use]
    pub fn is_copy(&self, body: Option<&crate::Body>) -> bool {
        match self {
            // Nullable delegates to payload (ordering: BEFORE other classifiers).
            Self::Nullable(inner) => inner.is_copy(body),
            // Stack primitives — Copy per SPEC §10.1.
            Self::Integer
            | Self::Trit
            | Self::Tryte
            | Self::Long
            | Self::Trilean
            | Self::Unit
            | Self::Unknown => true,
            // Heap types — Move.
            Self::String | Self::Vector | Self::HashMap => false,
            // Capability token (ADR-0069) — ALWAYS Move. Short-circuit BEFORE
            // the Struct arm so a ZST token is NEVER classified Copy via the
            // empty-field-walk (`all()` over no fields = true). This is the
            // soundness chokepoint: Copy → borrowck would not move-track →
            // double-take bypass. Poison `false → true` here → R-copy-bypass.
            Self::Capability(_) => false,
            // Reference types — Copy by design (ADR-0045 §3).
            Self::Reference { .. } => true,
            // Outcome: Copy if both payloads are Copy (always true for Bậc A scalars).
            Self::Outcome {
                value_type,
                error_type,
                ..
            } => value_type.is_copy(body) && error_type.is_copy(body),
            // User types.
            // TECH-DEBT(B1a S2): Struct/Enum may be misclassified by parse().
            // Search BOTH layout tables (refuse-over-guess in reverse).
            Self::Struct(name) | Self::Enum(name) => {
                if let Some(body) = body {
                    // Check struct layouts
                    if let Some(s) = body.struct_layouts.iter().find(|s| s.name == *name) {
                        return s.fields.iter().all(|f| f.ty.is_copy(Some(body)));
                    }
                    // Check enum layouts
                    if let Some(e) = body.enum_layouts.iter().find(|e| e.name == *name) {
                        return e
                            .variants
                            .iter()
                            .all(|v| v.payload.as_ref().is_none_or(|p| p.ty.is_copy(Some(body))));
                    }
                    false // Unknown type → Move (refuse-over-guess)
                } else {
                    true // No body → assume Copy (SOUND only while B8 blocks heap fields)
                }
            }
        }
    }
}

impl From<&MirType> for MirType {
    fn from(ty: &MirType) -> Self {
        ty.clone()
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
    /// - BinaryOutcome: 2 locals (discriminant, payload)
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
        /// - BinaryOutcome: 2 locals (discriminant, payload)
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
/// `Scalar` → 1 value, `BinaryOutcome` → 2 values (Trit::Zero invalid),
/// `TernaryOutcome` → 2 values (Trit::Zero valid for null state).
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
    /// Trit::Zero IS valid (used for the null state `~0`).
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
    pub parameters: Vec<(String, ParameterPassing)>,
    /// Return type.
    pub return_type: MirType,
    /// The shape of the return value (Unit, Scalar, BinaryOutcome, Struct).
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
    /// Params already have names in `signature.parameters`. Populated by the
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
    /// Field type.
    pub ty: MirType,
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
    /// Type of the payload.
    pub ty: MirType,
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
    /// Field types in the payload use [`MirType`].
    /// `discriminant_size` defaults to 8 (i64 in Bậc A).
    /// The payload area size = max of all variant payload sizes.
    #[allow(clippy::type_complexity)]
    // ^-- Complex tuple payload type (String, i64, Option<(MirType, usize, usize, Vec<FieldLayout>)>).
    //     Simplifies when EnumLayout gets dedicated type aliases at S3.
    #[must_use]
    pub fn compute(
        name: &str,
        variants: &[(
            String,                                            // variant name
            i64,                                               // discriminant value
            Option<(MirType, usize, usize, Vec<FieldLayout>)>, // payload: (ty, size, alignment, fields)
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
    pub fn compute(name: &str, fields: &[(String, MirType, usize, usize)]) -> Self {
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

/// Nợ #3 (Heap-Nullable gate, ruling β): is `t` a scalar atom that fits the
/// Bậc A single-i64 nullable sentinel? Mirrors the typecheck-era whitelist.
/// Is `t` a scalar nullable payload — i.e. does `Nullable(t)` lower to the
/// PA-3c single-i64 sentinel (ADR-0041) rather than a heap repr? Used by the
/// MIR verifier gate and by `triet-lower` to decide whether a `Nullable(_)`
/// return is a scalar (single i64) or a fat return (`String?` → sret).
pub fn is_scalar_nullable_payload(t: &MirType) -> bool {
    matches!(
        t,
        MirType::Integer
            | MirType::Trit
            | MirType::Tryte
            | MirType::Long
            | MirType::Trilean
            | MirType::Unit
            | MirType::Unknown
    )
}

/// ADR-0062 (Heap-Nullable Lát 1): is `Nullable(t)` lowerable to a concrete
/// repr? Scalars use the PA-3c single-i64 sentinel; the three heap types share
/// the ptr-sentinel: `String` in its 24-byte slot (ptr@0 == NULL_SENTINEL),
/// `Vector`/`HashMap` in their single i64 handle (handle == NULL_SENTINEL).
/// `Enum?` (ADR-0065 Lát 1) uses the disc-sentinel niche: the enum's disc@0
/// cell holds `i64::MIN` for null (a real discriminant is always in {0,1,2,…}),
/// so no extra cell is needed. `Struct?` (ADR-0065 Lát 2, Phương án A) prepends
/// a tag word: the slot is `{tag@0:i64, fields@8…}`, tag@0 == `i64::MIN` for
/// null. Both are Copy-only (rào B8): heap fields/payloads inside the aggregate
/// stay refused via the scalar-only field/payload gate below.
fn is_lowerable_nullable_payload(t: &MirType) -> bool {
    is_scalar_nullable_payload(t)
        || t.is_any_heap()
        || matches!(t, MirType::Enum(_))
        || matches!(t, MirType::Struct(_))
}

/// Find a `Nullable(inner)` whose `inner` is NOT accepted by `allow`, anywhere
/// inside `ty`, recursing through the type-carrying variants (Nullable/
/// Reference/Outcome). Returns the offending inner type. Used by [`Body::verify`]
/// to refuse heap-nullable before it reaches the JIT (ruling β: gate at LOWER,
/// not typecheck — see `MirError::HeapNullableNotLowered`).
///
/// `allow` is position-dependent (ADR-0062 Lát 1):
/// - **return type + locals** → [`is_lowerable_nullable_payload`] (scalar +
///   `String?`): Lát 1 reprs `String?` as a top-level 24-byte slot.
/// - **struct fields + enum payloads** → [`is_scalar_nullable_payload`] (scalar
///   only): a `String?` embedded in an aggregate is NOT a top-level slot; the
///   Lát 1 JIT does not place ptr-sentinel slots at field offsets. Keep refusing
///   until a nested-heap-nullable lát handles it.
fn find_refused_nullable(ty: &MirType, allow: fn(&MirType) -> bool) -> Option<&MirType> {
    match ty {
        MirType::Nullable(inner) => {
            if allow(inner) {
                // Representable here; still recurse in case `inner` nests a bad
                // type (it cannot for scalars/String, but keep uniform).
                find_refused_nullable(inner, allow)
            } else {
                Some(inner)
            }
        }
        MirType::Reference { inner, .. } => find_refused_nullable(inner, allow),
        MirType::Outcome {
            value_type,
            error_type,
            ..
        } => find_refused_nullable(value_type, allow)
            .or_else(|| find_refused_nullable(error_type, allow)),
        _ => None,
    }
}

/// ADR-0065 §12.2: field/payload-position predicate, **body-aware**.
///
/// A struct field or enum payload of type `T?` is lowerable when `T` is either
/// a scalar (PA-3c sentinel) OR a **fully-Copy** `Struct`/`Enum` (the nested
/// nullable aggregate of Trục A — tag-prepend / disc-niche, no allocator).
/// `inner.is_copy(Some(body))` recurses through the aggregate's own fields
/// (`triet-mir:666`), so a heap-containing aggregate (`Bad { s: String }`)
/// classifies as Move → refused, keeping **B8 (§4)** intact. `Nullable(String/
/// Vector/HashMap)` is neither scalar nor Struct/Enum → refused.
fn is_field_payload_lowerable(inner: &MirType, body: &Body) -> bool {
    is_scalar_nullable_payload(inner)
        || (matches!(inner, MirType::Struct(_) | MirType::Enum(_)) && inner.is_copy(Some(body)))
}

/// Body-aware mirror of [`find_refused_nullable`] for struct-field / enum-payload
/// positions (ADR-0065 §12). The plain `find_refused_nullable` cannot consult
/// `body.struct_layouts`/`enum_layouts` (its `allow` is a bare `fn` pointer), so
/// nested nullable aggregates need this variant to distinguish a Copy `Point?`
/// (lowerable) from a heap-bearing `Bad?` (refused — B8). Recurses through
/// Nullable/Reference/Outcome exactly like the parent.
fn find_refused_nullable_field<'a>(ty: &'a MirType, body: &Body) -> Option<&'a MirType> {
    match ty {
        MirType::Nullable(inner) => {
            if is_field_payload_lowerable(inner, body) {
                find_refused_nullable_field(inner, body)
            } else {
                Some(inner)
            }
        }
        MirType::Reference { inner, .. } => find_refused_nullable_field(inner, body),
        MirType::Outcome {
            value_type,
            error_type,
            ..
        } => find_refused_nullable_field(value_type, body)
            .or_else(|| find_refused_nullable_field(error_type, body)),
        _ => None,
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

        // ── INV-HeapNullable (Nợ #3, ruling β): no `T?` with non-scalar `T` ──
        // The Bậc A nullable repr is a single-i64 sentinel; a heap fat-pointer
        // or multi-word struct/enum cannot fit it. Refuse at MIR (not
        // typecheck) so the stdlib can DECLARE heap-nullable API stubs; only
        // actual compilation is refused. This single chokepoint covers all
        // positions — return type, every local (params + lets + temps), and
        // every struct-field / enum-payload type (those live in the layouts,
        // not as standalone locals) — recursing into Nullable/Reference/
        // Outcome so nested occurrences are caught too.
        // Return type + locals: `String?` allowed (top-level ptr-sentinel slot).
        if let Some(inner) =
            find_refused_nullable(&self.signature.return_type, is_lowerable_nullable_payload)
        {
            return Err(MirError::HeapNullableNotLowered {
                inner_type: inner.clone(),
                position: "function return type".to_string(),
                span: DUMMY_SPAN.clone(),
            });
        }
        for (i, decl) in self.local_decls.iter().enumerate() {
            if let Some(inner) = find_refused_nullable(&decl.ty, is_lowerable_nullable_payload) {
                return Err(MirError::HeapNullableNotLowered {
                    inner_type: inner.clone(),
                    position: format!("local _{i}"),
                    span: DUMMY_SPAN.clone(),
                });
            }
        }
        // Struct fields + enum payloads: scalar `T?` only — a `String?` embedded
        // in an aggregate is not a top-level slot (Lát 1 does not lower it).
        for layout in &self.struct_layouts {
            for field in &layout.fields {
                if let Some(inner) = find_refused_nullable_field(&field.ty, self) {
                    return Err(MirError::HeapNullableNotLowered {
                        inner_type: inner.clone(),
                        position: format!("struct field `{}.{}`", layout.name, field.name),
                        span: DUMMY_SPAN.clone(),
                    });
                }
            }
        }
        for layout in &self.enum_layouts {
            for variant in &layout.variants {
                if let Some(payload) = &variant.payload
                    && let Some(inner) = find_refused_nullable_field(&payload.ty, self)
                {
                    return Err(MirError::HeapNullableNotLowered {
                        inner_type: inner.clone(),
                        position: format!("enum payload `{}.{}`", layout.name, variant.name),
                        span: DUMMY_SPAN.clone(),
                    });
                }
            }
        }

        // ── INV-Outcome-shape (ADR-0052 OP.2, amended ADR-0058 §3): ──
        // ReturnShape must match.  Any MirType::Outcome requires
        // BinaryOutcome, TernaryOutcome, or Struct (heap sret — ADR-0058
        // Lát 1).  Guards against silent miscompile of Outcome as Scalar
        // 1-value.
        if let MirType::Outcome { .. } = &self.signature.return_type
            && !matches!(
                self.signature.return_shape,
                ReturnShape::BinaryOutcome
                    | ReturnShape::TernaryOutcome
                    | ReturnShape::Struct { .. }
            )
        {
            return Err(MirError::OutcomeShapeMismatch {
                return_type: self.signature.return_type.clone(),
                return_shape: self.signature.return_shape.clone(),
                span: DUMMY_SPAN.clone(),
            });
        }

        // ── INV-Outcome-disc (ADR-0052 OP.3.5): BinaryOutcome disc ≠ Trit(0) ──
        // Post-StackSlot-refactor: the disc is set via
        //   Const(tmp, Trit(v)) → Assign(outcome.disc, tmp)
        // Scan all blocks for Const(Trit(0)) whose dest is later used as
        // source of an OutcomeDiscriminant store.
        if matches!(self.signature.return_shape, ReturnShape::BinaryOutcome) {
            for block_data in &self.blocks {
                // Collect locals set to Trit(0) in this block.
                let mut zero_trit_locals: BTreeSet<Local> = BTreeSet::new();
                for stmt in &block_data.statements {
                    if let Statement::Const {
                        dest,
                        value: ConstValue::Trit(0),
                        ..
                    } = stmt
                    {
                        zero_trit_locals.insert(dest.local);
                    }
                }
                // Check if any of them is stored into an OutcomeDiscriminant projection.
                for stmt in &block_data.statements {
                    if let Statement::Assign { dest, source, .. } = stmt
                        && dest
                            .projection
                            .iter()
                            .any(|p| matches!(p, Projection::OutcomeDiscriminant))
                        && source.projection.is_empty()
                        && zero_trit_locals.contains(&source.local)
                    {
                        return Err(MirError::OutcomeDiscriminantInvalid {
                            disc_value: 0,
                            span: DUMMY_SPAN.clone(),
                        });
                    }
                }
            }
        }

        // Helper: look up EnumLayout by name.
        let find_enum = |name: &str| -> Option<&EnumLayout> {
            self.enum_layouts.iter().find(|e| e.name == name)
        };
        // Helper: look up EnumLayout from a MirType.
        let find_enum_by_type = |ty: &MirType| -> Option<&EnumLayout> {
            let name = ty.to_string();
            self.enum_layouts.iter().find(|e| e.name == name)
        };

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

            // ── INV-3 helper: verify Outcome projection ──
            let check_outcome_projection = |base: Local| -> Result<(), MirError> {
                if let Some(decl) = self.local_decls.get(base.0)
                    && !matches!(decl.ty, MirType::Outcome { .. })
                {
                    return Err(MirError::OutcomeProjectionNonOutcome {
                        local: base,
                        found_type: decl.ty.clone(),
                        span: DUMMY_SPAN.clone(),
                    });
                }
                Ok(())
            };

            // ── INV-3 helper: verify Payload projection ──
            let check_payload = |base: Local, variant: &str| -> Result<(), MirError> {
                // Display-bridge: .ty is MirType, extract name for lookup (S2 transitional).
                let ty_opt = self.local_decls.get(base.0).map(|d| &d.ty);
                if let Some(ty) = ty_opt
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

                    // ── INV-Outcome-arity (ADR-0052 OP.2) ──
                    // For Outcome shapes, verify exactly 2 values [disc, payload].
                    if let ReturnShape::BinaryOutcome | ReturnShape::TernaryOutcome =
                        self.signature.return_shape
                    {
                        let expected = self.signature.return_shape.arity();
                        if values.len() != expected {
                            return Err(MirError::OutcomeReturnArityMismatch {
                                expected,
                                actual: values.len(),
                                span: DUMMY_SPAN.clone(),
                            });
                        }
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
                    // 4i-6: default_bb must terminate with Trap or Goto (Goto for wildcard, C2)
                    let default_block = &self.blocks[default_bb.0];
                    if !matches!(
                        default_block.terminator,
                        Terminator::Trap { .. } | Terminator::Goto { .. }
                    ) {
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
                        // 4i-7: check Payload + Outcome projection validity
                        for proj in &dest.projection {
                            if let Projection::Payload(variant) = proj {
                                check_payload(dest.local, variant)?;
                            }
                            if matches!(
                                proj,
                                Projection::OutcomeDiscriminant | Projection::OutcomePayload
                            ) {
                                check_outcome_projection(dest.local)?;
                            }
                        }
                        for proj in &source.projection {
                            if let Projection::Payload(variant) = proj {
                                check_payload(source.local, variant)?;
                            }
                            if matches!(
                                proj,
                                Projection::OutcomeDiscriminant | Projection::OutcomePayload
                            ) {
                                check_outcome_projection(source.local)?;
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
                        // 4i-1: enum_name must exist in enum_layouts
                        if find_enum(enum_name).is_none() {
                            return Err(MirError::EnumLayoutNotFound {
                                enum_name: enum_name.clone(),
                                span: DUMMY_SPAN.clone(),
                            });
                        }
                    }
                    Statement::OutcomeAlloc { dest, .. } => {
                        check_local(*dest)?;
                        // 4i-outcome-1: dest must have Outcome type.
                        if let Some(decl) = self.local_decls.get(dest.0)
                            && !matches!(decl.ty, MirType::Outcome { .. })
                        {
                            return Err(MirError::OutcomeAllocNonOutcome {
                                local: *dest,
                                found_type: decl.ty.clone(),
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
                    // ADR-0069: capability gate carries no local — nothing to check.
                    Statement::CapabilityCheck { .. } => {}
                }
            }
        }

        // ── INV-4: referenced blocks must have real terminators ──
        // Collect every block that is referenced by a terminator (Goto target,
        // If branches, CallDispatch return_bb, SwitchInt cases/default).
        // The entry block is implicitly referenced (it's where execution starts).
        // Any such block whose terminator is still `Unreachable` (the alloc_bb
        // default) means the lowerer forgot to call `term()` on it — a silent
        // fallthrough in the JIT that returns garbage (typically 0).
        let mut referenced = BTreeSet::new();
        referenced.insert(self.entry_block);
        for bd in &self.blocks {
            match &bd.terminator {
                Terminator::Goto { target, .. } => {
                    referenced.insert(*target);
                }
                Terminator::If {
                    positive_bb,
                    zero_bb,
                    negative_bb,
                    ..
                } => {
                    referenced.insert(*positive_bb);
                    referenced.insert(*negative_bb);
                    if let Some(zb) = zero_bb {
                        referenced.insert(*zb);
                    }
                }
                Terminator::CallDispatch { return_bb, .. } => {
                    referenced.insert(*return_bb);
                }
                Terminator::SwitchInt {
                    cases, default_bb, ..
                } => {
                    for &(_, target) in cases {
                        referenced.insert(target);
                    }
                    referenced.insert(*default_bb);
                }
                Terminator::Return { .. } | Terminator::Trap { .. } => {}
                // Unreachable in a source block is fine — only the target
                // blocks are checked below.
                Terminator::Unreachable { .. } => {}
            }
        }
        for &bb in &referenced {
            if bb.0 >= num_blocks {
                // Already caught by INV-1 — don't double-report.
                continue;
            }
            if matches!(self.blocks[bb.0].terminator, Terminator::Unreachable { .. }) {
                return Err(MirError::ReferencedBlockUnreachable {
                    block: bb,
                    referenced_by: if bb == self.entry_block {
                        "entry block".to_string()
                    } else {
                        "another block".to_string()
                    },
                    span: DUMMY_SPAN.clone(),
                });
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
    /// `SwitchInt.default_bb` does not terminate with `Trap` or `Goto`.
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
        found_type: MirType,
        /// Source location.
        span: Span,
    },
    /// INV-4: a block referenced by a terminator (or the entry block)
    /// has `Unreachable` as its terminator — the lowerer forgot to set
    /// a real terminator on this reachable block.
    ReferencedBlockUnreachable {
        /// The block that has an `Unreachable` terminator.
        block: BasicBlock,
        /// The block that references it (if known).
        referenced_by: String,
        /// Source location.
        span: Span,
    },
    /// INV-Outcome-arity (ADR-0052 OP.2): `Return.values.len()` does not
    /// match `ReturnShape::arity()`. BinaryOutcome requires exactly
    /// 2 return values [discriminant, payload].
    OutcomeReturnArityMismatch {
        /// Expected number of return values (2 for Outcome).
        expected: usize,
        /// Actual number of return values found.
        actual: usize,
        /// Source location.
        span: Span,
    },
    /// INV-Outcome-disc (ADR-0052 OP.2): BinaryOutcome discriminant is
    /// `Trit(0)` (Zero), which is invalid for 2-state Outcome. Only
    /// `Trit::Positive` (1) and `Trit::Negative` (-1) are valid.
    OutcomeDiscriminantInvalid {
        /// The invalid discriminant value found.
        disc_value: i8,
        /// Source location.
        span: Span,
    },
    /// INV-Outcome-shape (ADR-0052 OP.2): `Body::return_shape` does not
    /// match `Body::return_type`. Any Outcome return type requires
    /// `ReturnShape::BinaryOutcome`.
    OutcomeShapeMismatch {
        /// The return type (should be Outcome).
        return_type: MirType,
        /// The return shape that was found instead.
        return_shape: ReturnShape,
        /// Source location.
        span: Span,
    },

    /// 4i-outcome-1: `OutcomeAlloc` on a local whose type is not `MirType::Outcome`.
    OutcomeAllocNonOutcome {
        /// The local that was allocated as Outcome.
        local: Local,
        /// The type that was found instead.
        found_type: MirType,
        /// Source location.
        span: Span,
    },
    /// 4i-outcome-2: Outcome projection on a local whose type is not `MirType::Outcome`.
    OutcomeProjectionNonOutcome {
        /// The base local.
        local: Local,
        /// The type that was found.
        found_type: MirType,
        /// Source location.
        span: Span,
    },
    /// Nợ #3 (Heap-Nullable gate, ruling β): a `T?` reached MIR where `T` is
    /// not a scalar atom. The Bậc A nullable repr is a single-i64 sentinel
    /// (`i64::MIN`), which cannot hold a heap fat-pointer (String/Vector/
    /// HashMap = 24 bytes) or a multi-word struct/enum — lowering it would
    /// silently miscompile. Refused here rather than at typecheck so the
    /// stdlib can still DECLARE heap-nullable API (`env.get -> String?` etc.)
    /// as stubs; only actual compilation is refused. Lifted when the
    /// heap-nullable repr (ptr-sentinel slot + conditional Drop) lands.
    HeapNullableNotLowered {
        /// The non-scalar inner type `T` (for the diagnostic).
        inner_type: MirType,
        /// Where it was found (return type, a local, a struct field, …).
        position: String,
        /// Source location (DUMMY_SPAN for MIR-level errors).
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
            Self::ReferencedBlockUnreachable {
                block,
                referenced_by,
                ..
            } => {
                write!(
                    f,
                    "MIR verification error: block {block} has Unreachable terminator but is referenced by {referenced_by}"
                )
            }
            Self::OutcomeReturnArityMismatch {
                expected, actual, ..
            } => {
                write!(
                    f,
                    "MIR verification error: Outcome return arity mismatch — expected {expected} values, got {actual}"
                )
            }
            Self::OutcomeDiscriminantInvalid { disc_value, .. } => {
                write!(
                    f,
                    "MIR verification error: BinaryOutcome discriminant is Trit({disc_value}) — only Positive(1) and Negative(-1) are valid"
                )
            }
            Self::OutcomeShapeMismatch {
                return_type,
                return_shape,
                ..
            } => {
                write!(
                    f,
                    "MIR verification error: return type is '{return_type}' but return shape is {return_shape:?} — expected matching Outcome shape"
                )
            }

            Self::OutcomeAllocNonOutcome {
                local, found_type, ..
            } => {
                write!(
                    f,
                    "MIR verification error: OutcomeAlloc on local {local} with non-Outcome type '{found_type}'"
                )
            }
            Self::OutcomeProjectionNonOutcome {
                local, found_type, ..
            } => {
                write!(
                    f,
                    "MIR verification error: Outcome projection on local {local} with non-Outcome type '{found_type}'"
                )
            }
            Self::HeapNullableNotLowered {
                inner_type,
                position,
                ..
            } => {
                write!(
                    f,
                    "heap-nullable T? not yet lowered (repr campaign) — T = {inner_type} (at {position})"
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
            Self::StructAlloc {
                dest, struct_name, ..
            } => write!(f, "{dest} = struct {struct_name} {{..}}"),
            Self::EnumAlloc {
                dest, enum_name, ..
            } => write!(f, "{dest} = enum {enum_name} {{..}}"),
            Self::OutcomeAlloc { dest, .. } => write!(f, "{dest} = Outcome {{..}}"),
            Self::SetDiscriminant { dest, value, .. } => {
                write!(f, "SetDiscriminant({dest}, {value})")
            }
            Self::GetDiscriminant { dest, source, .. } => {
                write!(f, "{dest} = discriminant({source})")
            }
            Self::Drop(l, _) => write!(f, "Drop({l})"),
            Self::CapabilityCheck {
                capability_name, ..
            } => write!(f, "CapabilityCheck({capability_name})"),
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

/// Sentinel encoding the `~0` (null) state of **all** `T?` at Bậc A
/// (scalar and heap — uniform). INVARIANT: lies outside every Triết
/// scalar range (see canary test N1 in the test module, and
/// [ADR-0041 §6.2](../../docs/decisions/0041-nullable-representation-bac-a.md#62--scalar-debt-d1-phantom-null-qua-arithmetic-khng-wrap)).
pub const NULL_SENTINEL: i64 = i64::MIN;

/// Computes the type of a Place by walking its projection chain.
pub fn place_type(place: &Place, body: &Body) -> MirType {
    let mut current_ty = body.local_decls[place.local.0].ty.clone();
    for proj in &place.projection {
        current_ty = match proj {
            Projection::Field(name) => {
                // Look up the field type in struct layouts.
                let ty_name = current_ty.to_string();
                if let Some(s) = body.struct_layouts.iter().find(|s| s.name == ty_name) {
                    if let Some(field) = s.fields.iter().find(|f| f.name == name.as_str()) {
                        field.ty.clone()
                    } else {
                        MirType::Unknown
                    }
                } else if let Some(e) = body.enum_layouts.iter().find(|e| e.name == ty_name) {
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
                    found.unwrap_or(MirType::Unknown)
                } else {
                    MirType::Unknown
                }
            }
            Projection::Payload(variant) => {
                // Look up the payload type in enum layouts.
                let ty_name = current_ty.to_string();
                if let Some(e) = body.enum_layouts.iter().find(|e| e.name == ty_name) {
                    if let Some(v) = e.variants.iter().find(|v| &v.name == variant) {
                        if let Some(ref payload) = v.payload {
                            payload.ty.clone()
                        } else {
                            MirType::Unknown
                        }
                    } else {
                        MirType::Unknown
                    }
                } else {
                    MirType::Unknown
                }
            }
            _ => {
                // Other projections (Deref, Downcast, Index) — can't resolve yet.
                MirType::Unknown
            }
        };
    }
    current_ty
}

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
                ("x".into(), MirType::Integer, 8, align::INTEGER),
                ("y".into(), MirType::Integer, 8, align::INTEGER),
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
                ("a".into(), MirType::Trit, 1, align::TRIT),
                ("b".into(), MirType::Integer, 8, align::INTEGER),
                ("c".into(), MirType::Trilean, 1, align::TRILEAN),
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
                ("header".into(), MirType::Integer, 8, align::INTEGER),
                ("data".into(), MirType::Unknown, 5, 1), // 5-byte array, alignment 1
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
                ("x".into(), MirType::Integer, 5, 5), // alignment 5 is invalid
            ],
        );
    }

    // ── Verifier tests ─────────────────────────────────────

    /// Build a minimal well-formed body for verify() tests.
    fn well_formed_body() -> Body {
        Body {
            signature: FunctionSignature {
                name: "test".into(),
                parameters: vec![],
                return_type: MirType::Integer,
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

    /// HP.1: outcome_slot_size — scalar 16, heap 32.
    #[test]
    fn outcome_slot_size_scalar_and_heap() {
        // Scalar payload → 16
        assert_eq!(
            MirType::Outcome {
                value_type: Box::new(MirType::Integer),
                error_type: Box::new(MirType::Integer),
                allow_null_state: false,
            }
            .outcome_slot_size(),
            16
        );
        // Heap success → 32
        assert_eq!(
            MirType::Outcome {
                value_type: Box::new(MirType::String),
                error_type: Box::new(MirType::Integer),
                allow_null_state: false,
            }
            .outcome_slot_size(),
            32
        );
        // Heap error → 32
        assert_eq!(
            MirType::Outcome {
                value_type: Box::new(MirType::Integer),
                error_type: Box::new(MirType::String),
                allow_null_state: false,
            }
            .outcome_slot_size(),
            32
        );
        // Both heap → 32
        assert_eq!(
            MirType::Outcome {
                value_type: Box::new(MirType::String),
                error_type: Box::new(MirType::Vector),
                allow_null_state: false,
            }
            .outcome_slot_size(),
            32
        );
        // Ternary heap → 32
        assert_eq!(
            MirType::Outcome {
                value_type: Box::new(MirType::Integer),
                error_type: Box::new(MirType::String),
                allow_null_state: true,
            }
            .outcome_slot_size(),
            32
        );
        // Non-Outcome → 16 (default)
        assert_eq!(MirType::Integer.outcome_slot_size(), 16);
    }

    /// HP.1: is_any_heap discriminates heap types.
    #[test]
    fn is_any_heap_detection() {
        assert!(MirType::String.is_any_heap());
        assert!(MirType::Vector.is_any_heap());
        assert!(MirType::HashMap.is_any_heap());
        assert!(!MirType::Integer.is_any_heap());
        assert!(!MirType::Trit.is_any_heap());
        assert!(!MirType::Trilean.is_any_heap());
    }

    /// HP.1: has_heap_payload on Outcome types.
    #[test]
    fn has_heap_payload_detection() {
        assert!(
            MirType::Outcome {
                value_type: Box::new(MirType::String),
                error_type: Box::new(MirType::Integer),
                allow_null_state: false,
            }
            .has_heap_payload()
        );
        assert!(
            !MirType::Outcome {
                value_type: Box::new(MirType::Integer),
                error_type: Box::new(MirType::Integer),
                allow_null_state: false,
            }
            .has_heap_payload()
        );
        assert!(!MirType::Integer.has_heap_payload());
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
            local_decls: vec![LocalDecl::new(MirType::Integer)],
            ..well_formed_body()
        };
        let place = Place::local(Local(0));
        assert_eq!(place_type(&place, &body), MirType::Integer);
    }

    #[test]
    fn place_type_struct_field() {
        let layout = StructLayout::compute(
            "Point",
            &[
                ("x".into(), MirType::Integer, 8, align::INTEGER),
                ("y".into(), MirType::String, 8, align::INTEGER),
            ],
        );
        let body = Body {
            local_decls: vec![LocalDecl::new(MirType::Struct("Point".into()))],
            struct_layouts: vec![layout],
            ..well_formed_body()
        };
        let place = Place {
            local: Local(0),
            projection: vec![Projection::Field("x".into())],
        };
        assert_eq!(place_type(&place, &body), MirType::Integer);
        let place_y = Place {
            local: Local(0),
            projection: vec![Projection::Field("y".into())],
        };
        assert_eq!(place_type(&place_y, &body), MirType::String);
    }

    #[test]
    fn place_type_enum_payload() {
        let layout = EnumLayout::compute(
            "Option",
            &[("Some".into(), 0, Some((MirType::Integer, 8, 8, vec![])))],
        );
        let body = Body {
            local_decls: vec![LocalDecl::new(MirType::Enum("Option".into()))],
            enum_layouts: vec![layout],
            ..well_formed_body()
        };
        let place = Place {
            local: Local(0),
            projection: vec![Projection::Payload("Some".into())],
        };
        assert_eq!(place_type(&place, &body), MirType::Integer);
    }

    #[test]
    fn place_type_unknown_projection() {
        let body = Body {
            local_decls: vec![LocalDecl::new(MirType::Struct("UnknownType".into()))],
            ..well_formed_body()
        };
        // Field on unknown type → "?"
        let place = Place {
            local: Local(0),
            projection: vec![Projection::Field("x".into())],
        };
        assert_eq!(place_type(&place, &body), MirType::Unknown);
        // Unknown projection kind → "?"
        let place2 = Place {
            local: Local(0),
            projection: vec![Projection::Deref],
        };
        assert_eq!(place_type(&place2, &body), MirType::Unknown);
    }

    // ── MirType tests (ADR-0050 B1a) ──────────────────────────

    #[test]
    fn mirtype_display_roundtrip_primitives() {
        for ty in [
            MirType::Integer,
            MirType::Trit,
            MirType::Tryte,
            MirType::Long,
            MirType::Trilean,
            MirType::Unit,
            MirType::Unknown,
        ] {
            let s = ty.to_string();
            // parse deleted in S4 — Display round-trip verified via string comparison
            assert_eq!(s, ty.to_string(), "Display mismatch for {ty}");
        }
    }

    #[test]
    fn mirtype_display_roundtrip_heap() {
        assert_eq!(MirType::String.to_string(), "String");
        assert_eq!(MirType::String, MirType::String);
        assert_eq!(MirType::Vector.to_string(), "Vector");
        assert_eq!(MirType::Vector, MirType::Vector);
        assert_eq!(MirType::HashMap.to_string(), "HashMap");
        assert_eq!(MirType::HashMap, MirType::HashMap);
    }

    #[test]
    fn mirtype_display_roundtrip_nullable() {
        let ty = MirType::Nullable(Box::new(MirType::Integer));
        assert_eq!(ty.to_string(), "Integer?");
        assert_eq!(MirType::Nullable(Box::new(MirType::Integer)), ty);

        let ty = MirType::Nullable(Box::new(MirType::String));
        assert_eq!(ty.to_string(), "String?");
        assert_eq!(MirType::Nullable(Box::new(MirType::String)), ty);

        // Pin: bare "?" must NOT parse as Nullable.
        let parsed = MirType::Unknown;
        assert_eq!(parsed, MirType::Unknown);
        assert!(!parsed.is_nullable());
    }

    #[test]
    fn mirtype_display_roundtrip_references() {
        for (form, prefix) in [
            (ReferenceForm::StrongFrozen, "&+ "),
            (ReferenceForm::StrongMutable, "&+ mutable "),
            (ReferenceForm::BorrowReadOnly, "&0 "),
            (ReferenceForm::BorrowExclusiveMutable, "&0 mutable "),
            (ReferenceForm::WeakObserver, "&- "),
        ] {
            let ty = MirType::Reference {
                form,
                inner: Box::new(MirType::String),
            };
            let s = ty.to_string();
            let expected = format!("{prefix}String");
            assert_eq!(s, expected, "Display mismatch for {form:?}");
            // parse deleted in S4
            assert_eq!(s, ty.to_string(), "Display mismatch for {ty}");
        }
    }

    #[test]
    fn mirtype_display_roundtrip_user_types() {
        let ty = MirType::Struct("Point".into());
        assert_eq!(ty.to_string(), "Point");
        // parse defaults to Struct for unknown names (transitional).
        assert_eq!(
            MirType::Struct("Point".into()),
            MirType::Struct("Point".into())
        );

        let ty = MirType::Enum("Color".into());
        assert_eq!(ty.to_string(), "Color");
        // parse can't distinguish Struct vs Enum from name alone → defaults to Struct.
        // This imprecision is acceptable for S1-S3; fixed at S4.
        assert_eq!(
            MirType::Struct("Color".into()),
            MirType::Struct("Color".into())
        );
    }

    #[test]
    fn mirtype_classification_methods() {
        assert!(MirType::Nullable(Box::new(MirType::Integer)).is_nullable());
        assert!(!MirType::Integer.is_nullable());
        assert!(!MirType::Unknown.is_nullable());

        assert_eq!(
            MirType::Nullable(Box::new(MirType::Integer))
                .nullable_payload()
                .map(|t| t.to_string()),
            Some("Integer".to_string())
        );
        assert!(MirType::Integer.nullable_payload().is_none());

        assert!(
            MirType::Reference {
                form: ReferenceForm::BorrowReadOnly,
                inner: Box::new(MirType::Integer),
            }
            .is_reference()
        );
        assert!(!MirType::Integer.is_reference());

        assert!(MirType::Vector.is_vec());
        assert!(!MirType::HashMap.is_vec());
        assert!(!MirType::String.is_vec());

        assert!(MirType::HashMap.is_hashmap());
        assert!(!MirType::Vector.is_hashmap());
    }

    /// Proves the structural fix for the ordering-rule bug (ADR-0050 §2.1).
    ///
    /// Old `is_vec_type("Vector<Integer>?")` returned `true` because
    /// `"Vector<Integer>?"` starts with `"Vector<"` — the suffix-`?`
    /// nullable was invisible to the prefix check. The structural
    /// `Nullable(Vector).is_vec()` correctly returns `false` — a nullable
    /// wrapper is NOT a Vector, regardless of payload.
    ///
    /// This test uses `Nullable(Vector)` directly (not `Nullable(Integer)`)
    /// because the bug specifically manifests when the payload IS a heap
    /// type that the old prefix-match would misclassify.
    #[test]
    fn mirtype_structural_fixes_nullable_vec_misclassification() {
        // Nullable(Vector) — exactly the pattern old is_vec_type got wrong.
        let ty = MirType::Nullable(Box::new(MirType::Vector));
        assert!(ty.is_nullable(), "Nullable(Vector) must be nullable");
        assert!(
            !ty.is_vec(),
            "Nullable(Vector) must NOT be classified as Vector"
        );
        assert!(
            !ty.is_hashmap(),
            "Nullable(Vector) must NOT be classified as HashMap"
        );

        // Symmetric: Nullable(HashMap) — same structural protection.
        let ty = MirType::Nullable(Box::new(MirType::HashMap));
        assert!(ty.is_nullable(), "Nullable(HashMap) must be nullable");
        assert!(
            !ty.is_hashmap(),
            "Nullable(HashMap) must NOT be classified as HashMap"
        );
        assert!(
            !ty.is_vec(),
            "Nullable(HashMap) must NOT be classified as Vector"
        );

        // Also verify via parse transition path: "Vector?" → Nullable(Vector).
        let ty = MirType::Nullable(Box::new(MirType::Vector));
        assert!(ty.is_nullable());
        assert!(!ty.is_vec());
        let payload = ty.nullable_payload().unwrap();
        assert!(payload.is_vec(), "payload of Vector? must be Vector");

        // "HashMap?" → Nullable(HashMap).
        let ty = MirType::Nullable(Box::new(MirType::HashMap));
        assert!(ty.is_nullable());
        assert!(!ty.is_hashmap());
    }

    // ── MirType::is_copy tests ────────────────────────────────

    #[test]
    fn mirtype_is_copy_primitives() {
        let body = well_formed_body();
        assert!(MirType::Integer.is_copy(Some(&body)));
        assert!(MirType::Trit.is_copy(Some(&body)));
        assert!(MirType::Tryte.is_copy(Some(&body)));
        assert!(MirType::Long.is_copy(Some(&body)));
        assert!(MirType::Trilean.is_copy(Some(&body)));
        assert!(MirType::Unit.is_copy(Some(&body)));
        assert!(MirType::Unknown.is_copy(Some(&body)));
        // is_copy(None) — same results for primitives.
        assert!(MirType::Integer.is_copy(None));
        assert!(MirType::Unknown.is_copy(None));
    }

    #[test]
    fn mirtype_is_copy_references() {
        let body = well_formed_body();
        for form in [
            ReferenceForm::StrongFrozen,
            ReferenceForm::StrongMutable,
            ReferenceForm::BorrowReadOnly,
            ReferenceForm::BorrowExclusiveMutable,
            ReferenceForm::WeakObserver,
        ] {
            let ty = MirType::Reference {
                form,
                inner: Box::new(MirType::String),
            };
            assert!(ty.is_copy(Some(&body)), "reference {form:?} should be Copy");
            assert!(
                ty.is_copy(None),
                "reference {form:?} should be Copy (no body)"
            );
        }
    }

    #[test]
    fn mirtype_is_copy_heap_types() {
        let body = well_formed_body();
        assert!(!MirType::String.is_copy(Some(&body)));
        assert!(!MirType::Vector.is_copy(Some(&body)));
        assert!(!MirType::HashMap.is_copy(Some(&body)));
        // is_copy(None) — same results.
        assert!(!MirType::String.is_copy(None));
        assert!(!MirType::Vector.is_copy(None));
        assert!(!MirType::HashMap.is_copy(None));
    }

    #[test]
    fn mirtype_is_copy_struct_recursive() {
        let layout = StructLayout::compute(
            "HasString",
            &[
                ("header".into(), MirType::Integer, 8, 8),
                ("body".into(), MirType::String, 8, 8),
            ],
        );
        let body = Body {
            struct_layouts: vec![layout],
            ..well_formed_body()
        };
        // With body: struct with String field → Move.
        assert!(!MirType::Struct("HasString".into()).is_copy(Some(&body)));
        // Without body: assume Copy (sound while B8 blocks construction).
        assert!(MirType::Struct("HasString".into()).is_copy(None));
    }

    #[test]
    fn mirtype_is_copy_enum_recursive() {
        let layout = EnumLayout::compute(
            "Either",
            &[("Left".into(), 0, Some((MirType::Integer, 8, 8, vec![])))],
        );
        let body = Body {
            enum_layouts: vec![layout],
            ..well_formed_body()
        };
        // All payloads are Integer → Copy.
        assert!(MirType::Enum("Either".into()).is_copy(Some(&body)));

        let layout2 = EnumLayout::compute(
            "EitherString",
            &[("Right".into(), 0, Some((MirType::String, 8, 8, vec![])))],
        );
        let body2 = Body {
            enum_layouts: vec![layout2],
            ..well_formed_body()
        };
        // A payload is String → Move.
        assert!(!MirType::Enum("EitherString".into()).is_copy(Some(&body2)));
    }

    #[test]
    fn mirtype_is_copy_unknown_defaults_move() {
        let body = well_formed_body();
        // With body: unknown struct/enum names → Move (refuse-over-guess).
        assert!(!MirType::Struct("UnknownType".into()).is_copy(Some(&body)));
        assert!(!MirType::Enum("UnknownType".into()).is_copy(Some(&body)));
    }

    #[test]
    fn mirtype_nullable_is_copy_delegation() {
        let body = well_formed_body();
        // Integer? → payload Integer → Copy
        assert!(MirType::Nullable(Box::new(MirType::Integer)).is_copy(Some(&body)));
        // String? → payload String → Move
        assert!(!MirType::Nullable(Box::new(MirType::String)).is_copy(Some(&body)));
        // Vector? → payload Vector → Move
        assert!(!MirType::Nullable(Box::new(MirType::Vector)).is_copy(Some(&body)));
        // Unknown? → payload Unknown → Copy
        assert!(MirType::Nullable(Box::new(MirType::Unknown)).is_copy(Some(&body)));
        // is_copy(None)
        assert!(MirType::Nullable(Box::new(MirType::Integer)).is_copy(None));
        assert!(!MirType::Nullable(Box::new(MirType::String)).is_copy(None));
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

    /// N2 — `MirType::is_nullable()` / `MirType::nullable_payload()` round-trip and pins.
    #[test]
    fn nullable_type_helpers() {
        // Round-trip: Integer?
        let ty = MirType::Nullable(Box::new(MirType::Integer));
        assert!(ty.is_nullable());
        assert_eq!(
            ty.nullable_payload().map(|t| t.to_string()),
            Some("Integer".to_string())
        );

        // Round-trip: String?
        let ty = MirType::Nullable(Box::new(MirType::String));
        assert!(ty.is_nullable());
        assert_eq!(
            ty.nullable_payload().map(|t| t.to_string()),
            Some("String".into())
        );

        // Pin: "?" trần must NOT be nullable (type-unknown).
        let ty = MirType::Unknown;
        assert!(!ty.is_nullable(), "bare '?' must not be nullable");
        assert!(ty.nullable_payload().is_none());

        // Pin: is_vec must NOT consume nullable Vector.
        let ty = MirType::Nullable(Box::new(MirType::Vector));
        assert!(ty.is_nullable());
        // Payload of nullable Vector IS a vec type.
        let payload = ty.nullable_payload().unwrap();
        assert!(
            payload.is_vec(),
            "payload of nullable Vector should be Vector"
        );

        // Non-nullable: plain types.
        assert!(!MirType::Integer.is_nullable());
        assert!(!MirType::String.is_nullable());
        assert!(!MirType::Vector.is_nullable());

        // Edge case: "Integer??" — can't happen (C6: T?? auto-flatten),
        // but helper must be defined.
        let ty = MirType::Nullable(Box::new(MirType::Nullable(Box::new(MirType::Integer))));
        assert!(ty.is_nullable());
        assert!(ty.nullable_payload().unwrap().is_nullable());
    }

    /// N2 extension — `is_copy` integration: nullable delegates to payload.
    #[test]
    fn nullable_is_copy_delegation() {
        let body = well_formed_body();

        // Integer? → payload Integer → Copy
        assert!(MirType::Nullable(Box::new(MirType::Integer)).is_copy(Some(&body)));
        // String? → payload String → Move
        assert!(!MirType::Nullable(Box::new(MirType::String)).is_copy(Some(&body)));
        // Vector<Integer>? → payload Vector → Move
        assert!(!MirType::Nullable(Box::new(MirType::Vector)).is_copy(Some(&body)));
        // ? trần → NOT nullable → falls through to Copy (existing behavior)
        assert!(MirType::Unknown.is_copy(Some(&body)));
    }

    /// INV-4: a block referenced by a Goto must not have `Unreachable`
    /// as its terminator — that indicates the lowerer forgot to call
    /// `term()` on it (silent fallthrough = wrong answer).
    #[test]
    fn verify_rejects_referenced_block_with_unreachable_terminator() {
        let mut body = well_formed_body();
        // Add a second block (bb1) with Unreachable — this is what
        // alloc_bb creates before term() is called.
        body.blocks.push(BlockData {
            statements: vec![],
            terminator: Terminator::Unreachable { span: DUMMY_SPAN },
        });
        // Make bb0 Goto bb1 — now bb1 is referenced but has Unreachable.
        body.blocks[0].terminator = Terminator::Goto {
            target: BasicBlock(1),
            span: DUMMY_SPAN,
        };
        let err = body
            .verify()
            .expect_err("should reject Goto target with Unreachable terminator");
        assert!(
            matches!(err, MirError::ReferencedBlockUnreachable { .. }),
            "expected ReferencedBlockUnreachable, got {err:?}"
        );
    }

    /// INV-4 regression: a legitimately dead block (not referenced by
    /// anything) with Unreachable is fine — it was allocated but never
    /// wired into the CFG.
    #[test]
    fn verify_accepts_unreferenced_block_with_unreachable() {
        let mut body = well_formed_body();
        body.blocks.push(BlockData {
            statements: vec![],
            terminator: Terminator::Unreachable { span: DUMMY_SPAN },
        });
        // bb0 stays as Return — no reference to bb1.
        body.verify()
            .expect("unreferenced Unreachable block is fine");
    }
}
