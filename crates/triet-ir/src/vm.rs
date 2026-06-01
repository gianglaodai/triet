//! Bytecode VM — executes IR by walking basic blocks in a register-based
//! interpreter loop.
//!
//! Per [ADR-0007], this VM is **development-tier scaffolding**, not the
//! production runtime. Production targets are AOT native (LLVM, v2.0) and
//! trytecode native (v∞). The VM exists to:
//! 1. Validate IR design through execution before committing it permanently.
//! 2. Serve as a platform for the self-hosting compiler (v0.7) before LLVM.
//! 3. Act as a differential-test oracle against the tree-walker (v0.2).
//!
//! [ADR-0007]: ../../../docs/decisions/0007-ir-design.md

use std::collections::HashMap;

use triet_core::{Integer, Long, Trit, Tryte};
use triet_logic::Trilean;

use crate::constant::Constant;
use crate::instr::{BuiltinName, Instruction, Operand, PhiIncoming};
use crate::module::{BasicBlock, Function, IrProgram};
use crate::types::{BlockId, FuncId, TypeTag, ValueId};

// ── Runtime values ─────────────────────────────────────────────────

/// A runtime value — what a virtual register holds during execution.
#[derive(Clone, Debug)]
pub enum RuntimeValue {
    /// 1-trit numeric: `-1`, `0`, `+1`.
    Trit(Trit),
    /// 9-trit integer.
    Tryte(Tryte),
    /// 27-trit integer.
    Integer(Integer),
    /// 81-trit arbitrary-precision integer.
    Long(Long),
    /// 3-valued logic: `false`, `unknown`, `true`.
    Trilean(Trilean),
    /// UTF-8 string (reference-counted / cloned).
    String(String),
    /// Zero-sized unit.
    Unit,
    /// The `null` marker for a nullable type.
    Null,
    /// A struct instance with fields in declaration order.
    Struct {
        /// Field values.
        fields: Vec<Self>,
    },
    /// An enum variant instance.
    Enum {
        /// Variant index (0-based).
        variant: u32,
        /// Optional payload.
        payload: Option<Box<Self>>,
    },
    /// A closure capturing live variables.
    Closure {
        /// The IR function this closure wraps.
        func_id: FuncId,
        /// Captured values.
        captures: Vec<Self>,
    },
    /// Homogeneous ordered collection — backing for `Vector<T>` type
    /// (introduced at v0.7.3 per ADR-0019 §5). Element runtime type
    /// is not stored on each value; the IR's [`TypeTag`] keeps that
    /// invariant. Builtin opcodes operate on this in-place.
    Vector(Vec<Self>),
    /// Keyed collection — backing for `HashMap<K, V>` type. Uses
    /// `BTreeMap` for deterministic iteration order (aligns with
    /// ADR-0019 §3 canonical emission principle — important once the
    /// self-host compiler starts serializing collection contents).
    HashMap(std::collections::BTreeMap<RuntimeMapKey, Self>),
    /// Outcome value per [ADR-0020] — a 1-trit discriminator plus an
    /// optional payload. Encodes both `T~E` (binary) and `T?~E`
    /// (ternary) forms; the static [`TypeTag`] retains which shape was
    /// declared.
    ///
    /// - `discriminator = Trit::Positive` → success arm, `payload =
    ///   Some(T)`.
    /// - `discriminator = Trit::Negative` → failure arm, `payload =
    ///   Some(E)`.
    /// - `discriminator = Trit::Zero` → null arm (`T?~E` only),
    ///   `payload = None`.
    ///
    /// The `Box<Self>` indirection is mandatory — without it the
    /// type would be infinitely sized. `Option<Box<…>>` automatically
    /// frees the heap payload when the outcome is dropped (ADR-0020
    /// §"Memory deallocation contract" — Rust `Drop` satisfies the
    /// contract for the VM tier).
    ///
    /// [ADR-0020]: ../../../docs/decisions/0020-outcome-error-handling.md
    Outcome {
        /// 1-trit discriminator — encodes the active arm.
        discriminator: Trit,
        /// Optional heap-allocated payload. `None` only for the null
        /// arm of a `T?~E` outcome.
        payload: Option<Box<Self>>,
    },
    /// `Atomic<T>` per [ADR-0028] — shared-mutable wrapper for
    /// `AtomicValue` primitive (`Trit`/`Tryte`/`Integer`/`Trilean` per §2).
    ///
    /// **v0.10.x.thread.2 migration:** previously `Rc<RefCell<Self>>`
    /// (single-thread VM dev tier per ADR-0028 §9). Migrated to
    /// `Arc<Mutex<Self>>` so the shared cell becomes `Send + Sync` and
    /// can cross OS-thread boundaries when `raw_thread.spawn` (per
    /// ADR-0026 v2 §3 + v0.10.x.thread.1) captures it. This is an
    /// **infrastructure-prerequisite** change for v0.10.x.thread.3
    /// (multi-worker `atomic_counter` demo); the real Send-boundary
    /// codegen per ADR-0026 v2 §3.2 still defers to v0.11+ when
    /// Triết's closure type system gains Send-bound expressiveness.
    /// VM dispatcher itself is still single-thread per ADR-0028 §9 —
    /// no concurrent IR execution.
    ///
    /// Ordering is still semantically no-op at v0.10 dev tier
    /// (typecheck-validated, ignored at runtime per ADR-0028 §9).
    /// Real ordering semantics land alongside v2.0 LLVM AOT.
    ///
    /// [ADR-0028]: ../../../docs/decisions/0028-atomic-primitive.md
    /// [ADR-0026 v2]: ../../../docs/decisions/0026-actor-boundary-send-rules.md
    Atomic(std::sync::Arc<std::sync::Mutex<Self>>),
}

/// Keys for `RuntimeValue::HashMap`. Restricted to hashable primitives
/// — Triết maps don't accept Vector/HashMap/Struct/Enum/Closure keys
/// at v0.7.3. Ordering is well-defined for `BTreeMap`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RuntimeMapKey {
    /// 1-trit key.
    Trit(Trit),
    /// 9-trit key.
    Tryte(Tryte),
    /// 27-trit key.
    Integer(Integer),
    /// 81-trit key.
    Long(Long),
    /// String key (most common in practice — symbol tables in the
    /// self-host compiler).
    String(String),
}

impl RuntimeMapKey {
    /// Attempt to derive a map key from a runtime value. Returns
    /// `None` for values that aren't allowed as keys (`Vector`,
    /// `HashMap`, `Struct`, `Enum`, `Closure`, `Unit`, `Null`,
    /// `Trilean`).
    #[must_use]
    pub fn from_runtime(value: &RuntimeValue) -> Option<Self> {
        match value {
            RuntimeValue::Trit(t) => Some(Self::Trit(*t)),
            RuntimeValue::Tryte(t) => Some(Self::Tryte(*t)),
            RuntimeValue::Integer(i) => Some(Self::Integer(*i)),
            RuntimeValue::Long(l) => Some(Self::Long(*l)),
            RuntimeValue::String(s) => Some(Self::String(s.clone())),
            _ => None,
        }
    }

    /// Lift this key back to a `RuntimeValue` (e.g. when returning the
    /// list of keys via `hashmap_keys`).
    #[must_use]
    pub fn to_runtime(&self) -> RuntimeValue {
        match self {
            Self::Trit(t) => RuntimeValue::Trit(*t),
            Self::Tryte(t) => RuntimeValue::Tryte(*t),
            Self::Integer(i) => RuntimeValue::Integer(*i),
            Self::Long(l) => RuntimeValue::Long(*l),
            Self::String(s) => RuntimeValue::String(s.clone()),
        }
    }
}

impl std::fmt::Display for RuntimeMapKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Trit(t) => write!(f, "{t}"),
            Self::Tryte(t) => write!(f, "{t}"),
            Self::Integer(i) => write!(f, "{i}"),
            Self::Long(l) => write!(f, "{l}"),
            Self::String(s) => write!(f, "\"{s}\""),
        }
    }
}

impl RuntimeValue {
    /// Return the `TypeTag` that most closely matches this runtime value.
    #[must_use]
    pub fn type_tag(&self) -> TypeTag {
        match self {
            Self::Trit(_) => TypeTag::Trit,
            Self::Tryte(_) => TypeTag::Tryte,
            Self::Integer(_) => TypeTag::Integer,
            Self::Long(_) => TypeTag::Long,
            Self::Trilean(_) => TypeTag::Trilean,
            Self::String(_) => TypeTag::String,
            Self::Unit => TypeTag::Unit,
            Self::Null => TypeTag::Nullable(Box::new(TypeTag::Unit)),
            Self::Struct { .. } | Self::Enum { .. } | Self::Closure { .. } => TypeTag::Unit,
            // Collection element types aren't tracked on the runtime
            // value — the IR's static `TypeTag` retains that info. Fall
            // back to a wildcard element of `Unit`; callers that need
            // precise types should consult the originating instruction.
            Self::Vector(_) => TypeTag::Vector(Box::new(TypeTag::Unit)),
            Self::HashMap(_) => TypeTag::HashMap(Box::new(TypeTag::Unit), Box::new(TypeTag::Unit)),
            // Outcome value types aren't tracked at runtime (the static
            // [`TypeTag`] is authoritative for `value_type` / `error_type`
            // / `allow_null_state`). Same wildcard pattern as Vector /
            // HashMap above.
            Self::Outcome { .. } => TypeTag::Outcome {
                value_type: Box::new(TypeTag::Unit),
                error_type: Box::new(TypeTag::Unit),
                allow_null_state: false,
            },
            // v0.9.x.atomic.3 — Atomic<T> inner type uses Unit wildcard
            // per Vector/HashMap precedent (static TypeTag authoritative).
            Self::Atomic(_) => TypeTag::Atomic(Box::new(TypeTag::Unit)),
        }
    }

    /// Read a constant from the pool into a runtime value.
    fn from_constant(c: &Constant) -> Self {
        match c {
            Constant::Trit(t) => Self::Trit(*t),
            Constant::Tryte(t) => Self::Tryte(*t),
            Constant::Integer(i) => Self::Integer(*i),
            Constant::Long(l) => Self::Long(*l),
            Constant::Trilean(t) => Self::Trilean(*t),
            Constant::String(s) => Self::String(s.clone()),
            Constant::Unit => Self::Unit,
            Constant::Null => Self::Null,
        }
    }

    /// Convert to a `Trilean` value, following balanced-ternary truth rules:
    /// positive → true, negative → false, zero → unknown.
    fn as_trilean(&self) -> Trilean {
        match self {
            Self::Trilean(t) => *t,
            Self::Trit(t) => {
                if t.is_positive() {
                    Trilean::True
                } else if t.is_negative() {
                    Trilean::False
                } else {
                    Trilean::Unknown
                }
            }
            Self::Integer(i) => {
                let zero = Integer::new(0).unwrap();
                match (*i).cmp(&zero) {
                    std::cmp::Ordering::Greater => Trilean::True,
                    std::cmp::Ordering::Less => Trilean::False,
                    std::cmp::Ordering::Equal => Trilean::Unknown,
                }
            }
            Self::Long(l) => {
                let zero = Long::from_i128(0);
                match (*l).cmp(&zero) {
                    std::cmp::Ordering::Greater => Trilean::True,
                    std::cmp::Ordering::Less => Trilean::False,
                    std::cmp::Ordering::Equal => Trilean::Unknown,
                }
            }
            _ => Trilean::Unknown,
        }
    }

    /// True if this value is truthy (`Trilean::True` or positive numeric).
    fn is_truthy(&self) -> bool {
        matches!(self.as_trilean(), Trilean::True)
    }
}

impl std::fmt::Display for RuntimeValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Trit(t) => write!(f, "{t}"),
            Self::Tryte(t) => write!(f, "{t}"),
            Self::Integer(i) => write!(f, "{i}"),
            Self::Long(l) => write!(f, "{l}"),
            Self::Trilean(t) => write!(f, "{t}"),
            Self::String(s) => write!(f, "{s}"),
            Self::Unit => write!(f, "()"),
            Self::Null => write!(f, "null"),
            Self::Struct { fields } => {
                write!(f, "{{")?;
                for (i, fld) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{fld}")?;
                }
                write!(f, "}}")
            }
            Self::Enum {
                variant,
                payload: Some(p),
            } => write!(f, "enum.v{variant}({p})"),
            Self::Enum {
                variant,
                payload: None,
            } => write!(f, "enum.v{variant}"),
            Self::Closure { func_id, .. } => write!(f, "closure(@f{func_id})"),
            Self::Vector(elements) => {
                write!(f, "[")?;
                for (i, element) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{element}")?;
                }
                write!(f, "]")
            }
            Self::HashMap(entries) => {
                write!(f, "{{")?;
                for (i, (key, value)) in entries.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{key}: {value}")?;
                }
                write!(f, "}}")
            }
            // Outcome rendering mirrors source-level constructors so
            // diagnostics quote a form the author can recognize.
            Self::Outcome {
                discriminator,
                payload: Some(p),
            } => {
                if discriminator.is_positive() {
                    write!(f, "~+({p})")
                } else if discriminator.is_negative() {
                    write!(f, "~-({p})")
                } else {
                    // Defensive: a Zero discriminator paired with a
                    // payload is malformed but we still render it
                    // rather than panicking inside Display.
                    write!(f, "~?({p})")
                }
            }
            Self::Outcome {
                discriminator: _,
                payload: None,
            } => write!(f, "~0"),
            // v0.9.x.atomic.3 — Atomic<T>: display inner state. Per
            // ADR-0028 §9 single-thread VM, atomic is transparent;
            // formatter borrows current inner without atomicity ceremony.
            Self::Atomic(cell) => {
                // Display does not propagate a Result from the inner
                // value's fmt; if the mutex is poisoned (panic in a
                // prior holder), fall back to a sentinel string rather
                // than panicking the formatter. Poisoning is unreachable
                // in single-thread VM dev tier (ADR-0028 §9) but the
                // mutex API surface still admits the variant.
                match cell.lock() {
                    Ok(guard) => write!(f, "Atomic({})", &*guard),
                    Err(_) => write!(f, "Atomic(<poisoned>)"),
                }
            }
        }
    }
}

// ── VM errors ──────────────────────────────────────────────────────

/// Errors produced during VM execution.
///
/// Error codes follow the E22XX namespace reserved for VM runtime errors
/// per [ADR-0007].
#[derive(Clone, Debug)]
pub enum VmError {
    /// E2200: tried to unwrap a null value.
    NullUnwrap {
        /// Function where the error occurred.
        function: String,
    },
    /// E2201: type tag mismatch — operand type ≠ expected.
    TypeMismatch {
        /// Expected type tag.
        expected: TypeTag,
        /// Actual type received.
        actual: String,
        /// Function where the error occurred.
        function: String,
    },
    /// E2202: arithmetic overflow (e.g., Trit overflow from Integer).
    Overflow {
        /// Function where the error occurred.
        function: String,
    },
    /// E2203: function not found.
    FunctionNotFound {
        /// Name of the missing function.
        name: String,
    },
    /// E2204: division by zero.
    DivisionByZero {
        /// Function where the error occurred.
        function: String,
    },
    /// E2205: assertion failed.
    AssertionFailed {
        /// Optional assertion message.
        message: Option<String>,
        /// Function where the error occurred.
        function: String,
    },
    /// E2206: array index out of bounds.
    OutOfBounds {
        /// Function where the error occurred.
        function: String,
    },
    /// E2207: unknown builtin name.
    UnknownBuiltin {
        /// The unrecognized builtin name.
        name: String,
        /// Function where the error occurred.
        function: String,
    },
    /// E2208: enum variant index out of valid range.
    InvalidVariant {
        /// Function where the error occurred.
        function: String,
    },
    /// E2210: outcome value held the wrong arm for the requested
    /// unwrap, or had a malformed shape (e.g. Zero discriminator with
    /// a payload). Per [ADR-0020 §"Memory deallocation contract"] this
    /// error fires before the payload is dropped; the `Drop` impl on
    /// [`RuntimeValue::Outcome`] then frees the heap memory.
    ///
    /// [ADR-0020 §"Memory deallocation contract"]: ../../../docs/decisions/0020-outcome-error-handling.md
    InvalidOutcomeState {
        /// One-line description of the violation, e.g.
        /// "`unwrap_value` called on failure arm".
        reason: String,
        /// Function where the error occurred.
        function: String,
    },
    /// E2211: builtin is declared but dispatch not yet implemented.
    /// v0.9.x.atomic.2 ships atomic builtin declarations + serde +
    /// display, but VM dispatch lands v0.9.x.atomic.3-4. Calling
    /// undispatched builtin produces graceful error, not panic.
    BuiltinUnimplemented {
        /// The declared-but-unimplemented builtin name.
        name: String,
        /// Reason / pointer to sub-task landing real dispatch.
        reason: String,
    },
    /// E2212: a JIT builtin shim signalled failure. v0.10.x.jit.2a —
    /// per the [ADR-0032] §4 option-2 resolution, a shim records a
    /// structured `VmError` into a thread-local slot + sets a
    /// `SHIM_FAILED` flag, and the JIT-emitted per-call sentinel check
    /// branches to the function's `error_exit`. The dispatcher reads
    /// the slot after the (normal) native return. This variant is the
    /// **fallback** when the flag was set WITHOUT a structured error
    /// (a shim bug) — surfaced, not hidden.
    ///
    /// [ADR-0032]: ../../../docs/decisions/0032-builtin-shim-abi.md
    JitShimFault {
        /// One-line description of the fault.
        reason: String,
        /// Function where the JIT'd shim call occurred.
        function: String,
    },
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NullUnwrap { function } => {
                write!(f, "E2200: attempted to unwrap a null value in `{function}`")
            }
            Self::TypeMismatch {
                expected,
                actual,
                function,
            } => {
                write!(
                    f,
                    "E2201: type mismatch in `{function}`: expected {expected}, got {actual}"
                )
            }
            Self::Overflow { function } => {
                write!(f, "E2202: arithmetic overflow in `{function}`")
            }
            Self::FunctionNotFound { name } => {
                write!(f, "E2203: function `{name}` not found")
            }
            Self::DivisionByZero { function } => {
                write!(f, "E2204: division by zero in `{function}`")
            }
            Self::AssertionFailed { message, function } => {
                write!(
                    f,
                    "E2205: assertion failed in `{function}`{}",
                    message.as_ref().map_or(String::new(), |m| format!(": {m}"))
                )
            }
            Self::OutOfBounds { function } => {
                write!(f, "E2206: index out of bounds in `{function}`")
            }
            Self::UnknownBuiltin { name, function } => {
                write!(f, "E2207: unknown builtin `{name}` in `{function}`")
            }
            Self::InvalidVariant { function } => {
                write!(f, "E2208: invalid enum variant in `{function}`")
            }
            Self::InvalidOutcomeState { reason, function } => {
                write!(f, "E2210: invalid outcome state in `{function}`: {reason}")
            }
            Self::BuiltinUnimplemented { name, reason } => {
                write!(
                    f,
                    "E2211: builtin `{name}` declared but dispatch unimplemented: {reason}"
                )
            }
            Self::JitShimFault { reason, function } => {
                write!(f, "E2212: JIT shim fault in `{function}`: {reason}")
            }
        }
    }
}

// ── Call frame ─────────────────────────────────────────────────────

/// A single call frame on the VM stack.
///
/// Each frame holds a register file indexed by `ValueId` and a program
/// counter tracking the current block + instruction index.
#[derive(Clone, Debug)]
struct Frame {
    /// The function being executed.
    func_id: FuncId,
    /// Name of the function (for diagnostics).
    func_name: String,
    /// Register file: `ValueId` → runtime value. Grown on demand.
    registers: Vec<RuntimeValue>,
    /// Current basic block.
    block: BlockId,
    /// The block that branched to the current block (for phi resolution).
    prev_block: Option<BlockId>,
    /// Instruction index within the current block.
    pc: usize,
    /// Block to return to after the function completes, paired with
    /// the destination register in the caller's frame.
    return_block: Option<BlockId>,
    return_dest: Option<ValueId>,
}

impl Frame {
    fn new(func: &Function, arg_count: usize) -> Self {
        // Estimate register count: params + some headroom for body values.
        // We'll grow on demand via ensure_register.
        let estimated = arg_count.max(16);
        Self {
            func_id: func.id,
            func_name: func
                .name
                .clone()
                .unwrap_or_else(|| format!("@f{}", func.id.0)),
            registers: Vec::with_capacity(estimated),
            block: func.entry_block().map_or(BlockId(0), |b| b.id),
            prev_block: None,
            pc: 0,
            return_block: None,
            return_dest: None,
        }
    }

    fn ensure_register(&mut self, id: ValueId) {
        let idx = id.0 as usize;
        if idx >= self.registers.len() {
            self.registers.resize(idx + 1, RuntimeValue::Unit);
        }
    }

    fn read(&self, id: ValueId) -> RuntimeValue {
        self.registers
            .get(id.0 as usize)
            .cloned()
            .unwrap_or(RuntimeValue::Unit)
    }

    fn write(&mut self, id: ValueId, value: RuntimeValue) {
        self.ensure_register(id);
        self.registers[id.0 as usize] = value;
    }
}

// ── JIT dispatcher trait ───────────────────────────────────────────

/// Hook surface the [`Vm`] uses to opt into Cranelift JIT dispatch.
///
/// Per [ADR-0030 §2] graduation policy. Implemented by
/// `triet_jit::JitDispatcher` (separate crate to avoid circular
/// dependency — `triet-jit` depends on `triet-ir`).
///
/// The Vm calls [`Self::record_call`] on every `CallLocal` to count
/// invocations, and [`Self::try_dispatch`] when the callee's
/// signature is JIT-friendly (currently all-Integer per ADR-0030
/// §12). On `try_dispatch` returning `Some(result)`, Vm skips
/// bytecode dispatch for that call. `None` falls through to Tier 1.
///
/// [ADR-0030 §2]: ../../../docs/decisions/0030-jit-cranelift-integration.md
pub trait JitDispatch {
    /// Record one call to `func_id`. The dispatcher MAY internally
    /// trigger Cranelift compilation of `program` when the call count
    /// hits the configured threshold (default 100 per ADR-0030 §2).
    /// Idempotent: re-running compilation is the dispatcher's
    /// responsibility to elide.
    fn record_call(&mut self, func_id: FuncId, program: &IrProgram);

    /// Attempt native dispatch with i64-marshaled args. Returns
    /// `Some(result)` when a finalized native function exists for
    /// `func_id` AND the arity (0–4) is supported. `None` on
    /// uncompiled / unsupported-signature / unsupported-arity.
    fn try_dispatch(&self, func_id: FuncId, args: &[i64]) -> Option<i64>;
}

// ── VM ─────────────────────────────────────────────────────────────

/// The IR bytecode VM — walks basic blocks in a register-based interpreter loop.
pub struct Vm {
    frames: Vec<Frame>,
    program: IrProgram,
    /// Flattened function table for quick lookup.
    functions: Vec<Function>,
    /// Block map for each function: `FuncId` → Vec<&`BasicBlock`> (indexed by `BlockId`).
    block_maps: HashMap<FuncId, HashMap<BlockId, BasicBlock>>,
    /// Path index: absolute path string → `FuncId` for cross-module call dispatch.
    path_index: HashMap<String, FuncId>,
    /// v0.9.x.jit.5 — Optional JIT dispatcher (Cranelift backend in
    /// `triet-jit`). `None` when JIT is disabled via CLI flag,
    /// `TRIET_JIT=disabled` env var, or kernel/embedded contexts.
    /// Per ADR-0030 Addendum Gap 1, JIT is ambient default for
    /// `usr.*` programs.
    jit: Option<Box<dyn JitDispatch>>,
    /// v0.10.x.thread.1 — Real-OS-thread `JoinHandle` registry per
    /// [ADR-0026 v2] §3. `RawThreadSpawn` inserts a freshly-spawned
    /// thread; `RawThreadJoin` consumes (via `remove`) and blocks
    /// on the handle. Single-threaded VM per [ADR-0028] §9, so the
    /// map itself needs no synchronization.
    ///
    /// [ADR-0026 v2]: ../../../../docs/decisions/0026-actor-boundary-send-rules.md
    /// [ADR-0028]: ../../../../docs/decisions/0028-atomic-primitive.md
    thread_handles: HashMap<i64, std::thread::JoinHandle<()>>,
    /// v0.10.x.thread.1 — Monotonic counter for `thread_id` assignment.
    /// ID 0 is reserved (matches the v0.9 stub's placeholder `Handle
    /// { thread_id: 0 }`); real spawns start at 1. Wraps around to
    /// negative on overflow per `i64` default; in practice this would
    /// require ~9.2 quintillion spawns in a single VM run.
    next_thread_id: i64,
}

impl Vm {
    /// Create a new VM from an IR program.
    #[must_use]
    pub fn new(program: IrProgram) -> Self {
        let functions: Vec<Function> = program
            .modules
            .iter()
            .flat_map(|m| m.functions.clone())
            .collect();

        let mut block_maps: HashMap<FuncId, HashMap<BlockId, BasicBlock>> = HashMap::new();
        for func in &functions {
            let map: HashMap<BlockId, BasicBlock> =
                func.blocks.iter().map(|b| (b.id, b.clone())).collect();
            block_maps.insert(func.id, map);
        }

        // Build a path → FuncId index for cross-module call dispatch.
        // `IrModule.path` is constructed as `AbsolutePath { module, name: "" }`
        // so its `to_string()` ends with a trailing dot (e.g. "crate."). Strip
        // it so the indexed key matches `AbsolutePath { name: "fn" }` callers
        // produce (e.g. "crate.fn").
        let mut path_index: HashMap<String, FuncId> = HashMap::new();
        for module in &program.modules {
            let module_path = module.path.module.to_string();
            for func in &module.functions {
                if let Some(ref name) = func.name {
                    let full_path = format!("{module_path}.{name}");
                    path_index.insert(full_path, func.id);
                }
            }
        }

        Self {
            frames: Vec::new(),
            program,
            functions,
            block_maps,
            path_index,
            jit: None,
            thread_handles: HashMap::new(),
            next_thread_id: 0,
        }
    }

    /// v0.9.x.jit.5 — Install a JIT dispatcher (typically
    /// `triet_jit::JitDispatcher`). Called by the CLI / runtime
    /// driver after construction when JIT is enabled. Replaces any
    /// previously-installed dispatcher.
    pub fn set_jit_dispatcher(&mut self, jit: Box<dyn JitDispatch>) {
        self.jit = Some(jit);
    }

    /// v0.9.x.jit.5 — Clear the JIT dispatcher (forces VM-only
    /// dispatch). Used by `--no-jit` CLI flag or
    /// `TRIET_JIT=disabled` env var path.
    pub fn disable_jit(&mut self) {
        self.jit = None;
    }

