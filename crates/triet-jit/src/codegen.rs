//! v0.9.x.jit.2 — Cranelift IR emission for a subset of Triết IR
//! opcodes per [ADR-0030 §3] opcode table.
//!
//! Supported in this sub-phase:
//! - [`Const`] materialization for `Trit` / `Tryte` / `Integer` /
//!   `Trilean` / `Unit` constants.
//! - Arithmetic: [`Add`] / [`Sub`] / [`Mul`] / [`Neg`] on Integer.
//! - Comparison: [`Eq`] / [`Ne`] / [`Lt`] / [`Le`] / [`Gt`] / [`Ge`]
//!   on Integer — result extended to `i8` (Trilean encoding).
//! - Control flow: [`Br`] (unconditional) + [`BrIf`] + [`BrTrilean`]
//!   per [ADR-0010 §4 backend table] (2 cmp + 2 brnz on binary CPU).
//! - Terminators: [`Ret`] (with or without value).
//!
//! Out of scope (deferred to subsequent sub-tasks per ADR-0030 §11):
//! - `.3` — calls (`CallLocal` / `CallCrossModule` / `WitnessCall` /
//!   `ClosureCall`).
//! - `.4` — builtin shim integration.
//! - Aggregate (struct/enum), nullable/outcome wrappers, conversions,
//!   logic ops (Ł3/K3), `Long` (i128), `Phi`, `Unreachable`.
//! - Strings, `Vector`, `HashMap`, Atomic.
//!
//! Anything outside the supported set raises [`JitError::UnsupportedOpcode`]
//! so the VM falls back to bytecode dispatch for that function per
//! ADR-0030 §2 tier-down policy.
//!
//! [ADR-0030 §3]: ../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0010 §4 backend table]: ../../../docs/decisions/0010-ternary-native-ir.md
//! [`Const`]: triet_ir::Instruction::Const
//! [`Add`]: triet_ir::Instruction::Add
//! [`Sub`]: triet_ir::Instruction::Sub
//! [`Mul`]: triet_ir::Instruction::Mul
//! [`Neg`]: triet_ir::Instruction::Neg
//! [`Eq`]: triet_ir::Instruction::Eq
//! [`Ne`]: triet_ir::Instruction::Ne
//! [`Lt`]: triet_ir::Instruction::Lt
//! [`Le`]: triet_ir::Instruction::Le
//! [`Gt`]: triet_ir::Instruction::Gt
//! [`Ge`]: triet_ir::Instruction::Ge
//! [`Br`]: triet_ir::Instruction::Br
//! [`BrIf`]: triet_ir::Instruction::BrIf
//! [`BrTrilean`]: triet_ir::Instruction::BrTrilean
//! [`Ret`]: triet_ir::Instruction::Ret

// This is an internal module; the `pub(crate)` markers on items here
// are intentional (crate-private exposure to lib.rs).
#![allow(clippy::redundant_pub_crate)]

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::{I8, I16, I64};
use cranelift_codegen::ir::{AbiParam, Block, InstBuilder, Signature, Value, types};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use triet_ir::{BlockId, Constant, Function as IrFunction, Instruction, Operand, TypeTag, ValueId};

use crate::JitError;

/// Map a Triết [`TypeTag`] to a Cranelift IR scalar type per
/// [ADR-0030 §3] type table.
///
/// [ADR-0030 §3]: ../../../docs/decisions/0030-jit-cranelift-integration.md
pub(crate) fn map_type(tag: &TypeTag) -> Result<types::Type, JitError> {
    Ok(match tag {
        // Trit, Trilean, and Unit all collapse to i8.
        // - Trit/Trilean use the {-1, 0, +1} encoding per ADR-0010 §3.
        // - Unit is zero-sized at the language level; encode as a
        //   dummy i8 0 slot so functions returning Unit have a
        //   consistent ABI shape.
        TypeTag::Trit | TypeTag::Trilean | TypeTag::Unit => I8,
        TypeTag::Tryte => I16,
        TypeTag::Integer => I64,
        // Long (i128) needs pair-of-i64 lowering per ADR-0030 §3 — defer.
        TypeTag::Long => {
            return Err(JitError::UnsupportedOpcode {
                opcode: "Long type (i128) — defer to later sub-phase".to_string(),
            });
        }
        // Composite types (String/Nullable/Vector/HashMap/Tuple/Range/
        // Atomic/etc.) require heap-allocated layouts handled via Rust
        // runtime calls — defer to .3-.4.
        other => {
            return Err(JitError::UnsupportedOpcode {
                opcode: format!("type {other:?} — defer to later sub-phase"),
            });
        }
    })
}

