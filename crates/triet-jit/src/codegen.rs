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
use cranelift_codegen::ir::{
    AbiParam, Block, InstBuilder, Signature, StackSlotData, StackSlotKind, Value, types,
};
use cranelift_codegen::isa::{CallConv, OwnedTargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId as ClFuncId, Linkage, Module};
use triet_ir::{
    BlockId, ConstId, Constant, ConstantPool, FuncId as TriFuncId, Function as IrFunction,
    Instruction, IrProgram, JitBinOp, JitConstKind, Operand, TypeTag, ValueId,
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
    /// v0.10.x.jit.1 — capability namespaces denied for this program
    /// (per ADR-0032 §3 compile-time defense-in-depth). Empty in the
    /// production path (capabilities already resolved at program-load
    /// time per ADR-0016 §5); the framework test passes a non-empty
    /// set to exercise the `BuiltinCapabilityDenied` tier-down.
    denied_namespaces: &'a [&'a str],
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
        // `Integer` (primitive) + composites all map to `i64`: Integer
        // is a 64-bit value; composites cross the shim ABI as `i64` raw
        // pointers (`Rc::into_raw` boxed `RuntimeValue`) per ADR-0032 §1.
        // Composite coverage grows per sub-task:
        //   - jit.2b-i: String, Vector, HashMap, Nullable.
        //   - jit.2b-ii: Atomic, Outcome (compare_exchange return).
        // (TypeTag has no Enum/Struct variant — user aggregates lower
        // via EnumNew/struct ops, not JIT-supported yet, so they tier
        // down at construction.) Tuple / Range also tier-down for now.
        TypeTag::Integer
        | TypeTag::String
        | TypeTag::Vector(_)
        | TypeTag::HashMap(..)
        | TypeTag::Nullable(_)
        | TypeTag::Atomic(_)
        | TypeTag::Outcome { .. } => I64,
        // Long (i128) needs pair-of-i64 lowering per ADR-0030 §3 — defer.
        // (Exhaustive match — no catch-all: a future `TypeTag` variant
        // will fail to compile here until explicitly mapped, preventing
        // a silent ABI miscompile of an unhandled type.)
        TypeTag::Long => {
            return Err(JitError::UnsupportedOpcode {
                opcode: "Long type (i128) — defer to later sub-phase".to_string(),
            });
        }
    })
}

/// Build a configured host [`OwnedTargetIsa`] via `cranelift-native`
/// target detection.
///
/// Shared by the `cranelift-jit` backend (Path B, `pic = false` —
/// in-process absolute addressing, unchanged from v0.10) and the
/// `cranelift-object` backend (Path A, `pic = true` so the emitted
/// `.o` uses PC-relative relocations — a relocatable object loaded at
/// an arbitrary `mmap` address per [ADR-0033 §1]). The exact
/// relocation set this produces is verified empirically by the
/// `aot` round-trip test, not assumed (Addendum safety constraint 3).
///
/// [ADR-0033 §1]: ../../../docs/decisions/0033-aot-cache-cranelift-object.md
pub(crate) fn build_host_isa(pic: bool) -> Result<OwnedTargetIsa, JitError> {
    let mut flag_builder = settings::builder();
    if pic {
        flag_builder
            .set("is_pic", "true")
            .map_err(|err| JitError::Cranelift {
                message: format!("set is_pic: {err}"),
            })?;
    }
    let isa_builder = cranelift_native::builder().map_err(|message| JitError::Cranelift {
        message: format!("ISA detection failed: {message}"),
    })?;
    isa_builder
        .finish(settings::Flags::new(flag_builder))
        .map_err(|err| JitError::Cranelift {
            message: format!("ISA finish failed: {err}"),
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
        let isa = build_host_isa(false)?;
        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        // v0.10.x.jit.1 — register builtin-shim symbols per ADR-0032 §6
        // so emitted `call $shim_symbol` instructions resolve at
        // finalize time. jit.1 registers only `__triet_drop_arc`; jit.2
        // appends the 43 production shims to `production_shim_entries`.
        register_shim_symbols(&mut builder, &crate::shims::production_shim_entries());
        let module = JITModule::new(builder);
        Ok(Self { module })
    }

    /// v0.10.x.jit.1 (test-support) — construct a backend with extra
    /// shim symbols registered alongside the production set. Used by
    /// the framework smoke tests to wire synthetic `__triet_test_*`
    /// shims without baking them into the production registry.
    #[cfg(test)]
    pub(crate) fn new_with_extra_shims(
        extra: &[crate::shims::ShimEntry],
    ) -> Result<Self, JitError> {
        let isa = build_host_isa(false)?;
        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        register_shim_symbols(&mut builder, &crate::shims::production_shim_entries());
        register_shim_symbols(&mut builder, extra);
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
            denied_namespaces: &[],
        };
        let signature = build_signature_for(func)?;
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
        emit_function_body(&mut self.module, func, &ctx, &mut cl_ctx, is_boxed(func))?;
        self.module
            .define_function(func_id, &mut cl_ctx)
            .map_err(cranelift_err)?;
        self.module.clear_context(&mut cl_ctx);
        self.module.finalize_definitions().map_err(cranelift_err)?;
        let raw_ptr = self.module.get_finalized_function(func_id);
        Ok(raw_ptr as usize)
    }

    /// v0.10.x.jit.1 (test-support) — compile a synthetic caller that
    /// invokes a registered shim symbol and returns its result,
    /// exercising the ADR-0032 §6 external-call mechanism (declare
    /// `Import` shim + `declare_func_in_func` + `builder.ins().call`)
    /// in isolation — the exact machinery v0.10.x.jit.2's `CallBuiltin`
    /// codegen will use.
    ///
    /// The caller forwards all its parameters to the shim. If the shim
    /// returns a value, the caller returns it; if the shim is `Unit`
    /// (`ret: None`), the caller returns `iconst 0` so it still matches
    /// the all-`i64` [`crate::dispatch_integer_caught`] ABI.
    #[cfg(test)]
    pub(crate) fn compile_shim_caller(
        &mut self,
        caller_sig: &crate::shims::ShimSignature,
        shim_symbol: &str,
        shim_sig: &crate::shims::ShimSignature,
    ) -> Result<usize, JitError> {
        // Declare the shim as an imported function (resolved to the
        // address registered via `JITBuilder::symbol`).
        let shim_clif_sig = shim_signature_to_clif(shim_sig);
        let shim_id = self
            .module
            .declare_function(shim_symbol, Linkage::Import, &shim_clif_sig)
            .map_err(cranelift_err)?;

        // Declare the caller (Local).
        let caller_clif_sig = shim_signature_to_clif(caller_sig);
        let caller_id = self
            .module
            .declare_function("__triet_test_caller", Linkage::Local, &caller_clif_sig)
            .map_err(cranelift_err)?;

        let mut cl_ctx = self.module.make_context();
        cl_ctx.func.signature = caller_clif_sig;
        {
            let mut fn_builder_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut cl_ctx.func, &mut fn_builder_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);
            let arg_values: Vec<Value> = builder.block_params(entry).to_vec();
            let shim_ref = self.module.declare_func_in_func(shim_id, builder.func);
            let call_inst = builder.ins().call(shim_ref, &arg_values);
            let ret_val = if shim_sig.ret.is_some() {
                builder.inst_results(call_inst)[0]
            } else {
                // Unit shim — caller returns an i64 0 placeholder.
                builder.ins().iconst(I64, 0)
            };
            builder.ins().return_(&[ret_val]);
            builder.finalize();
        }
        self.module
            .define_function(caller_id, &mut cl_ctx)
            .map_err(cranelift_err)?;
        self.module.clear_context(&mut cl_ctx);
        self.module.finalize_definitions().map_err(cranelift_err)?;
        Ok(self.module.get_finalized_function(caller_id) as usize)
    }

    /// Compile every function in `program` and collect the finalized
    /// native pointers into `out_cache`, keyed by Triết `FuncId`.
    ///
    /// The shared declare+define translation runs via
    /// [`declare_and_define_program`] (the backend-agnostic pass per
    /// [ADR-0033 §1]); this method adds the `cranelift-jit`-specific
    /// finalize: a single `finalize_definitions` flips all bodies from
    /// RW to RX, then `get_finalized_function` resolves each compiled
    /// function's host address.
    ///
    /// Per-function tier-down (per ADR-0030 §2) happens inside the
    /// shared pass — only successfully-defined functions reach
    /// `out_cache`. Returns Err only on a finalize failure that
    /// prevents the whole program from JIT-ing.
    ///
    /// [ADR-0033 §1]: ../../../docs/decisions/0033-aot-cache-cranelift-object.md
    pub(crate) fn compile_program(
        &mut self,
        program: &IrProgram,
        out_cache: &mut HashMap<TriFuncId, NativeCodePtr>,
        denied_namespaces: &[&str],
    ) -> Result<(), JitError> {
        let translated = declare_and_define_program(&mut self.module, program, denied_namespaces)?;
        // Finalize everything together. Single mmap-flip across all
        // bodies — required before `get_finalized_function`.
        self.module.finalize_definitions().map_err(cranelift_err)?;
        for tri_id in translated.compiled {
            let cl_id = translated.func_id_map[&tri_id];
            let raw = self.module.get_finalized_function(cl_id);
            out_cache.insert(tri_id, NativeCodePtr { addr: raw as usize });
        }
        Ok(())
    }

    /// v0.11.x (Hướng A) — diagnostic: attempt to JIT-translate every
    /// function in `program` and return the ones that tier down + why.
    /// Delegates to [`collect_tier_downs`] (resilient measurement, no
    /// finalize).
    pub(crate) fn audit(
        &mut self,
        program: &IrProgram,
    ) -> Vec<(TriFuncId, Option<String>, String)> {
        collect_tier_downs(&mut self.module, program)
    }
}

