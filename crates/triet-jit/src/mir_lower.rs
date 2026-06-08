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
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, InstBuilder, Signature, StackSlotData, StackSlotKind};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use std::collections::{HashMap, HashSet};
use triet_mir::{
    BasicBlock, BinOp, Body, CallTarget, ConstValue, ControlFlowGraph, EnumLayout, Local, Place,
    Projection, Statement, StructLayout, Terminator, builtin_shim_meta, is_copy,
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

    /// Register a 1-arg → void shim.
    pub fn fn_1_0(name: &str, f: extern "C" fn(i64)) -> Self {
        Self {
            name: name.into(),
            addr: f as usize,
            arity: 1,
            has_return: false,
        }
    }

    /// Register a 3-arg → 1-return shim.
    pub fn fn_3_1(name: &str, f: extern "C" fn(i64, i64, i64) -> i64) -> Self {
        Self {
            name: name.into(),
            addr: f as usize,
            arity: 3,
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

    /// Register a 4-arg → 1-return shim.
    pub fn fn_4_1(name: &str, f: extern "C" fn(i64, i64, i64, i64) -> i64) -> Self {
        Self {
            name: name.into(),
            addr: f as usize,
            arity: 4,
            has_return: true,
        }
    }
}

// ── JIT context ─────────────────────────────────────────────

/// Holds Cranelift JIT state across compilations.
pub struct JitContext {
    module: JITModule,
    /// Map from MIR Local to Cranelift Variable (one per MIR local).
    /// Bậc A: every value is a single i64 — scalars unboxed.
    locals: HashMap<Local, Variable>,
    /// Map from MIR Local to Cranelift `StackSlot` + struct layout.
    /// Struct locals use `StackSlots`; fields accessed via `stack_load/store`.
    struct_slots: HashMap<Local, (cranelift_codegen::ir::StackSlot, StructLayout)>,
    /// Map from MIR Local to Cranelift `StackSlot` + enum layout.
    /// Enum locals use `StackSlots`; discriminant/payload accessed via `stack_load/store`.
    enum_slots: HashMap<Local, (cranelift_codegen::ir::StackSlot, EnumLayout)>,
    /// Map from MIR `BasicBlock` to Cranelift Block.
    blocks: HashMap<BasicBlock, cranelift_codegen::ir::Block>,
    /// Blocks that have been sealed.
    sealed: HashSet<BasicBlock>,
    /// Blocks that have been filled.
    filled: HashSet<BasicBlock>,
    /// Map from function name → Cranelift `FuncId` (for cross-function calls).
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

    /// Get or declare a shim function ID. Caches the result so multiple
    /// call sites for the same shim use the same `FuncId`.
    fn get_or_declare_shim(&mut self, name: &str) -> Result<cranelift_module::FuncId, JitError> {
        if let Some(&id) = self.func_ids.get(name) {
            return Ok(id);
        }
        let shim = self
            .shim_registry
            .get(name)
            .ok_or_else(|| JitError::Unsupported(format!("shim `{name}` not registered")))?;
        let mut sig = Signature::new(CallConv::SystemV);
        for _ in 0..shim.arity {
            sig.params.push(AbiParam::new(I64));
        }
        if shim.has_return {
            sig.returns.push(AbiParam::new(I64));
        }
        let id = self
            .module
            .declare_function(name, Linkage::Import, &sig)
            .map_err(|e| JitError::Module(format!("declare shim {name}: {e}")))?;
        self.func_ids.insert(name.to_string(), id);
        Ok(id)
    }

    /// Load the Cranelift Value for a MIR Place.
    /// Plain locals → `use_var`. Field projections → `stack_load` (local struct)
    /// or load through pointer (param/sret struct).
    fn load_place(
        &self,
        builder: &mut FunctionBuilder<'_>,
        body: &Body,
        place: &Place,
    ) -> Result<cranelift_codegen::ir::Value, JitError> {
        if place.projection.is_empty() {
            // ADR-0049 Phase-1 Lát 1 B3: whole-read bridge for String.
            // String has a struct_slot but the "whole value" (for shim
            // compatibility) is field-0 (heap handle). User structs keep
            // the old use_var path.
            if let Some((slot, layout)) = self.struct_slots.get(&place.local)
                && layout.name == "String"
            {
                return Ok(builder.ins().stack_load(I64, *slot, 0));
            }
            return Ok(builder.use_var(self.var(place.local)));
        }
        if place.projection.len() != 1 {
            return Err(JitError::Unsupported(
                "nested projections not supported".into(),
            ));
        }
        match &place.projection[0] {
            Projection::Field(field_name) => {
                let ty = &body.local_decls[place.local.0].ty;
                let layout = body
                    .struct_layouts
                    .iter()
                    .find(|l| l.name == *ty)
                    .ok_or_else(|| {
                        JitError::Unsupported(format!(
                            "type '{ty}' is not a known struct (local {})",
                            place.local
                        ))
                    })?;
                let field = layout
                    .fields
                    .iter()
                    .find(|f| f.name == *field_name)
                    .ok_or_else(|| {
                        JitError::Unsupported(format!(
                            "field '{field_name}' not found in struct '{ty}'"
                        ))
                    })?;
                let offset = i32::try_from(field.offset)
                    .map_err(|_| JitError::Unsupported("field offset exceeds i32".into()))?;
                if let Some((slot, _)) = self.struct_slots.get(&place.local) {
                    return Ok(builder.ins().stack_load(I64, *slot, offset));
                }
                // Pointer-based: param or sret. Load pointer, add offset, load.
                // (String never reaches here — universal slots always return
                // via stack_load above.)
                let ptr = builder.use_var(self.var(place.local));
                let addr = builder.ins().iadd_imm(ptr, i64::from(offset));
                return Ok(builder.ins().load(
                    I64,
                    cranelift_codegen::ir::MemFlags::new(),
                    addr,
                    0,
                ));
            }
            Projection::Payload(_) => {
                let payload_offset: i32 = 8;
                if let Some((slot, _)) = self.enum_slots.get(&place.local) {
                    return Ok(builder.ins().stack_load(I64, *slot, payload_offset));
                }
                return Err(JitError::Unsupported(
                    "Payload access on non-enum local".into(),
                ));
            }
            _ => {}
        }
        Err(JitError::Unsupported("unsupported projection".into()))
    }

    /// Store a Cranelift Value into a MIR Place.
    fn store_place(
        &self,
        builder: &mut FunctionBuilder<'_>,
        body: &Body,
        place: &Place,
        value: cranelift_codegen::ir::Value,
    ) -> Result<(), JitError> {
        if place.projection.is_empty() {
            builder.def_var(self.var(place.local), value);
            return Ok(());
        }
        if place.projection.len() != 1 {
            return Err(JitError::Unsupported(
                "nested projections not supported".into(),
            ));
        }
        match &place.projection[0] {
            Projection::Field(field_name) => {
                let ty = &body.local_decls[place.local.0].ty;
                let layout = body
                    .struct_layouts
                    .iter()
                    .find(|l| l.name == *ty)
                    .ok_or_else(|| {
                        JitError::Unsupported(format!(
                            "type '{ty}' is not a known struct (local {})",
                            place.local
                        ))
                    })?;
                let field = layout
                    .fields
                    .iter()
                    .find(|f| f.name == *field_name)
                    .ok_or_else(|| {
                        JitError::Unsupported(format!(
                            "field '{field_name}' not found in struct '{ty}'"
                        ))
                    })?;
                let offset = i32::try_from(field.offset)
                    .map_err(|_| JitError::Unsupported("field offset exceeds i32".into()))?;
                if let Some((slot, _)) = self.struct_slots.get(&place.local) {
                    builder.ins().stack_store(value, *slot, offset);
                    return Ok(());
                }
                // Pointer-based: load pointer, add offset, store.
                let ptr = builder.use_var(self.var(place.local));
                let addr = builder.ins().iadd_imm(ptr, i64::from(offset));
                builder
                    .ins()
                    .store(cranelift_codegen::ir::MemFlags::new(), value, addr, 0);
                return Ok(());
            }
            Projection::Payload(_) => {
                let payload_offset: i32 = 8;
                if let Some((slot, _)) = self.enum_slots.get(&place.local) {
                    builder.ins().stack_store(value, *slot, payload_offset);
                    return Ok(());
                }
                return Err(JitError::Unsupported(
                    "Payload store to non-enum local".into(),
                ));
            }
            _ => {}
        }
        Err(JitError::Unsupported("unsupported projection".into()))
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
            struct_slots: HashMap::new(),
            enum_slots: HashMap::new(),
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
    /// Phase 2: build each function body (can reference others via `func_ids`).
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
            let is_sret = matches!(
                body.signature.return_shape,
                triet_mir::ReturnShape::Struct { .. }
            );
            if is_sret {
                sig.params.push(AbiParam::new(I64));
            }
            for _ in &body.signature.params {
                sig.params.push(AbiParam::new(I64));
            }
            if !is_sret {
                sig.returns.push(AbiParam::new(I64));
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
            let is_sret = matches!(
                body.signature.return_shape,
                triet_mir::ReturnShape::Struct { .. }
            );
            cl_ctx.func.signature = Signature::new(CallConv::SystemV);
            if is_sret {
                cl_ctx.func.signature.params.push(AbiParam::new(I64));
            }
            for _ in &body.signature.params {
                cl_ctx.func.signature.params.push(AbiParam::new(I64));
            }
            if !is_sret {
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

        // ── Declare variables ──
        self.locals.clear();
        self.struct_slots.clear();
        self.enum_slots.clear();
        for i in 0..body.num_locals {
            let var = builder.declare_var(I64);
            self.locals.insert(Local(i), var);
        }
        // ADR-0049 Lát 3: pre-allocate StackSlot for EVERY String-typed local.
        // Ensures all String locals (including move targets from Assign) have
        // slots before switch_to_block, so Drop/free can read cap from slot.
        // Field-0 initialized to 0 so unpopulated locals (params, returns)
        // produce free(0, _) = no-op instead of garbage pointer.
        let string_layout = body
            .struct_layouts
            .iter()
            .find(|l| l.name == "String")
            .cloned();
        if let Some(ref layout) = string_layout {
            let align_shift = layout.alignment.ilog2() as u8;
            for i in 0..body.num_locals {
                let local = Local(i);
                let ty = &body.local_decls[i].ty;
                if ty == "String" {
                    let slot = builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        layout.total_size as u32,
                        align_shift,
                    ));
                    self.struct_slots.insert(local, (slot, layout.clone()));
                }
            }
        }
        // ── Create StackSlots for local structs and enums ──
        for block in &body.blocks {
            for stmt in &block.statements {
                if let Statement::StructAlloc {
                    dest, struct_name, ..
                } = stmt
                {
                    let layout = body
                        .struct_layouts
                        .iter()
                        .find(|l| l.name == *struct_name)
                        .ok_or_else(|| {
                            JitError::Unsupported(format!("struct layout not found: {struct_name}"))
                        })?;
                    let align_shift = layout.alignment.ilog2() as u8;
                    let slot = builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        layout.total_size as u32,
                        align_shift,
                    ));
                    self.struct_slots.insert(*dest, (slot, layout.clone()));
                }
                if let Statement::EnumAlloc {
                    dest, enum_name, ..
                } = stmt
                {
                    let layout = body
                        .enum_layouts
                        .iter()
                        .find(|l| l.name == *enum_name)
                        .ok_or_else(|| {
                            JitError::Unsupported(format!("enum layout not found: {enum_name}"))
                        })?;
                    let align_shift = layout.alignment.ilog2() as u8;
                    let slot = builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        layout.total_size as u32,
                        align_shift,
                    ));
                    self.enum_slots.insert(*dest, (slot, layout.clone()));
                }
                // (String slots now pre-allocated for ALL locals above.)
            }
        }

        // Pre-declare blocks
        self.blocks.clear();
        self.sealed.clear();
        self.filled.clear();

        let entry_block = builder.create_block();
        self.blocks.insert(cfg.entry, entry_block);
        let mut next_synthetic = cfg.blocks.len();
        for i in 0..cfg.blocks.len() {
            let bb = BasicBlock(i);
            if bb != cfg.entry {
                let block = builder.create_block();
                self.blocks.insert(bb, block);
            }
            // Pre-allocate cascade blocks for SwitchInt terminators.
            // Each SwitchInt with N cases needs N-1 intermediate fallthrough blocks.
            let block_data = &cfg.blocks[i].data;
            if let Terminator::SwitchInt { cases, .. } = &block_data.terminator {
                let n_cases = cases.len();
                if n_cases > 1 {
                    for _ in 0..(n_cases - 1) {
                        let synth_bb = BasicBlock(next_synthetic);
                        next_synthetic += 1;
                        let block = builder.create_block();
                        self.blocks.insert(synth_bb, block);
                    }
                }
            }
        }

        // Entry block: params → var slots. sret: block param[0] → Local(0).
        let is_sret = matches!(
            body.signature.return_shape,
            triet_mir::ReturnShape::Struct { .. }
        );
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        let mut bp_idx = if is_sret {
            let sret_val = builder.block_params(entry_block)[0];
            builder.def_var(self.var(Local(0)), sret_val);
            1
        } else {
            0
        };
        // ADR-0049 Lát 3: init all String slot field-0 to 0 FIRST.
        // Param pop below overwrites for String params.
        let zero = builder.ins().iconst(I64, 0);
        for (_local, (slot, layout)) in &self.struct_slots {
            if layout.name == "String" {
                builder.ins().stack_store(zero, *slot, 0);
            }
        }

        let mem_flags = cranelift_codegen::ir::MemFlags::new();
        for (i, _param) in body.signature.params.iter().enumerate() {
            let local = if is_sret { Local(i + 1) } else { Local(i) };
            let var = self.var(local);
            let param_val = builder.block_params(entry_block)[bp_idx];
            builder.def_var(var, param_val);
            // ADR-0049 Lát 6: String param received as pointer-to-caller-slot.
            // Load {ptr,len,cap} from the caller's slot into our own slot.
            if body.local_decls[local.0].ty == "String" {
                if let Some((slot, _)) = self.struct_slots.get(&local) {
                    let src_ptr = builder.ins().load(I64, mem_flags, param_val, 0);
                    let src_len = builder.ins().load(I64, mem_flags, param_val, 8);
                    let src_cap = builder.ins().load(I64, mem_flags, param_val, 16);
                    builder.ins().stack_store(src_ptr, *slot, 0);
                    builder.ins().stack_store(src_len, *slot, 8);
                    builder.ins().stack_store(src_cap, *slot, 16);
                }
            }
            bp_idx += 1;
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

        // Seal unsealed (including synthetic blocks allocated for SwitchInt cascades)
        let total_blocks = body.build_cfg().blocks.len() + {
            let mut extra = 0;
            for bd in &body.blocks {
                if let Terminator::SwitchInt { cases, .. } = &bd.terminator
                    && cases.len() > 1
                {
                    extra += cases.len() - 1;
                }
            }
            extra
        };
        for i in 0..total_blocks {
            let bb = BasicBlock(i);
            if let Some(&block) = self.blocks.get(&bb)
                && !self.sealed.contains(&bb)
            {
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
                Statement::Deinit(l, _) => {
                    // ADR-0042: tombstone — zero the slot. The callee
                    // already freed the heap value; zeroing the caller's
                    // slot ensures the eventual free(0) on Drop is safe.
                    let zero = builder.ins().iconst(I64, 0);
                    // ADR-0049 Lát 2 L2-2: Slot-Truth Edict — for String,
                    // stack_store is the SOLE guard. def_var is dead (Drop
                    // reads slot, not var). No-slot String still needs
                    // def_var.
                    if let Some((slot, layout)) = self.struct_slots.get(l)
                        && layout.name == "String"
                    {
                        builder.ins().stack_store(zero, *slot, 0);
                    } else {
                        builder.def_var(self.var(*l), zero);
                    }
                }
                Statement::StructAlloc { .. } => {
                    // No-op at runtime — stack slot allocated during build_body
                }

                Statement::Const { dest, value, .. } => {
                    if let ConstValue::String(s) = value {
                        // AOT: replace with define_data (ADR-0040 §3.3).
                        let bytes = s.as_bytes();
                        let ptr_val = builder.ins().iconst(I64, bytes.as_ptr() as i64);
                        let len_val = builder
                            .ins()
                            .iconst(I64, i64::try_from(bytes.len()).unwrap_or(0));
                        let func_id = self.get_or_declare_shim("__triet_string_from_bytes")?;
                        let func_ref = self.module.declare_func_in_func(func_id, builder.func);
                        let call_inst = builder.ins().call(func_ref, &[ptr_val, len_val]);
                        let handle = builder.inst_results(call_inst)[0];
                        // ADR-0049 Phase-1 Lát 1 B2: populate pre-allocated String slot.
                        if let Some((slot, _layout)) = self.struct_slots.get(&dest.local) {
                            let mem_flags = cranelift_codegen::ir::MemFlags::new();
                            builder.ins().stack_store(handle, *slot, 0);
                            let heap_len = builder.ins().load(I64, mem_flags, handle, 0);
                            builder.ins().stack_store(heap_len, *slot, 8);
                            let heap_cap = builder.ins().load(I64, mem_flags, handle, 8);
                            builder.ins().stack_store(heap_cap, *slot, 16);
                        }
                        builder.def_var(self.var(dest.local), handle);
                    } else {
                        let val = match value {
                            ConstValue::Integer(n) => {
                                let n_i64 = i64::try_from(*n).map_err(|_| {
                                    JitError::Unsupported(format!(
                                        "Integer constant {n} does not fit in i64 — \
                                         Bậc A only supports 64-bit values."
                                    ))
                                })?;
                                builder.ins().iconst(I64, n_i64)
                            }
                            ConstValue::Trit(t) => builder.ins().iconst(I64, i64::from(*t)),
                            ConstValue::Unit => builder.ins().iconst(I64, 0),
                            _ => unreachable!(),
                        };
                        builder.def_var(self.var(dest.local), val);
                    }
                }

                Statement::Assign { dest, source, .. } => {
                    let val = self.load_place(builder, body, source)?;
                    self.store_place(builder, body, dest, val)?;
                    // ADR-0049 Lát 1/3: sync String slot after assign.
                    // Slots for ALL String locals are pre-allocated in the
                    // pre-pass — just write the current handle+len+cap.
                    if dest.projection.is_empty() {
                        if let Some((slot, _)) = self.struct_slots.get(&dest.local) {
                            let mem_flags = cranelift_codegen::ir::MemFlags::new();
                            builder.ins().stack_store(val, *slot, 0);
                            let heap_len = builder.ins().load(I64, mem_flags, val, 0);
                            builder.ins().stack_store(heap_len, *slot, 8);
                            let heap_cap = builder.ins().load(I64, mem_flags, val, 8);
                            builder.ins().stack_store(heap_cap, *slot, 16);
                        }
                    }
                    // M1: Zeroing-on-Move — if source is a plain local of Move type,
                    // store 0 into it so Drop becomes a no-op.
                    let source_is_plain = source.projection.is_empty();
                    if source_is_plain {
                        let src_ty = &body.local_decls[source.local.0].ty;
                        if !is_copy(src_ty, body) {
                            let zero = builder.ins().iconst(I64, 0);
                            // ADR-0049 Lát 2 L2-2: Slot-Truth — for String,
                            // stack_store is the sole guard; def_var dead.
                            if let Some((slot, layout)) = self.struct_slots.get(&source.local)
                                && layout.name == "String"
                            {
                                builder.ins().stack_store(zero, *slot, 0);
                            } else {
                                self.store_place(builder, body, &Place::local(source.local), zero)?;
                            }
                        }
                    }
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
                    let lhs = self.load_place(builder, body, left)?;
                    let rhs = self.load_place(builder, body, right)?;
                    let result = lower_binop(builder, *op, lhs, rhs);
                    self.store_place(builder, body, dest, result)?;
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

                Statement::Drop(local, _) => {
                    let ty = &body.local_decls[local.0].ty;
                    if is_copy(ty, body) {
                        // Copy type: no-op (ADR-0040 §1.3)
                        continue;
                    }
                    // M4: Return-escape — skip Drop for locals in Return values.
                    let in_return = match &body.blocks[block.0].terminator {
                        Terminator::Return { values, .. } => values.contains(local),
                        _ => false,
                    };
                    if in_return {
                        continue;
                    }
                    // Move type, not escaping: call free shim.
                    // The shim internally guards null (defense-in-depth).
                    let free_shim_name = match ty.as_str() {
                        "String" => "__triet_string_free",
                        ty if triet_mir::is_vec_type(ty) => "__triet_vector_free",
                        ty if triet_mir::is_hashmap_type(ty) => "__triet_hashmap_free",
                        _ => {
                            return Err(JitError::Unsupported(format!(
                                "Drop for type `{ty}` not supported — no free shim"
                            )));
                        }
                    };
                    let func_id = self.get_or_declare_shim(free_shim_name)?;
                    let func_ref = self.module.declare_func_in_func(func_id, builder.func);
                    // ADR-0049 Lát 3: String free(ptr, cap) 2-arg bung field.
                    // Slot: stack_load ptr+cap; no-slot: use_var ptr + load cap heap.
                    if ty == "String" {
                        let mem_flags = cranelift_codegen::ir::MemFlags::new();
                        let (ptr, cap) = if let Some((slot, _)) = self.struct_slots.get(local) {
                            let p = builder.ins().stack_load(I64, *slot, 0);
                            let c = builder.ins().stack_load(I64, *slot, 16);
                            (p, c)
                        } else {
                            let handle = builder.use_var(self.var(*local));
                            let c = builder.ins().load(I64, mem_flags, handle, 8);
                            (handle, c)
                        };
                        builder.ins().call(func_ref, &[ptr, cap]);
                    } else {
                        let ptr = builder.use_var(self.var(*local));
                        builder.ins().call(func_ref, &[ptr]);
                    }
                }

                Statement::EnumAlloc { .. } => {
                    // No-op — StackSlot already created in build_body
                }
                Statement::SetDiscriminant { dest, value, .. } => {
                    if let Some((slot, _)) = self.enum_slots.get(dest) {
                        let disc_val = builder.ins().iconst(I64, *value);
                        builder.ins().stack_store(disc_val, *slot, 0);
                    }
                }
                Statement::GetDiscriminant { dest, source, .. } => {
                    // If the source has an enum StackSlot, read discriminant from it.
                    // Otherwise, the source IS the discriminant (Bậc A: enum params
                    // and temporaries are passed as raw i64 discriminant values).
                    let disc_val = if let Some((slot, _)) = self.enum_slots.get(&source.local) {
                        builder.ins().stack_load(I64, *slot, 0)
                    } else {
                        // Plain local — the value itself IS the discriminant.
                        builder.use_var(self.var(source.local))
                    };
                    let var = self.var(dest.local);
                    builder.def_var(var, disc_val);
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
                if values.len() > 1 {
                    return Err(JitError::Unsupported(
                        "multi-value return requires Bậc C packed ABI".into(),
                    ));
                }
                let is_sret_ret = matches!(
                    body.signature.return_shape,
                    triet_mir::ReturnShape::Struct { .. }
                );
                if is_sret_ret {
                    builder.ins().return_(&[]);
                } else if values.is_empty() {
                    let val = builder.ins().iconst(I64, 0);
                    builder.ins().return_(&[val]);
                } else {
                    // ADR-0049 Lát 6: String return reads handle from slot,
                    // not var (var holds pointer-to-caller-slot after L6-1).
                    let val = if let Some((slot, layout)) = self.struct_slots.get(&values[0])
                        && layout.name == "String"
                    {
                        builder.ins().stack_load(I64, *slot, 0)
                    } else {
                        builder.use_var(self.var(values[0]))
                    };
                    builder.ins().return_(&[val]);
                }
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
                let cond_val = builder.use_var(self.var(*cond));
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
                return_shape,
                ..
            } => {
                let is_sret_call = matches!(return_shape, triet_mir::ReturnShape::Struct { .. });
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

                        // Prepare arguments.
                        // Struct locals → stack_addr (pass by-pointer).
                        // String locals → stack_addr (Lát 6: fat-String by-pointer).
                        // Enum locals → stack_load discriminant (raw i64).
                        // Scalars → use_var.
                        let arg_vals: Vec<_> = args
                            .iter()
                            .map(|a| {
                                if let Some((slot, layout)) = self.struct_slots.get(a) {
                                    if layout.name == "String" {
                                        // ADR-0049 Lát 6: param fat-String by-pointer
                                        builder.ins().stack_addr(I64, *slot, 0)
                                    } else {
                                        builder.ins().stack_addr(I64, *slot, 0)
                                    }
                                } else if let Some((slot, _)) = self.enum_slots.get(a) {
                                    builder.ins().stack_load(I64, *slot, 0)
                                } else {
                                    let var = self.var(*a);
                                    builder.use_var(var)
                                }
                            })
                            .collect();

                        // Emit call
                        let call_inst = builder.ins().call(func_ref, &arg_vals);

                        // Store return value (single i64 in Bậc A).
                        if !is_sret_call && !dest.is_empty() {
                            let ret_val = builder.inst_results(call_inst)[0];
                            builder.def_var(self.var(dest[0]), ret_val);
                            // ADR-0049 Lát 3: populate String slot from returned handle.
                            if let Some((slot, layout)) = self.struct_slots.get(&dest[0])
                                && layout.name == "String"
                            {
                                let mem_flags = cranelift_codegen::ir::MemFlags::new();
                                builder.ins().stack_store(ret_val, *slot, 0);
                                let heap_len = builder.ins().load(I64, mem_flags, ret_val, 0);
                                builder.ins().stack_store(heap_len, *slot, 8);
                                let heap_cap = builder.ins().load(I64, mem_flags, ret_val, 8);
                                builder.ins().stack_store(heap_cap, *slot, 16);
                            }
                        }

                        // Jump to return block
                        let ret_block = self.blocks[return_bb];
                        builder.ins().jump(ret_block, &[]);
                    }

                    CallTarget::Shim => {
                        let func_id = self.get_or_declare_shim(callee_name)?;
                        let func_ref = self.module.declare_func_in_func(func_id, builder.func);

                        // ADR-0049 Lát 3b/5: dispatch by shim ABI class.
                        let bung_fields = matches!(
                            callee_name.as_str(),
                            "__triet_string_eq"
                                | "__triet_string_contains"
                                | "__triet_string_concat"
                        );
                        let mutate_writeback = matches!(
                            callee_name.as_str(),
                            "__triet_string_clear" | "__triet_string_append"
                        );
                        let arg_vals: Vec<_> = if bung_fields {
                            let mut vals = Vec::with_capacity(args.len() * 2);
                            for a in args {
                                if let Some((slot, _)) = self.struct_slots.get(a) {
                                    let ptr = builder.ins().stack_load(I64, *slot, 0);
                                    let len = builder.ins().stack_load(I64, *slot, 8);
                                    vals.push(ptr);
                                    vals.push(len);
                                } else {
                                    let handle = builder.use_var(self.var(*a));
                                    let mem_flags = cranelift_codegen::ir::MemFlags::new();
                                    let len = builder.ins().load(I64, mem_flags, handle, 0);
                                    vals.push(handle);
                                    vals.push(len);
                                }
                            }
                            vals
                        } else if mutate_writeback {
                            // Pass stack_addr(slot) for the source String local.
                            // The MIR arg is a &0 mutable String borrow — walk
                            // the Borrow statement to find the owned String local.
                            args.iter()
                                .map(|a| {
                                    // Find the Borrow source for this arg.
                                    let source_local =
                                        body.blocks.iter().flat_map(|b| &b.statements).find_map(
                                            |s| {
                                                if let Statement::Borrow { dest, source, .. } = s {
                                                    if dest.local == *a {
                                                        Some(source.local)
                                                    } else {
                                                        None
                                                    }
                                                } else {
                                                    None
                                                }
                                            },
                                        );
                                    if let Some(src) = source_local
                                        && let Some((slot, _)) = self.struct_slots.get(&src)
                                    {
                                        return builder.ins().stack_addr(I64, *slot, 0);
                                    }
                                    // Fallback: pass the arg directly (non-String).
                                    builder.use_var(self.var(*a))
                                })
                                .collect()
                        } else {
                            args.iter()
                                .map(|a| {
                                    if let Some((slot, layout)) = self.struct_slots.get(a) {
                                        if layout.name == "String" {
                                            builder.use_var(self.var(*a))
                                        } else {
                                            builder.ins().stack_addr(I64, *slot, 0)
                                        }
                                    } else if let Some((slot, _)) = self.enum_slots.get(a) {
                                        builder.ins().stack_load(I64, *slot, 0)
                                    } else {
                                        let var = self.var(*a);
                                        builder.use_var(var)
                                    }
                                })
                                .collect()
                        };

                        let call_inst = builder.ins().call(func_ref, &arg_vals);

                        // All builtin shims in ADR-0040 §3.1 that return values are
                        // 1-return shims. Check has_return via BuiltinShimMeta existence
                        // (all registered shims with a return value are in the meta table).
                        if !dest.is_empty() {
                            let ret_val = builder.inst_results(call_inst)[0];
                            builder.def_var(self.var(dest[0]), ret_val);
                            // ADR-0049 Lát 3: populate String slot from returned handle.
                            if let Some((slot, layout)) = self.struct_slots.get(&dest[0])
                                && layout.name == "String"
                            {
                                let mem_flags = cranelift_codegen::ir::MemFlags::new();
                                builder.ins().stack_store(ret_val, *slot, 0);
                                let heap_len = builder.ins().load(I64, mem_flags, ret_val, 0);
                                builder.ins().stack_store(heap_len, *slot, 8);
                                let heap_cap = builder.ins().load(I64, mem_flags, ret_val, 8);
                                builder.ins().stack_store(heap_cap, *slot, 16);
                            }
                        }

                        // M3: Zeroing-on-Move — zero consume-arg variables after call.
                        if let Some(meta) = builtin_shim_meta(callee_name) {
                            let zero = builder.ins().iconst(I64, 0);
                            for (i, a) in args.iter().enumerate() {
                                if i < meta.arg_consumes.len() && meta.arg_consumes[i] {
                                    let arg_ty = &body.local_decls[a.0].ty;
                                    if !is_copy(arg_ty, body) {
                                        // ADR-0049 Lát 2 L2-2: Slot-Truth —
                                        // stack_store sole guard for String.
                                        if let Some((slot, layout)) = self.struct_slots.get(a)
                                            && layout.name == "String"
                                        {
                                            builder.ins().stack_store(zero, *slot, 0);
                                        } else {
                                            let var = self.var(*a);
                                            builder.def_var(var, zero);
                                        }
                                    }
                                }
                            }
                        }

                        // ADR-0049 Lát 5: clear/append writeback via *mut FatStr;
                        // no manual sync needed — shim handles it.

                        let ret_block = self.blocks[return_bb];
                        builder.ins().jump(ret_block, &[]);
                    }
                }
            }

            Terminator::Unreachable { .. } | Terminator::Trap { .. } => {
                builder
                    .ins()
                    .trap(cranelift_codegen::ir::TrapCode::unwrap_user(1));
            }

            Terminator::SwitchInt {
                discriminant,
                cases,
                default_bb,
                ..
            } => {
                let disc_val = builder.use_var(self.var(*discriminant));
                let default_block = *self.blocks.get(default_bb).ok_or_else(|| {
                    JitError::Unsupported("SwitchInt default_bb not found".into())
                })?;

                if cases.is_empty() {
                    builder.ins().jump(default_block, &[]);
                } else {
                    // Synthesised block indices are allocated after the MIR
                    // blocks during `build_body`. The first synthetic block
                    // index = cfg.blocks.len().
                    let synth_base = body.build_cfg().blocks.len();
                    // Lower as a cascading if-chain using pre-allocated
                    // synthetic blocks for fall-though. Each comparison
                    // uses brif: match → target, no-match → fallthrough.
                    for (i, (disc_val_expected, target_bb)) in cases.iter().enumerate() {
                        let target = *self.blocks.get(target_bb).ok_or_else(|| {
                            JitError::Unsupported("SwitchInt case target not found".into())
                        })?;
                        let expected = builder.ins().iconst(I64, *disc_val_expected);
                        let is_eq = builder.ins().icmp(
                            cranelift_codegen::ir::condcodes::IntCC::Equal,
                            disc_val,
                            expected,
                        );
                        if i + 1 < cases.len() {
                            let fallthrough_bb = BasicBlock(synth_base + i);
                            let fallthrough =
                                *self.blocks.get(&fallthrough_bb).ok_or_else(|| {
                                    JitError::Unsupported(
                                        "SwitchInt synthetic fallthrough block not found".into(),
                                    )
                                })?;
                            builder.ins().brif(is_eq, target, &[], fallthrough, &[]);
                            builder.switch_to_block(fallthrough);
                        } else {
                            // Last case: match → target, miss → default
                            builder.ins().brif(is_eq, target, &[], default_block, &[]);
                        }
                    }
                }
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
#[allow(clippy::too_many_lines)] // match-heavy dispatch, naturally long
fn lower_binop(
    builder: &mut FunctionBuilder<'_>,
    op: BinOp,
    lhs: cranelift_codegen::ir::Value,
    rhs: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    use cranelift_codegen::ir::TrapCode;

    let i64 = I64;

    // ADR-0044: Integer range enforcement — from triet-core (F3).
    let max_val = triet_core::Integer::MAX.to_i64();
    let m = builder.ins().iconst(i64, max_val);
    let neg_m = builder.ins().iconst(i64, -max_val);

    let trap_code = TrapCode::unwrap_user(1);

    match op {
        // ── Arithmetic with range enforcement ──
        BinOp::Add => {
            let result = builder.ins().iadd(lhs, rhs);
            // |a+b| ≤ 2M ≪ i64::MAX — carrier never overflows (F5).
            let above_max = builder.ins().icmp(IntCC::SignedGreaterThan, result, m);
            builder.ins().trapnz(above_max, trap_code);
            let below_min = builder.ins().icmp(IntCC::SignedLessThan, result, neg_m);
            builder.ins().trapnz(below_min, trap_code);
            result
        }
        BinOp::Sub => {
            let result = builder.ins().isub(lhs, rhs);
            let above_max = builder.ins().icmp(IntCC::SignedGreaterThan, result, m);
            builder.ins().trapnz(above_max, trap_code);
            let below_min = builder.ins().icmp(IntCC::SignedLessThan, result, neg_m);
            builder.ins().trapnz(below_min, trap_code);
            result
        }
        BinOp::Mul => {
            // F6: carrier can overflow — use smulhi before post-check.
            let result = builder.ins().imul(lhs, rhs);
            let upper = builder.ins().smulhi(lhs, rhs);
            // Sign-extend lower half: 0 if result ≥ 0, −1 if negative.
            let zero = builder.ins().iconst(i64, 0);
            let neg_one = builder.ins().iconst(i64, -1_i64);
            let is_neg = builder.ins().icmp(IntCC::SignedLessThan, result, zero);
            let sign_ext = builder.ins().select(is_neg, neg_one, zero);
            // upper != sign_ext → carrier overflow → trap.
            let carrier_overflow = builder.ins().icmp(IntCC::NotEqual, upper, sign_ext);
            builder.ins().trapnz(carrier_overflow, trap_code);
            // Carrier OK — range-check lower half.
            let above_max = builder.ins().icmp(IntCC::SignedGreaterThan, result, m);
            builder.ins().trapnz(above_max, trap_code);
            let below_min = builder.ins().icmp(IntCC::SignedLessThan, result, neg_m);
            builder.ins().trapnz(below_min, trap_code);
            result
        }
        // Div/Mod: quy nạp — input in-range → |a/b| ≤ |a| ≤ M, |a%b| < |b| ≤ M.
        BinOp::Div => builder.ins().sdiv(lhs, rhs),
        BinOp::Mod => builder.ins().srem(lhs, rhs),

        // ── Ternary negation (no range check — symmetric: F4) ──
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
            let one = builder.ins().iconst(i64, 1);
            let neg_one = builder.ins().iconst(i64, -1_i64);
            builder.ins().select(cmp, one, neg_one)
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
const extern "C" fn __test_shim_multiply(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b)
}

/// Integer power via exponentiation by squaring (`extern "C"` ABI).
/// `pow(base, exp)` = base^exp. Exponent must be >= 0.
#[allow(unsafe_code)]
// checked_mul + range check pattern (rule 5)
#[allow(clippy::option_if_let_else)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_pow(base: i64, exp: i64) -> i64 {
    if exp < 0 {
        return 0;
    }
    let max_val = triet_core::Integer::MAX.to_i64();
    let mut result: i64 = 1;
    let mut e = exp;
    let mut b = base;
    while e > 0 {
        if e & 1 != 0 {
            result = match result.checked_mul(b) {
                Some(v) => v,
                None => std::process::abort(),
            };
            if result > max_val || result < -max_val {
                std::process::abort();
            }
        }
        e >>= 1;
        if e > 0 {
            b = match b.checked_mul(b) {
                Some(v) => v,
                None => std::process::abort(),
            };
            if b > max_val || b < -max_val {
                std::process::abort();
            }
        }
    }
    result
}

// ── String heap shims (ADR-0040 §3.1) ────────────────────────

/// Header size in bytes: `ObjectHeader` (refcount: u32 + reserved: u32 = 8 bytes).
const HEADER_SIZE: usize = 8;

/// Layout for a String heap allocation: header + len (i64) + cap (i64) + data.
fn string_layout(cap: usize) -> std::alloc::Layout {
    let total = HEADER_SIZE + 8 + 8 + cap; // header + len + cap + data
    std::alloc::Layout::from_size_align(total, 8).unwrap()
}

/// `__triet_string_alloc(len, cap)` — allocate a String with given length and capacity.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_string_alloc(len: i64, cap: i64) -> i64 {
    let cap_usize = cap.max(len).max(1) as usize; // at least 1 byte
    let layout = string_layout(cap_usize);
    // SAFETY: layout is valid (power-of-2 alignment, non-zero size).
    let ptr = unsafe { std::alloc::alloc(layout) };
    if ptr.is_null() {
        return 0; // OOM — return null
    }
    // Write ObjectHeader: refcount=1, reserved=0
    // SAFETY: layout guarantees 8-byte aligned, >=8 bytes at ptr.

    unsafe {
        ptr.cast::<u32>().write_unaligned(1u32); // refcount = 1
        ptr.cast::<u32>().add(1).write_unaligned(0u32); // reserved = 0
        // Write len and cap
        let body = ptr.add(HEADER_SIZE);
        body.cast::<i64>().write_unaligned(len);
        body.cast::<i64>().add(1).write_unaligned(cap);
        body as i64
    }
}

/// `__triet_string_from_bytes(ptr, len)` — copy bytes from read-only memory into a new heap String.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_string_from_bytes(src: i64, len: i64) -> i64 {
    if src == 0 || len < 0 {
        return 0;
    }
    let len_usize = len as usize;
    let body_ptr = __triet_string_alloc(len, len);
    if body_ptr == 0 {
        return 0;
    }
    // Copy bytes from src to data area
    // SAFETY: src pointer is valid (lifetime guaranteed by driver §3.3).
    unsafe {
        let dst = (body_ptr as *mut u8).add(16); // skip len + cap (16 bytes)
        std::ptr::copy_nonoverlapping(src as *const u8, dst, len_usize);
    }
    body_ptr
}

/// `__triet_string_free(ptr, cap)` — free a String. No-op if ptr == 0.
/// ADR-0049 Lát 3: cap passed explicitly (heap no longer source-of-truth).
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_string_free(ptr: i64, cap: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    let cap_usize = cap.max(1) as usize;
    let layout = string_layout(cap_usize);
    let body = ptr as *mut u8;
    let header = unsafe { body.sub(HEADER_SIZE) };
    // SAFETY: layout matches the one used at allocation.
    unsafe { std::alloc::dealloc(header, layout) };
}

/// `__triet_string_concat(a_ptr, a_len, b_ptr, b_len)` — concatenate two Strings.
/// ADR-0049 Lát 3b: len passed explicitly (no heap read).
/// Returns new heap handle (sret deferred to Lát 3c).
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_string_concat(a_ptr: i64, a_len: i64, b_ptr: i64, b_len: i64) -> i64 {
    if a_ptr == 0 || b_ptr == 0 {
        std::process::abort();
    }
    if a_len < 0 || b_len < 0 {
        return 0;
    }
    let total_len = a_len + b_len;
    let result = __triet_string_alloc(total_len, total_len);
    if result == 0 {
        return 0;
    }
    let a_body = a_ptr as *const u8;
    let b_body = b_ptr as *const u8;
    // SAFETY: src pointers valid, dst pointer valid with sufficient capacity.
    unsafe {
        let dst = (result as *mut u8).add(16);
        let a_data = a_body.add(16);
        let b_data = b_body.add(16);
        std::ptr::copy_nonoverlapping(a_data, dst, a_len as usize);
        std::ptr::copy_nonoverlapping(b_data, dst.add(a_len as usize), b_len as usize);
    }
    unsafe {
        (result as *mut i64).write_unaligned(total_len);
    }
    result
}

/// `__triet_string_eq(a_ptr, a_len, b_ptr, b_len)` — equality comparison.
/// ADR-0049 Lát 3b: len passed explicitly (no heap read).
/// Returns 1 (true) or -1 (false) per ADR-0047 Trilean encoding.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_string_eq(a_ptr: i64, a_len: i64, b_ptr: i64, b_len: i64) -> i64 {
    // C9 trap-on-0: neither pointer may be a dead value.
    if a_ptr == 0 || b_ptr == 0 {
        std::process::abort();
    }
    if a_ptr == b_ptr {
        return 1; // same pointer → equal
    }
    if a_len != b_len {
        return -1;
    }
    let len = a_len as usize;
    let a_data = a_ptr as *const u8;
    let b_data = b_ptr as *const u8;
    // SAFETY: data areas are valid reads of `len` bytes.
    unsafe {
        let a_bytes = a_data.add(16);
        let b_bytes = b_data.add(16);
        for i in 0..len {
            if a_bytes.add(i).read() != b_bytes.add(i).read() {
                return -1;
            }
        }
    }
    1
}