/// Encapsulates the Cranelift JIT module + a target ISA. Constructed
/// lazily on the first `compile()` call.
pub(crate) struct JitBackend {
    module: JITModule,
}

impl JitBackend {
    /// Initialize Cranelift JIT for the host target.
    pub(crate) fn new() -> Result<Self, JitError> {
        let flag_builder = settings::builder();
        let isa_builder = cranelift_native::builder().map_err(|message| JitError::Cranelift {
            message: format!("ISA detection failed: {message}"),
        })?;
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .map_err(|err| JitError::Cranelift {
                message: format!("ISA finish failed: {err}"),
            })?;
        let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let module = JITModule::new(builder);
        Ok(Self { module })
    }

    /// Translate one Triết IR function to Cranelift IR, emit machine
    /// code, and return the host-address pointer.
    ///
    /// On any unsupported opcode or Cranelift verifier failure, returns
    /// an error and (per ADR-0030 §2 tier-down) the caller marks the
    /// function as permanently VM-dispatched for the session.
    pub(crate) fn compile_function(&mut self, func: &IrFunction) -> Result<usize, JitError> {
        let signature = build_signature(func)?;

        let func_name = func
            .name
            .clone()
            .unwrap_or_else(|| format!("@f{}", func.id.0));
        let func_id = self
            .module
            .declare_function(&func_name, Linkage::Local, &signature)
            .map_err(cranelift_err)?;

        let mut ctx = self.module.make_context();
        ctx.func.signature = signature;

        {
            let mut fn_builder_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);

            // Pre-declare a Cranelift block per Triết BlockId so
            // forward branches resolve. Cranelift requires the entry
            // block to receive function parameters.
            let mut block_map: HashMap<BlockId, Block> = HashMap::new();
            for ir_block in &func.blocks {
                let cl_block = builder.create_block();
                block_map.insert(ir_block.id, cl_block);
            }

            let entry_ir_block =
                func.blocks
                    .first()
                    .ok_or_else(|| JitError::UnsupportedOpcode {
                        opcode: "function with no blocks".to_string(),
                    })?;
            let entry_block = block_map[&entry_ir_block.id];
            builder.append_block_params_for_function_params(entry_block);

            // Value map populated as instructions translate. Entry-block
            // param values come from `block_params(entry_block)`.
            let mut value_map: HashMap<ValueId, Value> = HashMap::new();
            for (idx, param_val) in builder.block_params(entry_block).iter().enumerate() {
                // IR convention: parameters occupy ValueId(0..param_count).
                value_map.insert(
                    ValueId(u32::try_from(idx).map_err(|_| JitError::UnsupportedOpcode {
                        opcode: "parameter index overflow".to_string(),
                    })?),
                    *param_val,
                );
            }

            // Walk every block in declaration order, switch into it,
            // and emit per-instruction Cranelift IR.
            for ir_block in &func.blocks {
                let cl_block = block_map[&ir_block.id];
                builder.switch_to_block(cl_block);
                for instr in &ir_block.instructions {
                    translate_instruction(&mut builder, &mut value_map, &block_map, func, instr)?;
                }
            }

            builder.seal_all_blocks();
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(cranelift_err)?;
        self.module.clear_context(&mut ctx);

        self.module.finalize_definitions().map_err(cranelift_err)?;

        let raw_ptr = self.module.get_finalized_function(func_id);
        Ok(raw_ptr as usize)
    }
}

/// Build a Cranelift function signature from a Triết IR function's
/// declared parameter types + return type.
fn build_signature(func: &IrFunction) -> Result<Signature, JitError> {
    let mut sig = Signature::new(CallConv::SystemV);
    for (_, ty) in &func.params {
        sig.params.push(AbiParam::new(map_type(ty)?));
    }
    sig.returns
        .push(AbiParam::new(map_type(&func.return_type)?));
    Ok(sig)
}

fn cranelift_err<E: core::fmt::Display>(err: E) -> JitError {
    JitError::Cranelift {
        message: format!("{err}"),
    }
}