/// Outcome of [`declare_and_define_program`] — the shared declare+define
/// translation of an [`IrProgram`] into a Cranelift [`Module`], used by
/// both backends per [ADR-0033 §1]. The caller performs the
/// backend-specific finalization (`cranelift-jit` finalize +
/// `get_finalized_function`, or `cranelift-object` `finish` + `emit`).
///
/// [ADR-0033 §1]: ../../../docs/decisions/0033-aot-cache-cranelift-object.md
pub(crate) struct TranslatedProgram {
    /// Triết `FuncId` → Cranelift `FuncId` for every declared function.
    pub func_id_map: HashMap<TriFuncId, ClFuncId>,
    /// Functions whose body was successfully defined. Functions that
    /// tiered down (per ADR-0030 §2) are absent — finalize/cache only
    /// these.
    pub compiled: Vec<TriFuncId>,
    /// Triết `FuncId` → the mangled symbol name used at
    /// `declare_function` time. The AOT manifest records these so the
    /// Path-A loader can locate each function's offset in the loaded
    /// `.o` per [ADR-0033 §2].
    pub symbol_names: HashMap<TriFuncId, String>,
}

/// Run the two-pass declare+define translation of `program` into
/// `module`, generic over the Cranelift backend per [ADR-0033 §1] so
/// the `cranelift-jit` (Path B) and `cranelift-object` (Path A) paths
/// share one codegen pipeline.
///
/// 1. **Pre-pass:** for each Triết function build its Cranelift
///    signature + `declare_function` (mangled `name__f{id}` so two
///    modules can share a simple name). Populates the Triết→Cranelift
///    id map, the `AbsolutePath`→Triết map for call resolution, and the
///    `FuncId`→symbol-name map for the AOT manifest.
/// 2. **Body pass:** for each function emit its Cranelift IR body via
///    [`emit_function_body`] with full program context, then
///    `define_function`. On a per-function error the function is
///    skipped (tier-down per ADR-0030 §2); the rest continue.
///
/// Returns Err only on a pre-pass `declare_function` failure.
///
/// [ADR-0033 §1]: ../../../docs/decisions/0033-aot-cache-cranelift-object.md
/// [ADR-0033 §2]: ../../../docs/decisions/0033-aot-cache-cranelift-object.md
pub(crate) fn declare_and_define_program(
    module: &mut impl Module,
    program: &IrProgram,
    denied_namespaces: &[&str],
) -> Result<TranslatedProgram, JitError> {
    // Pre-pass: declare every function so calls can resolve.
    let mut func_id_map: HashMap<TriFuncId, ClFuncId> = HashMap::new();
    let mut path_to_funcid: HashMap<AbsolutePath, TriFuncId> = HashMap::new();
    let mut symbol_names: HashMap<TriFuncId, String> = HashMap::new();
    for ir_module in &program.modules {
        for func in &ir_module.functions {
            let signature = build_signature_for(func)?;
            let func_name = func
                .name
                .clone()
                .unwrap_or_else(|| format!("@f{}", func.id.0));
            // Mangle name with FuncId so two modules can share a
            // simple name (`main`, `helper`) without collision.
            let mangled = format!("{}__f{}", func_name, func.id.0);
            let cl_id = module
                .declare_function(&mangled, Linkage::Local, &signature)
                .map_err(cranelift_err)?;
            func_id_map.insert(func.id, cl_id);
            symbol_names.insert(func.id, mangled);
            if let Some(name) = &func.name {
                // `IrModule.path` is an AbsolutePath with empty item
                // name per lowerer convention (`module.rs` line 147).
                // Extract its `ModulePath` and re-wrap with `name`.
                let path = AbsolutePath::new(ir_module.path.module_path().clone(), name.clone());
                path_to_funcid.insert(path, func.id);
            }
        }
    }

    let ctx = ProgramContext {
        func_id_map: func_id_map.clone(),
        path_to_funcid,
        constants: &program.constants,
        denied_namespaces,
    };

    // Body pass: per-function codegen + define. On per-function error,
    // skip (tier-down) without aborting the whole program.
    let mut compiled: Vec<TriFuncId> = Vec::new();
    for ir_module in &program.modules {
        for func in &ir_module.functions {
            let cl_id = match func_id_map.get(&func.id) {
                Some(id) => *id,
                None => continue,
            };
            let mut cl_ctx = module.make_context();
            cl_ctx.func.signature = build_signature_for(func)?;
            if let Err(err) = emit_function_body(module, func, &ctx, &mut cl_ctx, is_boxed(func)) {
                // Tier-down: skip this function, others still compile.
                let _ = err;
                module.clear_context(&mut cl_ctx);
                continue;
            }
            if let Err(err) = module.define_function(cl_id, &mut cl_ctx) {
                let _ = err;
                module.clear_context(&mut cl_ctx);
                continue;
            }
            module.clear_context(&mut cl_ctx);
            compiled.push(func.id);
        }
    }

    Ok(TranslatedProgram {
        func_id_map,
        compiled,
        symbol_names,
    })
}