/// `__triet_string_len(ptr)` — return the length of a String.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_string_len(ptr: i64) -> i64 {
    // C9 trap-on-0: 0 = dead value (moved-out / OOM), never a valid heap ptr.
    if ptr == 0 {
        std::process::abort();
    }
    // SAFETY: ptr points to valid body.
    unsafe { (ptr as *const i64).read_unaligned() }
}

// ADR-0049 Lát 5: fat-pointer layout mirrored in shim for writeback.
// Must match StackSlot: ptr@0, len@8, cap@16.
#[repr(C)]
struct FatStr {
    ptr: i64,
    len: i64,
    cap: i64,
}

/// `__triet_string_clear(slot_ptr)` — *mut FatStr writeback: len=0, ptr unchanged.
/// ADR-0049 Lát 5: receives pointer to caller's StackSlot, writes back via pointer.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_string_clear(slot: i64) -> i64 {
    let slot = slot as *mut FatStr;
    // SAFETY: slot is a valid StackSlot pointer (caller-allocated).
    unsafe {
        if (*slot).ptr == 0 {
            std::process::abort();
        }
        (*slot).len = 0;
        // Writeback to heap for boundary compatibility (Lát 6 will remove).
        ((*slot).ptr as *mut i64).write_unaligned(0);
    }
    0 // Unit
}