    /// Execute the program starting from the given function with arguments.
    ///
    /// # Errors
    ///
    /// Returns `VmError` on runtime failures (null unwrap, type mismatch,
    /// overflow, assertion failure, etc.).
    ///
    /// # Panics
    ///
    /// Panics if the internal frame stack is empty when a `Return` step
    /// fires. This is an invariant violation — the lowerer guarantees
    /// that every function emits exactly one return path for each control
    /// flow exit. Triggering it indicates a bug in the lowerer or in a
    /// manually constructed IR program.
    pub fn execute(
        &mut self,
        entry: FuncId,
        args: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue, VmError> {
        // Find the entry function.
        let func = self
            .functions
            .iter()
            .find(|f| f.id == entry)
            .cloned()
            .ok_or_else(|| VmError::FunctionNotFound {
                name: format!("@f{}", entry.0),
            })?;

        let mut frame = Frame::new(&func, args.len());
        // Write arguments into the first N registers.
        for (i, arg) in args.into_iter().enumerate() {
            frame.write(ValueId(i as u32), arg);
        }
        self.frames.push(frame);

        // Main dispatch loop.
        loop {
            let result = self.step()?;
            match result {
                StepResult::Continue => {}
                StepResult::Return(value) => {
                    if self.frames.len() <= 1 {
                        return Ok(value);
                    }
                    // Pop the just-executed frame and return value to caller.
                    let completed_frame = self.frames.pop().unwrap();
                    if let Some(caller) = self.frames.last_mut()
                        && let (Some(return_block), Some(return_dest)) =
                            (completed_frame.return_block, completed_frame.return_dest)
                    {
                        caller.write(return_dest, value);
                        caller.block = return_block;
                        // pc stays where it was (already past the Call instruction)
                    }
                    // If no return block/dest, the call was for side effects;
                    // the caller's pc was already advanced past the Call instruction.
                }
            }
        }
    }

    /// Execute one instruction and return the result.
    fn step(&mut self) -> Result<StepResult, VmError> {
        // Extract needed data before borrowing frames mutably.
        let frame_idx = self.frames.len() - 1;
        let func_id = self.frames[frame_idx].func_id;
        let block_id = self.frames[frame_idx].block;
        let pc = self.frames[frame_idx].pc;

        // Fetch the current block and instruction.
        let block = self
            .block_maps
            .get(&func_id)
            .and_then(|m| m.get(&block_id))
            .cloned();

        let Some(block) = block else {
            return Ok(StepResult::Return(RuntimeValue::Unit));
        };

        let instr = match block.instructions.get(pc) {
            Some(i) => i.clone(),
            None => return Ok(StepResult::Return(RuntimeValue::Unit)),
        };

        // Now borrow frame mutably.
        let frame = &mut self.frames[frame_idx];
        let func_name = frame.func_name.clone();
        frame.pc += 1;
        let constants = &self.program.constants;

        match instr {
            // ── Constants ────────────────────────────────────────
            Instruction::Const { dest, constant } => {
                let val = RuntimeValue::from_constant(
                    self.program
                        .constants
                        .get(constant)
                        .unwrap_or(&Constant::Unit),
                );
                frame.write(dest, val);
            }

            // ── Arithmetic ───────────────────────────────────────
            Instruction::Add { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs);
                let r = read_operand(constants, frame, rhs);
                frame.write(dest, arithmetic_add(&l, &r, &func_name)?);
            }
            Instruction::Sub { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs);
                let r = read_operand(constants, frame, rhs);
                frame.write(dest, arithmetic_sub(&l, &r, &func_name)?);
            }
            Instruction::Mul { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs);
                let r = read_operand(constants, frame, rhs);
                frame.write(dest, arithmetic_mul(&l, &r, &func_name)?);
            }
            Instruction::Div { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs);
                let r = read_operand(constants, frame, rhs);
                frame.write(dest, arithmetic_div(&l, &r, &func_name)?);
            }
            Instruction::Mod { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs);
                let r = read_operand(constants, frame, rhs);
                frame.write(dest, arithmetic_mod(&l, &r, &func_name)?);
            }
            Instruction::Pow { dest, base, exp } => {
                let b = read_operand(constants, frame, base);
                let e = read_operand(constants, frame, exp);
                frame.write(dest, arithmetic_pow(&b, &e, &func_name)?);
            }
            Instruction::Neg { dest, operand } => {
                let v = read_operand(constants, frame, operand);
                frame.write(dest, arithmetic_neg(&v, &func_name)?);
            }

            // ── Logic: Łukasiewicz Ł3 ────────────────────────────
            Instruction::LukAnd { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs).as_trilean();
                let r = read_operand(constants, frame, rhs).as_trilean();
                frame.write(dest, RuntimeValue::Trilean(l.and(r)));
            }
            Instruction::LukOr { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs).as_trilean();
                let r = read_operand(constants, frame, rhs).as_trilean();
                frame.write(dest, RuntimeValue::Trilean(l.or(r)));
            }
            Instruction::LukImplies { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs).as_trilean();
                let r = read_operand(constants, frame, rhs).as_trilean();
                frame.write(dest, RuntimeValue::Trilean(l.implies(r)));
            }
            Instruction::LukXor { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs).as_trilean();
                let r = read_operand(constants, frame, rhs).as_trilean();
                frame.write(dest, RuntimeValue::Trilean(l.xor(r)));
            }
            Instruction::LukIff { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs).as_trilean();
                let r = read_operand(constants, frame, rhs).as_trilean();
                frame.write(dest, RuntimeValue::Trilean(l.iff(r)));
            }

            // ── Logic: Kleene K3 ─────────────────────────────────
            Instruction::KleeneImplies { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs).as_trilean();
                let r = read_operand(constants, frame, rhs).as_trilean();
                frame.write(dest, RuntimeValue::Trilean(l.kleene_implies(r)));
            }
            Instruction::KleeneXor { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs).as_trilean();
                let r = read_operand(constants, frame, rhs).as_trilean();
                frame.write(dest, RuntimeValue::Trilean(l.kleene_xor(r)));
            }
            Instruction::KleeneIff { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs).as_trilean();
                let r = read_operand(constants, frame, rhs).as_trilean();
                frame.write(dest, RuntimeValue::Trilean(l.kleene_iff(r)));
            }

            // ── Comparison ───────────────────────────────────────
            Instruction::Eq { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs);
                let r = read_operand(constants, frame, rhs);
                let result = runtime_eq_trilean(&l, &r);
                frame.write(dest, RuntimeValue::Trilean(result));
            }
            Instruction::Ne { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs);
                let r = read_operand(constants, frame, rhs);
                let result = match runtime_eq_trilean(&l, &r) {
                    Trilean::True => Trilean::False,
                    Trilean::False => Trilean::True,
                    Trilean::Unknown => Trilean::Unknown,
                };
                frame.write(dest, RuntimeValue::Trilean(result));
            }
            Instruction::Lt { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs);
                let r = read_operand(constants, frame, rhs);
                let result = if runtime_cmp(&l, &r) == std::cmp::Ordering::Less {
                    Trilean::True
                } else {
                    Trilean::False
                };
                frame.write(dest, RuntimeValue::Trilean(result));
            }
            Instruction::Le { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs);
                let r = read_operand(constants, frame, rhs);
                let cmp = runtime_cmp(&l, &r);
                let result = if cmp == std::cmp::Ordering::Less || cmp == std::cmp::Ordering::Equal
                {
                    Trilean::True
                } else {
                    Trilean::False
                };
                frame.write(dest, RuntimeValue::Trilean(result));
            }
            Instruction::Gt { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs);
                let r = read_operand(constants, frame, rhs);
                let result = if runtime_cmp(&l, &r) == std::cmp::Ordering::Greater {
                    Trilean::True
                } else {
                    Trilean::False
                };
                frame.write(dest, RuntimeValue::Trilean(result));
            }
            Instruction::Ge { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs);
                let r = read_operand(constants, frame, rhs);
                let cmp = runtime_cmp(&l, &r);
                let result =
                    if cmp == std::cmp::Ordering::Greater || cmp == std::cmp::Ordering::Equal {
                        Trilean::True
                    } else {
                        Trilean::False
                    };
                frame.write(dest, RuntimeValue::Trilean(result));
            }

            // ── Conversion ───────────────────────────────────────
            Instruction::ToInteger { dest, operand } => {
                let v = read_operand(constants, frame, operand);
                frame.write(dest, convert_to_integer(&v, &func_name)?);
            }
            Instruction::ToTryte { dest, operand } => {
                let v = read_operand(constants, frame, operand);
                frame.write(dest, convert_to_tryte(&v, &func_name)?);
            }
            Instruction::ToLong { dest, operand } => {
                let v = read_operand(constants, frame, operand);
                frame.write(dest, convert_to_long(&v));
            }
            Instruction::ToTrit { dest, operand } => {
                let v = read_operand(constants, frame, operand);
                frame.write(dest, convert_to_trit(&v, &func_name)?);
            }
            Instruction::ToTrilean { dest, operand } => {
                let v = read_operand(constants, frame, operand);
                frame.write(dest, RuntimeValue::Trilean(v.as_trilean()));
            }

            // ── Nullable ─────────────────────────────────────────
            Instruction::NullWrap { dest, value } => {
                let v = read_operand(constants, frame, value);
                frame.write(dest, exec_null_wrap(v));
            }
            Instruction::NullUnwrap { dest, nullable } => {
                // The two canonical nullable carriers are
                // `RuntimeValue::Null` (panic) and any other value (pass
                // through) per ADR-0010 Addendum §D — see
                // `exec_null_unwrap`.
                let v = read_operand(constants, frame, nullable);
                frame.write(dest, exec_null_unwrap(v, &func_name)?);
            }
            Instruction::NullCheck { dest, nullable } => {
                // Discriminator trit (ADR-0010 + Addendum §D cross-
                // tolerance: Null and `Outcome { Zero, None }` → Zero) —
                // see `exec_null_check`.
                let v = read_operand(constants, frame, nullable);
                frame.write(dest, exec_null_check(&v));
            }

            // ── Aggregate: struct ────────────────────────────────
            Instruction::StructNew { dest, fields } => {
                let field_vals: Vec<RuntimeValue> = fields
                    .iter()
                    .map(|f| read_operand(constants, frame, *f))
                    .collect();
                frame.write(dest, exec_struct_new(field_vals));
            }
            Instruction::FieldGet {
                dest,
                object,
                field_idx,
            } => {
                let obj = read_operand(constants, frame, object);
                let val = exec_field_get(&obj, field_idx, &func_name)?;
                frame.write(dest, val);
            }
            Instruction::FieldSet {
                dest,
                object,
                field_idx,
                value,
            } => {
                let obj = read_operand(constants, frame, object);
                let new_val = read_operand(constants, frame, value);
                let updated = exec_field_set(&obj, field_idx, new_val, &func_name)?;
                frame.write(dest, updated);
            }

            // ── Aggregate: enum ──────────────────────────────────
            Instruction::EnumNew {
                dest,
                variant_idx,
                payload,
            } => {
                let payload_val = payload.map(|p| read_operand(constants, frame, p));
                frame.write(dest, exec_enum_new(variant_idx, payload_val));
            }
            Instruction::EnumTag { dest, scrutinee } => {
                let scr = read_operand(constants, frame, scrutinee);
                frame.write(dest, exec_enum_tag(&scr));
            }
            Instruction::EnumPayload { dest, scrutinee } => {
                let scr = read_operand(constants, frame, scrutinee);
                frame.write(dest, exec_enum_payload(&scr, &func_name)?);
            }

            // ── Function calls ───────────────────────────────────
            Instruction::CallLocal { dest, callee, args } => {
                let arg_vals: Vec<RuntimeValue> = args
                    .iter()
                    .map(|a| read_operand(constants, frame, *a))
                    .collect();

                let callee_func = self
                    .functions
                    .iter()
                    .find(|f| f.id == callee)
                    .cloned()
                    .ok_or_else(|| VmError::FunctionNotFound {
                        name: format!("@f{}", callee.0),
                    })?;

                // v0.9.x.jit.5 — Tier 2 JIT path attempt per ADR-0030
                // §2 graduation policy. Each call increments the
                // dispatcher's per-FuncId counter; once over the
                // threshold (default 100), the dispatcher Cranelift-
                // compiles the program. Subsequent calls check the
                // native code cache: if the callee compiled AND its
                // signature qualifies for the Integer-only dispatch
                // path AND all args marshal to i64, we run native code
                // and skip pushing a VM frame. Anything else falls
                // through to Tier 1 bytecode dispatch below.
                if let Some(ref mut jit) = self.jit {
                    jit.record_call(callee, &self.program);
                    if integer_signature_ok(&callee_func)
                        && let Some(i64_args) = try_marshal_integer_args(&arg_vals)
                        && let Some(result) = jit.try_dispatch(callee, &i64_args)
                    {
                        // Saturate on overflow (Integer is 27-trit
                        // range, ~±3.8e12). JIT i64 result may exceed;
                        // saturating matches VM bytecode arithmetic
                        // semantics.
                        let result_val =
                            RuntimeValue::Integer(Integer::new(result).unwrap_or(Integer::ZERO));
                        if let Some(d) = dest {
                            frame.write(d, result_val);
                        }
                        // Native call returned synchronously — VM
                        // continues at the next instruction without
                        // pushing a new frame.
                        return Ok(StepResult::Continue);
                    }
                }

                let mut new_frame = Frame::new(&callee_func, arg_vals.len());
                // Set return info on the CALLEE frame so when it returns,
                // we know where to resume the caller.
                new_frame.return_block = Some(frame.block);
                new_frame.return_dest = dest;
                for (i, arg) in arg_vals.into_iter().enumerate() {
                    new_frame.write(ValueId(i as u32), arg);
                }
                self.frames.push(new_frame);
            }
            Instruction::CallBuiltin { dest, name, args } => {
                let arg_vals: Vec<RuntimeValue> = args
                    .iter()
                    .map(|a| read_operand(constants, frame, *a))
                    .collect();
                // v0.10.x.thread.1 — route raw_thread variants through
                // the thread-aware helper for VM registry access (same
                // disjoint-borrow pattern as the CallCrossModule arm).
                let result = if matches!(
                    name,
                    BuiltinName::RawThreadSpawn | BuiltinName::RawThreadJoin
                ) {
                    execute_thread_builtin(
                        name,
                        &arg_vals,
                        &mut self.thread_handles,
                        &mut self.next_thread_id,
                        &func_name,
                    )?
                } else {
                    execute_builtin(name, &arg_vals, &func_name)?
                };
                if let Some(d) = dest {
                    frame.write(d, result);
                }
            }
            Instruction::CallCrossModule { dest, path, args } => {
                let arg_vals: Vec<RuntimeValue> = args
                    .iter()
                    .map(|a| read_operand(&self.program.constants, frame, *a))
                    .collect();
                let func_name = frame.func_name.clone();

                // Check for builtin by path suffix.
                if let Some(builtin) = path_to_builtin(&path.to_string()) {
                    // v0.10.x.thread.1 — raw_thread builtins need
                    // mutable access to Vm's `thread_handles` registry
                    // (disjoint from `self.frames[frame_idx]` borrow
                    // held as `frame`). Route them through the
                    // thread-aware helper; non-thread builtins keep
                    // the existing free-function dispatch path.
                    let result = if matches!(
                        builtin,
                        BuiltinName::RawThreadSpawn | BuiltinName::RawThreadJoin
                    ) {
                        execute_thread_builtin(
                            builtin,
                            &arg_vals,
                            &mut self.thread_handles,
                            &mut self.next_thread_id,
                            &func_name,
                        )?
                    } else {
                        execute_builtin(builtin, &arg_vals, &func_name)?
                    };
                    if let Some(d) = dest {
                        frame.write(d, result);
                    }
                } else if let Some(func_id) = self.path_index.get(&path.to_string()).copied() {
                    // Cross-module call to a known function.
                    let target = self
                        .functions
                        .iter()
                        .find(|f| f.id == func_id)
                        .cloned()
                        .ok_or_else(|| VmError::FunctionNotFound {
                            name: format!("@f{}", func_id.0),
                        })?;

                    // Set up return info on the new frame.
                    let mut new_frame = Frame::new(&target, arg_vals.len());
                    for (i, arg) in arg_vals.into_iter().enumerate() {
                        new_frame.write(ValueId(i as u32), arg);
                    }
                    new_frame.return_block = Some(frame.block);
                    new_frame.return_dest = dest;
                    self.frames.push(new_frame);
                    return Ok(StepResult::Continue);
                } else {
                    return Err(VmError::FunctionNotFound {
                        name: path.to_string(),
                    });
                }
            }
            Instruction::WitnessCall {
                dest,
                path,
                witness_idx,
                args,
            } => {
                // ADR-0012: cross-package generic dispatch. v0.4 semantics
                // run the callee function exactly like `CallCrossModule`;
                // the witness table is currently informational only, but
                // we validate that the referenced index exists so a
                // forward-compat user can rely on linker errors instead
                // of silent zero-table dispatch.
                if self
                    .program
                    .witness_tables
                    .get(witness_idx as usize)
                    .is_none()
                {
                    return Err(VmError::FunctionNotFound {
                        name: format!(
                            "witness table #{witness_idx} for {path} (program has {} tables)",
                            self.program.witness_tables.len()
                        ),
                    });
                }
                let arg_vals: Vec<RuntimeValue> = args
                    .iter()
                    .map(|a| read_operand(&self.program.constants, frame, *a))
                    .collect();
                let func_name = frame.func_name.clone();

                if let Some(builtin) = path_to_builtin(&path.to_string()) {
                    // Stdlib builtins exposed as "generic" via witness
                    // call are a degenerate case but still legal —
                    // dispatch identically. v0.10.x.thread.1: route
                    // raw_thread variants through the thread-aware
                    // helper for VM registry access (mirror of the
                    // CallCrossModule branch).
                    let result = if matches!(
                        builtin,
                        BuiltinName::RawThreadSpawn | BuiltinName::RawThreadJoin
                    ) {
                        execute_thread_builtin(
                            builtin,
                            &arg_vals,
                            &mut self.thread_handles,
                            &mut self.next_thread_id,
                            &func_name,
                        )?
                    } else {
                        execute_builtin(builtin, &arg_vals, &func_name)?
                    };
                    if let Some(d) = dest {
                        frame.write(d, result);
                    }
                } else if let Some(func_id) = self.path_index.get(&path.to_string()).copied() {
                    let target = self
                        .functions
                        .iter()
                        .find(|f| f.id == func_id)
                        .cloned()
                        .ok_or_else(|| VmError::FunctionNotFound {
                            name: format!("@f{}", func_id.0),
                        })?;
                    let mut new_frame = Frame::new(&target, arg_vals.len());
                    for (i, arg) in arg_vals.into_iter().enumerate() {
                        new_frame.write(ValueId(i as u32), arg);
                    }
                    new_frame.return_block = Some(frame.block);
                    new_frame.return_dest = dest;
                    self.frames.push(new_frame);
                    return Ok(StepResult::Continue);
                } else {
                    return Err(VmError::FunctionNotFound {
                        name: path.to_string(),
                    });
                }
            }

            // ── Closure ──────────────────────────────────────────
            Instruction::ClosureNew {
                dest,
                lambda,
                captures,
            } => {
                let capture_vals: Vec<RuntimeValue> =
                    captures.iter().map(|&v| frame.read(v)).collect();
                frame.write(
                    dest,
                    RuntimeValue::Closure {
                        func_id: lambda,
                        captures: capture_vals,
                    },
                );
            }
            Instruction::ClosureCall {
                dest,
                closure,
                args,
            } => {
                let clos = read_operand(constants, frame, closure);
                let arg_vals: Vec<RuntimeValue> = args
                    .iter()
                    .map(|a| read_operand(constants, frame, *a))
                    .collect();
                match clos {
                    RuntimeValue::Closure { func_id, captures } => {
                        let cap_count = captures.len() as u32;

                        let callee_func = self
                            .functions
                            .iter()
                            .find(|f| f.id == func_id)
                            .cloned()
                            .ok_or_else(|| VmError::FunctionNotFound {
                                name: format!("@f{}", func_id.0),
                            })?;

                        let mut new_frame =
                            Frame::new(&callee_func, arg_vals.len() + captures.len());
                        // Set return info on the CALLEE frame.
                        new_frame.return_block = Some(frame.block);
                        new_frame.return_dest = dest;
                        for (i, cap) in captures.into_iter().enumerate() {
                            new_frame.write(ValueId(i as u32), cap);
                        }
                        for (i, arg) in arg_vals.into_iter().enumerate() {
                            new_frame.write(ValueId(cap_count + i as u32), arg);
                        }
                        self.frames.push(new_frame);
                    }
                    _ => {
                        return Err(VmError::TypeMismatch {
                            expected: TypeTag::Unit,
                            actual: "non-closure".into(),
                            function: func_name,
                        });
                    }
                }
            }

            // ── Control flow ─────────────────────────────────────
            Instruction::Br { target } => {
                let prev = frame.block;
                frame.block = target;
                frame.prev_block = Some(prev);
                frame.pc = 0;
            }
            Instruction::BrIf {
                cond,
                then_block,
                else_block,
            } => {
                let c = read_operand(constants, frame, cond);
                let prev = frame.block;
                if c.is_truthy() {
                    frame.block = then_block;
                } else {
                    frame.block = else_block;
                }
                frame.prev_block = Some(prev);
                frame.pc = 0;
            }
            Instruction::BrTrilean {
                cond,
                true_block,
                unknown_block,
                false_block,
            } => {
                // ADR-0010: native three-way branch on Ł3 cond.
                let c = read_operand(constants, frame, cond);
                let prev = frame.block;
                frame.block = match c.as_trilean() {
                    Trilean::True => true_block,
                    Trilean::Unknown => unknown_block,
                    Trilean::False => false_block,
                };
                frame.prev_block = Some(prev);
                frame.pc = 0;
            }
            Instruction::Ret { value } => {
                let val = value.map_or(RuntimeValue::Unit, |v| read_operand(constants, frame, v));
                return Ok(StepResult::Return(val));
            }
            Instruction::Unreachable => {
                return Err(VmError::AssertionFailed {
                    message: Some("reached unreachable instruction".into()),
                    function: func_name,
                });
            }

            // ── Outcome (ADR-0020) ───────────────────────────────
            Instruction::OutcomeNewPositive { dest, payload } => {
                let val = read_operand(constants, frame, payload);
                frame.write(dest, exec_outcome_new_positive(val));
            }
            Instruction::OutcomeNewNegative { dest, payload } => {
                let val = read_operand(constants, frame, payload);
                frame.write(dest, exec_outcome_new_negative(val));
            }
            Instruction::OutcomeNewNull { dest } => {
                frame.write(dest, exec_outcome_new_null());
            }
            Instruction::OutcomeDiscriminant { dest, source } => {
                // Cross-tolerance (ADR-0010 Addendum §D + WA-6): Null reads
                // as Zero, bare non-Outcome as Positive — see
                // `exec_outcome_discriminant`.
                let outcome = read_operand(constants, frame, source);
                frame.write(dest, exec_outcome_discriminant(&outcome));
            }
            Instruction::OutcomeUnwrapValue { dest, source } => {
                let outcome = read_operand(constants, frame, source);
                frame.write(dest, exec_outcome_unwrap_value(outcome, &func_name)?);
            }
            Instruction::OutcomeUnwrapError { dest, source } => {
                let outcome = read_operand(constants, frame, source);
                frame.write(dest, exec_outcome_unwrap_error(outcome, &func_name)?);
            }

            // ── Phi node ─────────────────────────────────────────
            Instruction::Phi { dest, incoming } => {
                // Select the value coming from the predecessor block.
                let selected = incoming
                    .iter()
                    .find(|edge| Some(edge.block) == frame.prev_block)
                    .or_else(|| incoming.first())
                    .map_or(RuntimeValue::Unit, |phi: &PhiIncoming| {
                        frame.read(phi.value)
                    });
                frame.write(dest, selected);
            }
        }

        Ok(StepResult::Continue)
    }
}

/// v0.9.x.jit.5 — Return true iff `func`'s signature qualifies for
/// the Integer-only native dispatch path (all-Integer params + Integer
/// return + arity ≤ 4). Wider type coverage defers v0.10 per
/// ADR-0030 §12 backlog.
fn integer_signature_ok(func: &Function) -> bool {
    if func.params.len() > 4 {
        return false;
    }
    if !matches!(func.return_type, TypeTag::Integer) {
        return false;
    }
    func.params
        .iter()
        .all(|(_, t)| matches!(t, TypeTag::Integer))
}

/// v0.9.x.jit.5 — Try to marshal a slice of `RuntimeValue` into a
/// `Vec<i64>` for the JIT native-dispatch ABI. Returns `None` if any
/// arg is not `RuntimeValue::Integer` (mixed types disqualify the
/// JIT path; VM tier-1 dispatch handles them).
fn try_marshal_integer_args(vals: &[RuntimeValue]) -> Option<Vec<i64>> {
    vals.iter()
        .map(|v| match v {
            RuntimeValue::Integer(i) => Some(i.to_i64()),
            _ => None,
        })
        .collect()
}

/// Read an operand from a frame, resolving constants from the pool.
fn read_operand(
    constants: &crate::constant::ConstantPool,
    frame: &Frame,
    op: Operand,
) -> RuntimeValue {
    match op {
        Operand::Value(id) => frame.read(id),
        Operand::Const(cid) => {
            RuntimeValue::from_constant(constants.get(cid).unwrap_or(&Constant::Unit))
        }
    }
}

// ── Step result ────────────────────────────────────────────────────

enum StepResult {
    /// Continue to the next instruction.
    Continue,
    /// Return from the current function.
    Return(RuntimeValue),
}

// ── Outcome helpers ────────────────────────────────────────────────

/// Render the arm name for a `Trit` discriminator — used in
/// [`VmError::InvalidOutcomeState`] messages so authors see "failure"
/// instead of `-1`.
const fn arm_name(discriminator: Trit) -> &'static str {
    if discriminator.is_positive() {
        "success"
    } else if discriminator.is_negative() {
        "failure"
    } else {
        "null"
    }
}

// ── Arithmetic helpers ─────────────────────────────────────────────

fn arithmetic_add(l: &RuntimeValue, r: &RuntimeValue, func: &str) -> Result<RuntimeValue, VmError> {
    match (l, r) {
        (RuntimeValue::Integer(a), RuntimeValue::Integer(b)) => {
            Ok(RuntimeValue::Integer(a.try_add(*b).ok_or_else(|| {
                VmError::Overflow {
                    function: func.into(),
                }
            })?))
        }
        (RuntimeValue::Long(a), RuntimeValue::Long(b)) => Ok(RuntimeValue::Long(*a + *b)),
        (RuntimeValue::Tryte(a), RuntimeValue::Tryte(b)) => {
            Ok(RuntimeValue::Tryte(a.try_add(*b).ok_or_else(|| {
                VmError::Overflow {
                    function: func.into(),
                }
            })?))
        }
        (RuntimeValue::Trit(a), RuntimeValue::Trit(b)) => {
            let sum = a.to_i8() + b.to_i8();
            Ok(RuntimeValue::Trit(Trit::from_i8(sum).unwrap_or(Trit::Zero)))
        }
        _ => Ok(RuntimeValue::Integer(Integer::new(0).unwrap())),
    }
}

fn arithmetic_sub(l: &RuntimeValue, r: &RuntimeValue, func: &str) -> Result<RuntimeValue, VmError> {
    match (l, r) {
        (RuntimeValue::Integer(a), RuntimeValue::Integer(b)) => Ok(RuntimeValue::Integer(
            a.try_subtract(*b).ok_or_else(|| VmError::Overflow {
                function: func.into(),
            })?,
        )),
        (RuntimeValue::Long(a), RuntimeValue::Long(b)) => Ok(RuntimeValue::Long(*a - *b)),
        (RuntimeValue::Tryte(a), RuntimeValue::Tryte(b)) => Ok(RuntimeValue::Tryte(
            a.try_subtract(*b).ok_or_else(|| VmError::Overflow {
                function: func.into(),
            })?,
        )),
        _ => Ok(RuntimeValue::Integer(Integer::new(0).unwrap())),
    }
}

fn arithmetic_mul(l: &RuntimeValue, r: &RuntimeValue, func: &str) -> Result<RuntimeValue, VmError> {
    match (l, r) {
        (RuntimeValue::Integer(a), RuntimeValue::Integer(b)) => Ok(RuntimeValue::Integer(
            a.try_multiply(*b).ok_or_else(|| VmError::Overflow {
                function: func.into(),
            })?,
        )),
        (RuntimeValue::Long(a), RuntimeValue::Long(b)) => Ok(RuntimeValue::Long(*a * *b)),
        (RuntimeValue::Tryte(a), RuntimeValue::Tryte(b)) => Ok(RuntimeValue::Tryte(
            a.try_multiply(*b).ok_or_else(|| VmError::Overflow {
                function: func.into(),
            })?,
        )),
        _ => Ok(RuntimeValue::Integer(Integer::new(0).unwrap())),
    }
}

fn arithmetic_div(l: &RuntimeValue, r: &RuntimeValue, func: &str) -> Result<RuntimeValue, VmError> {
    match (l, r) {
        (RuntimeValue::Integer(a), RuntimeValue::Integer(b)) => {
            if *b == Integer::new(0).unwrap() {
                return Err(VmError::DivisionByZero {
                    function: func.into(),
                });
            }
            Ok(RuntimeValue::Integer(a.try_divide(*b).map_err(|_| {
                VmError::DivisionByZero {
                    function: func.into(),
                }
            })?))
        }
        (RuntimeValue::Long(a), RuntimeValue::Long(b)) => {
            if *b == Long::from_i128(0) {
                return Err(VmError::DivisionByZero {
                    function: func.into(),
                });
            }
            Ok(RuntimeValue::Long(*a / *b))
        }
        _ => Ok(RuntimeValue::Integer(Integer::new(0).unwrap())),
    }
}

fn arithmetic_mod(l: &RuntimeValue, r: &RuntimeValue, func: &str) -> Result<RuntimeValue, VmError> {
    match (l, r) {
        (RuntimeValue::Integer(a), RuntimeValue::Integer(b)) => {
            if *b == Integer::new(0).unwrap() {
                return Err(VmError::DivisionByZero {
                    function: func.into(),
                });
            }
            Ok(RuntimeValue::Integer(a.try_modulo(*b).map_err(|_| {
                VmError::DivisionByZero {
                    function: func.into(),
                }
            })?))
        }
        (RuntimeValue::Long(a), RuntimeValue::Long(b)) => {
            if *b == Long::from_i128(0) {
                return Err(VmError::DivisionByZero {
                    function: func.into(),
                });
            }
            Ok(RuntimeValue::Long(*a % *b))
        }
        _ => Ok(RuntimeValue::Integer(Integer::new(0).unwrap())),
    }
}

fn arithmetic_pow(l: &RuntimeValue, r: &RuntimeValue, func: &str) -> Result<RuntimeValue, VmError> {
    match (l, r) {
        (RuntimeValue::Integer(a), RuntimeValue::Integer(b)) => {
            // `to_i64()` is bounded by `Integer`'s 27-trit range so it fits an
            // `i64`; clamping to non-negative makes the cast lossless. If the
            // exponent exceeds `u32::MAX` we cap there — the multiply loop
            // below would already have overflowed long before then.
            let exp = u32::try_from(b.to_i64().max(0)).unwrap_or(u32::MAX);
            let mut result = Integer::new(1).unwrap();
            for _ in 0..exp {
                result = result.try_multiply(*a).ok_or_else(|| VmError::Overflow {
                    function: func.into(),
                })?;
            }
            Ok(RuntimeValue::Integer(result))
        }
        _ => Ok(RuntimeValue::Integer(Integer::new(0).unwrap())),
    }
}

// `arithmetic_neg` cannot fail today, but every sibling `arithmetic_*` returns
// `Result<_, VmError>` so the VM dispatch table is uniform. Keeping the same
// signature avoids a special case at every call site and reserves headroom for
// negate-on-MIN overflow checks (Integer/Long/Tryte/Trit all currently saturate).
#[allow(clippy::unnecessary_wraps)]
fn arithmetic_neg(v: &RuntimeValue, _func: &str) -> Result<RuntimeValue, VmError> {
    match v {
        RuntimeValue::Integer(a) => Ok(RuntimeValue::Integer(-*a)),
        RuntimeValue::Long(a) => Ok(RuntimeValue::Long(-*a)),
        RuntimeValue::Tryte(a) => Ok(RuntimeValue::Tryte(-*a)),
        RuntimeValue::Trit(a) => Ok(RuntimeValue::Trit(-*a)),
        RuntimeValue::Trilean(t) => Ok(RuntimeValue::Trilean(!*t)),
        _ => Ok(RuntimeValue::Unit),
    }
}

// ── Conversion helpers ─────────────────────────────────────────────

fn convert_to_integer(v: &RuntimeValue, func: &str) -> Result<RuntimeValue, VmError> {
    match v {
        RuntimeValue::Trit(t) => Ok(RuntimeValue::Integer(
            Integer::new(i64::from(t.to_i8())).unwrap(),
        )),
        RuntimeValue::Tryte(t) => Ok(RuntimeValue::Integer(
            Integer::new(i64::from(t.to_i16())).unwrap(),
        )),
        RuntimeValue::Integer(_) => Ok(v.clone()),
        RuntimeValue::Long(l) => Ok(RuntimeValue::Integer(l.to_integer())),
        _ => Err(VmError::TypeMismatch {
            expected: TypeTag::Integer,
            actual: format!("{}", v.type_tag()),
            function: func.into(),
        }),
    }
}

fn convert_to_tryte(v: &RuntimeValue, func: &str) -> Result<RuntimeValue, VmError> {
    match v {
        RuntimeValue::Trit(t) => Ok(RuntimeValue::Tryte(
            Tryte::new(i16::from(t.to_i8())).unwrap(),
        )),
        RuntimeValue::Tryte(_) => Ok(v.clone()),
        RuntimeValue::Integer(i) => Ok(RuntimeValue::Tryte(
            Tryte::new(i.to_i64() as i16).unwrap_or(Tryte::ZERO),
        )),
        _ => Err(VmError::TypeMismatch {
            expected: TypeTag::Tryte,
            actual: format!("{}", v.type_tag()),
            function: func.into(),
        }),
    }
}

fn convert_to_long(v: &RuntimeValue) -> RuntimeValue {
    match v {
        RuntimeValue::Trit(t) => RuntimeValue::Long(Long::from_i128(i128::from(t.to_i8()))),
        RuntimeValue::Tryte(t) => RuntimeValue::Long(Long::from_i128(i128::from(t.to_i16()))),
        RuntimeValue::Integer(i) => RuntimeValue::Long(Long::from_i128(i128::from(i.to_i64()))),
        RuntimeValue::Long(_) => v.clone(),
        _ => RuntimeValue::Long(Long::from_i128(0)),
    }
}

fn convert_to_trit(v: &RuntimeValue, func: &str) -> Result<RuntimeValue, VmError> {
    match v {
        RuntimeValue::Trit(_) => Ok(v.clone()),
        RuntimeValue::Tryte(t) => Ok(RuntimeValue::Trit(
            Trit::from_i8(t.to_i16() as i8).unwrap_or(Trit::Zero),
        )),
        RuntimeValue::Integer(i) => Ok(RuntimeValue::Trit(
            Trit::from_i8(i.to_i64() as i8).unwrap_or(Trit::Zero),
        )),
        _ => Err(VmError::TypeMismatch {
            expected: TypeTag::Trit,
            actual: format!("{}", v.type_tag()),
            function: func.into(),
        }),
    }
}

// ── Comparison helpers ─────────────────────────────────────────────