/// Per-**module** translation for AOT object emission (ADR-0033
/// v0.11.0.2): declare + define ONLY `program.modules[local_idx]`'s
/// functions, so each module becomes its own cacheable `.o` keyed by
/// its `impl_hash_mod`.
///
/// Linkage split that makes the load-time linker work:
/// - **Local module's functions → `Export`** (globally referenceable):
///   defined here, and visible to *other* modules' objects at load.
/// - **Other modules' functions → `Import`** (undefined externals):
///   declared but not defined, so a cross-module call lowers to an
///   external relocation (GOTPCREL) the linker resolves against the
///   global symbol table. Unreferenced imports emit no relocation.
/// - Shims stay `Import` (handled inside `emit_function_body`).
///
/// The mangled symbol `{name}__f{func_id}` is program-global-unique, so
/// a caller module's `Import` matches the definer module's `Export` by
/// name. Same translator core (`emit_function_body`) as the
/// whole-program path.
///
/// # Errors
/// [`JitError::UnsupportedOpcode`] if `local_idx` is out of range, or a
/// `declare_function` failure.
pub(crate) fn declare_and_define_module(
    module: &mut impl Module,
    program: &IrProgram,
    local_idx: usize,
    denied_namespaces: &[&str],
) -> Result<TranslatedProgram, JitError> {
    if local_idx >= program.modules.len() {
        return Err(JitError::UnsupportedOpcode {
            opcode: format!("module index {local_idx} out of range"),
        });
    }

    // Pre-pass: declare every program function so calls resolve —
    // local ones `Export` (defined below), the rest `Import`.
    let mut func_id_map: HashMap<TriFuncId, ClFuncId> = HashMap::new();
    let mut path_to_funcid: HashMap<AbsolutePath, TriFuncId> = HashMap::new();
    let mut symbol_names: HashMap<TriFuncId, String> = HashMap::new();
    for (m_idx, ir_module) in program.modules.iter().enumerate() {
        let is_local = m_idx == local_idx;
        for func in &ir_module.functions {
            let signature = build_signature_for(func)?;
            let func_name = func
                .name
                .clone()
                .unwrap_or_else(|| format!("@f{}", func.id.0));
            let mangled = format!("{}__f{}", func_name, func.id.0);
            let linkage = if is_local {
                Linkage::Export
            } else {
                Linkage::Import
            };
            let cl_id = module
                .declare_function(&mangled, linkage, &signature)
                .map_err(cranelift_err)?;
            func_id_map.insert(func.id, cl_id);
            if is_local {
                symbol_names.insert(func.id, mangled);
            }
            if let Some(name) = &func.name {
                let path = AbsolutePath::new(ir_module.path.module_path().clone(), name.clone());
                path_to_funcid.insert(path, func.id);
            }
        }
    }

    let ctx = ProgramContext {
        func_id_map: func_id_map.clone(),
        path_to_funcid,
        constants: &program.constants,
        denied_namespaces,
    };

    // Body pass: define ONLY the local module's functions. Per-function
    // tier-down (per ADR-0030 §2) skips just that function.
    let mut compiled: Vec<TriFuncId> = Vec::new();
    for func in &program.modules[local_idx].functions {
        let Some(&cl_id) = func_id_map.get(&func.id) else {
            continue;
        };
        let mut cl_ctx = module.make_context();
        cl_ctx.func.signature = build_signature_for(func)?;
        if emit_function_body(module, func, &ctx, &mut cl_ctx, is_boxed(func)).is_err() {
            module.clear_context(&mut cl_ctx);
            continue;
        }
        if module.define_function(cl_id, &mut cl_ctx).is_err() {
            module.clear_context(&mut cl_ctx);
            continue;
        }
        module.clear_context(&mut cl_ctx);
        compiled.push(func.id);
    }

    Ok(TranslatedProgram {
        func_id_map,
        compiled,
        symbol_names,
    })
}

