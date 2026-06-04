//! MIR → Cranelift IR lowering (Phase 3.1+3.2).
//!
//! Takes a `triet_mir::Body` and produces compiled native code via
//! Cranelift JIT. Supports: integer arithmetic, comparisons, and
//! control flow (If / Goto / Return).
//!
//! # SSA handling
//!
//! MIR `Local` values are mutable (non-SSA). Each MIR `Local` maps to
//! a Cranelift `Variable`. `FunctionBuilder::declare_var` / `def_var` /
//! `use_var` handle SSA-ification automatically — Cranelift inserts
//! block parameters (φ-nodes) at seal time.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::{I8, I64};
use cranelift_codegen::ir::{AbiParam, InstBuilder, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use triet_mir::DUMMY_SPAN;
use triet_mir::{
    BasicBlock, BinOp, Body, CallTarget, ConstValue, ControlFlowGraph, Local, Statement, Terminator,
};

// ── Errors ──────────────────────────────────────────────────

/// JIT compilation error.
#[derive(Debug)]
pub enum JitError {
    /// A MIR construct that isn't supported yet.
    Unsupported(String),
    /// Cranelift module error.
    Module(String),
}

impl std::fmt::Display for JitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(msg) => write!(f, "JIT unsupported: {msg}"),
            Self::Module(msg) => write!(f, "JIT module error: {msg}"),
        }
    }
}

// ── Compiled function ───────────────────────────────────────

/// A JIT-compiled native function.
pub struct CompiledFunction {
    code_ptr: *const u8,
}

impl CompiledFunction {
    /// Call with zero args, returning i64.
    ///
    /// # Safety
    /// Caller must ensure the function has signature () -> i64
    /// and the JIT module that produced it is still alive.
    #[allow(unsafe_code)]
    pub unsafe fn call_i64_0(&self) -> i64 {
        let f: extern "C" fn() -> i64 = unsafe { std::mem::transmute(self.code_ptr) };
        f()
    }

    /// Call with one i64 arg, returning i64.
    ///
    /// # Safety
    /// Caller must ensure the function has signature (i64) -> i64
    /// and the JIT module that produced it is still alive.
    #[allow(unsafe_code)]
    pub unsafe fn call_i64_1(&self, a: i64) -> i64 {
        let f: extern "C" fn(i64) -> i64 = unsafe { std::mem::transmute(self.code_ptr) };
        f(a)
    }

    /// Call with two i64 args, returning i64.
    ///
    /// # Safety
    /// Caller must ensure the function has signature (i64, i64) -> i64
    /// and the JIT module that produced it is still alive.
    #[allow(unsafe_code)]
    pub unsafe fn call_i64_2(&self, a: i64, b: i64) -> i64 {
        let f: extern "C" fn(i64, i64) -> i64 = unsafe { std::mem::transmute(self.code_ptr) };
        f(a, b)
    }
}

// ── Shim symbols ─────────────────────────────────────────────

/// A registered `extern "C"` shim symbol callable from JIT code.
///
/// Construct via the type-safe factory methods (`fn_0_1`, `fn_2_1`, etc.)
/// which enforce the correct function signature at compile time. The method
/// name encodes the arity: `fn_N_M` = N args, returns M values (0 or 1).
#[derive(Clone, Debug)]
pub struct ShimSymbol {
    /// Symbol name (e.g. `__triet_pow`).
    pub name: String,
    /// Function pointer address.
    pub addr: usize,
    /// Number of i64 arguments.
    pub arity: usize,
    /// Whether the function returns an i64 (true) or is void (false).
    pub has_return: bool,
}

impl ShimSymbol {
    /// Register a 0-arg → 1-return shim.
    pub fn fn_0_1(name: &str, f: extern "C" fn() -> i64) -> Self {
        Self {
            name: name.into(),
            addr: f as usize,
            arity: 0,
            has_return: true,
        }
    }

    /// Register a 1-arg → 1-return shim.
    pub fn fn_1_1(name: &str, f: extern "C" fn(i64) -> i64) -> Self {
        Self {
            name: name.into(),
            addr: f as usize,
            arity: 1,
            has_return: true,
        }
    }

    /// Register a 2-arg → 1-return shim.
    pub fn fn_2_1(name: &str, f: extern "C" fn(i64, i64) -> i64) -> Self {
        Self {
            name: name.into(),
            addr: f as usize,
            arity: 2,
            has_return: true,
        }
    }

    /// Register a 2-arg → void shim.
    pub fn fn_2_0(name: &str, f: extern "C" fn(i64, i64)) -> Self {
        Self {
            name: name.into(),
            addr: f as usize,
            arity: 2,
            has_return: false,
        }
    }
}

// ── JIT context ─────────────────────────────────────────────

/// Holds Cranelift JIT state across compilations.
pub struct JitContext {
    module: JITModule,
    /// Map from MIR Local to Cranelift Variable (one per MIR local).
    /// Bậc A: every value is a single i64 — scalars unboxed, aggregates as
    /// opaque i64 pointers to VM heap objects. No split disc/payload for Outcome.
    locals: HashMap<Local, Variable>,
    /// Map from MIR `BasicBlock` to Cranelift Block.
    blocks: HashMap<BasicBlock, cranelift_codegen::ir::Block>,
    /// Blocks that have been sealed.
    sealed: HashSet<BasicBlock>,
    /// Blocks that have been filled.
    filled: HashSet<BasicBlock>,
    /// Map from function name → Cranelift FuncId (for cross-function calls).
    func_ids: HashMap<String, cranelift_module::FuncId>,
    /// Registered shim symbols (extern "C" functions).
    shim_registry: HashMap<String, ShimSymbol>,
}

impl JitContext {
    /// Return the Cranelift Variable for a MIR Local.
    /// Bậc A: one Cranelift Variable per MIR Local — everything is i64.
    fn var(&self, l: Local) -> Variable {
        Variable::from_u32(l.0 as u32)
    }

    /// Create a new JIT context with host ISA detection (no shims).
    pub fn new() -> Self {
        Self::with_shims(&[])
    }

    /// Create a new JIT context with registered shim symbols.
    ///
    /// Each shim is registered as an `extern "C"` symbol in the JIT module
    /// so that `CallTarget::Shim` calls resolve at link time.
    pub fn with_shims(shims: &[ShimSymbol]) -> Self {
        let flag_builder = cranelift_codegen::settings::builder();
        let isa_builder = cranelift_native::builder().expect("host ISA detection failed");
        let isa = isa_builder
            .finish(cranelift_codegen::settings::Flags::new(flag_builder))
            .expect("host ISA not supported");
        let mut jit_builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());