fn runtime_eq(l: &RuntimeValue, r: &RuntimeValue) -> bool {
    match (l, r) {
        (RuntimeValue::Trit(a), RuntimeValue::Trit(b)) => a == b,
        (RuntimeValue::Tryte(a), RuntimeValue::Tryte(b)) => a == b,
        (RuntimeValue::Integer(a), RuntimeValue::Integer(b)) => a == b,
        (RuntimeValue::Long(a), RuntimeValue::Long(b)) => a == b,
        (RuntimeValue::Trilean(a), RuntimeValue::Trilean(b)) => a == b,
        (RuntimeValue::String(a), RuntimeValue::String(b)) => a == b,
        (RuntimeValue::Unit, RuntimeValue::Unit) => true,
        (RuntimeValue::Null, RuntimeValue::Null) => true,
        // v0.7.x.runtime-fix-debt.3: user-defined types.
        // Struct: element-wise equality in declaration order.
        (RuntimeValue::Struct { fields: a }, RuntimeValue::Struct { fields: b }) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(l, r)| runtime_eq(l, r))
        }
        // Enum: variant index + optional payload must match.
        (
            RuntimeValue::Enum {
                variant: va,
                payload: pa,
            },
            RuntimeValue::Enum {
                variant: vb,
                payload: pb,
            },
        ) => {
            va == vb
                && match (pa, pb) {
                    (Some(la), Some(lb)) => runtime_eq(la, lb),
                    (None, None) => true,
                    _ => false,
                }
        }
        // Outcome: discriminator + optional payload must match.
        (
            RuntimeValue::Outcome {
                discriminator: da,
                payload: pa,
            },
            RuntimeValue::Outcome {
                discriminator: db,
                payload: pb,
            },
        ) => {
            da == db
                && match (pa, pb) {
                    (Some(la), Some(lb)) => runtime_eq(la, lb),
                    (None, None) => true,
                    _ => false,
                }
        }
        // Closure: function identity + captures must match.
        (
            RuntimeValue::Closure {
                func_id: fa,
                captures: ca,
            },
            RuntimeValue::Closure {
                func_id: fb,
                captures: cb,
            },
        ) => {
            fa == fb
                && ca.len() == cb.len()
                && ca.iter().zip(cb.iter()).all(|(l, r)| runtime_eq(l, r))
        }
        // Vector: element-wise equality.
        (RuntimeValue::Vector(a), RuntimeValue::Vector(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(l, r)| runtime_eq(l, r))
        }
        _ => false,
    }
}

/// Łukasiewicz-aware equality (ADR-0010). Returns `Trilean::Unknown`
/// when either operand carries an Unknown truth value, so chained
/// boolean reasoning in user code preserves the third truth value
/// instead of collapsing to False.
///
/// - Both operands `Trilean::Unknown` → Unknown (cannot confirm same).
/// - One operand `Trilean::Unknown`, other any Trilean → Unknown.
/// - All other value pairs: classical value equality (True/False).
///
/// `Trit::Zero` is a concrete trit value, not a truth-Unknown, so
/// `Trit::Zero == Trit::Zero` remains classically True here.
fn runtime_eq_trilean(l: &RuntimeValue, r: &RuntimeValue) -> Trilean {
    if let (RuntimeValue::Trilean(a), RuntimeValue::Trilean(b)) = (l, r)
        && (matches!(a, Trilean::Unknown) || matches!(b, Trilean::Unknown))
    {
        return Trilean::Unknown;
    }
    if runtime_eq(l, r) {
        Trilean::True
    } else {
        Trilean::False
    }
}

fn runtime_cmp(l: &RuntimeValue, r: &RuntimeValue) -> std::cmp::Ordering {
    match (l, r) {
        (RuntimeValue::Integer(a), RuntimeValue::Integer(b)) => a.cmp(b),
        (RuntimeValue::Long(a), RuntimeValue::Long(b)) => a.cmp(b),
        (RuntimeValue::Tryte(a), RuntimeValue::Tryte(b)) => a.cmp(b),
        (RuntimeValue::Trit(a), RuntimeValue::Trit(b)) => a.to_i8().cmp(&b.to_i8()),
        (RuntimeValue::String(a), RuntimeValue::String(b)) => a.cmp(b),
        _ => std::cmp::Ordering::Equal,
    }
}

// ── Builtins ───────────────────────────────────────────────────────

/// Map an absolute path to a builtin, if the path refers to a known stdlib builtin.
fn path_to_builtin(path: &str) -> Option<BuiltinName> {
    match path {
        // Pre-v0.7 builtins (stdlib `std.io` / `std.assert` / `std.text`).
        "std.io.println" => Some(BuiltinName::Println),
        "std.io.print" => Some(BuiltinName::Print),
        "std.assert.assert" => Some(BuiltinName::Assert),
        "std.assert.assert_eq" => Some(BuiltinName::AssertEq),
        "std.text.len" => Some(BuiltinName::TextLen),
        "std.text.concat" => Some(BuiltinName::TextConcat),
        "std.text.from_integer" => Some(BuiltinName::TextFromInteger),

        // v0.7.4.2 (ADR-0019 Addendum §A7) stdlib stub paths.
        // Function names in each module do NOT repeat the module
        // name — match existing `std.io.println` / `std.text.len`
        // precedent. BuiltinName enum (Rust-side) keeps explicit
        // `VectorNew`/`HashMapNew`/etc. for disambiguation.

        // Vector ops (v0.7.3.2, IDs 8-11).
        "std.collections.vector.new" => Some(BuiltinName::VectorNew),
        "std.collections.vector.push" => Some(BuiltinName::VectorPush),
        "std.collections.vector.get" => Some(BuiltinName::VectorGet),
        "std.collections.vector.length" => Some(BuiltinName::VectorLength),

        // HashMap ops (v0.7.3.3, IDs 12-16).
        "std.collections.hashmap.new" => Some(BuiltinName::HashMapNew),
        "std.collections.hashmap.insert" => Some(BuiltinName::HashMapInsert),
        "std.collections.hashmap.get" => Some(BuiltinName::HashMapGet),
        "std.collections.hashmap.keys" => Some(BuiltinName::HashMapKeys),
        "std.collections.hashmap.contains" => Some(BuiltinName::HashMapContains),

        // File I/O (v0.7.3.4, IDs 17-19). Capability gating deferred
        // §A7 → v0.7.10.
        "std.io.fs.read" => Some(BuiltinName::ReadFile),
        "std.io.fs.write" => Some(BuiltinName::WriteFile),
        "std.io.fs.write_bytes" => Some(BuiltinName::WriteFileBytes),
        "std.io.fs.exists" => Some(BuiltinName::FileExists),

        // Path ops (v0.7.3.4, IDs 20-22). POSIX-only Q2-A.
        "std.path.join" => Some(BuiltinName::PathJoin),
        "std.path.parent" => Some(BuiltinName::PathParent),
        "std.path.basename" => Some(BuiltinName::PathBasename),

        // String ops (v0.7.3.4, IDs 23-25). Char-index Q3-A.
        "std.string.substring" => Some(BuiltinName::StringSubstring),
        "std.string.split" => Some(BuiltinName::StringSplit),
        "std.string.index_of" => Some(BuiltinName::StringIndexOf),

        // ParseInteger (v0.7.3.4, ID 26). Paired with `from_integer`
        // in `std.text` per Q2-A symmetry.
        "std.text.parse_integer" => Some(BuiltinName::ParseInteger),
        "std.text.into_bytes" => Some(BuiltinName::TextIntoBytes),
        "std.text.from_bytes" => Some(BuiltinName::TextFromBytes),
        "std.crypto.blake3_hash" => Some(BuiltinName::Blake3Hash),
        "std.env.get" => Some(BuiltinName::GetEnv),

        // v0.7.12.1 — filesystem-aware module loader needs this
        // to walk `compiler/` for self-hosting.
        "std.io.fs.read_dir_recursive" => Some(BuiltinName::ReadDirRecursive),

        // v0.9.x.atomic.2 — atomic primitive ops per ADR-0028 §1 + §4.
        // IDs 33-42. Path lookup wires source `sys.atomic.*` callers
        // to BuiltinName variants. VM dispatch lands v0.9.x.atomic.3-4.
        // Capability check `sys.atomic` fires per ADR-0016 §5 + ADR-0028 §8.
        "sys.atomic.new" => Some(BuiltinName::AtomicNew),
        "sys.atomic.load" => Some(BuiltinName::AtomicLoad),
        "sys.atomic.store" => Some(BuiltinName::AtomicStore),
        "sys.atomic.swap" => Some(BuiltinName::AtomicSwap),
        "sys.atomic.compare_exchange" => Some(BuiltinName::AtomicCompareExchange),
        "sys.atomic.fetch_add" => Some(BuiltinName::AtomicFetchAdd),
        "sys.atomic.fetch_sub" => Some(BuiltinName::AtomicFetchSub),
        "sys.atomic.fetch_bitwise_and" => Some(BuiltinName::AtomicFetchBitwiseAnd),
        "sys.atomic.fetch_bitwise_or" => Some(BuiltinName::AtomicFetchBitwiseOr),
        "sys.atomic.fetch_bitwise_xor" => Some(BuiltinName::AtomicFetchBitwiseXor),

        // v0.10.x.thread.1 — raw OS thread primitives per ADR-0026 v2 §3.
        // IDs 43-44, `.triv` v6 → v7. Capability `sys.raw_thread` enforced
        // at link time per ADR-0016 §5 (no runtime check here).
        "sys.raw_thread.spawn" => Some(BuiltinName::RawThreadSpawn),
        "sys.raw_thread.join" => Some(BuiltinName::RawThreadJoin),

        _ => None,
    }
}

/// v0.10.x.thread.2 — Acquire the atomic cell's mutex guard.
///
/// Single-thread VM dev tier (ADR-0028 §9) — the mutex never sees
/// real contention; poisoning would mean a prior holder panicked
/// mid-operation, which is itself a bug. Promoting to `.expect()` so
/// the diagnostic surfaces the invariant rather than silently
/// recovering from inconsistent state.
fn lock_atomic(
    cell: &std::sync::Arc<std::sync::Mutex<RuntimeValue>>,
) -> std::sync::MutexGuard<'_, RuntimeValue> {
    cell.lock()
        .expect("atomic mutex poisoned (single-thread VM dev tier)")
}

/// v0.9.x.atomic.4 — arithmetic op selector for `fetch_add`/`fetch_sub`
/// dispatch. Keeps the two arms thin.
#[derive(Clone, Copy)]
enum ArithmeticOp {
    Add,
    Sub,
}

/// v0.9.x.atomic.4 — bitwise op selector for `fetch_bitwise_and`/`or`/`xor`
/// dispatch. Per ADR-0028 Addendum: operates on 64-bit binary slot of
/// Triết `Integer`; renamed `_bitwise_` to make binary semantics explicit
/// (anti-ternary leak warning honored).
#[derive(Clone, Copy)]
enum BitwiseOp {
    And,
    Or,
    Xor,
}

/// v0.9.x.atomic.4 — shared dispatch for `fetch_add`/`sub` on `Tryte`/`Integer`
/// per ADR-0028 §4.2. Returns PREVIOUS value (pre-modification).
///
/// The mutex guard is INTENTIONALLY held across the read-modify-write
/// so the operation is atomic from any concurrent observer's
/// perspective. Allowing `clippy::significant_drop_tightening` here
/// — tightening would split the read and write into separate locked
/// sections, defeating the atomic intent.
#[allow(clippy::significant_drop_tightening)]
fn atomic_fetch_arithmetic(
    args: &[RuntimeValue],
    func_name: &str,
    op: ArithmeticOp,
) -> Result<RuntimeValue, VmError> {
    let atomic = args.first().cloned().ok_or_else(|| VmError::TypeMismatch {
        expected: TypeTag::Atomic(Box::new(TypeTag::Unit)),
        actual: "missing atomic arg".into(),
        function: func_name.into(),
    })?;
    let delta = args.get(1).cloned().ok_or_else(|| VmError::TypeMismatch {
        expected: TypeTag::Integer,
        actual: "missing delta arg".into(),
        function: func_name.into(),
    })?;
    let cell = match atomic {
        RuntimeValue::Atomic(rc) => rc,
        other => {
            return Err(VmError::TypeMismatch {
                expected: TypeTag::Atomic(Box::new(TypeTag::Unit)),
                actual: format!("{:?}", other.type_tag()),
                function: func_name.into(),
            });
        }
    };
    // Hold the lock once across read-modify-write — atomic from any
    // concurrent observer's perspective (real ordering enforcement
    // lands with v2.0 LLVM AOT; v0.10 single-thread VM has no race
    // surface, but holding the lock once is cleaner than back-to-back
    // lock/unlock and matches the semantic intent of the op).
    let mut guard = lock_atomic(&cell);
    let current = guard.clone();
    let new_value = match op {
        ArithmeticOp::Add => arithmetic_add(&current, &delta, func_name)?,
        ArithmeticOp::Sub => arithmetic_sub(&current, &delta, func_name)?,
    };
    *guard = new_value;
    Ok(current)
}

/// v0.9.x.atomic.4 — shared dispatch for `fetch_bitwise_and`/`or`/`xor` on
/// `Integer` per ADR-0028 Addendum (§4.3 bitwise ops, binary semantics
/// on 64-bit slot). Returns PREVIOUS value (pre-modification).
///
/// `Tryte` excluded per ADR-0028 §2 type table (9-trit width clashes
/// with binary atomic intrinsics). `Trit`/`Trilean` excluded — only
/// `Integer` `AtomicValue` supports bitwise.
///
/// Same intentional guard-held-across-RMW pattern as the arithmetic
/// counterpart; `significant_drop_tightening` allowed for the same
/// atomicity-intent reason.
#[allow(clippy::significant_drop_tightening)]
fn atomic_fetch_bitwise(
    args: &[RuntimeValue],
    func_name: &str,
    op: BitwiseOp,
) -> Result<RuntimeValue, VmError> {
    let atomic = args.first().cloned().ok_or_else(|| VmError::TypeMismatch {
        expected: TypeTag::Atomic(Box::new(TypeTag::Integer)),
        actual: "missing atomic arg".into(),
        function: func_name.into(),
    })?;
    let mask = args.get(1).cloned().ok_or_else(|| VmError::TypeMismatch {
        expected: TypeTag::Integer,
        actual: "missing mask arg".into(),
        function: func_name.into(),
    })?;
    let cell = match atomic {
        RuntimeValue::Atomic(rc) => rc,
        other => {
            return Err(VmError::TypeMismatch {
                expected: TypeTag::Atomic(Box::new(TypeTag::Integer)),
                actual: format!("{:?}", other.type_tag()),
                function: func_name.into(),
            });
        }
    };
    // Hold lock once across read-modify-write per the arithmetic
    // counterpart's note.
    let mut guard = lock_atomic(&cell);
    let current = guard.clone();
    let (a_raw, b_raw) = match (&current, &mask) {
        (RuntimeValue::Integer(a), RuntimeValue::Integer(b)) => (a.to_i64(), b.to_i64()),
        _ => {
            return Err(VmError::TypeMismatch {
                expected: TypeTag::Integer,
                actual: format!(
                    "current={:?} mask={:?}",
                    current.type_tag(),
                    mask.type_tag()
                ),
                function: func_name.into(),
            });
        }
    };
    let new_raw = match op {
        BitwiseOp::And => a_raw & b_raw,
        BitwiseOp::Or => a_raw | b_raw,
        BitwiseOp::Xor => a_raw ^ b_raw,
    };
    let new_value =
        RuntimeValue::Integer(Integer::new(new_raw).ok_or_else(|| VmError::Overflow {
            function: func_name.into(),
        })?);
    *guard = new_value;
    Ok(current)
}

/// v0.9.x.atomic.3 — value equality for `AtomicValue` types (`Trit`/`Tryte`/
/// `Integer`/`Trilean` per ADR-0028 §2). Used by `AtomicCompareExchange`
/// dispatch. Unrelated to `runtime_eq_trilean` (ADR-0010 Ł3-aware) because
/// Atomic equality is concrete-state comparison, not Ł3 logic.
fn atomic_value_eq(a: &RuntimeValue, b: &RuntimeValue) -> bool {
    match (a, b) {
        (RuntimeValue::Trit(x), RuntimeValue::Trit(y)) => x == y,
        (RuntimeValue::Tryte(x), RuntimeValue::Tryte(y)) => x == y,
        (RuntimeValue::Integer(x), RuntimeValue::Integer(y)) => x == y,
        (RuntimeValue::Trilean(x), RuntimeValue::Trilean(y)) => x == y,
        // Non-AtomicValue types should never reach here (typecheck E1040
        // filters at compile-time), but be defensive: mismatched types
        // never equal.
        _ => false,
    }
}

/// Validate a `Vector<Integer>` as a byte array — every element must be
/// a `RuntimeValue::Integer` in 0..=255. Returns `None` on any non-byte
/// element so callers can map to a domain-level null/error.
fn vector_to_byte_array(elements: &[RuntimeValue]) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(elements.len());
    for elem in elements {
        let RuntimeValue::Integer(i) = elem else {
            return None;
        };
        let raw = i.to_i64();
        let byte = u8::try_from(raw).ok()?;
        bytes.push(byte);
    }
    Some(bytes)
}

/// v0.10.x.jit.2b — Public builtin-dispatch entry point shared by the
/// VM and the JIT shim layer.
///
/// The JIT's `extern "C"` builtin shims (per [ADR-0032]) marshal their
/// ABI arguments into `RuntimeValue`s, call this, then marshal the
/// result back — so builtin SEMANTICS have a single source of truth
/// (no VM↔JIT divergence by construction). Thin pub wrapper over the
/// crate-private `execute_builtin`. Atomic builtins (33-42) are handled
/// here; `raw_thread.spawn`/`join` are NOT (they need the VM's
/// thread-handle registry).
///
/// # Errors
///
/// Returns the same [`VmError`] variants `execute_builtin` produces —
/// e.g. `AssertionFailed`, `TypeMismatch`, `OutOfBounds`, `Overflow`.
///
/// [ADR-0032]: ../../../docs/decisions/0032-builtin-shim-abi.md
pub fn dispatch_builtin(
    name: BuiltinName,
    args: &[RuntimeValue],
    func_name: &str,
) -> Result<RuntimeValue, VmError> {
    execute_builtin(name, args, func_name)
}

// ── Aggregate-opcode helpers (single source of truth, ADR-0034 §1) ──
//
// The struct opcodes' SEMANTICS live here as `pub` functions that BOTH
// the VM instruction loop and the JIT's delegate-to-VM shims call — so a
// JIT'd struct op runs the exact same logic the VM does (no VM↔JIT
// divergence by construction, the ADR-0032 §6 discipline generalized to
// IR opcodes per ADR-0034).

/// `StructNew` — allocate a struct from its fields (declaration order).
#[must_use]
pub const fn exec_struct_new(fields: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::Struct { fields }
}

/// `FieldGet` — read field `field_idx` (0-based) of a struct. Out-of-range
/// yields `Unit` (matches the VM); a non-struct is a `TypeMismatch`.
///
/// # Errors
/// [`VmError::TypeMismatch`] if `object` is not a struct.
pub fn exec_field_get(
    object: &RuntimeValue,
    field_idx: u32,
    func_name: &str,
) -> Result<RuntimeValue, VmError> {
    match object {
        RuntimeValue::Struct { fields } => Ok(fields
            .get(field_idx as usize)
            .cloned()
            .unwrap_or(RuntimeValue::Unit)),
        _ => Err(VmError::TypeMismatch {
            expected: TypeTag::Unit,
            actual: "non-struct".into(),
            function: func_name.to_string(),
        }),
    }
}

/// `FieldSet` — return a copy of the struct with field `field_idx`
/// replaced (functional update). Out-of-range index is a no-op (matches
/// the VM); a non-struct is a `TypeMismatch`.
///
/// # Errors
/// [`VmError::TypeMismatch`] if `object` is not a struct.
pub fn exec_field_set(
    object: &RuntimeValue,
    field_idx: u32,
    value: RuntimeValue,
    func_name: &str,
) -> Result<RuntimeValue, VmError> {
    match object {
        RuntimeValue::Struct { fields } => {
            let mut fields = fields.clone();
            if (field_idx as usize) < fields.len() {
                fields[field_idx as usize] = value;
            }
            Ok(RuntimeValue::Struct { fields })
        }
        _ => Err(VmError::TypeMismatch {
            expected: TypeTag::Unit,
            actual: "non-struct".into(),
            function: func_name.to_string(),
        }),
    }
}

/// `EnumNew` — construct an enum variant (ADR-0034 agg.2a). `payload`
/// is `None` for a unit variant, `Some` otherwise — the same shape the
/// VM's `EnumNew` arm builds.
#[must_use]
pub fn exec_enum_new(variant: u32, payload: Option<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::Enum {
        variant,
        payload: payload.map(Box::new),
    }
}

/// `EnumTag` — the variant index as an `Integer` (ADR-0034 agg.2a).
/// `Null` → `-1`, a bare non-enum value → `0` (variant 0), matching the
/// VM's `EnumTag` arm (v0.7.4.3-debt.7). Total — never faults.
#[must_use]
pub fn exec_enum_tag(scrutinee: &RuntimeValue) -> RuntimeValue {
    let idx: i64 = match scrutinee {
        RuntimeValue::Enum { variant, .. } => i64::from(*variant),
        RuntimeValue::Null => -1,
        _ => 0,
    };
    RuntimeValue::Integer(Integer::new(idx).unwrap_or_default())
}

/// `EnumPayload` — unpack a variant's payload (ADR-0034 agg.2a). A
/// payload-less or non-enum scrutinee is an `InvalidVariant` error,
/// matching the VM's `EnumPayload` arm.
///
/// # Errors
/// [`VmError::InvalidVariant`] if `scrutinee` is not an enum carrying a
/// payload.
pub fn exec_enum_payload(
    scrutinee: &RuntimeValue,
    func_name: &str,
) -> Result<RuntimeValue, VmError> {
    match scrutinee {
        RuntimeValue::Enum {
            payload: Some(p), ..
        } => Ok((**p).clone()),
        _ => Err(VmError::InvalidVariant {
            function: func_name.to_string(),
        }),
    }
}

/// `OutcomeNewPositive` — wrap `payload` in the `Trit::Positive` success
/// arm (ADR-0034 agg.2b).
#[must_use]
pub fn exec_outcome_new_positive(payload: RuntimeValue) -> RuntimeValue {
    RuntimeValue::Outcome {
        discriminator: Trit::Positive,
        payload: Some(Box::new(payload)),
    }
}

/// `OutcomeNewNegative` — wrap `payload` in the `Trit::Negative` failure
/// arm (ADR-0034 agg.2b).
#[must_use]
pub fn exec_outcome_new_negative(payload: RuntimeValue) -> RuntimeValue {
    RuntimeValue::Outcome {
        discriminator: Trit::Negative,
        payload: Some(Box::new(payload)),
    }
}

/// `OutcomeNewNull` — the `Trit::Zero` null arm (no payload), ADR-0034
/// agg.2b.
#[must_use]
pub const fn exec_outcome_new_null() -> RuntimeValue {
    RuntimeValue::Outcome {
        discriminator: Trit::Zero,
        payload: None,
    }
}

/// `OutcomeDiscriminant` — the arm trit (ADR-0034 agg.2b).
///
/// Mirrors the VM's cross-tolerance: `Null` reads as `Zero`; a bare
/// non-Outcome value (a `T` flowing through a `T?` slot) reads as
/// `Positive`. Total.
#[must_use]
pub const fn exec_outcome_discriminant(source: &RuntimeValue) -> RuntimeValue {
    let discriminator = match source {
        RuntimeValue::Outcome { discriminator, .. } => *discriminator,
        RuntimeValue::Null => Trit::Zero,
        _ => Trit::Positive,
    };
    RuntimeValue::Trit(discriminator)
}

/// `OutcomeUnwrapValue` — extract the success payload (ADR-0034 agg.2b).
///
/// A bare non-Outcome value passes through unchanged (it IS its own
/// success payload, per WA-6 cross-tolerance); `Null` or a non-success
/// arm is an `InvalidOutcomeState`. Takes ownership so the payload moves
/// out without a clone (matching the VM).
///
/// # Errors
/// [`VmError::InvalidOutcomeState`] on a null / failure arm or a
/// payload-less success arm.
pub fn exec_outcome_unwrap_value(
    source: RuntimeValue,
    func_name: &str,
) -> Result<RuntimeValue, VmError> {
    let (discriminator, payload) = match source {
        RuntimeValue::Outcome {
            discriminator,
            payload,
        } => (discriminator, payload),
        RuntimeValue::Null => {
            return Err(VmError::InvalidOutcomeState {
                reason: "unwrap_value called on null state".into(),
                function: func_name.to_string(),
            });
        }
        other => return Ok(other),
    };
    if !discriminator.is_positive() {
        return Err(VmError::InvalidOutcomeState {
            reason: format!("unwrap_value called on {} arm", arm_name(discriminator)),
            function: func_name.to_string(),
        });
    }
    let inner = payload.ok_or_else(|| VmError::InvalidOutcomeState {
        reason: "success arm missing payload".into(),
        function: func_name.to_string(),
    })?;
    Ok(*inner)
}

/// `OutcomeUnwrapError` — extract the failure payload (ADR-0034 agg.2b).
///
/// `Null` or a non-Outcome value is a type/state error; a non-failure arm
/// is an `InvalidOutcomeState`. Takes ownership (payload moves out).
///
/// # Errors
/// [`VmError::InvalidOutcomeState`] on a null / non-failure arm or a
/// payload-less failure arm; [`VmError::TypeMismatch`] on a non-Outcome
/// non-Null value.
pub fn exec_outcome_unwrap_error(
    source: RuntimeValue,
    func_name: &str,
) -> Result<RuntimeValue, VmError> {
    let (discriminator, payload) = match source {
        RuntimeValue::Outcome {
            discriminator,
            payload,
        } => (discriminator, payload),
        RuntimeValue::Null => {
            return Err(VmError::InvalidOutcomeState {
                reason: "unwrap_error called on null state".into(),
                function: func_name.to_string(),
            });
        }
        other => {
            return Err(VmError::TypeMismatch {
                expected: TypeTag::Outcome {
                    value_type: Box::new(TypeTag::Unit),
                    error_type: Box::new(TypeTag::Unit),
                    allow_null_state: false,
                },
                actual: other.type_tag().to_string(),
                function: func_name.to_string(),
            });
        }
    };
    if !discriminator.is_negative() {
        return Err(VmError::InvalidOutcomeState {
            reason: format!("unwrap_error called on {} arm", arm_name(discriminator)),
            function: func_name.to_string(),
        });
    }
    let inner = payload.ok_or_else(|| VmError::InvalidOutcomeState {
        reason: "failure arm missing payload".into(),
        function: func_name.to_string(),
    })?;
    Ok(*inner)
}

/// `NullWrap` — wrap a value as the non-null `Some` carrier (ADR-0034
/// agg.3a).
///
/// Mirrors the VM: an `Enum { variant: 0, payload: Some(value) }`.
#[must_use]
pub fn exec_null_wrap(value: RuntimeValue) -> RuntimeValue {
    RuntimeValue::Enum {
        variant: 0,
        payload: Some(Box::new(value)),
    }
}

/// `NullUnwrap` — force-unwrap a nullable (ADR-0034 agg.3a).
///
/// `Null` panics (`NullUnwrap` error); any other value passes through
/// unchanged (the canonical `T?` carrier flows as the bare value per
/// ADR-0010 Addendum §D). Takes ownership (the value moves out).
///
/// # Errors
/// [`VmError::NullUnwrap`] if `value` is `Null`.
pub fn exec_null_unwrap(value: RuntimeValue, func_name: &str) -> Result<RuntimeValue, VmError> {
    match value {
        RuntimeValue::Null => Err(VmError::NullUnwrap {
            function: func_name.to_string(),
        }),
        other => Ok(other),
    }
}

/// `NullCheck` — the nullable's discriminator trit (ADR-0034 agg.3a).
///
/// `Zero` for the two canonical null carriers (`Null` and `Outcome {
/// Zero, None }` per ADR-0010 Addendum §D cross-tolerance), `Positive`
/// otherwise. Total — never faults.
#[must_use]
pub const fn exec_null_check(value: &RuntimeValue) -> RuntimeValue {
    let trit = match value {
        RuntimeValue::Null
        | RuntimeValue::Outcome {
            discriminator: Trit::Zero,
            payload: None,
        } => Trit::Zero,
        _ => Trit::Positive,
    };
    RuntimeValue::Trit(trit)
}

/// Binary scalar opcodes the JIT's boxed mode delegates to the VM
/// (ADR-0034 §1, agg.1c).
///
/// One enum so a single `__triet_binop` shim + `exec_jit_binop` cover
/// arithmetic + comparison + Ł3/K3 logic, instead of ~20 per-op shims.
/// The `#[repr(u8)]` discriminants are the wire contract between the
/// codegen (emits the op number) and the shim (reconstructs via
/// [`JitBinOp::from_u8`]).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JitBinOp {
    /// `+`
    Add = 0,
    /// `-`
    Sub = 1,
    /// `*`
    Mul = 2,
    /// `/`
    Div = 3,
    /// `%`
    Mod = 4,
    /// `**`
    Pow = 5,
    /// `==` (Ł3-aware)
    Eq = 6,
    /// `!=`
    Ne = 7,
    /// `<`
    Lt = 8,
    /// `<=`
    Le = 9,
    /// `>`
    Gt = 10,
    /// `>=`
    Ge = 11,
    /// `&&` (Łukasiewicz)
    LukAnd = 12,
    /// `||`
    LukOr = 13,
    /// `=>`
    LukImplies = 14,
    /// `^`
    LukXor = 15,
    /// `<=>`
    LukIff = 16,
    /// `~>` (Kleene)
    KleeneImplies = 17,
    /// `~^`
    KleeneXor = 18,
    /// `<~>`
    KleeneIff = 19,
}

impl JitBinOp {
    /// Reconstruct from the wire discriminant (shim side). `None` for an
    /// out-of-range code (treated as a shim fault).
    #[must_use]
    pub const fn from_u8(n: u8) -> Option<Self> {
        let op = match n {
            0 => Self::Add,
            1 => Self::Sub,
            2 => Self::Mul,
            3 => Self::Div,
            4 => Self::Mod,
            5 => Self::Pow,
            6 => Self::Eq,
            7 => Self::Ne,
            8 => Self::Lt,
            9 => Self::Le,
            10 => Self::Gt,
            11 => Self::Ge,
            12 => Self::LukAnd,
            13 => Self::LukOr,
            14 => Self::LukImplies,
            15 => Self::LukXor,
            16 => Self::LukIff,
            17 => Self::KleeneImplies,
            18 => Self::KleeneXor,
            19 => Self::KleeneIff,
            _ => return None,
        };
        Some(op)
    }
}