/// v0.11.x (Hướng A) — measure the JIT-coverage gap: attempt to
/// translate every function in `program` and return `(func_id, name,
/// reason)` for each that **tiers down**, WITHOUT finalizing or
/// executing. This bounds the work needed to make a program (the
/// self-host compiler) fully JIT-able, so the bootstrap byte-identical
/// gate can be lifted (ROADMAP v0.11).
///
/// **Resilient where [`declare_and_define_program`] aborts:** a function
/// whose *signature* is unsupported (e.g. a `Long` param) is recorded as
/// a tier-down and skipped rather than aborting the whole pass — so the
/// report covers every function, not just up to the first hard failure.
/// Only the opcode-translation stage (`emit_function_body`) is run, not
/// `define_function`/finalize: `UnsupportedOpcode` from translation is
/// the coverage signal we're measuring (verifier failures are a
/// separate, rare class out of scope here).
pub(crate) fn collect_tier_downs(
    module: &mut impl Module,
    program: &IrProgram,
) -> Vec<(TriFuncId, Option<String>, String)> {
    let mut tier_downs: Vec<(TriFuncId, Option<String>, String)> = Vec::new();

    // Resilient pre-pass: declare every function whose signature maps;
    // record signature failures (e.g. `Long`) as tier-downs.
    let mut func_id_map: HashMap<TriFuncId, ClFuncId> = HashMap::new();
    let mut path_to_funcid: HashMap<AbsolutePath, TriFuncId> = HashMap::new();
    for ir_module in &program.modules {
        for func in &ir_module.functions {
            let Ok(signature) = build_signature_for(func) else {
                tier_downs.push((
                    func.id,
                    func.name.clone(),
                    format!(
                        "unsupported signature type ({})",
                        build_signature_for(func).unwrap_err()
                    ),
                ));
                continue;
            };
            let func_name = func
                .name
                .clone()
                .unwrap_or_else(|| format!("@f{}", func.id.0));
            let mangled = format!("{}__f{}", func_name, func.id.0);
            if let Ok(cl_id) = module.declare_function(&mangled, Linkage::Local, &signature) {
                func_id_map.insert(func.id, cl_id);
                if let Some(name) = &func.name {
                    let path =
                        AbsolutePath::new(ir_module.path.module_path().clone(), name.clone());
                    path_to_funcid.insert(path, func.id);
                }
            }
        }
    }

    let ctx = ProgramContext {
        func_id_map: func_id_map.clone(),
        path_to_funcid,
        constants: &program.constants,
        denied_namespaces: &[],
    };

    // Body-pass: attempt opcode translation per function; record the
    // reason on the first unsupported opcode/constant. Each function is
    // wrapped in `catch_unwind` because the translator can *panic* (not
    // just error) on malformed-for-Cranelift IR — e.g. a Cranelift
    // "instruction added to a filled block" assertion. A panic is a
    // worse failure than a clean tier-down (it would abort the real JIT
    // mid-`compile_program`), so the audit records it as its own
    // category rather than aborting the measurement. `AssertUnwindSafe`
    // is sound here: the module is a throwaway discarded after the audit.
    for ir_module in &program.modules {
        for func in &ir_module.functions {
            if !func_id_map.contains_key(&func.id) {
                continue; // signature already recorded as a tier-down
            }
            // Safe: only functions whose signature mapped are in the map.
            let signature = build_signature_for(func).expect("signature mapped in pre-pass");
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut cl_ctx = module.make_context();
                cl_ctx.func.signature = signature;
                let r = emit_function_body(module, func, &ctx, &mut cl_ctx, is_boxed(func));
                module.clear_context(&mut cl_ctx);
                r
            }));
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(err)) => tier_downs.push((func.id, func.name.clone(), format!("{err}"))),
                Err(panic) => {
                    let msg = panic
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_string())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown".to_string());
                    tier_downs.push((
                        func.id,
                        func.name.clone(),
                        format!("translator PANIC: {msg}"),
                    ));
                }
            }
        }
    }

    tier_downs
}

/// Shared body-emit routine called by both the single-function and the
/// program-level paths. Threads `ProgramContext` for call dispatch +
/// constant pool access.
///
/// Generic over [`Module`] so the **same** Triết IR translator drives
/// both the `cranelift-jit` (Path B fresh-compile) and the
/// `cranelift-object` (Path A AOT-persist) backends per
/// [ADR-0033 §1] — one codegen pipeline, two emission targets. Only
/// the [`Module`] trait surface (`declare_function` /
/// `declare_func_in_func`) is used here; backend-specific finalize /
/// emit stays in the per-backend wrappers.
///
/// [ADR-0033 §1]: ../../../docs/decisions/0033-aot-cache-cranelift-object.md
fn emit_function_body(
    module: &mut impl Module,
    func: &IrFunction,
    ctx: &ProgramContext<'_>,
    cl_ctx: &mut cranelift_codegen::Context,
    boxed: bool,
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
    // emit per-instruction Cranelift IR. `fn_state` carries the
    // jit.2a composite-flow bookkeeping (created boxed values for
    // drop_arc emission + shim-call count for the single-call scope).
    let mut fn_state = FnState::default();
    for ir_block in &func.blocks {
        let cl_block = block_map[&ir_block.id];
        builder.switch_to_block(cl_block);
        for instr in &ir_block.instructions {
            if boxed {
                // Bậc A uniform boxing (ADR-0034 Addendum): every value is
                // a boxed RuntimeValue ptr; aggregate ops delegate to VM
                // shims. A separate translator handles the boxed opcode
                // subset + tier-downs the rest.
                translate_boxed_instruction(
                    &mut builder,
                    module,
                    &mut value_map,
                    ctx,
                    &mut fn_state,
                    instr,
                )?;
            } else {
                translate_instruction(
                    &mut builder,
                    module,
                    &mut value_map,
                    &block_map,
                    ctx,
                    func,
                    &mut fn_state,
                    instr,
                )?;
            }
            // Stop at the block's terminator. The lowerer can emit dead
            // instructions AFTER a terminator within one block (e.g. an
            // early `return` whose lexical-block continuation is still
            // appended) — the VM ignores them (it halts at the
            // terminator), but Cranelift panics ("instruction added to a
            // block already filled") if we emit into the now-filled
            // block. Skipping the unreachable tail is observably
            // equivalent to the VM + is required for correctness. (Per
            // ADR-0034 §6 — fixes the 10 translator panics in the
            // self-host compiler's equality helpers.)
            if matches!(
                instr,
                Instruction::Ret { .. }
                    | Instruction::Br { .. }
                    | Instruction::BrIf { .. }
                    | Instruction::BrTrilean { .. }
                    | Instruction::Unreachable
            ) {
                break;
            }
        }
    }

    // v0.10.x.jit.2b-i — emit the shared `error_exit` block's body
    // (created lazily on the first shim call per ADR-0032 §4
    // option-2). It returns a type-correct sentinel; the
    // dispatcher's `SHIM_FAILED` check converts the run to `Err`
    // regardless of the sentinel value. Created composites leak on
    // this path (one-time per error — see `FnState::created_boxed`).
    if let Some(error_block) = fn_state.error_exit {
        builder.switch_to_block(error_block);
        // Boxed functions return an i64 ptr regardless of `return_type`
        // (which lies — struct/enum lower to TypeTag::Unit); the sentinel
        // must match that i64 return, not `map_type(return_type)`.
        let sentinel_ty = if boxed {
            I64
        } else {
            map_type(&func.return_type)?
        };
        let sentinel = builder.ins().iconst(sentinel_ty, 0);
        builder.ins().return_(&[sentinel]);
    }

    builder.seal_all_blocks();
    builder.finalize();
    Ok(())
}

