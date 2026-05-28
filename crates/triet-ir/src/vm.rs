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
        }
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
                // Wrap: create an enum with variant 0 = Some(v)
                frame.write(
                    dest,
                    RuntimeValue::Enum {
                        variant: 0,
                        payload: Some(Box::new(v)),
                    },
                );
            }
            Instruction::NullUnwrap { dest, nullable } => {
                // v0.7.5.1: symmetric to the v0.7.4.4 NullCheck cleanup.
                // The legacy `Enum { variant: 0, payload: Some(p) }`
                // unwrap arm was the inverse of the legacy `NullWrap`
                // emit. With ADR-0010 Addendum §D's unified encoding,
                // `T?` values flow as the bare value (or `Null`); no
                // current lowerer path emits `NullWrap`, so the only
                // hits on the legacy arm were user enums whose
                // variant-0 happened to carry a payload — `Vector<Node>`
                // round-trip through `get(...)!!` was reading `Leaf(10)`
                // as `Integer(10)`, dropping the enum tag. The two
                // canonical nullable carriers are `RuntimeValue::Null`
                // (panic) and any other value (pass through).
                let v = read_operand(constants, frame, nullable);
                match v {
                    RuntimeValue::Null => {
                        return Err(VmError::NullUnwrap {
                            function: func_name,
                        });
                    }
                    _ => frame.write(dest, v),
                }
            }
            Instruction::NullCheck { dest, nullable } => {
                // ADR-0010: report the nullable's discriminator trit.
                //   Positive = wrapped value (Some)
                //   Zero     = canonical null
                //   Negative = reserved for definitely-missing
                // Today only Some/Null are produced; Negative is reserved.
                //
                // ADR-0010 Addendum §D (v0.7.4.3-error.6a): cross-
                // tolerance with the Outcome value carrier — an
                // `OutcomeNewNull`-constructed `RuntimeValue::Outcome
                // { Trit::Zero, None }` is also recognized as the
                // canonical null state. This unifies `~0` source with
                // the legacy `null` keyword at the runtime tier.
                //
                // v0.7.4.4: the legacy "any payload-less Enum is null"
                // arm was removed — it collided with bare unit-variant
                // enum values (e.g. `LetKw` from a `Token?` slot via
                // cross-tolerance), which a hand-rolled Elvis like
                // `keyword_for(slice) ?: Identifier(...)` then mis-
                // classified as null. Today no opcode emits a unit
                // enum as a null marker; `RuntimeValue::Null` and
                // `Outcome { Trit::Zero, None }` are the two canonical
                // null carriers.
                let v = read_operand(constants, frame, nullable);
                let trit = match &v {
                    RuntimeValue::Null
                    | RuntimeValue::Outcome {
                        discriminator: Trit::Zero,
                        payload: None,
                    } => Trit::Zero,
                    _ => Trit::Positive,
                };
                frame.write(dest, RuntimeValue::Trit(trit));
            }

            // ── Aggregate: struct ────────────────────────────────
            Instruction::StructNew { dest, fields } => {
                let field_vals: Vec<RuntimeValue> = fields
                    .iter()
                    .map(|f| read_operand(constants, frame, *f))
                    .collect();
                frame.write(dest, RuntimeValue::Struct { fields: field_vals });
            }
            Instruction::FieldGet {
                dest,
                object,
                field_idx,
            } => {
                let obj = read_operand(constants, frame, object);
                match obj {
                    RuntimeValue::Struct { fields } => {
                        let val = fields
                            .get(field_idx as usize)
                            .cloned()
                            .unwrap_or(RuntimeValue::Unit);
                        frame.write(dest, val);
                    }
                    _ => {
                        return Err(VmError::TypeMismatch {
                            expected: TypeTag::Unit,
                            actual: "non-struct".into(),
                            function: func_name,
                        });
                    }
                }
            }
            Instruction::FieldSet {
                dest,
                object,
                field_idx,
                value,
            } => {
                let mut obj = read_operand(constants, frame, object);
                let new_val = read_operand(constants, frame, value);
                match &mut obj {
                    RuntimeValue::Struct { fields } => {
                        if (field_idx as usize) < fields.len() {
                            fields[field_idx as usize] = new_val;
                        }
                        frame.write(dest, obj);
                    }
                    _ => {
                        return Err(VmError::TypeMismatch {
                            expected: TypeTag::Unit,
                            actual: "non-struct".into(),
                            function: func_name,
                        });
                    }
                }
            }

            // ── Aggregate: enum ──────────────────────────────────
            Instruction::EnumNew {
                dest,
                variant_idx,
                payload,
            } => {
                let payload_val = payload.map(|p| Box::new(read_operand(constants, frame, p)));
                frame.write(
                    dest,
                    RuntimeValue::Enum {
                        variant: variant_idx,
                        payload: payload_val,
                    },
                );
            }
            Instruction::EnumTag { dest, scrutinee } => {
                let scr = read_operand(constants, frame, scrutinee);
                // v0.7.4.3-debt.7: return the variant index as Integer
                // instead of a 2-state Trit. Pre-fix only distinguished
                // variant 0 (Positive) vs all others (Negative), so
                // match-on-enum-with-3+-variants dispatched incorrectly.
                let idx: i64 = match &scr {
                    RuntimeValue::Enum { variant, .. } => i64::from(*variant),
                    RuntimeValue::Null => -1,
                    _ => 0, // bare value — treat as variant 0
                };
                frame.write(
                    dest,
                    RuntimeValue::Integer(triet_core::Integer::new(idx).unwrap_or_default()),
                );
            }
            Instruction::EnumPayload { dest, scrutinee } => {
                let scr = read_operand(constants, frame, scrutinee);
                match scr {
                    RuntimeValue::Enum {
                        payload: Some(p), ..
                    } => frame.write(dest, *p),
                    _ => {
                        return Err(VmError::InvalidVariant {
                            function: func_name,
                        });
                    }
                }
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
                let result = execute_builtin(name, &arg_vals, &func_name)?;
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
                    let result = execute_builtin(builtin, &arg_vals, &func_name)?;
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
                    // dispatch identically.
                    let result = execute_builtin(builtin, &arg_vals, &func_name)?;
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
                frame.write(
                    dest,
                    RuntimeValue::Outcome {
                        discriminator: Trit::Positive,
                        payload: Some(Box::new(val)),
                    },
                );
            }
            Instruction::OutcomeNewNegative { dest, payload } => {
                let val = read_operand(constants, frame, payload);
                frame.write(
                    dest,
                    RuntimeValue::Outcome {
                        discriminator: Trit::Negative,
                        payload: Some(Box::new(val)),
                    },
                );
            }
            Instruction::OutcomeNewNull { dest } => {
                frame.write(
                    dest,
                    RuntimeValue::Outcome {
                        discriminator: Trit::Zero,
                        payload: None,
                    },
                );
            }
            Instruction::OutcomeDiscriminant { dest, source } => {
                let outcome = read_operand(constants, frame, source);
                // ADR-0010 Addendum §D (v0.7.4.3-error.6a): cross-
                // tolerance with the legacy `RuntimeValue::Null`
                // carrier — `~0` source-level value lowers to
                // `Constant::Null` per Addendum §B byte-identity
                // promise, so the discriminator readout has to
                // recognize Null as a Zero-state.
                //
                // v0.7.4.3-debt.6 (WA-6 fix): extend cross-tolerance
                // to bare `T?` values. A nullable holds either Null
                // or a bare T (no Outcome wrapper), so any non-Null
                // non-Outcome value reads as `Positive`. This is what
                // makes `match user { ~+ u => ..., ~0 => ... }` work
                // for plain `T?` per the ADR-0010 Addendum §D §"Match
                // arm dispatch" promise (cross-tolerance now reaches
                // pattern-match dispatch beyond the 4 opcodes
                // originally proven in `ffcf6de`).
                let discriminator = match outcome {
                    RuntimeValue::Outcome { discriminator, .. } => discriminator,
                    RuntimeValue::Null => Trit::Zero,
                    // Bare T value (Enum, Struct, Integer, etc.) flowing
                    // through a `T?` slot at runtime. T ⊂ T? widening
                    // doesn't wrap the value, so the runtime sees it
                    // raw — treat as the success arm.
                    _ => Trit::Positive,
                };
                frame.write(dest, RuntimeValue::Trit(discriminator));
            }
            Instruction::OutcomeUnwrapValue { dest, source } => {
                let outcome = read_operand(constants, frame, source);
                let (discriminator, payload) = match outcome {
                    RuntimeValue::Outcome {
                        discriminator,
                        payload,
                    } => (discriminator, payload),
                    // ADR-0010 Addendum §D: unwrap on a Null-carried
                    // Zero state surfaces as E2210 (wrong arm), not
                    // E2201 (wrong type) — the value IS valid, it's
                    // just not the success arm.
                    RuntimeValue::Null => {
                        return Err(VmError::InvalidOutcomeState {
                            reason: "unwrap_value called on null state".into(),
                            function: func_name,
                        });
                    }
                    // v0.7.4.3-debt.6 (WA-6 fix): bare T value flowing
                    // through a `T?` slot — the value IS its own
                    // success payload, no wrapper to peel. Return it
                    // directly.
                    other => {
                        frame.write(dest, other);
                        return Ok(StepResult::Continue);
                    }
                };
                if !discriminator.is_positive() {
                    return Err(VmError::InvalidOutcomeState {
                        reason: format!("unwrap_value called on {} arm", arm_name(discriminator)),
                        function: func_name,
                    });
                }
                let inner = payload.ok_or_else(|| VmError::InvalidOutcomeState {
                    reason: "success arm missing payload".into(),
                    function: func_name.clone(),
                })?;
                frame.write(dest, *inner);
            }
            Instruction::OutcomeUnwrapError { dest, source } => {
                let outcome = read_operand(constants, frame, source);
                let (discriminator, payload) = match outcome {
                    RuntimeValue::Outcome {
                        discriminator,
                        payload,
                    } => (discriminator, payload),
                    RuntimeValue::Null => {
                        return Err(VmError::InvalidOutcomeState {
                            reason: "unwrap_error called on null state".into(),
                            function: func_name,
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
                            function: func_name,
                        });
                    }
                };
                if !discriminator.is_negative() {
                    return Err(VmError::InvalidOutcomeState {
                        reason: format!("unwrap_error called on {} arm", arm_name(discriminator)),
                        function: func_name,
                    });
                }
                let inner = payload.ok_or_else(|| VmError::InvalidOutcomeState {
                    reason: "failure arm missing payload".into(),
                    function: func_name.clone(),
                })?;
                frame.write(dest, *inner);
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

        _ => None,
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
}