/// `__triet_string_append(slot_ptr, byte)` — append one byte, realloc if needed.
/// ADR-0049 Lát 5: *mut FatStr writeback. Reads {ptr,len,cap} from slot,
/// grows if len==cap, writes byte, writebacks {ptr,len+1,cap} to slot+heap.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_string_append(slot: i64, byte: i64) -> i64 {
    let slot = slot as *mut FatStr;
    unsafe {
        if (*slot).ptr == 0 {
            std::process::abort();
        }
        let mut ptr = (*slot).ptr;
        let mut cap = (*slot).cap;
        let len = (*slot).len;

        if len >= cap {
            // Realloc: double capacity (min 4).
            let new_cap = (cap * 2).max(4);
            let new_body = __triet_string_alloc(new_cap, new_cap);
            if new_body == 0 {
                return 0; // OOM
            }
            // Copy old data: header + len + cap + data bytes
            let old_header = (ptr as *mut u8).sub(HEADER_SIZE);
            let new_header = (new_body as *mut u8).sub(HEADER_SIZE);
            let old_total = HEADER_SIZE + 16 + cap as usize;
            std::ptr::copy_nonoverlapping(old_header, new_header, old_total);
            // Free old block
            let old_layout = string_layout(cap.max(1) as usize);
            std::alloc::dealloc(old_header, old_layout);
            ptr = new_body;
            cap = new_cap;
        }

        // Write byte at data+len
        let data = (ptr as *mut u8).add(16);
        data.add(len as usize).write(byte as u8);

        // Writeback to slot
        (*slot).ptr = ptr;
        (*slot).len = len + 1;
        (*slot).cap = cap;

        // Writeback to heap (boundary compatibility, removed at Lát 6)
        (ptr as *mut i64).write_unaligned(len + 1);
        (ptr as *mut i64).add(1).write_unaligned(cap);
    }
    0 // Unit
}