/// v0.10.x.jit.2a/2b-i — per-function composite-flow bookkeeping
/// threaded through instruction translation.
#[derive(Default)]
struct FnState {
    /// SSA values the function CREATED as boxed composites (shim `Ptr`
    /// returns). At the function's `Ret`, each is dropped via
    /// `__triet_drop_arc` EXCEPT the returned one (whose ownership
    /// transfers to the caller). Composite PARAMS are NOT here —
    /// they're borrowed (caller owns + drops) per ADR-0032 §2 rule 1.
    /// On the error path (`error_exit`) these leak — a one-time leak
    /// per runtime error, acceptable for the dev-tier JIT (errors are
    /// typically program-terminating). Per ADR-0032 §2.
    created_boxed: Vec<ValueId>,
    /// Lazily-created shared `error_exit` Cranelift block (jit.2b-i §4
    /// option-2). On the first shim call the function gains an
    /// `error_exit` that returns a type-correct sentinel; each shim
    /// call's per-call probe branches here when `SHIM_FAILED` is set.
    /// Its body is emitted once, after the instruction loop.
    error_exit: Option<Block>,
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

/// Whether a function is compiled in **boxed** mode (ADR-0034 Addendum
/// Bậc A — per-function uniform boxing) or the unboxed integer fast
/// path. A function is boxed iff it uses an aggregate opcode whose value
/// model requires uniform boxing. This set grows per agg.* sub-task; at
/// agg.1 it is the struct opcodes (the other aggregate families tier
/// down until their boxed codegen lands).
fn is_boxed(func: &IrFunction) -> bool {
    func.blocks.iter().flat_map(|b| &b.instructions).any(|i| {
        matches!(
            i,
            Instruction::StructNew { .. }
                | Instruction::FieldGet { .. }
                | Instruction::FieldSet { .. }
        )
    })
}

/// Signature for a function honouring its mode: unboxed → per-type
/// (`build_signature`); boxed → every param + the return is an `i64`
/// boxed-`RuntimeValue` ptr (ADR-0034 Addendum — the `TypeTag`s lie,
/// e.g. struct lowers to `TypeTag::Unit`, so they are NOT consulted).
fn build_signature_for(func: &IrFunction) -> Result<Signature, JitError> {
    if !is_boxed(func) {
        return build_signature(func);
    }
    let mut sig = Signature::new(CallConv::SystemV);
    for _ in &func.params {
        sig.params.push(AbiParam::new(I64));
    }
    sig.returns.push(AbiParam::new(I64));
    Ok(sig)
}

/// Map a primitive [`Constant`] to its boxed-mode `(kind, payload)` wire
/// form for `__triet_box_const` (ADR-0034 §1). `String`/`Long` have no
/// i64 payload (data-section / i128 — agg.3) → `None` (tier down).
fn boxed_const_kind_payload(constant: &Constant) -> Option<(JitConstKind, i64)> {
    let pair = match constant {
        Constant::Trit(t) => (JitConstKind::Trit, i64::from(t.to_i8())),
        Constant::Tryte(t) => (JitConstKind::Tryte, t.to_i64()),
        Constant::Integer(i) => (JitConstKind::Integer, i.to_i64()),
        Constant::Trilean(t) => (
            JitConstKind::Trilean,
            match t {
                Trilean::False => -1,
                Trilean::Unknown => 0,
                Trilean::True => 1,
            },
        ),
        Constant::Unit => (JitConstKind::Unit, 0),
        Constant::Null => (JitConstKind::Null, 0),
        _ => return None, // String / Long — defer (agg.3)
    };
    Some(pair)
}

/// Resolve an [`Operand`] to its boxed (`i64` ptr) [`Value`] in a boxed
/// function. `Value(id)` is already a ptr in `value_map`; an inline
/// `Const` is materialized via the `__triet_box_const` shim (tracked as
/// a function-created box). `String`/`Long` constants tier down.
fn materialize_boxed_operand(
    builder: &mut FunctionBuilder<'_>,
    module: &mut impl Module,
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
                    opcode: format!("ValueId({}) referenced before def (boxed)", id.0),
                })
        }
        Operand::Const(const_id) => {
            // Inline-const materialization: emit the box. It has no SSA
            // `dest`, so it can't join `created_boxed` (keyed by ValueId
            // for the drop-at-Ret pass) — the box leaks. Bounded + one-
            // time per inline-const operand, the same dev-tier-JIT leak
            // tolerance the error path already documents (ADR-0032 §2).
            // The statement form `Const { dest }` IS tracked + dropped.
            emit_boxed_const(builder, module, ctx, const_id)
        }
    }
}

/// Emit a `__triet_box_const` call materializing `const_id`, returning
/// the boxed ptr `Value`. `String`/`Long` constants tier down.
fn emit_boxed_const(
    builder: &mut FunctionBuilder<'_>,
    module: &mut impl Module,
    ctx: &ProgramContext<'_>,
    const_id: ConstId,
) -> Result<Value, JitError> {
    let constant = ctx
        .constants
        .get(const_id)
        .ok_or_else(|| JitError::Cranelift {
            message: format!("ConstId({}) missing from pool", const_id.0),
        })?;
    let (kind, payload) =
        boxed_const_kind_payload(constant).ok_or_else(|| JitError::UnsupportedOpcode {
            opcode: format!("boxed constant {constant:?} — defer (agg.3 String/Long)"),
        })?;
    let kind_v = builder.ins().iconst(I8, i64::from(kind as u8));
    let payload_v = builder.ins().iconst(I64, payload);
    emit_agg_shim(builder, module, "__triet_box_const", &[kind_v, payload_v])?.ok_or_else(|| {
        JitError::Cranelift {
            message: "__triet_box_const returned no value".to_string(),
        }
    })
}