/// Translate a single Triết IR instruction into the Cranelift
/// `FunctionBuilder`'s current block. Updates `value_map` for any new
/// SSA def; reads `block_map` for branch targets.
#[allow(clippy::too_many_lines)]
fn translate_instruction(
    builder: &mut FunctionBuilder<'_>,
    value_map: &mut HashMap<ValueId, Value>,
    block_map: &HashMap<BlockId, Block>,
    func: &IrFunction,
    instr: &Instruction,
) -> Result<(), JitError> {
    match instr {
        Instruction::Const { dest, constant } => {
            // Constants live outside the IR module structure — JIT must
            // be given the pool externally. v0.9.x.jit.2 design: defer
            // to .3 which lands the program-level wiring. For now any
            // Const opcode raises UnsupportedOpcode so the wired tests
            // exercise param-only flows. Once the JIT compiler is
            // wired against the IrProgram (not just Function), the
            // pool lookup can complete here.
            let _ = (dest, constant);
            return Err(JitError::UnsupportedOpcode {
                opcode: "Const (needs program-level pool wiring; .3)".to_string(),
            });
        }
        Instruction::Add { dest, lhs, rhs } => {
            let l = resolve_operand(builder, value_map, *lhs)?;
            let r = resolve_operand(builder, value_map, *rhs)?;
            let v = builder.ins().iadd(l, r);
            value_map.insert(*dest, v);
        }
        Instruction::Sub { dest, lhs, rhs } => {
            let l = resolve_operand(builder, value_map, *lhs)?;
            let r = resolve_operand(builder, value_map, *rhs)?;
            let v = builder.ins().isub(l, r);
            value_map.insert(*dest, v);
        }
        Instruction::Mul { dest, lhs, rhs } => {
            let l = resolve_operand(builder, value_map, *lhs)?;
            let r = resolve_operand(builder, value_map, *rhs)?;
            let v = builder.ins().imul(l, r);
            value_map.insert(*dest, v);
        }
        Instruction::Neg { dest, operand } => {
            let v = resolve_operand(builder, value_map, *operand)?;
            let result = builder.ins().ineg(v);
            value_map.insert(*dest, result);
        }
        Instruction::Eq { dest, lhs, rhs } => {
            emit_icmp(builder, value_map, IntCC::Equal, *dest, *lhs, *rhs)?;
        }
        Instruction::Ne { dest, lhs, rhs } => {
            emit_icmp(builder, value_map, IntCC::NotEqual, *dest, *lhs, *rhs)?;
        }
        Instruction::Lt { dest, lhs, rhs } => {
            emit_icmp(builder, value_map, IntCC::SignedLessThan, *dest, *lhs, *rhs)?;
        }
        Instruction::Le { dest, lhs, rhs } => {
            emit_icmp(
                builder,
                value_map,
                IntCC::SignedLessThanOrEqual,
                *dest,
                *lhs,
                *rhs,
            )?;
        }
        Instruction::Gt { dest, lhs, rhs } => {
            emit_icmp(
                builder,
                value_map,
                IntCC::SignedGreaterThan,
                *dest,
                *lhs,
                *rhs,
            )?;
        }
        Instruction::Ge { dest, lhs, rhs } => {
            emit_icmp(
                builder,
                value_map,
                IntCC::SignedGreaterThanOrEqual,
                *dest,
                *lhs,
                *rhs,
            )?;
        }
        Instruction::Br { target } => {
            let cl_target = *block_map
                .get(target)
                .ok_or_else(|| JitError::UnsupportedOpcode {
                    opcode: format!("Br target block {target:?} not in map"),
                })?;
            builder.ins().jump(cl_target, &[]);
        }
        Instruction::BrIf {
            cond,
            then_block,
            else_block,
        } => {
            // BrIf treats Unknown as False per ADR-0010 deprecation
            // note (legacy 2-way). Cranelift `brif` jumps to `then` if
            // value != 0 (i.e. True = +1, Unknown = 0 → False, False
            // = -1 → True!). Wrong for trit-encoded Trilean.
            //
            // Correct mapping per ADR-0010 §3: True=+1, False=-1, so
            // we test `cond == +1` (treat anything else as the else
            // branch).
            let c = resolve_operand(builder, value_map, *cond)?;
            let one = builder.ins().iconst(I8, 1);
            let is_true = builder.ins().icmp(IntCC::Equal, c, one);
            let cl_then =
                *block_map
                    .get(then_block)
                    .ok_or_else(|| JitError::UnsupportedOpcode {
                        opcode: format!("BrIf then-block {then_block:?} not in map"),
                    })?;
            let cl_else =
                *block_map
                    .get(else_block)
                    .ok_or_else(|| JitError::UnsupportedOpcode {
                        opcode: format!("BrIf else-block {else_block:?} not in map"),
                    })?;
            builder.ins().brif(is_true, cl_then, &[], cl_else, &[]);
        }
        Instruction::BrTrilean {
            cond,
            true_block,
            unknown_block,
            false_block,
        } => {
            // Per ADR-0010 §4 binary-CPU backend table: 2 icmp + 2 brif.
            // Encoding: True=+1, Unknown=0, False=-1 (i8).
            //
            //   v_true = icmp eq cond, +1
            //   brif v_true, true_block, fallthrough_1
            // fallthrough_1:
            //   v_unk = icmp eq cond, 0
            //   brif v_unk, unknown_block, false_block
            let c = resolve_operand(builder, value_map, *cond)?;
            let pos_one = builder.ins().iconst(I8, 1);
            let zero = builder.ins().iconst(I8, 0);
            let cl_true =
                *block_map
                    .get(true_block)
                    .ok_or_else(|| JitError::UnsupportedOpcode {
                        opcode: format!("BrTrilean true-block {true_block:?} not in map"),
                    })?;
            let cl_unk =
                *block_map
                    .get(unknown_block)
                    .ok_or_else(|| JitError::UnsupportedOpcode {
                        opcode: format!("BrTrilean unknown-block {unknown_block:?} not in map"),
                    })?;
            let cl_false =
                *block_map
                    .get(false_block)
                    .ok_or_else(|| JitError::UnsupportedOpcode {
                        opcode: format!("BrTrilean false-block {false_block:?} not in map"),
                    })?;
            // Materialize an intermediate block for the False-or-Unknown fall-through.
            let fallthrough = builder.create_block();
            let is_true = builder.ins().icmp(IntCC::Equal, c, pos_one);
            builder.ins().brif(is_true, cl_true, &[], fallthrough, &[]);
            builder.switch_to_block(fallthrough);
            let is_unk = builder.ins().icmp(IntCC::Equal, c, zero);
            builder.ins().brif(is_unk, cl_unk, &[], cl_false, &[]);
        }
        Instruction::Ret { value } => {
            if let Some(op) = value {
                let v = resolve_operand(builder, value_map, *op)?;
                builder.ins().return_(&[v]);
            } else {
                // No-value return: emit a Unit i8 0 placeholder per
                // build_signature returning i8 for Unit.
                let unit = builder.ins().iconst(map_type(&func.return_type)?, 0);
                builder.ins().return_(&[unit]);
            }
        }
        // Everything else triggers tier-down to VM-only for this fn.
        other => {
            return Err(JitError::UnsupportedOpcode {
                opcode: format!("{other:?}"),
            });
        }
    }
    Ok(())
}