        // Register shim symbols so they can be resolved at link time
        for shim in shims {
            jit_builder.symbol(&shim.name, shim.addr as *const u8);
        }

        let mut shim_registry = HashMap::new();
        for shim in shims {
            shim_registry.insert(shim.name.clone(), shim.clone());
        }

        let module = JITModule::new(jit_builder);
        Self {
            module,
            locals: HashMap::new(),
            blocks: HashMap::new(),
            sealed: HashSet::new(),
            filled: HashSet::new(),
            func_ids: HashMap::new(),
            shim_registry,
        }
    }

    /// Compile a single MIR body (no cross-function calls).
    pub fn compile(&mut self, body: &Body) -> Result<CompiledFunction, JitError> {
        let result = self.compile_multi(&[body])?;
        let compiled = result
            .into_iter()
            .next()
            .map(|(_, f)| f)
            .expect("just compiled one function");
        Ok(compiled)
    }

    /// Compile multiple MIR bodies that may call each other.
    ///
    /// Phase 1: declare all functions in the module.
    /// Phase 2: build each function body (can reference others via func_ids).
    /// Phase 3: define all functions + finalize.
    pub fn compile_multi(
        &mut self,
        bodies: &[&Body],
    ) -> Result<HashMap<String, CompiledFunction>, JitError> {
        // ── Phase 1: declare all functions ─────────────────
        let mut sigs: Vec<(String, Signature)> = Vec::new();
        self.func_ids.clear();

        for body in bodies {
            let mut sig = Signature::new(CallConv::SystemV);
            for _ in &body.signature.params {
                sig.params.push(AbiParam::new(I64));
            }
            // Bậc A: every function returns a single i64. Aggregates
            // (including Outcome) are opaque i64 pointers to VM heap objects.
            sig.returns.push(AbiParam::new(I64));

            let func_id = self
                .module
                .declare_function(&body.signature.name, Linkage::Local, &sig)
                .map_err(|e| JitError::Module(format!("declare {}: {e}", body.signature.name)))?;

            self.func_ids.insert(body.signature.name.clone(), func_id);
            sigs.push((body.signature.name.clone(), sig));
        }

        // ── Phase 2: build each function body ──────────────
        let mut contexts: Vec<cranelift_codegen::Context> = Vec::new();
        for body in bodies {
            let mut cl_ctx = self.module.make_context();
            cl_ctx.func.signature = Signature::new(CallConv::SystemV);
            for _ in &body.signature.params {
                cl_ctx.func.signature.params.push(AbiParam::new(I64));
            }
            // Bậc A: single i64 return for all functions
            cl_ctx.func.signature.returns.push(AbiParam::new(I64));

            let mut fn_builder_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut cl_ctx.func, &mut fn_builder_ctx);

            self.build_body(&mut builder, body)?;
            builder.finalize();

            contexts.push(cl_ctx);
        }

        // ── Phase 3: define + finalize ─────────────────────
        for (i, body) in bodies.iter().enumerate() {
            let func_id = self.func_ids[&body.signature.name];
            self.module
                .define_function(func_id, &mut contexts[i])
                .map_err(|e| JitError::Module(format!("define {}: {e}", body.signature.name)))?;
        }

        self.module
            .finalize_definitions()
            .map_err(|e| JitError::Module(format!("finalize: {e}")))?;

        // ── Collect results ────────────────────────────────
        let mut result = HashMap::new();
        for body in bodies {
            let func_id = self.func_ids[&body.signature.name];
            let code_ptr = self.module.get_finalized_function(func_id);
            result.insert(body.signature.name.clone(), CompiledFunction { code_ptr });
        }

        Ok(result)
    }

    /// Build the Cranelift IR for a single function body.
    fn build_body(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        body: &Body,
    ) -> Result<(), JitError> {
        let cfg = body.build_cfg();

        // ── Declare variables (1 per MIR local, Bậc A: all i64) ──
        self.locals.clear();
        for i in 0..body.num_locals {
            let var = builder.declare_var(I64);
            self.locals.insert(Local(i), var);
        }

        // Pre-declare blocks
        self.blocks.clear();
        self.sealed.clear();
        self.filled.clear();

        let entry_block = builder.create_block();
        self.blocks.insert(cfg.entry, entry_block);
        for i in 0..cfg.blocks.len() {
            let bb = BasicBlock(i);
            if bb != cfg.entry {
                let block = builder.create_block();
                self.blocks.insert(bb, block);
            }
        }

        // Entry block: params → var slots
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        for (i, _) in body.signature.params.iter().enumerate() {
            let var = self.var(Local(i));
            let param_val = builder.block_params(entry_block)[i];
            builder.def_var(var, param_val);
        }
        builder.seal_block(entry_block);
        self.sealed.insert(cfg.entry);
        self.filled.insert(cfg.entry);

        // Lower entry
        self.lower_block_statements(builder, body, cfg.entry)?;
        self.lower_block_terminator(builder, body, cfg.entry)?;

        // RPO for remaining
        let order = reverse_post_order(&cfg);
        for &block in &order {
            if block == cfg.entry {
                continue;
            }
            self.lower_block(builder, body, block)?;
        }

        // Seal unsealed
        for i in 0..cfg.blocks.len() {
            let bb = BasicBlock(i);
            if !self.sealed.contains(&bb) {
                let block = self.blocks[&bb];
                builder.seal_block(block);
                self.sealed.insert(bb);
            }
        }

        Ok(())
    }

    /// Fill a single basic block.
    fn lower_block(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        body: &Body,
        block: BasicBlock,
    ) -> Result<(), JitError> {
        let clif_block = self.blocks[&block];
        builder.switch_to_block(clif_block);
        self.lower_block_statements(builder, body, block)?;
        self.lower_block_terminator(builder, body, block)?;
        self.filled.insert(block);

        // Seal if all predecessors filled
        let all_preds_filled = body.build_cfg().blocks[block.0]
            .predecessors
            .iter()
            .all(|p| self.filled.contains(p));
        if all_preds_filled && !self.sealed.contains(&block) {
            builder.seal_block(clif_block);
            self.sealed.insert(block);
        }

        Ok(())
    }

    /// Lower statements in a block.
    fn lower_block_statements(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        body: &Body,
        block: BasicBlock,
    ) -> Result<(), JitError> {
        let block_data = &body.blocks[block.0];
        for stmt in &block_data.statements {
            match stmt {
                Statement::StorageLive(_, _) | Statement::StorageDead(_, _) => {
                    // No-op at runtime — borrow checker verified safety
                }

                Statement::Const { dest, value, .. } => {
                    let val = match value {
                        ConstValue::Integer(n) => {
                            let n_i64 = i64::try_from(*n).map_err(|_| {
                                JitError::Unsupported(format!(
                                    "Integer constant {n} does not fit in i64 — \
                                     Bậc A only supports 64-bit values. \
                                     Triết Integer is 27-trit (~7.6×10^12 signed), \
                                     which fits in i64 (~9.2×10^18). \
                                     This value may come from a buggy lowerer."
                                ))
                            })?;
                            builder.ins().iconst(I64, n_i64)
                        }
                        ConstValue::Trit(t) => builder.ins().iconst(I8, i64::from(*t)),
                        ConstValue::Unit => builder.ins().iconst(I64, 0),
                        ConstValue::String(_) => {
                            return Err(JitError::Unsupported(
                                "String const not yet supported".into(),
                            ));
                        }
                    };
                    let var = self.var(dest.local);
                    builder.def_var(var, val);
                }

                Statement::Assign { dest, source, .. } => {
                    let src_var = self.var(source.local);
                    let val = builder.use_var(src_var);
                    let dest_var = self.var(dest.local);
                    builder.def_var(dest_var, val);
                }

                Statement::Borrow { dest, source, .. } => {
                    // S6 references = raw pointers at runtime — just copy
                    let src_var = self.var(source.local);
                    let val = builder.use_var(src_var);
                    let dest_var = self.var(dest.local);
                    builder.def_var(dest_var, val);
                }

                Statement::BinaryOp {
                    dest,
                    op,
                    left,
                    right,
                    ..
                } => {
                    let l_var = self.var(left.local);
                    let r_var = self.var(right.local);
                    let lhs = builder.use_var(l_var);
                    let rhs = builder.use_var(r_var);

                    let result = lower_binop(builder, *op, lhs, rhs);
                    let dest_var = self.var(dest.local);
                    builder.def_var(dest_var, result);
                }

                // ── Outcome ops (provably unreachable through real pipeline) ─
                //
                // These MIR statements exist so the borrow checker can model
                // Outcome discriminant/payload extraction. However, the lowerer
                // (`triet-lower`) does NOT yet lower `Expr::OutcomeConstructor`
                // (it returns `Err(LowerError::unsupported_expr)`), and MIR has
                // no `OutcomeNew` statement to CREATE an Outcome value. Therefore
                // these extraction ops CANNOT be reached through the real
                // .tri → lower → MIR → JIT pipeline today.
                //
                // If they WERE reachable, the current pass-through (identity copy
                // of a single i64) would be WRONG: a single i64 cannot carry both
                // the trit discriminant AND the success/error payload. Real
                // extraction requires a packed representation (Bậc C).
                //
                // "Refuse over guess": the JIT refuses to compile these until
                // the lowerer produces real Outcome values and a packed ABI exists.
                Statement::OutcomeDiscriminant { .. }
                | Statement::OutcomeUnwrap { .. }
                | Statement::OutcomeUnwrapError { .. } => {
                    return Err(JitError::Unsupported(
                        "Outcome ops require Bậc C packed ABI; \
                         the lowerer does not yet produce Outcome values, \
                         so these MIR statements are unreachable through \
                         the real pipeline. If you are seeing this error, \
                         the lowerer has been updated to lower Outcome \
                         constructors — the JIT backend must be updated \
                         to implement packed extraction before removing \
                         this guard."
                            .into(),
                    ));
                }

                Statement::Drop(_, _) => {
                    // No-op for scalars at runtime
                }
            }

            // NLL loan ending: handled by borrow checker at compile time
        }

        Ok(())
    }

    /// Lower a block terminator.
    fn lower_block_terminator(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        body: &Body,
        block: BasicBlock,
    ) -> Result<(), JitError> {
        let block_data = &body.blocks[block.0];

        match &block_data.terminator {
            Terminator::Return { values, .. } => {
                // Bậc A: single i64 return. Multi-value return (BinaryOutcome,
                // TernaryOutcome, struct returns) is deferred to Bậc C.
                // Refuse over guess — returning only the first value would
                // silently drop the second, which is a miscompile.
                if values.len() > 1 {
                    return Err(JitError::Unsupported(
                        "multi-value return requires Bậc C packed ABI; \
                         Bậc A only supports single i64 returns. \
                         The function's ReturnShape asks for more than one \
                         return value, but the JIT cannot yet pack them."
                            .into(),
                    ));
                }
                let val = if values.is_empty() {
                    builder.ins().iconst(I64, 0)
                } else {
                    builder.use_var(self.var(values[0]))
                };
                builder.ins().return_(&[val]);
            }

            Terminator::Goto { target, .. } => {
                let target_block = self.blocks[target];
                builder.ins().jump(target_block, &[]);
            }

            Terminator::If {
                cond,
                positive_bb,
                zero_bb,
                negative_bb,
                ..
            } => {
                let cond_var = self.var(*cond);
                let cond_val = builder.use_var(cond_var);
                let pos_block = self.blocks[positive_bb];
                let neg_block = self.blocks[negative_bb];
                // cond_val is i64 (all MIR locals are i64). Trilean encoding:
                //   True=1, Unknown=0, False=-1
                let zero_val = builder.ins().iconst(I64, 0);

                if let Some(zero) = zero_bb {
                    // Ternary branch (if?): 3-way
                    let zero_block = self.blocks[zero];
                    let is_zero = builder.ins().icmp(IntCC::Equal, cond_val, zero_val);
                    let fallthrough = builder.create_block();
                    builder
                        .ins()
                        .brif(is_zero, zero_block, &[], fallthrough, &[]);

                    builder.switch_to_block(fallthrough);
                    let is_pos = builder
                        .ins()
                        .icmp(IntCC::SignedGreaterThan, cond_val, zero_val);
                    builder.ins().brif(is_pos, pos_block, &[], neg_block, &[]);
                } else {
                    // Binary branch (if): 2-way
                    let is_pos = builder
                        .ins()
                        .icmp(IntCC::SignedGreaterThan, cond_val, zero_val);
                    builder.ins().brif(is_pos, pos_block, &[], neg_block, &[]);
                }
            }

            Terminator::CallDispatch {
                callee_name,
                target,
                args,
                return_bb,
                dest,
                ..
            } => {
                match target {
                    CallTarget::Jit => {
                        // Look up callee's FuncId
                        let callee_id = self
                            .func_ids
                            .get(callee_name)
                            .copied()
                            .ok_or_else(|| {
                                JitError::Unsupported(format!(
                                    "callee `{callee_name}` not found — compile it first via compile_multi"
                                ))
                            })?;

                        // Import callee into current function
                        let func_ref = self.module.declare_func_in_func(callee_id, builder.func);

                        // Prepare arguments
                        let arg_vals: Vec<_> = args
                            .iter()
                            .map(|a| {
                                let var = self.var(*a);
                                builder.use_var(var)
                            })
                            .collect();

                        // Emit call
                        let call_inst = builder.ins().call(func_ref, &arg_vals);

                        // Store return value (single i64 in Bậc A).
                        if !dest.is_empty() {
                            let ret_val = builder.inst_results(call_inst)[0];
                            builder.def_var(self.var(dest[0]), ret_val);
                        }

                        // Jump to return block
                        let ret_block = self.blocks[return_bb];
                        builder.ins().jump(ret_block, &[]);
                    }

                    CallTarget::Shim => {
                        let shim = self.shim_registry.get(callee_name).ok_or_else(|| {
                            JitError::Unsupported(format!(
                                "shim `{callee_name}` not registered — add it to JitContext::with_shims()"
                            ))
                        })?;

                        // Declare the shim as an imported extern "C" function
                        // if we haven't already
                        let func_id = if let Some(&id) = self.func_ids.get(callee_name) {
                            id
                        } else {
                            let mut sig = Signature::new(CallConv::SystemV);
                            for _ in 0..shim.arity {
                                sig.params.push(AbiParam::new(I64));
                            }
                            if shim.has_return {
                                sig.returns.push(AbiParam::new(I64));
                            }
                            let id = self
                                .module
                                .declare_function(callee_name, Linkage::Import, &sig)
                                .map_err(|e| {
                                    JitError::Module(format!("declare shim {callee_name}: {e}"))
                                })?;
                            self.func_ids.insert(callee_name.clone(), id);
                            id
                        };

                        let func_ref = self.module.declare_func_in_func(func_id, builder.func);

                        let arg_vals: Vec<_> = args
                            .iter()
                            .map(|a| {
                                let var = self.var(*a);
                                builder.use_var(var)
                            })
                            .collect();

                        let call_inst = builder.ins().call(func_ref, &arg_vals);

                        if shim.has_return && !dest.is_empty() {
                            let ret_val = builder.inst_results(call_inst)[0];
                            builder.def_var(self.var(dest[0]), ret_val);
                        }

                        let ret_block = self.blocks[return_bb];
                        builder.ins().jump(ret_block, &[]);
                    }
                }
            }

            Terminator::Unreachable { .. } => {
                builder
                    .ins()
                    .trap(cranelift_codegen::ir::TrapCode::unwrap_user(0));
            }
        }

        Ok(())
    }
}