/// Map a binary-scalar IR instruction to its [`JitBinOp`] + operands,
/// or `None` if it isn't one. Lets the boxed translator handle the whole
/// arithmetic/comparison/Ł3/K3 family through one `__triet_binop` shim.
const fn boxed_binop_of(instr: &Instruction) -> Option<(JitBinOp, ValueId, Operand, Operand)> {
    let (op, dest, lhs, rhs) = match *instr {
        Instruction::Add { dest, lhs, rhs } => (JitBinOp::Add, dest, lhs, rhs),
        Instruction::Sub { dest, lhs, rhs } => (JitBinOp::Sub, dest, lhs, rhs),
        Instruction::Mul { dest, lhs, rhs } => (JitBinOp::Mul, dest, lhs, rhs),
        Instruction::Div { dest, lhs, rhs } => (JitBinOp::Div, dest, lhs, rhs),
        Instruction::Mod { dest, lhs, rhs } => (JitBinOp::Mod, dest, lhs, rhs),
        Instruction::Pow { dest, base, exp } => (JitBinOp::Pow, dest, base, exp),
        Instruction::Eq { dest, lhs, rhs } => (JitBinOp::Eq, dest, lhs, rhs),
        Instruction::Ne { dest, lhs, rhs } => (JitBinOp::Ne, dest, lhs, rhs),
        Instruction::Lt { dest, lhs, rhs } => (JitBinOp::Lt, dest, lhs, rhs),
        Instruction::Le { dest, lhs, rhs } => (JitBinOp::Le, dest, lhs, rhs),
        Instruction::Gt { dest, lhs, rhs } => (JitBinOp::Gt, dest, lhs, rhs),
        Instruction::Ge { dest, lhs, rhs } => (JitBinOp::Ge, dest, lhs, rhs),
        Instruction::LukAnd { dest, lhs, rhs } => (JitBinOp::LukAnd, dest, lhs, rhs),
        Instruction::LukOr { dest, lhs, rhs } => (JitBinOp::LukOr, dest, lhs, rhs),
        Instruction::LukImplies { dest, lhs, rhs } => (JitBinOp::LukImplies, dest, lhs, rhs),
        Instruction::LukXor { dest, lhs, rhs } => (JitBinOp::LukXor, dest, lhs, rhs),
        Instruction::LukIff { dest, lhs, rhs } => (JitBinOp::LukIff, dest, lhs, rhs),
        Instruction::KleeneImplies { dest, lhs, rhs } => (JitBinOp::KleeneImplies, dest, lhs, rhs),
        Instruction::KleeneXor { dest, lhs, rhs } => (JitBinOp::KleeneXor, dest, lhs, rhs),
        Instruction::KleeneIff { dest, lhs, rhs } => (JitBinOp::KleeneIff, dest, lhs, rhs),
        _ => return None,
    };
    Some((op, dest, lhs, rhs))
}

/// Translate one IR instruction in **boxed** mode (ADR-0034 Addendum
/// Bậc A): every value is a boxed `RuntimeValue` ptr; aggregate +
/// scalar ops delegate to the `__triet_*` VM-shims. Handles struct ops
/// (agg.1b), binary/unary scalar ops (agg.1c-i), and constants
/// (agg.1c-ii); everything else tiers down.
#[allow(clippy::too_many_lines)]
fn translate_boxed_instruction(
    builder: &mut FunctionBuilder<'_>,
    module: &mut impl Module,
    value_map: &mut HashMap<ValueId, Value>,
    ctx: &ProgramContext<'_>,
    fn_state: &mut FnState,
    instr: &Instruction,
) -> Result<(), JitError> {
    match instr {
        Instruction::Const { dest, constant } => {
            let v = emit_boxed_const(builder, module, ctx, *constant)?;
            value_map.insert(*dest, v);
            fn_state.created_boxed.push(*dest);
        }
        Instruction::StructNew { dest, fields } => {
            // §2 array-ptr+len ABI: spill the resolved field ptrs into a
            // stack slot, pass its address + length to the shim.
            let field_vals: Vec<Value> = fields
                .iter()
                .map(|op| materialize_boxed_operand(builder, module, value_map, ctx, *op))
                .collect::<Result<_, _>>()?;
            let len = field_vals.len();
            let bytes = u32::try_from(len * 8).map_err(|_| JitError::UnsupportedOpcode {
                opcode: "struct field count overflow".to_string(),
            })?;
            // ExplicitSlot, 8-byte aligned (align_shift 3 = 2^3). Min size
            // 8 so a fieldless struct still has a valid (unread) base.
            let slot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                bytes.max(8),
                3,
            ));
            for (i, v) in field_vals.iter().enumerate() {
                let offset = i32::try_from(i * 8).map_err(|_| JitError::UnsupportedOpcode {
                    opcode: "struct field offset overflow".to_string(),
                })?;
                builder.ins().stack_store(*v, slot, offset);
            }
            let base = builder.ins().stack_addr(I64, slot, 0);
            let len_val = builder.ins().iconst(I64, i64::try_from(len).unwrap_or(0));
            let r = emit_agg_shim(builder, module, "__triet_struct_new", &[base, len_val])?;
            record_boxed_result(value_map, fn_state, *dest, r);
            emit_shim_sentinel_check(builder, module, fn_state)?;
        }
        Instruction::FieldGet {
            dest,
            object,
            field_idx,
        } => {
            let obj = materialize_boxed_operand(builder, module, value_map, ctx, *object)?;
            let idx = builder.ins().iconst(I64, i64::from(*field_idx));
            let r = emit_agg_shim(builder, module, "__triet_field_get", &[obj, idx])?;
            record_boxed_result(value_map, fn_state, *dest, r);
            emit_shim_sentinel_check(builder, module, fn_state)?;
        }
        Instruction::FieldSet {
            dest,
            object,
            field_idx,
            value,
        } => {
            let obj = materialize_boxed_operand(builder, module, value_map, ctx, *object)?;
            let idx = builder.ins().iconst(I64, i64::from(*field_idx));
            let val = materialize_boxed_operand(builder, module, value_map, ctx, *value)?;
            let r = emit_agg_shim(builder, module, "__triet_field_set", &[obj, idx, val])?;
            record_boxed_result(value_map, fn_state, *dest, r);
            emit_shim_sentinel_check(builder, module, fn_state)?;
        }
        Instruction::Ret { value } => {
            // Drop every function-created box EXCEPT the returned one
            // (ownership transfers to the caller). Params are borrowed
            // (caller owns + drops) so they are not in `created_boxed`.
            let returned_id = match value {
                Some(Operand::Value(id)) => Some(*id),
                _ => None,
            };
            for boxed in &fn_state.created_boxed {
                if Some(*boxed) == returned_id {
                    continue;
                }
                if let Some(&ptr) = value_map.get(boxed) {
                    emit_drop_arc(builder, module, ptr)?;
                }
            }
            if let Some(op) = value {
                let v = materialize_boxed_operand(builder, module, value_map, ctx, *op)?;
                builder.ins().return_(&[v]);
            } else {
                // Unit return in boxed mode → a null (0) ptr; the
                // dispatcher unboxes it to `Null`/no-value.
                let z = builder.ins().iconst(I64, 0);
                builder.ins().return_(&[z]);
            }
        }
        // Binary scalar ops (arithmetic / comparison / Ł3 / K3): one
        // `__triet_binop(op, a, b)` shim delegating to the VM. The op
        // discriminant is an `i8` immediate.
        Instruction::Neg { dest, operand } => {
            let v = materialize_boxed_operand(builder, module, value_map, ctx, *operand)?;
            let r = emit_agg_shim(builder, module, "__triet_neg", &[v])?;
            record_boxed_result(value_map, fn_state, *dest, r);
            emit_shim_sentinel_check(builder, module, fn_state)?;
        }
        _ if boxed_binop_of(instr).is_some() => {
            let (binop, dest, lhs, rhs) = boxed_binop_of(instr).expect("guard guarantees Some");
            let l = materialize_boxed_operand(builder, module, value_map, ctx, lhs)?;
            let r = materialize_boxed_operand(builder, module, value_map, ctx, rhs)?;
            let op_imm = builder.ins().iconst(I8, i64::from(binop as u8));
            let res = emit_agg_shim(builder, module, "__triet_binop", &[op_imm, l, r])?;
            record_boxed_result(value_map, fn_state, dest, res);
            emit_shim_sentinel_check(builder, module, fn_state)?;
        }
        other => {
            return Err(JitError::UnsupportedOpcode {
                opcode: format!("{other} (boxed mode — defer to a later agg sub-task)"),
            });
        }
    }
    Ok(())
}

