//! v0.9.x.jit.2 + .3 — Cranelift IR emission for a subset of Triết IR
//! opcodes per [ADR-0030 §3] opcode table.
//!
//! Supported through `.3`:
//! - [`Const`] materialization (statement + inline `Operand::Const`)
//!   for `Trit` / `Tryte` / `Integer` / `Trilean` / `Unit` constants.
//! - Arithmetic: [`Add`] / [`Sub`] / [`Mul`] / [`Neg`] on Integer.
//! - Comparison: [`Eq`] / [`Ne`] / [`Lt`] / [`Le`] / [`Gt`] / [`Ge`]
//!   on Integer — result extended to `i8` (Trilean encoding).
//! - Control flow: [`Br`] (unconditional) + [`BrIf`] + [`BrTrilean`]
//!   per [ADR-0010 §4 backend table] (2 cmp + 2 brnz on binary CPU).
//! - Terminators: [`Ret`] (with or without value).
//! - **Calls** (`.3`): [`CallLocal`] (intra-module direct),
//!   [`CallCrossModule`] (path lookup → same `JITModule` `FuncId`),
//!   [`WitnessCall`] (witness table informational per ADR-0012 §2;
//!   dispatch identical to `CallCrossModule` at v0.4 semantics).
//!
//! Out of scope (deferred to subsequent sub-tasks per ADR-0030 §11):
//! - `.4` — builtin shim integration (Vec/HashMap/IO + Atomic).
//! - `ClosureNew` / `ClosureCall` — needs closure runtime layout.
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
//! [`CallLocal`]: triet_ir::Instruction::CallLocal
//! [`CallCrossModule`]: triet_ir::Instruction::CallCrossModule
//! [`WitnessCall`]: triet_ir::Instruction::WitnessCall

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
use cranelift_module::{FuncId as ClFuncId, Linkage, Module};
use triet_ir::{
    BlockId, ConstId, Constant, ConstantPool, FuncId as TriFuncId, Function as IrFunction,
    Instruction, IrProgram, Operand, TypeTag, ValueId,
};
use triet_logic::Trilean;
use triet_modules::AbsolutePath;

use crate::{JitError, NativeCodePtr};

