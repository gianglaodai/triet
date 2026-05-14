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
            Self::AssertionFailed {
                message,
                function,
            } => {
                write!(
                    f,
                    "E2205: assertion failed in `{function}`{}",
                    message
                        .as_ref()
                        .map_or(String::new(), |m| format!(": {m}"))
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
            func_name: func.name.clone().unwrap_or_else(|| format!("@f{}", func.id.0)),
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
            let map: HashMap<BlockId, BasicBlock> = func
                .blocks
                .iter()
                .map(|b| (b.id, b.clone()))
                .collect();
            block_maps.insert(func.id, map);
        }

        // Build a path → FuncId index for cross-module call dispatch.
        let mut path_index: HashMap<String, FuncId> = HashMap::new();
        for module in &program.modules {
            let module_path = module.path.to_string();
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
                let result = if runtime_eq(&l, &r) {
                    Trilean::True
                } else {
                    Trilean::False
                };
                frame.write(dest, RuntimeValue::Trilean(result));
            }
            Instruction::Ne { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs);
                let r = read_operand(constants, frame, rhs);
                let result = if runtime_eq(&l, &r) {
                    Trilean::False
                } else {
                    Trilean::True
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
                let result =
                    if cmp == std::cmp::Ordering::Less || cmp == std::cmp::Ordering::Equal {
                        Trilean::True
                    } else {
                        Trilean::False
                    };
                frame.write(dest, RuntimeValue::Trilean(result));
            }
            Instruction::Gt { dest, lhs, rhs } => {
                let l = read_operand(constants, frame, lhs);
                let r = read_operand(constants, frame, rhs);
                let result =
                    if runtime_cmp(&l, &r) == std::cmp::Ordering::Greater {
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
                let v = read_operand(constants, frame, nullable);
                match v {
                    RuntimeValue::Enum {
                        variant: 0,
                        payload: Some(p),
                    } => frame.write(dest, *p),
                    RuntimeValue::Null => {
                        return Err(VmError::NullUnwrap {
                            function: func_name,
                        });
                    }
                    _ => frame.write(dest, v), // pass through non-nullable
                }
            }
            Instruction::NullCheck { dest, nullable } => {
                let v = read_operand(constants, frame, nullable);
                let trit = match &v {
                    RuntimeValue::Null => Trit::Zero,
                    RuntimeValue::Enum { payload: None, .. } => Trit::Zero,
                    _ => Trit::Positive, // non-null
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
                let tag = match &scr {
                    RuntimeValue::Enum { variant, .. } => match variant {
                        0 => Trit::Positive,
                        _ => Trit::Negative,
                    },
                    RuntimeValue::Null => Trit::Zero,
                    _ => Trit::Positive,
                };
                frame.write(dest, RuntimeValue::Trit(tag));
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
            Instruction::CallLocal {
                dest,
                callee,
                args,
            } => {
                let arg_vals: Vec<RuntimeValue> =
                    args.iter().map(|a| read_operand(constants, frame, *a)).collect();

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
            Instruction::CallBuiltin {
                dest,
                name,
                args,
            } => {
                let arg_vals: Vec<RuntimeValue> =
                    args.iter().map(|a| read_operand(constants, frame, *a)).collect();
                let result = execute_builtin(name, &arg_vals, &func_name)?;
                if let Some(d) = dest {
                    frame.write(d, result);
                }
            }
            Instruction::CallCrossModule {
                dest,
                path,
                args,
            } => {
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

            // ── Closure ──────────────────────────────────────────
            Instruction::ClosureNew {
                dest,
                lambda,
                captures,
            } => {
                let capture_vals: Vec<RuntimeValue> = captures
                    .iter()
                    .map(|&v| frame.read(v))
                    .collect();
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
                let arg_vals: Vec<RuntimeValue> =
                    args.iter().map(|a| read_operand(constants, frame, *a)).collect();
                match clos {
                    RuntimeValue::Closure {
                        func_id,
                        captures,
                    } => {
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
fn read_operand(constants: &crate::constant::ConstantPool, frame: &Frame, op: Operand) -> RuntimeValue {
    match op {
        Operand::Value(id) => frame.read(id),
        Operand::Const(cid) => RuntimeValue::from_constant(
            constants.get(cid).unwrap_or(&Constant::Unit),
        ),
    }
}

// ── Step result ────────────────────────────────────────────────────

enum StepResult {
    /// Continue to the next instruction.
    Continue,
    /// Return from the current function.
    Return(RuntimeValue),
}

// ── Arithmetic helpers ─────────────────────────────────────────────

fn arithmetic_add(l: &RuntimeValue, r: &RuntimeValue, func: &str) -> Result<RuntimeValue, VmError> {
    match (l, r) {
        (RuntimeValue::Integer(a), RuntimeValue::Integer(b)) => {
            Ok(RuntimeValue::Integer(a.try_add(*b).ok_or_else(|| VmError::Overflow { function: func.into() })?))
        }
        (RuntimeValue::Long(a), RuntimeValue::Long(b)) => {
            Ok(RuntimeValue::Long(*a + *b))
        }
        (RuntimeValue::Tryte(a), RuntimeValue::Tryte(b)) => {
            Ok(RuntimeValue::Tryte(a.try_add(*b).ok_or_else(|| VmError::Overflow { function: func.into() })?))
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
        (RuntimeValue::Integer(a), RuntimeValue::Integer(b)) => {
            Ok(RuntimeValue::Integer(a.try_subtract(*b).ok_or_else(|| VmError::Overflow { function: func.into() })?))
        }
        (RuntimeValue::Long(a), RuntimeValue::Long(b)) => {
            Ok(RuntimeValue::Long(*a - *b))
        }
        (RuntimeValue::Tryte(a), RuntimeValue::Tryte(b)) => {
            Ok(RuntimeValue::Tryte(a.try_subtract(*b).ok_or_else(|| VmError::Overflow { function: func.into() })?))
        }
        _ => Ok(RuntimeValue::Integer(Integer::new(0).unwrap())),
    }
}

fn arithmetic_mul(l: &RuntimeValue, r: &RuntimeValue, func: &str) -> Result<RuntimeValue, VmError> {
    match (l, r) {
        (RuntimeValue::Integer(a), RuntimeValue::Integer(b)) => {
            Ok(RuntimeValue::Integer(a.try_multiply(*b).ok_or_else(|| VmError::Overflow { function: func.into() })?))
        }
        (RuntimeValue::Long(a), RuntimeValue::Long(b)) => {
            Ok(RuntimeValue::Long(*a * *b))
        }
        (RuntimeValue::Tryte(a), RuntimeValue::Tryte(b)) => {
            Ok(RuntimeValue::Tryte(a.try_multiply(*b).ok_or_else(|| VmError::Overflow { function: func.into() })?))
        }
        _ => Ok(RuntimeValue::Integer(Integer::new(0).unwrap())),
    }
}

fn arithmetic_div(l: &RuntimeValue, r: &RuntimeValue, func: &str) -> Result<RuntimeValue, VmError> {
    match (l, r) {
        (RuntimeValue::Integer(a), RuntimeValue::Integer(b)) => {
            if *b == Integer::new(0).unwrap() {
                return Err(VmError::DivisionByZero { function: func.into() });
            }
            Ok(RuntimeValue::Integer(a.try_divide(*b).map_err(|_| VmError::DivisionByZero { function: func.into() })?))
        }
        (RuntimeValue::Long(a), RuntimeValue::Long(b)) => {
            if *b == Long::from_i128(0) {
                return Err(VmError::DivisionByZero { function: func.into() });
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
                return Err(VmError::DivisionByZero { function: func.into() });
            }
            Ok(RuntimeValue::Integer(a.try_modulo(*b).map_err(|_| VmError::DivisionByZero { function: func.into() })?))
        }
        (RuntimeValue::Long(a), RuntimeValue::Long(b)) => {
            if *b == Long::from_i128(0) {
                return Err(VmError::DivisionByZero { function: func.into() });
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
                result = result
                    .try_multiply(*a)
                    .ok_or_else(|| VmError::Overflow { function: func.into() })?;
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
        RuntimeValue::Trit(t) => Ok(RuntimeValue::Integer(Integer::new(i64::from(t.to_i8())).unwrap())),
        RuntimeValue::Tryte(t) => Ok(RuntimeValue::Integer(Integer::new(i64::from(t.to_i16())).unwrap())),
        RuntimeValue::Integer(_) => Ok(v.clone()),
        RuntimeValue::Long(l) => {
            Ok(RuntimeValue::Integer(l.to_integer()))
        }
        _ => Err(VmError::TypeMismatch {
            expected: TypeTag::Integer,
            actual: format!("{}", v.type_tag()),
            function: func.into(),
        }),
    }
}

fn convert_to_tryte(v: &RuntimeValue, func: &str) -> Result<RuntimeValue, VmError> {
    match v {
        RuntimeValue::Trit(t) => Ok(RuntimeValue::Tryte(Tryte::new(i16::from(t.to_i8())).unwrap())),
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
        _ => false,
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
        "std.io.println" => Some(BuiltinName::Println),
        "std.io.print" => Some(BuiltinName::Print),
        "std.assert.assert" => Some(BuiltinName::Assert),
        "std.assert.assert_eq" => Some(BuiltinName::AssertEq),
        _ => None,
    }
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
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constant::ConstantPool;
    use crate::instr::PhiIncoming;
    use crate::module::{BasicBlock, Function, IrModule};
    use crate::ConstId;
    use triet_modules::{AbsolutePath, ModulePath};

    fn make_int(n: i64) -> RuntimeValue {
        RuntimeValue::Integer(Integer::new(n).unwrap())
    }

    fn make_simple_program(func: Function) -> IrProgram {
        IrProgram {
            modules: vec![IrModule {
                path: AbsolutePath::new(ModulePath::crate_root(), "test".into()),
                functions: vec![func],
            }],
            constants: ConstantPool::new(),
        }
    }

    // ── Arithmetic VM tests ──────────────────────────────────────

    #[test]
    fn vm_add_integers() {
        let pool = ConstantPool::new();
        let func = Function {
            id: FuncId(0),
            name: Some("add".into()),
            params: vec![("a".into(), TypeTag::Integer), ("b".into(), TypeTag::Integer)],
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
            params: vec![("a".into(), TypeTag::Integer), ("b".into(), TypeTag::Integer)],
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
            params: vec![("a".into(), TypeTag::Integer), ("b".into(), TypeTag::Integer)],
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
        assert!(matches!(result.unwrap_err(), VmError::DivisionByZero { .. }));
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
            params: vec![("a".into(), TypeTag::Integer), ("b".into(), TypeTag::Integer)],
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
        let result = vm
            .execute(FuncId(0), vec![make_int(5)])
            .unwrap();
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
                        Instruction::Br {
                            target: BlockId(3),
                        },
                    ],
                },
                BasicBlock {
                    id: BlockId(2),
                    name: Some("pos".into()),
                    instructions: vec![Instruction::Br {
                        target: BlockId(3),
                    }],
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
        let result = vm
            .execute(FuncId(0), vec![make_int(-5)])
            .unwrap();
        assert_eq!(result.to_string(), make_int(5).to_string());

        // abs(7) = 7
        let mut vm2 = Vm::new(prog);
        let result2 = vm2
            .execute(FuncId(0), vec![make_int(7)])
            .unwrap();
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
        let a = make_int(Integer::MAX.to_i64());  // +3_812_798_742_493
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
            [Trilean::False, Trilean::False, Trilean::False],   // F ∧ ...
            [Trilean::False, Trilean::Unknown, Trilean::Unknown], // U ∧ ...
            [Trilean::False, Trilean::Unknown, Trilean::True],    // T ∧ ...
        ];
        // Ł3 OR (=max): U∨F=U, U∨U=U, U∨T=T, F∨T=T, T∨T=T
        let expected_or = [
            [Trilean::False, Trilean::Unknown, Trilean::True], // F ∨ ...
            [Trilean::Unknown, Trilean::Unknown, Trilean::True], // U ∨ ...
            [Trilean::True, Trilean::True, Trilean::True],      // T ∨ ...
        ];
        // Ł3 IMPLIES (=>): min(1, 1-a+b). U⇒U=T (key!), F⇒U=T
        let expected_implies = [
            [Trilean::True, Trilean::True, Trilean::True],         // F ⇒ ...
            [Trilean::Unknown, Trilean::True, Trilean::True],      // U ⇒ ...  (U⇒U = T!)
            [Trilean::False, Trilean::Unknown, Trilean::True],     // T ⇒ ...
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
                    Instruction::Const { dest: ValueId(0), constant: cu },
                    Instruction::LukImplies {
                        dest: ValueId(1),
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Value(ValueId(0)),
                    },
                    Instruction::Ret { value: Some(Operand::Value(ValueId(1))) },
                ],
            }],
        };
        let mut prog1 = make_simple_program(l3_func);
        prog1.constants = pool.clone();
        let mut vm1 = Vm::new(prog1);
        let r1 = vm1.execute(FuncId(0), vec![]).unwrap();
        assert_eq!(r1.to_string(), Trilean::True.to_string(), "Ł3: U⇒U must be True");

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
                    Instruction::Const { dest: ValueId(0), constant: cu },
                    Instruction::KleeneImplies {
                        dest: ValueId(1),
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Value(ValueId(0)),
                    },
                    Instruction::Ret { value: Some(Operand::Value(ValueId(1))) },
                ],
            }],
        };
        let mut prog2 = make_simple_program(k3_func);
        prog2.constants = pool;
        let mut vm2 = Vm::new(prog2);
        let r2 = vm2.execute(FuncId(0), vec![]).unwrap();
        assert_eq!(r2.to_string(), Trilean::Unknown.to_string(), "K3: U~>U must be Unknown");
    }
}