impl Default for JitContext {
    fn default() -> Self {
        Self::new()
    }
}

// ── BinOp lowering ───────────────────────────────────────────

/// Lower a MIR `BinOp` into Cranelift IR values.
///
/// All values are i64. Trilean-typed results use the encoding:
///   +1 = True, 0 = Unknown, -1 = False.
fn lower_binop(
    builder: &mut FunctionBuilder<'_>,
    op: BinOp,
    lhs: cranelift_codegen::ir::Value,
    rhs: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let i64 = I64;
    let i8 = I8;

    match op {
        // ── Arithmetic ──
        BinOp::Add => builder.ins().iadd(lhs, rhs),
        BinOp::Sub => builder.ins().isub(lhs, rhs),
        BinOp::Mul => builder.ins().imul(lhs, rhs),
        BinOp::Div => builder.ins().sdiv(lhs, rhs),
        BinOp::Mod => builder.ins().srem(lhs, rhs),

        // ── Ternary negation ──
        BinOp::Neg => builder.ins().ineg(lhs),

        // ── Comparisons → Trilean! (+1 / -1, never Unknown) ──
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let cc = match op {
                BinOp::Eq => IntCC::Equal,
                BinOp::Ne => IntCC::NotEqual,
                BinOp::Lt => IntCC::SignedLessThan,
                BinOp::Le => IntCC::SignedLessThanOrEqual,
                BinOp::Gt => IntCC::SignedGreaterThan,
                BinOp::Ge => IntCC::SignedGreaterThanOrEqual,
                _ => unreachable!(),
            };
            let cmp = builder.ins().icmp(cc, lhs, rhs);
            let one = builder.ins().iconst(i8, 1);
            let neg_one = builder.ins().iconst(i8, -1_i64);
            let trilean_i8 = builder.ins().select(cmp, one, neg_one);
            builder.ins().sextend(i64, trilean_i8)
        }

        // ── Universal logic ops (identical in Ł3 and K3) ──
        // And = min(a, b)
        BinOp::LukAnd => {
            let is_lt = builder.ins().icmp(IntCC::SignedLessThan, lhs, rhs);
            builder.ins().select(is_lt, lhs, rhs)
        }
        // Or = max(a, b)
        BinOp::LukOr => {
            let is_gt = builder.ins().icmp(IntCC::SignedGreaterThan, lhs, rhs);
            builder.ins().select(is_gt, lhs, rhs)
        }

        // ── Łukasiewicz Ł3 implies ──
        // a → b:
        //   a = False (-1)  → True (+1)
        //   a = True  (+1)  → b
        //   a = Unknown (0) → (b == False) ? Unknown (0) : True (+1)
        BinOp::LukImplies => {
            let neg_one = builder.ins().iconst(i64, -1_i64);
            let zero = builder.ins().iconst(i64, 0);
            let one = builder.ins().iconst(i64, 1);

            let is_false = builder.ins().icmp(IntCC::Equal, lhs, neg_one);
            let is_true = builder.ins().icmp(IntCC::Equal, lhs, one);
            let b_is_false = builder.ins().icmp(IntCC::Equal, rhs, neg_one);
            let unknown_result = builder.ins().select(b_is_false, zero, one);
            let non_false_result = builder.ins().select(is_true, rhs, unknown_result);
            builder.ins().select(is_false, one, non_false_result)
        }

        // ── Łukasiewicz Ł3 iff: (a → b) ∧ (b → a) ──
        BinOp::LukIff => {
            let ab = lower_binop(builder, BinOp::LukImplies, lhs, rhs);
            let ba = lower_binop(builder, BinOp::LukImplies, rhs, lhs);
            lower_binop(builder, BinOp::LukAnd, ab, ba)
        }

        // ── Łukasiewicz Ł3 xor: ¬(a ↔ b) ──
        BinOp::LukXor => {
            let iff = lower_binop(builder, BinOp::LukIff, lhs, rhs);
            builder.ins().ineg(iff)
        }

        // ── Kleene K3 implies = max(¬a, b) = max(-a, b) ──
        BinOp::KleeneImplies => {
            let not_a = builder.ins().ineg(lhs);
            let is_gt = builder.ins().icmp(IntCC::SignedGreaterThan, not_a, rhs);
            builder.ins().select(is_gt, not_a, rhs)
        }

        // ── Kleene K3 iff: (a → b) ∧ (b → a) ──
        BinOp::KleeneIff => {
            let ab = lower_binop(builder, BinOp::KleeneImplies, lhs, rhs);
            let ba = lower_binop(builder, BinOp::KleeneImplies, rhs, lhs);
            lower_binop(builder, BinOp::LukAnd, ab, ba)
        }

        // ── Kleene K3 xor: ¬(a ↔ b) ──
        BinOp::KleeneXor => {
            let iff = lower_binop(builder, BinOp::KleeneIff, lhs, rhs);
            builder.ins().ineg(iff)
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────

// ── CFG traversal ───────────────────────────────────────────

fn reverse_post_order(cfg: &ControlFlowGraph) -> Vec<BasicBlock> {
    let mut visited = HashSet::new();
    let mut order = Vec::new();
    dfs_post(cfg, cfg.entry, &mut visited, &mut order);
    order.reverse();
    order
}

fn dfs_post(
    cfg: &ControlFlowGraph,
    block: BasicBlock,
    visited: &mut HashSet<BasicBlock>,
    order: &mut Vec<BasicBlock>,
) {
    if visited.contains(&block) {
        return;
    }
    visited.insert(block);
    for &succ in &cfg.blocks[block.0].successors {
        dfs_post(cfg, succ, visited, order);
    }
    order.push(block);
}

// ── Runtime shims (extern "C" functions callable from JIT) ───

/// Simple `extern "C"` shim: `multiply(a, b) = a * b`.
/// C ABI is the ONLY stable contract between Cranelift JIT and Rust.
/// `#[no_mangle]` ensures the symbol is visible to `nm` and dynamic lookup.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
extern "C" fn __test_shim_multiply(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b)
}

/// Integer power via exponentiation by squaring (`extern "C"` ABI).
/// `pow(base, exp)` = base^exp. Exponent must be >= 0.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_pow(base: i64, exp: i64) -> i64 {
    if exp < 0 {
        return 0; // undefined, fallback
    }
    let mut result: i64 = 1;
    let mut e = exp;
    let mut b = base;
    while e > 0 {
        if e & 1 != 0 {
            result = result.wrapping_mul(b);
        }
        e >>= 1;
        b = b.wrapping_mul(b);
    }
    result
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use triet_borrowck::{MirBuilder, binop, return_, storage_live};
    use triet_mir::{FunctionId, ParameterPassing};

    /// Compile and run `abs_diff`: `abs_diff(10, 3) == 7`.
    #[test]
    #[allow(unsafe_code)]
    fn abs_diff_jit_compile_and_run() {
        let mut b = MirBuilder::new("abs_diff_jit_test", "Integer");
        let a = b.add_param("a", ParameterPassing::Borrow);
        let b_param = b.add_param("b", ParameterPassing::Borrow);

        let cond = b.new_local();
        let tmp1 = b.new_local();
        let tmp2 = b.new_local();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(cond));
        b.push(bb0, binop(cond, BinOp::Gt, a, b_param));

        let bb1 = b.new_block();
        b.push(bb1, storage_live(tmp1));
        b.push(bb1, binop(tmp1, BinOp::Sub, a, b_param));
        b.set_terminator(bb1, return_(vec![tmp1]));

        let bb2 = b.new_block();
        b.push(bb2, storage_live(tmp2));
        b.push(bb2, binop(tmp2, BinOp::Sub, b_param, a));
        b.set_terminator(bb2, return_(vec![tmp2]));

        b.set_terminator(
            bb0,
            Terminator::If {
                cond,
                positive_bb: bb1,
                zero_bb: None,
                negative_bb: bb2,
                span: DUMMY_SPAN,
            },
        );

        let body = b.build(bb0);
        println!("=== MIR ===\n{body}");

        let mut ctx = JitContext::new();
        let func = ctx.compile(&body).expect("JIT compilation failed");

        let result = unsafe { func.call_i64_2(10, 3) };
        println!("abs_diff(10, 3) = {result}");
        assert_eq!(result, 7);

        let result = unsafe { func.call_i64_2(3, 10) };
        println!("abs_diff(3, 10) = {result}");
        assert_eq!(result, 7);

        let result = unsafe { func.call_i64_2(5, 5) };
        println!("abs_diff(5, 5) = {result}");
        assert_eq!(result, 0);
    }

    /// Simple addition: 42 + 58 = 100.
    #[test]
    #[allow(unsafe_code)]
    fn simple_add_jit_compile_and_run() {
        let mut b = MirBuilder::new("add_jit_test", "Integer");
        let a = b.add_param("a", ParameterPassing::Borrow);
        let b_param = b.add_param("b", ParameterPassing::Borrow);

        let result = b.new_local();
        let bb0 = b.new_block();
        b.push(bb0, storage_live(result));
        b.push(bb0, binop(result, BinOp::Add, a, b_param));
        b.set_terminator(bb0, return_(vec![result]));

        let body = b.build(bb0);
        let mut ctx = JitContext::new();
        let func = ctx.compile(&body).expect("JIT compilation failed");

        let result = unsafe { func.call_i64_2(42, 58) };
        println!("42 + 58 = {result}");
        assert_eq!(result, 100);
    }

    /// Compile and run recursive Fibonacci: `fib(10) == 55`.
    ///
    /// ```triet
    /// function fibonacci(n: Integer) -> Integer {
    ///     if n <= 1 {
    ///         return n;
    ///     } else {
    ///         return fibonacci(n - 1) + fibonacci(n - 2);
    ///     };
    /// }
    /// ```
    #[test]
    #[allow(unsafe_code)]
    fn fibonacci_jit_compile_and_run() {
        let mut b = MirBuilder::new("fibonacci", "Integer");
        let n = b.add_param("n", ParameterPassing::Borrow);

        let cond = b.new_local();
        let one = b.new_local();
        let tmp1 = b.new_local();
        let call1_result = b.new_local();
        let tmp2 = b.new_local();
        let call2_result = b.new_local();
        let sum = b.new_local();

        // bb0: if n <= 1 → bb1, else → bb2
        let bb0 = b.new_block();
        b.push(bb0, storage_live(cond));
        b.push(bb0, storage_live(one));
        b.push(
            bb0,
            triet_mir::Statement::Const {
                dest: one.into(),
                value: triet_mir::ConstValue::Integer(1),
                span: DUMMY_SPAN,
            },
        );
        b.push(bb0, binop(cond, BinOp::Le, n, one));

        // bb1: return n (base case: n <= 1)
        let bb1 = b.new_block();
        b.set_terminator(bb1, return_(vec![n]));

        // bb2: compute n - 1, call fibonacci(n - 1) → bb3
        let bb2 = b.new_block();
        b.push(bb2, storage_live(tmp1));
        b.push(bb2, binop(tmp1, BinOp::Sub, n, one));

        // bb3: store call1 result, compute n - 2, call fibonacci(n - 2) → bb4
        let bb3 = b.new_block();
        b.push(bb3, storage_live(tmp2));
        let two = b.new_local();
        b.push(bb3, storage_live(two));
        b.push(
            bb3,
            triet_mir::Statement::Const {
                dest: two.into(),
                value: triet_mir::ConstValue::Integer(2),
                span: DUMMY_SPAN,
            },
        );
        b.push(bb3, binop(tmp2, BinOp::Sub, n, two)); // tmp2 = n - 2

        // bb4: store call2 result, compute sum
        let bb4 = b.new_block();
        b.push(bb4, storage_live(sum));
        b.push(bb4, binop(sum, BinOp::Add, call1_result, call2_result));

        // bb5: return sum
        let bb5 = b.new_block();
        b.set_terminator(bb5, return_(vec![sum]));

        // Wire up terminators
        b.set_terminator(
            bb0,
            Terminator::If {
                cond,
                positive_bb: bb1, // n <= 1 → return n
                zero_bb: None,
                negative_bb: bb2, // n > 1 → compute
                span: DUMMY_SPAN,
            },
        );
        let fib_id = b.func_id_for("fibonacci");
        b.set_terminator(
            bb2,
            Terminator::CallDispatch {
                callee: fib_id,
                callee_name: "fibonacci".into(),
                target: CallTarget::Jit,
                args: vec![tmp1],
                return_bb: bb3,
                dest: vec![call1_result],
                span: DUMMY_SPAN,
            },
        );
        b.set_terminator(
            bb3,
            Terminator::CallDispatch {
                callee: fib_id,
                callee_name: "fibonacci".into(),
                target: CallTarget::Jit,
                args: vec![tmp2],
                return_bb: bb4,
                dest: vec![call2_result],
                span: DUMMY_SPAN,
            },
        );
        b.set_terminator(
            bb4,
            Terminator::Goto {
                target: bb5,
                span: DUMMY_SPAN,
            },
        );

        let body = b.build(bb0);
        println!("=== MIR (fibonacci) ===\n{body}");

        let mut ctx = JitContext::new();
        let compiled = ctx.compile_multi(&[&body]).expect("JIT compilation failed");
        let fib = compiled.get("fibonacci").expect("fibonacci function");

        // fib(0) = 0
        let result = unsafe { fib.call_i64_1(0) };
        println!("fib(0) = {result}");
        assert_eq!(result, 0);

        // fib(1) = 1
        let result = unsafe { fib.call_i64_1(1) };
        println!("fib(1) = {result}");
        assert_eq!(result, 1);

        // fib(5) = 5
        let result = unsafe { fib.call_i64_1(5) };
        println!("fib(5) = {result}");
        assert_eq!(result, 5);

        // fib(10) = 55
        let result = unsafe { fib.call_i64_1(10) };
        println!("fib(10) = {result}");
        assert_eq!(result, 55);
    }

    /// Outcome ops are **provably unreachable** through the real pipeline:
    /// the lowerer returns `Err(LowerError::unsupported_expr)` for
    /// `Expr::OutcomeConstructor`, and MIR has no `OutcomeNew` statement.
    ///
    /// This test hand-builds MIR with `OutcomeDiscriminant` (bypassing the
    /// lowerer guard) and verifies the JIT **refuses** to compile it. A
    /// pass-through identity copy would be wrong — a single i64 cannot
    /// carry both discriminant and payload. Real extraction requires Bậc C
    /// packed ABI.
    ///
    /// **If this test ever fails (JIT compiles successfully), someone
    /// removed the guard without implementing proper packed extraction.**
    /// That would be a miscompile — `disc(~+ v) == disc(~- e)` for
    /// identical payloads.
    #[test]
    fn outcome_discriminant_jit_refuses_to_compile() {
        let mut b = MirBuilder::new("outcome_disc_test", "Integer");
        let _dummy = b.add_param("dummy", ParameterPassing::Borrow);
        let outcome_val = b.new_local();
        let disc_result = b.new_local();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(outcome_val));
        b.push(
            bb0,
            triet_mir::Statement::Const {
                dest: outcome_val.into(),
                value: ConstValue::Integer(1),
                span: DUMMY_SPAN,
            },
        );
        b.push(bb0, storage_live(disc_result));
        b.push(
            bb0,
            triet_mir::Statement::OutcomeDiscriminant {
                dest: disc_result.into(),
                source: outcome_val.into(),
                span: DUMMY_SPAN,
            },
        );
        b.set_terminator(
            bb0,
            Terminator::Return {
                values: vec![disc_result],
                span: DUMMY_SPAN,
            },
        );

        let body = b.build(bb0);
        let mut ctx = JitContext::new();
        let result = ctx.compile(&body);

        match result {
            Err(JitError::Unsupported(msg)) => {
                assert!(
                    msg.contains("Outcome"),
                    "expected Outcome-related error, got: {msg}"
                );
            }
            Ok(_) => {
                panic!(
                    "JIT compiled OutcomeDiscriminant as pass-through — \
                     this is a miscompile. The JIT guard was removed \
                     without implementing packed extraction. \
                     disc(~+ v) would equal disc(~- e) for identical payloads."
                );
            }
            Err(other) => {
                panic!(
                    "unexpected JIT error (expected Unsupported, got {other}) — \
                     if the guard was changed, verify Outcome ops still refuse"
                );
            }
        }
    }

    /// Multi-value return is **provably unreachable** through the real
    /// pipeline (lowerer does not produce `ReturnShape::BinaryOutcome`).
    /// This test hand-builds MIR with a 2-value return and verifies the
    /// JIT **refuses** to compile it.
    ///
    /// **If this test fails**, someone removed the multi-return guard
    /// without implementing Bậc C packed ABI. Returning only `values[0]`
    /// would silently drop the second return value — a miscompile.
    #[test]
    fn multi_value_return_refuses_to_compile() {
        // Build a callee that returns 2 values (BinaryOutcome)
        let mut callee = MirBuilder::new("make_outcome", "Outcome");
        callee.set_return_shape(triet_mir::ReturnShape::BinaryOutcome);
        let _dummy = callee.add_param("dummy", ParameterPassing::Borrow);
        let disc_val = callee.new_local();
        let payload_val = callee.new_local();

        let bb0 = callee.new_block();
        callee.push(bb0, storage_live(disc_val));
        callee.push(
            bb0,
            triet_mir::Statement::Const {
                dest: disc_val.into(),
                value: ConstValue::Integer(1),
                span: DUMMY_SPAN,
            },
        );
        callee.push(bb0, storage_live(payload_val));
        callee.push(
            bb0,
            triet_mir::Statement::Const {
                dest: payload_val.into(),
                value: ConstValue::Integer(42),
                span: DUMMY_SPAN,
            },
        );
        callee.set_terminator(
            bb0,
            Terminator::Return {
                values: vec![disc_val, payload_val], // 2 values — triggers P6
                span: DUMMY_SPAN,
            },
        );
        let callee_body = callee.build(bb0);

        let mut ctx = JitContext::new();
        let result = ctx.compile(&callee_body);

        match result {
            Err(JitError::Unsupported(msg)) => {
                assert!(
                    msg.contains("multi-value"),
                    "expected multi-value-related error, got: {msg}"
                );
            }
            Ok(_) => {
                panic!(
                    "JIT compiled a 2-value return as single i64 — \
                     this is a miscompile. The multi-return guard was \
                     removed without implementing Bậc C packed ABI. \
                     The second return value would be silently dropped."
                );
            }
            Err(other) => {
                panic!(
                    "unexpected JIT error (expected Unsupported, got {other}) — \
                     if the guard was changed, verify multi-return still refuses"
                );
            }
        }
    }

    // ── Logic op truth table tests ─────────────────────────

    /// Trilean encoding: +1=True, 0=Unknown, -1=False.
    const T: i64 = 1;
    const U: i64 = 0;
    const F: i64 = -1;
    const ALL: [i64; 3] = [T, U, F];

    /// Build a MIR function `op(a, b)` that applies `binop` and returns the result.
    fn build_binop_tester(op: BinOp) -> Body {
        let mut b = MirBuilder::new(&format!("test_{op:?}"), "Integer");
        let a = b.add_param("a", ParameterPassing::Borrow);
        let b_param = b.add_param("b", ParameterPassing::Borrow);
        let result = b.new_local();
        let bb0 = b.new_block();
        b.push(bb0, storage_live(result));
        b.push(bb0, binop(result, op, a, b_param));
        b.set_terminator(bb0, return_(vec![result]));
        b.build(bb0)
    }

    /// JIT-compile a binop tester and call with (x, y).
    #[allow(unsafe_code)]
    fn call_binop(op: BinOp, x: i64, y: i64) -> i64 {
        let body = build_binop_tester(op);
        let mut ctx = JitContext::new();
        let func = ctx.compile(&body).expect("compile");
        unsafe { func.call_i64_2(x, y) }
    }

    // ── Łukasiewicz Ł3 And (min) ──

    #[test]
    #[allow(unsafe_code)]
    fn luk_and_truth_table() {
        // And = min(a, b): False dominates
        for a in ALL {
            for b in ALL {
                let expected = a.min(b);
                let got = call_binop(BinOp::LukAnd, a, b);
                assert_eq!(
                    got, expected,
                    "Ł3 And: {a} && {b} should be {expected}, got {got}"
                );
            }
        }
    }

    // ── Łukasiewicz Ł3 Or (max) ──

    #[test]
    #[allow(unsafe_code)]
    fn luk_or_truth_table() {
        // Or = max(a, b): True dominates
        for a in ALL {
            for b in ALL {
                let expected = a.max(b);
                let got = call_binop(BinOp::LukOr, a, b);
                assert_eq!(
                    got, expected,
                    "Ł3 Or: {a} || {b} should be {expected}, got {got}"
                );
            }
        }
    }

    // ── Łukasiewicz Ł3 Implies ──

    /// Expected Ł3 implies per triet-logic reference.
    fn expected_luk_implies(a: i64, b: i64) -> i64 {
        match (a, b) {
            (-1, _) => 1, // False → anything = True
            (1, x) => x,  // True → x = x
            (0, 1) => 1,  // Unknown → True = True
            (0, 0) => 1,  // Unknown → Unknown = True (Ł3 signature)
            (0, -1) => 0, // Unknown → False = Unknown
            _ => unreachable!(),
        }
    }

    #[test]
    #[allow(unsafe_code)]
    fn luk_implies_truth_table() {
        for a in ALL {
            for b in ALL {
                let expected = expected_luk_implies(a, b);
                let got = call_binop(BinOp::LukImplies, a, b);
                assert_eq!(
                    got, expected,
                    "Ł3 Implies: {a} => {b} should be {expected}, got {got}"
                );
            }
        }
    }

    // ── Łukasiewicz Ł3 Iff ──

    /// Expected Ł3 iff per triet-logic: (a→b) ∧ (b→a)
    fn expected_luk_iff(a: i64, b: i64) -> i64 {
        let ab = expected_luk_implies(a, b);
        let ba = expected_luk_implies(b, a);
        ab.min(ba) // And = min
    }

    #[test]
    #[allow(unsafe_code)]
    fn luk_iff_truth_table() {
        for a in ALL {
            for b in ALL {
                let expected = expected_luk_iff(a, b);
                let got = call_binop(BinOp::LukIff, a, b);
                assert_eq!(
                    got, expected,
                    "Ł3 Iff: {a} <=> {b} should be {expected}, got {got}"
                );
            }
        }
    }

    // ── Łukasiewicz Ł3 Xor ──

    /// Expected Ł3 xor = ¬(a↔b)
    fn expected_luk_xor(a: i64, b: i64) -> i64 {
        -expected_luk_iff(a, b) // negation
    }

    #[test]
    #[allow(unsafe_code)]
    fn luk_xor_truth_table() {
        for a in ALL {
            for b in ALL {
                let expected = expected_luk_xor(a, b);
                let got = call_binop(BinOp::LukXor, a, b);
                assert_eq!(
                    got, expected,
                    "Ł3 Xor: {a} ^ {b} should be {expected}, got {got}"
                );
            }
        }
    }

    // ── Kleene K3 Implies ──

    /// Expected K3 implies = max(-a, b)
    fn expected_kleene_implies(a: i64, b: i64) -> i64 {
        (-a).max(b)
    }

    #[test]
    #[allow(unsafe_code)]
    fn kleene_implies_truth_table() {
        for a in ALL {
            for b in ALL {
                let expected = expected_kleene_implies(a, b);
                let got = call_binop(BinOp::KleeneImplies, a, b);
                assert_eq!(
                    got, expected,
                    "K3 Implies: {a} ~> {b} should be {expected}, got {got}"
                );
            }
        }
    }

    // ── Verify Ł3 vs K3 differ ONLY at (Unknown, Unknown) ──

    #[test]
    #[allow(unsafe_code)]
    fn luk_vs_kleene_differs_only_at_unknown_unknown() {
        for a in ALL {
            for b in ALL {
                let luk = call_binop(BinOp::LukImplies, a, b);
                let kleene = call_binop(BinOp::KleeneImplies, a, b);
                if a == U && b == U {
                    assert_eq!(luk, T, "Ł3 U→U must be True");
                    assert_eq!(kleene, U, "K3 U→U must be Unknown");
                    assert_ne!(luk, kleene);
                } else {
                    assert_eq!(
                        luk, kleene,
                        "Ł3/K3 disagree at ({a}→{b}): Ł3={luk}, K3={kleene}"
                    );
                }
            }
        }
    }

    // ── Negation ──

    #[test]
    #[allow(unsafe_code)]
    fn neg_truth_table() {
        let mut b = MirBuilder::new("test_neg", "Integer");
        let a = b.add_param("a", ParameterPassing::Borrow);
        let result = b.new_local();
        let bb0 = b.new_block();
        b.push(bb0, storage_live(result));
        // Neg is a unary op — we model it via BinOp::Neg by ignoring the rhs
        // (lower_binop only uses lhs for Neg). Pass a dummy rhs.
        b.push(bb0, binop(result, BinOp::Neg, a, a)); // rhs ignored
        b.set_terminator(bb0, return_(vec![result]));
        let body = b.build(bb0);

        let mut ctx = JitContext::new();
        let func = ctx.compile(&body).expect("compile");

        assert_eq!(unsafe { func.call_i64_1(T) }, F, "neg True = False");
        assert_eq!(unsafe { func.call_i64_1(U) }, U, "neg Unknown = Unknown");
        assert_eq!(unsafe { func.call_i64_1(F) }, T, "neg False = True");
    }

    // ── Shim call tests ────────────────────────────────────

    #[test]
    #[allow(unsafe_code)]
    fn shim_call_multiply_via_jit() {
        // Build a JIT function that calls __test_shim_multiply via Shim
        let mut b = MirBuilder::new("test_shim_mul", "Integer");
        let a = b.add_param("a", ParameterPassing::Borrow);
        let b_param = b.add_param("b", ParameterPassing::Borrow);
        let result = b.new_local();
        let call_bb = b.new_block();
        let ret_bb = b.new_block();
        b.push(call_bb, storage_live(result));
        b.set_terminator(
            call_bb,
            Terminator::CallDispatch {
                callee: FunctionId(0),
                callee_name: "__test_shim_multiply".into(),
                target: CallTarget::Shim,
                args: vec![a, b_param],
                return_bb: ret_bb,
                dest: vec![result],
                span: DUMMY_SPAN,
            },
        );
        b.set_terminator(ret_bb, return_(vec![result]));
        let body = b.build(call_bb);

        let shims = &[ShimSymbol::fn_2_1(
            "__test_shim_multiply",
            super::__test_shim_multiply,
        )];

        let mut ctx = JitContext::with_shims(shims);
        let func = ctx.compile(&body).expect("shim JIT compile");
        let result = unsafe { func.call_i64_2(7, 9) };
        assert_eq!(
            result, 63,
            "__test_shim_multiply(7, 9) = 63 via JIT shim call"
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn shim_call_pow_via_jit() {
        // Build a JIT function that calls __triet_pow via Shim
        let mut b = MirBuilder::new("test_pow", "Integer");
        let base = b.add_param("base", ParameterPassing::Borrow);
        let exp = b.add_param("exp", ParameterPassing::Borrow);
        let result = b.new_local();
        let call_bb = b.new_block();
        let ret_bb = b.new_block();
        b.push(call_bb, storage_live(result));
        b.set_terminator(
            call_bb,
            Terminator::CallDispatch {
                callee: FunctionId(0),
                callee_name: "__triet_pow".into(),
                target: CallTarget::Shim,
                args: vec![base, exp],
                return_bb: ret_bb,
                dest: vec![result],
                span: DUMMY_SPAN,
            },
        );
        b.set_terminator(ret_bb, return_(vec![result]));
        let body = b.build(call_bb);

        let shims = &[ShimSymbol::fn_2_1("__triet_pow", super::__triet_pow)];

        let mut ctx = JitContext::with_shims(shims);
        let func = ctx.compile(&body).expect("pow shim JIT compile");

        assert_eq!(unsafe { func.call_i64_2(2, 10) }, 1024, "2^10 = 1024");
        assert_eq!(unsafe { func.call_i64_2(3, 5) }, 243, "3^5 = 243");
        assert_eq!(unsafe { func.call_i64_2(5, 0) }, 1, "5^0 = 1");
        assert_eq!(unsafe { func.call_i64_2(7, 1) }, 7, "7^1 = 7");
    }
}