/// Per-program lookup context built during the pre-pass of
/// [`JitBackend::compile_program`]. Threaded into per-instruction
/// translation so calls and inline constant operands resolve in O(1).
struct ProgramContext<'a> {
    /// Triết `FuncId` → Cranelift `FuncId` for `CallLocal` /
    /// cross-module dispatch (all functions live in the same
    /// `JITModule`).
    func_id_map: HashMap<TriFuncId, ClFuncId>,
    /// `AbsolutePath` → Triết `FuncId` for `CallCrossModule` /
    /// `WitnessCall` path resolution (paths are unique per `IrProgram`).
    path_to_funcid: HashMap<AbsolutePath, TriFuncId>,
    /// Shared constant pool for inline `Operand::Const(id)` materialization.
    constants: &'a ConstantPool,
}

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
    /// **Single-function path:** no cross-call resolution + no
    /// constant pool available. Calls + inline `Operand::Const`
    /// raise [`JitError::UnsupportedOpcode`]. Used by tests that
    /// don't need program-level wiring; production callers go
    /// through [`Self::compile_program`].
    pub(crate) fn compile_function(&mut self, func: &IrFunction) -> Result<usize, JitError> {
        let empty_pool = ConstantPool::new();
        let ctx = ProgramContext {
            func_id_map: HashMap::new(),
            path_to_funcid: HashMap::new(),
            constants: &empty_pool,
        };
        let signature = build_signature(func)?;
        let func_name = func
            .name
            .clone()
            .unwrap_or_else(|| format!("@f{}", func.id.0));
        let func_id = self
            .module
            .declare_function(&func_name, Linkage::Local, &signature)
            .map_err(cranelift_err)?;
        let mut cl_ctx = self.module.make_context();
        cl_ctx.func.signature = signature;
        self.emit_function_body(func, &ctx, &mut cl_ctx)?;
        self.module
            .define_function(func_id, &mut cl_ctx)
            .map_err(cranelift_err)?;
        self.module.clear_context(&mut cl_ctx);
        self.module.finalize_definitions().map_err(cranelift_err)?;
        let raw_ptr = self.module.get_finalized_function(func_id);
        Ok(raw_ptr as usize)
    }

    /// Compile every function in `program` in a two-pass shape:
    ///
    /// 1. **Pre-pass:** for each Triết function, build its Cranelift
    ///    signature + `declare_function` so cross-references resolve.
    ///    Populates `func_id_map` (Triết → Cranelift) + `path_to_funcid`
    ///    (`AbsolutePath` → Triết) used by call sites.
    /// 2. **Body pass:** for each function, emit its Cranelift IR body
    ///    via [`Self::emit_function_body`] with full program context
    ///    (call resolution + constant pool access).
    /// 3. **Finalize:** single `finalize_definitions` flips all
    ///    machine code from RW to RX, then collect raw pointers into
    ///    `out_cache` keyed by Triết `FuncId`.
    ///
    /// On any per-function error, the function is dropped from the
    /// cache (tier-down per ADR-0030 §2); other functions in the same
    /// program continue compiling. Returns Err only on
    /// pre-pass / finalize failures that prevent the whole program
    /// from JIT-ing.
    pub(crate) fn compile_program(
        &mut self,
        program: &IrProgram,
        out_cache: &mut HashMap<TriFuncId, NativeCodePtr>,
    ) -> Result<(), JitError> {
        // Pre-pass: declare every function so calls can resolve.
        let mut func_id_map: HashMap<TriFuncId, ClFuncId> = HashMap::new();
        let mut path_to_funcid: HashMap<AbsolutePath, TriFuncId> = HashMap::new();
        for ir_module in &program.modules {
            for func in &ir_module.functions {
                let signature = build_signature(func)?;
                let func_name = func
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("@f{}", func.id.0));
                // Mangle name with FuncId so two modules can share a
                // simple name (`main`, `helper`) without collision.
                let mangled = format!("{}__f{}", func_name, func.id.0);
                let cl_id = self
                    .module
                    .declare_function(&mangled, Linkage::Local, &signature)
                    .map_err(cranelift_err)?;
                func_id_map.insert(func.id, cl_id);
                if let Some(name) = &func.name {
                    // `IrModule.path` is an AbsolutePath with empty item
                    // name per lowerer convention (`module.rs` line 147).
                    // Extract its `ModulePath` and re-wrap with `name`.
                    let path =
                        AbsolutePath::new(ir_module.path.module_path().clone(), name.clone());
                    path_to_funcid.insert(path, func.id);
                }
            }
        }

        let ctx = ProgramContext {
            func_id_map: func_id_map.clone(),
            path_to_funcid,
            constants: &program.constants,
        };

        // Body pass: per-function codegen + define. On per-function
        // error, skip (tier-down) without aborting the whole program.
        let mut compiled: Vec<TriFuncId> = Vec::new();
        for ir_module in &program.modules {
            for func in &ir_module.functions {
                let cl_id = match func_id_map.get(&func.id) {
                    Some(id) => *id,
                    None => continue,
                };
                let mut cl_ctx = self.module.make_context();
                cl_ctx.func.signature = build_signature(func)?;
                if let Err(err) = self.emit_function_body(func, &ctx, &mut cl_ctx) {
                    // Tier-down: skip this function, others still compile.
                    let _ = err;
                    self.module.clear_context(&mut cl_ctx);
                    continue;
                }
                if let Err(err) = self.module.define_function(cl_id, &mut cl_ctx) {
                    let _ = err;
                    self.module.clear_context(&mut cl_ctx);
                    continue;
                }
                self.module.clear_context(&mut cl_ctx);
                compiled.push(func.id);
            }
        }

        // Finalize everything together. Single mmap-flip across all
        // bodies — required before `get_finalized_function`.
        self.module.finalize_definitions().map_err(cranelift_err)?;
        for tri_id in compiled {
            let cl_id = func_id_map[&tri_id];
            let raw = self.module.get_finalized_function(cl_id);
            out_cache.insert(tri_id, NativeCodePtr { addr: raw as usize });
        }
        Ok(())
    }

    /// Shared body-emit routine called by both the single-function
    /// and the program-level paths. Threads `ProgramContext` for call
    /// dispatch + constant pool access.
    fn emit_function_body(
        &mut self,
        func: &IrFunction,
        ctx: &ProgramContext<'_>,
        cl_ctx: &mut cranelift_codegen::Context,
    ) -> Result<(), JitError> {
        let mut fn_builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut cl_ctx.func, &mut fn_builder_ctx);

        // Pre-declare a Cranelift block per Triết BlockId so forward
        // branches resolve. Cranelift requires the entry block to
        // receive function parameters.
        let mut block_map: HashMap<BlockId, Block> = HashMap::new();
        for ir_block in &func.blocks {
            let cl_block = builder.create_block();
            block_map.insert(ir_block.id, cl_block);
        }

        let entry_ir_block = func
            .blocks
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

        // Walk every block in declaration order, switch into it, and
        // emit per-instruction Cranelift IR.
        for ir_block in &func.blocks {
            let cl_block = block_map[&ir_block.id];
            builder.switch_to_block(cl_block);
            for instr in &ir_block.instructions {
                translate_instruction(
                    &mut builder,
                    &mut self.module,
                    &mut value_map,
                    &block_map,
                    ctx,
                    func,
                    instr,
                )?;
            }
        }

        builder.seal_all_blocks();
        builder.finalize();
        Ok(())
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
/// SSA def; reads `block_map` for branch targets; consults `ctx` for
/// inline constants + call targets.
#[allow(clippy::too_many_lines)]
fn translate_instruction(
    builder: &mut FunctionBuilder<'_>,
    module: &mut JITModule,
    value_map: &mut HashMap<ValueId, Value>,
    block_map: &HashMap<BlockId, Block>,
    ctx: &ProgramContext<'_>,
    func: &IrFunction,
    instr: &Instruction,
) -> Result<(), JitError> {
    match instr {
        Instruction::Const { dest, constant } => {
            let val = materialize_constant(builder, ctx.constants, *constant)?;
            value_map.insert(*dest, val);
        }
        Instruction::Add { dest, lhs, rhs } => {
            let l = resolve_operand(builder, value_map, ctx, *lhs)?;
            let r = resolve_operand(builder, value_map, ctx, *rhs)?;
            let v = builder.ins().iadd(l, r);
            value_map.insert(*dest, v);
        }
        Instruction::Sub { dest, lhs, rhs } => {
            let l = resolve_operand(builder, value_map, ctx, *lhs)?;
            let r = resolve_operand(builder, value_map, ctx, *rhs)?;
            let v = builder.ins().isub(l, r);
            value_map.insert(*dest, v);
        }
        Instruction::Mul { dest, lhs, rhs } => {
            let l = resolve_operand(builder, value_map, ctx, *lhs)?;
            let r = resolve_operand(builder, value_map, ctx, *rhs)?;
            let v = builder.ins().imul(l, r);
            value_map.insert(*dest, v);
        }
        Instruction::Neg { dest, operand } => {
            let v = resolve_operand(builder, value_map, ctx, *operand)?;
            let result = builder.ins().ineg(v);
            value_map.insert(*dest, result);
        }
        Instruction::CallLocal { dest, callee, args } => {
            translate_call(builder, module, value_map, ctx, *dest, *callee, args)?;
        }
        Instruction::CallCrossModule { dest, path, args } => {
            let callee = ctx.path_to_funcid.get(path).copied().ok_or_else(|| {
                JitError::UnsupportedOpcode {
                    opcode: format!("CallCrossModule path `{path}` not in program"),
                }
            })?;
            translate_call(builder, module, value_map, ctx, *dest, callee, args)?;
        }
        Instruction::WitnessCall {
            dest,
            path,
            witness_idx: _,
            args,
        } => {
            // v0.4 semantics per ADR-0012: witness tables informational
            // only; dispatch identical to CallCrossModule. The linker
            // already monomorphized intra-package generics into CallLocal,
            // so reaching this opcode means cross-package generic +
            // witness validation already passed at typecheck time.
            let callee = ctx.path_to_funcid.get(path).copied().ok_or_else(|| {
                JitError::UnsupportedOpcode {
                    opcode: format!("WitnessCall path `{path}` not in program"),
                }
            })?;
            translate_call(builder, module, value_map, ctx, *dest, callee, args)?;
        }
        Instruction::Eq { dest, lhs, rhs } => {
            emit_icmp(builder, value_map, ctx, IntCC::Equal, *dest, *lhs, *rhs)?;
        }
        Instruction::Ne { dest, lhs, rhs } => {
            emit_icmp(builder, value_map, ctx, IntCC::NotEqual, *dest, *lhs, *rhs)?;
        }
        Instruction::Lt { dest, lhs, rhs } => {
            emit_icmp(
                builder,
                value_map,
                ctx,
                IntCC::SignedLessThan,
                *dest,
                *lhs,
                *rhs,
            )?;
        }
        Instruction::Le { dest, lhs, rhs } => {
            emit_icmp(
                builder,
                value_map,
                ctx,
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
                ctx,
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
                ctx,
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
            let c = resolve_operand(builder, value_map, ctx, *cond)?;
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
            let c = resolve_operand(builder, value_map, ctx, *cond)?;
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
                let v = resolve_operand(builder, value_map, ctx, *op)?;
                builder.ins().return_(&[v]);
            } else {
                // No-value return: emit a Unit i8 0 placeholder per
                // build_signature returning i8 for Unit.
                let unit = builder.ins().iconst(map_type(&func.return_type)?, 0);
                builder.ins().return_(&[unit]);
            }
        }
        // v0.9.x.jit.4 — structured CallBuiltin tier-down per ADR-0030
        // §12 backlog. Full builtin shim layer (extern "C" Rust
        // registry + RuntimeValue ABI marshaling for String / Vector /
        // HashMap / Atomic / etc. across 43 builtins) defers v0.10.
        // Until then, any function calling a stdlib builtin
        // tier-downs to VM dispatch with a structured diagnostic that
        // names the specific builtin — easier to grep + roadmap than
        // the catch-all Debug-format fallback.
        Instruction::CallBuiltin { name, args, .. } => {
            return Err(JitError::UnsupportedOpcode {
                opcode: format!(
                    "CallBuiltin({name}) with {} arg(s) — full builtin shim \
                     layer defers v0.10 per ADR-0030 §12 backlog \
                     (RuntimeValue ABI marshaling complexity)",
                    args.len()
                ),
            });
        }
        // Everything else triggers tier-down to VM-only for this fn.
        // Use the IR `Display` impl (via `triet_ir::Instruction`'s
        // pretty form) rather than `Debug` — easier to read in
        // diagnostics, and stable across refactors of internal
        // struct shape.
        other => {
            return Err(JitError::UnsupportedOpcode {
                opcode: format!("{other}"),
            });
        }
    }
    Ok(())
}