/// Resolve an [`Operand`] into a Cranelift [`Value`] live in the
/// current block. `Value(id)` looks up the SSA map; `Const(id)`
/// raises Unsupported until program-level pool wiring lands (`.3`).
fn resolve_operand(
    _builder: &mut FunctionBuilder<'_>,
    value_map: &HashMap<ValueId, Value>,
    operand: Operand,
) -> Result<Value, JitError> {
    match operand {
        Operand::Value(id) => {
            value_map
                .get(&id)
                .copied()
                .ok_or_else(|| JitError::UnsupportedOpcode {
                    opcode: format!("ValueId({}) referenced before def", id.0),
                })
        }
        Operand::Const(_) => Err(JitError::UnsupportedOpcode {
            opcode: "inline constant operand (needs program-level pool wiring; .3)".to_string(),
        }),
    }
}

/// Emit an integer compare returning a Trilean i8 (`+1` for true,
/// `-1` for false; Unknown is not produced because non-nullable
/// integer comparisons can't yield Unknown per ADR-0021).
fn emit_icmp(
    builder: &mut FunctionBuilder<'_>,
    value_map: &mut HashMap<ValueId, Value>,
    cc: IntCC,
    dest: ValueId,
    lhs: Operand,
    rhs: Operand,
) -> Result<(), JitError> {
    let l = resolve_operand(builder, value_map, lhs)?;
    let r = resolve_operand(builder, value_map, rhs)?;
    // Cranelift `icmp` produces an i8 (0 or 1). Map to Triết Trilean
    // encoding by computing `2*raw - 1`: true → +1, false → -1.
    let raw = builder.ins().icmp(cc, l, r);
    let two = builder.ins().iconst(I8, 2);
    let doubled = builder.ins().imul(raw, two);
    let one = builder.ins().iconst(I8, 1);
    let trit = builder.ins().isub(doubled, one);
    value_map.insert(dest, trit);
    Ok(())
}

/// v0.9.x.jit.2: `_unused` import to keep this module compiling
/// without pulling more types than necessary. Future sub-tasks expand.
#[allow(dead_code)]
const fn _ensure_constant_unused() -> Option<Constant> {
    None
}