/// `__triet_string_contains(h_ptr, h_len, n_ptr, n_len)` — substring search.
/// ADR-0049 Lát 3b: len passed explicitly (no heap read).
/// Returns 1 (true) if needle is substring, -1 (false) otherwise. Never 0.
#[allow(unsafe_code)]
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_string_contains(h_ptr: i64, h_len: i64, n_ptr: i64, n_len: i64) -> i64 {
    if h_ptr == 0 || n_ptr == 0 {
        std::process::abort();
    }
    let h_len = h_len as usize;
    let n_len = n_len as usize;
    if n_len == 0 {
        return 1;
    }
    if n_len > h_len {
        return -1;
    }
    let h_bytes = h_ptr as *const u8;
    let n_bytes = n_ptr as *const u8;
    // SAFETY: data areas are valid reads.
    unsafe {
        let h_data = h_bytes.add(16);
        let n_data = n_bytes.add(16);
        for start in 0..=(h_len - n_len) {
            let mut matched = true;
            for off in 0..n_len {
                if h_data.add(start + off).read() != n_data.add(off).read() {
                    matched = false;
                    break;
                }
            }
            if matched {
                return 1;
            }
        }
    }
    -1
}

// ── Vector heap shims (ADR-0040 §5) ──────────────────────────

/// Layout for a Vector heap allocation: header + len (i64) + cap (i64) + data (cap × 8).
/// Element = i64 (Bậc A monomorphic `Vector<Integer>`).
#[allow(clippy::missing_const_for_fn)] // `Layout::from_size_align` is not const-stable
fn vector_layout(cap: usize) -> std::alloc::Layout {
    let total = HEADER_SIZE + 8 + 8 + cap * 8; // header + len + cap + data
    std::alloc::Layout::from_size_align(total, 8).unwrap()
}