/// Resolve an [`Operand`] into a Cranelift [`Value`] live in the
/// current block. `Value(id)` looks up the SSA map;
/// `Operand::Const(id)` materializes via [`materialize_constant`]
/// using the program-level constant pool.
fn resolve_operand(
    builder: &mut FunctionBuilder<'_>,
    value_map: &HashMap<ValueId, Value>,
    ctx: &ProgramContext<'_>,
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
        Operand::Const(const_id) => materialize_constant(builder, ctx.constants, const_id),
    }
}

/// Materialize a [`Constant`] pool entry into a Cranelift SSA value
/// of the appropriate Cranelift type. Used by both `Instruction::Const`
/// (statement form) and `Operand::Const` (inline form).
fn materialize_constant(
    builder: &mut FunctionBuilder<'_>,
    constants: &ConstantPool,
    const_id: ConstId,
) -> Result<Value, JitError> {
    let constant = constants.get(const_id).ok_or_else(|| JitError::Cranelift {
        message: format!("ConstId({}) missing from pool", const_id.0),
    })?;
    let val = match constant {
        Constant::Integer(i) => builder.ins().iconst(I64, i.to_i64()),
        Constant::Tryte(t) => {
            // Tryte fits in i16 by construction (9-trit range
            // ~±9841), so the i64→i16 narrowing is lossless.
            #[allow(clippy::cast_possible_truncation)]
            let narrowed = t.to_i64() as i16;
            builder.ins().iconst(I16, i64::from(narrowed))
        }
        Constant::Trit(t) => builder.ins().iconst(I8, i64::from(t.to_i8())),
        Constant::Trilean(t) => {
            // Trilean → i8 with {-1, 0, +1} encoding per ADR-0010 §3.
            let raw = match t {
                Trilean::False => -1_i64,
                Trilean::Unknown => 0,
                Trilean::True => 1,
            };
            builder.ins().iconst(I8, raw)
        }
        Constant::Unit => builder.ins().iconst(I8, 0),
        // Strings + Long + Null defer .4 (heap layouts + i128 pair lowering).
        other => {
            return Err(JitError::UnsupportedOpcode {
                opcode: format!("Constant variant {other:?} — defer to later sub-phase"),
            });
        }
    };
    Ok(val)
}