/// Emit a call to a registered aggregate-op shim (looked up by symbol),
/// returning its result `Value`. The caller records the result + emits
/// the per-call failure sentinel — the same machinery `CallBuiltin`
/// uses, reused for the boxed aggregate opcodes.
fn emit_agg_shim(
    builder: &mut FunctionBuilder<'_>,
    module: &mut impl Module,
    symbol: &str,
    args: &[Value],
) -> Result<Option<Value>, JitError> {
    let shim = crate::shims::shim_entry_by_symbol(symbol).ok_or_else(|| JitError::Cranelift {
        message: format!("aggregate shim {symbol} not registered"),
    })?;
    emit_shim_call(builder, module, &shim, args)
}

/// Record a boxed shim's `Ptr` result as `dest` + track it as a
/// function-created box (dropped at `Ret` unless it is the returned
/// value), per ADR-0032 §2.
fn record_boxed_result(
    value_map: &mut HashMap<ValueId, Value>,
    fn_state: &mut FnState,
    dest: ValueId,
    result: Option<Value>,
) {
    if let Some(v) = result {
        value_map.insert(dest, v);
        fn_state.created_boxed.push(dest);
    }
}

fn cranelift_err<E: core::fmt::Display>(err: E) -> JitError {
    JitError::Cranelift {
        message: format!("{err}"),
    }
}

/// v0.10.x.jit.1 — Map an [`AbiScalar`] to its Cranelift IR type per
/// ADR-0032 §1. `Ptr` is `i64`-wide on the v0.10 target triples.
const fn abi_scalar_to_clif(scalar: crate::shims::AbiScalar) -> types::Type {
    match scalar {
        crate::shims::AbiScalar::I8 => I8,
        crate::shims::AbiScalar::I16 => I16,
        // Integer + composite pointers are both i64-wide.
        crate::shims::AbiScalar::I64 | crate::shims::AbiScalar::Ptr => I64,
    }
}

/// v0.10.x.jit.1 — Build a Cranelift [`Signature`] from a shim's ABI
/// description per ADR-0032 §1/§6.
fn shim_signature_to_clif(sig: &crate::shims::ShimSignature) -> Signature {
    let mut clif = Signature::new(CallConv::SystemV);
    for param in sig.params {
        clif.params.push(AbiParam::new(abi_scalar_to_clif(*param)));
    }
    if let Some(ret) = sig.ret {
        clif.returns.push(AbiParam::new(abi_scalar_to_clif(ret)));
    }
    clif
}

/// v0.10.x.jit.1 — Register each shim entry's symbol → address with the
/// `JITBuilder` per ADR-0032 §6 (`__triet_*` prefix). Must run BEFORE
/// `JITModule::new` consumes the builder.
fn register_shim_symbols(builder: &mut JITBuilder, entries: &[crate::shims::ShimEntry]) {
    for entry in entries {
        // `entry.addr` is the address of a `#[unsafe(no_mangle)]
        // extern "C-unwind"` shim function (from `crate::shims` or a
        // `#[cfg(test)]` framework shim). The `usize → *const u8` cast
        // is a SAFE pointer cast; `JITBuilder::symbol` only records the
        // address for later relocation — it never dereferences or calls
        // it here. Backed by ADR-0032 §6.
        let addr = entry.addr as *const u8;
        builder.symbol(entry.symbol, addr);
    }
}