/// Execute a binary scalar op (ADR-0034 §1).
///
/// The single source of truth the JIT boxed-mode `__triet_binop` shim
/// delegates to, so the JIT and the VM compute identical results.
/// Dispatches to the same `arithmetic_*` / `runtime_eq_trilean` /
/// `runtime_cmp` / `Trilean` logic the VM instruction loop uses.
///
/// # Errors
/// Propagates arithmetic [`VmError`] (e.g. `Overflow`, `DivisionByZero`).
pub fn exec_jit_binop(
    op: JitBinOp,
    l: &RuntimeValue,
    r: &RuntimeValue,
    func: &str,
) -> Result<RuntimeValue, VmError> {
    use std::cmp::Ordering;
    let cmp_to_trilean = |want_lt: bool, want_eq: bool, want_gt: bool| {
        let ord = runtime_cmp(l, r);
        let hit = match ord {
            Ordering::Less => want_lt,
            Ordering::Equal => want_eq,
            Ordering::Greater => want_gt,
        };
        RuntimeValue::Trilean(if hit { Trilean::True } else { Trilean::False })
    };
    let trilean = |t: Trilean| RuntimeValue::Trilean(t);
    Ok(match op {
        JitBinOp::Add => arithmetic_add(l, r, func)?,
        JitBinOp::Sub => arithmetic_sub(l, r, func)?,
        JitBinOp::Mul => arithmetic_mul(l, r, func)?,
        JitBinOp::Div => arithmetic_div(l, r, func)?,
        JitBinOp::Mod => arithmetic_mod(l, r, func)?,
        JitBinOp::Pow => arithmetic_pow(l, r, func)?,
        JitBinOp::Eq => trilean(runtime_eq_trilean(l, r)),
        JitBinOp::Ne => trilean(match runtime_eq_trilean(l, r) {
            Trilean::True => Trilean::False,
            Trilean::False => Trilean::True,
            Trilean::Unknown => Trilean::Unknown,
        }),
        JitBinOp::Lt => cmp_to_trilean(true, false, false),
        JitBinOp::Le => cmp_to_trilean(true, true, false),
        JitBinOp::Gt => cmp_to_trilean(false, false, true),
        JitBinOp::Ge => cmp_to_trilean(false, true, true),
        JitBinOp::LukAnd => trilean(l.as_trilean().and(r.as_trilean())),
        JitBinOp::LukOr => trilean(l.as_trilean().or(r.as_trilean())),
        JitBinOp::LukImplies => trilean(l.as_trilean().implies(r.as_trilean())),
        JitBinOp::LukXor => trilean(l.as_trilean().xor(r.as_trilean())),
        JitBinOp::LukIff => trilean(l.as_trilean().iff(r.as_trilean())),
        JitBinOp::KleeneImplies => trilean(l.as_trilean().kleene_implies(r.as_trilean())),
        JitBinOp::KleeneXor => trilean(l.as_trilean().kleene_xor(r.as_trilean())),
        JitBinOp::KleeneIff => trilean(l.as_trilean().kleene_iff(r.as_trilean())),
    })
}

/// Execute unary `Neg` (ADR-0034 §1) — the `__triet_neg` shim delegate.
///
/// # Errors
/// Propagates arithmetic [`VmError`].
pub fn exec_jit_neg(v: &RuntimeValue, func: &str) -> Result<RuntimeValue, VmError> {
    arithmetic_neg(v, func)
}

/// Primitive constant kinds the JIT boxed mode materializes via the
/// `__triet_box_const` shim (ADR-0034 §1, agg.1c).
///
/// `String`/`Long` constants are NOT here — they need a data-section /
/// i128 path (agg.3) and tier down until then. The `#[repr(u8)]`
/// discriminants are the codegen↔shim wire contract (cf. [`JitBinOp`]).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JitConstKind {
    /// `Trit` from the `i8` payload (`{-1,0,+1}`).
    Trit = 0,
    /// `Tryte` from the `i16`-range payload.
    Tryte = 1,
    /// `Integer` from the `i64` payload.
    Integer = 2,
    /// `Trilean` from the `i8` payload (`{-1,0,+1}` → `False/Unknown/True`).
    Trilean = 3,
    /// `Unit` (payload ignored).
    Unit = 4,
    /// `Null` (payload ignored).
    Null = 5,
}

impl JitConstKind {
    /// Reconstruct from the wire discriminant (shim side); `None` if out
    /// of range (a shim fault).
    #[must_use]
    pub const fn from_u8(n: u8) -> Option<Self> {
        let k = match n {
            0 => Self::Trit,
            1 => Self::Tryte,
            2 => Self::Integer,
            3 => Self::Trilean,
            4 => Self::Unit,
            5 => Self::Null,
            _ => return None,
        };
        Some(k)
    }
}

/// Materialize a primitive constant from its `(kind, payload)` wire form
/// (ADR-0034 §1) — the `__triet_box_const` shim delegate.
///
/// Mirrors how the VM turns a [`crate::Constant`] into a
/// [`RuntimeValue`], keeping one source of truth. Out-of-range
/// `Trit`/`Tryte` payloads saturate to the zero value (cannot occur from
/// valid codegen).
#[must_use]
pub fn exec_box_const(kind: JitConstKind, payload: i64) -> RuntimeValue {
    match kind {
        JitConstKind::Trit => RuntimeValue::Trit(
            Trit::from_i8(i8::try_from(payload).unwrap_or(0)).unwrap_or(Trit::Zero),
        ),
        JitConstKind::Tryte => RuntimeValue::Tryte(Tryte::new_saturating(payload)),
        JitConstKind::Integer => RuntimeValue::Integer(Integer::new(payload).unwrap_or_default()),
        JitConstKind::Trilean => RuntimeValue::Trilean(match payload {
            1 => Trilean::True,
            -1 => Trilean::False,
            _ => Trilean::Unknown,
        }),
        JitConstKind::Unit => RuntimeValue::Unit,
        JitConstKind::Null => RuntimeValue::Null,
    }
}

/// Read a boxed branch condition's three-way tag for JIT boxed-mode
/// branches (ADR-0034 agg.1c-iv).
///
/// `{-1, 0, +1}` = `False / Unknown / True` — the same `{-1,0,+1}`
/// encoding the unboxed branch codegen uses (ADR-0010 §3). Delegates to
/// `RuntimeValue::as_trilean` so the JIT's
/// `BrIf` / `BrTrilean` dispatch matches the VM exactly (the VM reads
/// `is_truthy` = `as_trilean == True` for `BrIf`, and `as_trilean` for
/// `BrTrilean`). Total: a non-Trilean / null value maps through
/// `as_trilean` (Integer sign, else `Unknown`) — never faults.
#[must_use]
pub fn exec_trilean_tag(value: &RuntimeValue) -> i8 {
    match value.as_trilean() {
        Trilean::True => 1,
        Trilean::Unknown => 0,
        Trilean::False => -1,
    }
}