/// Emit an integer compare returning a Trilean i8 (`+1` for true,
/// `-1` for false; Unknown is not produced because non-nullable
/// integer comparisons can't yield Unknown per ADR-0021).
fn emit_icmp(
    builder: &mut FunctionBuilder<'_>,
    value_map: &mut HashMap<ValueId, Value>,
    ctx: &ProgramContext<'_>,
    cc: IntCC,
    dest: ValueId,
    lhs: Operand,
    rhs: Operand,
) -> Result<(), JitError> {
    let l = resolve_operand(builder, value_map, ctx, lhs)?;
    let r = resolve_operand(builder, value_map, ctx, rhs)?;
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

/// Emit a direct call given a resolved Triết [`TriFuncId`] callee.
/// Shared by `CallLocal` / `CallCrossModule` / `WitnessCall` since
/// all three lower to the same Cranelift `call $func` form at the
/// v0.4 dispatch level. Witness tables remain informational only
/// per ADR-0012 §2.
fn translate_call(
    builder: &mut FunctionBuilder<'_>,
    module: &mut JITModule,
    value_map: &mut HashMap<ValueId, Value>,
    ctx: &ProgramContext<'_>,
    dest: Option<ValueId>,
    callee: TriFuncId,
    args: &[Operand],
) -> Result<(), JitError> {
    let cl_callee =
        ctx.func_id_map
            .get(&callee)
            .copied()
            .ok_or_else(|| JitError::UnsupportedOpcode {
                opcode: format!("call target FuncId({}) not in program", callee.0),
            })?;
    let arg_values: Vec<Value> = args
        .iter()
        .map(|op| resolve_operand(builder, value_map, ctx, *op))
        .collect::<Result<_, _>>()?;
    let func_ref = module.declare_func_in_func(cl_callee, builder.func);
    let call_inst = builder.ins().call(func_ref, &arg_values);
    if let Some(dest_id) = dest {
        let results = builder.inst_results(call_inst);
        if let Some(&result_val) = results.first() {
            value_map.insert(dest_id, result_val);
        }
    }
    Ok(())
}