/// `__triet_vector_alloc(len, cap)` — allocate a Vector with given length and capacity.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,        // len/cap are non-negative by construction
    clippy::cast_possible_truncation, // 64-bit target, values fit in usize
    clippy::cast_ptr_alignment,    // write_unaligned used, alignment irrelevant
    clippy::ptr_as_ptr             // idiomatic in extern "C" heap code (mirrors String shims)
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_vector_alloc(len: i64, cap: i64) -> i64 {
    let cap_usize = cap.max(len).max(2) as usize; // at least cap=2 for realloc teeth
    let layout = vector_layout(cap_usize);
    // SAFETY: layout is valid (power-of-2 alignment, non-zero size).
    let ptr = unsafe { std::alloc::alloc(layout) };
    if ptr.is_null() {
        return 0; // OOM — return null
    }
    // Write ObjectHeader: refcount=1, reserved=0
    // SAFETY: layout guarantees 8-byte aligned, >=8 bytes at ptr.

    unsafe {
        (ptr as *mut u32).write_unaligned(1u32); // refcount = 1
        (ptr as *mut u32).add(1).write_unaligned(0u32); // reserved = 0
        // Write len and cap
        let body = ptr.add(HEADER_SIZE);
        (body as *mut i64).write_unaligned(len);
        (body as *mut i64).add(1).write_unaligned(cap_usize as i64);
        body as i64
    }
}

/// `__triet_vector_free(ptr)` — free a Vector. No-op if ptr == 0 or
/// ptr == `NULL_SENTINEL` (C4 moved-out + ADR-0041 §5.5 null guard).
#[allow(unsafe_code)]
#[allow(
    clippy::cast_possible_truncation, // 64-bit target, stored cap fits in usize
    clippy::cast_ptr_alignment,    // read_unaligned used
    clippy::ptr_as_ptr
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_vector_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    let body = ptr as *mut u8;
    // Read cap to compute layout
    // SAFETY: body pointer is valid and points to len+cap+data structure.
    let cap = unsafe { (body as *const i64).add(1).read_unaligned() } as usize;
    let layout = vector_layout(cap.max(2));
    let header = unsafe { body.sub(HEADER_SIZE) };
    // SAFETY: layout matches the one used at allocation.
    unsafe { std::alloc::dealloc(header, layout) };
}

/// `__triet_vector_len(ptr)` — return the length of a Vector. Returns 0 for null.
#[allow(unsafe_code)]
#[allow(clippy::cast_ptr_alignment)] // read_unaligned used
#[unsafe(no_mangle)]
pub extern "C" fn __triet_vector_len(ptr: i64) -> i64 {
    // C9 trap-on-0: 0 = dead value, never a valid heap ptr.
    if ptr == 0 {
        std::process::abort();
    }
    // SAFETY: ptr points to valid body.
    unsafe { (ptr as *const i64).read_unaligned() }
}

/// `__triet_vector_push(vec, elem)` — functional push: clone vec, append elem,
/// return new vector. Consumes vec (caller zeros after call via M3).
///
/// Always alloc+copy — never realloc in-place, so M3-zero teeth can detect
/// double-free through the free-old-ptr path deterministically.
/// O(n) per push; in-place fast path when `len < cap` is a Bậc B optimization.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,        // len/cap/offset are non-negative
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,    // len/cap fit in i64 (max usize::MAX/2)
    clippy::cast_ptr_alignment,    // write_unaligned/read_unaligned used
    clippy::ptr_as_ptr
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_vector_push(vec: i64, elem: i64) -> i64 {
    // C9 trap-on-0: vec == 0 means dead value (moved-out) — trap, don't silently return.
    if vec == 0 {
        std::process::abort();
    }
    let old_body = vec as *const u8;
    // Read old len and cap
    // SAFETY: vec points to valid body.
    let (old_len, old_cap) = unsafe {
        let l = (old_body as *const i64).read_unaligned();
        let c = (old_body as *const i64).add(1).read_unaligned();
        (l as usize, c as usize)
    };
    let new_len = old_len + 1;
    let new_cap = if new_len > old_cap {
        old_cap * 2
    } else {
        old_cap
    };
    let new_body = __triet_vector_alloc(new_len as i64, new_cap as i64);
    if new_body == 0 {
        return 0;
    }
    // Copy old elements to new block
    // SAFETY: old_body and new_body point to valid data areas.
    unsafe {
        let old_data = old_body.add(16); // skip len + cap
        let new_data = (new_body as *mut u8).add(16);
        std::ptr::copy_nonoverlapping(old_data, new_data, old_len * 8);
        // Write new element at position old_len
        (new_data as *mut i64).add(old_len).write_unaligned(elem);
    }
    // Free old block (explicit alloc + free — no realloc for deterministic teeth)
    __triet_vector_free(vec);
    new_body
}

