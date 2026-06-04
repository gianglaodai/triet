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
use cranelift_codegen::ir::types::{I64, I8};
use cranelift_codegen::ir::{AbiParam, InstBuilder, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use std::collections::{HashMap, HashSet};
use triet_mir::{BasicBlock, BinOp, Body, ConstValue, ControlFlowGraph, Local, ReturnShape, Statement, Terminator};

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
}

// ── JIT context ─────────────────────────────────────────────

/// Holds Cranelift JIT state across compilations.
pub struct JitContext {
    module: JITModule,
    /// Map from MIR Local to Cranelift Variable (discriminant for Outcome, value for scalars).
    locals: HashMap<Local, Variable>,
    /// Set of locals that hold Outcome values (need split disc+payload representation).
    outcome_locals: HashSet<Local>,
    /// Map from MIR `BasicBlock` to Cranelift Block.
    blocks: HashMap<BasicBlock, cranelift_codegen::ir::Block>,
    /// Blocks that have been sealed.
    sealed: HashSet<BasicBlock>,
    /// Blocks that have been filled.
    filled: HashSet<BasicBlock>,
    /// Map from function name → Cranelift FuncId (for cross-function calls).
    func_ids: HashMap<String, cranelift_module::FuncId>,
}

impl JitContext {
    /// Return the Cranelift Variable for the discriminant component of a Local.
    /// For Outcome locals: this is the disc (i8 extended to i64).
    /// For scalar locals: this is the only value.
    fn disc_var(&self, l: Local) -> Variable {
        Variable::from_u32(l.0 as u32 * 2)
    }

    /// Return the Cranelift Variable for the payload component of a Local.
    /// Only meaningful for Outcome locals.
    fn payload_var(&self, l: Local) -> Variable {
        Variable::from_u32(l.0 as u32 * 2 + 1)
    }

    /// Check if a local is Outcome-typed (has split disc+payload).
    fn is_outcome(&self, l: Local) -> bool {
        self.outcome_locals.contains(&l)
    }