/// Translate a single Triết IR instruction into the Cranelift
/// `FunctionBuilder`'s current block. Updates `value_map` for any new
/// SSA def; reads `block_map` for branch targets; consults `ctx` for
/// inline constants + call targets.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn translate_instruction(
    builder: &mut FunctionBuilder<'_>,
    module: &mut impl Module,
    value_map: &mut HashMap<ValueId, Value>,
    block_map: &HashMap<BlockId, Block>,
    ctx: &ProgramContext<'_>,
    func: &IrFunction,
    fn_state: &mut FnState,
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
            // v0.10.x.jit.2b-i — drop every function-created boxed
            // composite EXCEPT the returned one (ownership of the
            // returned box transfers to the caller per ADR-0032 §2).
            // Composite params are NOT dropped (borrowed; caller owns).
            // Dropping at Ret (rather than precise last-use) is a
            // conservative-correct subset: the Rc lives slightly longer
            // but is balanced exactly once — no leak (success path), no
            // double-free, no use-after.
            let returned_id = match value {
                Some(Operand::Value(id)) => Some(*id),
                _ => None,
            };
            for boxed in &fn_state.created_boxed {
                if Some(*boxed) == returned_id {
                    continue;
                }
                if let Some(&ptr_val) = value_map.get(boxed) {
                    emit_drop_arc(builder, module, ptr_val)?;
                }
            }
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
        // v0.10.x.jit.2a — production builtin-shim dispatch per ADR-0032
        // (§4 option-2 + §1 hybrid ABI). For builtins with an
        // implemented shim: marshal args, call the registered shim
        // symbol, record composite returns for drop_arc. Builtins
        // without a shim (38 pending jit.2b) tier-down. Scope:
        // single-shim-call functions (boundary TLS check suffices); a
        // 2nd shim call tier-downs (per-call sentinel codegen → jit.2b).
        Instruction::CallBuiltin { dest, name, args } => {
            // §3 capability defense-in-depth (empty denied-set = no-op).
            crate::check_builtin_capability(*name, ctx.denied_namespaces)?;
            let Some(shim) = crate::shims::builtin_shim(*name) else {
                return Err(JitError::UnsupportedOpcode {
                    opcode: format!("CallBuiltin({name}) — no shim implemented (defers jit.2b)"),
                });
            };
            // jit.2b-i scope: shim calls only in single-Triết-block
            // functions. Multi-block (if/match/loop) with shims tier-
            // downs — the continue-block chain assumes linear within-
            // block flow; cross-block composite lifetime + multi-Ret
            // drop analysis defers to a later refinement.
            if func.blocks.len() > 1 {
                return Err(JitError::UnsupportedOpcode {
                    opcode: format!(
                        "CallBuiltin({name}) in multi-block function — \
                         single-block shim scope (jit.2b-i)"
                    ),
                });
            }
            if args.len() != shim.signature.params.len() {
                return Err(JitError::UnsupportedOpcode {
                    opcode: format!(
                        "CallBuiltin({name}) arity {} != shim signature {} args",
                        args.len(),
                        shim.signature.params.len()
                    ),
                });
            }
            let arg_values: Vec<Value> = args
                .iter()
                .map(|op| resolve_operand(builder, value_map, ctx, *op))
                .collect::<Result<_, _>>()?;
            let result = emit_shim_call(builder, module, &shim, &arg_values)?;
            if let Some(dest_id) = dest
                && let Some(result_val) = result
            {
                value_map.insert(*dest_id, result_val);
                // Composite (Ptr) returns are freshly-boxed values the
                // function owns → track for drop_arc-at-Ret.
                if shim.signature.ret == Some(crate::shims::AbiScalar::Ptr) {
                    fn_state.created_boxed.push(*dest_id);
                }
            }
            // v0.10.x.jit.2b-i — per-call sentinel (ADR-0032 §4
            // option-2): probe `__triet_shim_failed`; on failure branch
            // to the shared `error_exit` so subsequent shims (e.g.
            // side-effecting `println`) do NOT run after one fails.
            emit_shim_sentinel_check(builder, module, fn_state)?;
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
    module: &mut impl Module,
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

/// v0.10.x.jit.2a — Emit a call to a registered builtin shim (declared
/// `Import`) with already-resolved argument values, per ADR-0032 §6.
/// Returns the call's result `Value` when the shim has a return slot.
fn emit_shim_call(
    builder: &mut FunctionBuilder<'_>,
    module: &mut impl Module,
    shim: &crate::shims::ShimEntry,
    arg_values: &[Value],
) -> Result<Option<Value>, JitError> {
    let clif_sig = shim_signature_to_clif(&shim.signature);
    let shim_id = module
        .declare_function(shim.symbol, Linkage::Import, &clif_sig)
        .map_err(cranelift_err)?;
    let func_ref = module.declare_func_in_func(shim_id, builder.func);
    let call_inst = builder.ins().call(func_ref, arg_values);
    Ok(if shim.signature.ret.is_some() {
        builder.inst_results(call_inst).first().copied()
    } else {
        None
    })
}

/// v0.10.x.jit.2b-i — emit a `__triet_drop_arc(ptr)` call to balance a
/// composite box-out (ADR-0032 §2). Called from the `Ret` arm for each
/// function-created composite that is NOT the returned value.
fn emit_drop_arc(
    builder: &mut FunctionBuilder<'_>,
    module: &mut impl Module,
    ptr_value: Value,
) -> Result<(), JitError> {
    let mut sig = Signature::new(CallConv::SystemV);
    sig.params.push(AbiParam::new(I64));
    let drop_id = module
        .declare_function("__triet_drop_arc", Linkage::Import, &sig)
        .map_err(cranelift_err)?;
    let func_ref = module.declare_func_in_func(drop_id, builder.func);
    builder.ins().call(func_ref, &[ptr_value]);
    Ok(())
}

/// v0.10.x.jit.2b-i — emit the per-call failure sentinel (ADR-0032 §4
/// option-2). After a shim call: probe `__triet_shim_failed`; if set,
/// branch to the function's shared `error_exit` block (created lazily
/// here, body emitted after the instruction loop); otherwise fall
/// through into a fresh `continue` block where translation resumes.
/// This guarantees abort-on-first-shim-failure — subsequent
/// side-effecting shims do not run after one fails.
fn emit_shim_sentinel_check(
    builder: &mut FunctionBuilder<'_>,
    module: &mut impl Module,
    fn_state: &mut FnState,
) -> Result<(), JitError> {
    // Lazily create the shared error_exit block (body emitted post-loop
    // in `emit_function_body`).
    let error_exit = if let Some(b) = fn_state.error_exit {
        b
    } else {
        let b = builder.create_block();
        fn_state.error_exit = Some(b);
        b
    };
    // Probe: call `__triet_shim_failed() -> i8`.
    let mut probe_sig = Signature::new(CallConv::SystemV);
    probe_sig.returns.push(AbiParam::new(I8));
    let probe_id = module
        .declare_function("__triet_shim_failed", Linkage::Import, &probe_sig)
        .map_err(cranelift_err)?;
    let probe_ref = module.declare_func_in_func(probe_id, builder.func);
    let probe_call = builder.ins().call(probe_ref, &[]);
    let failed = builder.inst_results(probe_call)[0];
    // Branch: failed (non-zero) → error_exit; else → continue.
    let cont = builder.create_block();
    builder.ins().brif(failed, error_exit, &[], cont, &[]);
    builder.switch_to_block(cont);
    Ok(())
}