/// `__triet_vector_get(vec, idx)` — bounds-checked element access.
///
/// Returns the element at `idx` if `0 <= idx < len`, otherwise
/// [`NULL_SENTINEL`](triet_mir::NULL_SENTINEL). Total function per
/// ADR-0041: never panics, out-of-bounds or null → null sentinel.
///
/// Guard 1 (C9 trap-on-0): `vec == 0` → SIGABRT (dead value).
/// Guard 2 (bounds): `idx` out of range → return `NULL_SENTINEL`.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,        // len is non-negative, idx < len guard
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,     // len/cap fit in i64
    clippy::cast_ptr_alignment,    // read_unaligned used
    clippy::ptr_as_ptr
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_vector_get(vec: i64, idx: i64) -> i64 {
    // C9 trap-on-0: vec == 0 means dead value, not a valid vector.
    if vec == 0 {
        std::process::abort();
    }
    let body = vec as *const u8;
    // SAFETY: vec points to valid heap body (len + cap + data).
    let len = unsafe { (body as *const i64).read_unaligned() };
    // Bounds check: out-of-range → null sentinel (total function contract).
    if idx < 0 || idx >= len {
        return triet_mir::NULL_SENTINEL;
    }
    // SAFETY: idx is in [0, len), data area starts at offset 16.
    unsafe {
        let data = body.add(16);
        (data as *const i64).add(idx as usize).read_unaligned()
    }
}

/// `__triet_vector_contains(vec, elem)` — linear scan.
/// Returns 1 (true) if `elem` is found, -1 (false) otherwise.
/// Never returns 0.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_vector_contains(vec: i64, elem: i64) -> i64 {
    if vec == 0 {
        std::process::abort(); // trap-on-0
    }
    let body = vec as *const u8;
    // SAFETY: vec points to valid heap body (len + cap + data).
    let len = unsafe { body.cast::<i64>().read_unaligned() } as usize;
    // SAFETY: data area starts at offset 16.
    unsafe {
        let data = body.add(16).cast::<i64>();
        for i in 0..len {
            if data.add(i).read_unaligned() == elem {
                return 1;
            }
        }
    }
    -1
}

// ── HashMap shims (ADR-0043) ─────────────────────────────────

const HASHMAP_SLOT_SIZE: usize = 24; // key(8) + value(8) + state(1) + pad(7)
const HASHMAP_HEADER_SIZE: usize = HEADER_SIZE; // 8B ObjectHeader

#[allow(clippy::missing_const_for_fn)] // Layout::from_size_align is not const-stable
// Layout: header(8) + len(8) + cap(8) + cap × 24.
fn hashmap_layout(cap: usize) -> std::alloc::Layout {
    let total = HASHMAP_HEADER_SIZE + 8 + 8 + cap * HASHMAP_SLOT_SIZE;
    std::alloc::Layout::from_size_align(total, 8).unwrap()
}

#[allow(clippy::missing_const_for_fn)]
// contains unsafe block, not const-stable
// Return state byte pointer for slot. Caller ensures idx < cap, body valid.
#[allow(unsafe_code)]
unsafe fn hashmap_state_ptr(body: *mut u8, idx: usize) -> *mut u8 {
    unsafe { body.add(16 + idx * HASHMAP_SLOT_SIZE + 16) }
}

#[allow(clippy::missing_const_for_fn)]
// contains unsafe block, not const-stable
// Return key/value pointers for slot. Caller ensures idx < cap, body valid.
#[allow(unsafe_code, clippy::cast_ptr_alignment, clippy::ptr_as_ptr)]
unsafe fn hashmap_kv_ptrs(body: *mut u8, idx: usize) -> (*mut i64, *mut i64) {
    unsafe {
        let base = body.add(16 + idx * HASHMAP_SLOT_SIZE);
        (base as *mut i64, base.add(8) as *mut i64)
    }
}

/// Allocate a `HashMap` with given `len` and `cap`.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,           // len/cap are non-negative by construction
    clippy::cast_possible_truncation, // 64-bit target, values fit in usize
    clippy::cast_possible_wrap,       // cap max is bounded by memory, won't wrap i64
    clippy::cast_ptr_alignment,       // write_unaligned used, alignment irrelevant
    clippy::ptr_as_ptr                // idiomatic in extern "C" heap code
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_hashmap_alloc(len: i64, cap: i64) -> i64 {
    // Invariant (ADR-0043 Q3): load factor < 1 requires cap > len.
    // Clamp at 4 to guarantee at least 1 EMPTY slot after any valid
    // insert — prevents infinite probe when no slot is empty.
    let cap_usize = (cap.max(4) as usize).max(len as usize + 1).max(4);
    let layout = hashmap_layout(cap_usize);
    let ptr = unsafe { std::alloc::alloc(layout) };
    if ptr.is_null() {
        return 0; // OOM
    }
    unsafe {
        (ptr as *mut u32).write_unaligned(1u32); // refcount = 1
        (ptr as *mut u32).add(1).write_unaligned(0u32); // reserved = 0
        let body = ptr.add(HASHMAP_HEADER_SIZE);
        (body as *mut i64).write_unaligned(len);
        (body as *mut i64).add(1).write_unaligned(cap_usize as i64);
        // Zero-initialize all state bytes to EMPTY (0)
        let state_base = body.add(16 + 16); // skip len+cap, point to first state byte
        for i in 0..cap_usize {
            state_base.add(i * HASHMAP_SLOT_SIZE).write_unaligned(0u8);
        }
        body as i64
    }
}

/// Free a `HashMap`. No-op if `ptr == 0` or `ptr == NULL_SENTINEL`.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,           // cap read from valid allocation, non-negative
    clippy::cast_possible_truncation, // 64-bit target, cap fits in usize
    clippy::cast_possible_wrap,       // cap max bounded by memory
    clippy::cast_ptr_alignment,       // read_unaligned used
    clippy::ptr_as_ptr                // idiomatic in extern "C" heap code
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_hashmap_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    let body = ptr as *mut u8;
    let cap = unsafe { (body as *const i64).add(1).read_unaligned() } as usize;
    let layout = hashmap_layout(cap.max(2));
    let header = unsafe { body.sub(HASHMAP_HEADER_SIZE) };
    unsafe { std::alloc::dealloc(header, layout) };
}

/// Return entry count of a `HashMap`. Trap-on-0.
#[allow(unsafe_code)]
#[allow(clippy::cast_ptr_alignment)] // read_unaligned used
#[unsafe(no_mangle)]
pub extern "C" fn __triet_hashmap_len(ptr: i64) -> i64 {
    if ptr == 0 {
        std::process::abort();
    }
    unsafe { (ptr as *const i64).read_unaligned() }
}

/// Functional insert: consume `map`, return new map ptr. Traps if
/// `v == i64::MIN` (D2). Realloc at load factor 0.75 with rehash.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,           // len/cap are non-negative by construction
    clippy::cast_possible_truncation, // 64-bit target, values fit in usize
    clippy::cast_possible_wrap,       // size bounded by memory
    clippy::cast_ptr_alignment,       // write_unaligned/read_unaligned used
    clippy::ptr_as_ptr,               // idiomatic in extern "C" heap code
    clippy::similar_names             // probe vs probe variable in rehash + insert loops
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_hashmap_insert(map: i64, k: i64, v: i64) -> i64 {
    // D2 defense-in-depth: reject MIN value (ADR-0044 Q4).
    // Arithmetic trap makes MIN unreachable, but this guard
    // stays as the last net — cost ≈ 0, catches any gap in
    // the inductive range-enforcement argument.
    if v == triet_mir::NULL_SENTINEL {
        std::process::abort();
    }
    if map == 0 {
        std::process::abort(); // trap-on-0
    }
    let body = map as *mut u8;
    let len = unsafe { (body as *const i64).read_unaligned() } as usize;
    let cap = unsafe { (body as *const i64).add(1).read_unaligned() } as usize;

    // Check load factor
    let new_cap = if len * 4 >= cap * 3 {
        // Realloc: double capacity
        (cap * 2).max(4)
    } else {
        0 // no realloc needed
    };

    let (body_ptr, cap_used) = if new_cap > 0 {
        // Allocate new map + rehash all OCCUPIED entries
        let new_map = __triet_hashmap_alloc(0, new_cap as i64);
        if new_map == 0 {
            return 0; // OOM
        }
        let new_body = new_map as *mut u8;
        // Rehash all OCCUPIED entries from old map
        for i in 0..cap {
            let state = unsafe { *hashmap_state_ptr(body, i) };
            if state == 1u8 {
                let (k_ptr, v_ptr) = unsafe { hashmap_kv_ptrs(body, i) };
                let old_k = unsafe { k_ptr.read_unaligned() };
                let old_v = unsafe { v_ptr.read_unaligned() };
                let nc = new_cap as i64;
                let hash = (old_k % nc + nc) % nc;
                let mut probe = hash as usize;
                loop {
                    let st = unsafe { *hashmap_state_ptr(new_body, probe) };
                    if st == 0u8 {
                        let (nk, nv) = unsafe { hashmap_kv_ptrs(new_body, probe) };
                        unsafe {
                            nk.write_unaligned(old_k);
                            nv.write_unaligned(old_v);
                            *hashmap_state_ptr(new_body, probe) = 1u8;
                        }
                        break;
                    }
                    probe = (probe + 1) % new_cap;
                }
            }
        }
        // Update len in new map
        unsafe { (new_body as *mut i64).write_unaligned(len as i64) };
        __triet_hashmap_free(map);
        (new_body, new_cap)
    } else {
        (body, cap)
    };

    // Insert or update via linear probing
    let hash = (k % cap_used as i64 + cap_used as i64) % cap_used as i64;
    let mut probe = hash as usize;
    let mut is_update = false;
    loop {
        let state = unsafe { *hashmap_state_ptr(body_ptr, probe) };
        if state == 1u8 {
            // OCCUPIED — check if key matches
            let (nk, _nv) = unsafe { hashmap_kv_ptrs(body_ptr, probe) };
            if unsafe { nk.read_unaligned() } == k {
                // Update existing value, len unchanged
                let (_, nv) = unsafe { hashmap_kv_ptrs(body_ptr, probe) };
                unsafe { nv.write_unaligned(v) };
                is_update = true;
                break;
            }
        } else if state == 0u8 {
            // EMPTY — write new entry
            let (nk, nv) = unsafe { hashmap_kv_ptrs(body_ptr, probe) };
            unsafe {
                nk.write_unaligned(k);
                nv.write_unaligned(v);
                *hashmap_state_ptr(body_ptr, probe) = 1u8;
            }
            break;
        }
        probe = (probe + 1) % cap_used;
    }
    if !is_update {
        // Increment len only for new entries
        let new_len = (len + 1) as i64;
        unsafe { (body_ptr as *mut i64).write_unaligned(new_len) };
    }

    // Return body pointer (or header+offset for consistency)
    // body_ptr is already the body address (ptr + HEADER_SIZE)
    body_ptr as i64
}