    /// Create a new JIT context with host ISA detection.
    pub fn new() -> Self {
        let flag_builder = cranelift_codegen::settings::builder();
        let isa_builder = cranelift_native::builder()
            .expect("host ISA detection failed");
        let isa = isa_builder
            .finish(cranelift_codegen::settings::Flags::new(flag_builder))
            .expect("host ISA not supported");
        let mut jit_builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        // Register production shim symbols (so `__triet_*` calls resolve)
        register_shim_symbols(
            &mut jit_builder,
            &crate::shims::production_shim_entries(),
        );
        let module = JITModule::new(jit_builder);
        Self {
            module,
            locals: HashMap::new(),
            blocks: HashMap::new(),
            sealed: HashSet::new(),
            filled: HashSet::new(),
            func_ids: HashMap::new(),
            outcome_locals: HashSet::new(),
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
            sig.returns.push(AbiParam::new(I64));
            if body.signature.return_shape != ReturnShape::Scalar {
                sig.returns.push(AbiParam::new(I64)); // second return: payload
            }

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
            cl_ctx.func.signature.returns.push(AbiParam::new(I64));
            if body.signature.return_shape != ReturnShape::Scalar {
                cl_ctx.func.signature.returns.push(AbiParam::new(I64));
            }

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

        self.module.finalize_definitions().map_err(|e| {
            JitError::Module(format!("finalize: {e}"))
        })?;

        // ── Collect results ────────────────────────────────
        let mut result = HashMap::new();
        for body in bodies {
            let func_id = self.func_ids[&body.signature.name];
            let code_ptr = self.module.get_finalized_function(func_id);
            result.insert(
                body.signature.name.clone(),
                CompiledFunction { code_ptr },
            );
        }

        Ok(result)
    }

    /// Build the Cranelift IR for a single function body.
    fn build_body(
        &mut self,
        builder: &mut FunctionBuilder,
        body: &Body,
    ) -> Result<(), JitError> {
        let cfg = body.build_cfg();

        // ── Pre-scan: identify Outcome locals ─────────────────
        self.outcome_locals.clear();
        // Sources used in Outcome ops
        for block_data in &body.blocks {
            for stmt in &block_data.statements {
                if let Statement::OutcomeDiscriminant { source, .. }
                | Statement::OutcomeUnwrap { source, .. }
                | Statement::OutcomeUnwrapError { source, .. } = stmt {
                    self.outcome_locals.insert(*source);
                }
            }
        }
        // Return values of Outcome functions
        if body.signature.return_shape != ReturnShape::Scalar {
            for block_data in &body.blocks {
                if let Terminator::Return { values } = &block_data.terminator {
                    if !values.is_empty() {
                        self.outcome_locals.insert(values[0]);
                    }
                }
            }
        }

        // ── Declare variables (2× for Outcome split support) ──
        self.locals.clear();
        for i in 0..(body.num_locals * 2) {
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

        // Entry block: params go into disc_var slots
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        for (i, _) in body.signature.params.iter().enumerate() {
            let var = self.disc_var(Local(i));
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
        builder: &mut FunctionBuilder,
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
        builder: &mut FunctionBuilder,
        body: &Body,
        block: BasicBlock,
    ) -> Result<(), JitError> {
        let block_data = &body.blocks[block.0];
        for stmt in &block_data.statements {
            match stmt {
                Statement::StorageLive(_) | Statement::StorageDead(_) => {
                    // No-op at runtime — borrow checker verified safety
                }

                Statement::Const { dest, value } => {
                    let val = match value {
                        ConstValue::Integer(n) => {
                            let n_i64 = i64::try_from(*n).unwrap_or(0);
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
                    let var = self.disc_var(*dest);
                    builder.def_var(var, val);
                    // If Outcome local, also define payload (default 0)
                    if self.is_outcome(*dest) {
                        let zero = builder.ins().iconst(I64, 0);
                        builder.def_var(self.payload_var(*dest), zero);
                    }
                }

                Statement::Assign { dest, source } => {
                    let src_var = self.disc_var(*source);
                    let val = builder.use_var(src_var);
                    let dest_var = self.disc_var(*dest);
                    builder.def_var(dest_var, val);
                }

                Statement::Borrow { dest, source, .. } => {
                    // S6 references = raw pointers at runtime — just copy
                    let src_var = self.disc_var(*source);
                    let val = builder.use_var(src_var);
                    let dest_var = self.disc_var(*dest);
                    builder.def_var(dest_var, val);
                }

                Statement::BinaryOp { dest, op, left, right } => {
                    let l_var = self.disc_var(*left);
                    let r_var = self.disc_var(*right);
                    let lhs = builder.use_var(l_var);
                    let rhs = builder.use_var(r_var);

                    let result = match op {
                        BinOp::Add => builder.ins().iadd(lhs, rhs),
                        BinOp::Sub => builder.ins().isub(lhs, rhs),
                        BinOp::Mul => builder.ins().imul(lhs, rhs),
                        BinOp::Gt | BinOp::Le => {
                            let cc = match op {
                                BinOp::Gt => IntCC::SignedGreaterThan,
                                BinOp::Le => IntCC::SignedLessThanOrEqual,
                                _ => unreachable!(),
                            };
                            let cmp = builder.ins().icmp(cc, lhs, rhs);
                            let one = builder.ins().iconst(I8, 1);
                            let neg_one = builder.ins().iconst(I8, -1_i64);
                            let trilean = builder.ins().select(cmp, one, neg_one);
                            builder.ins().sextend(I64, trilean)
                        }
                    };
                    let dest_var = self.disc_var(*dest);
                    builder.def_var(dest_var, result);
                }

                Statement::OutcomeDiscriminant { dest, source } => {
                    let disc_val = builder.use_var(self.disc_var(*source));
                    builder.def_var(self.disc_var(*dest), disc_val);
                }

                Statement::OutcomeUnwrap { dest, source } => {
                    let payload_val = builder.use_var(self.payload_var(*source));
                    builder.def_var(self.disc_var(*dest), payload_val);
                }

                Statement::OutcomeUnwrapError { dest, source } => {
                    let payload_val = builder.use_var(self.payload_var(*source));
                    builder.def_var(self.disc_var(*dest), payload_val);
                }

                Statement::Drop(_) => {
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
        builder: &mut FunctionBuilder,
        body: &Body,
        block: BasicBlock,
    ) -> Result<(), JitError> {
        let block_data = &body.blocks[block.0];

        match &block_data.terminator {
            Terminator::Return { values } => {
                if values.is_empty() {
                    let zero = builder.ins().iconst(I64, 0);
                    builder.ins().return_(&[zero]);
                } else if values.len() == 1 {
                    let val = builder.use_var(self.disc_var(values[0]));
                    builder.ins().return_(&[val]);
                } else {
                    // Outcome: 2 values — disc + payload
                    let disc = builder.use_var(self.disc_var(values[0]));
                    let payload = builder.use_var(self.payload_var(values[1]));
                    builder.ins().return_(&[disc, payload]);
                }
            }

            Terminator::Goto { target } => {
                let target_block = self.blocks[target];
                builder.ins().jump(target_block, &[]);
            }

            Terminator::If {
                cond,
                positive_bb,
                zero_bb,
                negative_bb,
            } => {
                let cond_var = self.disc_var(*cond);
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
                    builder.ins().brif(is_zero, zero_block, &[], fallthrough, &[]);

                    builder.switch_to_block(fallthrough);
                    let is_pos = builder.ins().icmp(IntCC::SignedGreaterThan, cond_val, zero_val);
                    builder.ins().brif(is_pos, pos_block, &[], neg_block, &[]);
                } else {
                    // Binary branch (if): 2-way
                    let is_pos = builder.ins().icmp(IntCC::SignedGreaterThan, cond_val, zero_val);
                    builder.ins().brif(is_pos, pos_block, &[], neg_block, &[]);
                }
            }

            Terminator::CallDispatch {
                callee_name,
                args,
                return_bb,
                dest,
                ..
            } => {
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
                        let var = self.disc_var(*a);
                        builder.use_var(var)
                    })
                    .collect();

                // Emit call
                let call_inst = builder.ins().call(func_ref, &arg_vals);

                // Store return values into dest locals.
                // dest.len() matches callee's ReturnShape::arity():
                //   Unit → empty, Scalar → 1, Outcome → 2 (disc, payload)
                if dest.len() == 1 {
                    let ret_val = builder.inst_results(call_inst)[0];
                    builder.def_var(self.disc_var(dest[0]), ret_val);
                } else if dest.len() == 2 {
                    let ret_disc = builder.inst_results(call_inst)[0];
                    let ret_payload = builder.inst_results(call_inst)[1];
                    builder.def_var(self.disc_var(dest[0]), ret_disc);
                    builder.def_var(self.payload_var(dest[1]), ret_payload);
                }

                // Jump to return block
                let ret_block = self.blocks[return_bb];
                builder.ins().jump(ret_block, &[]);
            }

            Terminator::Unreachable => {
                builder.ins().trap(cranelift_codegen::ir::TrapCode::unwrap_user(0));
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

// ── Helpers ─────────────────────────────────────────────────

/// Register production shim symbols so that calls to `__triet_*` resolve.
fn register_shim_symbols(
    builder: &mut JITBuilder,
    entries: &[crate::shims::ShimEntry],
) {
    for entry in entries {
        builder.symbol(entry.symbol, entry.addr as *const u8);
    }
}

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

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use triet_borrowck::{binop, return_, storage_live, MirBuilder};
    use triet_mir::ParameterPassing;

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
        b.push(bb0, triet_mir::Statement::Const { dest: one, value: triet_mir::ConstValue::Integer(1) });
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
        b.push(bb3, triet_mir::Statement::Const { dest: two, value: triet_mir::ConstValue::Integer(2) });
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
                positive_bb: bb1,   // n <= 1 → return n
                zero_bb: None,
                negative_bb: bb2,   // n > 1 → compute
            },
        );
        let fib_id = b.func_id_for("fibonacci");
        b.set_terminator(
            bb2,
            Terminator::CallDispatch {
                callee: fib_id,
                callee_name: "fibonacci".into(),
                args: vec![tmp1],
                return_bb: bb3,
                dest: vec![call1_result],
            },
        );
        b.set_terminator(
            bb3,
            Terminator::CallDispatch {
                callee: fib_id,
                callee_name: "fibonacci".into(),
                args: vec![tmp2],
                return_bb: bb4,
                dest: vec![call2_result],
            },
        );
        b.set_terminator(bb4, Terminator::Goto { target: bb5 });

        let body = b.build(bb0);
        println!("=== MIR (fibonacci) ===\n{body}");

        let mut ctx = JitContext::new();
        let compiled = ctx
            .compile_multi(&[&body])
            .expect("JIT compilation failed");
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

    /// Outcome ops: compile a function that uses `OutcomeDiscriminant`
    /// to read the disc component of a split Outcome local.
    #[test]
    #[allow(unsafe_code)]
    fn outcome_discriminant_jit() {
        // Function that creates an "Outcome" value (disc via Const) and reads
        // the discriminant back. This exercises the split variable mapping.
        let mut b = MirBuilder::new("outcome_disc_test", "Integer");
        let _dummy = b.add_param("dummy", ParameterPassing::Borrow);
        let outcome_val = b.new_local(); // will be Outcome source
        let disc_result = b.new_local();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(outcome_val));
        b.push(bb0, triet_mir::Statement::Const {
            dest: outcome_val, value: ConstValue::Integer(1),
        });
        b.push(bb0, storage_live(disc_result));
        // This marks outcome_val as Outcome source in pre-scan
        b.push(bb0, triet_mir::Statement::OutcomeDiscriminant {
            dest: disc_result,
            source: outcome_val,
        });
        b.set_terminator(bb0, Terminator::Return { values: vec![disc_result] });

        let body = b.build(bb0);
        println!("=== MIR ===\n{body}");

        let mut ctx = JitContext::new();
        let func = ctx.compile(&body).expect("Outcome compilation failed");

        let result = unsafe { func.call_i64_1(0) };
        println!("outcome_discriminant(const 1) = {result}");
        assert_eq!(result, 1, "OutcomeDiscriminant should return the disc value");
    }

    /// Outcome ABI: full caller-callee with Outcome return.
    /// Callee returns Outcome (2-value), caller calls and extracts disc.
    #[test]
    #[allow(unsafe_code)]
    fn outcome_caller_callee_jit() {
        // ── Callee ───────────────────────────────────────────
        let mut callee = MirBuilder::new("make_outcome", "Outcome");
        callee.set_return_shape(triet_mir::ReturnShape::BinaryOutcome);
        let _dummy = callee.add_param("dummy", ParameterPassing::Borrow);
        let disc_val = callee.new_local();
        let payload_val = callee.new_local();

        let bb0 = callee.new_block();
        callee.push(bb0, storage_live(disc_val));
        callee.push(bb0, triet_mir::Statement::Const {
            dest: disc_val, value: ConstValue::Integer(1),  // disc = +1 (success)
        });
        callee.push(bb0, storage_live(payload_val));
        callee.push(bb0, triet_mir::Statement::Const {
            dest: payload_val, value: ConstValue::Integer(42), // payload = 42
        });
        callee.set_terminator(bb0, Terminator::Return { values: vec![disc_val, payload_val] });
        let callee_body = callee.build(bb0);

        // ── Caller ───────────────────────────────────────────
        let mut caller = MirBuilder::new("call_outcome", "Integer");
        let disc_out = caller.new_local();
        let payload_out = caller.new_local();

        let c_bb0 = caller.new_block();
        caller.push(c_bb0, storage_live(disc_out));
        caller.push(c_bb0, storage_live(payload_out));

        // Return the disc directly — it's already the first return value
        let c_bb1 = caller.new_block();
        caller.set_terminator(c_bb1, Terminator::Return { values: vec![disc_out] });

        let make_id = caller.func_id_for("make_outcome");
        let dummy_arg = caller.new_local();
        caller.push(c_bb0, storage_live(dummy_arg));
        caller.push(c_bb0, triet_mir::Statement::Const { dest: dummy_arg, value: ConstValue::Integer(0) });
        caller.set_terminator(c_bb0, Terminator::CallDispatch {
            callee: make_id,
            callee_name: "make_outcome".into(),
            args: vec![dummy_arg],
            return_bb: c_bb1,
            dest: vec![disc_out, payload_out],
        });
        let caller_body = caller.build(c_bb0);

        // Compile both
        let mut ctx = JitContext::new();
        let compiled = ctx.compile_multi(&[&callee_body, &caller_body])
            .expect("Outcome compilation failed");
        let func = compiled.get("call_outcome").expect("call_outcome");

        let result = unsafe { func.call_i64_1(0) };
        println!("call_outcome() disc = {result}");
        assert_eq!(result, 1, "discriminant should be 1");
    }
}