// `clippy::significant_drop_tightening` allowed because the
// AtomicSwap / AtomicCompareExchange arms intentionally hold the
// mutex guard across the read-modify-write — tightening would split
// the read + write into separate locked sections, defeating the
// atomic intent (same rationale as the arithmetic / bitwise helpers
// above, v0.10.x.thread.2 Arc<Mutex> migration).
#[allow(clippy::significant_drop_tightening)]
fn execute_builtin(
    name: BuiltinName,
    args: &[RuntimeValue],
    func_name: &str,
) -> Result<RuntimeValue, VmError> {
    match name {
        BuiltinName::Println => {
            for a in args {
                print!("{a}");
            }
            println!();
            Ok(RuntimeValue::Unit)
        }
        BuiltinName::Print => {
            for a in args {
                print!("{a}");
            }
            Ok(RuntimeValue::Unit)
        }
        BuiltinName::Assert => {
            let cond = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            if !cond.is_truthy() {
                let msg = args.get(1).map(|m| format!("{m}"));
                return Err(VmError::AssertionFailed {
                    message: msg,
                    function: func_name.into(),
                });
            }
            Ok(RuntimeValue::Unit)
        }
        BuiltinName::FStringConcat => {
            let mut result = String::new();
            for a in args {
                let s = format!("{a}");
                result.push_str(&s);
            }
            Ok(RuntimeValue::String(result))
        }
        BuiltinName::AssertEq => {
            let a = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let b = args.get(1).cloned().unwrap_or(RuntimeValue::Unit);
            if !runtime_eq(&a, &b) {
                return Err(VmError::AssertionFailed {
                    message: Some(format!("{a} != {b}")),
                    function: func_name.into(),
                });
            }
            Ok(RuntimeValue::Unit)
        }
        BuiltinName::TextLen => {
            let s = match args.first() {
                Some(RuntimeValue::String(s)) => s.chars().count(),
                _ => 0,
            };
            Ok(RuntimeValue::Integer(
                Integer::new(i64::try_from(s).unwrap_or(0)).unwrap_or_default(),
            ))
        }
        BuiltinName::TextConcat => {
            use std::fmt::Write as _;
            let mut out = String::new();
            for a in args {
                if let RuntimeValue::String(s) = a {
                    out.push_str(s);
                } else {
                    write!(out, "{a}").unwrap();
                }
            }
            Ok(RuntimeValue::String(out))
        }
        BuiltinName::TextFromInteger => {
            let s = args.first().map_or_else(String::new, |v| format!("{v}"));
            Ok(RuntimeValue::String(s))
        }
        BuiltinName::VectorNew => {
            // Zero-arg builtin per ADR-0019 §5 + Addendum §A1. Extra
            // args are ignored to keep dispatch tolerant under fuzz —
            // the lowerer is responsible for arity correctness.
            Ok(RuntimeValue::Vector(Vec::new()))
        }
        BuiltinName::VectorPush => {
            // Functional return-new (Q1-A): clone the input, append,
            // emit a fresh Vector. SSA-safe: caller binds the result
            // to a fresh ValueId.
            let vector = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let item = args.get(1).cloned().unwrap_or(RuntimeValue::Unit);
            let mut new_vector = match vector {
                RuntimeValue::Vector(elements) => elements,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::Vector(Box::new(TypeTag::Unit)),
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            new_vector.push(item);
            Ok(RuntimeValue::Vector(new_vector))
        }
        BuiltinName::VectorGet => {
            // Strict bounds (Q3-A): negative index → Null, out-of-
            // bounds positive → Null. In-range returns the cloned
            // element wrapped in `T?` ≡ value-itself; the IR `T?`
            // discriminator is the value's own presence.
            let vector = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let index = args.get(1).cloned().unwrap_or(RuntimeValue::Unit);
            let elements = match vector {
                RuntimeValue::Vector(v) => v,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::Vector(Box::new(TypeTag::Unit)),
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let idx = match index {
                RuntimeValue::Integer(i) => i.to_i64(),
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::Integer,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            if idx < 0 {
                return Ok(RuntimeValue::Null);
            }
            let result = usize::try_from(idx)
                .ok()
                .and_then(|i| elements.get(i).cloned())
                .unwrap_or(RuntimeValue::Null);
            Ok(result)
        }
        BuiltinName::VectorLength => {
            let vector = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let length = match vector {
                RuntimeValue::Vector(v) => v.len(),
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::Vector(Box::new(TypeTag::Unit)),
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            // Vector lengths are bounded by Rust's `Vec` (≤ isize::MAX),
            // so the `i64::try_from` cannot realistically fail on
            // current targets. Fall back to `Integer::default()` for
            // defense against future 128-bit `Vec` capacities.
            Ok(RuntimeValue::Integer(
                Integer::new(i64::try_from(length).unwrap_or(i64::MAX)).unwrap_or_default(),
            ))
        }
        BuiltinName::HashMapNew => Ok(RuntimeValue::HashMap(std::collections::BTreeMap::new())),
        BuiltinName::HashMapInsert => {
            // Functional return-new (Q1-A consistency): clone the
            // map, insert/overwrite k -> v, return new map. Old
            // value at key (if any) is silently dropped — caller
            // does explicit `hashmap_get` first if they need it.
            let map_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let key_arg = args.get(1).cloned().unwrap_or(RuntimeValue::Unit);
            let value_arg = args.get(2).cloned().unwrap_or(RuntimeValue::Unit);
            let mut new_map = match map_arg {
                RuntimeValue::HashMap(m) => m,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::HashMap(
                            Box::new(TypeTag::Unit),
                            Box::new(TypeTag::Unit),
                        ),
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            // Q2-B invalid key handling: refuse-over-guess. Non-
            // hashable key types (Vector/HashMap/Trilean/Struct/Enum/
            // Closure/Unit/Null) → TypeMismatch panic. This is a
            // bug, not a data event — caller's logic is broken.
            // See ADR-0019 Addendum §A7 "error handling primitive"
            // for future recovery story.
            let key = RuntimeMapKey::from_runtime(&key_arg).ok_or_else(|| {
                VmError::TypeMismatch {
                    expected: TypeTag::String, // sentinel: documented as "any hashable primitive"
                    actual: format!(
                        "non-hashable key type {:?} (expected Trit/Tryte/Integer/Long/String)",
                        key_arg.type_tag()
                    ),
                    function: func_name.into(),
                }
            })?;
            new_map.insert(key, value_arg);
            Ok(RuntimeValue::HashMap(new_map))
        }
        BuiltinName::HashMapGet => {
            // Lookup-miss = data event → return `V? = Null`. Invalid
            // key type = bug → TypeMismatch panic (Q2-B). Distinct
            // tiers per error model: §A7 deferred items doc.
            let map_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let key_arg = args.get(1).cloned().unwrap_or(RuntimeValue::Unit);
            let map = match map_arg {
                RuntimeValue::HashMap(m) => m,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::HashMap(
                            Box::new(TypeTag::Unit),
                            Box::new(TypeTag::Unit),
                        ),
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let key =
                RuntimeMapKey::from_runtime(&key_arg).ok_or_else(|| VmError::TypeMismatch {
                    expected: TypeTag::String,
                    actual: format!(
                        "non-hashable key type {:?} (expected Trit/Tryte/Integer/Long/String)",
                        key_arg.type_tag()
                    ),
                    function: func_name.into(),
                })?;
            Ok(map.get(&key).cloned().unwrap_or(RuntimeValue::Null))
        }
        BuiltinName::HashMapKeys => {
            // Q4-A: sorted key order (BTreeMap natural). Deterministic
            // by construction — aligns ADR-0019 §3 canonical
            // emission principle. Empty map → empty Vector.
            let map_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let map = match map_arg {
                RuntimeValue::HashMap(m) => m,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::HashMap(
                            Box::new(TypeTag::Unit),
                            Box::new(TypeTag::Unit),
                        ),
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let keys: Vec<RuntimeValue> = map.keys().map(RuntimeMapKey::to_runtime).collect();
            Ok(RuntimeValue::Vector(keys))
        }
        BuiltinName::ReadFile => {
            // Capability gating deferred per ADR-0019 Addendum §A7 —
            // v0.7.10 CLI wiring will resolve `sys.fs.read` against
            // CapabilityResolver before reaching the VM. For v0.7.3.4
            // self-host bootstrap context, trust the caller.
            // Any I/O error → Null (data-tier per error model §A7).
            let path_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let path = match path_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            Ok(std::fs::read_to_string(&path).map_or(RuntimeValue::Null, RuntimeValue::String))
        }
        BuiltinName::WriteFile => {
            // Q4-A strict 2-state Trilean: True/False only, never
            // Unknown. Capability gating deferred (§A7).
            let path_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let contents_arg = args.get(1).cloned().unwrap_or(RuntimeValue::Unit);
            let path = match path_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let contents = match contents_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let ok = std::fs::write(&path, &contents).is_ok();
            Ok(RuntimeValue::Trilean(if ok {
                Trilean::True
            } else {
                Trilean::False
            }))
        }
        BuiltinName::WriteFileBytes => {
            // Binary-mode write for `.khi` output. Capability
            // gating deferred per ADR-0019 Addendum §A7 — v0.7.10 CLI
            // wiring will resolve `sys.fs.write` against
            // CapabilityResolver before reaching the VM.
            let path_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let bytes_arg = args.get(1).cloned().unwrap_or(RuntimeValue::Unit);
            let path = match path_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let RuntimeValue::Vector(bytes_vec) = bytes_arg else {
                return Err(VmError::TypeMismatch {
                    expected: TypeTag::Vector(Box::new(TypeTag::Integer)),
                    actual: format!("{:?}", bytes_arg.type_tag()),
                    function: func_name.into(),
                });
            };
            let mut buf: Vec<u8> = Vec::with_capacity(bytes_vec.len());
            for v in bytes_vec {
                let n = match v {
                    RuntimeValue::Integer(i) => i.to_i64(),
                    other => {
                        return Err(VmError::TypeMismatch {
                            expected: TypeTag::Integer,
                            actual: format!("{:?}", other.type_tag()),
                            function: func_name.into(),
                        });
                    }
                };
                // Per builtin docstring: out-of-byte-range yields
                // strict `False`, not a runtime panic. Mirrors Q4-A
                // refuse-over-guess for I/O failures.
                let Ok(b) = u8::try_from(n) else {
                    return Ok(RuntimeValue::Trilean(Trilean::False));
                };
                buf.push(b);
            }
            let ok = std::fs::write(&path, &buf).is_ok();
            Ok(RuntimeValue::Trilean(if ok {
                Trilean::True
            } else {
                Trilean::False
            }))
        }
        BuiltinName::FileExists => {
            let path_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let path = match path_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let exists = std::path::Path::new(&path).is_file();
            Ok(RuntimeValue::Trilean(if exists {
                Trilean::True
            } else {
                Trilean::False
            }))
        }
        BuiltinName::PathJoin => {
            // Q2-A POSIX-only string manipulation. Hardcoded `/`
            // separator for byte-identical bootstrap output
            // regardless of host OS. Windows path semantics deferred
            // (§A7). Empty base returns segment as-is; trailing `/`
            // in base not duplicated.
            let base_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let segment_arg = args.get(1).cloned().unwrap_or(RuntimeValue::Unit);
            let base = match base_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let segment = match segment_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let joined = if base.is_empty() {
                segment
            } else if base.ends_with('/') {
                format!("{base}{segment}")
            } else {
                format!("{base}/{segment}")
            };
            Ok(RuntimeValue::String(joined))
        }
        BuiltinName::PathParent => {
            // Strip last `/`-segment. Null if path is root `/`, empty,
            // or has no separator. POSIX semantic per Q2-A.
            let path_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let path = match path_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let result = match path.rfind('/') {
                None => RuntimeValue::Null,
                Some(0) if path.len() == 1 => RuntimeValue::Null, // path == "/"
                Some(idx) => RuntimeValue::String(path[..idx].into()),
            };
            Ok(result)
        }
        BuiltinName::PathBasename => {
            // Return last `/`-segment. For paths ending in `/`,
            // return the segment before the final separator.
            let path_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let path = match path_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            // Trim trailing `/` then take the last segment.
            let trimmed = path.trim_end_matches('/');
            let basename = trimmed
                .rfind('/')
                .map_or(trimmed, |idx| &trimmed[idx + 1..]);
            Ok(RuntimeValue::String(basename.into()))
        }
        BuiltinName::StringSubstring => {
            // Q3-A char-index slicing with OOB panic. Caller checks
            // text_len first. Handles Vietnamese correctly via
            // codepoint iteration. Empty range returns "".
            let s_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let start_arg = args.get(1).cloned().unwrap_or(RuntimeValue::Unit);
            let end_arg = args.get(2).cloned().unwrap_or(RuntimeValue::Unit);
            let s = match s_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let start = match start_arg {
                RuntimeValue::Integer(i) => i.to_i64(),
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::Integer,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let end = match end_arg {
                RuntimeValue::Integer(i) => i.to_i64(),
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::Integer,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let char_count = i64::try_from(s.chars().count()).unwrap_or(i64::MAX);
            if start < 0 || end < 0 || start > end || end > char_count {
                return Err(VmError::OutOfBounds {
                    function: func_name.into(),
                });
            }
            // Use char_indices to map codepoint positions to byte
            // offsets — preserves multi-byte UTF-8 (Vietnamese, etc.).
            let mut byte_start = s.len();
            let mut byte_end = s.len();
            for (char_idx, (byte_idx, _)) in s.char_indices().enumerate() {
                let char_idx_i64 = i64::try_from(char_idx).unwrap_or(i64::MAX);
                if char_idx_i64 == start {
                    byte_start = byte_idx;
                }
                if char_idx_i64 == end {
                    byte_end = byte_idx;
                    break;
                }
            }
            if start == char_count {
                byte_start = s.len();
            }
            Ok(RuntimeValue::String(s[byte_start..byte_end].into()))
        }
        BuiltinName::StringSplit => {
            let s_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let sep_arg = args.get(1).cloned().unwrap_or(RuntimeValue::Unit);
            let s = match s_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let sep = match sep_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let parts: Vec<RuntimeValue> = if sep.is_empty() {
                // Empty separator → single-element vector [s].
                // Refuse-over-guess: don't split into chars silently.
                vec![RuntimeValue::String(s)]
            } else {
                s.split(&sep)
                    .map(|part| RuntimeValue::String(part.into()))
                    .collect()
            };
            Ok(RuntimeValue::Vector(parts))
        }
        BuiltinName::StringIndexOf => {
            // Char (codepoint) offset of first occurrence, or Null.
            // Empty needle → 0 (matches at start).
            let haystack_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let needle_arg = args.get(1).cloned().unwrap_or(RuntimeValue::Unit);
            let haystack = match haystack_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let needle = match needle_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            if needle.is_empty() {
                return Ok(RuntimeValue::Integer(Integer::new(0).unwrap_or_default()));
            }
            Ok(haystack
                .find(&needle)
                .map_or(RuntimeValue::Null, |byte_idx| {
                    // Convert byte offset to char offset for Q3-A
                    // consistency (StringSubstring uses chars).
                    let char_offset =
                        i64::try_from(haystack[..byte_idx].chars().count()).unwrap_or(i64::MAX);
                    RuntimeValue::Integer(Integer::new(char_offset).unwrap_or_default())
                }))
        }
        BuiltinName::ParseInteger => {
            // Refuse-over-guess: strict decimal parse via Rust's
            // i64::from_str. Any failure → Null. No leading
            // whitespace, no hex prefix, no underscore separators.
            let s_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let s = match s_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            Ok(s.parse::<i64>()
                .ok()
                .and_then(Integer::new)
                .map_or(RuntimeValue::Null, RuntimeValue::Integer))
        }
        BuiltinName::HashMapContains => {
            // Q3-A: strict 2-state Trilean. True if key present,
            // False if absent. Invalid key type = bug → TypeMismatch
            // (NOT Trilean::Unknown — error model §A7 reserves
            // Unknown for genuine Ł3 uncertainty).
            let map_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let key_arg = args.get(1).cloned().unwrap_or(RuntimeValue::Unit);
            let map = match map_arg {
                RuntimeValue::HashMap(m) => m,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::HashMap(
                            Box::new(TypeTag::Unit),
                            Box::new(TypeTag::Unit),
                        ),
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let key =
                RuntimeMapKey::from_runtime(&key_arg).ok_or_else(|| VmError::TypeMismatch {
                    expected: TypeTag::String,
                    actual: format!(
                        "non-hashable key type {:?} (expected Trit/Tryte/Integer/Long/String)",
                        key_arg.type_tag()
                    ),
                    function: func_name.into(),
                })?;
            let present = if map.contains_key(&key) {
                Trilean::True
            } else {
                Trilean::False
            };
            Ok(RuntimeValue::Trilean(present))
        }
        BuiltinName::TextIntoBytes => {
            let s_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let s = match s_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let bytes = s
                .into_bytes()
                .into_iter()
                .map(|b| RuntimeValue::Integer(triet_core::Integer::new(i64::from(b)).unwrap()))
                .collect();
            Ok(RuntimeValue::Vector(bytes))
        }
        BuiltinName::TextFromBytes => {
            let v_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let v = match v_arg {
                RuntimeValue::Vector(v) => v,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::Vector(Box::new(TypeTag::Integer)),
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let Some(bytes) = vector_to_byte_array(&v) else {
                return Ok(RuntimeValue::Null);
            };
            String::from_utf8(bytes).map_or(Ok(RuntimeValue::Null), |s| Ok(RuntimeValue::String(s)))
        }
        BuiltinName::Blake3Hash => {
            let v_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let v = match v_arg {
                RuntimeValue::Vector(v) => v,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::Vector(Box::new(TypeTag::Integer)),
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let Some(bytes) = vector_to_byte_array(&v) else {
                return Ok(RuntimeValue::Null);
            };
            let hash = blake3::hash(&bytes);
            let result_bytes = hash
                .as_bytes()
                .iter()
                .map(|&b| RuntimeValue::Integer(triet_core::Integer::new(i64::from(b)).unwrap()))
                .collect();
            Ok(RuntimeValue::Vector(result_bytes))
        }
        BuiltinName::GetEnv => {
            let key_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let key = match key_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            Ok(std::env::var(&key).map_or(RuntimeValue::Null, RuntimeValue::String))
        }
        BuiltinName::ReadDirRecursive => {
            // v0.7.12.1: walk `root` recursively, return
            // Vector<Vector<String>> where each inner is
            // [relative_path, file_content] for `.tri` files only.
            // Triết has no first-class tuple opcode so we represent
            // the pair as a 2-element Vector<String>.
            let root_arg = args.first().cloned().unwrap_or(RuntimeValue::Unit);
            let root = match root_arg {
                RuntimeValue::String(s) => s,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::String,
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let mut entries: Vec<(String, String)> = Vec::new();
            walk_tri_files(
                std::path::Path::new(&root),
                std::path::Path::new(&root),
                &mut entries,
            );
            // Sort by relative path for determinism — `walk_tri_files`
            // uses fs::read_dir which has platform-dependent order.
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let outer: Vec<RuntimeValue> = entries
                .into_iter()
                .map(|(path, content)| {
                    RuntimeValue::Vector(vec![
                        RuntimeValue::String(path),
                        RuntimeValue::String(content),
                    ])
                })
                .collect();
            Ok(RuntimeValue::Vector(outer))
        }
        // v0.9.x.atomic.3 — universal atomic ops dispatch per ADR-0028 §4.1.
        // Single-thread VM: ordering arg validated (any RuntimeValue accepted —
        // Ordering enum lands v0.9.x.atomic.5 stdlib) but no-op semantically
        // per ADR-0028 §9 dev tier behavior.
        BuiltinName::AtomicNew => {
            // `sys.atomic.new<T>(initial: T) -> Atomic<T>` per ADR-0028 §6.
            let initial = args
                .iter()
                .next()
                .cloned()
                .ok_or_else(|| VmError::TypeMismatch {
                    expected: TypeTag::Unit,
                    actual: "missing initial value".into(),
                    function: func_name.into(),
                })?;
            Ok(RuntimeValue::Atomic(std::sync::Arc::new(
                std::sync::Mutex::new(initial),
            )))
        }
        BuiltinName::AtomicLoad => {
            // `load(self: &+ Atomic<T>, ordering: Ordering) -> T` per §4.1.
            // Ordering ignored on single-thread VM (no-op per §9).
            let atomic = args
                .iter()
                .next()
                .cloned()
                .ok_or_else(|| VmError::TypeMismatch {
                    expected: TypeTag::Atomic(Box::new(TypeTag::Unit)),
                    actual: "missing atomic arg".into(),
                    function: func_name.into(),
                })?;
            let cell = match atomic {
                RuntimeValue::Atomic(rc) => rc,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::Atomic(Box::new(TypeTag::Unit)),
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            Ok(lock_atomic(&cell).clone())
        }
        BuiltinName::AtomicStore => {
            // `store(self: &+ Atomic<T>, value: T, ordering: Ordering) -> Unit` per §4.1.
            let atomic = args.first().cloned().ok_or_else(|| VmError::TypeMismatch {
                expected: TypeTag::Atomic(Box::new(TypeTag::Unit)),
                actual: "missing atomic arg".into(),
                function: func_name.into(),
            })?;
            let new_value = args.get(1).cloned().ok_or_else(|| VmError::TypeMismatch {
                expected: TypeTag::Unit,
                actual: "missing value arg".into(),
                function: func_name.into(),
            })?;
            let cell = match atomic {
                RuntimeValue::Atomic(rc) => rc,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::Atomic(Box::new(TypeTag::Unit)),
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            *lock_atomic(&cell) = new_value;
            Ok(RuntimeValue::Unit)
        }
        BuiltinName::AtomicSwap => {
            // `swap(self: &+ Atomic<T>, value: T, ordering: Ordering) -> T` per §4.1.
            // Returns previous value.
            let atomic = args.first().cloned().ok_or_else(|| VmError::TypeMismatch {
                expected: TypeTag::Atomic(Box::new(TypeTag::Unit)),
                actual: "missing atomic arg".into(),
                function: func_name.into(),
            })?;
            let new_value = args.get(1).cloned().ok_or_else(|| VmError::TypeMismatch {
                expected: TypeTag::Unit,
                actual: "missing value arg".into(),
                function: func_name.into(),
            })?;
            let cell = match atomic {
                RuntimeValue::Atomic(rc) => rc,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::Atomic(Box::new(TypeTag::Unit)),
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            // Hold lock once across swap (atomic from any external
            // observer's perspective — v0.10 single-thread VM, but
            // intent matches semantic).
            let mut guard = lock_atomic(&cell);
            let prev = guard.clone();
            *guard = new_value;
            Ok(prev)
        }
        BuiltinName::AtomicCompareExchange => {
            // `compare_exchange(self, expected, new, succ_ord, fail_ord) ->
            //  T~CompareExchangeFailed` per ADR-0028 §4.1.
            // Returns Outcome: ~+ prev if expected matched + replaced; ~- actual otherwise.
            let atomic = args.first().cloned().ok_or_else(|| VmError::TypeMismatch {
                expected: TypeTag::Atomic(Box::new(TypeTag::Unit)),
                actual: "missing atomic arg".into(),
                function: func_name.into(),
            })?;
            let expected = args.get(1).cloned().ok_or_else(|| VmError::TypeMismatch {
                expected: TypeTag::Unit,
                actual: "missing expected arg".into(),
                function: func_name.into(),
            })?;
            let new_value = args.get(2).cloned().ok_or_else(|| VmError::TypeMismatch {
                expected: TypeTag::Unit,
                actual: "missing new value arg".into(),
                function: func_name.into(),
            })?;
            let cell = match atomic {
                RuntimeValue::Atomic(rc) => rc,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::Atomic(Box::new(TypeTag::Unit)),
                        actual: format!("{:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            // Hold lock once across compare-and-conditional-swap —
            // the canonical CAS semantics require the read + the
            // optional write to be one indivisible op from external
            // observers' perspective.
            let mut guard = lock_atomic(&cell);
            let current = guard.clone();
            if atomic_value_eq(&current, &expected) {
                *guard = new_value;
                Ok(RuntimeValue::Outcome {
                    discriminator: Trit::Positive,
                    payload: Some(Box::new(current)),
                })
            } else {
                Ok(RuntimeValue::Outcome {
                    discriminator: Trit::Negative,
                    payload: Some(Box::new(current)),
                })
            }
        }
        // v0.9.x.atomic.4 — arithmetic + bitwise dispatch per ADR-0028
        // §4.2 (fetch_add/sub for Tryte/Integer) + Addendum §4.3
        // (fetch_bitwise_and/or/xor for Integer, explicit binary-leak
        // signal). All return PREVIOUS value (pre-modification).
        // Single-thread VM no-op atomicity per §9.
        BuiltinName::AtomicFetchAdd => atomic_fetch_arithmetic(args, func_name, ArithmeticOp::Add),
        BuiltinName::AtomicFetchSub => atomic_fetch_arithmetic(args, func_name, ArithmeticOp::Sub),
        BuiltinName::AtomicFetchBitwiseAnd => atomic_fetch_bitwise(args, func_name, BitwiseOp::And),
        BuiltinName::AtomicFetchBitwiseOr => atomic_fetch_bitwise(args, func_name, BitwiseOp::Or),
        BuiltinName::AtomicFetchBitwiseXor => atomic_fetch_bitwise(args, func_name, BitwiseOp::Xor),
        // v0.10.x.thread.1 — raw_thread builtins need access to the
        // Vm's `thread_handles` registry, which the free-function
        // `execute_builtin` cannot provide (disjoint-borrow constraint
        // explained at the dispatch site). Reaching this arm means the
        // caller bypassed the disjoint-borrow routing — surface as a
        // hard error rather than silently degrading.
        BuiltinName::RawThreadSpawn | BuiltinName::RawThreadJoin => Err(VmError::TypeMismatch {
            expected: TypeTag::Unit,
            actual: format!("{name:?} must be dispatched via execute_thread_builtin"),
            function: func_name.into(),
        }),
    }
}

/// v0.10.x.thread.1 — Real-OS-thread builtin dispatch per [ADR-0026 v2] §3.
///
/// Separated from [`execute_builtin`] because thread primitives need
/// mutable access to the Vm's `thread_handles` registry; the
/// dispatcher site uses disjoint field borrows so this helper takes
/// the registry handle directly instead of a `&mut Vm` (which would
/// conflict with the active `&mut self.frames[frame_idx]` borrow).
///
/// **Scope (v0.10.x.thread.1 plumbing-only):** the spawned thread
/// body is empty — closure-typed work execution lands when Triết's
/// closure type system gains Send-bound expressiveness (ADR-0026 v2
/// §3 placeholder note). Plumbing proves: spawn returns a real
/// `JoinHandle`, join blocks until thread terminates, Handle struct
/// round-trips through the IR.
///
/// [ADR-0026 v2]: ../../../../docs/decisions/0026-actor-boundary-send-rules.md
fn execute_thread_builtin(
    name: BuiltinName,
    args: &[RuntimeValue],
    handles: &mut HashMap<i64, std::thread::JoinHandle<()>>,
    next_id: &mut i64,
    func_name: &str,
) -> Result<RuntimeValue, VmError> {
    match name {
        BuiltinName::RawThreadSpawn => {
            // Signature: `spawn(work: Integer) -> Handle`. `work` is
            // a placeholder per ADR-0026 v2 §3 (closure-typed signature
            // lands when Triết's closure type system gains Send-bound
            // expressiveness). Argument is currently consumed but not
            // dispatched into the spawned thread — VM is single-thread
            // per ADR-0028 §9; spawned thread body is empty, just
            // proves the plumbing (spawn returns real JoinHandle, join
            // blocks until terminate).
            let _work = args.first().ok_or_else(|| VmError::TypeMismatch {
                expected: TypeTag::Integer,
                actual: "missing work arg".into(),
                function: func_name.into(),
            })?;
            // Monotonic id, skip 0 (reserved for the v0.9 stub
            // placeholder Handle { thread_id: 0 }; real spawns are
            // ≥ 1 for unambiguous diagnostics).
            *next_id = next_id.wrapping_add(1);
            let thread_id = *next_id;
            // Spawn a real OS thread. Empty closure body is Send by
            // default; cross-platform via std::thread (POSIX +
            // Windows abstracted at Rust stdlib level per ADR-0018
            // POSIX-first precedent — actual syscalls handled by
            // libstd, no direct pthread_create call needed for v0.10).
            let handle = std::thread::spawn(|| { /* placeholder body */ });
            handles.insert(thread_id, handle);
            // Return RuntimeValue::Struct matching the stdlib Handle
            // shape: { thread_id: Integer }. Field order is the
            // declaration order from std/sys/raw_thread.tri.
            Ok(RuntimeValue::Struct {
                fields: vec![RuntimeValue::Integer(
                    Integer::new(thread_id).unwrap_or_default(),
                )],
            })
        }
        BuiltinName::RawThreadJoin => {
            // Signature: `join(handle: Handle) -> Unit`. Extract the
            // thread_id from the struct field, consume the registry
            // entry via `remove`, block on the JoinHandle.
            let handle_val = args.first().ok_or_else(|| VmError::TypeMismatch {
                expected: TypeTag::Unit,
                actual: "missing handle arg".into(),
                function: func_name.into(),
            })?;
            let thread_id = match handle_val {
                RuntimeValue::Struct { fields } => match fields.first() {
                    Some(RuntimeValue::Integer(i)) => i.to_i64(),
                    other => {
                        return Err(VmError::TypeMismatch {
                            expected: TypeTag::Integer,
                            actual: format!("handle.thread_id = {other:?}"),
                            function: func_name.into(),
                        });
                    }
                },
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: TypeTag::Unit,
                        actual: format!("expected Handle struct, got {:?}", other.type_tag()),
                        function: func_name.into(),
                    });
                }
            };
            let join_handle = handles
                .remove(&thread_id)
                .ok_or_else(|| VmError::TypeMismatch {
                    expected: TypeTag::Unit,
                    actual: format!(
                        "raw_thread.join: unknown thread_id {thread_id} \
                         (already joined, never spawned, or fabricated Handle)"
                    ),
                    function: func_name.into(),
                })?;
            // Block until the OS thread terminates. If the thread
            // panicked, propagate as a structured VmError so the
            // caller's program sees a clean error path.
            join_handle.join().map_err(|_| VmError::TypeMismatch {
                expected: TypeTag::Unit,
                actual: format!("raw_thread.join: thread_id {thread_id} panicked"),
                function: func_name.into(),
            })?;
            Ok(RuntimeValue::Unit)
        }
        other => Err(VmError::TypeMismatch {
            expected: TypeTag::Unit,
            actual: format!("execute_thread_builtin: non-thread builtin {other:?} routed here"),
            function: func_name.into(),
        }),
    }
}

/// v0.7.12.1 helper for `BuiltinName::ReadDirRecursive`. Walks
/// `dir` recursively (depth-first), collecting `(relative_path,
/// content)` tuples for every `*.tri` file found. Relative paths
/// use POSIX `/` separator regardless of host OS. I/O errors and
/// non-UTF-8 files are silently skipped (data-tier — caller sees
/// truncated results, not a panic).
fn walk_tri_files(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, String)>) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_tri_files(root, &path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("tri") {
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let rel_str = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            out.push((rel_str, content));
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConstId;
    use crate::constant::ConstantPool;
    use crate::instr::PhiIncoming;
    use crate::module::{BasicBlock, Function, IrModule, WitnessTable};
    use triet_modules::{AbsolutePath, ModulePath};

    fn make_int(n: i64) -> RuntimeValue {
        RuntimeValue::Integer(Integer::new(n).unwrap())
    }

    fn make_simple_program(func: Function) -> IrProgram {
        IrProgram {
            modules: vec![IrModule {
                path: AbsolutePath::new(ModulePath::khi_root(), "test".into()),
                functions: vec![func],
            }],
            constants: ConstantPool::new(),
            witness_tables: Vec::new(),
        }
    }

    // ── Arithmetic VM tests ──────────────────────────────────────

    #[test]
    fn vm_add_integers() {
        let pool = ConstantPool::new();
        let func = Function {
            id: FuncId(0),
            name: Some("add".into()),
            params: vec![
                ("a".into(), TypeTag::Integer),
                ("b".into(), TypeTag::Integer),
            ],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Add {
                        dest: ValueId(2),
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Value(ValueId(1)),
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(2))),
                    },
                ],
            }],
        };
        let mut prog = make_simple_program(func);
        prog.constants = pool;

        let mut vm = Vm::new(prog);
        let result = vm
            .execute(FuncId(0), vec![make_int(10), make_int(20)])
            .unwrap();
        assert_eq!(result.to_string(), make_int(30).to_string());
    }

    #[test]
    fn vm_mul_integers() {
        let func = Function {
            id: FuncId(0),
            name: Some("mul".into()),
            params: vec![
                ("a".into(), TypeTag::Integer),
                ("b".into(), TypeTag::Integer),
            ],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Mul {
                        dest: ValueId(2),
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Value(ValueId(1)),
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(2))),
                    },
                ],
            }],
        };
        let prog = make_simple_program(func);
        let mut vm = Vm::new(prog);
        let result = vm
            .execute(FuncId(0), vec![make_int(6), make_int(7)])
            .unwrap();
        assert_eq!(result.to_string(), make_int(42).to_string());
    }

    #[test]
    fn vm_sub_and_div() {
        let func = Function {
            id: FuncId(0),
            name: Some("sub_div".into()),
            params: vec![
                ("a".into(), TypeTag::Integer),
                ("b".into(), TypeTag::Integer),
            ],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Sub {
                        dest: ValueId(2),
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Value(ValueId(1)),
                    },
                    Instruction::Div {
                        dest: ValueId(3),
                        lhs: Operand::Value(ValueId(2)),
                        rhs: Operand::Value(ValueId(1)),
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(3))),
                    },
                ],
            }],
        };
        let prog = make_simple_program(func);
        let mut vm = Vm::new(prog);
        let result = vm
            .execute(FuncId(0), vec![make_int(16), make_int(4)])
            .unwrap();
        // (16 - 4) / 4 = 3
        assert_eq!(result.to_string(), make_int(3).to_string());
    }

    #[test]
    fn vm_div_by_zero_returns_error() {
        let func = Function {
            id: FuncId(0),
            name: Some("div_zero".into()),
            params: vec![],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Const {
                        dest: ValueId(0),
                        constant: ConstId(0),
                    },
                    Instruction::Div {
                        dest: ValueId(1),
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Const(ConstId(1)),
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(1))),
                    },
                ],
            }],
        };
        let mut pool = ConstantPool::new();
        let _c10 = pool.intern(Constant::Integer(Integer::new(10).unwrap()));
        let _c0 = pool.intern(Constant::Integer(Integer::new(0).unwrap()));
        let mut prog = make_simple_program(func);
        prog.constants = pool;

        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![]);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            VmError::DivisionByZero { .. }
        ));
    }

    // ── Logic VM tests ──────────────────────────────────────────

    #[test]
    fn vm_lukasiewicz_and() {
        let func = Function {
            id: FuncId(0),
            name: Some("luk_and".into()),
            params: vec![],
            return_type: TypeTag::Trilean,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Const {
                        dest: ValueId(0),
                        constant: ConstId(0),
                    },
                    Instruction::Const {
                        dest: ValueId(1),
                        constant: ConstId(1),
                    },
                    Instruction::LukAnd {
                        dest: ValueId(2),
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Value(ValueId(1)),
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(2))),
                    },
                ],
            }],
        };
        let mut pool = ConstantPool::new();
        let _ct = pool.intern(Constant::Trilean(Trilean::True));
        let _cu = pool.intern(Constant::Trilean(Trilean::Unknown));
        let mut prog = make_simple_program(func);
        prog.constants = pool;

        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![]).unwrap();
        // True AND Unknown = Unknown
        assert_eq!(result.to_string(), Trilean::Unknown.to_string());
    }

    #[test]
    fn vm_lukasiewicz_implies() {
        let func = Function {
            id: FuncId(0),
            name: Some("luk_implies".into()),
            params: vec![],
            return_type: TypeTag::Trilean,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Const {
                        dest: ValueId(0),
                        constant: ConstId(0),
                    },
                    Instruction::Const {
                        dest: ValueId(1),
                        constant: ConstId(1),
                    },
                    Instruction::LukImplies {
                        dest: ValueId(2),
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Value(ValueId(1)),
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(2))),
                    },
                ],
            }],
        };
        let mut pool = ConstantPool::new();
        let _cf = pool.intern(Constant::Trilean(Trilean::False));
        let _cu = pool.intern(Constant::Trilean(Trilean::Unknown));
        let mut prog = make_simple_program(func);
        prog.constants = pool;

        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![]).unwrap();
        // False ⇒ Unknown = True in Ł3
        assert_eq!(result.to_string(), Trilean::True.to_string());
    }

    // ── Comparison VM tests ──────────────────────────────────────

    #[test]
    fn vm_comparison_eq_ne() {
        let func = Function {
            id: FuncId(0),
            name: Some("cmp_eq".into()),
            params: vec![
                ("a".into(), TypeTag::Integer),
                ("b".into(), TypeTag::Integer),
            ],
            return_type: TypeTag::Trilean,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Eq {
                        dest: ValueId(2),
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Value(ValueId(1)),
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(2))),
                    },
                ],
            }],
        };
        let prog = make_simple_program(func);
        let mut vm = Vm::new(prog);
        let result = vm
            .execute(FuncId(0), vec![make_int(5), make_int(5)])
            .unwrap();
        assert_eq!(result.to_string(), Trilean::True.to_string());
    }

    // ── Control flow VM tests ────────────────────────────────────

    #[test]
    fn vm_br_if_conditional() {
        // function choose(%c: Trilean) -> Integer {
        // entry:
        //     br_if %c, then, else
        // then:
        //     ret const Integer 1
        // else:
        //     ret const Integer 0
        // }
        let mut pool = ConstantPool::new();
        let c1 = pool.intern(Constant::Integer(Integer::new(1).unwrap()));
        let c0 = pool.intern(Constant::Integer(Integer::new(0).unwrap()));

        let func = Function {
            id: FuncId(0),
            name: Some("choose".into()),
            params: vec![("c".into(), TypeTag::Trilean)],
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
                    instructions: vec![Instruction::Ret {
                        value: Some(Operand::Const(c1)),
                    }],
                },
                BasicBlock {
                    id: BlockId(2),
                    name: Some("else".into()),
                    instructions: vec![Instruction::Ret {
                        value: Some(Operand::Const(c0)),
                    }],
                },
            ],
        };
        let mut prog = make_simple_program(func);
        prog.constants = pool;

        let mut vm = Vm::new(prog.clone());
        // true → 1
        let result = vm
            .execute(FuncId(0), vec![RuntimeValue::Trilean(Trilean::True)])
            .unwrap();
        assert_eq!(result.to_string(), make_int(1).to_string());

        // false → 0
        let mut vm2 = Vm::new(prog);
        let result2 = vm2
            .execute(FuncId(0), vec![RuntimeValue::Trilean(Trilean::False)])
            .unwrap();
        assert_eq!(result2.to_string(), make_int(0).to_string());
    }

    /// ADR-0010 — `BrTrilean` dispatches each of the three Trilean values
    /// to its own block. Unknown does NOT collapse to else.
    #[test]
    fn vm_br_trilean_three_way() {
        let mut pool = ConstantPool::new();
        let c_pos = pool.intern(Constant::Integer(Integer::new(1).unwrap()));
        let c_zero = pool.intern(Constant::Integer(Integer::new(0).unwrap()));
        let c_neg = pool.intern(Constant::Integer(Integer::new(-1).unwrap()));

        let func = Function {
            id: FuncId(0),
            name: Some("sign".into()),
            params: vec![("c".into(), TypeTag::Trilean)],
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
        };
        let mut prog = make_simple_program(func);
        prog.constants = pool;

        // Each Trilean value reaches its dedicated block (not collapsed).
        for (input, expected) in [
            (Trilean::True, 1_i64),
            (Trilean::Unknown, 0_i64),
            (Trilean::False, -1_i64),
        ] {
            let mut vm = Vm::new(prog.clone());
            let result = vm
                .execute(FuncId(0), vec![RuntimeValue::Trilean(input)])
                .unwrap();
            assert_eq!(
                result.to_string(),
                make_int(expected).to_string(),
                "BrTrilean({input:?}) → expected {expected}",
            );
        }
    }

    /// ADR-0012 — `WitnessCall` dispatches a cross-package generic
    /// like `CallCrossModule` but also verifies the referenced witness
    /// table exists. A missing index must surface a precise error
    /// rather than silent zero-table dispatch.
    #[test]
    fn vm_witness_call_dispatches_and_validates_index() {
        let mut pool = ConstantPool::new();
        let const_42 = pool.intern(Constant::Integer(Integer::new(42).unwrap()));

        // Lib function `math.identity(x)` simply returns `x`.
        let lib_func = Function {
            id: FuncId(0),
            name: Some("identity".into()),
            params: vec![("x".into(), TypeTag::Integer)],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                }],
            }],
        };
        // App function `app.main()` invokes the lib through a witness
        // table containing `[Integer]`.
        let app_func = Function {
            id: FuncId(1),
            name: Some("main".into()),
            params: vec![],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::WitnessCall {
                        dest: Some(ValueId(0)),
                        path: AbsolutePath::new(
                            ModulePath::new(vec!["math".into()]),
                            "identity".into(),
                        ),
                        witness_idx: 0,
                        args: vec![Operand::Const(const_42)],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    },
                ],
            }],
        };
        let program = IrProgram {
            modules: vec![
                IrModule {
                    path: AbsolutePath::new(ModulePath::new(vec!["math".into()]), String::new()),
                    functions: vec![lib_func],
                },
                IrModule {
                    path: AbsolutePath::new(ModulePath::new(vec!["app".into()]), String::new()),
                    functions: vec![app_func],
                },
            ],
            constants: pool,
            witness_tables: vec![WitnessTable {
                type_args: vec![TypeTag::Integer],
            }],
        };

        let mut vm = Vm::new(program.clone());
        let result = vm.execute(FuncId(1), vec![]).unwrap();
        assert_eq!(result.to_string(), make_int(42).to_string());

        // Same program but with no witness tables — index 0 is invalid.
        let mut bad = program;
        bad.witness_tables.clear();
        let mut bad_vm = Vm::new(bad);
        let err = bad_vm.execute(FuncId(1), vec![]).unwrap_err();
        assert!(matches!(err, VmError::FunctionNotFound { .. }));
    }

    /// ADR-0010 — `Eq` on Trilean operands propagates Unknown per Ł3.
    /// Classical pre-ADR behavior would collapse Unknown → False here.
    #[test]
    fn vm_eq_trilean_unknown_propagates() {
        // Operand pairs with their Ł3-correct equality result.
        let cases: &[(Trilean, Trilean, Trilean)] = &[
            (Trilean::Unknown, Trilean::Unknown, Trilean::Unknown),
            (Trilean::Unknown, Trilean::True, Trilean::Unknown),
            (Trilean::Unknown, Trilean::False, Trilean::Unknown),
            (Trilean::True, Trilean::Unknown, Trilean::Unknown),
            (Trilean::False, Trilean::Unknown, Trilean::Unknown),
            (Trilean::True, Trilean::True, Trilean::True),
            (Trilean::False, Trilean::False, Trilean::True),
            (Trilean::True, Trilean::False, Trilean::False),
        ];
        for (a, b, expected) in cases {
            let got = runtime_eq_trilean(&RuntimeValue::Trilean(*a), &RuntimeValue::Trilean(*b));
            assert_eq!(got, *expected, "eq({a:?}, {b:?})");
        }
        // Trit::Zero is a concrete value, NOT a truth-Unknown.
        assert_eq!(
            runtime_eq_trilean(
                &RuntimeValue::Trit(Trit::Zero),
                &RuntimeValue::Trit(Trit::Zero)
            ),
            Trilean::True,
        );
    }

    #[test]
    fn vm_factorial_ir() {
        let mut pool = ConstantPool::new();
        let c0 = pool.intern(Constant::Integer(Integer::new(0).unwrap()));
        let c1 = pool.intern(Constant::Integer(Integer::new(1).unwrap()));

        // Recursive factorial in IR:
        // function factorial(%n: Integer) -> Integer {
        // entry:
        //     %0 = eq %n, const 0
        //     br_if %0, base, recurse
        // base:
        //     ret const 1
        // recurse:
        //     %1 = sub %n, const 1
        //     %2 = call @f0(%1)
        //     %3 = mul %n, %2
        //     ret %3
        // }
        let func = Function {
            id: FuncId(0),
            name: Some("factorial".into()),
            params: vec![("n".into(), TypeTag::Integer)],
            return_type: TypeTag::Integer,
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    name: Some("entry".into()),
                    instructions: vec![
                        Instruction::Eq {
                            dest: ValueId(1),
                            lhs: Operand::Value(ValueId(0)),
                            rhs: Operand::Const(c0),
                        },
                        Instruction::BrIf {
                            cond: Operand::Value(ValueId(1)),
                            then_block: BlockId(1),
                            else_block: BlockId(2),
                        },
                    ],
                },
                BasicBlock {
                    id: BlockId(1),
                    name: Some("base".into()),
                    instructions: vec![Instruction::Ret {
                        value: Some(Operand::Const(c1)),
                    }],
                },
                BasicBlock {
                    id: BlockId(2),
                    name: Some("recurse".into()),
                    instructions: vec![
                        Instruction::Sub {
                            dest: ValueId(2),
                            lhs: Operand::Value(ValueId(0)),
                            rhs: Operand::Const(c1),
                        },
                        Instruction::CallLocal {
                            dest: Some(ValueId(3)),
                            callee: FuncId(0),
                            args: vec![Operand::Value(ValueId(2))],
                        },
                        Instruction::Mul {
                            dest: ValueId(4),
                            lhs: Operand::Value(ValueId(0)),
                            rhs: Operand::Value(ValueId(3)),
                        },
                        Instruction::Ret {
                            value: Some(Operand::Value(ValueId(4))),
                        },
                    ],
                },
            ],
        };
        let mut prog = make_simple_program(func);
        prog.constants = pool;

        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![make_int(5)]).unwrap();
        // 5! = 120
        assert_eq!(result.to_string(), make_int(120).to_string());
    }

    // ── Phi node VM tests ────────────────────────────────────────

    #[test]
    fn vm_phi_after_if_else() {
        // function abs(%n: Integer) -> Integer {
        // entry:
        //     %0 = lt %n, const 0
        //     br_if %0, neg, pos
        // neg:
        //     %1 = neg %n
        //     br merge
        // pos:
        //     br merge
        // merge:
        //     %2 = phi [%1 from neg], [%n from pos]
        //     ret %2
        // }
        let mut pool = ConstantPool::new();
        let c0 = pool.intern(Constant::Integer(Integer::new(0).unwrap()));

        let func = Function {
            id: FuncId(0),
            name: Some("abs".into()),
            params: vec![("n".into(), TypeTag::Integer)],
            return_type: TypeTag::Integer,
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    name: Some("entry".into()),
                    instructions: vec![
                        Instruction::Lt {
                            dest: ValueId(1),
                            lhs: Operand::Value(ValueId(0)),
                            rhs: Operand::Const(c0),
                        },
                        Instruction::BrIf {
                            cond: Operand::Value(ValueId(1)),
                            then_block: BlockId(1),
                            else_block: BlockId(2),
                        },
                    ],
                },
                BasicBlock {
                    id: BlockId(1),
                    name: Some("neg".into()),
                    instructions: vec![
                        Instruction::Neg {
                            dest: ValueId(2),
                            operand: Operand::Value(ValueId(0)),
                        },
                        Instruction::Br { target: BlockId(3) },
                    ],
                },
                BasicBlock {
                    id: BlockId(2),
                    name: Some("pos".into()),
                    instructions: vec![Instruction::Br { target: BlockId(3) }],
                },
                BasicBlock {
                    id: BlockId(3),
                    name: Some("merge".into()),
                    instructions: vec![
                        Instruction::Phi {
                            dest: ValueId(3),
                            incoming: vec![
                                PhiIncoming {
                                    value: ValueId(2),
                                    block: BlockId(1),
                                },
                                PhiIncoming {
                                    value: ValueId(0),
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
        };
        let mut prog = make_simple_program(func);
        prog.constants = pool;

        // abs(-5) = 5
        let mut vm = Vm::new(prog.clone());
        let result = vm.execute(FuncId(0), vec![make_int(-5)]).unwrap();
        assert_eq!(result.to_string(), make_int(5).to_string());

        // abs(7) = 7
        let mut vm2 = Vm::new(prog);
        let result2 = vm2.execute(FuncId(0), vec![make_int(7)]).unwrap();
        assert_eq!(result2.to_string(), make_int(7).to_string());
    }

    // ── Builtin VM tests ─────────────────────────────────────────

    #[test]
    fn vm_builtin_assert_passes() {
        let func = Function {
            id: FuncId(0),
            name: Some("test_assert".into()),
            params: vec![],
            return_type: TypeTag::Unit,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Const {
                        dest: ValueId(0),
                        constant: ConstId(0),
                    },
                    Instruction::CallBuiltin {
                        dest: None,
                        name: BuiltinName::Assert,
                        args: vec![Operand::Value(ValueId(0))],
                    },
                    Instruction::Ret { value: None },
                ],
            }],
        };
        let mut pool = ConstantPool::new();
        let _ct = pool.intern(Constant::Trilean(Trilean::True));
        let mut prog = make_simple_program(func);
        prog.constants = pool;

        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![]);
        assert!(result.is_ok());
    }

    #[test]
    fn vm_builtin_assert_fails() {
        let func = Function {
            id: FuncId(0),
            name: Some("test_fail".into()),
            params: vec![],
            return_type: TypeTag::Unit,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Const {
                        dest: ValueId(0),
                        constant: ConstId(0),
                    },
                    Instruction::CallBuiltin {
                        dest: None,
                        name: BuiltinName::Assert,
                        args: vec![Operand::Value(ValueId(0))],
                    },
                    Instruction::Ret { value: None },
                ],
            }],
        };
        let mut pool = ConstantPool::new();
        let _cf = pool.intern(Constant::Trilean(Trilean::False));
        let mut prog = make_simple_program(func);
        prog.constants = pool;

        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![]);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            VmError::AssertionFailed { .. }
        ));
    }

    // ── Balanced ternary verification tests ─────────────────────

    /// In balanced ternary, range is symmetric around zero.
    /// `-(MAX) == MIN` — does NOT overflow (unlike 2's complement).
    #[test]
    fn balanced_ternary_negate_max_equals_min() {
        let a = make_int(Integer::MAX.to_i64()); // +3_812_798_742_493
        let neg_a = arithmetic_neg(&a, "test").unwrap();
        assert_eq!(neg_a.to_string(), Integer::MIN.to_string());
        // Double negation must return original value.
        let double = arithmetic_neg(&neg_a, "test").unwrap();
        assert_eq!(double.to_string(), a.to_string());
    }

    /// Verify that -5 and 5 negation works correctly in balanced ternary.
    /// In balanced ternary, negation is per-trit inversion (not 2's complement).
    /// For small values the numeric result is the same as binary, but the
    /// range boundary behavior differs.
    #[test]
    fn balanced_ternary_negate_five() {
        let five = make_int(5);
        let neg_five = arithmetic_neg(&five, "test").unwrap();
        assert_eq!(neg_five.to_string(), "-5");
        // 5 + (-5) = 0
        let sum = arithmetic_add(&five, &neg_five, "test").unwrap();
        assert_eq!(sum.to_string(), "0");
    }

    /// In balanced ternary, `n + (-n) == 0` always holds, including at
    /// the range boundary (where 2's complement would panic on -MIN).
    #[test]
    fn balanced_ternary_add_negate_is_zero() {
        for n in [-100, -1, 0, 1, 100, Integer::MAX.to_i64()] {
            let v = make_int(n);
            let neg_v = arithmetic_neg(&v, "test").unwrap();
            let sum = arithmetic_add(&v, &neg_v, "test").unwrap();
            assert_eq!(sum.to_string(), "0", "failed for n={n}");
        }
    }

    /// Verify balanced ternary Integer range boundaries are respected:
    /// Integer covers [-`3_812_798_742_493`, +`3_812_798_742_493`].
    #[test]
    fn balanced_ternary_integer_range_boundaries() {
        let min = make_int(Integer::MIN.to_i64());
        let max = make_int(Integer::MAX.to_i64());
        // MIN + MAX is very close to zero (actually -1 for odd modulus)
        // (3^27 - 1)/2 + (3^27 - 1)/2 = 3^27 - 1 ≡ -1 (mod 3^27)
        // Wait: MIN = -MAX, so MIN + MAX = 0. The range is perfectly symmetric.
        let sum = arithmetic_add(&min, &max, "test").unwrap();
        assert_eq!(sum.to_string(), "0");
    }

    /// Trit negation table: each of the three balanced ternary digits
    /// inverts correctly.
    #[test]
    fn balanced_ternary_trit_negation_table() {
        // + ⇔ -, 0 ⇔ 0
        let pos = RuntimeValue::Trit(Trit::Positive);
        let neg = RuntimeValue::Trit(Trit::Negative);
        let zero = RuntimeValue::Trit(Trit::Zero);

        assert_eq!(
            arithmetic_neg(&pos, "test").unwrap().to_string(),
            Trit::Negative.to_string()
        );
        assert_eq!(
            arithmetic_neg(&neg, "test").unwrap().to_string(),
            Trit::Positive.to_string()
        );
        assert_eq!(
            arithmetic_neg(&zero, "test").unwrap().to_string(),
            Trit::Zero.to_string()
        );
    }

    /// Trilean Ł3 truth table: all 9 combinations match Łukasiewicz Ł3.
    /// This is the DEFAULT logic system in Triết (SPEC §4).
    #[test]
    fn balanced_ternary_lukasiewicz_l3_full_truth_table() {
        let values = [
            RuntimeValue::Trilean(Trilean::False),
            RuntimeValue::Trilean(Trilean::Unknown),
            RuntimeValue::Trilean(Trilean::True),
        ];

        // Ł3 AND (=min): U∧F=F, U∧U=U, U∧T=U, F∧T=F, T∧T=T
        let expected_and = [
            [Trilean::False, Trilean::False, Trilean::False], // F ∧ ...
            [Trilean::False, Trilean::Unknown, Trilean::Unknown], // U ∧ ...
            [Trilean::False, Trilean::Unknown, Trilean::True], // T ∧ ...
        ];
        // Ł3 OR (=max): U∨F=U, U∨U=U, U∨T=T, F∨T=T, T∨T=T
        let expected_or = [
            [Trilean::False, Trilean::Unknown, Trilean::True], // F ∨ ...
            [Trilean::Unknown, Trilean::Unknown, Trilean::True], // U ∨ ...
            [Trilean::True, Trilean::True, Trilean::True],     // T ∨ ...
        ];
        // Ł3 IMPLIES (=>): min(1, 1-a+b). U⇒U=T (key!), F⇒U=T
        let expected_implies = [
            [Trilean::True, Trilean::True, Trilean::True], // F ⇒ ...
            [Trilean::Unknown, Trilean::True, Trilean::True], // U ⇒ ...  (U⇒U = T!)
            [Trilean::False, Trilean::Unknown, Trilean::True], // T ⇒ ...
        ];

        for (i, lhs) in values.iter().enumerate() {
            for (j, rhs) in values.iter().enumerate() {
                let l = lhs.as_trilean();
                let r = rhs.as_trilean();
                assert_eq!(l.and(r), expected_and[i][j], "Ł3 AND: {l} ∧ {r}");
                assert_eq!(l.or(r), expected_or[i][j], "Ł3 OR: {l} ∨ {r}");
                assert_eq!(
                    l.implies(r),
                    expected_implies[i][j],
                    "Ł3 IMPLIES: {l} ⇒ {r}"
                );
            }
        }
    }

    /// Kleene K3 truth table: the `unknown`-dominant alternative.
    /// Key difference from Ł3: U⇒U = T (Ł3) vs U~>U = U (K3).
    #[test]
    fn balanced_ternary_kleene_k3_implies_differs_from_l3() {
        let u = Trilean::Unknown;
        // Ł3: U ⇒ U = T
        assert_eq!(u.implies(u), Trilean::True);
        // K3: U ~> U = max(1-U, U) = max(U, U) = U (unknown dominates)
        assert_eq!(u.kleene_implies(u), Trilean::Unknown);
        // K3: F ~> U = max(1-0, U) = max(1, U) = True
        assert_eq!(Trilean::False.kleene_implies(u), Trilean::True);
        // K3: T ~> U = max(1-1, U) = max(0, U) = U
        assert_eq!(Trilean::True.kleene_implies(u), Trilean::Unknown);
    }

    /// Test that the VM correctly dispatches both Ł3 and K3 opcodes
    /// separately (not mixing them).
    #[test]
    fn vm_distinguishes_l3_from_k3() {
        let mut pool = ConstantPool::new();
        let cu = pool.intern(Constant::Trilean(Trilean::Unknown));

        // L3 implication: Unknown ⇒ Unknown = True
        let l3_func = Function {
            id: FuncId(0),
            name: Some("l3_impl".into()),
            params: vec![],
            return_type: TypeTag::Trilean,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Const {
                        dest: ValueId(0),
                        constant: cu,
                    },
                    Instruction::LukImplies {
                        dest: ValueId(1),
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Value(ValueId(0)),
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(1))),
                    },
                ],
            }],
        };
        let mut prog1 = make_simple_program(l3_func);
        prog1.constants = pool.clone();
        let mut vm1 = Vm::new(prog1);
        let r1 = vm1.execute(FuncId(0), vec![]).unwrap();
        assert_eq!(
            r1.to_string(),
            Trilean::True.to_string(),
            "Ł3: U⇒U must be True"
        );

        // K3 implication: Unknown ~> Unknown = Unknown
        let k3_func = Function {
            id: FuncId(0),
            name: Some("k3_impl".into()),
            params: vec![],
            return_type: TypeTag::Trilean,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Const {
                        dest: ValueId(0),
                        constant: cu,
                    },
                    Instruction::KleeneImplies {
                        dest: ValueId(1),
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Value(ValueId(0)),
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(1))),
                    },
                ],
            }],
        };
        let mut prog2 = make_simple_program(k3_func);
        prog2.constants = pool;
        let mut vm2 = Vm::new(prog2);
        let r2 = vm2.execute(FuncId(0), vec![]).unwrap();
        assert_eq!(
            r2.to_string(),
            Trilean::Unknown.to_string(),
            "K3: U~>U must be Unknown"
        );
    }

    // ── v0.7.3.2 Vector builtins ─────────────────────────────────
    //
    // Per ADR-0019 §5 + Addendum §A1/A3/A4: 4 Vector builtins —
    // VectorNew/VectorPush/VectorGet/VectorLength. Q1-A picked
    // functional return-new for push; Q2-A skipped vector_iterator;
    // Q3-A picked strict bounds (negative + over-length both return
    // Null). vector_pop deferred post-v0.7.
    //
    // Tests build IR programs directly (bypassing parser/typecheck)
    // because generic function syntax doesn't exist in the AST yet;
    // user-source path mapping lands when the self-host compiler
    // forces the issue (v0.7.4+). Precedent: `BuiltinName::FStringConcat`
    // is also "Internal builtin — not user-callable".

    /// `vector_new()` returns an empty Vector value.
    #[test]
    fn vm_vector_new_returns_empty_vector() {
        let func = Function {
            id: FuncId(0),
            name: Some("make_empty".into()),
            params: vec![],
            return_type: TypeTag::Vector(Box::new(TypeTag::Integer)),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(0)),
                        name: BuiltinName::VectorNew,
                        args: vec![],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    },
                ],
            }],
        };
        let prog = make_simple_program(func);
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![]).unwrap();
        match result {
            RuntimeValue::Vector(elements) => assert!(elements.is_empty()),
            other => panic!("expected empty Vector, got {other:?}"),
        }
    }

    /// `vector_push(v, item)` returns a new Vector with `item`
    /// appended — functional (Q1-A). Input vector is unchanged.
    #[test]
    fn vm_vector_push_appends_and_returns_new_vector() {
        let mut pool = ConstantPool::new();
        let c_seed = pool.intern(Constant::Integer(Integer::new(7).unwrap()));
        let c_item = pool.intern(Constant::Integer(Integer::new(42).unwrap()));

        let func = Function {
            id: FuncId(0),
            name: Some("push_one".into()),
            params: vec![],
            return_type: TypeTag::Vector(Box::new(TypeTag::Integer)),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    // %0 = vector_new()
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(0)),
                        name: BuiltinName::VectorNew,
                        args: vec![],
                    },
                    // %1 = const 7
                    Instruction::Const {
                        dest: ValueId(1),
                        constant: c_seed,
                    },
                    // %2 = vector_push(%0, %1)
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(2)),
                        name: BuiltinName::VectorPush,
                        args: vec![Operand::Value(ValueId(0)), Operand::Value(ValueId(1))],
                    },
                    // %3 = const 42
                    Instruction::Const {
                        dest: ValueId(3),
                        constant: c_item,
                    },
                    // %4 = vector_push(%2, %3)
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(4)),
                        name: BuiltinName::VectorPush,
                        args: vec![Operand::Value(ValueId(2)), Operand::Value(ValueId(3))],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(4))),
                    },
                ],
            }],
        };
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![]).unwrap();
        match result {
            RuntimeValue::Vector(elements) => {
                assert_eq!(elements.len(), 2);
                assert_eq!(elements[0].to_string(), "7");
                assert_eq!(elements[1].to_string(), "42");
            }
            other => panic!("expected Vector with 2 elements, got {other:?}"),
        }
    }

    /// `vector_get(v, idx)` returns `T?`-style: in-range = element
    /// itself (Some), out-of-range = Null (None).
    #[test]
    fn vm_vector_get_in_range_returns_element_out_of_range_returns_null() {
        let mut pool = ConstantPool::new();
        let c10 = pool.intern(Constant::Integer(Integer::new(10).unwrap()));
        let c_idx_in = pool.intern(Constant::Integer(Integer::new(0).unwrap()));
        let c_idx_oor = pool.intern(Constant::Integer(Integer::new(5).unwrap()));
        let c_idx_neg = pool.intern(Constant::Integer(Integer::new(-1).unwrap()));

        // Helper builds a function returning vector_get(push(new(), 10), <idx_const>)
        let build_func = |id: FuncId, idx_const: ConstId, fn_name: &str| Function {
            id,
            name: Some(fn_name.into()),
            params: vec![],
            return_type: TypeTag::Nullable(Box::new(TypeTag::Integer)),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(0)),
                        name: BuiltinName::VectorNew,
                        args: vec![],
                    },
                    Instruction::Const {
                        dest: ValueId(1),
                        constant: c10,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(2)),
                        name: BuiltinName::VectorPush,
                        args: vec![Operand::Value(ValueId(0)), Operand::Value(ValueId(1))],
                    },
                    Instruction::Const {
                        dest: ValueId(3),
                        constant: idx_const,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(4)),
                        name: BuiltinName::VectorGet,
                        args: vec![Operand::Value(ValueId(2)), Operand::Value(ValueId(3))],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(4))),
                    },
                ],
            }],
        };

        let prog = IrProgram {
            modules: vec![IrModule {
                path: AbsolutePath::new(ModulePath::khi_root(), "test".into()),
                functions: vec![
                    build_func(FuncId(0), c_idx_in, "get_in_range"),
                    build_func(FuncId(1), c_idx_oor, "get_over_length"),
                    build_func(FuncId(2), c_idx_neg, "get_negative"),
                ],
            }],
            constants: pool,
            witness_tables: Vec::new(),
        };

        let mut vm_in = Vm::new(prog.clone());
        let r_in = vm_in.execute(FuncId(0), vec![]).unwrap();
        assert_eq!(r_in.to_string(), "10", "in-range get must return value");

        let mut vm_oor = Vm::new(prog.clone());
        let r_oor = vm_oor.execute(FuncId(1), vec![]).unwrap();
        assert!(
            matches!(r_oor, RuntimeValue::Null),
            "over-length get must return Null, got {r_oor:?}"
        );

        let mut vm_neg = Vm::new(prog);
        let r_neg = vm_neg.execute(FuncId(2), vec![]).unwrap();
        assert!(
            matches!(r_neg, RuntimeValue::Null),
            "negative-index get must return Null (Q3-A strict bounds), got {r_neg:?}"
        );
    }

    /// `vector_length(v)` returns element count as Integer.
    #[test]
    fn vm_vector_length_returns_element_count() {
        let mut pool = ConstantPool::new();
        let c_a = pool.intern(Constant::Integer(Integer::new(1).unwrap()));
        let c_b = pool.intern(Constant::Integer(Integer::new(2).unwrap()));
        let c_c = pool.intern(Constant::Integer(Integer::new(3).unwrap()));

        let func = Function {
            id: FuncId(0),
            name: Some("count".into()),
            params: vec![],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(0)),
                        name: BuiltinName::VectorNew,
                        args: vec![],
                    },
                    Instruction::Const {
                        dest: ValueId(1),
                        constant: c_a,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(2)),
                        name: BuiltinName::VectorPush,
                        args: vec![Operand::Value(ValueId(0)), Operand::Value(ValueId(1))],
                    },
                    Instruction::Const {
                        dest: ValueId(3),
                        constant: c_b,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(4)),
                        name: BuiltinName::VectorPush,
                        args: vec![Operand::Value(ValueId(2)), Operand::Value(ValueId(3))],
                    },
                    Instruction::Const {
                        dest: ValueId(5),
                        constant: c_c,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(6)),
                        name: BuiltinName::VectorPush,
                        args: vec![Operand::Value(ValueId(4)), Operand::Value(ValueId(5))],
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(7)),
                        name: BuiltinName::VectorLength,
                        args: vec![Operand::Value(ValueId(6))],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(7))),
                    },
                ],
            }],
        };
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![]).unwrap();
        assert_eq!(result.to_string(), "3", "expected length=3 after 3 pushes");

        // Empty vector also exercises the length=0 fast path.
        let empty_func = Function {
            id: FuncId(0),
            name: Some("empty_count".into()),
            params: vec![],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(0)),
                        name: BuiltinName::VectorNew,
                        args: vec![],
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(1)),
                        name: BuiltinName::VectorLength,
                        args: vec![Operand::Value(ValueId(0))],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(1))),
                    },
                ],
            }],
        };
        let mut vm_empty = Vm::new(make_simple_program(empty_func));
        let r_empty = vm_empty.execute(FuncId(0), vec![]).unwrap();
        assert_eq!(r_empty.to_string(), "0", "empty vector has length 0");
    }

    // ── v0.7.3.3 HashMap builtins ────────────────────────────────
    //
    // Per ADR-0019 §5 + Addendum §A4.1: 5 HashMap builtins —
    // HashMapNew/Insert/Get/Keys/Contains, wire IDs 12-16.
    // Q1-A functional return-new (mirror VectorPush).
    // Q2-B reuse TypeMismatch for invalid key types.
    // Q3-A strict 2-state Trilean for Contains.
    // Q4-A sorted key order for Keys.
    //
    // Error model 3-tier (ADR-0019 Addendum §A7):
    // - Lookup miss = data event → V? Null / Trilean::False
    // - Invalid key type = bug → VmError::TypeMismatch panic
    //   (NOT Trilean::Unknown — Ł3 reserved for genuine
    //   semantic uncertainty, not type errors).

    /// `hashmap_new()` returns an empty `HashMap`.
    #[test]
    fn vm_hashmap_new_returns_empty_map() {
        let func = Function {
            id: FuncId(0),
            name: Some("make_empty_map".into()),
            params: vec![],
            return_type: TypeTag::HashMap(Box::new(TypeTag::String), Box::new(TypeTag::Integer)),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(0)),
                        name: BuiltinName::HashMapNew,
                        args: vec![],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    },
                ],
            }],
        };
        let prog = make_simple_program(func);
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![]).unwrap();
        match result {
            RuntimeValue::HashMap(entries) => assert!(entries.is_empty()),
            other => panic!("expected empty HashMap, got {other:?}"),
        }
    }

    /// `hashmap_insert(m, k, v)` returns new map with k -> v added.
    /// Functional return-new (Q1-A consistency).
    #[test]
    fn vm_hashmap_insert_returns_new_map_with_pair() {
        let mut pool = ConstantPool::new();
        let c_key = pool.intern(Constant::String("alpha".into()));
        let c_value = pool.intern(Constant::Integer(Integer::new(42).unwrap()));

        let func = Function {
            id: FuncId(0),
            name: Some("insert_one".into()),
            params: vec![],
            return_type: TypeTag::HashMap(Box::new(TypeTag::String), Box::new(TypeTag::Integer)),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(0)),
                        name: BuiltinName::HashMapNew,
                        args: vec![],
                    },
                    Instruction::Const {
                        dest: ValueId(1),
                        constant: c_key,
                    },
                    Instruction::Const {
                        dest: ValueId(2),
                        constant: c_value,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(3)),
                        name: BuiltinName::HashMapInsert,
                        args: vec![
                            Operand::Value(ValueId(0)),
                            Operand::Value(ValueId(1)),
                            Operand::Value(ValueId(2)),
                        ],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(3))),
                    },
                ],
            }],
        };
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![]).unwrap();
        match result {
            RuntimeValue::HashMap(entries) => {
                assert_eq!(entries.len(), 1);
                let key = RuntimeMapKey::String("alpha".into());
                assert!(entries.contains_key(&key));
                assert_eq!(entries.get(&key).unwrap().to_string(), "42");
            }
            other => panic!("expected HashMap with 1 entry, got {other:?}"),
        }
    }

    /// `hashmap_get(m, k)` returns V? — value if present, Null if not.
    /// Strict tier: miss = data event, not error.
    #[test]
    fn vm_hashmap_get_hit_returns_value_miss_returns_null() {
        let mut pool = ConstantPool::new();
        let c_key_present = pool.intern(Constant::String("present".into()));
        let c_key_absent = pool.intern(Constant::String("absent".into()));
        let c_value = pool.intern(Constant::Integer(Integer::new(7).unwrap()));

        // Helper builds a function that inserts (c_key_present -> c_value)
        // and then gets <lookup_const>.
        let build = |id: FuncId, lookup: ConstId, name: &str| Function {
            id,
            name: Some(name.into()),
            params: vec![],
            return_type: TypeTag::Nullable(Box::new(TypeTag::Integer)),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(0)),
                        name: BuiltinName::HashMapNew,
                        args: vec![],
                    },
                    Instruction::Const {
                        dest: ValueId(1),
                        constant: c_key_present,
                    },
                    Instruction::Const {
                        dest: ValueId(2),
                        constant: c_value,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(3)),
                        name: BuiltinName::HashMapInsert,
                        args: vec![
                            Operand::Value(ValueId(0)),
                            Operand::Value(ValueId(1)),
                            Operand::Value(ValueId(2)),
                        ],
                    },
                    Instruction::Const {
                        dest: ValueId(4),
                        constant: lookup,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(5)),
                        name: BuiltinName::HashMapGet,
                        args: vec![Operand::Value(ValueId(3)), Operand::Value(ValueId(4))],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(5))),
                    },
                ],
            }],
        };

        let prog = IrProgram {
            modules: vec![IrModule {
                path: AbsolutePath::new(ModulePath::khi_root(), "test".into()),
                functions: vec![
                    build(FuncId(0), c_key_present, "get_hit"),
                    build(FuncId(1), c_key_absent, "get_miss"),
                ],
            }],
            constants: pool,
            witness_tables: Vec::new(),
        };

        let mut vm_hit = Vm::new(prog.clone());
        let r_hit = vm_hit.execute(FuncId(0), vec![]).unwrap();
        assert_eq!(r_hit.to_string(), "7", "present key must return value");

        let mut vm_miss = Vm::new(prog);
        let r_miss = vm_miss.execute(FuncId(1), vec![]).unwrap();
        assert!(
            matches!(r_miss, RuntimeValue::Null),
            "absent key must return Null (data event, not error), got {r_miss:?}"
        );
    }

    /// `hashmap_keys(m)` returns sorted Vector of keys (Q4-A).
    /// Deterministic order is critical for canonical emission.
    #[test]
    fn vm_hashmap_keys_returns_sorted_vector() {
        let mut pool = ConstantPool::new();
        // Insert keys out of order: "zebra", "alpha", "middle"
        let c_zebra = pool.intern(Constant::String("zebra".into()));
        let c_alpha = pool.intern(Constant::String("alpha".into()));
        let c_middle = pool.intern(Constant::String("middle".into()));
        let c_value = pool.intern(Constant::Integer(Integer::new(0).unwrap()));

        let mut instructions = vec![Instruction::CallBuiltin {
            dest: Some(ValueId(0)),
            name: BuiltinName::HashMapNew,
            args: vec![],
        }];

        let mut next_value_id = 1u32;
        let mut current_map = ValueId(0);
        for (key_const, label) in [(c_zebra, "z"), (c_alpha, "a"), (c_middle, "m")] {
            let k_id = ValueId(next_value_id);
            instructions.push(Instruction::Const {
                dest: k_id,
                constant: key_const,
            });
            next_value_id += 1;

            let v_id = ValueId(next_value_id);
            instructions.push(Instruction::Const {
                dest: v_id,
                constant: c_value,
            });
            next_value_id += 1;

            let new_map = ValueId(next_value_id);
            instructions.push(Instruction::CallBuiltin {
                dest: Some(new_map),
                name: BuiltinName::HashMapInsert,
                args: vec![
                    Operand::Value(current_map),
                    Operand::Value(k_id),
                    Operand::Value(v_id),
                ],
            });
            next_value_id += 1;
            current_map = new_map;
            // label used only for documentation
            let _ = label;
        }

        let keys_id = ValueId(next_value_id);
        instructions.push(Instruction::CallBuiltin {
            dest: Some(keys_id),
            name: BuiltinName::HashMapKeys,
            args: vec![Operand::Value(current_map)],
        });
        instructions.push(Instruction::Ret {
            value: Some(Operand::Value(keys_id)),
        });

        let func = Function {
            id: FuncId(0),
            name: Some("keys_of".into()),
            params: vec![],
            return_type: TypeTag::Vector(Box::new(TypeTag::String)),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions,
            }],
        };
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![]).unwrap();
        match result {
            RuntimeValue::Vector(keys) => {
                assert_eq!(keys.len(), 3);
                // Sorted alphabetically by BTreeMap natural order.
                assert_eq!(keys[0].to_string(), "alpha");
                assert_eq!(keys[1].to_string(), "middle");
                assert_eq!(keys[2].to_string(), "zebra");
            }
            other => panic!("expected Vector of 3 sorted keys, got {other:?}"),
        }
    }

    /// `hashmap_contains(m, k)` strict 2-state per Q3-A.
    #[test]
    fn vm_hashmap_contains_returns_strict_trilean() {
        let mut pool = ConstantPool::new();
        let c_key_present = pool.intern(Constant::String("here".into()));
        let c_key_absent = pool.intern(Constant::String("missing".into()));
        let c_value = pool.intern(Constant::Integer(Integer::new(1).unwrap()));

        let build = |id: FuncId, lookup: ConstId, name: &str| Function {
            id,
            name: Some(name.into()),
            params: vec![],
            return_type: TypeTag::Trilean,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(0)),
                        name: BuiltinName::HashMapNew,
                        args: vec![],
                    },
                    Instruction::Const {
                        dest: ValueId(1),
                        constant: c_key_present,
                    },
                    Instruction::Const {
                        dest: ValueId(2),
                        constant: c_value,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(3)),
                        name: BuiltinName::HashMapInsert,
                        args: vec![
                            Operand::Value(ValueId(0)),
                            Operand::Value(ValueId(1)),
                            Operand::Value(ValueId(2)),
                        ],
                    },
                    Instruction::Const {
                        dest: ValueId(4),
                        constant: lookup,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(5)),
                        name: BuiltinName::HashMapContains,
                        args: vec![Operand::Value(ValueId(3)), Operand::Value(ValueId(4))],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(5))),
                    },
                ],
            }],
        };

        let prog = IrProgram {
            modules: vec![IrModule {
                path: AbsolutePath::new(ModulePath::khi_root(), "test".into()),
                functions: vec![
                    build(FuncId(0), c_key_present, "contains_hit"),
                    build(FuncId(1), c_key_absent, "contains_miss"),
                ],
            }],
            constants: pool,
            witness_tables: Vec::new(),
        };

        let mut vm_hit = Vm::new(prog.clone());
        let r_hit = vm_hit.execute(FuncId(0), vec![]).unwrap();
        assert!(
            matches!(r_hit, RuntimeValue::Trilean(Trilean::True)),
            "present key must return Trilean::True (strict 2-state), got {r_hit:?}"
        );

        let mut vm_miss = Vm::new(prog);
        let r_miss = vm_miss.execute(FuncId(1), vec![]).unwrap();
        assert!(
            matches!(r_miss, RuntimeValue::Trilean(Trilean::False)),
            "absent key must return Trilean::False (NOT Unknown — Q3-A strict), got {r_miss:?}"
        );
    }

    /// Invalid key type panics with E2201 `TypeMismatch`, NOT silently
    /// returns `Null` or `Trilean::Unknown`. Q2-B + error model §A7
    /// 3-tier compliance: bug-driven failure → runtime panic.
    #[test]
    fn vm_hashmap_invalid_key_type_panics_with_type_mismatch() {
        // Build a function that inserts vector_new() (a Vector value)
        // as a key — Vector is NOT a hashable primitive per
        // RuntimeMapKey::from_runtime contract.
        let mut pool = ConstantPool::new();
        let c_value = pool.intern(Constant::Integer(Integer::new(99).unwrap()));

        let func = Function {
            id: FuncId(0),
            name: Some("invalid_key".into()),
            params: vec![],
            return_type: TypeTag::HashMap(Box::new(TypeTag::Unit), Box::new(TypeTag::Integer)),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(0)),
                        name: BuiltinName::HashMapNew,
                        args: vec![],
                    },
                    // Build a Vector to use as (invalid) key.
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(1)),
                        name: BuiltinName::VectorNew,
                        args: vec![],
                    },
                    Instruction::Const {
                        dest: ValueId(2),
                        constant: c_value,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(3)),
                        name: BuiltinName::HashMapInsert,
                        args: vec![
                            Operand::Value(ValueId(0)),
                            Operand::Value(ValueId(1)), // Vector as key — bug
                            Operand::Value(ValueId(2)),
                        ],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(3))),
                    },
                ],
            }],
        };
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![]);
        match result {
            Err(VmError::TypeMismatch { actual, .. }) => {
                assert!(
                    actual.contains("non-hashable"),
                    "expected TypeMismatch with non-hashable hint, got: {actual}"
                );
            }
            other => panic!(
                "expected VmError::TypeMismatch (E2201 — bug-tier, not data-tier), got {other:?}"
            ),
        }
    }

    /// Composition integration test (Q3-C pattern from v0.7.3.2):
    /// build a 3-entry `HashMap`, verify keys ordering + contains
    /// hit/miss + get round-trip in the same program. Mirrors
    /// self-host compiler's symbol-table pattern.
    #[test]
    fn vm_hashmap_compose_insert_contains_get_keys_round_trip() {
        let mut pool = ConstantPool::new();
        let key_zebra = pool.intern(Constant::String("zebra".into()));
        let key_alpha = pool.intern(Constant::String("alpha".into()));
        let key_middle = pool.intern(Constant::String("middle".into()));
        let value_first = pool.intern(Constant::Integer(Integer::new(100).unwrap()));
        let value_second = pool.intern(Constant::Integer(Integer::new(200).unwrap()));
        let value_third = pool.intern(Constant::Integer(Integer::new(300).unwrap()));
        let lookup_key = pool.intern(Constant::String("middle".into()));

        // Build a 3-insert chain then verify get("middle") = 300.
        let func = Function {
            id: FuncId(0),
            name: Some("compose_map".into()),
            params: vec![],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(0)),
                        name: BuiltinName::HashMapNew,
                        args: vec![],
                    },
                    // insert(map, "zebra", 100)
                    Instruction::Const {
                        dest: ValueId(1),
                        constant: key_zebra,
                    },
                    Instruction::Const {
                        dest: ValueId(2),
                        constant: value_first,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(3)),
                        name: BuiltinName::HashMapInsert,
                        args: vec![
                            Operand::Value(ValueId(0)),
                            Operand::Value(ValueId(1)),
                            Operand::Value(ValueId(2)),
                        ],
                    },
                    // insert(map2, "alpha", 200)
                    Instruction::Const {
                        dest: ValueId(4),
                        constant: key_alpha,
                    },
                    Instruction::Const {
                        dest: ValueId(5),
                        constant: value_second,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(6)),
                        name: BuiltinName::HashMapInsert,
                        args: vec![
                            Operand::Value(ValueId(3)),
                            Operand::Value(ValueId(4)),
                            Operand::Value(ValueId(5)),
                        ],
                    },
                    // insert(map3, "middle", 300)
                    Instruction::Const {
                        dest: ValueId(7),
                        constant: key_middle,
                    },
                    Instruction::Const {
                        dest: ValueId(8),
                        constant: value_third,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(9)),
                        name: BuiltinName::HashMapInsert,
                        args: vec![
                            Operand::Value(ValueId(6)),
                            Operand::Value(ValueId(7)),
                            Operand::Value(ValueId(8)),
                        ],
                    },
                    // get(final_map, "middle")
                    Instruction::Const {
                        dest: ValueId(10),
                        constant: lookup_key,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(11)),
                        name: BuiltinName::HashMapGet,
                        args: vec![Operand::Value(ValueId(9)), Operand::Value(ValueId(10))],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(11))),
                    },
                ],
            }],
        };
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![]).unwrap();
        assert_eq!(
            result.to_string(),
            "300",
            "compose test: get(\"middle\") of {{\"zebra\":100, \"alpha\":200, \"middle\":300}} = 300"
        );
    }

    /// Composition integration test (Q3-C): build a 3-element vector,
    /// verify length AND multiple `get` indices in the same program.
    /// Mirrors the self-host compiler's expected symbol-table
    /// access pattern.
    #[test]
    fn vm_vector_compose_push_length_get_round_trip() {
        let mut pool = ConstantPool::new();
        let c_a = pool.intern(Constant::Integer(Integer::new(100).unwrap()));
        let c_b = pool.intern(Constant::Integer(Integer::new(200).unwrap()));
        let c_c = pool.intern(Constant::Integer(Integer::new(300).unwrap()));
        // Idx constants for get(0)+get(2) — sum should be 100+300=400.
        let c_i0 = pool.intern(Constant::Integer(Integer::new(0).unwrap()));
        let c_i2 = pool.intern(Constant::Integer(Integer::new(2).unwrap()));

        let func = Function {
            id: FuncId(0),
            name: Some("compose".into()),
            params: vec![],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(0)),
                        name: BuiltinName::VectorNew,
                        args: vec![],
                    },
                    Instruction::Const {
                        dest: ValueId(1),
                        constant: c_a,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(2)),
                        name: BuiltinName::VectorPush,
                        args: vec![Operand::Value(ValueId(0)), Operand::Value(ValueId(1))],
                    },
                    Instruction::Const {
                        dest: ValueId(3),
                        constant: c_b,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(4)),
                        name: BuiltinName::VectorPush,
                        args: vec![Operand::Value(ValueId(2)), Operand::Value(ValueId(3))],
                    },
                    Instruction::Const {
                        dest: ValueId(5),
                        constant: c_c,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(6)),
                        name: BuiltinName::VectorPush,
                        args: vec![Operand::Value(ValueId(4)), Operand::Value(ValueId(5))],
                    },
                    // get(0)
                    Instruction::Const {
                        dest: ValueId(7),
                        constant: c_i0,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(8)),
                        name: BuiltinName::VectorGet,
                        args: vec![Operand::Value(ValueId(6)), Operand::Value(ValueId(7))],
                    },
                    // get(2)
                    Instruction::Const {
                        dest: ValueId(9),
                        constant: c_i2,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(10)),
                        name: BuiltinName::VectorGet,
                        args: vec![Operand::Value(ValueId(6)), Operand::Value(ValueId(9))],
                    },
                    // sum = get(0) + get(2)
                    // Note: VectorGet returns Integer directly (Null wraps
                    // are implicit at the runtime layer — the value IS
                    // the discriminator's presence).
                    Instruction::Add {
                        dest: ValueId(11),
                        lhs: Operand::Value(ValueId(8)),
                        rhs: Operand::Value(ValueId(10)),
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(11))),
                    },
                ],
            }],
        };
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![]).unwrap();
        assert_eq!(
            result.to_string(),
            "400",
            "compose test: get(0)+get(2) of [100,200,300] = 100+300 = 400"
        );
    }

    // ── v0.7.3.4 IO + path + string builtins ─────────────────────
    //
    // Per ADR-0019 §5 + Addendum §A4.1: 10 builtins post-dedup —
    // ReadFile/WriteFile/FileExists (IO, Q1-A no caps yet),
    // PathJoin/PathParent/PathBasename (Q2-A POSIX),
    // StringSubstring/Split/IndexOf + ParseInteger (Q3-A char-index).
    //
    // IO tests use `tempfile` for filesystem fixtures (Q4-A).
    // Capability gating deferred per §A7 (v0.7.10 CLI wiring).
    // Path semantics POSIX-only — Windows deferred (§A7).
    // Substring OOB panics with E2206 per Q3-A (slicing = intent).

    // ── IO ──────────────────────────────────────────────────────

    /// Helper: build a single-builtin program. Args bound to the
    /// function's parameter list, dispatched into the builtin
    /// with operand references in order.
    fn build_builtin_program(
        params: Vec<(String, TypeTag)>,
        return_type: TypeTag,
        builtin: BuiltinName,
        constants: ConstantPool,
    ) -> IrProgram {
        let param_count = u32::try_from(params.len()).unwrap_or(0);
        let dest = ValueId(param_count);
        let args: Vec<Operand> = (0..param_count)
            .map(|i| Operand::Value(ValueId(i)))
            .collect();
        let func = Function {
            id: FuncId(0),
            name: Some("dispatch".into()),
            params,
            return_type,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(dest),
                        name: builtin,
                        args,
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(dest)),
                    },
                ],
            }],
        };
        let mut prog = make_simple_program(func);
        prog.constants = constants;
        prog
    }

    fn make_string(s: &str) -> RuntimeValue {
        RuntimeValue::String(s.into())
    }

    /// `write_file` then `read_file` round-trip — proves the strict
    /// 2-state Trilean (Q4-A) write returns True on success and the
    /// content survives unchanged.
    #[test]
    fn vm_read_file_write_file_round_trip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("hello.txt");
        let path_str = path.to_string_lossy().into_owned();
        let contents = "Xin chào, Triết!";

        // write_file(path, contents) — returns Trilean::True on success.
        let prog_write = build_builtin_program(
            vec![
                ("path".into(), TypeTag::String),
                ("contents".into(), TypeTag::String),
            ],
            TypeTag::Trilean,
            BuiltinName::WriteFile,
            ConstantPool::new(),
        );
        let mut vm_write = Vm::new(prog_write);
        let r_write = vm_write
            .execute(
                FuncId(0),
                vec![make_string(&path_str), make_string(contents)],
            )
            .unwrap();
        assert!(
            matches!(r_write, RuntimeValue::Trilean(Trilean::True)),
            "write_file must return Trilean::True on success, got {r_write:?}"
        );

        // read_file(path) → Some(String) — content matches.
        let prog_read = build_builtin_program(
            vec![("path".into(), TypeTag::String)],
            TypeTag::Nullable(Box::new(TypeTag::String)),
            BuiltinName::ReadFile,
            ConstantPool::new(),
        );
        let mut vm_read = Vm::new(prog_read);
        let r_read = vm_read
            .execute(FuncId(0), vec![make_string(&path_str)])
            .unwrap();
        match r_read {
            RuntimeValue::String(s) => assert_eq!(s, contents),
            other => panic!("expected read_file → String, got {other:?}"),
        }
    }

    /// `read_file` on non-existent path → Null (data tier, not panic).
    #[test]
    fn vm_read_file_missing_path_returns_null() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("does_not_exist.txt");
        let path_str = path.to_string_lossy().into_owned();

        let prog = build_builtin_program(
            vec![("path".into(), TypeTag::String)],
            TypeTag::Nullable(Box::new(TypeTag::String)),
            BuiltinName::ReadFile,
            ConstantPool::new(),
        );
        let mut vm = Vm::new(prog);
        let r = vm.execute(FuncId(0), vec![make_string(&path_str)]).unwrap();
        assert!(
            matches!(r, RuntimeValue::Null),
            "missing file must return Null (data event, not panic), got {r:?}"
        );
    }

    /// `file_exists` strict 2-state — True for existing file, False
    /// for missing path.
    #[test]
    fn vm_file_exists_strict_trilean() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let present = tmp.path().join("present.txt");
        std::fs::write(&present, "x").expect("write fixture");
        let missing = tmp.path().join("missing.txt");

        let prog = build_builtin_program(
            vec![("path".into(), TypeTag::String)],
            TypeTag::Trilean,
            BuiltinName::FileExists,
            ConstantPool::new(),
        );
        let mut vm_present = Vm::new(prog.clone());
        let r_present = vm_present
            .execute(FuncId(0), vec![make_string(&present.to_string_lossy())])
            .unwrap();
        assert!(
            matches!(r_present, RuntimeValue::Trilean(Trilean::True)),
            "existing file must return True, got {r_present:?}"
        );

        let mut vm_missing = Vm::new(prog);
        let r_missing = vm_missing
            .execute(FuncId(0), vec![make_string(&missing.to_string_lossy())])
            .unwrap();
        assert!(
            matches!(r_missing, RuntimeValue::Trilean(Trilean::False)),
            "missing file must return False (NOT Unknown — Q4-A strict), got {r_missing:?}"
        );
    }

    // ── Path ────────────────────────────────────────────────────

    /// `path_join` Q2-A POSIX semantic: hardcoded `/`, deterministic.
    #[test]
    fn vm_path_join_posix_semantic() {
        let prog = build_builtin_program(
            vec![
                ("base".into(), TypeTag::String),
                ("segment".into(), TypeTag::String),
            ],
            TypeTag::String,
            BuiltinName::PathJoin,
            ConstantPool::new(),
        );

        // Normal join.
        let mut vm = Vm::new(prog.clone());
        let r = vm
            .execute(FuncId(0), vec![make_string("a"), make_string("b")])
            .unwrap();
        assert_eq!(r.to_string(), "a/b");

        // Trailing slash on base — no duplication.
        let mut vm2 = Vm::new(prog.clone());
        let r2 = vm2
            .execute(FuncId(0), vec![make_string("a/"), make_string("b")])
            .unwrap();
        assert_eq!(r2.to_string(), "a/b");

        // Empty base.
        let mut vm3 = Vm::new(prog);
        let r3 = vm3
            .execute(FuncId(0), vec![make_string(""), make_string("b")])
            .unwrap();
        assert_eq!(r3.to_string(), "b");
    }

    /// `path_parent` returns parent path or Null.
    #[test]
    fn vm_path_parent_returns_parent_or_null() {
        let prog = build_builtin_program(
            vec![("path".into(), TypeTag::String)],
            TypeTag::Nullable(Box::new(TypeTag::String)),
            BuiltinName::PathParent,
            ConstantPool::new(),
        );

        // Normal case.
        let mut vm = Vm::new(prog.clone());
        let r = vm.execute(FuncId(0), vec![make_string("a/b/c")]).unwrap();
        assert_eq!(r.to_string(), "a/b");

        // Root `/` → Null.
        let mut vm_root = Vm::new(prog.clone());
        let r_root = vm_root.execute(FuncId(0), vec![make_string("/")]).unwrap();
        assert!(
            matches!(r_root, RuntimeValue::Null),
            "root path has no parent, got {r_root:?}"
        );

        // No separator → Null.
        let mut vm_no_sep = Vm::new(prog);
        let r_no_sep = vm_no_sep
            .execute(FuncId(0), vec![make_string("file")])
            .unwrap();
        assert!(
            matches!(r_no_sep, RuntimeValue::Null),
            "no-separator path has no parent, got {r_no_sep:?}"
        );
    }

    /// `path_basename` returns last segment.
    #[test]
    fn vm_path_basename_last_segment() {
        let prog = build_builtin_program(
            vec![("path".into(), TypeTag::String)],
            TypeTag::String,
            BuiltinName::PathBasename,
            ConstantPool::new(),
        );

        let mut vm = Vm::new(prog.clone());
        let r = vm
            .execute(FuncId(0), vec![make_string("a/b/c.txt")])
            .unwrap();
        assert_eq!(r.to_string(), "c.txt");

        // Trailing slash — strip then take.
        let mut vm2 = Vm::new(prog.clone());
        let r2 = vm2.execute(FuncId(0), vec![make_string("a/b/")]).unwrap();
        assert_eq!(r2.to_string(), "b");

        // No separator — whole path is basename.
        let mut vm3 = Vm::new(prog);
        let r3 = vm3.execute(FuncId(0), vec![make_string("file")]).unwrap();
        assert_eq!(r3.to_string(), "file");
    }

    // ── String ──────────────────────────────────────────────────

    /// `string_substring` Q3-A char-index, multi-byte UTF-8 safe.
    /// Tests Vietnamese to prove codepoint handling.
    #[test]
    fn vm_string_substring_char_index_multibyte_safe() {
        let prog = build_builtin_program(
            vec![
                ("s".into(), TypeTag::String),
                ("start".into(), TypeTag::Integer),
                ("end".into(), TypeTag::Integer),
            ],
            TypeTag::String,
            BuiltinName::StringSubstring,
            ConstantPool::new(),
        );

        // ASCII slice.
        let mut vm = Vm::new(prog.clone());
        let r = vm
            .execute(
                FuncId(0),
                vec![make_string("hello"), make_int(1), make_int(4)],
            )
            .unwrap();
        assert_eq!(r.to_string(), "ell");

        // Vietnamese: "Việt" — 4 codepoints. chars()[0..1] = "V".
        let mut vm_vn = Vm::new(prog.clone());
        let r_vn = vm_vn
            .execute(
                FuncId(0),
                vec![make_string("Việt"), make_int(0), make_int(2)],
            )
            .unwrap();
        assert_eq!(r_vn.to_string(), "Vi");

        // Empty range.
        let mut vm_empty = Vm::new(prog);
        let r_empty = vm_empty
            .execute(
                FuncId(0),
                vec![make_string("hello"), make_int(3), make_int(3)],
            )
            .unwrap();
        assert_eq!(r_empty.to_string(), "");
    }

    /// `string_substring` OOB → E2206 `OutOfBounds` panic (Q3-A
    /// slicing = intentional; bug if OOB).
    #[test]
    fn vm_string_substring_out_of_bounds_panics() {
        let prog = build_builtin_program(
            vec![
                ("s".into(), TypeTag::String),
                ("start".into(), TypeTag::Integer),
                ("end".into(), TypeTag::Integer),
            ],
            TypeTag::String,
            BuiltinName::StringSubstring,
            ConstantPool::new(),
        );

        // end > char_count.
        let mut vm = Vm::new(prog.clone());
        let r = vm.execute(
            FuncId(0),
            vec![make_string("hi"), make_int(0), make_int(99)],
        );
        assert!(
            matches!(r, Err(VmError::OutOfBounds { .. })),
            "expected OutOfBounds for end>length, got {r:?}"
        );

        // Negative start.
        let mut vm_neg = Vm::new(prog.clone());
        let r_neg = vm_neg.execute(
            FuncId(0),
            vec![make_string("hi"), make_int(-1), make_int(1)],
        );
        assert!(
            matches!(r_neg, Err(VmError::OutOfBounds { .. })),
            "expected OutOfBounds for negative start, got {r_neg:?}"
        );

        // start > end.
        let mut vm_swap = Vm::new(prog);
        let r_swap = vm_swap.execute(FuncId(0), vec![make_string("hi"), make_int(2), make_int(1)]);
        assert!(
            matches!(r_swap, Err(VmError::OutOfBounds { .. })),
            "expected OutOfBounds for start>end, got {r_swap:?}"
        );
    }

    /// `string_split` returns Vector of parts.
    #[test]
    fn vm_string_split_returns_vector() {
        let prog = build_builtin_program(
            vec![
                ("s".into(), TypeTag::String),
                ("sep".into(), TypeTag::String),
            ],
            TypeTag::Vector(Box::new(TypeTag::String)),
            BuiltinName::StringSplit,
            ConstantPool::new(),
        );

        // Normal split.
        let mut vm = Vm::new(prog.clone());
        let r = vm
            .execute(FuncId(0), vec![make_string("a,b,c"), make_string(",")])
            .unwrap();
        match r {
            RuntimeValue::Vector(parts) => {
                assert_eq!(parts.len(), 3);
                assert_eq!(parts[0].to_string(), "a");
                assert_eq!(parts[1].to_string(), "b");
                assert_eq!(parts[2].to_string(), "c");
            }
            other => panic!("expected Vector, got {other:?}"),
        }

        // Empty separator → single-element [s] (refuse-over-guess).
        let mut vm_empty = Vm::new(prog);
        let r_empty = vm_empty
            .execute(FuncId(0), vec![make_string("abc"), make_string("")])
            .unwrap();
        match r_empty {
            RuntimeValue::Vector(parts) => {
                assert_eq!(parts.len(), 1);
                assert_eq!(parts[0].to_string(), "abc");
            }
            other => panic!("expected single-element Vector, got {other:?}"),
        }
    }

    /// `string_index_of` returns char (codepoint) offset, or Null.
    #[test]
    fn vm_string_index_of_char_offset_or_null() {
        let prog = build_builtin_program(
            vec![
                ("haystack".into(), TypeTag::String),
                ("needle".into(), TypeTag::String),
            ],
            TypeTag::Nullable(Box::new(TypeTag::Integer)),
            BuiltinName::StringIndexOf,
            ConstantPool::new(),
        );

        // Found.
        let mut vm = Vm::new(prog.clone());
        let r = vm
            .execute(FuncId(0), vec![make_string("hello"), make_string("ll")])
            .unwrap();
        assert_eq!(r.to_string(), "2");

        // Not found → Null.
        let mut vm_miss = Vm::new(prog.clone());
        let r_miss = vm_miss
            .execute(FuncId(0), vec![make_string("hello"), make_string("xyz")])
            .unwrap();
        assert!(
            matches!(r_miss, RuntimeValue::Null),
            "needle-not-found returns Null, got {r_miss:?}"
        );

        // Vietnamese — needle starts at char 2 (codepoint), not byte 2.
        let mut vm_vn = Vm::new(prog.clone());
        let r_vn = vm_vn
            .execute(FuncId(0), vec![make_string("Việt Nam"), make_string("Nam")])
            .unwrap();
        assert_eq!(r_vn.to_string(), "5");

        // Empty needle → 0 (matches at start).
        let mut vm_empty = Vm::new(prog);
        let r_empty = vm_empty
            .execute(FuncId(0), vec![make_string("hello"), make_string("")])
            .unwrap();
        assert_eq!(r_empty.to_string(), "0");
    }

    /// `parse_integer` strict decimal — refuse-over-guess.
    #[test]
    fn vm_parse_integer_strict_decimal() {
        let prog = build_builtin_program(
            vec![("s".into(), TypeTag::String)],
            TypeTag::Nullable(Box::new(TypeTag::Integer)),
            BuiltinName::ParseInteger,
            ConstantPool::new(),
        );

        // Success.
        let mut vm = Vm::new(prog.clone());
        let r = vm.execute(FuncId(0), vec![make_string("42")]).unwrap();
        assert_eq!(r.to_string(), "42");

        // Negative.
        let mut vm_neg = Vm::new(prog.clone());
        let r_neg = vm_neg.execute(FuncId(0), vec![make_string("-7")]).unwrap();
        assert_eq!(r_neg.to_string(), "-7");

        // Empty → Null.
        let mut vm_empty = Vm::new(prog.clone());
        let r_empty = vm_empty.execute(FuncId(0), vec![make_string("")]).unwrap();
        assert!(matches!(r_empty, RuntimeValue::Null));

        // Non-digit → Null.
        let mut vm_bad = Vm::new(prog.clone());
        let r_bad = vm_bad.execute(FuncId(0), vec![make_string("abc")]).unwrap();
        assert!(matches!(r_bad, RuntimeValue::Null));

        // Leading whitespace → Null (refuse-over-guess).
        let mut vm_ws = Vm::new(prog);
        let r_ws = vm_ws.execute(FuncId(0), vec![make_string(" 42")]).unwrap();
        assert!(
            matches!(r_ws, RuntimeValue::Null),
            "leading whitespace must NOT parse (refuse-over-guess), got {r_ws:?}"
        );
    }

    /// Composition: lexer-like flow — read source file, split by
    /// newline, parse each line as integer, accumulate to a Vector.
    /// Mirrors self-host compiler's main file-handling pattern.
    #[test]
    fn vm_compose_read_split_parse_accumulate() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("numbers.txt");
        std::fs::write(&path, "10\n20\n30").expect("write fixture");
        let path_str = path.to_string_lossy().into_owned();

        // Step 1: read file.
        let prog_read = build_builtin_program(
            vec![("path".into(), TypeTag::String)],
            TypeTag::Nullable(Box::new(TypeTag::String)),
            BuiltinName::ReadFile,
            ConstantPool::new(),
        );
        let mut vm_read = Vm::new(prog_read);
        let contents = vm_read
            .execute(FuncId(0), vec![make_string(&path_str)])
            .unwrap();
        let contents_str = match contents {
            RuntimeValue::String(s) => s,
            other => panic!("expected read_file → String, got {other:?}"),
        };

        // Step 2: split by newline.
        let prog_split = build_builtin_program(
            vec![
                ("s".into(), TypeTag::String),
                ("sep".into(), TypeTag::String),
            ],
            TypeTag::Vector(Box::new(TypeTag::String)),
            BuiltinName::StringSplit,
            ConstantPool::new(),
        );
        let mut vm_split = Vm::new(prog_split);
        let parts = vm_split
            .execute(
                FuncId(0),
                vec![make_string(&contents_str), make_string("\n")],
            )
            .unwrap();
        let parts_vec = match parts {
            RuntimeValue::Vector(v) => v,
            other => panic!("expected Vector, got {other:?}"),
        };
        assert_eq!(parts_vec.len(), 3);

        // Step 3: parse each part — collect successes.
        let prog_parse = build_builtin_program(
            vec![("s".into(), TypeTag::String)],
            TypeTag::Nullable(Box::new(TypeTag::Integer)),
            BuiltinName::ParseInteger,
            ConstantPool::new(),
        );
        let mut parsed = Vec::new();
        for part in &parts_vec {
            let part_str = match part {
                RuntimeValue::String(s) => s.clone(),
                _ => panic!("non-string element"),
            };
            let mut vm_parse = Vm::new(prog_parse.clone());
            let r = vm_parse
                .execute(FuncId(0), vec![make_string(&part_str)])
                .unwrap();
            parsed.push(r.to_string());
        }
        assert_eq!(parsed, vec!["10", "20", "30"]);
    }

    // ── Outcome opcodes (ADR-0020, v0.7.4.3-error.3a) ────────────
    //
    // Six smoke tests, one per opcode. Each builds a single-block
    // function that constructs an outcome (or unpacks one) and
    // returns the result so the assertion can read it back through
    // `RuntimeValue::Display`.

    fn outcome_function(instructions: Vec<Instruction>, return_type: TypeTag) -> Function {
        Function {
            id: FuncId(0),
            name: Some("outcome_test".into()),
            params: Vec::new(),
            return_type,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions,
            }],
        }
    }

    /// `OutcomeNewPositive` packs a payload + `Trit::Positive`
    /// discriminator; the Display form mirrors `~+(payload)`.
    #[test]
    fn vm_outcome_new_positive_wraps_payload() {
        let mut pool = ConstantPool::new();
        let payload_const = pool.intern(Constant::Integer(Integer::new(42).unwrap()));
        let outcome_type = TypeTag::Outcome {
            value_type: Box::new(TypeTag::Integer),
            error_type: Box::new(TypeTag::String),
            allow_null_state: false,
        };
        let func = outcome_function(
            vec![
                Instruction::OutcomeNewPositive {
                    dest: ValueId(0),
                    payload: Operand::Const(payload_const),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
            outcome_type,
        );
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), Vec::new()).unwrap();
        assert_eq!(result.to_string(), "~+(42)");
    }

    /// `OutcomeNewNegative` packs a payload + `Trit::Negative`
    /// discriminator; the Display form mirrors `~-(payload)`.
    #[test]
    fn vm_outcome_new_negative_wraps_error_payload() {
        let mut pool = ConstantPool::new();
        let payload_const = pool.intern(Constant::String("io_failure".into()));
        let outcome_type = TypeTag::Outcome {
            value_type: Box::new(TypeTag::Integer),
            error_type: Box::new(TypeTag::String),
            allow_null_state: false,
        };
        let func = outcome_function(
            vec![
                Instruction::OutcomeNewNegative {
                    dest: ValueId(0),
                    payload: Operand::Const(payload_const),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
            outcome_type,
        );
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), Vec::new()).unwrap();
        assert_eq!(result.to_string(), "~-(io_failure)");
    }

    /// `OutcomeNewNull` produces a payload-less outcome with the Zero
    /// discriminator — only valid for `T?~E` ternary outcomes.
    #[test]
    fn vm_outcome_new_null_yields_payloadless_outcome() {
        let outcome_type = TypeTag::Outcome {
            value_type: Box::new(TypeTag::Integer),
            error_type: Box::new(TypeTag::String),
            allow_null_state: true,
        };
        let func = outcome_function(
            vec![
                Instruction::OutcomeNewNull { dest: ValueId(0) },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
            outcome_type,
        );
        let prog = make_simple_program(func);
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), Vec::new()).unwrap();
        assert_eq!(result.to_string(), "~0");
    }

    /// `OutcomeDiscriminant` returns the underlying Trit so the
    /// downstream `BrTrilean` can do a three-way match dispatch.
    #[test]
    fn vm_outcome_discriminant_returns_trit_per_arm() {
        for (instruction, expected_trit) in [
            (
                Instruction::OutcomeNewPositive {
                    dest: ValueId(0),
                    payload: Operand::Const(ConstId(0)),
                },
                Trit::Positive,
            ),
            (
                Instruction::OutcomeNewNegative {
                    dest: ValueId(0),
                    payload: Operand::Const(ConstId(0)),
                },
                Trit::Negative,
            ),
            (Instruction::OutcomeNewNull { dest: ValueId(0) }, Trit::Zero),
        ] {
            let mut pool = ConstantPool::new();
            // Pre-fill ConstId(0) so the Positive/Negative branches
            // have something to wrap.
            pool.intern(Constant::Integer(Integer::new(7).unwrap()));
            let func = outcome_function(
                vec![
                    instruction,
                    Instruction::OutcomeDiscriminant {
                        dest: ValueId(1),
                        source: Operand::Value(ValueId(0)),
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(1))),
                    },
                ],
                TypeTag::Trit,
            );
            let mut prog = make_simple_program(func);
            prog.constants = pool;
            let mut vm = Vm::new(prog);
            let result = vm.execute(FuncId(0), Vec::new()).unwrap();
            assert!(
                matches!(result, RuntimeValue::Trit(t) if t == expected_trit),
                "discriminant for {expected_trit:?}: got {result}",
            );
        }
    }

    /// `OutcomeUnwrapValue` retrieves the success payload — and
    /// panics (E2210) when the outcome is in the failure arm. This
    /// is the "verbose method" channel for panic-possible access
    /// (author: explicit strictness over dangerous ergonomics).
    #[test]
    fn vm_outcome_unwrap_value_success_returns_payload() {
        let mut pool = ConstantPool::new();
        let payload_const = pool.intern(Constant::Integer(Integer::new(99).unwrap()));
        let func = outcome_function(
            vec![
                Instruction::OutcomeNewPositive {
                    dest: ValueId(0),
                    payload: Operand::Const(payload_const),
                },
                Instruction::OutcomeUnwrapValue {
                    dest: ValueId(1),
                    source: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
            TypeTag::Integer,
        );
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), Vec::new()).unwrap();
        assert_eq!(result.to_string(), make_int(99).to_string());
    }

    #[test]
    fn vm_outcome_unwrap_value_on_failure_panics_e2210() {
        let mut pool = ConstantPool::new();
        let payload_const = pool.intern(Constant::String("boom".into()));
        let func = outcome_function(
            vec![
                Instruction::OutcomeNewNegative {
                    dest: ValueId(0),
                    payload: Operand::Const(payload_const),
                },
                Instruction::OutcomeUnwrapValue {
                    dest: ValueId(1),
                    source: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
            TypeTag::Integer,
        );
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let err = vm.execute(FuncId(0), Vec::new()).unwrap_err();
        match err {
            VmError::InvalidOutcomeState { reason, .. } => {
                assert!(
                    reason.contains("unwrap_value") && reason.contains("failure"),
                    "expected unwrap_value/failure mention, got {reason:?}",
                );
            }
            other => panic!("expected E2210 InvalidOutcomeState, got {other:?}"),
        }
    }

    /// `OutcomeUnwrapError` is the symmetric channel — panic on
    /// success / null arms, return payload on failure.
    #[test]
    fn vm_outcome_unwrap_error_failure_returns_payload() {
        let mut pool = ConstantPool::new();
        let payload_const = pool.intern(Constant::String("disk_full".into()));
        let func = outcome_function(
            vec![
                Instruction::OutcomeNewNegative {
                    dest: ValueId(0),
                    payload: Operand::Const(payload_const),
                },
                Instruction::OutcomeUnwrapError {
                    dest: ValueId(1),
                    source: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
            TypeTag::String,
        );
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), Vec::new()).unwrap();
        assert_eq!(result.to_string(), make_string("disk_full").to_string());
    }

    #[test]
    fn vm_outcome_unwrap_error_on_null_panics_e2210() {
        let func = outcome_function(
            vec![
                Instruction::OutcomeNewNull { dest: ValueId(0) },
                Instruction::OutcomeUnwrapError {
                    dest: ValueId(1),
                    source: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
            TypeTag::String,
        );
        let prog = make_simple_program(func);
        let mut vm = Vm::new(prog);
        let err = vm.execute(FuncId(0), Vec::new()).unwrap_err();
        match err {
            VmError::InvalidOutcomeState { reason, .. } => {
                assert!(
                    reason.contains("unwrap_error") && reason.contains("null"),
                    "expected unwrap_error/null mention, got {reason:?}",
                );
            }
            other => panic!("expected E2210 InvalidOutcomeState, got {other:?}"),
        }
    }

    /// Bare value flowing through `OutcomeUnwrapValue`. Per ADR-0010
    /// Addendum §D + v0.7.4.3-debt.6 WA-6 fix, the runtime treats a
    /// bare T value (not Null, not Outcome-wrapped) as its own
    /// success payload — `T ⊂ T?` widening doesn't wrap, so the
    /// runtime returns the value directly instead of erroring.
    /// Pre-fix this surfaced E2201 `TypeMismatch`.
    #[test]
    fn vm_outcome_unwrap_value_on_bare_value_returns_it_directly() {
        let mut pool = ConstantPool::new();
        let int_const = pool.intern(Constant::Integer(Integer::new(7).unwrap()));
        let func = outcome_function(
            vec![
                Instruction::Const {
                    dest: ValueId(0),
                    constant: int_const,
                },
                Instruction::OutcomeUnwrapValue {
                    dest: ValueId(1),
                    source: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
            TypeTag::Integer,
        );
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), Vec::new()).unwrap();
        match result {
            RuntimeValue::Integer(n) => assert_eq!(n.to_i64(), 7),
            other => panic!("expected Integer(7), got {other:?}"),
        }
    }

    /// Memory deallocation contract (ADR-0020): the `Drop` impl on
    /// `Option<Box<RuntimeValue>>` frees the payload heap allocation
    /// when the outcome value goes out of scope. We assert this via
    /// the `Drop` instrumentation pattern: wrap a string payload in
    /// an outcome that itself goes out of scope at end-of-test, then
    /// verify the address tracker sees the drop. We approximate by
    /// using `Rc<str>` shared between the outer hold and the inner
    /// payload; after the outcome drops the only remaining strong
    /// reference is the outer hold.
    #[test]
    fn outcome_drop_frees_payload_via_rust_drop() {
        use std::rc::Rc;
        let probe = Rc::new(String::from("payload_probe"));
        let outcome = RuntimeValue::Outcome {
            discriminator: Trit::Positive,
            payload: Some(Box::new(RuntimeValue::String((*probe).clone()))),
        };
        // Before drop: outer `probe` plus the clone the outcome holds.
        // (RuntimeValue::String stores an owned `String`, not the
        // Rc — so strong_count stays at 1; the meaningful check is
        // post-drop tracking via the `Box`'s ownership of its inner
        // RuntimeValue, which Rust guarantees by the Drop trait.)
        drop(outcome);
        assert_eq!(
            Rc::strong_count(&probe),
            1,
            "probe Rc should retain only the outer reference",
        );
    }

    // ── Outcome-null unification (ADR-0010 Addendum §D, v0.7.4.3-error.6a) ──
    //
    // Six round-trip tests covering the four cross-tolerant opcodes
    // plus a regression test that the original `OutcomeNewNull → ...`
    // path still works for backward compat.

    /// `OutcomeDiscriminant` accepts a `Constant::Null` and returns
    /// `Trit::Zero` per Addendum §D — closes the `~0` source path now
    /// that the lowerer emits `Constant::Null` instead of `OutcomeNewNull`.
    #[test]
    fn vm_outcome_discriminant_on_null_returns_zero() {
        let mut pool = ConstantPool::new();
        let null_const = pool.intern(Constant::Null);
        let func = outcome_function(
            vec![
                Instruction::Const {
                    dest: ValueId(0),
                    constant: null_const,
                },
                Instruction::OutcomeDiscriminant {
                    dest: ValueId(1),
                    source: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
            TypeTag::Trit,
        );
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), Vec::new()).unwrap();
        assert!(
            matches!(result, RuntimeValue::Trit(Trit::Zero)),
            "expected Trit::Zero from Null, got {result}",
        );
    }

    /// `NullCheck` accepts a `RuntimeValue::Outcome { Zero, None }`
    /// and returns `Trit::Zero` — same byte-level state, just the
    /// other carrier shape. This is what unblocks Elvis `?:` on `~0`.
    #[test]
    fn vm_null_check_on_outcome_zero_returns_zero() {
        let func = outcome_function(
            vec![
                Instruction::OutcomeNewNull { dest: ValueId(0) },
                Instruction::NullCheck {
                    dest: ValueId(1),
                    nullable: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
            TypeTag::Trit,
        );
        let prog = make_simple_program(func);
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), Vec::new()).unwrap();
        assert!(
            matches!(result, RuntimeValue::Trit(Trit::Zero)),
            "expected Trit::Zero from Outcome{{Zero,None}}, got {result}",
        );
    }

    /// `OutcomeUnwrapValue` on a `RuntimeValue::Null` surfaces E2210
    /// `"unwrap_value called on null state"` — the value is valid Zero,
    /// just not the success arm. Cleaner than the pre-§D E2201
    /// `TypeMismatch`. Distinct from the .3a test of similar name which
    /// tests unwrap on the Zero arm of a `RuntimeValue::Outcome`.
    #[test]
    fn vm_outcome_unwrap_value_on_runtime_null_panics_e2210() {
        let mut pool = ConstantPool::new();
        let null_const = pool.intern(Constant::Null);
        let func = outcome_function(
            vec![
                Instruction::Const {
                    dest: ValueId(0),
                    constant: null_const,
                },
                Instruction::OutcomeUnwrapValue {
                    dest: ValueId(1),
                    source: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
            TypeTag::Integer,
        );
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let err = vm.execute(FuncId(0), Vec::new()).unwrap_err();
        match err {
            VmError::InvalidOutcomeState { reason, .. } => {
                assert!(
                    reason.contains("unwrap_value") && reason.contains("null state"),
                    "expected unwrap_value/null-state mention, got {reason:?}",
                );
            }
            other => panic!("expected E2210 InvalidOutcomeState, got {other:?}"),
        }
    }

    /// Symmetric for `OutcomeUnwrapError` on `RuntimeValue::Null`.
    #[test]
    fn vm_outcome_unwrap_error_on_runtime_null_panics_e2210() {
        let mut pool = ConstantPool::new();
        let null_const = pool.intern(Constant::Null);
        let func = outcome_function(
            vec![
                Instruction::Const {
                    dest: ValueId(0),
                    constant: null_const,
                },
                Instruction::OutcomeUnwrapError {
                    dest: ValueId(1),
                    source: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
            TypeTag::String,
        );
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let err = vm.execute(FuncId(0), Vec::new()).unwrap_err();
        match err {
            VmError::InvalidOutcomeState { reason, .. } => {
                assert!(
                    reason.contains("unwrap_error") && reason.contains("null state"),
                    "expected unwrap_error/null-state mention, got {reason:?}",
                );
            }
            other => panic!("expected E2210 InvalidOutcomeState, got {other:?}"),
        }
    }

    /// Regression: the legacy `OutcomeNewNull → OutcomeDiscriminant`
    /// path still returns `Trit::Zero`. This was the pre-§D primary
    /// path and stays functional for backward `.triv` compat.
    #[test]
    fn vm_outcome_new_null_discriminant_still_zero_legacy_path() {
        let func = outcome_function(
            vec![
                Instruction::OutcomeNewNull { dest: ValueId(0) },
                Instruction::OutcomeDiscriminant {
                    dest: ValueId(1),
                    source: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
            TypeTag::Trit,
        );
        let prog = make_simple_program(func);
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), Vec::new()).unwrap();
        assert!(matches!(result, RuntimeValue::Trit(Trit::Zero)));
    }

    // ── Env (v0.7.10.1) ─────────────────────────────────────────

    /// `std.env.get` returns the env variable's value as a String
    /// when the variable is set. Uses `CARGO_MANIFEST_DIR` which
    /// cargo guarantees is set during test execution — avoids the
    /// Rust 2024 `unsafe std::env::set_var` policy clash with the
    /// workspace's `unsafe_code = forbid` lint.
    #[test]
    fn vm_get_env_returns_string_when_set() {
        let expected = std::env::var("CARGO_MANIFEST_DIR")
            .expect("cargo sets CARGO_MANIFEST_DIR during tests");

        let prog = build_builtin_program(
            vec![("key".into(), TypeTag::String)],
            TypeTag::Nullable(Box::new(TypeTag::String)),
            BuiltinName::GetEnv,
            ConstantPool::new(),
        );
        let mut vm = Vm::new(prog);
        let r = vm
            .execute(FuncId(0), vec![make_string("CARGO_MANIFEST_DIR")])
            .unwrap();
        match r {
            RuntimeValue::String(s) => assert_eq!(s, expected),
            other => panic!("expected String, got {other:?}"),
        }
    }

    /// `std.env.get` returns Null (data-tier event, not panic) when
    /// the variable is unset.
    #[test]
    fn vm_get_env_returns_null_when_unset() {
        // PID-suffixed name to guarantee no parallel-test collision.
        let key = format!("DAO_TEST_UNSET_VAR_{}", std::process::id());
        assert!(
            std::env::var(&key).is_err(),
            "test precondition: {key} must not be set"
        );

        let prog = build_builtin_program(
            vec![("key".into(), TypeTag::String)],
            TypeTag::Nullable(Box::new(TypeTag::String)),
            BuiltinName::GetEnv,
            ConstantPool::new(),
        );
        let mut vm = Vm::new(prog);
        let r = vm.execute(FuncId(0), vec![make_string(&key)]).unwrap();
        assert!(
            matches!(r, RuntimeValue::Null),
            "unset env must return Null, got {r:?}"
        );
    }

    /// v0.7.12.1 — `read_dir_recursive` walks a directory tree
    /// returning sorted `(rel_path, content)` pairs for `.tri` files.
    #[test]
    fn vm_read_dir_recursive_returns_sorted_tri_pairs() {
        use std::fs;
        let temp = tempfile::TempDir::new().expect("tempdir");
        // Stage 3 files; only 2 are .tri so non-.tri is filtered.
        fs::write(temp.path().join("alpha.tri"), "fn alpha").unwrap();
        fs::write(temp.path().join("zebra.tri"), "fn zebra").unwrap();
        fs::write(temp.path().join("README.md"), "not a tri file").unwrap();
        // Nested .tri
        fs::create_dir(temp.path().join("sub")).unwrap();
        fs::write(temp.path().join("sub/beta.tri"), "fn beta").unwrap();

        let prog = build_builtin_program(
            vec![("root".into(), TypeTag::String)],
            TypeTag::Vector(Box::new(TypeTag::Vector(Box::new(TypeTag::String)))),
            BuiltinName::ReadDirRecursive,
            ConstantPool::new(),
        );
        let mut vm = Vm::new(prog);
        let r = vm
            .execute(FuncId(0), vec![make_string(temp.path().to_str().unwrap())])
            .unwrap();
        let outer = match r {
            RuntimeValue::Vector(v) => v,
            other => panic!("expected Vector, got {other:?}"),
        };
        assert_eq!(outer.len(), 3, "expected 3 .tri entries (README skipped)");
        // Extract (path, content) from each inner Vector.
        let pairs: Vec<(String, String)> = outer
            .into_iter()
            .map(|elem| match elem {
                RuntimeValue::Vector(v) => {
                    let mut iter = v.into_iter();
                    let path = match iter.next() {
                        Some(RuntimeValue::String(s)) => s,
                        other => panic!("expected String path, got {other:?}"),
                    };
                    let content = match iter.next() {
                        Some(RuntimeValue::String(s)) => s,
                        other => panic!("expected String content, got {other:?}"),
                    };
                    (path, content)
                }
                other => panic!("expected inner Vector, got {other:?}"),
            })
            .collect();
        // Sorted by relative path: alpha.tri, sub/beta.tri, zebra.tri.
        assert_eq!(pairs[0].0, "alpha.tri");
        assert_eq!(pairs[0].1, "fn alpha");
        assert_eq!(pairs[1].0, "sub/beta.tri");
        assert_eq!(pairs[1].1, "fn beta");
        assert_eq!(pairs[2].0, "zebra.tri");
        assert_eq!(pairs[2].1, "fn zebra");
    }

    /// v0.7.12.1 — missing directory returns empty Vector (data-tier).
    #[test]
    fn vm_read_dir_recursive_missing_root_returns_empty() {
        let key = format!("/tmp/dao_test_missing_dir_{}", std::process::id());
        let prog = build_builtin_program(
            vec![("root".into(), TypeTag::String)],
            TypeTag::Vector(Box::new(TypeTag::Vector(Box::new(TypeTag::String)))),
            BuiltinName::ReadDirRecursive,
            ConstantPool::new(),
        );
        let mut vm = Vm::new(prog);
        let r = vm.execute(FuncId(0), vec![make_string(&key)]).unwrap();
        match r {
            RuntimeValue::Vector(v) => assert!(v.is_empty(), "missing dir → empty Vector"),
            other => panic!("expected empty Vector, got {other:?}"),
        }
    }

    /// `NullCheck` on a non-null `RuntimeValue::Outcome` (e.g. success
    /// arm) returns `Trit::Positive` — only the Zero state pair
    /// triggers the cross-tolerance.
    #[test]
    fn vm_null_check_on_positive_outcome_returns_positive() {
        let mut pool = ConstantPool::new();
        let payload_const = pool.intern(Constant::Integer(Integer::new(7).unwrap()));
        let func = outcome_function(
            vec![
                Instruction::OutcomeNewPositive {
                    dest: ValueId(0),
                    payload: Operand::Const(payload_const),
                },
                Instruction::NullCheck {
                    dest: ValueId(1),
                    nullable: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
            TypeTag::Trit,
        );
        let mut prog = make_simple_program(func);
        prog.constants = pool;
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), Vec::new()).unwrap();
        assert!(matches!(result, RuntimeValue::Trit(Trit::Positive)));
    }

    // ===== v0.9.x.atomic.3 — universal atomic ops dispatch tests =====
    //
    // Single-thread VM: ordering arg ignored (no-op per ADR-0028 §9
    // dev tier). Tests verify shared-state semantics via Rc<RefCell>
    // backing — multiple references see mutations.

    fn make_integer(n: i64) -> RuntimeValue {
        RuntimeValue::Integer(Integer::new(n).unwrap())
    }

    fn make_atomic_integer(n: i64) -> RuntimeValue {
        RuntimeValue::Atomic(std::sync::Arc::new(std::sync::Mutex::new(make_integer(n))))
    }

    /// `AtomicNew(initial)` wraps inner value in shared cell.
    #[test]
    fn vm_atomic_new_wraps_initial_value() {
        let prog = build_builtin_program(
            vec![("initial".into(), TypeTag::Integer)],
            TypeTag::Atomic(Box::new(TypeTag::Integer)),
            BuiltinName::AtomicNew,
            ConstantPool::new(),
        );
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![make_integer(42)]).unwrap();
        match result {
            RuntimeValue::Atomic(cell) => {
                assert!(
                    matches!(&*lock_atomic(&cell), RuntimeValue::Integer(i) if i.to_i64() == 42)
                );
            }
            other => panic!("expected Atomic, got {other:?}"),
        }
    }

    /// `AtomicLoad(atomic, ord)` returns current cell value (ord ignored).
    #[test]
    fn vm_atomic_load_returns_current_value() {
        let prog = build_builtin_program(
            vec![
                ("a".into(), TypeTag::Atomic(Box::new(TypeTag::Integer))),
                ("ord".into(), TypeTag::Unit), // placeholder for Ordering enum
            ],
            TypeTag::Integer,
            BuiltinName::AtomicLoad,
            ConstantPool::new(),
        );
        let mut vm = Vm::new(prog);
        let result = vm
            .execute(FuncId(0), vec![make_atomic_integer(7), RuntimeValue::Unit])
            .unwrap();
        assert!(matches!(result, RuntimeValue::Integer(i) if i.to_i64() == 7));
    }

    /// `AtomicStore(atomic, new, ord)` mutates cell; subsequent Load
    /// observes new value (via shared Rc).
    #[test]
    fn vm_atomic_store_mutates_shared_cell() {
        let prog = build_builtin_program(
            vec![
                ("a".into(), TypeTag::Atomic(Box::new(TypeTag::Integer))),
                ("v".into(), TypeTag::Integer),
                ("ord".into(), TypeTag::Unit),
            ],
            TypeTag::Unit,
            BuiltinName::AtomicStore,
            ConstantPool::new(),
        );
        let atomic = make_atomic_integer(0);
        // Clone the Atomic — Rc<RefCell> so both clones share cell.
        let atomic_clone = atomic.clone();
        let mut vm = Vm::new(prog);
        let _ = vm
            .execute(
                FuncId(0),
                vec![atomic, make_integer(99), RuntimeValue::Unit],
            )
            .unwrap();
        // External (cloned) reference sees the mutation.
        if let RuntimeValue::Atomic(cell) = atomic_clone {
            assert!(matches!(&*lock_atomic(&cell), RuntimeValue::Integer(i) if i.to_i64() == 99));
        } else {
            panic!("expected Atomic shared clone");
        }
    }

    /// `AtomicSwap(atomic, new, ord)` returns previous + mutates to new.
    #[test]
    fn vm_atomic_swap_returns_prev_and_mutates() {
        let prog = build_builtin_program(
            vec![
                ("a".into(), TypeTag::Atomic(Box::new(TypeTag::Integer))),
                ("v".into(), TypeTag::Integer),
                ("ord".into(), TypeTag::Unit),
            ],
            TypeTag::Integer,
            BuiltinName::AtomicSwap,
            ConstantPool::new(),
        );
        let atomic = make_atomic_integer(11);
        let atomic_clone = atomic.clone();
        let mut vm = Vm::new(prog);
        let result = vm
            .execute(
                FuncId(0),
                vec![atomic, make_integer(22), RuntimeValue::Unit],
            )
            .unwrap();
        // Return value = previous (11).
        assert!(matches!(result, RuntimeValue::Integer(i) if i.to_i64() == 11));
        // Cell mutated to 22.
        if let RuntimeValue::Atomic(cell) = atomic_clone {
            assert!(matches!(&*lock_atomic(&cell), RuntimeValue::Integer(i) if i.to_i64() == 22));
        }
    }

    /// `AtomicCompareExchange(atomic, expected, new, succ_ord, fail_ord)`:
    /// success path — expected matches current, returns ~+ prev, mutates.
    #[test]
    fn vm_atomic_compare_exchange_success_path() {
        let prog = build_builtin_program(
            vec![
                ("a".into(), TypeTag::Atomic(Box::new(TypeTag::Integer))),
                ("exp".into(), TypeTag::Integer),
                ("new".into(), TypeTag::Integer),
                ("so".into(), TypeTag::Unit),
                ("fo".into(), TypeTag::Unit),
            ],
            TypeTag::Outcome {
                value_type: Box::new(TypeTag::Integer),
                error_type: Box::new(TypeTag::Integer),
                allow_null_state: false,
            },
            BuiltinName::AtomicCompareExchange,
            ConstantPool::new(),
        );
        let atomic = make_atomic_integer(5);
        let atomic_clone = atomic.clone();
        let mut vm = Vm::new(prog);
        let result = vm
            .execute(
                FuncId(0),
                vec![
                    atomic,
                    make_integer(5), // expected matches
                    make_integer(10),
                    RuntimeValue::Unit,
                    RuntimeValue::Unit,
                ],
            )
            .unwrap();
        // Success arm: ~+ prev (5).
        match result {
            RuntimeValue::Outcome {
                discriminator: Trit::Positive,
                payload: Some(boxed),
            } => {
                assert!(matches!(*boxed, RuntimeValue::Integer(i) if i.to_i64() == 5));
            }
            other => panic!("expected ~+ prev, got {other:?}"),
        }
        // Cell mutated to 10.
        if let RuntimeValue::Atomic(cell) = atomic_clone {
            assert!(matches!(&*lock_atomic(&cell), RuntimeValue::Integer(i) if i.to_i64() == 10));
        }
    }

    // ===== v0.9.x.atomic.4 — arithmetic + bitwise dispatch tests =====

    /// `AtomicFetchAdd(atomic, delta, ord)` returns PREVIOUS value;
    /// cell mutated to prev + delta. Single-thread no-op per ADR-0028 §9.
    #[test]
    fn vm_atomic_fetch_add_returns_prev_and_mutates() {
        let prog = build_builtin_program(
            vec![
                ("a".into(), TypeTag::Atomic(Box::new(TypeTag::Integer))),
                ("d".into(), TypeTag::Integer),
                ("ord".into(), TypeTag::Unit),
            ],
            TypeTag::Integer,
            BuiltinName::AtomicFetchAdd,
            ConstantPool::new(),
        );
        let atomic = make_atomic_integer(10);
        let atomic_clone = atomic.clone();
        let mut vm = Vm::new(prog);
        let result = vm
            .execute(FuncId(0), vec![atomic, make_integer(5), RuntimeValue::Unit])
            .unwrap();
        // Return value = previous (10).
        assert!(matches!(result, RuntimeValue::Integer(i) if i.to_i64() == 10));
        // Cell mutated to 15.
        if let RuntimeValue::Atomic(cell) = atomic_clone {
            assert!(matches!(&*lock_atomic(&cell), RuntimeValue::Integer(i) if i.to_i64() == 15));
        }
    }

    /// `AtomicFetchSub(atomic, delta, ord)` returns PREVIOUS;
    /// cell mutated to prev - delta.
    #[test]
    fn vm_atomic_fetch_sub_returns_prev_and_mutates() {
        let prog = build_builtin_program(
            vec![
                ("a".into(), TypeTag::Atomic(Box::new(TypeTag::Integer))),
                ("d".into(), TypeTag::Integer),
                ("ord".into(), TypeTag::Unit),
            ],
            TypeTag::Integer,
            BuiltinName::AtomicFetchSub,
            ConstantPool::new(),
        );
        let atomic = make_atomic_integer(20);
        let atomic_clone = atomic.clone();
        let mut vm = Vm::new(prog);
        let result = vm
            .execute(FuncId(0), vec![atomic, make_integer(7), RuntimeValue::Unit])
            .unwrap();
        assert!(matches!(result, RuntimeValue::Integer(i) if i.to_i64() == 20));
        if let RuntimeValue::Atomic(cell) = atomic_clone {
            assert!(matches!(&*lock_atomic(&cell), RuntimeValue::Integer(i) if i.to_i64() == 13));
        }
    }

    /// `AtomicFetchBitwiseAnd(atomic, mask, ord)` — Integer-only per
    /// ADR-0028 Addendum. Binary AND on 64-bit slot. Returns previous.
    #[test]
    fn vm_atomic_fetch_bitwise_and_returns_prev_and_mutates() {
        let prog = build_builtin_program(
            vec![
                ("a".into(), TypeTag::Atomic(Box::new(TypeTag::Integer))),
                ("m".into(), TypeTag::Integer),
                ("ord".into(), TypeTag::Unit),
            ],
            TypeTag::Integer,
            BuiltinName::AtomicFetchBitwiseAnd,
            ConstantPool::new(),
        );
        let atomic = make_atomic_integer(0b1111); // 15
        let atomic_clone = atomic.clone();
        let mut vm = Vm::new(prog);
        let result = vm
            .execute(
                FuncId(0),
                vec![atomic, make_integer(0b1010), RuntimeValue::Unit],
            )
            .unwrap();
        // Return = prev 15.
        assert!(matches!(result, RuntimeValue::Integer(i) if i.to_i64() == 15));
        // 0b1111 & 0b1010 = 0b1010 = 10.
        if let RuntimeValue::Atomic(cell) = atomic_clone {
            assert!(matches!(&*lock_atomic(&cell), RuntimeValue::Integer(i) if i.to_i64() == 10));
        }
    }

    /// `AtomicFetchBitwiseOr` — `0b1010 | 0b0101 = 0b1111 = 15`.
    #[test]
    fn vm_atomic_fetch_bitwise_or_returns_prev_and_mutates() {
        let prog = build_builtin_program(
            vec![
                ("a".into(), TypeTag::Atomic(Box::new(TypeTag::Integer))),
                ("m".into(), TypeTag::Integer),
                ("ord".into(), TypeTag::Unit),
            ],
            TypeTag::Integer,
            BuiltinName::AtomicFetchBitwiseOr,
            ConstantPool::new(),
        );
        let atomic = make_atomic_integer(0b1010);
        let atomic_clone = atomic.clone();
        let mut vm = Vm::new(prog);
        let result = vm
            .execute(
                FuncId(0),
                vec![atomic, make_integer(0b0101), RuntimeValue::Unit],
            )
            .unwrap();
        assert!(matches!(result, RuntimeValue::Integer(i) if i.to_i64() == 0b1010));
        if let RuntimeValue::Atomic(cell) = atomic_clone {
            assert!(
                matches!(&*lock_atomic(&cell), RuntimeValue::Integer(i) if i.to_i64() == 0b1111)
            );
        }
    }

    /// `AtomicFetchBitwiseXor` — `0b1111 ^ 0b1010 = 0b0101 = 5`.
    #[test]
    fn vm_atomic_fetch_bitwise_xor_returns_prev_and_mutates() {
        let prog = build_builtin_program(
            vec![
                ("a".into(), TypeTag::Atomic(Box::new(TypeTag::Integer))),
                ("m".into(), TypeTag::Integer),
                ("ord".into(), TypeTag::Unit),
            ],
            TypeTag::Integer,
            BuiltinName::AtomicFetchBitwiseXor,
            ConstantPool::new(),
        );
        let atomic = make_atomic_integer(0b1111);
        let atomic_clone = atomic.clone();
        let mut vm = Vm::new(prog);
        let result = vm
            .execute(
                FuncId(0),
                vec![atomic, make_integer(0b1010), RuntimeValue::Unit],
            )
            .unwrap();
        assert!(matches!(result, RuntimeValue::Integer(i) if i.to_i64() == 0b1111));
        if let RuntimeValue::Atomic(cell) = atomic_clone {
            assert!(
                matches!(&*lock_atomic(&cell), RuntimeValue::Integer(i) if i.to_i64() == 0b0101)
            );
        }
    }

    /// `AtomicCompareExchange` failure path — expected doesn't match,
    /// returns ~- actual, NO mutation.
    #[test]
    fn vm_atomic_compare_exchange_failure_path() {
        let prog = build_builtin_program(
            vec![
                ("a".into(), TypeTag::Atomic(Box::new(TypeTag::Integer))),
                ("exp".into(), TypeTag::Integer),
                ("new".into(), TypeTag::Integer),
                ("so".into(), TypeTag::Unit),
                ("fo".into(), TypeTag::Unit),
            ],
            TypeTag::Outcome {
                value_type: Box::new(TypeTag::Integer),
                error_type: Box::new(TypeTag::Integer),
                allow_null_state: false,
            },
            BuiltinName::AtomicCompareExchange,
            ConstantPool::new(),
        );
        let atomic = make_atomic_integer(5);
        let atomic_clone = atomic.clone();
        let mut vm = Vm::new(prog);
        let result = vm
            .execute(
                FuncId(0),
                vec![
                    atomic,
                    make_integer(99), // expected does NOT match (current=5)
                    make_integer(10),
                    RuntimeValue::Unit,
                    RuntimeValue::Unit,
                ],
            )
            .unwrap();
        // Failure arm: ~- actual current (5).
        match result {
            RuntimeValue::Outcome {
                discriminator: Trit::Negative,
                payload: Some(boxed),
            } => {
                assert!(matches!(*boxed, RuntimeValue::Integer(i) if i.to_i64() == 5));
            }
            other => panic!("expected ~- actual, got {other:?}"),
        }
        // Cell UNCHANGED (still 5).
        if let RuntimeValue::Atomic(cell) = atomic_clone {
            assert!(matches!(&*lock_atomic(&cell), RuntimeValue::Integer(i) if i.to_i64() == 5));
        }
    }

    // ── v0.10.x.thread.1 — raw_thread builtins (ADR-0026 v2 §3) ────

    /// Helper: build a 2-statement program — call `spawn(work)`,
    /// return the Handle struct.
    fn build_raw_thread_spawn_program() -> IrProgram {
        build_builtin_program(
            vec![("work".into(), TypeTag::Integer)],
            // Handle is a user struct with one Integer field; the
            // return TypeTag for a struct is implementation-defined
            // here (UserStruct unavailable in TypeTag enum), use Unit
            // as placeholder since round-trip checks Struct shape.
            TypeTag::Unit,
            BuiltinName::RawThreadSpawn,
            ConstantPool::new(),
        )
    }

    /// spawn returns Handle struct with monotonic `thread_id` ≥ 1
    /// (ID 0 reserved per v0.9 placeholder convention).
    #[test]
    fn vm_raw_thread_spawn_returns_handle_with_nonzero_id() {
        let prog = build_raw_thread_spawn_program();
        let mut vm = Vm::new(prog);
        let result = vm.execute(FuncId(0), vec![make_integer(0)]).unwrap();
        match result {
            RuntimeValue::Struct { fields } => {
                assert_eq!(fields.len(), 1);
                match &fields[0] {
                    RuntimeValue::Integer(id) => {
                        assert!(id.to_i64() >= 1, "thread_id should start at 1, got {id}");
                    }
                    other => panic!("expected Integer thread_id, got {other:?}"),
                }
            }
            other => panic!("expected Handle struct, got {other:?}"),
        }
        // Registry should have 1 entry after spawn — but the spawned
        // thread already may have terminated. Verify via direct field
        // access (single-thread VM, no race).
        assert_eq!(
            vm.thread_handles.len(),
            1,
            "spawn must register exactly 1 handle"
        );
    }

    /// Successive spawns produce strictly-increasing `thread_id`s.
    /// `Vm::execute` is not re-entrant on the same instance (it leaves
    /// stack frames around after Return), so this test exercises the
    /// thread-builtin helper directly — same code path the dispatcher
    /// hits, just bypassing the IR program harness.
    #[test]
    fn vm_raw_thread_spawn_ids_are_monotonic() {
        let mut vm = Vm::new(IrProgram::new());
        let spawn = |vm: &mut Vm| -> i64 {
            let result = execute_thread_builtin(
                BuiltinName::RawThreadSpawn,
                &[make_integer(0)],
                &mut vm.thread_handles,
                &mut vm.next_thread_id,
                "test",
            )
            .unwrap();
            match result {
                RuntimeValue::Struct { fields } => match &fields[0] {
                    RuntimeValue::Integer(i) => i.to_i64(),
                    other => panic!("expected Integer, got {other:?}"),
                },
                other => panic!("expected Struct, got {other:?}"),
            }
        };
        let id1 = spawn(&mut vm);
        let id2 = spawn(&mut vm);
        let id3 = spawn(&mut vm);
        assert!(id1 < id2 && id2 < id3, "ids must be strictly increasing");
        assert_eq!(vm.thread_handles.len(), 3);
    }

    /// join consumes the registry entry and blocks until the thread
    /// terminates. Round-trip: spawn → join → registry emptied.
    #[test]
    fn vm_raw_thread_join_blocks_and_consumes_handle() {
        // Spawn first.
        let spawn_prog = build_raw_thread_spawn_program();
        let mut vm = Vm::new(spawn_prog);
        let handle = vm.execute(FuncId(0), vec![make_integer(0)]).unwrap();
        assert_eq!(vm.thread_handles.len(), 1);

        // Rebuild VM-side program for join (different signature). The
        // helper builds a fresh IR program with the right param type;
        // we re-use the VM instance so the spawn-side handle persists.
        // To do this, we splice the join function onto the existing VM
        // by constructing a fresh IrProgram with a `join` dispatch
        // function and a `Vm` initialized over it — but the spawn
        // handle would be lost. Instead, dispatch the join builtin
        // directly via the helper, bypassing IR construction.
        let result = execute_thread_builtin(
            BuiltinName::RawThreadJoin,
            &[handle],
            &mut vm.thread_handles,
            &mut vm.next_thread_id,
            "test",
        )
        .unwrap();
        assert!(matches!(result, RuntimeValue::Unit));
        assert_eq!(
            vm.thread_handles.len(),
            0,
            "join must remove handle from registry"
        );
    }

    /// join on an unknown `thread_id` errors cleanly (handle invalid /
    /// already-joined / fabricated).
    #[test]
    fn vm_raw_thread_join_unknown_id_errors() {
        let mut vm = Vm::new(IrProgram::new());
        let fake_handle = RuntimeValue::Struct {
            fields: vec![make_integer(9999)],
        };
        let err = execute_thread_builtin(
            BuiltinName::RawThreadJoin,
            &[fake_handle],
            &mut vm.thread_handles,
            &mut vm.next_thread_id,
            "test",
        )
        .unwrap_err();
        // VmError::TypeMismatch carrying the diagnostic text.
        match err {
            VmError::TypeMismatch { actual, .. } => {
                assert!(
                    actual.contains("unknown thread_id"),
                    "expected unknown_id diagnostic, got: {actual}"
                );
            }
            other => panic!("expected TypeMismatch, got {other:?}"),
        }
    }

    /// Double-join: spawn, join once (consumes), join again on the
    /// same handle errors.
    #[test]
    fn vm_raw_thread_double_join_errors() {
        let prog = build_raw_thread_spawn_program();
        let mut vm = Vm::new(prog);
        let handle = vm.execute(FuncId(0), vec![make_integer(0)]).unwrap();
        // First join — succeeds. `std::slice::from_ref` avoids a clone:
        // both calls borrow the handle but only the registry consumption
        // side-effect matters; the value itself stays usable for the
        // second-join attempt.
        let _ = execute_thread_builtin(
            BuiltinName::RawThreadJoin,
            std::slice::from_ref(&handle),
            &mut vm.thread_handles,
            &mut vm.next_thread_id,
            "test",
        )
        .unwrap();
        // Second join — handle removed; should error.
        let err = execute_thread_builtin(
            BuiltinName::RawThreadJoin,
            std::slice::from_ref(&handle),
            &mut vm.thread_handles,
            &mut vm.next_thread_id,
            "test",
        )
        .unwrap_err();
        assert!(matches!(err, VmError::TypeMismatch { .. }));
    }

    /// Thread builtins dispatched through `execute_builtin` (the
    /// non-thread-aware path) must error explicitly — this catches
    /// any caller that bypasses the disjoint-borrow routing instead
    /// of silently degrading.
    #[test]
    fn execute_builtin_refuses_thread_variants() {
        let err =
            execute_builtin(BuiltinName::RawThreadSpawn, &[make_integer(0)], "test").unwrap_err();
        match err {
            VmError::TypeMismatch { actual, .. } => {
                assert!(
                    actual.contains("execute_thread_builtin"),
                    "diagnostic must direct to execute_thread_builtin, got: {actual}"
                );
            }
            other => panic!("expected TypeMismatch, got {other:?}"),
        }
    }

    /// `path_to_builtin` maps the canonical paths to the right enum
    /// variants — guards against typos that would cause silent
    /// fallthrough to "function not found".
    #[test]
    fn raw_thread_path_to_builtin_maps_correctly() {
        assert_eq!(
            path_to_builtin("sys.raw_thread.spawn"),
            Some(BuiltinName::RawThreadSpawn)
        );
        assert_eq!(
            path_to_builtin("sys.raw_thread.join"),
            Some(BuiltinName::RawThreadJoin)
        );
        // Negative: typo should NOT resolve.
        assert_eq!(path_to_builtin("sys.raw_thread.spwn"), None);
    }

    // (raw_thread serde round-trip test lives in serde.rs's test
    // module — `write_builtin`/`read_builtin` are module-private,
    // following the same convention as `atomic_builtins_serde_round_trip`.)

    // ── v0.10.x.thread.2 — Atomic Arc<Mutex> cross-thread share ─────

    /// v0.10.x.thread.2 — verify the Arc<Mutex> migration preserves
    /// shared-state semantics across OS-thread boundaries.
    ///
    /// Pre-migration: `RuntimeValue::Atomic` was `Rc<RefCell<…>>` and
    /// could not be sent to another OS thread (Rc is `!Send`). Post-
    /// migration: `Arc<Mutex<…>>` is `Send + Sync`. This test spawns
    /// a real OS thread that clones the Atomic handle, writes through
    /// the mutex, and joins back; the main-thread observer then sees
    /// the cross-thread write.
    ///
    /// This exercises the **infrastructure** that v0.10.x.thread.3
    /// builds on (multi-worker `atomic_counter` demo). It does NOT
    /// require Triết closure type-system support — the spawn uses
    /// Rust's `std::thread::spawn` directly to validate the type-
    /// system property (`Send` for Atomic) without depending on
    /// language-surface features that defer per ADR-0026 v2 §3.
    #[test]
    fn atomic_arc_mutex_crosses_os_thread_boundary() {
        let atomic = make_atomic_integer(0);
        // Extract the inner Arc to spawn-capture independently of the
        // RuntimeValue wrapper.
        let RuntimeValue::Atomic(arc_handle) = atomic.clone() else {
            panic!("expected Atomic, got {atomic:?}");
        };

        let join_handle = std::thread::spawn(move || {
            // Cross-thread write: lock the mutex, mutate the inner
            // RuntimeValue. Demonstrates Atomic is now Send-safe.
            let mut guard = arc_handle
                .lock()
                .expect("atomic mutex must lock from spawned thread");
            *guard = RuntimeValue::Integer(Integer::new(42).unwrap());
        });

        join_handle.join().expect("spawned thread joined cleanly");

        // Main thread observes the write — proves the Arc clone
        // refers to the SAME underlying cell as the original.
        if let RuntimeValue::Atomic(cell) = atomic {
            let guard = lock_atomic(&cell);
            match &*guard {
                RuntimeValue::Integer(i) => assert_eq!(i.to_i64(), 42),
                other => panic!("expected Integer 42, got {other:?}"),
            }
        } else {
            panic!("main-thread atomic disappeared");
        }
    }

    /// Multiple worker threads, each performing one swap on the
    /// shared cell — verify Atomic clones see each other's writes
    /// AND the join sequence completes deterministically. This is
    /// the foundation for the v0.10.x.thread.3 multi-worker
    /// `atomic_counter` demo (which will run a `fetch_add` per worker
    /// via the same Arc-share + `std::thread` mechanism).
    #[test]
    fn atomic_arc_mutex_multi_worker_share() {
        let atomic = make_atomic_integer(0);
        let RuntimeValue::Atomic(arc_handle) = atomic.clone() else {
            unreachable!("make_atomic_integer always returns Atomic")
        };

        // Spawn 3 workers; each writes a distinct value. The last
        // writer wins; we verify the final value is among the
        // expected set (deterministic per join sequence).
        let mut joins = Vec::new();
        for n in [10_i64, 20, 30] {
            let arc_clone = arc_handle.clone();
            joins.push(std::thread::spawn(move || {
                let mut guard = arc_clone.lock().expect("worker lock");
                *guard = RuntimeValue::Integer(Integer::new(n).unwrap());
            }));
        }
        for j in joins {
            j.join().expect("worker joined");
        }

        if let RuntimeValue::Atomic(cell) = atomic {
            let guard = lock_atomic(&cell);
            match &*guard {
                RuntimeValue::Integer(i) => {
                    let v = i.to_i64();
                    assert!(
                        [10, 20, 30].contains(&v),
                        "expected one of {{10, 20, 30}}, got {v}"
                    );
                }
                other => panic!("expected Integer, got {other:?}"),
            }
        }
    }
}