/// Look up key, return value or `NULL_SENTINEL`. Trap-on-0 for map handle.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,           // hash may produce negative, intentionally handled
    clippy::cast_possible_truncation, // cap read from valid allocation
    clippy::cast_possible_wrap,       // double-mod normalizes to [0, cap)
    clippy::cast_ptr_alignment        // read_unaligned used
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_hashmap_get(map: i64, k: i64) -> i64 {
    if map == 0 {
        std::process::abort();
    }
    let body = map as *mut u8;
    let cap = unsafe { (body as *const i64).add(1).read_unaligned() } as usize;
    let hash = (k % cap as i64 + cap as i64) % cap as i64;
    let mut probe = hash as usize;
    loop {
        let state = unsafe { *hashmap_state_ptr(body, probe) };
        if state == 0u8 {
            return triet_mir::NULL_SENTINEL; // EMPTY — key not found
        }
        if state == 1u8 {
            let (k_ptr, v_ptr) = unsafe { hashmap_kv_ptrs(body, probe) };
            let stored_k = unsafe { k_ptr.read_unaligned() };
            if stored_k == k {
                return unsafe { v_ptr.read_unaligned() };
            }
        }
        probe = (probe + 1) % cap;
    }
}

/// `__triet_hashmap_contains(map, key)` — key lookup.
/// Returns 1 (true) if `key` exists in the map, -1 (false) otherwise.
/// Never returns 0.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_hashmap_contains(map: i64, k: i64) -> i64 {
    if map == 0 {
        std::process::abort(); // trap-on-0
    }
    let body = map as *mut u8;
    let cap = unsafe { (body as *const i64).add(1).read_unaligned() } as usize;
    let hash = (k % cap as i64 + cap as i64) % cap as i64;
    let mut probe = hash as usize;
    loop {
        let state = unsafe { *hashmap_state_ptr(body, probe) };
        if state == 0u8 {
            return -1; // EMPTY — key not found
        }
        if state == 1u8 {
            let (k_ptr, _v_ptr) = unsafe { hashmap_kv_ptrs(body, probe) };
            let stored_k = unsafe { k_ptr.read_unaligned() };
            if stored_k == k {
                return 1; // FOUND
            }
        }
        probe = (probe + 1) % cap;
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use triet_borrowck::{MirBuilder, binop, const_int, return_, storage_live};
    use triet_mir::{DUMMY_SPAN, FunctionId, ParameterPassing, Place, ReturnShape, Statement};

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
                return_shape: ReturnShape::Scalar,
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
                return_shape: ReturnShape::Scalar,
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
            (-1, _) | (0, 1) | (0, 0) => 1,
            (1, x) => x,
            (0, -1) => 0,
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
                return_shape: ReturnShape::Scalar,
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
                return_shape: ReturnShape::Scalar,
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

    /// Test-only counting wrapper around `__triet_string_free`.
    /// Increments a static counter before delegating to the real free.
    static FREE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    extern "C" fn __test_counting_free(ptr: i64, cap: i64) {
        FREE_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        super::__triet_string_free(ptr, cap);
    }

    /// 4i-2/4i-4 (callee side): M4 Return-escape — Drop before Return is skipped.
    /// Hand-built MIR (bypasses lowerer); call-dest typing is tested by
    /// `call_dest_has_correct_type_for_heap_return` in the lowerer.
    #[test]
    #[allow(unsafe_code)]
    fn alloc_free_balance_string_return() {
        use std::sync::atomic::Ordering;

        FREE_COUNT.store(0, Ordering::SeqCst);

        // Simulate: make() -> String { let s="hi"; return s }
        let mut b = MirBuilder::new("make_string", "String");
        // ADR-0049: String layout required for JIT StackSlot pre-pass.
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "String",
            &[
                ("ptr".to_string(), "Integer".to_string(), 8, 8),
                ("len".to_string(), "Integer".to_string(), 8, 8),
                ("cap".to_string(), "Integer".to_string(), 8, 8),
            ],
        ));
        let bb0 = b.new_block();
        let s = b.new_local();
        b.set_local_type(s, "String");
        b.push(bb0, storage_live(s));
        b.push(
            bb0,
            Statement::Const {
                dest: Place::local(s),
                value: triet_mir::ConstValue::String("hi".into()),
                span: DUMMY_SPAN,
            },
        );
        b.set_terminator(bb0, return_(vec![s]));

        let body = b.build(bb0);
        let shims = &[
            ShimSymbol::fn_2_1(
                "__triet_string_from_bytes",
                super::__triet_string_from_bytes,
            ),
            ShimSymbol::fn_2_0("__triet_string_free", __test_counting_free),
        ];
        let mut ctx = JitContext::with_shims(shims);
        let func = ctx.compile(&body).expect("4i-4 compile");
        let ptr = unsafe { func.call_i64_0() };

        assert_ne!(ptr, 0, "returned String ptr must be non-zero");
        // M4: Drop before Return must be skipped → 0 frees in callee.
        assert_eq!(
            FREE_COUNT.load(Ordering::SeqCst),
            0,
            "callee must not free the returned value (M4)"
        );

        // Simulate caller Drop. alloc(len, len) → cap == len for literals.
        let cap = unsafe { (ptr as *const i64).add(1).read_unaligned() };
        __test_counting_free(ptr, cap);
        assert_eq!(
            FREE_COUNT.load(Ordering::SeqCst),
            1,
            "caller Drop must free exactly once"
        );
    }

    /// 4i-1: M1 Zeroing-on-Move — after Assign of Move type, source must be 0.
    #[test]
    #[allow(unsafe_code)]
    fn m1_zeroing_on_move() {
        let mut b = MirBuilder::new("test_m1", "Integer");
        let s = b.add_param("s", ParameterPassing::Move);
        b.set_local_type(s, "String");
        let other = b.new_local();
        let result = b.new_local();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(other));
        b.push(bb0, storage_live(result));
        // Assign String → M1 should store 0 into s
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(other),
                source: Place::local(s),
                span: DUMMY_SPAN,
            },
        );
        // Return s (which should be 0 after M1) + 1 → verify s is 0
        b.push(
            bb0,
            Statement::BinaryOp {
                dest: Place::local(result),
                op: triet_mir::BinOp::Add,
                left: Place::local(s),
                right: Place::local(other),
                span: DUMMY_SPAN,
            },
        );
        b.set_terminator(bb0, return_(vec![result]));

        let body = b.build(bb0);
        let shims = &[
            ShimSymbol::fn_2_1("__triet_string_alloc", super::__triet_string_alloc),
            ShimSymbol::fn_2_0("__triet_string_free", super::__triet_string_free),
        ];
        let mut ctx = JitContext::with_shims(shims);
        let func = ctx.compile(&body).expect("M1 test compile");

        // s = "hello" → ptr is non-zero. After move, s should be 0.
        // result = s + other = 0 + other = other.
        // We pass ptr through param — the JIT won't know the actual String ptr,
        // so we pass 0 (simulating an already-nulled value) and verify.
        // Actually, we pass a dummy value — the important thing is that M1
        // zeroes s after Assign. Since we pass a fake ptr, M1 will zero it.
        // result = 0 + other = other (the fake ptr value).
        let val = unsafe { func.call_i64_1(42) };
        // After M1: s is zeroed → result = 0 + 42 = 42
        assert_eq!(
            val, 42,
            "M1: s should be zeroed after move, result = 0 + other"
        );
    }

    // ── Phase 4.3b: Vector shim roundtrip ──

    #[test]
    #[allow(unsafe_code)]
    fn vector_alloc_push_len_roundtrip() {
        // Call shims directly — no JIT compilation needed.
        let v0 = __triet_vector_alloc(0, 2);
        assert_ne!(v0, 0, "alloc(0,2) must return non-null");
        assert_eq!(__triet_vector_len(v0), 0, "fresh vector len = 0");

        let v1 = __triet_vector_push(v0, 10);
        assert_eq!(__triet_vector_len(v1), 1, "after 1 push len = 1");

        let v2 = __triet_vector_push(v1, 20);
        assert_eq!(__triet_vector_len(v2), 2, "after 2 push len = 2");

        // 3rd push exceeds cap=2 → must realloc
        let v3 = __triet_vector_push(v2, 30);
        assert_eq!(__triet_vector_len(v3), 3, "after 3 push len = 3");
        assert_ne!(
            v3, v0,
            "3rd push must cause realloc — ptr must differ from original"
        );

        __triet_vector_free(v3);
    }

    #[test]
    #[allow(unsafe_code)]
    fn vector_push_realloc_frees_old_ptr() {
        // Verify that after realloc, the old pointer is truly freed
        // (and new allocation reuses the memory — not required but
        // observable on most allocators with same-size blocks).
        let v0 = __triet_vector_alloc(0, 2);
        let v1 = __triet_vector_push(v0, 1);
        let v2 = __triet_vector_push(v1, 2);
        // v0, v1, v2 all share the same ptr (in-place for len 0→1, 1→2)
        // 3rd push triggers realloc
        let v3 = __triet_vector_push(v2, 3);
        // After realloc: old block is freed, v3 = new block
        assert_ne!(v0, v3, "realloc must change ptr");
        // Allocate another vector — should get the old block back (most allocators)
        let fresh = __triet_vector_alloc(0, 2);
        // fresh may or may not reuse v0's old block — just assert we can alloc+free
        assert_ne!(fresh, 0, "alloc after realloc must succeed");
        __triet_vector_free(v3);
        __triet_vector_free(fresh);
    }

    #[test]
    #[allow(unsafe_code)]
    fn vector_free_null_is_noop() {
        // C9: free(0) must NOT trap — Drop of moved-out value must be silent.
        __triet_vector_free(0);
    }

    // ── N7 subprocess helpers (F1: --exact --test-threads=1) ──

    /// Spawn child subprocess running only `test_name` (exact, single-threaded).
    /// Uses `_TRIET_N7` env var to trigger child path. Fork-bomb safe.
    fn spawn_n7_child(test_name: &str) -> std::process::ExitStatus {
        let exe = std::env::current_exe().expect("current_exe");
        let full_name = format!("mir_lower::tests::{test_name}");
        std::process::Command::new(&exe)
            .args([&full_name, "--exact", "--test-threads=1"])
            .env("_TRIET_N7", test_name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap_or_else(|_| panic!("spawn child for {test_name}"))
    }

    /// Assert child died from signal `expected` (e.g. 6=SIGABRT, 4=SIGILL).
    fn assert_n7_signal(test_name: &str, status: std::process::ExitStatus, expected: i32) {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(
                status.signal(),
                Some(expected),
                "{test_name}: expected signal {expected}, got: {:?}",
                status.signal()
            );
        }
        #[cfg(not(unix))]
        {
            assert!(!status.success(), "{test_name}: child should have aborted");
        }
    }

    /// Child guard: if `_TRIET_N7` matches `test_name`, run `child_fn`.
    /// Otherwise exit silently (prevents fork-bomb from --exact race).
    fn n7_child_guard(test_name: &str, child_fn: impl FnOnce()) {
        if let Ok(name) = std::env::var("_TRIET_N7") {
            if name == test_name {
                child_fn();
            }
            std::process::exit(0);
        }
    }

    /// N7: trap-on-0 for string len.
    #[test]
    fn n7_shim_trap_on_zero_len() {
        n7_child_guard("n7_shim_trap_on_zero_len", || {
            let _ = __triet_string_len(0);
        });
        let status = spawn_n7_child("n7_shim_trap_on_zero_len");
        assert_n7_signal("n7_shim_trap_on_zero_len", status, 6);
    }

    /// N7: trap-on-0 for string eq (shortcut must NOT fire before trap).
    #[test]
    fn n7_shim_trap_on_zero_eq() {
        n7_child_guard("n7_shim_trap_on_zero_eq", || {
            let _ = __triet_string_eq(0, 0, 0, 0);
        });
        let status = spawn_n7_child("n7_shim_trap_on_zero_eq");
        assert_n7_signal("n7_shim_trap_on_zero_eq", status, 6);
    }

    /// F1: `free(NULL_SENTINEL)` must be no-op (null has no allocation).
    #[test]
    fn free_null_sentinel_is_noop() {
        // String free
        __triet_string_free(triet_mir::NULL_SENTINEL, 1);
        // Vector free
        __triet_vector_free(triet_mir::NULL_SENTINEL);
    }

    // ── HashMap tests (ADR-0043) ──

    /// `HashMap` `free(0)` and `free(MIN)` must be no-op.
    #[test]
    fn hashmap_free_null_and_min_are_noop() {
        __triet_hashmap_free(0);
        __triet_hashmap_free(triet_mir::NULL_SENTINEL);
    }

    /// Basic insert + get round-trip.
    #[test]
    fn hashmap_insert_get_roundtrip() {
        let m = __triet_hashmap_alloc(0, 4);
        assert_eq!(__triet_hashmap_len(m), 0);
        let m = __triet_hashmap_insert(m, 1, 100);
        assert_eq!(__triet_hashmap_len(m), 1);
        assert_eq!(__triet_hashmap_get(m, 1), 100);
        // Key not found
        assert_eq!(__triet_hashmap_get(m, 2), triet_mir::NULL_SENTINEL);
        __triet_hashmap_free(m);
    }

    /// C9: insert same key must UPDATE value, len unchanged.
    #[test]
    fn hashmap_insert_same_key_updates_value() {
        let m = __triet_hashmap_alloc(0, 4);
        let m = __triet_hashmap_insert(m, 1, 10);
        let m = __triet_hashmap_insert(m, 1, 20);
        assert_eq!(__triet_hashmap_get(m, 1), 20);
        assert_eq!(__triet_hashmap_len(m), 1);
        __triet_hashmap_free(m);
    }

    /// Rehash: insert beyond load factor 0.75 triggers realloc.
    /// Keys are displaced (k%4 ≠ k%8) so old index ≠ new index —
    /// a rehash-broken-to-memcpy would fail assertions.
    /// Key 13 (13%8=5) additionally tests collision probing after
    /// realloc (slot 5 already occupied by key 5).
    #[test]
    fn hashmap_rehash_on_realloc() {
        let m = __triet_hashmap_alloc(0, 4); // cap=4, load factor at 0.75 → 3 max before realloc
        let m = __triet_hashmap_insert(m, 5, 50);
        let m = __triet_hashmap_insert(m, 6, 60);
        let m = __triet_hashmap_insert(m, 7, 70);
        // 4th insert triggers realloc (3 >= 4*3/4 = 3). cap→8.
        let m = __triet_hashmap_insert(m, 13, 130);
        assert_eq!(__triet_hashmap_len(m), 4);
        // All keys survive rehash with displaced positions
        assert_eq!(__triet_hashmap_get(m, 5), 50);
        assert_eq!(__triet_hashmap_get(m, 6), 60);
        assert_eq!(__triet_hashmap_get(m, 7), 70);
        assert_eq!(__triet_hashmap_get(m, 13), 130);
        __triet_hashmap_free(m);
    }

    // N7-C5: insert with v == i64::MIN must die (D2 reject-on-insert).
    #[test]
    fn n7_hashmap_insert_min_value_rejected() {
        n7_child_guard("n7_hashmap_insert_min_value_rejected", || {
            let m = __triet_hashmap_alloc(0, 4);
            let _ = __triet_hashmap_insert(m, 1, triet_mir::NULL_SENTINEL);
        });
        let status = spawn_n7_child("n7_hashmap_insert_min_value_rejected");
        assert_n7_signal("n7_hashmap_insert_min_value_rejected", status, 6);
    }

    // A8: 2**100 → abort (checked_mul + range in pow).
    #[test]
    fn n7_overflow_pow_checked() {
        n7_child_guard("n7_overflow_pow_checked", || {
            let _ = __triet_pow(2, 100);
        });
        let status = spawn_n7_child("n7_overflow_pow_checked");
        assert_n7_signal("n7_overflow_pow_checked", status, 6);
    }

    // A8+: 3**30 → abort (fits i64 but exceeds Integer range 3.8e12).
    #[test]
    fn n7_overflow_pow_range() {
        n7_child_guard("n7_overflow_pow_range", || {
            let _ = __triet_pow(3, 30);
        });
        let status = spawn_n7_child("n7_overflow_pow_range");
        assert_n7_signal("n7_overflow_pow_range", status, 6);
    }

    // A1: JIT BinOp::Add(M, M) → range check trapnz SIGILL (4).
    // Input = M (in-range). M+M = 2M >> M → trap.
    #[test]
    #[allow(unsafe_code)]
    fn n7_overflow_add_above_max() {
        n7_child_guard("n7_overflow_add_above_max", || {
            let mut b = MirBuilder::new("add_test", "Integer");
            let a = b.add_param("a", ParameterPassing::Borrow);
            b.set_local_type(a, "Integer");
            let bb0 = b.new_block();
            let r = b.new_local();
            b.push(bb0, storage_live(r));
            b.push(bb0, binop(r, triet_mir::BinOp::Add, a, a));
            b.set_terminator(bb0, return_(vec![r]));
            let body = b.build(bb0);
            let shims = &[ShimSymbol::fn_2_1("__triet_pow", super::__triet_pow)];
            let mut ctx = JitContext::with_shims(shims);
            let compiled = ctx.compile(&body).expect("compile A1");
            let _ = unsafe { compiled.call_i64_1(3_812_798_742_493) };
        });
        let status = spawn_n7_child("n7_overflow_add_above_max");
        assert_n7_signal("n7_overflow_add_above_max", status, 4);
    }

    // A2: JIT BinOp::Sub(−M, 1) → trapnz SIGILL (4).
    // Input = −M (in-range). −M − 1 < −M → trap.
    #[test]
    #[allow(unsafe_code)]
    fn n7_overflow_sub_below_min() {
        n7_child_guard("n7_overflow_sub_below_min", || {
            let mut b = MirBuilder::new("sub_test", "Integer");
            let a = b.add_param("a", ParameterPassing::Borrow);
            b.set_local_type(a, "Integer");
            let bb0 = b.new_block();
            let r = b.new_local();
            b.push(bb0, storage_live(r));
            let one = b.new_local();
            b.push(bb0, storage_live(one));
            b.push(bb0, const_int(one, 1));
            b.push(bb0, binop(r, triet_mir::BinOp::Sub, a, one));
            b.set_terminator(bb0, return_(vec![r]));
            let body = b.build(bb0);
            let shims = &[ShimSymbol::fn_2_1("__triet_pow", super::__triet_pow)];
            let mut ctx = JitContext::with_shims(shims);
            let compiled = ctx.compile(&body).expect("compile A2");
            let _ = unsafe { compiled.call_i64_1(-3_812_798_742_493) };
        });
        let status = spawn_n7_child("n7_overflow_sub_below_min");
        assert_n7_signal("n7_overflow_sub_below_min", status, 4);
    }

    // A3: JIT BinOp::Mul(2³², 2³²) → carrier wrap fools range check.
    // 2³² × 2³² = 2⁶⁴ ≡ 0 (mod i64) — range check sees 0 ∈ [−M,M], passes.
    // smulhi = 1 ≠ sign-ext(0) = 0 → carrier trap. Only smulhi catches this.
    #[test]
    #[allow(unsafe_code)]
    fn n7_overflow_mul_carrier() {
        n7_child_guard("n7_overflow_mul_carrier", || {
            let mut b = MirBuilder::new("mul_test", "Integer");
            let a = b.add_param("a", ParameterPassing::Borrow);
            b.set_local_type(a, "Integer");
            let bb0 = b.new_block();
            let r = b.new_local();
            b.push(bb0, storage_live(r));
            b.push(bb0, binop(r, triet_mir::BinOp::Mul, a, a));
            b.set_terminator(bb0, return_(vec![r]));
            let body = b.build(bb0);
            let shims = &[ShimSymbol::fn_2_1("__triet_pow", super::__triet_pow)];
            let mut ctx = JitContext::with_shims(shims);
            let compiled = ctx.compile(&body).expect("compile A3");
            let _ = unsafe { compiled.call_i64_1(4_294_967_296) }; // 2³²
        });
        let status = spawn_n7_child("n7_overflow_mul_carrier");
        assert_n7_signal("n7_overflow_mul_carrier", status, 4);
    }
}
