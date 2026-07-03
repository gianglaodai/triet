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
use cranelift_codegen::ir::{
    AbiParam, BlockArg, InstBuilder, Signature, StackSlotData, StackSlotKind,
};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use std::collections::{HashMap, HashSet};
use triet_mir::{
    BasicBlock, BinOp, Body, CallTarget, ConstValue, ControlFlowGraph, EnumLayout, Local, MirType,
    Place, Projection, Statement, StructLayout, Terminator, builtin_shim_meta,
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

    /// Register a 5-arg → 0-return shim (C6: concat sret).
    pub fn fn_5_0(name: &str, f: extern "C" fn(i64, i64, i64, i64, i64)) -> Self {
        Self {
            name: name.into(),
            addr: f as usize,
            arity: 5,
            has_return: false,
        }
    }
}

// ── JIT context ─────────────────────────────────────────────

/// ADR-0065 §12.7: which Construction-Taxonomy copy an `Assign` between a
/// `Struct?` and a `Struct` performs, dispatched on `(src_ty, dest_ty)`.
#[derive(Clone, Copy)]
enum NullableStructCopy {
    /// `Nullable(Struct) ← Nullable(Struct)`: copy N+8 bytes, tag@0 first
    /// (tag propagates verbatim). Subsumes Delta 4b.
    WholeCopy,
    /// `Nullable(Struct) ← Struct`: set tag=present(1)@dest+0, copy N fields
    /// src+0 → dest+8. Subsumes Delta 4a + field-position construction.
    Widen,
    /// `Struct ← Nullable(Struct)`: copy N fields src+8 → dest+0 (drop tag).
    /// This IS match-bind `pt = scrut`; the old base-downcast made implicit.
    Downcast,
}

/// ADR-0067 2a/2b+: classifies a heap leaf found by `collect_heap_leaves`.
/// A `Heap` leaf is freed/tombstoned UNCONDITIONALLY at a compile-time offset
/// (the static value model). An `Enum` leaf carries its enum name and is freed
/// via a runtime tag-switch (`emit_enum_drop_glue_at`) — only the ACTIVE
/// variant's payload — and tombstoned by zeroing the payload word (NOT the
/// discriminant). The two cannot share a flat unconditional list, hence the split.
#[derive(Debug)]
enum LeafKind {
    Heap(MirType),
    Enum(String),
}

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
    /// Map from MIR Local to Cranelift `StackSlot` for Outcome values.
    /// 16-byte slots: disc@0, payload@8.
    outcome_slots: HashMap<Local, cranelift_codegen::ir::StackSlot>,
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
    #[allow(clippy::unused_self)] // method form kept for call-site readability (30+ sites)
    fn var(&self, l: Local) -> Variable {
        Variable::from_u32(usize_to_u32(l.0))
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

    /// ADR-0065 §12: if `ty` is a nested nullable aggregate, return its
    /// unwrapped inner type and the tag-shift to apply mid-walk — `Struct?`
    /// prepends an 8-byte tag word (Phương án A), `Enum?` uses the 0-byte
    /// disc-niche. `None` for any non-nullable-aggregate type.
    fn nested_nullable_shift(ty: &MirType) -> Option<(MirType, i32)> {
        if let MirType::Nullable(inner) = ty {
            match inner.as_ref() {
                MirType::Struct(_) => return Some(((**inner).clone(), 8)),
                MirType::Enum(_) => return Some(((**inner).clone(), 0)),
                _ => {}
            }
        }
        None
    }

    /// ADR-0060 P2: walk through a Place's projections, accumulating byte
    /// offset from the base local and returning the final field type.
    /// Handles `Field(name)` and `Payload(variant)` projections; errors
    /// on unsupported projection types (`Deref`, `Index`).
    // A single flat dispatch over projection kinds that threads the
    // `total_offset`/`current_ty` accumulators; ADR-0065 §12 added the
    // nested-nullable unwrap step, nudging it one line past the lint. Splitting
    // it would scatter the accumulators across helpers and hurt readability more
    // than the extra line costs.
    #[allow(clippy::too_many_lines)]
    fn walk_projections(body: &Body, place: &Place) -> Result<(MirType, i32), JitError> {
        // ADR-0065 Lát 1: `Enum?` shares the enum's slot layout (disc-sentinel
        // niche). Unwrap Nullable so payload/field projections resolve against
        // the inner Enum (the construction of a widened `Enum?` local still
        // emits `_.Payload(V)`). Mirror of Lát 4.8's unwrap-at-site idiom.
        let base_ty = &body.local_decls[place.local.0].ty;
        let mut current_ty = base_ty.nullable_payload().unwrap_or(base_ty).clone();
        // ADR-0065 §12.7: faithful walk — base offset is 0. The `Struct?` tag
        // shift is NO LONGER baked here (the old base-downcast nuốt the tag in
        // whole-slot Assign-copy, Bug A). The Construction Taxonomy at the
        // Assign chokepoint applies the +8 downcast explicitly (case 3).
        let mut total_offset = 0i32;
        for proj in &place.projection {
            // ADR-0065 §12: unwrap a nested nullable-aggregate field, +tag-shift.
            if let Some((unwrapped, shift)) = Self::nested_nullable_shift(&current_ty) {
                current_ty = unwrapped;
                total_offset += shift;
            }
            match proj {
                Projection::Field(field_name) => {
                    let ty_name = match &current_ty {
                        MirType::Struct(name) | MirType::Enum(name) => name.as_str(),
                        MirType::String => "String",
                        other => {
                            return Err(JitError::Unsupported(format!(
                                "field access on non-aggregate type '{other}'"
                            )));
                        }
                    };
                    let layout = body
                        .struct_layouts
                        .iter()
                        .find(|l| l.name == ty_name)
                        .ok_or_else(|| {
                            JitError::Unsupported(format!(
                                "type '{current_ty}' is not a known struct (local {})",
                                place.local
                            ))
                        })?;
                    let field = layout
                        .fields
                        .iter()
                        .find(|f| f.name == *field_name)
                        .ok_or_else(|| {
                            JitError::Unsupported(format!(
                                "field '{field_name}' not found in struct '{current_ty}'"
                            ))
                        })?;
                    let field_off = i32::try_from(field.offset)
                        .map_err(|_| JitError::Unsupported("field offset exceeds i32".into()))?;
                    total_offset += field_off;
                    current_ty = field.ty.clone();
                }
                Projection::Payload(variant_name) => {
                    total_offset += 8; // Payload always at offset 8 in Bậc A
                    let ty_name = match &current_ty {
                        MirType::Enum(name) => name.as_str(),
                        other => {
                            return Err(JitError::Unsupported(format!(
                                "payload access on non-enum type '{other}'"
                            )));
                        }
                    };
                    let enum_layout = body
                        .enum_layouts
                        .iter()
                        .find(|e| e.name == ty_name)
                        .ok_or_else(|| {
                            JitError::Unsupported(format!("enum '{current_ty}' layout not found"))
                        })?;
                    let variant = enum_layout
                        .variants
                        .iter()
                        .find(|v| &v.name == variant_name)
                        .ok_or_else(|| {
                            JitError::Unsupported(format!(
                                "variant '{variant_name}' not found in enum '{current_ty}'"
                            ))
                        })?;
                    if let Some(ref payload) = variant.payload {
                        current_ty = payload.ty.clone();
                    } else {
                        return Err(JitError::Unsupported(format!(
                            "variant '{variant_name}' has no payload"
                        )));
                    }
                }
                Projection::OutcomeDiscriminant => {
                    // discriminant is always at offset 0 in the Outcome slot.
                    total_offset += 0;
                    current_ty = MirType::Integer; // disc is i64
                }
                Projection::OutcomePayload => {
                    // payload is always at offset 8 in the Outcome slot.
                    total_offset += 8;
                    current_ty = MirType::Integer; // scalar payload is i64
                }
                Projection::OutcomePayloadLen => {
                    // len field at offset 16 (for heap Outcome: {disc@0, ptr@8, len@16, cap@24}).
                    total_offset += 16;
                    current_ty = MirType::Integer;
                }
                Projection::OutcomePayloadCap => {
                    // cap field at offset 24.
                    total_offset += 24;
                    current_ty = MirType::Integer;
                }
                other => {
                    return Err(JitError::Unsupported(format!(
                        "unsupported projection in nested position: {other:?}"
                    )));
                }
            }
        }
        Ok((current_ty, total_offset))
    }

    /// ADR-0067 2a: statically collect every heap LEAF reachable from a struct,
    /// as a flat `(absolute_offset, leaf_kind)` list. Recurses into nested
    /// struct fields accumulating offsets (the value model stores nested structs
    /// inline/FLAT), so a single pass over the returned list frees/tombstones
    /// every heap byte regardless of nesting depth. The struct graph is a DAG
    /// (recursive types are blocked by typecheck), so this terminates; the depth
    /// limit is a last-resort net that returns a `JitError` instead of hanging
    /// the compiler if a recursive type ever slips through. Enum fields are NOT
    /// recursed — enum-payload heap is tag-dependent (runtime disc) → ADR-0067 2b+
    /// pushes them as `LeafKind::Enum` for a runtime tag-switch at drop/deinit.
    fn collect_heap_leaves(
        struct_name: &str,
        base_offset: i32,
        body: &Body,
        depth: usize,
        out: &mut Vec<(i32, LeafKind)>,
    ) -> Result<(), JitError> {
        if depth > 64 {
            return Err(JitError::Unsupported(format!(
                "struct nesting exceeds depth 64 (recursive type? → ADR-0068): {struct_name}"
            )));
        }
        let layout = body
            .struct_layouts
            .iter()
            .find(|l| l.name == struct_name)
            .ok_or_else(|| {
                JitError::Unsupported(format!("unknown struct layout: {struct_name}"))
            })?;
        for f in &layout.fields {
            let abs = base_offset
                + i32::try_from(f.offset)
                    .map_err(|_| JitError::Unsupported("struct field offset exceeds i32".into()))?;
            match &f.ty {
                t if t.is_any_heap() => out.push((abs, LeafKind::Heap(t.clone()))),
                MirType::Struct(inner) => {
                    Self::collect_heap_leaves(inner, abs, body, depth + 1, out)?;
                }
                // ADR-0067 2b+: an enum field is a leaf with a runtime tag-switch
                // free (no static recursion into its payload — the active variant
                // is only known at runtime). A Copy enum yields no heap arm at
                // drop time → harmless no-op (and a struct holding only Copy enums
                // is itself Copy → never dropped).
                MirType::Enum(name) => out.push((abs, LeafKind::Enum(name.clone()))),
                // ADR-0076: a heap-`T?` leaf field (`String?`/`Vector?`/
                // `HashMap?`) is a sentinel-bearing slot at the field-offset —
                // the SAME repr as the plain heap field (ptr@abs ∈ {ptr, 0,
                // NULL_SENTINEL}). Push it as a `Heap` leaf of the INNER type so
                // Drop/tombstone hit ptr@abs UNCONDITIONALLY; the free shim
                // no-ops on NULL_SENTINEL (null) and 0 (moved-out), so no `brif`
                // is needed (§Conditional-drop = sentinel-no-op, R4).
                MirType::Nullable(inner) if inner.is_any_heap() => {
                    out.push((abs, LeafKind::Heap((**inner).clone())));
                }
                _ => {} // scalar / Nullable(scalar/aggregate) → skip
            }
        }
        Ok(())
    }

    /// ADR-0060 P2: return the total byte size of a `MirType`.
    /// Struct/enum types look up their layout in `body`; scalars are 8.
    fn ty_total_size(body: &Body, ty: &MirType) -> usize {
        match ty {
            MirType::Struct(name) => body
                .struct_layouts
                .iter()
                .find(|l| l.name == *name)
                .map_or(8, |l| l.total_size),
            MirType::Enum(name) => body
                .enum_layouts
                .iter()
                .find(|l| l.name == *name)
                .map_or(16, |l| l.total_size),
            _ => 8,
        }
    }

    /// ADR-0077 Typed Vector P1: per-element STRIDE for a `Vector<T>` element
    /// type. A compile-time constant for every built-in element — the crux that
    /// keeps Vector P1 ⊥ native-layout. Scalar / handle / `Nullable(scalar)` =
    /// 8; `String` / `Nullable(String)` = 24; `Vector` / `HashMap` handle = 8.
    /// `Struct` / `Enum` (and any other non-built-in) → REFUSE (`JitError`) —
    /// the P1/P2 boundary (`Vector<UserStruct>` needs native-layout, deferred).
    ///
    /// ⚠️ NOT `ty_total_size` (which returns 8 for `String` — wrong stride 24).
    fn vector_elem_size(ty: &MirType) -> Result<i64, JitError> {
        // Nullable rides its payload's repr (sentinel in the same slot width).
        let eff = ty.nullable_payload().unwrap_or(ty);
        match eff {
            MirType::String => Ok(24),
            MirType::Integer
            | MirType::Trit
            | MirType::Tryte
            | MirType::Long
            | MirType::Trilean
            | MirType::Unit
            | MirType::Unknown
            | MirType::Vector(_)
            | MirType::HashMap(..)
            | MirType::Reference { .. } => Ok(8),
            MirType::Struct(_)
            | MirType::Enum(_)
            | MirType::Capability(_)
            | MirType::Outcome { .. } => Err(JitError::Unsupported(format!(
                "Vector<{eff}>: element type is not a built-in known-size type \
                     (ADR-0077 P1 refuses Struct/Enum/Capability/Outcome elements — \
                     by-value aggregate elements need native-layout, deferred to P2)"
            ))),
            MirType::Nullable(_) => unreachable!("nullable_payload already stripped one layer"),
        }
    }

    /// ADR-0060 P2-Boundary: base address of a local + byte offset. Slot-backed
    /// locals (struct/enum) → `stack_addr`; sret/param/match-binding → the
    /// pointer value in the variable.
    fn copy_base_addr(
        &self,
        builder: &mut FunctionBuilder<'_>,
        local: Local,
        offset: i32,
    ) -> cranelift_codegen::ir::Value {
        let base = if let Some((slot, _)) = self.struct_slots.get(&local) {
            builder.ins().stack_addr(I64, *slot, 0)
        } else if let Some((slot, _)) = self.enum_slots.get(&local) {
            builder.ins().stack_addr(I64, *slot, 0)
        } else {
            builder.use_var(self.var(local))
        };
        if offset != 0 {
            builder.ins().iadd_imm(base, i64::from(offset))
        } else {
            base
        }
    }

    /// ADR-0065 §12.7: resolve a Place's logical type (KEEPING the `Nullable`
    /// wrapper for a leaf nullable-aggregate field) + its faithful byte offset.
    /// The Construction Taxonomy dispatches on these types, so the wrapper must
    /// survive (unlike `walk_projections`, which unwraps the base). Empty
    /// projection → the local's declared type at offset 0.
    fn resolve_place_for_copy(body: &Body, place: &Place) -> Result<(MirType, i32), JitError> {
        if place.projection.is_empty() {
            Ok((body.local_decls[place.local.0].ty.clone(), 0))
        } else {
            Self::walk_projections(body, place)
        }
    }

    /// ADR-0065 §12.7: classify an `Assign` between a `Struct?` and a `Struct`
    /// into a Construction-Taxonomy case. Returns `(case, src_off, dest_off,
    /// inner_n)` where `inner_n` is the byte count the case copies (N+8 for
    /// `WholeCopy`, N for `Widen`/`Downcast`). `None` when neither side is a nullable
    /// struct (the caller falls through to the general aggregate/scalar copy).
    /// `Nullable(Enum)` is intentionally NOT matched here — its disc-niche has
    /// 0-byte tag, so the general copy (no shift) already moves it correctly.
    fn nullable_struct_taxonomy(
        body: &Body,
        dest: &Place,
        source: &Place,
    ) -> Result<Option<(NullableStructCopy, i32, i32, usize)>, JitError> {
        // Inner Struct type of a `Nullable(Struct)`, else None.
        fn nstruct_inner(ty: &MirType) -> Option<MirType> {
            if let MirType::Nullable(inner) = ty
                && matches!(**inner, MirType::Struct(_))
            {
                return Some((**inner).clone());
            }
            None
        }
        let is_plain_struct = |ty: &MirType| matches!(ty, MirType::Struct(n) if n != "String");

        let (src_ty, src_off) = Self::resolve_place_for_copy(body, source)?;
        let (dest_ty, dest_off) = Self::resolve_place_for_copy(body, dest)?;
        let src_inner = nstruct_inner(&src_ty);
        let dest_inner = nstruct_inner(&dest_ty);

        let case = match (&src_inner, &dest_inner) {
            // case 1: Nullable(Struct) ← Nullable(Struct) → whole-copy N+8.
            (Some(_), Some(di)) => (
                NullableStructCopy::WholeCopy,
                Self::ty_total_size(body, di) + 8,
            ),
            // case 2: Struct ← (dest Nullable(Struct)) → widen, N fields.
            (None, Some(di)) if is_plain_struct(&src_ty) => {
                (NullableStructCopy::Widen, Self::ty_total_size(body, di))
            }
            // case 3: Nullable(Struct) → plain Struct → downcast, N fields.
            (Some(si), None) if is_plain_struct(&dest_ty) => {
                (NullableStructCopy::Downcast, Self::ty_total_size(body, si))
            }
            _ => return Ok(None),
        };
        Ok(Some((case.0, src_off, dest_off, case.1)))
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
            // ADR-0060 P2: whole-read for struct/enum locals reads from
            // the stack slot (use_var may be unset — struct fields are
            // built via field-level stack_store, not def_var).
            if let Some((slot, _)) = self.struct_slots.get(&place.local) {
                return Ok(builder.ins().stack_load(I64, *slot, 0));
            }
            if let Some((slot, _)) = self.enum_slots.get(&place.local) {
                return Ok(builder.ins().stack_load(I64, *slot, 0));
            }
            return Ok(builder.use_var(self.var(place.local)));
        }
        // Outcome* projections are always single-level (never nested inside
        // struct/enum). Handle them before entering the nested walk.
        if place.projection.len() == 1 {
            match &place.projection[0] {
                Projection::OutcomeDiscriminant => {
                    let disc_offset: i32 = 0;
                    if let Some(slot) = self.outcome_slots.get(&place.local) {
                        return Ok(builder.ins().stack_load(I64, *slot, disc_offset));
                    }
                    return Err(JitError::Unsupported(
                        "OutcomeDiscriminant access on non-Outcome local".into(),
                    ));
                }
                Projection::OutcomePayload => {
                    let payload_offset: i32 = 8;
                    if let Some(slot) = self.outcome_slots.get(&place.local) {
                        return Ok(builder.ins().stack_load(I64, *slot, payload_offset));
                    }
                    return Err(JitError::Unsupported(
                        "OutcomePayload access on non-Outcome local".into(),
                    ));
                }
                Projection::OutcomePayloadLen => {
                    if let Some(slot) = self.outcome_slots.get(&place.local) {
                        return Ok(builder.ins().stack_load(I64, *slot, 16));
                    }
                    return Err(JitError::Unsupported(
                        "OutcomePayloadLen access on non-Outcome local".into(),
                    ));
                }
                Projection::OutcomePayloadCap => {
                    if let Some(slot) = self.outcome_slots.get(&place.local) {
                        return Ok(builder.ins().stack_load(I64, *slot, 24));
                    }
                    return Err(JitError::Unsupported(
                        "OutcomePayloadCap access on non-Outcome local".into(),
                    ));
                }
                _ => {} // Field/Payload: fall through to walk
            }
        }

        // ADR-0060 P2: nested projection walk for Field + Payload.
        // Accumulates offset through each level, then loads from the
        // base slot (struct_slots or enum_slots) at the final offset.
        let (_final_ty, total_offset) = Self::walk_projections(body, place)?;
        if let Some((slot, _)) = self.struct_slots.get(&place.local) {
            return Ok(builder.ins().stack_load(I64, *slot, total_offset));
        }
        if let Some((slot, _)) = self.enum_slots.get(&place.local) {
            return Ok(builder.ins().stack_load(I64, *slot, total_offset));
        }
        // Pointer-based: param or sret. Load pointer, add offset, load.
        let ptr = builder.use_var(self.var(place.local));
        let addr = builder.ins().iadd_imm(ptr, i64::from(total_offset));
        Ok(builder
            .ins()
            .load(I64, cranelift_codegen::ir::MemFlags::new(), addr, 0))
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
            // ADR-0060 P2: for struct/enum locals, also store to the stack slot
            // so the slot stays in sync with the variable.
            if let Some((slot, _)) = self.struct_slots.get(&place.local) {
                builder.ins().stack_store(value, *slot, 0);
            }
            if let Some((slot, _)) = self.enum_slots.get(&place.local) {
                builder.ins().stack_store(value, *slot, 0);
            }
            builder.def_var(self.var(place.local), value);
            return Ok(());
        }
        // Outcome* projections are always single-level (never nested inside
        // struct/enum). Handle them before entering the nested walk.
        if place.projection.len() == 1 {
            match &place.projection[0] {
                Projection::OutcomeDiscriminant => {
                    let disc_offset: i32 = 0;
                    if let Some(slot) = self.outcome_slots.get(&place.local) {
                        builder.ins().stack_store(value, *slot, disc_offset);
                        return Ok(());
                    }
                    return Err(JitError::Unsupported(
                        "OutcomeDiscriminant store to non-Outcome local".into(),
                    ));
                }
                Projection::OutcomePayload => {
                    if let Some(slot) = self.outcome_slots.get(&place.local) {
                        builder.ins().stack_store(value, *slot, 8);
                        return Ok(());
                    }
                    return Err(JitError::Unsupported(
                        "OutcomePayload store to non-Outcome local".into(),
                    ));
                }
                Projection::OutcomePayloadLen => {
                    if let Some(slot) = self.outcome_slots.get(&place.local) {
                        builder.ins().stack_store(value, *slot, 16);
                        return Ok(());
                    }
                    return Err(JitError::Unsupported(
                        "OutcomePayloadLen store to non-Outcome local".into(),
                    ));
                }
                Projection::OutcomePayloadCap => {
                    if let Some(slot) = self.outcome_slots.get(&place.local) {
                        builder.ins().stack_store(value, *slot, 24);
                        return Ok(());
                    }
                    return Err(JitError::Unsupported(
                        "OutcomePayloadCap store to non-Outcome local".into(),
                    ));
                }
                _ => {} // Field/Payload: fall through to walk
            }
        }

        // ADR-0060 P2: nested projection walk for Field + Payload.
        // Accumulates offset through each level, then stores to the
        // base slot at the final offset.
        let (_final_ty, total_offset) = Self::walk_projections(body, place)?;
        if let Some((slot, _)) = self.struct_slots.get(&place.local) {
            builder.ins().stack_store(value, *slot, total_offset);
            return Ok(());
        }
        if let Some((slot, _)) = self.enum_slots.get(&place.local) {
            builder.ins().stack_store(value, *slot, total_offset);
            return Ok(());
        }
        // Pointer-based: load pointer, add offset, store.
        let ptr = builder.use_var(self.var(place.local));
        let addr = builder.ins().iadd_imm(ptr, i64::from(total_offset));
        builder
            .ins()
            .store(cranelift_codegen::ir::MemFlags::new(), value, addr, 0);
        Ok(())
    }

    /// Create a new JIT context with host ISA detection (no shims).
    pub fn new() -> Self {
        Self::with_shims(&[])
    }

    /// Create a new JIT context with registered shim symbols.
    ///
    /// Each shim is registered as an `extern "C"` symbol in the JIT module
    /// so that `CallTarget::Shim` calls resolve at link time.
    ///
    /// # Panics
    ///
    /// Panics if the host ISA is not detected or not supported by Cranelift.
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
            outcome_slots: HashMap::new(),
            blocks: HashMap::new(),
            sealed: HashSet::new(),
            filled: HashSet::new(),
            func_ids: HashMap::new(),
            shim_registry,
        }
    }

    /// Compile a single MIR body (no cross-function calls).
    ///
    /// # Panics
    ///
    /// Panics if `compile_multi` returns an empty map (internal bug).
    ///
    /// # Errors
    ///
    /// Returns [`JitError::Module`] if a function declaration fails, or
    /// [`JitError::Unsupported`] if a MIR construct cannot be lowered.
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
    ///
    /// # Errors
    ///
    /// Returns [`JitError::Module`] if a function cannot be declared or
    /// finalized, or [`JitError::Unsupported`] if a MIR construct cannot
    /// be lowered to Cranelift IR.
    pub fn compile_multi(
        &mut self,
        bodies: &[&Body],
    ) -> Result<HashMap<String, CompiledFunction>, JitError> {
        // ── Phase 1: declare all functions ─────────────────
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
            for _ in &body.signature.parameters {
                sig.params.push(AbiParam::new(I64));
            }
            if !is_sret {
                match body.signature.return_shape {
                    triet_mir::ReturnShape::BinaryOutcome
                    | triet_mir::ReturnShape::TernaryOutcome => {
                        sig.returns.push(AbiParam::new(I64)); // disc
                        sig.returns.push(AbiParam::new(I64)); // payload
                    }
                    _ => {
                        sig.returns.push(AbiParam::new(I64));
                    }
                }
            }

            let func_id = self
                .module
                .declare_function(&body.signature.name, Linkage::Local, &sig)
                .map_err(|e| JitError::Module(format!("declare {}: {e}", body.signature.name)))?;

            self.func_ids.insert(body.signature.name.clone(), func_id);
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
            for _ in &body.signature.parameters {
                cl_ctx.func.signature.params.push(AbiParam::new(I64));
            }
            if !is_sret {
                match body.signature.return_shape {
                    triet_mir::ReturnShape::BinaryOutcome
                    | triet_mir::ReturnShape::TernaryOutcome => {
                        cl_ctx.func.signature.returns.push(AbiParam::new(I64)); // disc
                        cl_ctx.func.signature.returns.push(AbiParam::new(I64)); // payload
                    }
                    _ => {
                        cl_ctx.func.signature.returns.push(AbiParam::new(I64));
                    }
                }
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

    /// HP.2 / ADR-0066 KCN-1+1b: emit inline free for ONE heap value whose fat
    /// pointer lives at `addr`. Loads `{ptr@addr+0, cap@addr+16}` and calls the
    /// type-appropriate free shim. String → 2-arg free(ptr,cap) (cap is the
    /// dealloc-layout source — ADR-0049 Lát 3); Vector/HashMap → 1-arg free(ptr).
    /// `addr` is a computed memory address (`stack_addr`/`copy_base_addr`), so a
    /// single helper serves BOTH slot-backed locals AND by-pointer params (1b
    /// arg-move callee drop-glue). Callers: Outcome drop-glue passes
    /// `stack_addr(slot, 8)` (slot = {disc@0, payload@8}); struct drop-glue
    /// passes `copy_base_addr(local, field.offset)`.
    fn emit_heap_free_at(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        addr: cranelift_codegen::ir::Value,
        payload_ty: &MirType,
    ) -> Result<(), JitError> {
        if !payload_ty.is_any_heap() {
            return Ok(());
        }
        let mem = cranelift_codegen::ir::MemFlags::new();
        let ptr = builder.ins().load(I64, mem, addr, 0);
        // ADR-0077: a Vector leaf frees its heap ELEMENTS first (element-free
        // loop), then its buffer — `emit_vector_free_value` drives both. This is
        // the SAME entry the standalone Drop arm uses, so a `Vector<String>`
        // inside a struct/enum/another vector recurses correctly.
        if let MirType::Vector(inner) = payload_ty {
            let inner = (**inner).clone();
            self.emit_vector_free_value(builder, ptr, &inner)?;
            return Ok(());
        }
        // ADR-0078/0080: a HashMap leaf frees its heap KEYS then VALUES
        // (slot-iteration loops), then the buffer. Scalar key/value
        // (Integer, stride 8) → loop skipped, byte-compat with old
        // __triet_hashmap_free(ptr).
        if let MirType::HashMap(k, v) = payload_ty {
            let key_ty = (**k).clone();
            let value_ty = (**v).clone();
            self.emit_hashmap_free_value(builder, ptr, &key_ty, &value_ty)?;
            return Ok(());
        }
        let free_name = if matches!(payload_ty, MirType::String) {
            "__triet_string_free"
        } else if payload_ty.is_hashmap() {
            // Only reached for a naked HashMap(..) pointer that didn't match
            // the branch above — should not happen in practice.
            "__triet_hashmap_free"
        } else {
            return Ok(());
        };
        let func_id = self.get_or_declare_shim(free_name)?;
        let func_ref = self.module.declare_func_in_func(func_id, builder.func);
        if matches!(payload_ty, MirType::String) {
            let cap = builder.ins().load(I64, mem, addr, 16);
            builder.ins().call(func_ref, &[ptr, cap]);
        } else {
            builder.ins().call(func_ref, &[ptr]);
        }
        Ok(())
    }

    /// ADR-0077 Typed Vector P1 (MŨI 3) — free a Vector given its buffer pointer
    /// VALUE (not an address). Frees the heap ELEMENTS via a runtime loop, then
    /// the buffer. For a scalar/handle element (`elem_kind` 0) the loop is
    /// skipped → byte-identical to the legacy `__triet_vector_free(ptr)`.
    ///
    /// The element-free calls route through `emit_heap_free_at` → the shim
    /// registry, so they are COUNTED / poison-testable (a Rust-internal shim
    /// loop would bypass the counting harnesses — see WO consult). Reused by
    /// both the standalone Vector `Drop` arm and the aggregate-leaf path.
    fn emit_vector_free_value(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        ptr_val: cranelift_codegen::ir::Value,
        inner_ty: &MirType,
    ) -> Result<(), JitError> {
        self.emit_vector_element_free_loop(builder, ptr_val, inner_ty)?;
        let func_id = self.get_or_declare_shim("__triet_vector_free")?;
        let func_ref = self.module.declare_func_in_func(func_id, builder.func);
        builder.ins().call(func_ref, &[ptr_val]);
        Ok(())
    }

    /// ADR-0077 (MŨI 3) — emit a runtime loop that frees each heap element of a
    /// Vector buffer. No-op (returns immediately, no blocks) when the element is
    /// a scalar/handle (Copy) → Vector<Integer>/Vector<Vector<_>> stay
    /// byte-compatible. A null/sentinel/zero buffer pointer skips the loop.
    ///
    /// `len` is read from the buffer header (`body@0`); `stride` from the header
    /// `reserved` u32 (high 32 bits of the 8-byte object header at `body-8`).
    /// Each element at `data + i*stride` is freed via `emit_heap_free_at`
    /// (registry-routed, sentinel-no-op R4) — so nested `Vector<Vector<String>>`
    /// recurses through `emit_heap_free_at`'s Vector branch.
    fn emit_vector_element_free_loop(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        ptr_val: cranelift_codegen::ir::Value,
        inner_ty: &MirType,
    ) -> Result<(), JitError> {
        // Nullable rides the inner repr; only a heap element needs per-element drop.
        let eff = inner_ty.nullable_payload().unwrap_or(inner_ty).clone();
        if !eff.is_any_heap() {
            return Ok(()); // scalar / handle element → no loop (byte-compat)
        }
        let mem = cranelift_codegen::ir::MemFlags::new();
        let zero = builder.ins().iconst(I64, 0);
        let sentinel = builder.ins().iconst(I64, triet_mir::NULL_SENTINEL);
        let is_zero = builder.ins().icmp(IntCC::Equal, ptr_val, zero);
        let is_sent = builder.ins().icmp(IntCC::Equal, ptr_val, sentinel);
        let is_dead = builder.ins().bor(is_zero, is_sent);

        let setup_bb = builder.create_block();
        let header_bb = builder.create_block();
        builder.append_block_param(header_bb, I64); // induction var i
        let body_bb = builder.create_block();
        let exit_bb = builder.create_block();

        // Dead (null/sentinel/0) buffer → skip straight to exit.
        builder.ins().brif(is_dead, exit_bb, &[], setup_bb, &[]);

        // setup: read len + stride, compute data base, enter loop at i=0.
        builder.switch_to_block(setup_bb);
        builder.seal_block(setup_bb);
        let len = builder.ins().load(I64, mem, ptr_val, 0);
        let hdr = builder.ins().load(I64, mem, ptr_val, -8); // {refcount|reserved}
        let stride = builder.ins().ushr_imm(hdr, 32); // reserved = high u32 = stride
        let data = builder.ins().iadd_imm(ptr_val, 16); // skip len@0 + cap@8
        let i0 = builder.ins().iconst(I64, 0);
        builder.ins().jump(header_bb, &[BlockArg::from(i0)]);

        // header: i < len ? body : exit
        builder.switch_to_block(header_bb);
        let i = builder.block_params(header_bb)[0];
        let cond = builder.ins().icmp(IntCC::SignedLessThan, i, len);
        builder.ins().brif(cond, body_bb, &[], exit_bb, &[]);

        // body: free element at data + i*stride, then i+1 → header.
        builder.switch_to_block(body_bb);
        builder.seal_block(body_bb);
        let off = builder.ins().imul(i, stride);
        let elem_addr = builder.ins().iadd(data, off);
        self.emit_heap_free_at(builder, elem_addr, &eff)?;
        let i_next = builder.ins().iadd_imm(i, 1);
        builder.ins().jump(header_bb, &[BlockArg::from(i_next)]);

        // header has both predecessors now (setup + body back-edge) → seal.
        builder.seal_block(header_bb);
        builder.switch_to_block(exit_bb);
        builder.seal_block(exit_bb);
        Ok(())
    }

    /// ADR-0078/0080 Typed `HashMap` P1 (MŨI C / ADR-0080 Mũi D) — free a
    /// `HashMap` given its buffer pointer VALUE. Frees heap KEYS (ADR-0080
    /// D.1, String key only) then heap VALUES (ADR-0078) via slot-iteration
    /// loops, then frees the buffer. `key_ty` GATES whether the key-free
    /// loop is emitted AT ALL (compile-time skip, mirroring how `value_ty`
    /// already gates the value loop) — an `Integer`-keyed map emits ZERO
    /// extra Cranelift blocks and, critically, never DECLARES the
    /// `__triet_string_free` shim import, so `HashMap<Integer,_>` stays
    /// byte-compat with callers whose shim list never registered it (a
    /// compile-time-UNCONDITIONAL declare — regardless of the runtime
    /// `key_stride` check inside the loop — broke exactly these callers;
    /// `get_or_declare_shim` runs at codegen time, not gated by a runtime
    /// `brif`). The loop body ITSELF still re-checks `key_stride == 24` at
    /// runtime (self-describing per ADR-0080 Mũi A) as defense-in-depth for
    /// the String-keyed case. Reused by `emit_heap_free_at` and the
    /// standalone `HashMap` `Drop` arm.
    fn emit_hashmap_free_value(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        ptr_val: cranelift_codegen::ir::Value,
        key_ty: &MirType,
        value_ty: &MirType,
    ) -> Result<(), JitError> {
        if key_ty.is_any_heap() {
            self.emit_hashmap_key_free_loop(builder, ptr_val)?;
        }
        self.emit_hashmap_value_free_loop(builder, ptr_val, value_ty)?;
        let func_id = self.get_or_declare_shim("__triet_hashmap_free")?;
        let func_ref = self.module.declare_func_in_func(func_id, builder.func);
        builder.ins().call(func_ref, &[ptr_val]);
        Ok(())
    }

    /// ADR-0080 Mũi D.1 — iterate all `cap` `HashMap` slots, free the KEY of
    /// every OCCUPIED slot. Only called when `key_ty.is_any_heap()`
    /// (compile-time gate in `emit_hashmap_free_value`); the runtime
    /// `key_stride == 24` re-check here is defense-in-depth, not the
    /// primary gate (see that function's doc for why a purely-runtime gate
    /// broke Integer-keyed callers). Sentinel/no-op R4 on a dead buffer
    /// pointer. Mirrors `emit_hashmap_value_free_loop`'s shape.
    fn emit_hashmap_key_free_loop(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        ptr_val: cranelift_codegen::ir::Value,
    ) -> Result<(), JitError> {
        let mem = cranelift_codegen::ir::MemFlags::new();
        let zero = builder.ins().iconst(I64, 0);
        let sentinel = builder.ins().iconst(I64, triet_mir::NULL_SENTINEL);
        let is_zero = builder.ins().icmp(IntCC::Equal, ptr_val, zero);
        let is_sent = builder.ins().icmp(IntCC::Equal, ptr_val, sentinel);
        let is_dead = builder.ins().bor(is_zero, is_sent);

        let setup_bb = builder.create_block();
        let key_check_bb = builder.create_block();
        let header_bb = builder.create_block();
        builder.append_block_param(header_bb, I64);
        let body_bb = builder.create_block();
        let exit_bb = builder.create_block();

        builder.ins().brif(is_dead, exit_bb, &[], setup_bb, &[]);

        builder.switch_to_block(setup_bb);
        builder.seal_block(setup_bb);
        let cap = builder.ins().load(I64, mem, ptr_val, 8);
        let hdr = builder.ins().load(I64, mem, ptr_val, -8);
        let reserved = builder.ins().ushr_imm(hdr, 32);
        let value_stride = builder.ins().band_imm(reserved, 0xFFFF);
        let key_stride = builder.ins().ushr_imm(reserved, 16);
        let c24 = builder.ins().iconst(I64, 24);
        let is_string_key = builder.ins().icmp(IntCC::Equal, key_stride, c24);
        builder
            .ins()
            .brif(is_string_key, key_check_bb, &[], exit_bb, &[]);

        builder.switch_to_block(key_check_bb);
        builder.seal_block(key_check_bb);
        let slot_size = {
            let one = builder.ins().iconst(I64, 1);
            let tmp = builder.ins().iadd(key_stride, value_stride);
            builder.ins().iadd(tmp, one)
        };
        let state_off = {
            let c16 = builder.ins().iconst(I64, 16);
            let tmp = builder.ins().iadd(key_stride, value_stride);
            builder.ins().iadd(tmp, c16)
        };
        let i0 = builder.ins().iconst(I64, 0);
        builder.ins().jump(header_bb, &[BlockArg::from(i0)]);

        builder.switch_to_block(header_bb);
        let i = builder.block_params(header_bb)[0];
        let cond = builder.ins().icmp(IntCC::SignedLessThan, i, cap);
        builder.ins().brif(cond, body_bb, &[], exit_bb, &[]);

        builder.switch_to_block(body_bb);
        builder.seal_block(body_bb);
        let slot_off = builder.ins().imul(i, slot_size);
        let off = builder.ins().iadd(state_off, slot_off);
        let state_addr = builder.ins().iadd(ptr_val, off);
        let state = builder.ins().load(I64, mem, state_addr, 0);
        let state_byte = builder.ins().band_imm(state, 0xFF);
        let occupied = builder.ins().iconst(I64, 1);
        let is_occ = builder.ins().icmp(IntCC::Equal, state_byte, occupied);
        let skip_bb = builder.create_block();
        let free_bb = builder.create_block();
        builder.ins().brif(is_occ, free_bb, &[], skip_bb, &[]);

        // key cell addr = body + 16 + slot_off (key is the FIRST field).
        builder.switch_to_block(free_bb);
        builder.seal_block(free_bb);
        let c16b = builder.ins().iconst(I64, 16);
        let key_off = builder.ins().iadd(c16b, slot_off);
        let key_cell = builder.ins().iadd(ptr_val, key_off);
        self.emit_heap_free_at(builder, key_cell, &MirType::String)?;
        builder.ins().jump(skip_bb, &[]);

        builder.switch_to_block(skip_bb);
        builder.seal_block(skip_bb);
        let i_next = builder.ins().iadd_imm(i, 1);
        builder.ins().jump(header_bb, &[BlockArg::from(i_next)]);

        builder.seal_block(header_bb);
        builder.switch_to_block(exit_bb);
        builder.seal_block(exit_bb);
        Ok(())
    }

    /// ADR-0078 (MŨI C) — iterate all `cap` `HashMap` slots, free the VALUE
    /// of every OCCUPIED slot. No-op when the value is Copy (Integer:
    /// `is_any_heap` → false). Sentinel/no-op R4 on the value ptr (if value is
    /// heap, state==occupied → free, else skip). Scalars/handles skip entirely.
    /// ADR-0080: slot geometry (`slot_size`/cell offsets) now reads
    /// `key_stride` from the header at RUNTIME instead of a hardcoded `8`.
    fn emit_hashmap_value_free_loop(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        ptr_val: cranelift_codegen::ir::Value,
        value_ty: &MirType,
    ) -> Result<(), JitError> {
        let eff = value_ty.nullable_payload().unwrap_or(value_ty).clone();
        if !eff.is_any_heap() {
            return Ok(());
        }
        let mem = cranelift_codegen::ir::MemFlags::new();
        let zero = builder.ins().iconst(I64, 0);
        let sentinel = builder.ins().iconst(I64, triet_mir::NULL_SENTINEL);
        let is_zero = builder.ins().icmp(IntCC::Equal, ptr_val, zero);
        let is_sent = builder.ins().icmp(IntCC::Equal, ptr_val, sentinel);
        let is_dead = builder.ins().bor(is_zero, is_sent);

        let setup_bb = builder.create_block();
        let header_bb = builder.create_block();
        builder.append_block_param(header_bb, I64);
        let body_bb = builder.create_block();
        let exit_bb = builder.create_block();

        builder.ins().brif(is_dead, exit_bb, &[], setup_bb, &[]);

        builder.switch_to_block(setup_bb);
        builder.seal_block(setup_bb);
        // Read cap, key_stride + value_stride (packed in header reserved,
        // ADR-0080 Mũi A: high 16 bits = key_stride, low 16 = value_stride).
        let cap = builder.ins().load(I64, mem, ptr_val, 8);
        let hdr = builder.ins().load(I64, mem, ptr_val, -8);
        let reserved = builder.ins().ushr_imm(hdr, 32);
        let value_stride = builder.ins().band_imm(reserved, 0xFFFF);
        let key_stride = builder.ins().ushr_imm(reserved, 16);
        let slot_size = {
            let one = builder.ins().iconst(I64, 1);
            let tmp = builder.ins().iadd(key_stride, value_stride);
            builder.ins().iadd(tmp, one)
        };
        let c16 = builder.ins().iconst(I64, 16);
        let state_off = {
            let tmp = builder.ins().iadd(key_stride, value_stride);
            builder.ins().iadd(tmp, c16)
        };
        let i0 = builder.ins().iconst(I64, 0);
        builder.ins().jump(header_bb, &[BlockArg::from(i0)]);

        builder.switch_to_block(header_bb);
        let i = builder.block_params(header_bb)[0];
        let cond = builder.ins().icmp(IntCC::SignedLessThan, i, cap);
        builder.ins().brif(cond, body_bb, &[], exit_bb, &[]);

        builder.switch_to_block(body_bb);
        builder.seal_block(body_bb);
        let slot_off = builder.ins().imul(i, slot_size);
        // state byte addr = body + state_off + slot_off
        let off = builder.ins().iadd(state_off, slot_off);
        let state_addr = builder.ins().iadd(ptr_val, off);
        let state = builder.ins().load(I64, mem, state_addr, 0);
        let state_byte = builder.ins().band_imm(state, 0xFF);
        let occupied = builder.ins().iconst(I64, 1);
        let is_occ = builder.ins().icmp(IntCC::Equal, state_byte, occupied);
        let skip_bb = builder.create_block();
        let free_bb = builder.create_block();
        builder.ins().brif(is_occ, free_bb, &[], skip_bb, &[]);

        // value cell addr = body + 16 + key_stride + slot_off
        builder.switch_to_block(free_bb);
        builder.seal_block(free_bb);
        let cell_base = builder.ins().iadd_imm(key_stride, 16);
        let value_off = builder.ins().iadd(cell_base, slot_off);
        let value_cell = builder.ins().iadd(ptr_val, value_off);
        self.emit_heap_free_at(builder, value_cell, &eff)?;
        builder.ins().jump(skip_bb, &[]);

        builder.switch_to_block(skip_bb);
        builder.seal_block(skip_bb);
        let i_next = builder.ins().iadd_imm(i, 1);
        builder.ins().jump(header_bb, &[BlockArg::from(i_next)]);

        builder.seal_block(header_bb);
        builder.switch_to_block(exit_bb);
        builder.seal_block(exit_bb);
        Ok(())
    }

    /// HP.2 / ADR-0057: emit disc-dynamic drop glue for an Outcome `local`.
    /// Reads the discriminant from the slot and frees the heap payload of
    /// whichever arm it selects (positive → `value_type`, negative →
    /// `error_type`, zero → no-op). A scalar Outcome (no heap payload) emits
    /// nothing. Returns `true` if `local` is an Outcome (so the caller, e.g.
    /// `Statement::Drop`, knows it was handled), `false` otherwise.
    ///
    /// Shared by `Statement::Drop` (free on scope exit) and the ADR-0057
    /// `Statement::Assign` leak-guard (drop dest's old Outcome before overwrite).
    fn emit_outcome_drop_glue(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        body: &Body,
        local: Local,
    ) -> Result<bool, JitError> {
        let MirType::Outcome {
            value_type,
            error_type,
            ..
        } = &body.local_decls[local.0].ty
        else {
            return Ok(false);
        };
        // Clone to release the immutable borrow on `body` before the
        // `&mut self` calls to `emit_outcome_payload_free`.
        let value_type = value_type.clone();
        let error_type = error_type.clone();
        if !(value_type.is_any_heap() || error_type.is_any_heap()) {
            // Scalar Outcome — nothing to free.
            return Ok(true);
        }
        let slot = *self
            .outcome_slots
            .get(&local)
            .ok_or_else(|| JitError::Unsupported("Outcome Drop without slot".into()))?;
        let disc = builder.ins().stack_load(I64, slot, 0);
        let pos_val = builder.ins().iconst(I64, 1);
        let neg_val = builder.ins().iconst(I64, -1);

        let free_pos_bb = builder.create_block();
        let free_neg_bb = builder.create_block();
        let noop_bb = builder.create_block();
        let merge_bb = builder.create_block();

        // Branch disc == 1 → free_pos.
        let is_pos = builder.ins().icmp(IntCC::Equal, disc, pos_val);
        let fallthrough1 = builder.create_block();
        builder
            .ins()
            .brif(is_pos, free_pos_bb, &[], fallthrough1, &[]);

        // Branch disc == -1 → free_neg; else → noop.
        builder.switch_to_block(fallthrough1);
        let is_neg = builder.ins().icmp(IntCC::Equal, disc, neg_val);
        builder.ins().brif(is_neg, free_neg_bb, &[], noop_bb, &[]);

        // ── free_pos_bb ──
        builder.switch_to_block(free_pos_bb);
        let pos_addr = builder.ins().stack_addr(I64, slot, 8);
        self.emit_heap_free_at(builder, pos_addr, &value_type)?;
        builder.ins().jump(merge_bb, &[]);

        // ── free_neg_bb ──
        builder.switch_to_block(free_neg_bb);
        let neg_addr = builder.ins().stack_addr(I64, slot, 8);
        self.emit_heap_free_at(builder, neg_addr, &error_type)?;
        builder.ins().jump(merge_bb, &[]);

        // ── noop_bb (Zero / scalar payload) ──
        builder.switch_to_block(noop_bb);
        builder.ins().jump(merge_bb, &[]);

        // ── merge ──
        builder.switch_to_block(merge_bb);
        builder.seal_block(free_pos_bb);
        builder.seal_block(free_neg_bb);
        builder.seal_block(noop_bb);
        builder.seal_block(merge_bb);
        builder.seal_block(fallthrough1);
        Ok(true)
    }

    /// ADR-0067 2b-2: emit tag-switch drop-glue for an enum `local`. Reads the
    /// discriminant (disc@0) and, via a `brif` chain over the layout's variants,
    /// frees ONLY the heap payload of the ACTIVE variant (never touches the
    /// inactive variants' garbage). Scalar/unit variants emit no arm. Returns
    /// `true` if `local` is an enum (so the `Statement::Drop` caller knows it was
    /// handled), `false` otherwise.
    ///
    /// This is the slot-based WRAPPER: it resolves the enum's `enum_slot`, forms
    /// the base address (`stack_addr(slot, 0)`), and delegates to the
    /// address-based core `emit_enum_drop_glue_at`. A top-level enum local thus
    /// frees byte-identically to before the 2b+ split.
    fn emit_enum_drop_glue(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        body: &Body,
        local: Local,
    ) -> Result<bool, JitError> {
        // ADR-0076 / R8: unwrap `Enum?` (`Nullable(Enum)`) — the disc-niche stores
        // the discriminant at slot@0 (null = `i64::MIN`), so the tag-switch core
        // is naturally null-safe (MIN matches no heap variant → frees nothing).
        let decl_ty = &body.local_decls[local.0].ty;
        let MirType::Enum(name) = decl_ty.nullable_payload().unwrap_or(decl_ty) else {
            return Ok(false);
        };
        let name = name.clone();
        let slot = self
            .enum_slots
            .get(&local)
            .ok_or_else(|| JitError::Unsupported("enum Drop without slot".into()))?
            .0;
        let base_addr = builder.ins().stack_addr(I64, slot, 0);
        self.emit_enum_drop_glue_at(builder, body, &name, base_addr)?;
        Ok(true)
    }

    /// ADR-0067 2b+: address-based core of the enum tag-switch drop-glue.
    /// `base_addr` points at the enum's first byte (disc@`base_addr+0`,
    /// payload@`base_addr+8`). Used both by the slot-based wrapper above
    /// (top-level enum local) AND by the struct drop walk (`collect_heap_leaves`
    /// yields a `LeafKind::Enum` at `copy_base_addr(local, abs_offset)` — an enum
    /// sitting INSIDE a struct field has no `enum_slot` of its own). Generalizes
    /// `emit_outcome_drop_glue` (2-arm) to N-arm. `emit_heap_free_at` reads
    /// `{ptr@off, cap@off+16}` (String) or `{ptr@off}` (Vector/HashMap).
    fn emit_enum_drop_glue_at(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        body: &Body,
        enum_name: &str,
        base_addr: cranelift_codegen::ir::Value,
    ) -> Result<(), JitError> {
        // Clone (disc_value, payload_ty) for heap-payload variants before the
        // `&mut self` free calls (releases the `body` borrow).
        let heap_variants: Vec<(i64, MirType)> = {
            let layout = body
                .enum_layouts
                .iter()
                .find(|e| e.name == enum_name)
                .ok_or_else(|| {
                    JitError::Unsupported(format!("enum Drop without layout: {enum_name}"))
                })?;
            layout
                .variants
                .iter()
                .filter_map(|v| {
                    v.payload
                        .as_ref()
                        .filter(|p| p.ty.is_any_heap())
                        .map(|p| (v.discriminant_value, p.ty.clone()))
                })
                .collect()
        };
        if heap_variants.is_empty() {
            // Scalar/unit enum — nothing to free.
            return Ok(());
        }
        let payload_off: i64 = 8; // payload_offset, Bậc A
        let mem = cranelift_codegen::ir::MemFlags::new();
        let disc = builder.ins().load(I64, mem, base_addr, 0);
        let merge_bb = builder.create_block();

        for (disc_value, payload_ty) in heap_variants {
            let arm_bb = builder.create_block();
            let next_bb = builder.create_block();
            let dv = builder.ins().iconst(I64, disc_value);
            let is_match = builder.ins().icmp(IntCC::Equal, disc, dv);
            builder.ins().brif(is_match, arm_bb, &[], next_bb, &[]);
            // ── arm: free THIS variant's heap payload ──
            builder.switch_to_block(arm_bb);
            builder.seal_block(arm_bb);
            let addr = builder.ins().iadd_imm(base_addr, payload_off);
            self.emit_heap_free_at(builder, addr, &payload_ty)?;
            builder.ins().jump(merge_bb, &[]);
            // ── fall through: test the next variant ──
            builder.switch_to_block(next_bb);
            builder.seal_block(next_bb);
        }
        // No variant matched (disc = a scalar/unit variant) → no-op → merge.
        builder.ins().jump(merge_bb, &[]);
        builder.switch_to_block(merge_bb);
        builder.seal_block(merge_bb);
        Ok(())
    }

    /// Build the Cranelift IR for a single function body.
    #[allow(clippy::too_many_lines)] // match-heavy dispatch + param-entry, naturally long
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
        self.outcome_slots.clear();
        for i in 0..body.num_locals {
            let var = builder.declare_var(I64);
            self.locals.insert(Local(i), var);
        }
        // ADR-0049 Lát 3: pre-allocate StackSlot for EVERY String-typed local.
        // Ensures all String locals (including move targets from Assign) have
        // slots before switch_to_block, so Drop/free can read cap from slot.
        // Field-0 initialized to 0 so unpopulated locals (parameters, returns)
        // produce free(0, _) = no-op instead of garbage pointer.
        let string_layout = body
            .struct_layouts
            .iter()
            .find(|l| l.name == "String")
            .cloned();
        if let Some(ref layout) = string_layout {
            let align_shift = u32_to_u8(layout.alignment.ilog2());
            for i in 0..body.num_locals {
                let local = Local(i);
                let ty = &body.local_decls[i].ty;
                // ADR-0062: `String?` shares String's 24-byte slot (ptr-sentinel).
                if ty.is_string_repr() {
                    let slot = builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        usize_to_u32(layout.total_size),
                        align_shift,
                    ));
                    self.struct_slots.insert(local, (slot, layout.clone()));
                }
            }
        }
        // ADR-0057: pre-allocate a StackSlot for EVERY Outcome-typed local,
        // not just OutcomeAlloc dests. A merge result (`_2 = move _3`) is typed
        // Outcome (ADR-0056) but never flows through OutcomeAlloc, so without
        // this it has no slot and `_2.disc` (OutcomeDiscriminant) refuses. This
        // is the SINGLE source of Outcome slots (the OutcomeAlloc scan below no
        // longer creates them). Size from outcome_slot_size (16 scalar/32 heap).
        for i in 0..body.num_locals {
            if matches!(body.local_decls[i].ty, MirType::Outcome { .. }) {
                let slot_size = body.local_decls[i].ty.outcome_slot_size();
                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    slot_size,
                    3u8, // log2(8)
                ));
                self.outcome_slots.insert(Local(i), slot);
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
                    let align_shift = u32_to_u8(layout.alignment.ilog2());
                    let slot = builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        usize_to_u32(layout.total_size),
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
                    let align_shift = u32_to_u8(layout.alignment.ilog2());
                    let slot = builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        usize_to_u32(layout.total_size),
                        align_shift,
                    ));
                    self.enum_slots.insert(*dest, (slot, layout.clone()));
                }
                // (Outcome slots pre-allocated for ALL Outcome locals above;
                //  String slots pre-allocated for ALL String locals above.)
            }
        }
        // ── ADR-0065 Lát 1: enum slots for derived Enum / Enum? locals ──
        // EnumAlloc dests get slots above; but `Enum?` locals (`~0` null, match
        // present-bind) and plain `Enum` match-result locals never flow through
        // EnumAlloc, so they have no slot — aggregate-copy's resolve_addr would
        // then fall back to use_var (a garbage pointer). Pre-allocate a slot
        // (enum total_size) for every Enum / Nullable(Enum) local not already
        // covered. Unwrap Nullable at the site (no new predicate, per the work
        // order — mirror of Lát 4.8's `nullable_payload().unwrap_or`).
        for i in 0..body.num_locals {
            let local = Local(i);
            if self.enum_slots.contains_key(&local) {
                continue;
            }
            let ty = &body.local_decls[i].ty;
            let eff = ty.nullable_payload().unwrap_or(ty);
            if let MirType::Enum(enum_name) = eff
                && let Some(layout) = body.enum_layouts.iter().find(|l| l.name == *enum_name)
            {
                let align_shift = u32_to_u8(layout.alignment.ilog2());
                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    usize_to_u32(layout.total_size),
                    align_shift,
                ));
                self.enum_slots.insert(local, (slot, layout.clone()));
            }
        }
        // ── ADR-0065 Lát 2: struct slots for derived Struct / Struct? locals ──
        // Mirror of the enum loop above, but Phương án A prepends a tag word for
        // `Struct?`: a `Nullable(Struct)` slot is `{tag@0, fields@8…}` = layout
        // total_size + 8; a plain `Struct` (match present-bind, Elvis/match
        // result) keeps total_size. The STORED layout is the plain struct layout
        // (field offsets relative to the struct base, offset 0); the +8 tag shift
        // is applied by walk_projections (Delta 3) and the widening/whole-slot
        // Assign branches (Delta 4) — layout offsets are NEVER mutated. Skip
        // "String" (slot allocated above) and already-slotted locals (StructAlloc
        // dests). Phân biệt Struct? (+8) vs Struct (+0) bằng `is_nullable` here —
        // lẫn hai = SIGSEGV (Nhát dao G #2). EXCLUDE the sret-return local
        // (Local 0 when the function returns a struct by sret) and parameter
        // locals: those are received as caller pointers (pointer-based via
        // use_var), giving them a stack slot would shadow the pointer and
        // miscompile boundary structs (172/14).
        let derived_is_sret = matches!(
            body.signature.return_shape,
            triet_mir::ReturnShape::Struct { .. }
        );
        let reserved_locals = if derived_is_sret {
            body.signature.parameters.len() + 1
        } else {
            body.signature.parameters.len()
        };
        for i in 0..body.num_locals {
            let local = Local(i);
            if i < reserved_locals {
                continue;
            }
            if self.struct_slots.contains_key(&local) || self.enum_slots.contains_key(&local) {
                continue;
            }
            let ty = &body.local_decls[i].ty;
            let is_nullable = matches!(ty, MirType::Nullable(_));
            let eff = ty.nullable_payload().unwrap_or(ty);
            if let MirType::Struct(struct_name) = eff
                && struct_name.as_str() != "String"
                && let Some(layout) = body.struct_layouts.iter().find(|l| l.name == *struct_name)
            {
                let size = if is_nullable {
                    layout.total_size + 8
                } else {
                    layout.total_size
                };
                let align_shift = u32_to_u8(layout.alignment.ilog2());
                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    usize_to_u32(size),
                    align_shift,
                ));
                self.struct_slots.insert(local, (slot, layout.clone()));
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

        // Entry block: parameters → var slots. sret: block param[0] → Local(0).
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
        // Param pop below overwrites for String parameters.
        let zero = builder.ins().iconst(I64, 0);
        for (slot, layout) in self.struct_slots.values() {
            if layout.name == "String" {
                builder.ins().stack_store(zero, *slot, 0);
            }
        }

        let mem_flags = cranelift_codegen::ir::MemFlags::new();
        for (i, _param) in body.signature.parameters.iter().enumerate() {
            let local = if is_sret { Local(i + 1) } else { Local(i) };
            let var = self.var(local);
            let param_val = builder.block_params(entry_block)[bp_idx];
            builder.def_var(var, param_val);
            // ADR-0049 Lát 6: String param received as pointer-to-caller-slot.
            // Load {ptr,len,cap} from the caller's slot into our own slot.
            if body.local_decls[local.0].ty.is_string_repr()
                && let Some((slot, _)) = self.struct_slots.get(&local)
            {
                let src_ptr = builder.ins().load(I64, mem_flags, param_val, 0);
                let src_len = builder.ins().load(I64, mem_flags, param_val, 8);
                let src_cap = builder.ins().load(I64, mem_flags, param_val, 16);
                builder.ins().stack_store(src_ptr, *slot, 0);
                builder.ins().stack_store(src_len, *slot, 8);
                builder.ins().stack_store(src_cap, *slot, 16);
            }
            // C1: Enum param received as pointer-to-caller-slot.
            // Create enum_slots entry + load [disc][payload] from pointer.
            // Casts follow struct precedent (StackSlotData API requires u32/u8).
            #[allow(clippy::cast_possible_truncation)]
            if let MirType::Enum(enum_name) = &body.local_decls[local.0].ty
                && let Some(layout) = body.enum_layouts.iter().find(|e| e.name == *enum_name)
            {
                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    usize_to_u32(layout.total_size),
                    layout.alignment.ilog2() as u8,
                ));
                self.enum_slots.insert(local, (slot, layout.clone()));
                // Load disc from caller pointer @ offset 0
                let disc = builder.ins().load(I64, mem_flags, param_val, 0);
                builder.ins().stack_store(disc, slot, 0);
                // Load payload area (8B increments)
                for off in (8..usize_to_i32(layout.total_size)).step_by(8) {
                    let field = builder.ins().load(I64, mem_flags, param_val, off);
                    builder.ins().stack_store(field, slot, off);
                }
            }
            // ADR-0057/0058 gap: Outcome param received as pointer-to-caller-slot
            // (Site 1, call-site packing at ~2676, already correct). Mirror the
            // String/Enum copy-in above — without this, `param_val` is loaded
            // into a plain Variable and never dereferenced, and every
            // `_N.disc`/`_N.payload` read (walk at line 644) reads the fresh,
            // empty slot pre-allocated at line ~1453 instead of the caller's
            // data. disc@0, payload@8, and — for heap Outcome (slot_size 32) —
            // payload_len@16, payload_cap@24.
            if let MirType::Outcome { .. } = &body.local_decls[local.0].ty
                && let Some(&slot) = self.outcome_slots.get(&local)
            {
                let disc = builder.ins().load(I64, mem_flags, param_val, 0);
                builder.ins().stack_store(disc, slot, 0);
                let payload = builder.ins().load(I64, mem_flags, param_val, 8);
                builder.ins().stack_store(payload, slot, 8);
                if body.local_decls[local.0].ty.outcome_slot_size() == 32 {
                    let payload_len = builder.ins().load(I64, mem_flags, param_val, 16);
                    builder.ins().stack_store(payload_len, slot, 16);
                    let payload_cap = builder.ins().load(I64, mem_flags, param_val, 24);
                    builder.ins().stack_store(payload_cap, slot, 24);
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
    #[allow(clippy::too_many_lines)] // JIT lowering dispatch — splitting would scatter Cranelift builder state
    fn lower_block_statements(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        body: &Body,
        block: BasicBlock,
    ) -> Result<(), JitError> {
        let block_data = &body.blocks[block.0];
        for stmt in &block_data.statements {
            match stmt {
                Statement::StorageLive(_, _)
                | Statement::StorageDead(_, _)
                | Statement::StructAlloc { .. }
                | Statement::EnumAlloc { .. }
                | Statement::OutcomeAlloc { .. } => {
                    // No-op at runtime: borrow checker verified safety;
                    // stack slot allocated during build_body
                }
                Statement::Deinit(l, _) => {
                    // ADR-0042: tombstone — zero the slot.
                    let zero = builder.ins().iconst(I64, 0);
                    // HP.3: Outcome Deinit → set disc=0 (tombstone).
                    // Drop glue (HP.2 SwitchInt) sees Zero→no-op.
                    if self.outcome_slots.contains_key(l) {
                        if let Some(slot) = self.outcome_slots.get(l) {
                            builder.ins().stack_store(zero, *slot, 0);
                        }
                    } else if matches!(&body.local_decls[l.0].ty, MirType::Nullable(inner)
                        if matches!(inner.as_ref(), MirType::Struct(_) | MirType::Enum(_)))
                    {
                        // ADR-0076: tombstone an OUTER nullable-aggregate
                        // (`Struct?`/`Enum?`) after a present-bind move. The niche
                        // tag (Struct?, @0) / disc (Enum?, @0) IS the drop-flag:
                        // storing NULL_SENTINEL makes the tag-guarded `Struct?` drop
                        // AND the `Enum?` disc tag-switch a no-op on the join path →
                        // the moved-out payload is freed ONCE (by the bind target),
                        // never twice. Must precede the generic struct/enum branches
                        // below, which zero a FIELD/payload word at the wrong
                        // (non-niche) offset and leave the live ptr → double-free.
                        let sentinel = builder.ins().iconst(I64, triet_mir::NULL_SENTINEL);
                        if let Some((slot, _)) = self.struct_slots.get(l) {
                            builder.ins().stack_store(sentinel, *slot, 0);
                        } else if let Some((slot, _)) = self.enum_slots.get(l) {
                            builder.ins().stack_store(sentinel, *slot, 0);
                        }
                    } else if let Some((slot, layout)) = self.struct_slots.get(l)
                        && layout.name == "String"
                    {
                        builder.ins().stack_store(zero, *slot, 0);
                    } else if let Some((slot, layout)) = self.struct_slots.get(l) {
                        // ADR-0066 (C) + ADR-0067 2a: tombstone a heap-struct after
                        // a move. Zero EVERY heap leaf's ptr@abs_offset so the
                        // later Drop-glue reads ptr=0 → free no-op → no double-free.
                        // SYMMETRIC with the Drop walk (G mandate: free N tiers →
                        // zero N tiers) via the SHARED `collect_heap_leaves` —
                        // recurses nested structs. A Copy struct yields no leaves →
                        // no-op (it never gets a Deinit anyway). String slot
                        // (layout.name=="String") is handled by the branch above.
                        let slot = *slot;
                        let name = layout.name.clone();
                        let mut leaves: Vec<(i32, LeafKind)> = Vec::new();
                        Self::collect_heap_leaves(&name, 0, body, 0, &mut leaves)?;
                        for (abs, kind) in leaves {
                            // ADR-0067 2b+: tombstone per leaf kind. A `Heap` leaf
                            // zeroes the pointer word @abs (free no-op). An `Enum`
                            // leaf zeroes the PAYLOAD word @abs+8 STATICALLY — ptr=0
                            // makes the tag-switch free a no-op for ANY heap variant
                            // — and NEVER touches disc@abs+0 (a valid variant tag,
                            // 2b-3 law).
                            match kind {
                                LeafKind::Heap(_) => {
                                    builder.ins().stack_store(zero, slot, abs);
                                }
                                LeafKind::Enum(_) => {
                                    builder.ins().stack_store(zero, slot, abs + 8);
                                }
                            }
                        }
                    } else if let Some((slot, _layout)) = self.enum_slots.get(l) {
                        // ADR-0067 2b-3: tombstone a heap-enum after a move. Zero
                        // ONLY the payload pointer @payload_offset(8) so the later
                        // tag-switch Drop-glue reads ptr=0 → free no-op → no
                        // double-free. DO NOT touch the discriminant @0: unlike
                        // Outcome (disc=0 = no-payload Zero arm), discriminant 0
                        // is a VALID enum variant that may itself be heap —
                        // zeroing it would mis-route the tag-switch. ptr=0 alone
                        // makes the free shim a no-op for any heap payload type.
                        builder.ins().stack_store(zero, *slot, 8);
                    } else {
                        builder.def_var(self.var(*l), zero);
                    }
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
                        // ADR-0049 Lát 6.3: populate String slot from
                        // compile-time-known values (no heap len/cap).
                        if let Some((slot, _layout)) = self.struct_slots.get(&dest.local) {
                            builder.ins().stack_store(handle, *slot, 0);
                            builder.ins().stack_store(len_val, *slot, 8);
                            builder.ins().stack_store(len_val, *slot, 16); // cap = len
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
                            ConstValue::String(_) => unreachable!("String handled by if-let above"),
                        };
                        builder.def_var(self.var(dest.local), val);
                        // ADR-0062: `~0` (null) into a `String?` local materializes
                        // NULL_SENTINEL into the slot's ptr@0 (the repr is the
                        // 24-byte String slot; len/cap stay don't-care). A scalar
                        // Integer const into a String-repr local only happens for
                        // this null-sentinel materialize. len@8/cap@16 left as-is
                        // — the shim no-ops on ptr@0 == NULL_SENTINEL before
                        // reading cap, and consumers null-check ptr@0 only.
                        if dest.projection.is_empty()
                            && let Some((slot, _)) = self.struct_slots.get(&dest.local)
                        {
                            builder.ins().stack_store(val, *slot, 0);
                        }
                        // ADR-0065 Lát 1: `~0` (null) into an `Enum?` local
                        // materializes NULL_SENTINEL into the enum slot's disc@0
                        // (the disc-sentinel niche). A scalar Integer const into
                        // an enum-repr local only happens for this null-sentinel
                        // materialize; payload area stays don't-care (the match
                        // `~0` arm never reads it).
                        if dest.projection.is_empty()
                            && let Some((slot, _)) = self.enum_slots.get(&dest.local)
                        {
                            builder.ins().stack_store(val, *slot, 0);
                        }
                    }
                }

                Statement::Assign { dest, source, .. } => {
                    // ADR-0057: Outcome slot-to-slot move. When both dest and
                    // source are whole Outcome locals (empty projection) backed
                    // by slots, the value is a 16/32-byte StackSlot — a 1-word
                    // load/store would drop everything but the disc. Copy the
                    // whole slot word-by-word.
                    if dest.projection.is_empty()
                        && source.projection.is_empty()
                        && self.outcome_slots.contains_key(&dest.local)
                        && self.outcome_slots.contains_key(&source.local)
                    {
                        // ADR-0058 §3 Lát 2 ⚰️ LỆNH TỬ HÌNH:
                        // Skip leak-guard for heap Outcome dest. Merge-result
                        // is SSA fresh → slot disc is garbage; leak-guard
                        // would stack_load(disc)→branch→free wild pointer→UB.
                        // Scalar Outcome leak-guard is a safe no-op (returns
                        // early at is_any_heap() check) → keep for defense.
                        let dest_ty = &body.local_decls[dest.local.0].ty;
                        if !dest_ty.has_heap_payload() {
                            // Leak guard (§3.4): drop dest's old Outcome before
                            // overwrite (no-op for scalar payload; safe because
                            // scalar disc is always valid).
                            self.emit_outcome_drop_glue(builder, body, dest.local)?;
                        }
                        let dest_slot = self.outcome_slots[&dest.local];
                        let src_slot = self.outcome_slots[&source.local];
                        let slot_size = body.local_decls[dest.local.0].ty.outcome_slot_size();
                        let size_i32 = i32::try_from(slot_size).map_err(|_| {
                            JitError::Unsupported("Outcome slot size exceeds i32".into())
                        })?;
                        let mut off = 0i32;
                        while off < size_i32 {
                            let v = builder.ins().stack_load(I64, src_slot, off);
                            builder.ins().stack_store(v, dest_slot, off);
                            off += 8;
                        }
                        // Tombstone source disc=0 (§3.3): source's Drop becomes
                        // a no-op → no double-free of the moved Outcome.
                        let zero = builder.ins().iconst(I64, 0);
                        builder.ins().stack_store(zero, src_slot, 0);
                    } else if let Some((tcase, src_off, dest_off, inner_n)) =
                        Self::nullable_struct_taxonomy(body, dest, source)?
                    {
                        // ADR-0065 §12.7: Construction Taxonomy. Faithful walk
                        // gives the real offsets; dispatch by (src_ty, dest_ty)
                        // keeping the Nullable wrapper. Subsumes Delta 4a (WIDEN)
                        // + 4b (WHOLE-COPY) and makes the +8 downcast explicit
                        // (DOWNCAST). Copy-only (rào B8): no tombstone, no
                        // drop-glue. Works for projected places (field-position
                        // construction `_0.p`, readback `_2 = _0.p`) — the gap
                        // Delta 4a/4b never covered (they gated empty-proj).
                        let src_addr = self.copy_base_addr(builder, source.local, src_off);
                        let dest_addr = self.copy_base_addr(builder, dest.local, dest_off);
                        let n_i32 = i32::try_from(inner_n).map_err(|_| {
                            JitError::Unsupported("struct slot size exceeds i32".into())
                        })?;
                        let mem = cranelift_codegen::ir::MemFlags::new();
                        match tcase {
                            NullableStructCopy::WholeCopy => {
                                // case 1: N+8 bytes, tag@0 FIRST → tag (null/
                                // present) propagates verbatim. `inner_n` = N+8.
                                let mut off = 0i32;
                                while off < n_i32 {
                                    let v = builder.ins().load(I64, mem, src_addr, off);
                                    builder.ins().store(mem, v, dest_addr, off);
                                    off += 8;
                                }
                            }
                            NullableStructCopy::Widen => {
                                // case 2: set tag=present(1)@dest+0, copy N field
                                // bytes src+0 → dest+8. `inner_n` = N.
                                let one = builder.ins().iconst(I64, 1);
                                builder.ins().store(mem, one, dest_addr, 0);
                                let mut off = 0i32;
                                while off < n_i32 {
                                    let v = builder.ins().load(I64, mem, src_addr, off);
                                    builder.ins().store(mem, v, dest_addr, off + 8);
                                    off += 8;
                                }
                            }
                            NullableStructCopy::Downcast => {
                                // case 3: copy N field bytes src+8 → dest+0
                                // (drop the tag — this IS match-bind `pt = scrut`).
                                // `inner_n` = N.
                                let mut off = 0i32;
                                while off < n_i32 {
                                    let v = builder.ins().load(I64, mem, src_addr, off + 8);
                                    builder.ins().store(mem, v, dest_addr, off);
                                    off += 8;
                                }
                            }
                        }
                    } else {
                        // Multi-word copy for struct/enum aggregate.
                        let (src_ty, src_off) = Self::walk_projections(body, source)?;
                        let (dest_ty, dest_off) = Self::walk_projections(body, dest)?;
                        // ADR-0065 Lát 2: a downcast of a 1-field (8B) struct
                        // would otherwise fall to the scalar path and read tag@0
                        // instead of the field — force aggregate whenever either
                        // side is a real struct. EXCLUDE "String": hand-built MIR
                        // types a String local as `Struct("String")` (a slot-less
                        // pointer param), so forcing it aggregate would deref the
                        // raw pointer value — same "String" skip as the slot-alloc
                        // loop and is_string_repr.
                        let is_aggregate = Self::ty_total_size(body, &src_ty) > 8
                            || Self::ty_total_size(body, &dest_ty) > 8
                            || matches!(&src_ty, MirType::Struct(n) if n != "String")
                            || matches!(&dest_ty, MirType::Struct(n) if n != "String");
                        if is_aggregate {
                            let src_size = Self::ty_total_size(body, &src_ty);
                            let dest_size = Self::ty_total_size(body, &dest_ty);
                            let copy_size = src_size.max(dest_size);
                            let size_i32 = i32::try_from(copy_size).map_err(|_| {
                                JitError::Unsupported("aggregate copy size exceeds i32".into())
                            })?;
                            let src_addr = self.copy_base_addr(builder, source.local, src_off);
                            let dest_addr = self.copy_base_addr(builder, dest.local, dest_off);
                            let mut off = 0i32;
                            while off < size_i32 {
                                let v = builder.ins().load(
                                    I64,
                                    cranelift_codegen::ir::MemFlags::new(),
                                    src_addr,
                                    off,
                                );
                                builder.ins().store(
                                    cranelift_codegen::ir::MemFlags::new(),
                                    v,
                                    dest_addr,
                                    off,
                                );
                                off += 8;
                            }
                            // Struct/enum types are Copy in Bậc A — no M1 zeroing needed.
                        } else {
                            let val = self.load_place(builder, body, source)?;
                            self.store_place(builder, body, dest, val)?;
                            // ADR-0049 Lát 6.3: sync String slot from source slot.
                            // Read {ptr,len,cap} from source slot if available;
                            // fall back to heap-read for non-slot sources (should
                            // not occur for String after Lát 3 pre-allocation).
                            if dest.projection.is_empty()
                                && let Some((dest_slot, _)) = self.struct_slots.get(&dest.local)
                            {
                                builder.ins().stack_store(val, *dest_slot, 0);
                                if source.projection.is_empty()
                                    && let Some((src_slot, _)) =
                                        self.struct_slots.get(&source.local)
                                {
                                    let src_len = builder.ins().stack_load(I64, *src_slot, 8);
                                    let src_cap = builder.ins().stack_load(I64, *src_slot, 16);
                                    builder.ins().stack_store(src_len, *dest_slot, 8);
                                    builder.ins().stack_store(src_cap, *dest_slot, 16);
                                }
                            }
                            // ADR-0066 STEP 4: a String VALUE stored into a
                            // PROJECTED struct field must copy the FULL fat
                            // pointer. `store_place` above wrote only ptr@dest_off
                            // (one i64); copy len@+8 / cap@+16 from the source
                            // String slot so the inline drop-glue (KCN-1) frees
                            // with the REAL cap — without this, cap@+16 is
                            // uninitialized stack garbage → `__triet_string_free`
                            // is UB (panics on negative/huge cap). String-ONLY:
                            // Vector/HashMap are 8B thin handles (no len/cap in
                            // the slot). Reuses the dest_ty/dest_off from the
                            // walk above; reads src BEFORE M1 zeroing (which only
                            // touches src@0).
                            if !dest.projection.is_empty()
                                && matches!(dest_ty, MirType::String)
                                && let Some((dest_slot, _)) = self.struct_slots.get(&dest.local)
                                && source.projection.is_empty()
                                && let Some((src_slot, _)) = self.struct_slots.get(&source.local)
                            {
                                let src_len = builder.ins().stack_load(I64, *src_slot, 8);
                                let src_cap = builder.ins().stack_load(I64, *src_slot, 16);
                                builder.ins().stack_store(src_len, *dest_slot, dest_off + 8);
                                builder
                                    .ins()
                                    .stack_store(src_cap, *dest_slot, dest_off + 16);
                            }
                            // ADR-0067 2b-0b: a String VALUE stored into a PROJECTED
                            // ENUM payload must copy the FULL fat pointer (analog of
                            // STEP 4 above, but for `enum_slots`). `store_place`
                            // wrote ptr@dest_off; copy len@+8 / cap@+16 from the
                            // source String slot so the tag-switch drop-glue (2b-2)
                            // frees with the REAL cap. dest_off = payload_offset (8)
                            // → len@16 / cap@24 in the 32B enum slot (2b-0a sizing).
                            if !dest.projection.is_empty()
                                && matches!(dest_ty, MirType::String)
                                && let Some((dest_slot, _)) = self.enum_slots.get(&dest.local)
                                && source.projection.is_empty()
                                && let Some((src_slot, _)) = self.struct_slots.get(&source.local)
                            {
                                let src_len = builder.ins().stack_load(I64, *src_slot, 8);
                                let src_cap = builder.ins().stack_load(I64, *src_slot, 16);
                                builder.ins().stack_store(src_len, *dest_slot, dest_off + 8);
                                builder
                                    .ins()
                                    .stack_store(src_cap, *dest_slot, dest_off + 16);
                            }
                            // M1: Zeroing-on-Move — if source is a plain local of Move type,
                            // store 0 into it so Drop becomes a no-op.
                            let source_is_plain = source.projection.is_empty();
                            if source_is_plain {
                                let src_ty = &body.local_decls[source.local.0].ty;
                                if !src_ty.is_copy(Some(body)) {
                                    let zero = builder.ins().iconst(I64, 0);
                                    // ADR-0049 Lát 2 L2-2: Slot-Truth — for String,
                                    // stack_store is the sole guard; def_var dead.
                                    if let Some((slot, layout)) =
                                        self.struct_slots.get(&source.local)
                                        && layout.name == "String"
                                    {
                                        builder.ins().stack_store(zero, *slot, 0);
                                    } else {
                                        self.store_place(
                                            builder,
                                            body,
                                            &Place::local(source.local),
                                            zero,
                                        )?;
                                    }
                                }
                            }
                        } // if is_aggregate
                    }
                    // ADR-0070 read-side: a single heap-field move-out
                    // `_d = move _b.field`. The branches above copied the field
                    // VALUE into dest; now (1) sync dest's full String fat pointer
                    // (the scalar copy path only stored ptr@0 for a projected
                    // source → dest's cap@16 would be stack garbage → free UB) and
                    // (2) tombstone the moved field's heap leaves in the BASE slot
                    // so the base's Drop reads ptr=0 → free no-op → no double-free.
                    // Gated on a Field projection of a non-copy type (heap scalar
                    // or heap-struct); a Copy scalar field never enters. Capability
                    // fields are 0-byte ZSTs (no heap leaf) → no-op.
                    // WO-0075 (ADR-0070 §AMEND Phase 3): accept a MULTI-Field path
                    // (`h.inner.x`), not just exactly-one Field — `walk_projections`
                    // returns the leaf type + its ABSOLUTE offset in the base slot
                    // at any depth, so the tombstone below zeroes the correct leaf.
                    // A non-Field projection (Index/Deref/Payload) is excluded.
                    if !source.projection.is_empty()
                        && source
                            .projection
                            .iter()
                            .all(|p| matches!(p, Projection::Field(_)))
                    {
                        let (field_ty, field_off) = Self::walk_projections(body, source)?;
                        if !field_ty.is_copy(Some(body))
                            && let Some(base_slot) =
                                self.struct_slots.get(&source.local).map(|(s, _)| *s)
                        {
                            let zero = builder.ins().iconst(I64, 0);
                            // (1) String dest: sync len@8/cap@16 (scalar copy
                            // stored only ptr@0 since the source is projected).
                            // `is_string_repr()` so `String?` (24B fat, same repr)
                            // syncs too — else dest cap@16 = stack garbage → free
                            // UB at Drop(dest). Vector?/HashMap? (8B handle) need
                            // no sync.
                            if field_ty.is_string_repr()
                                && dest.projection.is_empty()
                                && let Some(dest_slot) =
                                    self.struct_slots.get(&dest.local).map(|(s, _)| *s)
                            {
                                let len = builder.ins().stack_load(I64, base_slot, field_off + 8);
                                let cap = builder.ins().stack_load(I64, base_slot, field_off + 16);
                                builder.ins().stack_store(len, dest_slot, 8);
                                builder.ins().stack_store(cap, dest_slot, 16);
                            }
                            // (2) tombstone the moved field's heap leaf(s). A heap
                            // SCALAR zeroes its single ptr@field_off. A heap-STRUCT
                            // (Phase 2) recurses its layout via the SHARED
                            // `collect_heap_leaves` with base_offset=field_off → the
                            // leaves come back as ABSOLUTE offsets in the BASE slot;
                            // zero each (SYMMETRIC with the Deinit struct-branch's
                            // base_offset=0 walk, G mandate: free N tiers → zero N
                            // tiers). Capability is a 0-byte ZST (no heap leaf);
                            // Nullable/Outcome field-move stays refused upstream.
                            match &field_ty {
                                MirType::String | MirType::Vector(_) | MirType::HashMap(..) => {
                                    builder.ins().stack_store(zero, base_slot, field_off);
                                }
                                // WO-0074 (Phase 3): a heap-carrying ENUM field
                                // move-out. The enum slot is [disc@0][payload@8];
                                // its heap payload ptr lives at field_off+8 in the
                                // BASE slot. Zero ONLY the payload ptr (disc stays)
                                // so the base's tag-switch Drop glue
                                // (`emit_enum_drop_glue_at`, payload_off=8) reads
                                // ptr=0 → `emit_heap_free_at` returns early → free
                                // no-op. SYMMETRIC with the leaf-Enum tombstone
                                // (`abs + 8`) in the Struct branch below.
                                MirType::Enum(_) => {
                                    builder.ins().stack_store(zero, base_slot, field_off + 8);
                                }
                                MirType::Struct(name) => {
                                    let mut leaves: Vec<(i32, LeafKind)> = Vec::new();
                                    Self::collect_heap_leaves(
                                        name,
                                        field_off,
                                        body,
                                        0,
                                        &mut leaves,
                                    )?;
                                    for (abs, kind) in leaves {
                                        match kind {
                                            LeafKind::Heap(_) => {
                                                builder.ins().stack_store(zero, base_slot, abs);
                                            }
                                            LeafKind::Enum(_) => {
                                                builder.ins().stack_store(zero, base_slot, abs + 8);
                                            }
                                        }
                                    }
                                }
                                // WO-NullableFieldMoveOut (ADR-0076 §AMEND): a
                                // heap-`T?` field (`String?` 24B fat / `Vector?`/
                                // `HashMap?` 8B handle) keeps the heap ptr/handle
                                // @field_off. Zero it (static tombstone) so the
                                // base's Drop reads ptr@field_off ∈ {0, NULL_SENTINEL}
                                // → free shim no-ops (no `brif`, R4). SYMMETRIC with
                                // the plain heap-SCALAR arm above — the only delta is
                                // the `is_string_repr()` len/cap sync handled at (1).
                                MirType::Nullable(inner) if inner.is_any_heap() => {
                                    builder.ins().stack_store(zero, base_slot, field_off);
                                }
                                _ => {}
                            }
                        }
                    }
                }

                Statement::Borrow { dest, source, .. } => {
                    // S6 references = raw pointers at runtime.
                    // ADR-0049 Lát 6.3: for String, pass pointer-to-slot so
                    // the callee can read {ptr,len,cap} (no heap len/cap).
                    if let Some((slot, layout)) = self.struct_slots.get(&source.local)
                        && layout.name == "String"
                    {
                        let val = builder.ins().stack_addr(I64, *slot, 0);
                        let dest_var = self.var(dest.local);
                        builder.def_var(dest_var, val);
                    } else {
                        let src_var = self.var(source.local);
                        let val = builder.use_var(src_var);
                        let dest_var = self.var(dest.local);
                        builder.def_var(dest_var, val);
                    }
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
                Statement::Drop(local, _) => {
                    let ty = &body.local_decls[local.0].ty;
                    if ty.is_copy(Some(body)) {
                        continue;
                    }
                    // ADR-0069: a capability token is a ZST — non-copy (so the
                    // borrow checker move-tracks it) but with NO heap behind it.
                    // Drop is a pure no-op: no shim, no free. Must short-circuit
                    // BEFORE the heap-free dispatch below, which would otherwise
                    // fall through to `Drop for type Capability not supported`.
                    if matches!(ty, MirType::Capability(_)) {
                        continue;
                    }
                    // M4: Return-escape.
                    let in_return = match &body.blocks[block.0].terminator {
                        Terminator::Return { values, .. } => values.contains(local),
                        _ => false,
                    };
                    if in_return {
                        continue;
                    }

                    // HP.2: heap Outcome — inline disc-dynamic drop glue
                    // (scalar Outcome → no-op). Shared with the ADR-0057
                    // Assign leak-guard via `emit_outcome_drop_glue`.
                    if self.emit_outcome_drop_glue(builder, body, *local)? {
                        continue;
                    }

                    // ADR-0067 2b-2: heap enum — inline tag-switch drop-glue
                    // (frees only the active variant's heap payload; scalar enum
                    // → no-op). Before the struct/regular branches.
                    if self.emit_enum_drop_glue(builder, body, *local)? {
                        continue;
                    }

                    // ADR-0066 KCN-1 (1a/1b) + ADR-0067 2a: inline per-struct
                    // static drop-glue. A struct local with heap leaves (B8
                    // relaxed) frees each one. `collect_heap_leaves` walks the
                    // layout RECURSIVELY at compile time (nested non-recursive
                    // structs → flat (abs_offset, leaf) list); `copy_base_addr`
                    // unifies slot-backed locals (1a, `stack_addr`) AND by-pointer
                    // params (1b arg-move callee, `use_var` pointer). The plan is
                    // built before the `&mut self` free calls so the body borrow
                    // is released.
                    // A bare struct (niche 0, unconditional) OR a `Struct?` niche
                    // (ADR-0076: `Nullable(Struct)`, fields@+8, tag@0). For a
                    // `Struct?` a null value (tag@slot+0 == NULL_SENTINEL) has a
                    // GARBAGE field area — unlike a heap LEAF (ptr-sentinel no-op),
                    // a struct's fields can't be sentinel-checked individually, so
                    // the whole free is tag-guarded (mirror of the enum tag-switch).
                    let struct_drop: Option<(String, i32, bool)> = match ty {
                        MirType::Struct(n) => Some((n.clone(), 0, false)),
                        MirType::Nullable(inner) => match inner.as_ref() {
                            MirType::Struct(n) => Some((n.clone(), 8, true)),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some((name, niche, is_nullable)) = struct_drop {
                        let mut leaves: Vec<(i32, LeafKind)> = Vec::new();
                        Self::collect_heap_leaves(&name, niche, body, 0, &mut leaves)?;
                        // ADR-0070: a non-copy struct with NO heap leaves
                        // (capability-only fields are ZSTs) → Drop is a pure no-op.
                        // `collect_heap_leaves` returning Ok(empty) means there is
                        // genuinely nothing to free — a depth-64 recursive type
                        // already bailed via `?` above, so empty here is real, not
                        // a swallowed error. Falling through to the heap-shim
                        // dispatch would wrongly error `Drop for type S not
                        // supported`. (A copy struct already `continue`d earlier.)
                        if leaves.is_empty() {
                            continue;
                        }
                        // ADR-0076: tag-guard the `Struct?` niche so a null value
                        // (garbage fields) frees nothing; a plain struct (and a
                        // present `Struct?`) frees its leaves.
                        let merge_bb = if is_nullable {
                            let slot = self
                                .struct_slots
                                .get(local)
                                .ok_or_else(|| {
                                    JitError::Unsupported("Struct? Drop without slot".into())
                                })?
                                .0;
                            let tag = builder.ins().stack_load(I64, slot, 0);
                            let min = builder.ins().iconst(I64, triet_mir::NULL_SENTINEL);
                            let is_null = builder.ins().icmp(IntCC::Equal, tag, min);
                            let free_bb = builder.create_block();
                            let merge_bb = builder.create_block();
                            builder.ins().brif(is_null, merge_bb, &[], free_bb, &[]);
                            builder.switch_to_block(free_bb);
                            builder.seal_block(free_bb);
                            Some(merge_bb)
                        } else {
                            None
                        };
                        for (abs, kind) in leaves {
                            let addr = self.copy_base_addr(builder, *local, abs);
                            // ADR-0067 2b+: a `Heap` leaf frees unconditionally;
                            // an `Enum` leaf runs the tag-switch core at the
                            // field's address (disc@addr, payload@addr+8) so
                            // only the ACTIVE variant's payload is freed.
                            match kind {
                                LeafKind::Heap(fty) => {
                                    self.emit_heap_free_at(builder, addr, &fty)?;
                                }
                                LeafKind::Enum(enum_name) => {
                                    self.emit_enum_drop_glue_at(builder, body, &enum_name, addr)?;
                                }
                            }
                        }
                        if let Some(merge_bb) = merge_bb {
                            builder.ins().jump(merge_bb, &[]);
                            builder.switch_to_block(merge_bb);
                            builder.seal_block(merge_bb);
                        }
                        continue;
                    }

                    // Regular heap types: call free shim.
                    // ADR-0062: heap-nullable (`String?`/`Vector?`/`HashMap?`)
                    // drops via the same free shim as the non-null type — null
                    // (handle/ptr@0 == NULL_SENTINEL) is a no-op inside the shim
                    // (§4). Unwrap `Nullable(inner)` at the dispatch site so
                    // `Vector?`/`HashMap?` resolve to is_vec/is_hashmap (the
                    // ptr-sentinel shares the inner's repr); String? is already
                    // covered by is_string_repr.
                    let eff = ty.nullable_payload().unwrap_or(ty);
                    // ADR-0077: a Vector Drop frees its heap ELEMENTS (loop) then
                    // the buffer. `emit_vector_free_value` skips the loop for a
                    // scalar/handle element → byte-identical to the legacy
                    // `__triet_vector_free(ptr)`. The handle is an i64 var (the
                    // ptr-sentinel of a null `Vector?` makes the loop + buffer
                    // free no-op inside the shim).
                    if let MirType::Vector(inner) = eff {
                        let inner = (**inner).clone();
                        let ptr = builder.use_var(self.var(*local));
                        self.emit_vector_free_value(builder, ptr, &inner)?;
                        continue;
                    }
                    // ADR-0078/0080: HashMap Drop frees heap KEYS then VALUES
                    // (slot-iteration loops), then the buffer. Scalar
                    // key/value skip their loop → byte-compat.
                    if let MirType::HashMap(k, v) = eff {
                        let key_ty = (**k).clone();
                        let value_ty = (**v).clone();
                        let ptr = builder.use_var(self.var(*local));
                        self.emit_hashmap_free_value(builder, ptr, &key_ty, &value_ty)?;
                        continue;
                    }
                    let free_shim_name = if ty.is_string_repr() {
                        "__triet_string_free"
                    } else if eff.is_hashmap() {
                        "__triet_hashmap_free"
                    } else {
                        return Err(JitError::Unsupported(format!(
                            "Drop for type `{ty}` not supported"
                        )));
                    };
                    let func_id = self.get_or_declare_shim(free_shim_name)?;
                    let func_ref = self.module.declare_func_in_func(func_id, builder.func);
                    if ty.is_string_repr() {
                        let [ptr, cap] = if let Some((slot, _)) = self.struct_slots.get(local) {
                            let p = builder.ins().stack_load(I64, *slot, 0);
                            let c = builder.ins().stack_load(I64, *slot, 16);
                            [p, c]
                        } else {
                            return Err(JitError::Unsupported(
                                "String Drop without pre-allocated slot".into(),
                            ));
                        };
                        builder.ins().call(func_ref, &[ptr, cap]);
                    } else {
                        let ptr = builder.use_var(self.var(*local));
                        builder.ins().call(func_ref, &[ptr]);
                    }
                }

                Statement::SetDiscriminant { dest, value, .. } => {
                    if let Some((slot, _)) = self.enum_slots.get(dest) {
                        let disc_val = builder.ins().iconst(I64, *value);
                        builder.ins().stack_store(disc_val, *slot, 0);
                    }
                }
                Statement::GetDiscriminant { dest, source, .. } => {
                    // If the source has an enum StackSlot, read discriminant from it.
                    // Otherwise, the source IS the discriminant (Bậc A: enum parameters
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

                Statement::CapabilityCheck {
                    capability_name, ..
                } => {
                    // ADR-0069 Lát 3 (§5 LOCK): emit EXACTLY ONE policy-hook
                    // call + a fail-closed trap at the `defer` mint site.
                    // result = __triet_cap_check(cap_id); result <= 0 (Deny −1 OR
                    // Unknown 0) → trap `unwrap_user(2)` (SIGILL). user(2) is
                    // DISTINCT from arithmetic range-check user(1) so a core dump
                    // tells "capability denied" apart from "overflow".
                    let cap_id = builder.ins().iconst(I64, cap_id_hash(capability_name));
                    let func_id = self.get_or_declare_shim("__triet_cap_check")?;
                    let func_ref = self.module.declare_func_in_func(func_id, builder.func);
                    let call_inst = builder.ins().call(func_ref, &[cap_id]);
                    let result = builder.inst_results(call_inst)[0];
                    let zero = builder.ins().iconst(I64, 0);
                    let denied = builder
                        .ins()
                        .icmp(IntCC::SignedLessThanOrEqual, result, zero);
                    builder
                        .ins()
                        .trapnz(denied, cranelift_codegen::ir::TrapCode::unwrap_user(2));
                }
            }

            // NLL loan ending: handled by borrow checker at compile time
        }

        Ok(())
    }

    /// Lower a block terminator.
    #[allow(clippy::too_many_lines)] // JIT lowering dispatch — splitting would scatter Cranelift builder state
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
                    if !matches!(
                        body.signature.return_shape,
                        triet_mir::ReturnShape::BinaryOutcome
                            | triet_mir::ReturnShape::TernaryOutcome
                    ) || values.len() != 2
                    {
                        return Err(JitError::Unsupported(
                            "multi-value return requires Bậc C packed ABI".into(),
                        ));
                    }
                    // ADR-0052 OP.3: BinaryOutcome 2-return — disc, payload.
                    let disc_val = builder.use_var(self.var(values[0]));
                    let payload_val = builder.use_var(self.var(values[1]));
                    builder.ins().return_(&[disc_val, payload_val]);
                    return Ok(());
                }
                let is_sret_ret = matches!(
                    body.signature.return_shape,
                    triet_mir::ReturnShape::Struct { .. }
                );
                if is_sret_ret {
                    // ADR-0049 Lát 6 Lối d: String sret — write {ptr,len,cap}
                    // from local slot to caller's sret buffer (Local(0)).
                    if !values.is_empty() {
                        if let Some((slot, layout)) = self.struct_slots.get(&values[0])
                            && layout.name == "String"
                        {
                            let sret_ptr = builder.use_var(self.var(Local(0)));
                            let mem_flags = cranelift_codegen::ir::MemFlags::new();
                            let ptr = builder.ins().stack_load(I64, *slot, 0);
                            let len = builder.ins().stack_load(I64, *slot, 8);
                            let cap = builder.ins().stack_load(I64, *slot, 16);
                            builder.ins().store(mem_flags, ptr, sret_ptr, 0);
                            builder.ins().store(mem_flags, len, sret_ptr, 8);
                            builder.ins().store(mem_flags, cap, sret_ptr, 16);
                        } else if let Some(slot) = self.outcome_slots.get(&values[0]) {
                            // ADR-0058 Lát 1: heap Outcome sret — write
                            // {disc,payload,len,cap} from slot to caller's
                            // sret buffer (Local(0)).
                            let sret_ptr = builder.use_var(self.var(Local(0)));
                            let mem_flags = cranelift_codegen::ir::MemFlags::new();
                            let disc = builder.ins().stack_load(I64, *slot, 0);
                            let payload = builder.ins().stack_load(I64, *slot, 8);
                            let len = builder.ins().stack_load(I64, *slot, 16);
                            let cap = builder.ins().stack_load(I64, *slot, 24);
                            builder.ins().store(mem_flags, disc, sret_ptr, 0);
                            builder.ins().store(mem_flags, payload, sret_ptr, 8);
                            builder.ins().store(mem_flags, len, sret_ptr, 16);
                            builder.ins().store(mem_flags, cap, sret_ptr, 24);
                        }
                    }
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
                    builder.seal_block(fallthrough);
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
                                if let Some((slot, _)) = self.struct_slots.get(a) {
                                    // ADR-0049: struct param by-pointer
                                    builder.ins().stack_addr(I64, *slot, 0)
                                } else if let Some((slot, _)) = self.enum_slots.get(a) {
                                    // C1: enum param by-pointer (như struct — ADR-0049)
                                    builder.ins().stack_addr(I64, *slot, 0)
                                } else if let Some(slot) = self.outcome_slots.get(a) {
                                    // ADR-0058 Lát 1: heap Outcome sret buffer
                                    // — pass by-pointer.
                                    builder.ins().stack_addr(I64, *slot, 0)
                                } else {
                                    let var = self.var(*a);
                                    builder.use_var(var)
                                }
                            })
                            .collect();

                        // Emit call
                        let call_inst = builder.ins().call(func_ref, &arg_vals);

                        // Store return values.
                        if matches!(
                            return_shape,
                            triet_mir::ReturnShape::BinaryOutcome
                                | triet_mir::ReturnShape::TernaryOutcome
                        ) {
                            // ADR-0052 OP.4a: Outcome call — store 2 return
                            // values into the dest Outcome slot.
                            let disc = builder.inst_results(call_inst)[0];
                            let payload = builder.inst_results(call_inst)[1];
                            if let Some(&slot) = self.outcome_slots.get(&dest[0]) {
                                builder.ins().stack_store(disc, slot, 0);
                                builder.ins().stack_store(payload, slot, 8);
                            }
                        } else if !is_sret_call && !dest.is_empty() {
                            let ret_val = builder.inst_results(call_inst)[0];
                            builder.def_var(self.var(dest[0]), ret_val);
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
                            "__triet_string_eq" | "__triet_string_contains"
                        );
                        let concat_sret = callee_name.as_str() == "__triet_string_concat";
                        let mutate_writeback = matches!(
                            callee_name.as_str(),
                            "__triet_string_clear" | "__triet_string_append"
                        );
                        // ADR-0077: `pop` on a Vector of FAT elements (String 24B)
                        // returns the element by sret — the dest's slot is filled
                        // by memcpy, so the generic i64-return def_var is skipped
                        // (the shim returns 0, not a value).
                        let vector_pop_fat = callee_name.as_str() == "__triet_vector_pop" && {
                            let vty = &body.local_decls[args[0].0].ty;
                            match vty.nullable_payload().unwrap_or(vty) {
                                MirType::Vector(inner) => {
                                    Self::vector_elem_size(inner).is_ok_and(|s| s > 8)
                                }
                                _ => false,
                            }
                        };
                        // ADR-0078: hashmap_remove fat value (String 24B) returns
                        // via out_ptr — same shim→sret pattern as vector_pop.
                        let hashmap_remove_fat = callee_name.as_str() == "__triet_hashmap_remove"
                            && {
                                let mty = &body.local_decls[args[0].0].ty;
                                match mty.nullable_payload().unwrap_or(mty) {
                                    MirType::HashMap(_, v) => {
                                        Self::vector_elem_size(v).is_ok_and(|s| s > 8)
                                    }
                                    _ => false,
                                }
                            };
                        // ADR-0080 §AMEND-1: post-call registry-routed key frees.
                        // `insert`'s redundant incoming key (D.2) frees only when
                        // the shim signals an UPDATE (gated on the flag it wrote
                        // to the scratch slot); `remove`'s resident key (D.5)
                        // frees UNCONDITIONALLY (sentinel-no-op R4 makes the
                        // not-found case safe). Populated below, consumed after
                        // the call is emitted.
                        let mut insert_key_free_gate: Option<(
                            cranelift_codegen::ir::Value,
                            cranelift_codegen::ir::Value,
                        )> = None;
                        let mut remove_key_free_ptr: Option<cranelift_codegen::ir::Value> = None;
                        let arg_vals: Vec<_> = if concat_sret {
                            // C6: concat receives dest_slot as first arg (callee-fill via *mut FatStr).
                            // Followed by bung-field source args {a_ptr, a_len, b_ptr, b_len}.
                            let mut vals = Vec::with_capacity(1 + args.len() * 2);
                            // Pass dest slot pointer as first arg.
                            if let Some((slot, _)) = self.struct_slots.get(&dest[0]) {
                                vals.push(builder.ins().stack_addr(I64, *slot, 0));
                            } else {
                                return Err(JitError::Unsupported(
                                    "concat: dest slot not found".into(),
                                ));
                            }
                            // Bung source String args.
                            for a in args {
                                if let Some((slot, _)) = self.struct_slots.get(a) {
                                    let ptr = builder.ins().stack_load(I64, *slot, 0);
                                    let len = builder.ins().stack_load(I64, *slot, 8);
                                    vals.push(ptr);
                                    vals.push(len);
                                } else {
                                    return Err(JitError::Unsupported(
                                        "concat: String arg without slot".into(),
                                    ));
                                }
                            }
                            vals
                        } else if bung_fields {
                            let mut vals = Vec::with_capacity(args.len() * 2);
                            for a in args {
                                if let Some((slot, _)) = self.struct_slots.get(a) {
                                    let ptr = builder.ins().stack_load(I64, *slot, 0);
                                    let len = builder.ins().stack_load(I64, *slot, 8);
                                    vals.push(ptr);
                                    vals.push(len);
                                } else {
                                    // ADR-0049 Lát 6.3: for &-reference to
                                    // String, the var holds a slot_addr.
                                    // Load {ptr,len} from the pointed-to slot.
                                    let arg_ty = &body.local_decls[a.0].ty;
                                    if arg_ty.is_reference() {
                                        let slot_ptr = builder.use_var(self.var(*a));
                                        let mem_flags = cranelift_codegen::ir::MemFlags::new();
                                        let ptr = builder.ins().load(I64, mem_flags, slot_ptr, 0);
                                        let len = builder.ins().load(I64, mem_flags, slot_ptr, 8);
                                        vals.push(ptr);
                                        vals.push(len);
                                    } else {
                                        // ADR-0049 Lát 6.3: heap len/cap
                                        // removed — this fallback would
                                        // read garbage. Every String-typed
                                        // local must have a pre-allocated
                                        // slot; this path is unreachable.
                                        return Err(JitError::Unsupported(
                                            "bung_fields: String arg without slot — \
                                             universal-slot invariant violated"
                                                .into(),
                                        ));
                                    }
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
                        } else if callee_name == "__triet_vector_alloc" {
                            // ADR-0077: append the per-element STRIDE as a 3rd arg
                            // so the shim stashes it in the buffer header. Stride
                            // comes from the dest's `Vector(inner)` element type —
                            // a Struct/Enum element REFUSES here (P1/P2 boundary).
                            let stride = {
                                let dty = &body.local_decls[dest[0].0].ty;
                                match dty.nullable_payload().unwrap_or(dty) {
                                    MirType::Vector(inner) => Self::vector_elem_size(inner)?,
                                    _ => 8,
                                }
                            };
                            let mut vals: Vec<_> =
                                args.iter().map(|a| builder.use_var(self.var(*a))).collect();
                            vals.push(builder.ins().iconst(I64, stride));
                            vals
                        } else if callee_name == "__triet_hashmap_alloc" {
                            // ADR-0080 Mũi A: append key_stride THEN value_stride
                            // as 3rd/4th args (was value_stride-only pre-0080).
                            // Strides from HashMap(K, V) → vector_elem_size(K/V).
                            let (key_stride, value_stride) = {
                                let dty = &body.local_decls[dest[0].0].ty;
                                match dty.nullable_payload().unwrap_or(dty) {
                                    MirType::HashMap(k, v) => {
                                        (Self::vector_elem_size(k)?, Self::vector_elem_size(v)?)
                                    }
                                    _ => (8, 8),
                                }
                            };
                            let mut vals: Vec<_> =
                                args.iter().map(|a| builder.use_var(self.var(*a))).collect();
                            vals.push(builder.ins().iconst(I64, key_stride));
                            vals.push(builder.ins().iconst(I64, value_stride));
                            vals
                        } else if callee_name == "__triet_vector_push" {
                            // ADR-0077 fat-element ABI: a fat element (stride > 8,
                            // e.g. String 24B) is passed BY-POINTER (stack_addr of
                            // its slot) so the shim memcpy's `stride` bytes; a
                            // scalar/handle element (stride 8) stays by-value i64
                            // (fast path, Vector<Integer> byte-compat).
                            let stride = {
                                let vty = &body.local_decls[args[0].0].ty;
                                match vty.nullable_payload().unwrap_or(vty) {
                                    MirType::Vector(inner) => Self::vector_elem_size(inner)?,
                                    _ => 8,
                                }
                            };
                            let vec_val = builder.use_var(self.var(args[0]));
                            let elem = args[1];
                            let elem_val = if stride > 8 {
                                if let Some((slot, _)) = self.struct_slots.get(&elem) {
                                    builder.ins().stack_addr(I64, *slot, 0)
                                } else {
                                    return Err(JitError::Unsupported(
                                        "vector_push: fat element without a slot (ADR-0077 \
                                         by-pointer ABI requires a pre-allocated element slot)"
                                            .into(),
                                    ));
                                }
                            } else {
                                builder.use_var(self.var(elem))
                            };
                            vec![vec_val, elem_val]
                        } else if callee_name == "__triet_hashmap_insert" {
                            // ADR-0078 fat-value ABI: value_stride <= 8 → by i64;
                            // > 8 → by-pointer. ADR-0080 Mũi B: key_stride follows
                            // the SAME convention for the key arg. ADR-0080
                            // §AMEND-1 D.2: append `is_update_out` — a fresh
                            // scratch i64 slot the shim writes 1/0 into. Only
                            // allocated (and only checked post-call) when the key
                            // is String — Integer keys never redundant-free.
                            let (key_stride, value_stride) = {
                                let mty = &body.local_decls[args[0].0].ty;
                                match mty.nullable_payload().unwrap_or(mty) {
                                    MirType::HashMap(k, v) => {
                                        (Self::vector_elem_size(k)?, Self::vector_elem_size(v)?)
                                    }
                                    _ => (8, 8),
                                }
                            };
                            let map_val = builder.use_var(self.var(args[0]));
                            let key_arg = args[1];
                            let key_val = if key_stride > 8 {
                                if let Some((slot, _)) = self.struct_slots.get(&key_arg) {
                                    builder.ins().stack_addr(I64, *slot, 0)
                                } else {
                                    return Err(JitError::Unsupported(
                                        "hashmap_insert: fat key without a slot (ADR-0080 \
                                         by-pointer ABI requires a pre-allocated key slot)"
                                            .into(),
                                    ));
                                }
                            } else {
                                builder.use_var(self.var(key_arg))
                            };
                            let val_arg = args[2];
                            let value_val = if value_stride > 8 {
                                if let Some((slot, _)) = self.struct_slots.get(&val_arg) {
                                    builder.ins().stack_addr(I64, *slot, 0)
                                } else {
                                    return Err(JitError::Unsupported(
                                        "hashmap_insert: fat value without a slot (ADR-0078 \
                                         by-pointer ABI requires a pre-allocated value slot)"
                                            .into(),
                                    ));
                                }
                            } else {
                                builder.use_var(self.var(val_arg))
                            };
                            let is_update_out = if key_stride > 8 {
                                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                                    StackSlotKind::ExplicitSlot,
                                    8,
                                    3u8, // log2(8)
                                ));
                                let addr = builder.ins().stack_addr(I64, slot, 0);
                                insert_key_free_gate = Some((key_val, addr));
                                addr
                            } else {
                                builder.ins().iconst(I64, 0)
                            };
                            vec![map_val, key_val, value_val, is_update_out]
                        } else if callee_name == "__triet_hashmap_remove" {
                            // ADR-0078: append out_ptr (same pattern as vector_pop).
                            // Fat value → dest slot addr; scalar → 0. ADR-0080
                            // Mũi B: key_stride routes the key arg the same way.
                            // ADR-0080 §AMEND-1 D.5: append `key_out_ptr` — a
                            // fresh scratch 24B slot the shim writes the resident
                            // key's fat bytes into (or NULL_SENTINEL if not
                            // found); freed unconditionally post-call.
                            let key_stride = {
                                let mty = &body.local_decls[args[0].0].ty;
                                match mty.nullable_payload().unwrap_or(mty) {
                                    MirType::HashMap(k, _) => Self::vector_elem_size(k)?,
                                    _ => 8,
                                }
                            };
                            let map_val = builder.use_var(self.var(args[0]));
                            let key_arg = args[1];
                            let key_val = if key_stride > 8 {
                                if let Some((slot, _)) = self.struct_slots.get(&key_arg) {
                                    builder.ins().stack_addr(I64, *slot, 0)
                                } else {
                                    return Err(JitError::Unsupported(
                                        "hashmap_remove: fat key without a slot (ADR-0080 \
                                         by-pointer ABI requires a pre-allocated key slot)"
                                            .into(),
                                    ));
                                }
                            } else {
                                builder.use_var(self.var(key_arg))
                            };
                            let out_ptr = if hashmap_remove_fat {
                                let (slot, _) =
                                    self.struct_slots.get(&dest[0]).ok_or_else(|| {
                                        JitError::Unsupported(
                                            "hashmap_remove: fat value dest without a slot".into(),
                                        )
                                    })?;
                                builder.ins().stack_addr(I64, *slot, 0)
                            } else {
                                builder.ins().iconst(I64, 0)
                            };
                            let key_out_ptr = if key_stride > 8 {
                                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                                    StackSlotKind::ExplicitSlot,
                                    24,
                                    3u8, // log2(8)
                                ));
                                let addr = builder.ins().stack_addr(I64, slot, 0);
                                remove_key_free_ptr = Some(addr);
                                addr
                            } else {
                                builder.ins().iconst(I64, 0)
                            };
                            vec![map_val, key_val, out_ptr, key_out_ptr]
                        } else if matches!(
                            callee_name.as_str(),
                            "__triet_hashmap_get"
                                | "__triet_hashmap_get_ref"
                                | "__triet_hashmap_contains"
                        ) {
                            // ADR-0080 Mũi B: key by-pointer when key_stride>8
                            // (String, fat {ptr,len,cap}), by-value i64 when
                            // Integer (byte-compat) — same convention as
                            // hashmap_insert's key arg. Pure lookup — no
                            // ownership transfer, no post-call free.
                            let key_stride = {
                                let mty = &body.local_decls[args[0].0].ty;
                                match mty.nullable_payload().unwrap_or(mty) {
                                    MirType::HashMap(k, _) => Self::vector_elem_size(k)?,
                                    _ => 8,
                                }
                            };
                            let map_val = builder.use_var(self.var(args[0]));
                            let key_arg = args[1];
                            let key_val = if key_stride > 8 {
                                if let Some((slot, _)) = self.struct_slots.get(&key_arg) {
                                    builder.ins().stack_addr(I64, *slot, 0)
                                } else {
                                    return Err(JitError::Unsupported(format!(
                                        "{callee_name}: fat key without a slot (ADR-0080 \
                                         by-pointer ABI requires a pre-allocated key slot)"
                                    )));
                                }
                            } else {
                                builder.use_var(self.var(key_arg))
                            };
                            vec![map_val, key_val]
                        } else if callee_name == "__triet_vector_pop" {
                            // ADR-0077: append the OUT-pointer. Fat element →
                            // the dest's slot addr (shim memcpy's the element in);
                            // scalar → 0 (unused, element comes back by i64).
                            let vec_val = builder.use_var(self.var(args[0]));
                            let out_ptr = if vector_pop_fat {
                                let (slot, _) =
                                    self.struct_slots.get(&dest[0]).ok_or_else(|| {
                                        JitError::Unsupported(
                                            "vector_pop: fat element dest without a slot".into(),
                                        )
                                    })?;
                                builder.ins().stack_addr(I64, *slot, 0)
                            } else {
                                builder.ins().iconst(I64, 0)
                            };
                            vec![vec_val, out_ptr]
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
                                        // C1: enum param by-pointer (như struct)
                                        builder.ins().stack_addr(I64, *slot, 0)
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
                        // C6: concat returns void (callee writes dest slot via *mut FatStr).
                        if vector_pop_fat || hashmap_remove_fat {
                            // ADR-0077/0078: the shim memcpy'd into the dest
                            // String slot — bind the dest var to ptr@0 (the i64
                            // return is 0, a sentinel).
                            if let Some((slot, _)) = self.struct_slots.get(&dest[0]) {
                                let ptr = builder.ins().stack_load(I64, *slot, 0);
                                builder.def_var(self.var(dest[0]), ptr);
                            } else {
                                return Err(JitError::Unsupported(format!(
                                    "{callee_name}: fat element dest without a slot"
                                )));
                            }
                        } else if !dest.is_empty() && !concat_sret {
                            let ret_val = builder.inst_results(call_inst)[0];
                            builder.def_var(self.var(dest[0]), ret_val);
                            // ADR-0049 Lát 6.3: populate String slot from shim args
                            // (no len/cap on heap). Derive len/cap from known args.
                            if let Some((slot, layout)) = self.struct_slots.get(&dest[0])
                                && layout.name == "String"
                            {
                                builder.ins().stack_store(ret_val, *slot, 0);
                                let (slot_len, slot_cap) = match callee_name.as_str() {
                                    "__triet_string_from_bytes" => {
                                        // args: (ptr, len) — len is arg_vals[1]
                                        (arg_vals[1], arg_vals[1])
                                    }
                                    "__triet_string_alloc" => {
                                        // args: (len, cap)
                                        (arg_vals[0], arg_vals[1])
                                    }
                                    _ => {
                                        // Other shims don't return String.
                                        return Err(JitError::Unsupported(format!(
                                            "unexpected String return from shim `{callee_name}`"
                                        )));
                                    }
                                };
                                builder.ins().stack_store(slot_len, *slot, 8);
                                builder.ins().stack_store(slot_cap, *slot, 16);
                            }
                        }

                        // ADR-0049 Lát 5: clear/append writeback via *mut FatStr;
                        // no manual sync needed — shim handles it.

                        // ADR-0080 §AMEND-1 D.2: insert's redundant incoming key
                        // — free ONLY if the shim signalled an update (dup-
                        // content key kept the resident, made the caller's
                        // incoming key dead). Registry-routed (get_or_declare_shim
                        // inside emit_heap_free_at) → counting-testable. MUST run
                        // BEFORE M3 zeroing below: ADR-0080 Mũi D3 made insert's
                        // key arg consumed (`arg_consumes[1]=true` for a String
                        // key), so M3 would zero `key_addr`'s slot first — this
                        // free reads `key_addr`'s CONTENT (ptr@0/cap@16), and a
                        // zeroed ptr makes `emit_heap_free_at` a silent no-op
                        // (sentinel-no-op R4), LEAKING the redundant key instead
                        // of freeing it. Order was proven wrong by the KM-P1b
                        // D3 regression: `adr0080_km_p1a_update_frees_redundant_key`
                        // would have silently dropped from count 2 → 1.
                        if let Some((key_addr, is_update_addr)) = insert_key_free_gate {
                            let mem = cranelift_codegen::ir::MemFlags::new();
                            let flag = builder.ins().load(I64, mem, is_update_addr, 0);
                            let one = builder.ins().iconst(I64, 1);
                            let is_update = builder.ins().icmp(IntCC::Equal, flag, one);
                            let free_bb = builder.create_block();
                            let merge_bb = builder.create_block();
                            builder.ins().brif(is_update, free_bb, &[], merge_bb, &[]);
                            builder.switch_to_block(free_bb);
                            builder.seal_block(free_bb);
                            self.emit_heap_free_at(builder, key_addr, &MirType::String)?;
                            builder.ins().jump(merge_bb, &[]);
                            builder.switch_to_block(merge_bb);
                            builder.seal_block(merge_bb);
                        }
                        // ADR-0080 §AMEND-1 D.5: remove's resident key — free
                        // UNCONDITIONALLY; the shim wrote NULL_SENTINEL to
                        // `key_out_ptr` on not-found (sentinel-no-op R4), so an
                        // unconditional free is safe either way. `key_out_ptr`
                        // is a FRESH scratch slot (not `args`-derived), so M3
                        // ordering doesn't affect it — kept alongside D.2 above
                        // for readability (both precede M3).
                        if let Some(key_out_ptr) = remove_key_free_ptr {
                            self.emit_heap_free_at(builder, key_out_ptr, &MirType::String)?;
                        }

                        // M3: Zeroing-on-Move — zero consume-arg variables after call.
                        if let Some(meta) = builtin_shim_meta(callee_name) {
                            let zero = builder.ins().iconst(I64, 0);
                            for (i, a) in args.iter().enumerate() {
                                if i < meta.arg_consumes.len() && meta.arg_consumes[i] {
                                    let arg_ty = &body.local_decls[a.0].ty;
                                    if !arg_ty.is_copy(Some(body)) {
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

/// ADR-0069 Lát 3: process-global capability policy. Default `0` = Łukasiewicz
/// Unknown = fail-closed (a `defer` mint traps unless an embedder/test grants).
/// Set via [`__set_cap_policy`]. A process-global (not per-cap) policy is the
/// Lát-3 scope; `cap_id` is passed to the hook for future per-cap routing.
static CAP_POLICY: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// ADR-0069 Lát 3: runtime capability policy hook.
///
/// Returns the Ł3-Trit verdict for `cap_id`: `+1` allow / `-1` deny / `0`
/// Unknown. The JIT traps (fail-closed) on any result `<= 0`. Lát 3 reads a
/// process-global policy and ignores `cap_id` (kept in the ABI for future
/// per-capability routing).
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_cap_check(_cap_id: i64) -> i64 {
    CAP_POLICY.load(std::sync::atomic::Ordering::SeqCst)
}

/// Set the process-global capability policy ([`CAP_POLICY`]). Embedder/test
/// hook — production embedders wire a real policy here; the default `0`
/// (Unknown → fail-closed) holds until then.
pub fn __set_cap_policy(verdict: i64) {
    CAP_POLICY.store(verdict, std::sync::atomic::Ordering::SeqCst);
}

/// ADR-0069 Lát 3: stable FNV-1a hash of a capability name → `cap_id`.
///
/// Threads an identifier to the policy hook; deterministic across runs (NOT
/// `DefaultHasher`, which could vary). Lát 3's process-global policy ignores
/// the value, but a stable id keeps the ABI ready for per-cap routing.
#[must_use]
pub fn cap_id_hash(name: &str) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // Reinterpret the bits as i64 — the hook treats cap_id as an opaque token
    // (bit-cast, not a numeric narrowing → no wrap concern).
    i64::from_ne_bytes(h.to_ne_bytes())
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

/// Layout for a String heap allocation: header + data.
/// ADR-0049 Lát 6.3: len/cap removed from heap — sole truth in `StackSlot`.
fn string_layout(cap: usize) -> std::alloc::Layout {
    let total = HEADER_SIZE + cap; // header + data (no len/cap on heap)
    std::alloc::Layout::from_size_align(total, 8).unwrap()
}

/// `__triet_string_alloc(len, cap)` — allocate a String with given length and capacity.
#[allow(unsafe_code)]
#[allow(clippy::cast_ptr_alignment)] // write_unaligned does not require alignment
#[unsafe(no_mangle)]
pub extern "C" fn __triet_string_alloc(len: i64, cap: i64) -> i64 {
    let cap_usize = i64_to_usize(cap.max(len).max(1)); // at least 1 byte
    let layout = string_layout(cap_usize);
    // SAFETY: layout is valid (power-of-2 alignment, non-zero size).
    let ptr = unsafe { std::alloc::alloc(layout) };
    if ptr.is_null() {
        return 0; // OOM — return null
    }
    // Write ObjectHeader: refcount=1, reserved=0
    // ADR-0049 Lát 6.3: no len/cap on heap — only header.
    // SAFETY: layout guarantees 8-byte aligned, >=8 bytes at ptr.

    unsafe {
        ptr.cast::<u32>().write_unaligned(1u32); // refcount = 1
        ptr.cast::<u32>().add(1).write_unaligned(0u32); // reserved = 0
        ptr.add(HEADER_SIZE) as i64 // body = data area
    }
}

/// `__triet_string_from_bytes(ptr, len)` — copy bytes from read-only memory into a new heap String.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_string_from_bytes(src: i64, len: i64) -> i64 {
    if src == 0 || len < 0 {
        return 0;
    }
    let len_usize = i64_to_usize(len);
    let body_ptr = __triet_string_alloc(len, len);
    if body_ptr == 0 {
        return 0;
    }
    // Copy bytes from src to data area.
    // ADR-0049 Lát 6.3: no len/cap on heap — data starts at body_ptr.
    // SAFETY: src pointer is valid (lifetime guaranteed by driver §3.3).
    unsafe {
        let dst = body_ptr as *mut u8;
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
    let cap_usize = i64_to_usize(cap.max(1));
    let layout = string_layout(cap_usize);
    let body = ptr as *mut u8;
    let header = unsafe { body.sub(HEADER_SIZE) };
    // SAFETY: layout matches the one used at allocation.
    unsafe { std::alloc::dealloc(header, layout) };
}

/// `__triet_string_concat(dest_slot, a_ptr, a_len, b_ptr, b_len)` — concatenate two Strings.
///
/// C6: callee-fill via `*mut FatStr` writeback (append precedent, ADR-0049).
/// `dest_slot` is a pointer to the caller's `StackSlot`; the callee writes
/// `{new_ptr, total_len, total_cap}` directly into it.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_string_concat(
    dest_slot: i64,
    a_ptr: i64,
    a_len: i64,
    b_ptr: i64,
    b_len: i64,
) {
    if a_ptr == 0 || b_ptr == 0 {
        std::process::abort();
    }
    let total_len = if a_len >= 0 && b_len >= 0 {
        a_len + b_len
    } else {
        return; // invalid input — leave slot as-is
    };
    let result = __triet_string_alloc(total_len, total_len);
    if result == 0 {
        return;
    }
    let a_data = a_ptr as *const u8;
    let b_data = b_ptr as *const u8;
    // SAFETY: src pointers valid, dst pointer valid with sufficient capacity.
    unsafe {
        let dst = result as *mut u8;
        std::ptr::copy_nonoverlapping(a_data, dst, i64_to_usize(a_len));
        std::ptr::copy_nonoverlapping(b_data, dst.add(i64_to_usize(a_len)), i64_to_usize(b_len));
    }
    // C6: write {ptr, len, cap} into caller's slot (append precedent).
    let slot = dest_slot as *mut FatStr;
    unsafe {
        (*slot).ptr = result;
        (*slot).len = total_len;
        (*slot).cap = total_len;
    }
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
    let len = i64_to_usize(a_len);
    // ADR-0049 Lát 6.3: no len/cap on heap — data starts at body (ptr itself).
    // SAFETY: data areas are valid reads of `len` bytes.
    unsafe {
        let a_bytes = a_ptr as *const u8;
        let b_bytes = b_ptr as *const u8;
        for i in 0..len {
            if a_bytes.add(i).read() != b_bytes.add(i).read() {
                return -1;
            }
        }
    }
    1
}

/// `__triet_string_hash(ptr, len) -> i64` — FNV-1a content hash (ADR-0080 Mũi B).
///
/// Mirrors `cap_id_hash` (:3372) — deterministic across runs (NOT
/// `DefaultHasher`, which is process-seeded). Two Strings with equal content
/// but different allocations hash identically (ADR-0080 tooth #5). Trap-on-0
/// ptr, matching `__triet_string_eq`'s C9 rule.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_string_hash(ptr: i64, len: i64) -> i64 {
    if ptr == 0 {
        std::process::abort();
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    // SAFETY: ADR-0049 Lát 6.3 — no len/cap on heap, data starts at ptr.
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, i64_to_usize(len)) };
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    i64::from_ne_bytes(h.to_ne_bytes())
}

/// `__triet_string_len(ptr)` — return the length of a String.
///
/// ADR-0049 Lát 6.3: for borrowed String, `ptr` is a pointer to the owner's
/// `StackSlot` (`slot_addr` from Borrow). Len lives at slot offset 8.
/// Owned String `length` is handled by `Field("len")` projection in the JIT
/// and never calls this shim.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_string_len(ptr: i64) -> i64 {
    // C9 trap-on-0: 0 = dead value (moved-out / OOM), never a valid pointer.
    if ptr == 0 {
        std::process::abort();
    }
    // SAFETY: ptr points to a StackSlot; len is at offset 8.
    unsafe { (ptr as *const i64).add(1).read_unaligned() }
}

// ADR-0049 Lát 5: fat-pointer layout mirrored in shim for writeback.
// Must match StackSlot: ptr@0, len@8, cap@16.
#[repr(C)]
struct FatStr {
    ptr: i64,
    len: i64,
    cap: i64,
}

/// `__triet_string_clear(slot_ptr)` — *mut `FatStr` writeback: len=0, ptr unchanged.
/// ADR-0049 Lát 5: receives pointer to caller's `StackSlot`, writes back via pointer.
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
        // ADR-0049 Lát 6.3: no heap len/cap — slot is sole truth.
    }
    0 // Unit
}

/// `__triet_string_append(slot_ptr, byte)` — append one byte, realloc if needed.
///
/// ADR-0049 Lát 5: `*mut FatStr` writeback. Reads {ptr,len,cap} from slot,
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
            // ADR-0049 Lát 6.3: copy header + data (no len/cap on heap).
            let old_header = (ptr as *mut u8).sub(HEADER_SIZE);
            let new_header = (new_body as *mut u8).sub(HEADER_SIZE);
            let old_total = HEADER_SIZE + i64_to_usize(cap);
            std::ptr::copy_nonoverlapping(old_header, new_header, old_total);
            // Free old block
            let old_layout = string_layout(i64_to_usize(cap.max(1)));
            std::alloc::dealloc(old_header, old_layout);
            ptr = new_body;
            cap = new_cap;
        }

        // Write byte at data[len].
        // ADR-0049 Lát 6.3: data starts at ptr (no len/cap prefix).
        if !(0..=255).contains(&byte) {
            // E1b-2 / ADR-0044: trap instead of silent truncation
            std::process::abort();
        }
        let data = ptr as *mut u8;
        data.add(i64_to_usize(len)).write(i64_low_byte(byte));

        // Writeback to slot — sole truth.
        (*slot).ptr = ptr;
        (*slot).len = len + 1;
        (*slot).cap = cap;
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
    // ADR-0049 Lát 6.3: no len/cap on heap — data starts at body (ptr itself).
    // SAFETY: data areas are valid reads.
    unsafe {
        let h_data = h_ptr as *const u8;
        let n_data = n_ptr as *const u8;
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

/// Layout for a Vector heap allocation: header + len (i64) + cap (i64) +
/// data (cap × stride). ADR-0077 Typed Vector P1: the per-element `stride`
/// (8 for scalar/handle, 24 for String) is no longer hardcoded — it is stashed
/// in the header's `reserved` field at alloc and read back by push/get/free.
#[allow(clippy::missing_const_for_fn)] // `Layout::from_size_align` is not const-stable
fn vector_layout(cap: usize, stride: usize) -> std::alloc::Layout {
    let total = HEADER_SIZE + 8 + 8 + cap * stride; // header + len + cap + data
    std::alloc::Layout::from_size_align(total, 8).unwrap()
}

/// Read the element `stride` from a live Vector body's header (ADR-0077).
/// `body` is the returned data pointer; the stride lives in the `reserved`
/// u32 at `header + 4` = `body - 4`.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_ptr_alignment,
    clippy::ptr_as_ptr,
    clippy::missing_const_for_fn // raw-ptr read_unaligned is not const-stable
)]
fn vector_stride(body: i64) -> usize {
    // SAFETY: body points just past the 8-byte header; reserved is at body-4.
    unsafe { ((body as *const u8).sub(4) as *const u32).read_unaligned() as usize }
}

/// `__triet_vector_alloc(len, cap, stride)` — allocate a Vector (ADR-0077).
///
/// `stride` is the per-element byte size; it is stored in the header's
/// `reserved` field so push/get/free recover it without a param.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,        // len/cap/stride are non-negative by construction
    clippy::cast_possible_truncation, // 64-bit target, values fit in usize
    clippy::cast_ptr_alignment,    // write_unaligned used, alignment irrelevant
    clippy::ptr_as_ptr             // idiomatic in extern "C" heap code (mirrors String shims)
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_vector_alloc(len: i64, cap: i64, stride: i64) -> i64 {
    let cap_usize = i64_to_usize(cap.max(len).max(2)); // at least cap=2 for realloc teeth
    let stride_usize = i64_to_usize(stride.max(8)); // floor at 8 (scalar/handle)
    let layout = vector_layout(cap_usize, stride_usize);
    // SAFETY: layout is valid (power-of-2 alignment, non-zero size).
    let ptr = unsafe { std::alloc::alloc(layout) };
    if ptr.is_null() {
        return 0; // OOM — return null
    }
    // Write ObjectHeader: refcount=1, reserved=stride (ADR-0077).
    // SAFETY: layout guarantees 8-byte aligned, >=8 bytes at ptr.

    unsafe {
        (ptr as *mut u32).write_unaligned(1u32); // refcount = 1
        (ptr as *mut u32)
            .add(1)
            .write_unaligned(stride_usize as u32); // reserved = stride
        // Write len and cap
        let body = ptr.add(HEADER_SIZE);
        (body as *mut i64).write_unaligned(len);
        (body as *mut i64)
            .add(1)
            .write_unaligned(usize_to_i64(cap_usize));
        body as i64
    }
}

/// `__triet_vector_free(ptr)` — free a Vector's BUFFER (the backing block).
/// No-op if ptr == 0 or ptr == `NULL_SENTINEL` (C4 moved-out + ADR-0041 §5.5).
///
/// ADR-0077: this frees ONLY the buffer, not the elements. For a `Vector<T>`
/// with heap `T`, the JIT emits a per-element free loop (reusing
/// `emit_heap_free_at`) BEFORE calling this — so element frees route through
/// the shim registry and are counted/poison-testable. A scalar/handle element
/// (`elem_kind` 0) needs no loop; only this buffer dealloc runs.
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
    // Read cap + stride to compute the exact allocation layout.
    // SAFETY: body pointer is valid and points to len+cap+data structure.
    let cap = i64_to_usize(unsafe { (body as *const i64).add(1).read_unaligned() });
    let stride = vector_stride(ptr);
    let layout = vector_layout(cap.max(2), stride);
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
    // Read old len, cap, and the per-element stride (ADR-0077 — from header).
    // SAFETY: vec points to valid body.
    let (old_len, old_cap) = unsafe {
        let l = (old_body as *const i64).read_unaligned();
        let c = (old_body as *const i64).add(1).read_unaligned();
        (i64_to_usize(l), i64_to_usize(c))
    };
    let stride = vector_stride(vec);
    let new_len = old_len + 1;
    let new_cap = if new_len > old_cap {
        old_cap * 2
    } else {
        old_cap
    };
    // Carry the SAME stride into the new buffer (header → header).
    let new_body = __triet_vector_alloc(new_len as i64, new_cap as i64, usize_to_i64(stride));
    if new_body == 0 {
        return 0;
    }
    // Copy old elements (byte-exact by stride) and write the new element.
    // ADR-0077 fat-element ABI (MŨI 4): `stride <= 8` → `elem` is a by-value
    // i64 (scalar/handle) written into its 8-byte cell; `stride > 8` → `elem`
    // is a POINTER to the source element (String 24B fat) memcpy'd `stride`
    // bytes. The header stride disambiguates — no separate fat shim.
    // SAFETY: old_body and new_body point to valid data areas sized by stride.
    unsafe {
        let old_data = old_body.add(16); // skip len + cap
        let new_data = (new_body as *mut u8).add(16);
        std::ptr::copy_nonoverlapping(old_data, new_data, old_len * stride);
        let dst = new_data.add(old_len * stride);
        if stride <= 8 {
            (dst as *mut i64).write_unaligned(elem);
        } else {
            std::ptr::copy_nonoverlapping(elem as *const u8, dst, stride);
        }
    }
    // Free old buffer ONLY (elements were moved byte-wise into the new buffer —
    // freeing elements here would double-free). Explicit alloc+free, no realloc.
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
    // ADR-0077: index by the per-element stride (header). Returns the 8-byte
    // word at `data + idx*stride` — correct for scalar/handle elements
    // (stride 8). Heap-element `get` (stride 24) is REFUSED at typecheck
    // (Slice B): the owned element must leave via `pop` (move-out), not a
    // copy through a single i64.
    let stride = vector_stride(vec);
    // SAFETY: idx is in [0, len), data area starts at offset 16.
    unsafe {
        let data = body.add(16);
        (data.add(idx as usize * stride) as *const i64).read_unaligned()
    }
}

/// ADR-0079 Slice B: `get_ref(&0 vec, idx)` — zero-copy borrow of the element slot.
///
/// Returns a POINTER to the element within the Vector buffer (NOT a copy).
/// Out-of-bounds or empty → `NULL_SENTINEL`. ZERO-COPY: no allocation, no
/// memcpy — the caller reads directly from the buffer through the returned
/// reference. Valid as long as the vector is not mutated (enforced by
/// borrowck U2/U3).
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment,
    clippy::ptr_as_ptr
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_vector_get_ref(vec: i64, idx: i64) -> i64 {
    if vec == 0 {
        std::process::abort();
    }
    let body = vec as *const u8;
    let len = unsafe { (body as *const i64).read_unaligned() };
    if idx < 0 || idx >= len {
        return triet_mir::NULL_SENTINEL;
    }
    let stride = vector_stride(vec);
    // Zero-copy: return the address of the element slot.
    unsafe { body.add(16).add(idx as usize * stride) as i64 }
}

/// `__triet_vector_pop(vec, out_ptr)` — MOVE the last element out (ADR-0077).
///
/// Decrements the buffer's `len` in place so the popped slot is no longer owned
/// by the vector — `Drop(vec)` then frees only the `len-1` survivors, and the
/// popped element is owned by the caller (no double-free, no mid-array hole).
/// Ownership is CLEANLY CUT.
///
/// ABI (stride-disambiguated, like push):
/// - `stride <= 8` (scalar/handle): returns the element by value as i64;
///   `out_ptr` is unused (the JIT passes 0).
/// - `stride > 8` (fat, String 24B): memcpy's `stride` bytes of the element
///   into `out_ptr` (the caller's dest slot) and returns 0.
///
/// `vec == 0` (dead) or empty (`len == 0`) → `NULL_SENTINEL`, no decrement.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment,
    clippy::ptr_as_ptr
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_vector_pop(vec: i64, out_ptr: i64) -> i64 {
    if vec == 0 {
        std::process::abort(); // trap-on-0 (dead value)
    }
    let body = vec as *mut u8;
    // SAFETY: vec points to a valid buffer {len@0, cap@8, data@16}.
    let len = unsafe { (body as *const i64).read_unaligned() };
    if len <= 0 {
        // ADR-0077 P1.5: for a fat element (stride > 8) the JIT reads the
        // dest slot directly (skipping the i64 return), so the out_ptr must
        // carry NULL_SENTINEL or the slot contains stack garbage → the match
        // `~0` arm is misrouted. Write the sentinel into the first word of
        // the fat element slot.
        let stride = vector_stride(vec);
        if stride > 8 {
            unsafe { (out_ptr as *mut i64).write_unaligned(triet_mir::NULL_SENTINEL) };
        }
        return triet_mir::NULL_SENTINEL; // empty → nothing to pop
    }
    let stride = vector_stride(vec);
    let new_len = len - 1;
    // Decrement len in place — the popped slot leaves the vector's ownership.
    // SAFETY: body@0 is the len field.
    unsafe { (body as *mut i64).write_unaligned(new_len) };
    // SAFETY: new_len in [0, len), data starts at offset 16.
    unsafe {
        let elem = body.add(16).add(new_len as usize * stride);
        if stride <= 8 {
            (elem as *const i64).read_unaligned()
        } else {
            std::ptr::copy_nonoverlapping(elem, out_ptr as *mut u8, stride);
            0
        }
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

// ── HashMap shims (ADR-0043; ADR-0078 typed value P1; ADR-0080 key-typed P1) ──
//
// Slot layout (ADR-0080 Mũi A): [key@key_stride | value@value_stride | state1].
// `key_stride ∈ {8, 24}` — 8 = Integer (identity, byte-compat with pre-0080
// maps), 24 = String (fat `{ptr,len,cap}`, content hash/eq). Both strides are
// packed into the header `reserved` u32 (high 16 bits = key_stride, low 16
// bits = value_stride) — the buffer is SELF-DESCRIBING: `free`/`rehash` read
// key kind from the header alone, no external type info needed (ADR-0080 Mũi
// A invariant). Neither stride ever exceeds 24, so u16 packing never truncates.

const HASHMAP_HEADER_SIZE: usize = HEADER_SIZE; // 8B ObjectHeader

/// Read the per-slot VALUE stride (low 16 bits of the packed `reserved`
/// header field — ADR-0080 Mũi A).
#[allow(unsafe_code)]
#[allow(clippy::cast_ptr_alignment, clippy::ptr_as_ptr)]
const fn hashmap_value_stride(body: i64) -> usize {
    unsafe { ((body as *const u8).sub(4) as *const u32).read_unaligned() as usize & 0xFFFF }
}

/// Read the per-slot KEY stride (high 16 bits of the packed `reserved`
/// header field — ADR-0080 Mũi A). `8` = Integer (identity), `24` = String
/// (content hash/eq). Doubles as the key-kind discriminator — no separate
/// tag byte ("`key_stride` kiêm luôn `discriminator` dispatch").
#[allow(unsafe_code)]
#[allow(clippy::cast_ptr_alignment, clippy::ptr_as_ptr)]
const fn hashmap_key_stride(body: i64) -> usize {
    unsafe { ((body as *const u8).sub(4) as *const u32).read_unaligned() as usize >> 16 }
}

#[allow(clippy::missing_const_for_fn)]
fn hashmap_slot_size(body: i64) -> usize {
    hashmap_key_stride(body) + hashmap_value_stride(body) + 1 // key + value + state
}

#[allow(clippy::missing_const_for_fn)]
fn hashmap_layout(cap: usize, key_stride: usize, value_stride: usize) -> std::alloc::Layout {
    let slot = key_stride + value_stride + 1;
    let total = HASHMAP_HEADER_SIZE + 8 + 8 + cap * slot;
    std::alloc::Layout::from_size_align(total, 8).unwrap()
}

/// State byte pointer for a slot. Key is at `body + 16 + idx*slot_size`;
/// value is at `key + key_stride`; state is at `value + value_stride`.
#[allow(unsafe_code)]
unsafe fn hashmap_state_ptr(body: *mut u8, idx: usize) -> *mut u8 {
    let slot = hashmap_slot_size(body as i64);
    let ks = hashmap_key_stride(body as i64);
    let vs = hashmap_value_stride(body as i64);
    unsafe { body.add(16 + idx * slot + ks + vs) }
}

/// Key cell pointer for a slot (= first field, width `key_stride`).
/// ADR-0080: `*mut u8` (was `*mut i64`) — a String key's cell is 24 bytes
/// (`FatStr`-shaped `{ptr,len,cap}`), not a single i64. Integer callers cast
/// to `*const i64`/`*mut i64` at the read/write site.
#[allow(unsafe_code, clippy::ptr_as_ptr, clippy::cast_ptr_alignment)]
unsafe fn hashmap_key_ptr(body: *mut u8, idx: usize) -> *mut u8 {
    let slot = hashmap_slot_size(body as i64);
    unsafe { body.add(16 + idx * slot) }
}

/// Value pointer for a slot (= after the key, width `key_stride`).
#[allow(unsafe_code)]
unsafe fn hashmap_value_ptr(body: *mut u8, idx: usize) -> *mut u8 {
    let slot = hashmap_slot_size(body as i64);
    let ks = hashmap_key_stride(body as i64);
    unsafe { body.add(16 + idx * slot + ks) }
}

/// Copy a VALUE into a slot. `stride <= 8` → write i64; else memcpy `stride`
/// bytes from `src` (a pointer to the caller's source element).
#[allow(unsafe_code)]
const unsafe fn hashmap_write_value(dst: *mut u8, stride: usize, src: i64) {
    unsafe {
        if stride <= 8 {
            dst.cast::<i64>().write_unaligned(src);
        } else {
            std::ptr::copy_nonoverlapping(src as *const u8, dst, stride);
        }
    }
}

/// Write a KEY into a slot (ADR-0080 Mũi A). `key_stride <= 8` → raw i64
/// (Integer, byte-compat). `key_stride > 8` (24, String) → memcpy the fat
/// `{ptr,len,cap}` FROM `k` (a POINTER to the caller's fat key, same by-
/// pointer convention as `hashmap_write_value`'s fat path) — this transfers
/// the caller's String ownership bytes into the slot (the map now owns it).
#[allow(unsafe_code)]
const unsafe fn hashmap_write_key(dst: *mut u8, key_stride: usize, k: i64) {
    unsafe {
        if key_stride <= 8 {
            dst.cast::<i64>().write_unaligned(k);
        } else {
            std::ptr::copy_nonoverlapping(k as *const u8, dst, key_stride);
        }
    }
}

/// Probe-start slot index for key `k` against `cap` (ADR-0080 Mũi B).
/// `key_stride <= 8` (Integer): identity modulo on `k` itself — BYTE-COMPAT
/// with pre-ADR-0080 maps. `key_stride > 8` (String): `k` is a POINTER to
/// the caller's fat `{ptr,len,cap}` key; hash is the FNV-1a content hash
/// (`__triet_string_hash`) of `{ptr,len}` — two Strings with equal content
/// but different allocations hash identically (ADR-0080 tooth #5).
/// `slot_size = key_stride + value_stride + 1` is ODD whenever `value_stride`
/// is even (always — 8 or 24), so slot N>0's key cell is NOT guaranteed
/// 8-byte aligned (unlike the caller-stack `k` pointer, which alignment
/// varies too — `k` is a `*const FatStr` from a JIT `stack_addr` OR a raw slot
/// pointer during rehash). Every `FatStr` access here MUST go through
/// `read_unaligned` — a plain `&*(ptr as *const FatStr)` reference deref
/// panics ("misaligned pointer dereference") the moment a probe lands past
/// slot 0.
#[allow(unsafe_code, clippy::cast_ptr_alignment, clippy::ptr_as_ptr)]
fn hashmap_key_hash(key_stride: usize, k: i64, cap: usize) -> usize {
    let raw = if key_stride > 8 {
        let fat = unsafe { (k as *const FatStr).read_unaligned() };
        __triet_string_hash(fat.ptr, fat.len)
    } else {
        k
    };
    let cap_i = usize_to_i64(cap);
    i64_to_usize((raw % cap_i + cap_i) % cap_i)
}

/// Equality between a stored slot key (`slot_key_ptr`, width `key_stride`)
/// and a probe key `k` (same shape convention as `hashmap_key_hash`: raw
/// i64 for Integer, pointer-to-fat for String). String path reuses
/// `__triet_string_eq` (content compare, ADR-0080 tooth #5); Integer path is
/// the original identity `==` (byte-compat). `read_unaligned` throughout —
/// see `hashmap_key_hash`'s doc for why a plain reference deref is unsound.
#[allow(unsafe_code, clippy::cast_ptr_alignment, clippy::ptr_as_ptr)]
unsafe fn hashmap_key_eq(slot_key_ptr: *const u8, key_stride: usize, k: i64) -> bool {
    if key_stride > 8 {
        let slot_fat = unsafe { (slot_key_ptr as *const FatStr).read_unaligned() };
        let k_fat = unsafe { (k as *const FatStr).read_unaligned() };
        __triet_string_eq(slot_fat.ptr, slot_fat.len, k_fat.ptr, k_fat.len) == 1
    } else {
        let stored = unsafe { (slot_key_ptr as *const i64).read_unaligned() };
        stored == k
    }
}

/// `__triet_hashmap_alloc(len, cap, key_stride, value_stride)` — allocate a
/// `HashMap` (ADR-0080 Mũi A: +`key_stride` arg vs. the pre-ADR-0080 3-arg form).
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment,
    clippy::ptr_as_ptr
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_hashmap_alloc(
    len: i64,
    cap: i64,
    key_stride: i64,
    value_stride: i64,
) -> i64 {
    let key_stride = i64_to_usize(key_stride.max(8));
    let value_stride = i64_to_usize(value_stride.max(8));
    let cap_usize = (cap.max(4) as usize).max(len as usize + 1).max(4);
    let layout = hashmap_layout(cap_usize, key_stride, value_stride);
    let ptr = unsafe { std::alloc::alloc(layout) };
    if ptr.is_null() {
        return 0; // OOM
    }
    unsafe {
        (ptr as *mut u32).write_unaligned(1u32); // refcount = 1
        // reserved = packed (key_stride<<16 | value_stride) — ADR-0080 Mũi A.
        let packed = ((key_stride as u32) << 16) | (value_stride as u32);
        (ptr as *mut u32).add(1).write_unaligned(packed);
        let body = ptr.add(HASHMAP_HEADER_SIZE);
        (body as *mut i64).write_unaligned(len);
        (body as *mut i64)
            .add(1)
            .write_unaligned(usize_to_i64(cap_usize));
        // Zero state bytes
        let slot_size = key_stride + value_stride + 1;
        for i in 0..cap_usize {
            let state = body.add(16 + i * slot_size + key_stride + value_stride);
            state.write_unaligned(0u8);
        }
        body as i64
    }
}

/// Free a `HashMap` BUFFER. No-op if ptr == 0 or `NULL_SENTINEL`.
///
/// ADR-0078/0080: frees ONLY the buffer; the JIT emits per-slot key-free
/// (ADR-0080 D.1) and value-free (ADR-0078) loops before calling this (for
/// heap keys/values).
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment,
    clippy::ptr_as_ptr
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_hashmap_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    let body = ptr as *mut u8;
    let cap = unsafe { (body as *const i64).add(1).read_unaligned() } as usize;
    let ks = hashmap_key_stride(ptr);
    let vs = hashmap_value_stride(ptr);
    let layout = hashmap_layout(cap.max(2), ks, vs);
    let header = unsafe { body.sub(HASHMAP_HEADER_SIZE) };
    unsafe { std::alloc::dealloc(header, layout) };
}

/// Return entry count of a `HashMap`. Trap-on-0.
#[allow(unsafe_code)]
#[allow(clippy::cast_ptr_alignment)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_hashmap_len(ptr: i64) -> i64 {
    if ptr == 0 {
        std::process::abort();
    }
    unsafe { (ptr as *const i64).read_unaligned() }
}

/// Functional insert: consume `map`, return new map ptr.
///
/// ADR-0078 ABI: v == by-value i64 when stride <= 8; v == by-pointer when
/// stride > 8 (fat value, memcpy stride bytes). ADR-0080 Mũi B: k follows
/// the SAME by-value/by-pointer convention keyed on `key_stride` (JIT routes
/// both). ADR-0080 §AMEND-1 D.2: `is_update_out` — scratch i64 the shim
/// writes 1 into IFF this hit the UPDATE branch (dup-content key). The shim
/// does NOT free the caller's now-redundant incoming key itself (a Rust-
/// internal free call would be invisible to `shim_registry` counting-harness
/// substitution — ADR §AMEND-1 finding); the JIT call-site frees it via a
/// registry-routed call, gated on this flag, so it stays counting-testable.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment,
    clippy::ptr_as_ptr,
    clippy::similar_names
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_hashmap_insert(map: i64, k: i64, v: i64, is_update_out: i64) -> i64 {
    // D2 defense-in-depth: reject MIN value (ADR-0044 Q4).
    if v == triet_mir::NULL_SENTINEL {
        std::process::abort();
    }
    if map == 0 {
        std::process::abort();
    }
    let body = map as *mut u8;
    let key_stride = hashmap_key_stride(map);
    let value_stride = hashmap_value_stride(map);
    let len = unsafe { (body as *const i64).read_unaligned() } as usize;
    let cap = unsafe { (body as *const i64).add(1).read_unaligned() } as usize;

    let new_cap = if len * 4 >= cap * 3 {
        (cap * 2).max(4)
    } else {
        0
    };

    let (body_ptr, cap_used) = if new_cap > 0 {
        let new_map = __triet_hashmap_alloc(
            0,
            new_cap as i64,
            usize_to_i64(key_stride),
            usize_to_i64(value_stride),
        );
        if new_map == 0 {
            return 0;
        }
        let new_body = new_map as *mut u8;
        for i in 0..cap {
            let state = unsafe { *hashmap_state_ptr(body, i) };
            if state == 1u8 {
                let old_key_ptr = unsafe { hashmap_key_ptr(body, i) };
                let old_v_ptr = unsafe { hashmap_value_ptr(body, i) };
                // ADR-0080 bất biến rehash: hash from the STORED key content
                // (String: FNV-1a on {ptr,len} read from the slot itself —
                // `old_key_ptr` IS a fat-shaped cell, same convention as a
                // pointer-to-fat `k` arg; Integer: raw i64 read from the cell).
                let k_shape = if key_stride > 8 {
                    old_key_ptr as i64
                } else {
                    unsafe { (old_key_ptr as *const i64).read_unaligned() }
                };
                let mut probe = hashmap_key_hash(key_stride, k_shape, new_cap);
                loop {
                    let st = unsafe { *hashmap_state_ptr(new_body, probe) };
                    if st == 0u8 {
                        unsafe {
                            // Move key bytes by `key_stride` (NOT an i64-only
                            // read/write — ADR-0080 tooth #7: a fat 24B key
                            // truncated to 8B corrupts len/cap on the far side).
                            std::ptr::copy_nonoverlapping(
                                old_key_ptr,
                                hashmap_key_ptr(new_body, probe),
                                key_stride,
                            );
                            // Copy value cell: memcpy stride bytes from old
                            // to new (works for both scalar 8B and fat 24B).
                            std::ptr::copy_nonoverlapping(
                                old_v_ptr,
                                hashmap_value_ptr(new_body, probe),
                                value_stride,
                            );
                            *hashmap_state_ptr(new_body, probe) = 1u8;
                        }
                        break;
                    }
                    probe = (probe + 1) % new_cap;
                }
            }
        }
        unsafe { (new_body as *mut i64).write_unaligned(len as i64) };
        __triet_hashmap_free(map); // buffer only — keys/values already moved.
        (new_body, new_cap)
    } else {
        (body, cap)
    };

    // Insert or update via linear probing
    let mut probe = hashmap_key_hash(key_stride, k, cap_used);
    let mut is_update = false;
    loop {
        let state = unsafe { *hashmap_state_ptr(body_ptr, probe) };
        if state == 1u8 {
            let slot_key_ptr = unsafe { hashmap_key_ptr(body_ptr, probe) };
            if unsafe { hashmap_key_eq(slot_key_ptr, key_stride, k) } {
                unsafe {
                    hashmap_write_value(hashmap_value_ptr(body_ptr, probe), value_stride, v);
                }
                is_update = true;
                break;
            }
        } else if state == 0u8 {
            unsafe {
                hashmap_write_key(hashmap_key_ptr(body_ptr, probe), key_stride, k);
                hashmap_write_value(hashmap_value_ptr(body_ptr, probe), value_stride, v);
                *hashmap_state_ptr(body_ptr, probe) = 1u8;
            }
            break;
        }
        probe = (probe + 1) % cap_used;
    }
    if !is_update {
        let new_len = (len + 1) as i64;
        unsafe { (body_ptr as *mut i64).write_unaligned(new_len) };
    }
    if is_update_out != 0 {
        unsafe {
            (is_update_out as *mut i64).write_unaligned(i64::from(is_update));
        }
    }
    body_ptr as i64
}

/// Look up key, return value or `NULL_SENTINEL`. Trap-on-0 for map handle.
/// ADR-0080 Mũi B: `k` by-value (Integer) or by-pointer-to-fat (String), per
/// `key_stride` — JIT routes.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_hashmap_get(map: i64, k: i64) -> i64 {
    if map == 0 {
        std::process::abort();
    }
    let body = map as *mut u8;
    let key_stride = hashmap_key_stride(map);
    let cap = unsafe { (body as *const i64).add(1).read_unaligned() } as usize;
    let mut probe = hashmap_key_hash(key_stride, k, cap);
    loop {
        let state = unsafe { *hashmap_state_ptr(body, probe) };
        if state == 0u8 {
            return triet_mir::NULL_SENTINEL;
        }
        if state == 1u8 {
            let slot_key_ptr = unsafe { hashmap_key_ptr(body, probe) };
            if unsafe { hashmap_key_eq(slot_key_ptr, key_stride, k) } {
                let vptr = unsafe { hashmap_value_ptr(body, probe) };
                return unsafe { (vptr as *const i64).read_unaligned() };
            }
        }
        probe = (probe + 1) % cap;
    }
}

/// ADR-0079 Slice B: `get_ref(&0 map, k)` — zero-copy borrow of the value slot.
///
/// Returns a pointer to the value cell within the `HashMap` buffer (NOT a copy
/// of the value). Key not found → `NULL_SENTINEL`. The returned pointer is
/// valid as long as the map is not mutated (enforced by borrowck U2/U3).
/// ZERO-COPY: no allocation, no memcpy — the caller reads directly from the
/// buffer slot through the returned reference. ADR-0080 Mũi B: `k` dispatch
/// same as `get`.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment,
    clippy::ptr_as_ptr
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_hashmap_get_ref(map: i64, k: i64) -> i64 {
    if map == 0 {
        std::process::abort();
    }
    let body = map as *mut u8;
    let key_stride = hashmap_key_stride(map);
    let cap = unsafe { (body as *const i64).add(1).read_unaligned() } as usize;
    let mut probe = hashmap_key_hash(key_stride, k, cap);
    loop {
        let state = unsafe { *hashmap_state_ptr(body, probe) };
        if state == 0u8 {
            return triet_mir::NULL_SENTINEL;
        }
        if state == 1u8 {
            let slot_key_ptr = unsafe { hashmap_key_ptr(body, probe) };
            if unsafe { hashmap_key_eq(slot_key_ptr, key_stride, k) } {
                // Zero-copy: return the address of the value cell.
                return unsafe { hashmap_value_ptr(body, probe) as i64 };
            }
        }
        probe = (probe + 1) % cap;
    }
}

/// P1.5: `remove(map, key) -> V?` — take-out the value by key (ADR-0078 MŨI D).
///
/// Move-out: the value cell is tombstoned (state→deleted for fat: state=2;
/// for scalar/handle: key+value memset to avoid stale read on future probe),
/// and the value is returned to the caller (by-value i64 for stride<=8;
/// memcpy to `out_ptr` for stride>8). Empty/not-found → `NULL_SENTINEL`.
///
/// ADR-0080 §AMEND-1 D.5: `key_out_ptr` — scratch 24B the shim writes the
/// RESIDENT key's fat `{ptr,len,cap}` into when `key_stride > 8` (String),
/// then tombstone-zeroes the slot's key cell (prevents a stale read on a
/// future probe reusing this slot after the surfaced bytes are freed). Not-
/// found → `NULL_SENTINEL` written to `key_out_ptr` (sentinel-no-op R4, same
/// as the value's `out_ptr` convention). The shim does NOT free the resident
/// key itself (a Rust-internal free call would be invisible to `shim_registry`
/// counting-harness substitution); the JIT call-site frees `key_out_ptr`'s
/// content unconditionally post-call via a registry-routed call — the
/// sentinel-no-op on not-found makes that unconditional call safe.
/// CẤM free `k` (the lookup key) here — it belongs to the CALLER (borrow,
/// ADR-0080 Mũi D point 4); only the RESIDENT surface (`key_out_ptr`) frees.
#[allow(unsafe_code)]
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment,
    clippy::ptr_as_ptr
)]
#[unsafe(no_mangle)]
pub extern "C" fn __triet_hashmap_remove(map: i64, k: i64, out_ptr: i64, key_out_ptr: i64) -> i64 {
    if map == 0 {
        std::process::abort();
    }
    let body = map as *mut u8;
    let key_stride = hashmap_key_stride(map);
    let value_stride = hashmap_value_stride(map);
    let cap = unsafe { (body as *const i64).add(1).read_unaligned() } as usize;
    let mut probe = hashmap_key_hash(key_stride, k, cap);
    loop {
        let state = unsafe { *hashmap_state_ptr(body, probe) };
        if state == 0u8 {
            // Not found — write sentinel to out_ptr for fat values
            if value_stride > 8 {
                unsafe { (out_ptr as *mut i64).write_unaligned(triet_mir::NULL_SENTINEL) };
            }
            if key_stride > 8 {
                unsafe { (key_out_ptr as *mut i64).write_unaligned(triet_mir::NULL_SENTINEL) };
            }
            return triet_mir::NULL_SENTINEL;
        }
        if state == 1u8 {
            let slot_key_ptr = unsafe { hashmap_key_ptr(body, probe) };
            if unsafe { hashmap_key_eq(slot_key_ptr, key_stride, k) } {
                let vptr = unsafe { hashmap_value_ptr(body, probe) };
                // D.5: surface the resident key to the caller's key_out_ptr,
                // then tombstone-zero the cell (JIT frees key_out_ptr's
                // content post-call, registry-routed).
                if key_stride > 8 {
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            slot_key_ptr,
                            key_out_ptr as *mut u8,
                            key_stride,
                        );
                        std::ptr::write_bytes(slot_key_ptr, 0, key_stride);
                    }
                }
                // Tombstone: state → 2 (deleted)
                unsafe { *hashmap_state_ptr(body, probe) = 2u8 };
                // Decrement len
                let len = unsafe { (body as *const i64).read_unaligned() };
                unsafe { (body as *mut i64).write_unaligned(len - 1) };
                if value_stride <= 8 {
                    return unsafe { (vptr as *const i64).read_unaligned() };
                }
                // Fat: memcpy to out_ptr
                unsafe { std::ptr::copy_nonoverlapping(vptr, out_ptr as *mut u8, value_stride) };
                return 0;
            }
        }
        probe = (probe + 1) % cap;
    }
}

/// `__triet_hashmap_contains(map, key)` — key lookup.
/// Returns 1 (true) if `key` exists in the map, -1 (false) otherwise.
/// Never returns 0. ADR-0080 Mũi B: `k` dispatch same as `get`.
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
    let key_stride = hashmap_key_stride(map);
    let cap = unsafe { (body as *const i64).add(1).read_unaligned() } as usize;
    let mut probe = hashmap_key_hash(key_stride, k, cap);
    loop {
        let state = unsafe { *hashmap_state_ptr(body, probe) };
        if state == 0u8 {
            return -1; // EMPTY — key not found
        }
        if state == 1u8 {
            let slot_key_ptr = unsafe { hashmap_key_ptr(body, probe) };
            if unsafe { hashmap_key_eq(slot_key_ptr, key_stride, k) } {
                return 1; // FOUND
            }
        }
        probe = (probe + 1) % cap;
    }
}

// ── Cast helpers (E1b-2) ─────────────────────────────────────
// Centralized i64↔machine casts for the JIT value-model.
// Each wraps a clippy::cast_* allow with a debug_assert that documents
// and enforces the invariant.  debug_assert fires in debug/test builds
// → JIT-emit bugs surface AT the cast, not as silent truncation downstream.
// These invariants are COMPILER-INTERNAL (offset/size/len from layout, ≥0
// by construction) — never user runtime values.

/// Cast i64→usize.  Value must be ≥ 0 (layout metadata or heap length).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn i64_to_usize(v: i64) -> usize {
    debug_assert!(v >= 0, "JIT cast i64→usize: negative {v}");
    v as usize
}

/// Cast usize→i64.  Value must fit (layout metadata or heap capacity).
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::checked_conversions
)]
fn usize_to_i64(v: usize) -> i64 {
    // `i64::MAX as usize`: on 64-bit this is a no-op; on 32-bit it truncates
    // to u32::MAX — still a safe upper bound for the assert (any v ≤ u32::MAX
    // that fits in the assert also fits in i64).
    debug_assert!(v <= i64::MAX as usize, "JIT cast usize→i64: overflow {v}");
    v as i64
}

/// Cast usize→u32.  Value must fit (Variable index, stack size metadata).
#[allow(clippy::cast_possible_truncation, clippy::checked_conversions)]
fn usize_to_u32(v: usize) -> u32 {
    debug_assert!(v <= u32::MAX as usize, "JIT cast usize→u32: overflow {v}");
    v as u32
}

/// Cast usize→i32.  Value must fit (non-negative layout metadata).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::checked_conversions
)]
fn usize_to_i32(v: usize) -> i32 {
    debug_assert!(v <= i32::MAX as usize, "JIT cast usize→i32: overflow {v}");
    v as i32
}

/// Cast u32→u8.  Value must fit (\``ilog2()`\` ≤ 63 — always true in practice).
#[allow(clippy::cast_possible_truncation, clippy::checked_conversions)]
fn u32_to_u8(v: u32) -> u8 {
    debug_assert!(v <= u32::from(u8::MAX), "JIT cast u32→u8: overflow {v}");
    v as u8
}

/// Cast i64→u8 by truncation (low byte).  This is INTENTIONAL truncation
/// — the caller (\`__`triet_string_append`\`) receives a user-supplied \`byte\`
/// parameter whose range is defined by language semantics (SPEC), not by
/// the JIT.  No `debug_assert` — the range contract belongs to the language
/// spec, and E1b does NOT silently encode one.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
const fn i64_low_byte(v: i64) -> u8 {
    (v & 0xFF) as u8
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use triet_borrowck::{MirBuilder, binop, const_int, return_, storage_live};
    use triet_mir::{
        CallTarget, ConstValue, DUMMY_SPAN, FunctionId, MirType, ParameterPassing, Place,
        Projection, ReturnShape, Statement, Terminator,
    };

    // ── E1b-2 helper unit tests ────────────────────────────

    #[test]
    #[should_panic(expected = "JIT cast i64→usize: negative -1")]
    fn i64_to_usize_rejects_negative() {
        i64_to_usize(-1); // poison: negative → panic in debug
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)] // i64::MAX as usize — in-range test
    fn i64_to_usize_happy() {
        assert_eq!(i64_to_usize(0), 0_usize);
        assert_eq!(i64_to_usize(42), 42_usize);
        assert_eq!(i64_to_usize(i64::MAX), i64::MAX as usize);
    }

    #[test]
    #[should_panic(expected = "JIT cast usize→i64: overflow")]
    fn usize_to_i64_rejects_overflow() {
        usize_to_i64(usize::MAX); // poison: never fits → panic in debug
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)] // i64::MAX as usize — in-range test
    fn usize_to_i64_happy() {
        assert_eq!(usize_to_i64(0), 0_i64);
        assert_eq!(usize_to_i64(42), 42_i64);
        assert_eq!(usize_to_i64(i64::MAX as usize), i64::MAX);
    }

    #[test]
    #[should_panic(expected = "JIT cast usize→u32: overflow")]
    fn usize_to_u32_rejects_overflow() {
        usize_to_u32(1 << 33); // poison: 8 GiB → panic in debug
    }

    #[test]
    fn usize_to_u32_happy() {
        assert_eq!(usize_to_u32(0), 0_u32);
        assert_eq!(usize_to_u32(42), 42_u32);
        assert_eq!(usize_to_u32(u32::MAX as usize), u32::MAX);
    }

    #[test]
    #[should_panic(expected = "JIT cast usize→i32: overflow")]
    fn usize_to_i32_rejects_overflow() {
        usize_to_i32(1 << 33); // poison: 8 GiB → panic in debug
    }

    #[test]
    fn usize_to_i32_happy() {
        assert_eq!(usize_to_i32(0), 0_i32);
        assert_eq!(usize_to_i32(42), 42_i32);
        assert_eq!(usize_to_i32(i32::MAX as usize), i32::MAX);
    }

    #[test]
    #[should_panic(expected = "JIT cast u32→u8: overflow")]
    fn u32_to_u8_rejects_overflow() {
        u32_to_u8(256); // poison: outside u8 range → panic in debug
    }

    #[test]
    fn u32_to_u8_happy() {
        assert_eq!(u32_to_u8(0), 0_u8);
        assert_eq!(u32_to_u8(42), 42_u8);
        assert_eq!(u32_to_u8(255), 255_u8);
    }

    #[test]
    fn i64_low_byte_truncates() {
        // low-byte extraction, no panic
        assert_eq!(i64_low_byte(0x00), 0x00_u8);
        assert_eq!(i64_low_byte(0xFF), 0xFF_u8);
        assert_eq!(i64_low_byte(0x1FF), 0xFF_u8); // truncation is intentional
        assert_eq!(i64_low_byte(-1), 0xFF_u8);
    }

    // ── Pipeline tests ────────────────────────────

    /// Compile and run `abs_diff`: `abs_diff(10, 3) == 7`.
    #[test]
    #[allow(unsafe_code)]
    fn abs_diff_jit_compile_and_run() {
        let mut b = MirBuilder::new("abs_diff_jit_test", MirType::Integer);
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
        let mut b = MirBuilder::new("add_jit_test", MirType::Integer);
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
    #[allow(clippy::too_many_lines)] // JIT integration test — refactor not in E1b scope
    fn fibonacci_jit_compile_and_run() {
        let mut b = MirBuilder::new("fibonacci", MirType::Integer);
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

    /// Build and compile an Outcome function via the real `StackSlot` path,
    /// returning `(disc, payload)` through `Repr2`.
    #[allow(unsafe_code, clippy::cast_possible_truncation)]
    // `unsafe_code`: transmute code_ptr → extern "C" fn pointer.
    // `cast_possible_truncation`: disc_val as i8 is always in valid Trit range.
    unsafe fn compile_outcome_via_slot(disc_val: i64, payload_val: i64) -> (i64, i64) {
        #[repr(C)]
        struct Repr2(i64, i64);

        let outcome_ty = MirType::Outcome {
            value_type: Box::new(MirType::Integer),
            error_type: Box::new(MirType::Integer),
            allow_null_state: false,
        };
        let mut b = MirBuilder::new("outcome_test", outcome_ty);
        b.set_return_shape(triet_mir::ReturnShape::BinaryOutcome);

        let outcome = b.new_local();
        b.set_local_mir_type(
            outcome,
            MirType::Outcome {
                value_type: Box::new(MirType::Integer),
                error_type: Box::new(MirType::Integer),
                allow_null_state: false,
            },
        );
        let disc_tmp = b.new_local();
        let payload_tmp = b.new_local();
        let ret_disc = b.new_local();
        let ret_payload = b.new_local();
        let bb0 = b.new_block();

        // Allocate 16-byte Outcome slot.
        b.push(bb0, storage_live(outcome));
        b.push(
            bb0,
            Statement::OutcomeAlloc {
                dest: outcome,
                span: DUMMY_SPAN,
            },
        );

        // Store disc via OutcomeDiscriminant projection (offset 0).
        b.push(bb0, storage_live(disc_tmp));
        b.push(
            bb0,
            Statement::Const {
                dest: disc_tmp.into(),
                value: ConstValue::Trit(disc_val as i8),
                span: DUMMY_SPAN,
            },
        );
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(outcome).project(Projection::OutcomeDiscriminant),
                source: Place::local(disc_tmp),
                span: DUMMY_SPAN,
            },
        );

        // Store payload via OutcomePayload projection (offset 8).
        b.push(bb0, storage_live(payload_tmp));
        b.push(
            bb0,
            Statement::Const {
                dest: payload_tmp.into(),
                value: ConstValue::Integer(i128::from(payload_val)),
                span: DUMMY_SPAN,
            },
        );
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(outcome).project(Projection::OutcomePayload),
                source: Place::local(payload_tmp),
                span: DUMMY_SPAN,
            },
        );

        // Load disc from slot via projection.
        b.push(bb0, storage_live(ret_disc));
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(ret_disc),
                source: Place::local(outcome).project(Projection::OutcomeDiscriminant),
                span: DUMMY_SPAN,
            },
        );

        // Load payload from slot via projection.
        b.push(bb0, storage_live(ret_payload));
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(ret_payload),
                source: Place::local(outcome).project(Projection::OutcomePayload),
                span: DUMMY_SPAN,
            },
        );

        b.set_terminator(
            bb0,
            Terminator::Return {
                values: vec![ret_disc, ret_payload],
                span: DUMMY_SPAN,
            },
        );
        let body = b.build(bb0);

        let func = {
            let mut ctx = JitContext::new();
            ctx.compile(&body).expect("OK")
        };
        let f: extern "C" fn() -> Repr2 = unsafe { std::mem::transmute(func.code_ptr) };
        let r = f();
        (r.0, r.1)
    }

    /// `ADR-0052` `OP.3` + `OP.3.5`: `BinaryOutcome` `StackSlot` 16-byte end-to-end.
    ///
    /// Routes through the `REAL` path: `OutcomeAlloc` → store disc via
    /// `OutcomeDiscriminant` projection → store payload via `OutcomePayload`
    /// projection → load both back via projections → `Return[disc,payload]`.
    /// This verifies the offset machinery (`disc@0`, `payload@8`) — not just
    /// the 2-register ABI.
    #[test]
    #[allow(unsafe_code)]
    fn binary_outcome_2return() {
        // ~+ 42 → disc=1, payload=42
        let (disc, payload) = unsafe { compile_outcome_via_slot(1, 42) };
        assert_eq!(disc, 1, "discriminant should be Positive(1)");
        assert_eq!(payload, 42, "payload should be 42");

        // ~- -1 → disc=-1, payload=-1
        let (disc, payload) = unsafe { compile_outcome_via_slot(-1, -1) };
        assert_eq!(disc, -1, "discriminant should be Negative(-1)");
        assert_eq!(payload, -1, "payload should be -1");
    }

    /// ADR-0052 §3.5: generic multi-value (non-BinaryOutcome) must STILL be
    /// rejected. Only `BinaryOutcome` is un-deferred; tuple/struct multi-return
    /// requires Bậc C packed ABI.
    ///
    /// **If this test fails**, the guard at 1068 was weakened to allow ANY
    /// shape with `values.len()>1` — a soundness regression per ADR-0052 §3.5.
    #[test]
    fn generic_multi_value_refuses_to_compile() {
        // Build a callee with Scalar return shape but 2 return values.
        // This should be REJECTED — generic multi-value is NOT un-deferred.
        let mut callee = MirBuilder::new("generic_multi", MirType::Integer);
        callee.set_return_shape(triet_mir::ReturnShape::Scalar);
        let _dummy = callee.add_param("dummy", ParameterPassing::Borrow);
        let v0 = callee.new_local();
        let v1 = callee.new_local();

        let bb0 = callee.new_block();
        callee.push(bb0, storage_live(v0));
        callee.push(
            bb0,
            triet_mir::Statement::Const {
                dest: v0.into(),
                value: ConstValue::Integer(1),
                span: DUMMY_SPAN,
            },
        );
        callee.push(bb0, storage_live(v1));
        callee.push(
            bb0,
            triet_mir::Statement::Const {
                dest: v1.into(),
                value: ConstValue::Integer(2),
                span: DUMMY_SPAN,
            },
        );
        callee.set_terminator(
            bb0,
            Terminator::Return {
                values: vec![v0, v1], // 2 values with Scalar shape → must ERR
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
                    "JIT compiled generic 2-value return as single i64 — \
                     this is a miscompile. The multi-return guard was \
                     weakened to allow non-BinaryOutcome shapes. \
                     Only BinaryOutcome should be un-deferred (ADR-0052 §3.5)."
                );
            }
            Err(other) => {
                panic!(
                    "unexpected JIT error (expected Unsupported, got {other}) — \
                     verify the guard still refuses non-BinaryOutcome multi-return"
                );
            }
        }
    }

    /// Build a callee that returns `(disc, payload)` via `BinaryOutcome`.
    #[allow(clippy::cast_possible_truncation)]
    fn build_outcome_callee(disc_val: i64, payload_val: i64) -> Body {
        let outcome_ty = MirType::Outcome {
            value_type: Box::new(MirType::Integer),
            error_type: Box::new(MirType::Integer),
            allow_null_state: false,
        };
        let mut b = MirBuilder::new("make_outcome", outcome_ty);
        b.set_return_shape(triet_mir::ReturnShape::BinaryOutcome);
        let slot = b.new_local();
        b.set_local_mir_type(
            slot,
            MirType::Outcome {
                value_type: Box::new(MirType::Integer),
                error_type: Box::new(MirType::Integer),
                allow_null_state: false,
            },
        );
        let disc_tmp = b.new_local();
        let payl_tmp = b.new_local();
        let ld_disc = b.new_local();
        let ld_payl = b.new_local();
        let bb0 = b.new_block();
        b.push(bb0, storage_live(slot));
        b.push(
            bb0,
            Statement::OutcomeAlloc {
                dest: slot,
                span: DUMMY_SPAN,
            },
        );
        b.push(bb0, storage_live(disc_tmp));
        b.push(
            bb0,
            Statement::Const {
                dest: disc_tmp.into(),
                value: ConstValue::Trit(disc_val as i8),
                span: DUMMY_SPAN,
            },
        );
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(slot).project(Projection::OutcomeDiscriminant),
                source: Place::local(disc_tmp),
                span: DUMMY_SPAN,
            },
        );
        b.push(bb0, storage_live(payl_tmp));
        b.push(
            bb0,
            Statement::Const {
                dest: payl_tmp.into(),
                value: ConstValue::Integer(i128::from(payload_val)),
                span: DUMMY_SPAN,
            },
        );
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(slot).project(Projection::OutcomePayload),
                source: Place::local(payl_tmp),
                span: DUMMY_SPAN,
            },
        );
        b.push(bb0, storage_live(ld_disc));
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(ld_disc),
                source: Place::local(slot).project(Projection::OutcomeDiscriminant),
                span: DUMMY_SPAN,
            },
        );
        b.push(bb0, storage_live(ld_payl));
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(ld_payl),
                source: Place::local(slot).project(Projection::OutcomePayload),
                span: DUMMY_SPAN,
            },
        );
        b.set_terminator(
            bb0,
            Terminator::Return {
                values: vec![ld_disc, ld_payl],
                span: DUMMY_SPAN,
            },
        );
        b.build(bb0)
    }

    /// Build a caller that calls `callee_name`, receives `BinaryOutcome` into
    /// a slot, loads from the slot, and returns the 2 values.
    fn build_outcome_caller(callee_name: &str) -> Body {
        let outcome_ty = MirType::Outcome {
            value_type: Box::new(MirType::Integer),
            error_type: Box::new(MirType::Integer),
            allow_null_state: false,
        };
        let mut b = MirBuilder::new("call", outcome_ty);
        b.set_return_shape(triet_mir::ReturnShape::BinaryOutcome);
        let dest_slot = b.new_local();
        b.set_local_mir_type(
            dest_slot,
            MirType::Outcome {
                value_type: Box::new(MirType::Integer),
                error_type: Box::new(MirType::Integer),
                allow_null_state: false,
            },
        );
        let disc_out = b.new_local();
        let payl_out = b.new_local();
        let bb0 = b.new_block();
        let ret_bb = b.new_block();
        b.push(bb0, storage_live(dest_slot));
        b.push(
            bb0,
            Statement::OutcomeAlloc {
                dest: dest_slot,
                span: DUMMY_SPAN,
            },
        );
        b.set_terminator(
            bb0,
            Terminator::CallDispatch {
                callee: FunctionId(0),
                callee_name: callee_name.into(),
                target: CallTarget::Jit,
                args: vec![],
                return_bb: ret_bb,
                dest: vec![dest_slot],
                return_shape: triet_mir::ReturnShape::BinaryOutcome,
                span: DUMMY_SPAN,
            },
        );
        b.push(ret_bb, storage_live(disc_out));
        b.push(
            ret_bb,
            Statement::Assign {
                dest: Place::local(disc_out),
                source: Place::local(dest_slot).project(Projection::OutcomeDiscriminant),
                span: DUMMY_SPAN,
            },
        );
        b.push(ret_bb, storage_live(payl_out));
        b.push(
            ret_bb,
            Statement::Assign {
                dest: Place::local(payl_out),
                source: Place::local(dest_slot).project(Projection::OutcomePayload),
                span: DUMMY_SPAN,
            },
        );
        b.set_terminator(
            ret_bb,
            Terminator::Return {
                values: vec![disc_out, payl_out],
                span: DUMMY_SPAN,
            },
        );
        b.build(bb0)
    }

    /// ADR-0052 `OP.4a`: Outcome caller round-trip.
    ///
    /// Callee returns `BinaryOutcome`, caller calls it, stores
    /// `inst_results` into dest Outcome slot, loads back via projections,
    /// returns 2 values to Rust. Verifies the full call ABI.
    #[test]
    #[allow(unsafe_code)]
    fn outcome_call_roundtrip() {
        #[repr(C)]
        struct Repr2(i64, i64);

        unsafe fn compile_roundtrip(disc_val: i64, payload_val: i64) -> Repr2 {
            let callee_fn = build_outcome_callee(disc_val, payload_val);
            let call_site = build_outcome_caller("make_outcome");
            let funcs = {
                let mut ctx = JitContext::new();
                ctx.compile_multi(&[&callee_fn, &call_site]).expect("OP.4a")
            };
            let f: extern "C" fn() -> Repr2 =
                unsafe { std::mem::transmute(funcs["call"].code_ptr) };
            f()
        }

        let r = unsafe { compile_roundtrip(1, 42) };
        assert_eq!(r.0, 1, "discriminant should be Positive(1)");
        assert_eq!(r.1, 42, "payload should be 42");

        let r = unsafe { compile_roundtrip(-1, -1) };
        assert_eq!(r.0, -1, "discriminant should be Negative(-1)");
        assert_eq!(r.1, -1, "payload should be -1");
    }

    // ── Logic op truth table tests ─────────────────────────

    /// Trilean encoding: +1=True, 0=Unknown, -1=False.
    const T: i64 = 1;
    const U: i64 = 0;
    const F: i64 = -1;
    const ALL: [i64; 3] = [T, U, F];

    /// Build a MIR function `op(a, b)` that applies `binop` and returns the result.
    fn build_binop_tester(op: BinOp) -> Body {
        let mut b = MirBuilder::new(&format!("test_{op:?}"), MirType::Integer);
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
            (-1, _) | (0, 0 | 1) => 1,
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
        let mut b = MirBuilder::new("test_neg", MirType::Integer);
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
        let mut b = MirBuilder::new("test_shim_mul", MirType::Integer);
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
        let mut b = MirBuilder::new("test_pow", MirType::Integer);
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

    /// HP.2: counting-only free (no real dealloc). Used with fake ptr/cap
    /// to test drop glue without needing real heap allocations.
    static HP2_FREE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    extern "C" fn __hp2_count_free(ptr: i64, cap: i64) {
        let _ = (ptr, cap);
        HP2_FREE_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// HP.3a: per-test counter for deinit→drop test.
    static HP3A_FREE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    extern "C" fn __hp3a_count_free(ptr: i64, cap: i64) {
        let _ = (ptr, cap);
        HP3A_FREE_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// HP.3b: per-test counter for no-deinit double-free test.
    static HP3B_FREE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    extern "C" fn __hp3b_count_free(ptr: i64, cap: i64) {
        let _ = (ptr, cap);
        HP3B_FREE_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// HP.4: per-test counter for the heap map (`~+>`) free-balance test.
    /// Mirrors the real `__triet_string_free` ptr==0 guard so it counts only
    /// frees of LIVE allocations — a tombstoned (Deinit'd, ptr=0) value's Drop
    /// still *calls* free, but frees nothing, exactly as in production.
    static HP4_FREE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    extern "C" fn __hp4_count_free(ptr: i64, cap: i64) {
        let _ = cap;
        if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
            return;
        }
        HP4_FREE_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// ADR-0080 KM-P1a: real-allocator counting free for the String-key
    /// `HashMap` teeth (map-drop / update-leak / remove-leak). Counts AND
    /// really frees (mirrors `__test_counting_free`) — real pointers, so a
    /// double-free would SIGABRT (G gold standard), not just miscount.
    /// ONE DEDICATED counter+shim per test (not shared) — `cargo test` runs
    /// this file's tests in parallel by default; a shared counter races
    /// (matches the established per-test-counter convention used by
    /// `HP2_FREE_COUNT`/`HP3A_FREE_COUNT`/etc. above, not a `Mutex` lock).
    static KM_P1A_MAP_DROP_FREE_COUNT: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    extern "C" fn __km_p1a_map_drop_count_free(ptr: i64, cap: i64) {
        KM_P1A_MAP_DROP_FREE_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        super::__triet_string_free(ptr, cap);
    }

    static KM_P1A_UPDATE_FREE_COUNT: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    extern "C" fn __km_p1a_update_count_free(ptr: i64, cap: i64) {
        KM_P1A_UPDATE_FREE_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        super::__triet_string_free(ptr, cap);
    }

    static KM_P1A_REMOVE_FREE_COUNT: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    extern "C" fn __km_p1a_remove_count_free(ptr: i64, cap: i64) {
        KM_P1A_REMOVE_FREE_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
        let mut b = MirBuilder::new("make_string", MirType::String);
        // ADR-0049: String layout required for JIT StackSlot pre-pass.
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "String",
            &[
                ("ptr".to_string(), MirType::Integer, 8, 8),
                ("len".to_string(), MirType::Integer, 8, 8),
                ("cap".to_string(), MirType::Integer, 8, 8),
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

    /// HP.2: heap Outcome drop glue — String success payload freed exactly once.
    /// Poison: swap free-as-T ↔ free-as-E → wrong-arm SIGABRT.
    /// Poison: Zero no-op removed → tombstone double-free → `FREE_COUNT` 2.
    #[test]
    #[allow(unsafe_code)]
    fn hp2_outcome_drop_glue_frees_exactly_once() {
        use std::sync::atomic::Ordering;

        HP2_FREE_COUNT.store(0, Ordering::SeqCst);

        // Build a minimal body: Outcome<String,Integer> with disc=1,
        // non-zero ptr+cap stored directly (no real String allocation).
        // Drop fires inline SwitchInt → free-as-T → counting shim.
        let mut b = MirBuilder::new("hp2_drop_pos", MirType::Unit);
        let outcome_ty = MirType::Outcome {
            value_type: Box::new(MirType::String),
            error_type: Box::new(MirType::Integer),
            allow_null_state: false,
        };

        let bb0 = b.new_block();

        let o = b.new_local();
        b.set_local_mir_type(o, outcome_ty);
        b.push(bb0, storage_live(o));
        b.push(
            bb0,
            Statement::OutcomeAlloc {
                dest: o,
                span: DUMMY_SPAN,
            },
        );

        // disc = 1
        let disc_tmp = b.new_local();
        b.push(bb0, storage_live(disc_tmp));
        b.push(
            bb0,
            Statement::Const {
                dest: Place::local(disc_tmp),
                value: triet_mir::ConstValue::Trit(1),
                span: DUMMY_SPAN,
            },
        );
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(o).project(Projection::OutcomeDiscriminant),
                source: Place::local(disc_tmp),
                span: DUMMY_SPAN,
            },
        );

        // Dummy ptr=1, len=0, cap=8 (non-null → shim calls free; counting-only shim won't crash).
        for (val, proj) in [
            (1i128, Projection::OutcomePayload),
            (0i128, Projection::OutcomePayloadLen),
            (8i128, Projection::OutcomePayloadCap),
        ] {
            let tmp = b.new_local();
            b.push(bb0, storage_live(tmp));
            b.push(
                bb0,
                Statement::Const {
                    dest: Place::local(tmp),
                    value: triet_mir::ConstValue::Integer(val),
                    span: DUMMY_SPAN,
                },
            );
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(o).project(proj),
                    source: Place::local(tmp),
                    span: DUMMY_SPAN,
                },
            );
        }

        b.push(bb0, Statement::Drop(o, DUMMY_SPAN));
        b.set_terminator(bb0, return_(vec![]));

        let body = b.build(bb0);
        println!("=== MIR (hp2_drop_pos) ===\n{body}");

        let shims = &[ShimSymbol::fn_2_0("__triet_string_free", __hp2_count_free)];
        let mut ctx = JitContext::with_shims(shims);
        let func = ctx.compile(&body).expect("HP.2 drop glue compile");
        HP2_FREE_COUNT.store(0, Ordering::SeqCst);
        let _ = unsafe { func.call_i64_0() };

        assert_eq!(
            HP2_FREE_COUNT.load(Ordering::SeqCst),
            1,
            "HP.2: Outcome<String,Integer> drop (Positive arm) must free exactly once"
        );
    }

    /// ADR-0066 KCN-1: counting-only free for the FLAT struct drop-glue tests.
    static LAT1A_FREE_COUNT: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    extern "C" fn __lat1a_count_free(ptr: i64, cap: i64) {
        let _ = (ptr, cap);
        LAT1A_FREE_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    static LAT1A_MULTI_FREE_COUNT: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    extern "C" fn __lat1a_multi_count_free(ptr: i64, cap: i64) {
        let _ = (ptr, cap);
        LAT1A_MULTI_FREE_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// ADR-0066 KCN-1: `struct Person { name: String }` construct + Drop frees
    /// the heap field exactly once (inline static drop-glue walks the layout).
    /// Poison R-leak (skip the `emit_heap_free_at` in the struct drop branch) →
    /// `FREE_COUNT == 0`.
    #[test]
    #[allow(unsafe_code)]
    fn lat1a_struct_drop_frees_once() {
        use std::sync::atomic::Ordering;

        LAT1A_FREE_COUNT.store(0, Ordering::SeqCst);

        let mut b = MirBuilder::new("lat1a_person", MirType::Unit);
        // M-1 layout: name@0, String = 24B.
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "Person",
            &[("name".to_string(), MirType::String, 24, 8)],
        ));

        let bb0 = b.new_block();
        let p = b.new_local();
        b.set_local_mir_type(p, MirType::Struct("Person".to_string()));
        b.push(bb0, storage_live(p));
        b.push(
            bb0,
            Statement::StructAlloc {
                dest: p,
                struct_name: "Person".to_string(),
                span: DUMMY_SPAN,
            },
        );

        // Store a non-zero ptr into name@0 (so the drop-glue reads ptr != 0).
        let ptr_tmp = b.new_local();
        b.push(bb0, storage_live(ptr_tmp));
        b.push(
            bb0,
            Statement::Const {
                dest: Place::local(ptr_tmp),
                value: ConstValue::Integer(1),
                span: DUMMY_SPAN,
            },
        );
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(p).project(Projection::Field("name".to_string())),
                source: Place::local(ptr_tmp),
                span: DUMMY_SPAN,
            },
        );

        b.push(bb0, Statement::Drop(p, DUMMY_SPAN));
        b.set_terminator(bb0, return_(vec![]));

        let body = b.build(bb0);
        let shims = &[ShimSymbol::fn_2_0(
            "__triet_string_free",
            __lat1a_count_free,
        )];
        let mut ctx = JitContext::with_shims(shims);
        let func = ctx.compile(&body).expect("lat1a struct drop compile");
        LAT1A_FREE_COUNT.store(0, Ordering::SeqCst);
        let _ = unsafe { func.call_i64_0() };

        assert_eq!(
            LAT1A_FREE_COUNT.load(Ordering::SeqCst),
            1,
            "ADR-0066: struct with one heap field must free it exactly once on Drop"
        );
    }

    /// ADR-0066 KCN-1 / R2: `struct Pair { a: String, b: String }` Drop frees
    /// BOTH heap fields (drop-glue walks every heap field). Poison R2 (walk only
    /// the first field) → `FREE_COUNT == 1 < 2`.
    #[test]
    #[allow(unsafe_code)]
    fn lat1a_multi_heap_field_frees_all() {
        use std::sync::atomic::Ordering;

        LAT1A_MULTI_FREE_COUNT.store(0, Ordering::SeqCst);

        let mut b = MirBuilder::new("lat1a_pair", MirType::Unit);
        // M-1 layout: a@0, b@24, each String = 24B.
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "Pair",
            &[
                ("a".to_string(), MirType::String, 24, 8),
                ("b".to_string(), MirType::String, 24, 8),
            ],
        ));

        let bb0 = b.new_block();
        let p = b.new_local();
        b.set_local_mir_type(p, MirType::Struct("Pair".to_string()));
        b.push(bb0, storage_live(p));
        b.push(
            bb0,
            Statement::StructAlloc {
                dest: p,
                struct_name: "Pair".to_string(),
                span: DUMMY_SPAN,
            },
        );

        // Store distinct non-zero ptrs into a@0 and b@24.
        for (field, ptr_val) in [("a", 1i128), ("b", 2i128)] {
            let tmp = b.new_local();
            b.push(bb0, storage_live(tmp));
            b.push(
                bb0,
                Statement::Const {
                    dest: Place::local(tmp),
                    value: ConstValue::Integer(ptr_val),
                    span: DUMMY_SPAN,
                },
            );
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(p).project(Projection::Field(field.to_string())),
                    source: Place::local(tmp),
                    span: DUMMY_SPAN,
                },
            );
        }

        b.push(bb0, Statement::Drop(p, DUMMY_SPAN));
        b.set_terminator(bb0, return_(vec![]));

        let body = b.build(bb0);
        let shims = &[ShimSymbol::fn_2_0(
            "__triet_string_free",
            __lat1a_multi_count_free,
        )];
        let mut ctx = JitContext::with_shims(shims);
        let func = ctx.compile(&body).expect("lat1a pair drop compile");
        LAT1A_MULTI_FREE_COUNT.store(0, Ordering::SeqCst);
        let _ = unsafe { func.call_i64_0() };

        assert_eq!(
            LAT1A_MULTI_FREE_COUNT.load(Ordering::SeqCst),
            2,
            "ADR-0066: struct with two heap fields must free BOTH on Drop"
        );
    }

    /// ADR-0066 STEP 5 (R-cap): the cap the drop-glue passes to
    /// `__triet_string_free` for a struct String field must be the REAL cap
    /// (here 5 for "Giang"), not uninitialized stack garbage. Records the cap
    /// and asserts == 5. Poison STEP 4 (drop the `cap@dest_off+16` copy) →
    /// drop-glue reads garbage → cap != 5 → RED. This is the ONLY teeth that
    /// catches the latent `__triet_string_free` UB (the counting shims ignore
    /// cap). Records-only (no real dealloc) so a poisoned garbage cap fails the
    /// assert deterministically instead of crashing.
    static RCAP_SEEN: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    extern "C" fn __rcap_record_free(ptr: i64, cap: i64) {
        if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
            return;
        }
        RCAP_SEEN.store(cap, std::sync::atomic::Ordering::SeqCst);
    }

    #[test]
    #[allow(unsafe_code)]
    fn lat1a_struct_field_cap_preserved() {
        use std::sync::atomic::Ordering;

        RCAP_SEEN.store(-1, Ordering::SeqCst);

        let mut b = MirBuilder::new("lat1a_cap", MirType::Unit);
        // String layout (for the source String local's slot pre-allocation).
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "String",
            &[
                ("ptr".to_string(), MirType::Integer, 8, 8),
                ("len".to_string(), MirType::Integer, 8, 8),
                ("cap".to_string(), MirType::Integer, 8, 8),
            ],
        ));
        // Person layout: name@0, String = 24B (M-1).
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "Person",
            &[("name".to_string(), MirType::String, 24, 8)],
        ));

        let bb0 = b.new_block();
        let p = b.new_local();
        b.set_local_mir_type(p, MirType::Struct("Person".to_string()));
        b.push(bb0, storage_live(p));
        b.push(
            bb0,
            Statement::StructAlloc {
                dest: p,
                struct_name: "Person".to_string(),
                span: DUMMY_SPAN,
            },
        );

        // Source String "Giang" → slot {ptr, len=5, cap=5}.
        let s = b.new_local();
        b.set_local_mir_type(s, MirType::String);
        b.push(bb0, storage_live(s));
        b.push(
            bb0,
            Statement::Const {
                dest: Place::local(s),
                value: ConstValue::String("Giang".into()),
                span: DUMMY_SPAN,
            },
        );

        // Construct: p.name = move s (STEP 4 copies len/cap into the field slot).
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(p).project(Projection::Field("name".to_string())),
                source: Place::local(s),
                span: DUMMY_SPAN,
            },
        );

        b.push(bb0, Statement::Drop(p, DUMMY_SPAN));
        b.set_terminator(bb0, return_(vec![]));

        let body = b.build(bb0);
        let shims = &[
            ShimSymbol::fn_2_1(
                "__triet_string_from_bytes",
                super::__triet_string_from_bytes,
            ),
            ShimSymbol::fn_2_0("__triet_string_free", __rcap_record_free),
        ];
        let mut ctx = JitContext::with_shims(shims);
        let func = ctx.compile(&body).expect("lat1a cap-preserve compile");
        RCAP_SEEN.store(-1, Ordering::SeqCst);
        let _ = unsafe { func.call_i64_0() };

        assert_eq!(
            RCAP_SEEN.load(Ordering::SeqCst),
            5,
            "ADR-0066 STEP 4: struct String field drop must free with the REAL \
             cap (5 for \"Giang\"), not stack garbage"
        );
    }

    /// ADR-0067 2a / R-recursive-creep: the depth-64 limit in
    /// `collect_heap_leaves` is the last-resort net against a self-referential
    /// struct slipping past typecheck. A hand-built recursive layout
    /// (`Node { next: Node }`) — which the real pipeline rejects at typecheck —
    /// must yield a `JitError`, NOT infinite compile-time recursion / stack
    /// overflow. Poison (remove the depth check) → this stack-overflows → proves
    /// the limit is load-bearing.
    #[test]
    fn collect_heap_leaves_recursive_hits_depth_limit() {
        let mut b = MirBuilder::new("rec", MirType::Unit);
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "Node",
            &[(
                "next".to_string(),
                MirType::Struct("Node".to_string()),
                8,
                8,
            )],
        ));
        let bb0 = b.new_block();
        b.set_terminator(bb0, return_(vec![]));
        let body = b.build(bb0);

        let mut leaves: Vec<(i32, LeafKind)> = Vec::new();
        let result = JitContext::collect_heap_leaves("Node", 0, &body, 0, &mut leaves);
        assert!(
            result.is_err(),
            "recursive struct must hit the depth limit (JitError), not recurse forever"
        );
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("depth 64"),
            "depth-limit error expected, got: {msg}"
        );
    }

    /// HP.3: Deinit Outcome sets disc=0 → Drop glue no-op.
    /// Build Outcome<String,Integer> with disc=1, Deinit it,
    /// then Drop — must NOT call free (count stays 0).
    #[test]
    #[allow(unsafe_code)]
    fn hp3_deinit_then_drop_must_not_free() {
        use std::sync::atomic::Ordering;

        HP3A_FREE_COUNT.store(0, Ordering::SeqCst);

        let mut b = MirBuilder::new("hp3_deinit", MirType::Unit);
        let outcome_ty = MirType::Outcome {
            value_type: Box::new(MirType::String),
            error_type: Box::new(MirType::Integer),
            allow_null_state: false,
        };

        let bb0 = b.new_block();
        let o = b.new_local();
        b.set_local_mir_type(o, outcome_ty);
        b.push(bb0, storage_live(o));
        b.push(
            bb0,
            Statement::OutcomeAlloc {
                dest: o,
                span: DUMMY_SPAN,
            },
        );

        let disc_tmp = b.new_local();
        b.push(bb0, storage_live(disc_tmp));
        b.push(
            bb0,
            Statement::Const {
                dest: Place::local(disc_tmp),
                value: triet_mir::ConstValue::Trit(1),
                span: DUMMY_SPAN,
            },
        );
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(o).project(Projection::OutcomeDiscriminant),
                source: Place::local(disc_tmp),
                span: DUMMY_SPAN,
            },
        );

        for (val, proj) in [
            (1i128, Projection::OutcomePayload),
            (0i128, Projection::OutcomePayloadLen),
            (8i128, Projection::OutcomePayloadCap),
        ] {
            let tmp = b.new_local();
            b.push(bb0, storage_live(tmp));
            b.push(
                bb0,
                Statement::Const {
                    dest: Place::local(tmp),
                    value: triet_mir::ConstValue::Integer(val),
                    span: DUMMY_SPAN,
                },
            );
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(o).project(proj),
                    source: Place::local(tmp),
                    span: DUMMY_SPAN,
                },
            );
        }

        b.push(bb0, Statement::Deinit(o, DUMMY_SPAN));
        b.push(bb0, Statement::Drop(o, DUMMY_SPAN));
        b.set_terminator(bb0, return_(vec![]));
        let body = b.build(bb0);

        let shims = &[ShimSymbol::fn_2_0("__triet_string_free", __hp3a_count_free)];
        let mut ctx = JitContext::with_shims(shims);
        let func = ctx.compile(&body).expect("HP.3 compile");
        let _ = unsafe { func.call_i64_0() };

        assert_eq!(
            HP3A_FREE_COUNT.load(Ordering::SeqCst),
            0,
            "HP.3a: Deinit→Drop must free 0 times"
        );
    }

    /// HP.3b: without Deinit, Drop after bind = double-free (2 frees).
    #[test]
    #[allow(unsafe_code)]
    #[allow(clippy::too_many_lines)] // JIT integration test — refactor not in E1b scope
    fn hp3_no_deinit_double_frees() {
        use std::sync::atomic::Ordering;

        HP3B_FREE_COUNT.store(0, Ordering::SeqCst);

        let mut b = MirBuilder::new("hp3_nod", MirType::Unit);
        let outcome_ty = MirType::Outcome {
            value_type: Box::new(MirType::String),
            error_type: Box::new(MirType::Integer),
            allow_null_state: false,
        };
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "String",
            &[
                ("ptr".to_string(), MirType::Integer, 8, 8),
                ("len".to_string(), MirType::Integer, 8, 8),
                ("cap".to_string(), MirType::Integer, 8, 8),
            ],
        ));

        let bb0 = b.new_block();
        let o = b.new_local();
        b.set_local_mir_type(o, outcome_ty);
        b.push(bb0, storage_live(o));
        b.push(
            bb0,
            Statement::OutcomeAlloc {
                dest: o,
                span: DUMMY_SPAN,
            },
        );

        let disc_tmp = b.new_local();
        b.push(bb0, storage_live(disc_tmp));
        b.push(
            bb0,
            Statement::Const {
                dest: Place::local(disc_tmp),
                value: triet_mir::ConstValue::Trit(1),
                span: DUMMY_SPAN,
            },
        );
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(o).project(Projection::OutcomeDiscriminant),
                source: Place::local(disc_tmp),
                span: DUMMY_SPAN,
            },
        );

        for (val, proj) in [
            (1i128, Projection::OutcomePayload),
            (0i128, Projection::OutcomePayloadLen),
            (8i128, Projection::OutcomePayloadCap),
        ] {
            let tmp = b.new_local();
            b.push(bb0, storage_live(tmp));
            b.push(
                bb0,
                Statement::Const {
                    dest: Place::local(tmp),
                    value: triet_mir::ConstValue::Integer(val),
                    span: DUMMY_SPAN,
                },
            );
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(o).project(proj),
                    source: Place::local(tmp),
                    span: DUMMY_SPAN,
                },
            );
        }

        let x = b.new_local();
        b.set_local_mir_type(x, MirType::String);
        b.push(bb0, storage_live(x));
        for (field, proj) in [
            ("ptr", Projection::OutcomePayload),
            ("len", Projection::OutcomePayloadLen),
            ("cap", Projection::OutcomePayloadCap),
        ] {
            let tmp = b.new_local();
            b.push(bb0, storage_live(tmp));
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(tmp),
                    source: Place::local(o).project(proj),
                    span: DUMMY_SPAN,
                },
            );
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(x).project(Projection::Field(field.to_string())),
                    source: Place::local(tmp),
                    span: DUMMY_SPAN,
                },
            );
        }

        b.push(bb0, Statement::Drop(x, DUMMY_SPAN));
        b.push(bb0, Statement::Drop(o, DUMMY_SPAN));
        b.set_terminator(bb0, return_(vec![]));
        let body = b.build(bb0);

        let shims = &[ShimSymbol::fn_2_0("__triet_string_free", __hp3b_count_free)];
        let mut ctx = JitContext::with_shims(shims);
        let func = ctx.compile(&body).expect("HP.3b compile");
        let _ = unsafe { func.call_i64_0() };

        assert_eq!(
            HP3B_FREE_COUNT.load(Ordering::SeqCst),
            2,
            "HP.3b: without Deinit, Drop(x)+Drop(o) must free twice (double-free)"
        );
    }

    /// HP.4: heap `~+>` map (`|v| v`) free-balance. Replicates the desugar's
    /// emitted chain for an identity map of `Outcome<String,Integer>`:
    ///   inner (disc=Pos, heap String) → bind v {ptr,len,cap} → Deinit(inner)
    ///   → recompose v into result → Deinit(v) → Drop(inner)/Drop(v)/Drop(result).
    /// Exactly ONE heap value is live (result owns it); the two Deinit
    /// tombstones make Drop(inner) and Drop(v) no-ops → free count == 1.
    /// Teeth: drop `Deinit(inner)` → Drop(inner) also frees → count 2.
    /// Teeth: drop `Deinit(v)` → Drop(v) also frees → count 2.
    /// Teeth: skip the v→result recompose → result.ptr=0 → count 0 (leak).
    #[test]
    #[allow(unsafe_code)]
    #[allow(clippy::too_many_lines)] // hand-built MIR replicating the map desugar — naturally long
    fn hp4_heap_map_frees_exactly_once() {
        use std::sync::atomic::Ordering;

        HP4_FREE_COUNT.store(0, Ordering::SeqCst);

        let mut b = MirBuilder::new("hp4_map", MirType::Unit);
        let outcome_ty = MirType::Outcome {
            value_type: Box::new(MirType::String),
            error_type: Box::new(MirType::Integer),
            allow_null_state: false,
        };
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "String",
            &[
                ("ptr".to_string(), MirType::Integer, 8, 8),
                ("len".to_string(), MirType::Integer, 8, 8),
                ("cap".to_string(), MirType::Integer, 8, 8),
            ],
        ));

        let bb0 = b.new_block();

        // ── inner Outcome<String,Integer>, disc=Pos, fake heap String ──
        let inner = b.new_local();
        b.set_local_mir_type(inner, outcome_ty.clone());
        b.push(bb0, storage_live(inner));
        b.push(
            bb0,
            Statement::OutcomeAlloc {
                dest: inner,
                span: DUMMY_SPAN,
            },
        );
        let disc_pos = b.new_local();
        b.push(bb0, storage_live(disc_pos));
        b.push(
            bb0,
            Statement::Const {
                dest: Place::local(disc_pos),
                value: triet_mir::ConstValue::Trit(1),
                span: DUMMY_SPAN,
            },
        );
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(inner).project(Projection::OutcomeDiscriminant),
                source: Place::local(disc_pos),
                span: DUMMY_SPAN,
            },
        );
        for (val, proj) in [
            (1i128, Projection::OutcomePayload),
            (0i128, Projection::OutcomePayloadLen),
            (8i128, Projection::OutcomePayloadCap),
        ] {
            let tmp = b.new_local();
            b.push(bb0, storage_live(tmp));
            b.push(
                bb0,
                Statement::Const {
                    dest: Place::local(tmp),
                    value: triet_mir::ConstValue::Integer(val),
                    span: DUMMY_SPAN,
                },
            );
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(inner).project(proj),
                    source: Place::local(tmp),
                    span: DUMMY_SPAN,
                },
            );
        }

        // ── bind v = decompose inner.payload {ptr,len,cap} ──
        let v = b.new_local();
        b.set_local_mir_type(v, MirType::String);
        b.push(bb0, storage_live(v));
        for (field, proj) in [
            ("ptr", Projection::OutcomePayload),
            ("len", Projection::OutcomePayloadLen),
            ("cap", Projection::OutcomePayloadCap),
        ] {
            let tmp = b.new_local();
            b.push(bb0, storage_live(tmp));
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(tmp),
                    source: Place::local(inner).project(proj),
                    span: DUMMY_SPAN,
                },
            );
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(v).project(Projection::Field(field.to_string())),
                    source: Place::local(tmp),
                    span: DUMMY_SPAN,
                },
            );
        }
        // inner payload moved out → tombstone.
        b.push(bb0, Statement::Deinit(inner, DUMMY_SPAN));

        // ── result Outcome, disc=Pos, recompose v {ptr,len,cap} ──
        let result = b.new_local();
        b.set_local_mir_type(result, outcome_ty);
        b.push(bb0, storage_live(result));
        b.push(
            bb0,
            Statement::OutcomeAlloc {
                dest: result,
                span: DUMMY_SPAN,
            },
        );
        let disc_pos2 = b.new_local();
        b.push(bb0, storage_live(disc_pos2));
        b.push(
            bb0,
            Statement::Const {
                dest: Place::local(disc_pos2),
                value: triet_mir::ConstValue::Trit(1),
                span: DUMMY_SPAN,
            },
        );
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(result).project(Projection::OutcomeDiscriminant),
                source: Place::local(disc_pos2),
                span: DUMMY_SPAN,
            },
        );
        for (field, proj) in [
            ("ptr", Projection::OutcomePayload),
            ("len", Projection::OutcomePayloadLen),
            ("cap", Projection::OutcomePayloadCap),
        ] {
            let tmp = b.new_local();
            b.push(bb0, storage_live(tmp));
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(tmp),
                    source: Place::local(v).project(Projection::Field(field.to_string())),
                    span: DUMMY_SPAN,
                },
            );
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(result).project(proj),
                    source: Place::local(tmp),
                    span: DUMMY_SPAN,
                },
            );
        }
        // result now owns the heap value → tombstone v.
        b.push(bb0, Statement::Deinit(v, DUMMY_SPAN));

        // ── scope-pop Drops (F1 fix order: after the moves) ──
        b.push(bb0, Statement::Drop(inner, DUMMY_SPAN)); // no-op (Deinit'd)
        b.push(bb0, Statement::Drop(v, DUMMY_SPAN)); // no-op (Deinit'd)
        b.push(bb0, Statement::Drop(result, DUMMY_SPAN)); // frees once
        b.set_terminator(bb0, return_(vec![]));
        let body = b.build(bb0);
        println!("=== MIR (hp4_map) ===\n{body}");

        let shims = &[ShimSymbol::fn_2_0("__triet_string_free", __hp4_count_free)];
        let mut ctx = JitContext::with_shims(shims);
        let func = ctx.compile(&body).expect("HP.4 map compile");
        let _ = unsafe { func.call_i64_0() };

        assert_eq!(
            HP4_FREE_COUNT.load(Ordering::SeqCst),
            1,
            "HP.4: heap map (inner→v→result, 2 Deinit) must free exactly once"
        );
    }

    /// 4i-1: M1 Zeroing-on-Move — after Assign of Move type, source must be 0.
    #[test]
    #[allow(unsafe_code)]
    fn m1_zeroing_on_move() {
        let mut b = MirBuilder::new("test_m1", MirType::Integer);
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
        let v0 = __triet_vector_alloc(0, 2, 8);
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
        let v0 = __triet_vector_alloc(0, 2, 8);
        let v1 = __triet_vector_push(v0, 1);
        let v2 = __triet_vector_push(v1, 2);
        // v0, v1, v2 all share the same ptr (in-place for len 0→1, 1→2)
        // 3rd push triggers realloc
        let v3 = __triet_vector_push(v2, 3);
        // After realloc: old block is freed, v3 = new block
        assert_ne!(v0, v3, "realloc must change ptr");
        // Allocate another vector — should get the old block back (most allocators)
        let fresh = __triet_vector_alloc(0, 2, 8);
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

    /// Basic insert + get round-trip. `key_stride`=8 (Integer, byte-compat) —
    /// `is_update_out`=0 (unused, Integer key never redundant-free).
    #[test]
    fn hashmap_insert_get_roundtrip() {
        let m = __triet_hashmap_alloc(0, 4, 8, 8);
        assert_eq!(__triet_hashmap_len(m), 0);
        let m = __triet_hashmap_insert(m, 1, 100, 0);
        assert_eq!(__triet_hashmap_len(m), 1);
        assert_eq!(__triet_hashmap_get(m, 1), 100);
        // Key not found
        assert_eq!(__triet_hashmap_get(m, 2), triet_mir::NULL_SENTINEL);
        __triet_hashmap_free(m);
    }

    /// C9: insert same key must UPDATE value, len unchanged.
    #[test]
    fn hashmap_insert_same_key_updates_value() {
        let m = __triet_hashmap_alloc(0, 4, 8, 8);
        let m = __triet_hashmap_insert(m, 1, 10, 0);
        let m = __triet_hashmap_insert(m, 1, 20, 0);
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
        let m = __triet_hashmap_alloc(0, 4, 8, 8); // cap=4, load factor at 0.75 → 3 max before realloc
        let m = __triet_hashmap_insert(m, 5, 50, 0);
        let m = __triet_hashmap_insert(m, 6, 60, 0);
        let m = __triet_hashmap_insert(m, 7, 70, 0);
        // 4th insert triggers realloc (3 >= 4*3/4 = 3). cap→8.
        let m = __triet_hashmap_insert(m, 13, 130, 0);
        assert_eq!(__triet_hashmap_len(m), 4);
        // All keys survive rehash with displaced positions
        assert_eq!(__triet_hashmap_get(m, 5), 50);
        assert_eq!(__triet_hashmap_get(m, 6), 60);
        assert_eq!(__triet_hashmap_get(m, 7), 70);
        assert_eq!(__triet_hashmap_get(m, 13), 130);
        __triet_hashmap_free(m);
    }

    /// ADR-0078 Tooth #3: rehash with fat `value_stride=24` must copy the FULL
    /// 24B value cell, NOT just the first i64. Insert 4 -> trigger realloc ->
    /// read back value cell directly. The second i64 in the cell (len field of
    /// fake {ptr,len,cap}) must stay intact (=5). Poison the rehash loop's
    /// `copy_nonoverlapping(stride)` -> i64-only read/write -> len@8=0 (RED).
    #[test]
    #[allow(unsafe_code)]
    #[allow(clippy::cast_sign_loss)]
    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::cast_possible_wrap)]
    #[allow(clippy::cast_ptr_alignment)]
    #[allow(clippy::ptr_as_ptr)]
    fn hashmap_rehash_fat_value_preserves_full_cell() {
        let m = __triet_hashmap_alloc(0, 4, 8, 24);
        assert_ne!(m, 0);
        let e1 = [101_i64, 5, 8];
        let e2 = [202_i64, 5, 8];
        let e3 = [303_i64, 5, 8];
        let e4 = [404_i64, 5, 8];
        let m = __triet_hashmap_insert(m, 1, e1.as_ptr() as i64, 0);
        let m = __triet_hashmap_insert(m, 2, e2.as_ptr() as i64, 0);
        let m = __triet_hashmap_insert(m, 3, e3.as_ptr() as i64, 0);
        let m = __triet_hashmap_insert(m, 4, e4.as_ptr() as i64, 0);
        assert_eq!(__triet_hashmap_get(m, 1), 101, "ptr@0 OK");
        assert_eq!(__triet_hashmap_get(m, 4), 404, "ptr@0 OK");
        // Verify FULL 24B cell: len@8 must be 5.
        let body = m as *mut u8;
        let cap = unsafe { (body as *const i64).add(1).read_unaligned() } as usize;
        let hash = (1_i64 % cap as i64 + cap as i64) % cap as i64;
        let mut probe = hash as usize;
        loop {
            let state = unsafe { *super::hashmap_state_ptr(body, probe) };
            assert_ne!(state, 0u8, "key=1 not found after rehash");
            if state == 1u8
                && unsafe { (super::hashmap_key_ptr(body, probe) as *const i64).read_unaligned() }
                    == 1
            {
                let vptr = unsafe { super::hashmap_value_ptr(body, probe) };
                let len_field = unsafe { (vptr as *const i64).add(1).read_unaligned() };
                assert_eq!(
                    len_field, 5,
                    "ADR-0078 tooth #3: rehash must preserve len@8 of fat value                      cell (i64-only copy loses it — got {len_field}, expected 5)"
                );
                break;
            }
            probe = (probe + 1) % cap;
        }
        __triet_hashmap_free(m);
    }

    // N7-C5: insert with v == i64::MIN must die (D2 reject-on-insert).
    #[test]
    fn n7_hashmap_insert_min_value_rejected() {
        n7_child_guard("n7_hashmap_insert_min_value_rejected", || {
            let m = __triet_hashmap_alloc(0, 4, 8, 8);
            let _ = __triet_hashmap_insert(m, 1, triet_mir::NULL_SENTINEL, 0);
        });
        let status = spawn_n7_child("n7_hashmap_insert_min_value_rejected");
        assert_n7_signal("n7_hashmap_insert_min_value_rejected", status, 6);
    }

    // ── ADR-0080 KM-P1a: key-typed HashMap<String,V> — backend/shim teeth ──
    //
    // Each JIT test below drives the FULL registry-routed path (real
    // allocator, `String` fields built via `ConstValue::String` so every key
    // is a genuine independent heap allocation — poisoning the free path
    // would SIGABRT on a real double-free, not just miscount). A fresh
    // `Local` is used for every call's `dest` (never reusing an `args[0]`
    // local as its own `dest`) — that self-aliasing pattern would collide
    // with M3 Zeroing-on-Move (which zeroes `args[0]`'s var AFTER dest
    // binding, since `__triet_hashmap_insert` consumes arg 0) and is not
    // how a real lowerer emits `map = insert(map, ...)` (fresh SSA-ish local
    // per assignment); avoiding it keeps these teeth honest about what the
    // registry-routing mechanism does, not an artifact of hand-built MIR.

    fn km_p1a_string_layout() -> triet_mir::StructLayout {
        triet_mir::StructLayout::compute(
            "String",
            &[
                ("ptr".to_string(), MirType::Integer, 8, 8),
                ("len".to_string(), MirType::Integer, 8, 8),
                ("cap".to_string(), MirType::Integer, 8, 8),
            ],
        )
    }

    fn km_p1a_string_const(local: Local, text: &str) -> Statement {
        Statement::Const {
            dest: Place::local(local),
            value: ConstValue::String(text.to_string()),
            span: DUMMY_SPAN,
        }
    }

    fn km_p1a_shim_call(
        callee_name: &str,
        args: Vec<Local>,
        dest: Vec<Local>,
        return_bb: BasicBlock,
    ) -> Terminator {
        Terminator::CallDispatch {
            callee: FunctionId(0),
            callee_name: callee_name.into(),
            target: CallTarget::Shim,
            args,
            return_bb,
            dest,
            return_shape: ReturnShape::Scalar,
            span: DUMMY_SPAN,
        }
    }

    /// ADR-0080 tooth #1 (Author BẮT BUỘC): map `Drop` must free EVERY
    /// resident String key via the JIT-emitted key-free loop (D.1,
    /// registry-routed — `emit_hashmap_key_free_loop` inside
    /// `emit_hashmap_free_value`). Poison → RED: comment out the
    /// `self.emit_hashmap_key_free_loop(builder, ptr_val)?;` call in
    /// `emit_hashmap_free_value` → count stays 0 (leak) instead of 1.
    #[test]
    #[allow(unsafe_code)]
    fn adr0080_km_p1a_map_drop_frees_string_key() {
        use std::sync::atomic::Ordering;
        KM_P1A_MAP_DROP_FREE_COUNT.store(0, Ordering::SeqCst);

        let mut b = MirBuilder::new("km_p1a_map_drop", MirType::Unit);
        b.add_struct_layout(km_p1a_string_layout());

        let bb0 = b.new_block();
        let bb1 = b.new_block();
        let bb2 = b.new_block();

        let key = b.new_local();
        b.set_local_mir_type(key, MirType::String);
        b.push(bb0, storage_live(key));
        b.push(bb0, km_p1a_string_const(key, "alice"));
        let len0 = b.new_local();
        let cap0 = b.new_local();
        b.push(bb0, storage_live(len0));
        b.push(bb0, const_int(len0, 0));
        b.push(bb0, storage_live(cap0));
        b.push(bb0, const_int(cap0, 4));
        let map0 = b.new_local();
        b.set_local_mir_type(
            map0,
            MirType::HashMap(Box::new(MirType::String), Box::new(MirType::Integer)),
        );
        b.push(bb0, storage_live(map0));
        b.set_terminator(
            bb0,
            km_p1a_shim_call("__triet_hashmap_alloc", vec![len0, cap0], vec![map0], bb1),
        );

        let value = b.new_local();
        b.push(bb1, storage_live(value));
        b.push(bb1, const_int(value, 42));
        let map1 = b.new_local();
        b.set_local_mir_type(
            map1,
            MirType::HashMap(Box::new(MirType::String), Box::new(MirType::Integer)),
        );
        b.push(bb1, storage_live(map1));
        b.set_terminator(
            bb1,
            km_p1a_shim_call(
                "__triet_hashmap_insert",
                vec![map0, key, value],
                vec![map1],
                bb2,
            ),
        );

        b.push(bb2, Statement::Drop(map1, DUMMY_SPAN));
        b.set_terminator(bb2, return_(vec![]));

        let body = b.build(bb0);
        let shims = &[
            ShimSymbol::fn_2_1(
                "__triet_string_from_bytes",
                super::__triet_string_from_bytes,
            ),
            ShimSymbol::fn_2_0("__triet_string_free", __km_p1a_map_drop_count_free),
            ShimSymbol::fn_4_1("__triet_hashmap_alloc", super::__triet_hashmap_alloc),
            ShimSymbol::fn_4_1("__triet_hashmap_insert", super::__triet_hashmap_insert),
            ShimSymbol::fn_1_0("__triet_hashmap_free", super::__triet_hashmap_free),
        ];
        let mut ctx = JitContext::with_shims(shims);
        let func = ctx.compile(&body).expect("KM-P1a map-drop compile");
        let _ = unsafe { func.call_i64_0() };

        assert_eq!(
            KM_P1A_MAP_DROP_FREE_COUNT.load(Ordering::SeqCst),
            1,
            "ADR-0080 tooth #1: map Drop must free the resident String key \
             exactly once via the JIT-emitted key-free loop"
        );
    }

    /// ADR-0080 tooth #2 (Author BẮT BUỘC): inserting a dup-content String
    /// key hits the UPDATE branch — the map keeps the RESIDENT key, and the
    /// caller's now-redundant incoming key must be freed by the JIT
    /// call-site (D.2, gated on `is_update_out`, registry-routed — a
    /// Rust-internal free inside the shim would be invisible to this exact
    /// counting harness, per ADR §AMEND-1). Two independent frees are
    /// expected: the redundant key (right after the 2nd insert) + the
    /// resident key (at map Drop, D.1). Poison → RED (either mechanism):
    /// gut the `insert_key_free_gate` conditional block → count 2→1
    /// (redundant leak); gut `emit_hashmap_key_free_loop` → count 2→1
    /// (resident leak, independently of the first poison).
    #[test]
    #[allow(unsafe_code)]
    fn adr0080_km_p1a_update_frees_redundant_key() {
        use std::sync::atomic::Ordering;
        KM_P1A_UPDATE_FREE_COUNT.store(0, Ordering::SeqCst);

        let mut b = MirBuilder::new("km_p1a_update", MirType::Unit);
        b.add_struct_layout(km_p1a_string_layout());

        let bb0 = b.new_block();
        let bb1 = b.new_block();
        let bb2 = b.new_block();
        let bb3 = b.new_block();

        let key1 = b.new_local();
        b.set_local_mir_type(key1, MirType::String);
        b.push(bb0, storage_live(key1));
        b.push(bb0, km_p1a_string_const(key1, "alice"));
        let len0 = b.new_local();
        let cap0 = b.new_local();
        b.push(bb0, storage_live(len0));
        b.push(bb0, const_int(len0, 0));
        b.push(bb0, storage_live(cap0));
        b.push(bb0, const_int(cap0, 4));
        let map0 = b.new_local();
        b.set_local_mir_type(
            map0,
            MirType::HashMap(Box::new(MirType::String), Box::new(MirType::Integer)),
        );
        b.push(bb0, storage_live(map0));
        b.set_terminator(
            bb0,
            km_p1a_shim_call("__triet_hashmap_alloc", vec![len0, cap0], vec![map0], bb1),
        );

        let val1 = b.new_local();
        b.push(bb1, storage_live(val1));
        b.push(bb1, const_int(val1, 1));
        let map1 = b.new_local();
        b.set_local_mir_type(
            map1,
            MirType::HashMap(Box::new(MirType::String), Box::new(MirType::Integer)),
        );
        b.push(bb1, storage_live(map1));
        b.set_terminator(
            bb1,
            km_p1a_shim_call(
                "__triet_hashmap_insert",
                vec![map0, key1, val1],
                vec![map1],
                bb2,
            ),
        );

        // key2: SAME CONTENT as key1, a FRESH independent allocation
        // (ConstValue::String always calls __triet_string_from_bytes fresh).
        let key2 = b.new_local();
        b.set_local_mir_type(key2, MirType::String);
        b.push(bb2, storage_live(key2));
        b.push(bb2, km_p1a_string_const(key2, "alice"));
        let val2 = b.new_local();
        b.push(bb2, storage_live(val2));
        b.push(bb2, const_int(val2, 2));
        let map2 = b.new_local();
        b.set_local_mir_type(
            map2,
            MirType::HashMap(Box::new(MirType::String), Box::new(MirType::Integer)),
        );
        b.push(bb2, storage_live(map2));
        b.set_terminator(
            bb2,
            km_p1a_shim_call(
                "__triet_hashmap_insert",
                vec![map1, key2, val2],
                vec![map2],
                bb3,
            ),
        );

        b.push(bb3, Statement::Drop(map2, DUMMY_SPAN));
        b.set_terminator(bb3, return_(vec![]));

        let body = b.build(bb0);
        let shims = &[
            ShimSymbol::fn_2_1(
                "__triet_string_from_bytes",
                super::__triet_string_from_bytes,
            ),
            ShimSymbol::fn_2_0("__triet_string_free", __km_p1a_update_count_free),
            ShimSymbol::fn_4_1("__triet_hashmap_alloc", super::__triet_hashmap_alloc),
            ShimSymbol::fn_4_1("__triet_hashmap_insert", super::__triet_hashmap_insert),
            ShimSymbol::fn_1_0("__triet_hashmap_free", super::__triet_hashmap_free),
        ];
        let mut ctx = JitContext::with_shims(shims);
        let func = ctx.compile(&body).expect("KM-P1a update compile");
        let _ = unsafe { func.call_i64_0() };

        assert_eq!(
            KM_P1A_UPDATE_FREE_COUNT.load(Ordering::SeqCst),
            2,
            "ADR-0080 tooth #2: dup-content insert must free the redundant \
             incoming key (D.2) + map Drop frees the resident key (D.1) — \
             expected 2 total frees"
        );
    }

    /// ADR-0080 tooth #3: `remove` on a String key must free the RESIDENT
    /// key surfaced through `key_out_ptr` (D.5, registry-routed). The
    /// lookup key itself is a BORROW (ADR-0080 Mũi D point 4) — never freed
    /// by remove. After remove, the map is empty so the subsequent Drop
    /// must NOT double-free (tombstoned slot skips D.1's occupied check).
    /// Poison → RED: gut the `remove_key_free_ptr` free call → count stays
    /// 0 instead of 1.
    #[test]
    #[allow(unsafe_code)]
    fn adr0080_km_p1a_remove_frees_resident_key() {
        use std::sync::atomic::Ordering;
        KM_P1A_REMOVE_FREE_COUNT.store(0, Ordering::SeqCst);

        let mut b = MirBuilder::new("km_p1a_remove", MirType::Unit);
        b.add_struct_layout(km_p1a_string_layout());

        let bb0 = b.new_block();
        let bb1 = b.new_block();
        let bb2 = b.new_block();
        let bb3 = b.new_block();

        let key = b.new_local();
        b.set_local_mir_type(key, MirType::String);
        b.push(bb0, storage_live(key));
        b.push(bb0, km_p1a_string_const(key, "bob"));
        let len0 = b.new_local();
        let cap0 = b.new_local();
        b.push(bb0, storage_live(len0));
        b.push(bb0, const_int(len0, 0));
        b.push(bb0, storage_live(cap0));
        b.push(bb0, const_int(cap0, 4));
        let map0 = b.new_local();
        b.set_local_mir_type(
            map0,
            MirType::HashMap(Box::new(MirType::String), Box::new(MirType::Integer)),
        );
        b.push(bb0, storage_live(map0));
        b.set_terminator(
            bb0,
            km_p1a_shim_call("__triet_hashmap_alloc", vec![len0, cap0], vec![map0], bb1),
        );

        let val = b.new_local();
        b.push(bb1, storage_live(val));
        b.push(bb1, const_int(val, 7));
        let map1 = b.new_local();
        b.set_local_mir_type(
            map1,
            MirType::HashMap(Box::new(MirType::String), Box::new(MirType::Integer)),
        );
        b.push(bb1, storage_live(map1));
        b.set_terminator(
            bb1,
            km_p1a_shim_call(
                "__triet_hashmap_insert",
                vec![map0, key, val],
                vec![map1],
                bb2,
            ),
        );

        // remove mutates map1 IN PLACE (tombstone) — no fresh dest for the
        // map; only the removed VALUE binds a dest. ADR-0080 Mũi D3: insert
        // now CONSUMES `key` (arg_consumes[1]=true, String not Copy) — the
        // JIT's M3 zeroing tombstones `key`'s own slot right after the
        // insert call (bb1), so reusing that SAME local as remove's lookup
        // arg would read a zeroed ptr → `__triet_string_hash` trap-on-0
        // SIGABRT. A real `.tri` program hits the identical rule as E2420
        // (use-after-move) at borrowck — hand-built MIR bypasses borrowck,
        // so the test itself must respect the same ownership discipline: a
        // FRESH same-content key for the lookup (mirrors ADR-0080 Mũi D4 —
        // remove's key is a BORROW of the CALLER's own key, never the one
        // already moved into the map).
        let lookup_key = b.new_local();
        b.set_local_mir_type(lookup_key, MirType::String);
        b.push(bb2, storage_live(lookup_key));
        b.push(bb2, km_p1a_string_const(lookup_key, "bob"));
        let removed_val = b.new_local();
        b.push(bb2, storage_live(removed_val));
        b.set_terminator(
            bb2,
            km_p1a_shim_call(
                "__triet_hashmap_remove",
                vec![map1, lookup_key],
                vec![removed_val],
                bb3,
            ),
        );

        // lookup_key is a BORROW (D4, never consumed by remove) — the
        // CALLER still owns it and must free it itself, same as any other
        // owned local.
        b.push(bb3, Statement::Drop(lookup_key, DUMMY_SPAN));
        b.push(bb3, Statement::Drop(map1, DUMMY_SPAN));
        b.set_terminator(bb3, return_(vec![]));

        let body = b.build(bb0);
        let shims = &[
            ShimSymbol::fn_2_1(
                "__triet_string_from_bytes",
                super::__triet_string_from_bytes,
            ),
            ShimSymbol::fn_2_0("__triet_string_free", __km_p1a_remove_count_free),
            ShimSymbol::fn_4_1("__triet_hashmap_alloc", super::__triet_hashmap_alloc),
            ShimSymbol::fn_4_1("__triet_hashmap_insert", super::__triet_hashmap_insert),
            ShimSymbol::fn_4_1("__triet_hashmap_remove", super::__triet_hashmap_remove),
            ShimSymbol::fn_1_0("__triet_hashmap_free", super::__triet_hashmap_free),
        ];
        let mut ctx = JitContext::with_shims(shims);
        let func = ctx.compile(&body).expect("KM-P1a remove compile");
        let _ = unsafe { func.call_i64_0() };

        assert_eq!(
            KM_P1A_REMOVE_FREE_COUNT.load(Ordering::SeqCst),
            2,
            "ADR-0080 tooth #3: remove must free the resident String key \
             exactly once (D.5) + caller frees its own borrowed lookup_key \
             once; subsequent map Drop must not double-free"
        );
    }

    /// ADR-0080 tooth #5: content hash/eq — two Strings with EQUAL CONTENT
    /// but DIFFERENT allocations must HIT on `get` (identity-hash-on-pointer
    /// would MISS). Rust-level (bypasses JIT — pure shim logic, no
    /// drop-glue codegen involved). Poison: swap `hashmap_key_hash`'s
    /// String branch for the Integer identity path → hashes the POINTER
    /// instead of content → equal-content different-allocation probe MISSES.
    ///
    /// Two independent checks, both required RED under the poison:
    /// (1) a large `cap` (prime-ish, `1_000_003`) so that under a broken
    ///     pointer-hash, the probe almost certainly lands on an EMPTY slot
    ///     before reaching the occupied one (linear probing stops at the
    ///     first empty slot) — a small cap (e.g. 4) risks a FALSE-GREEN
    ///     accidental hit if the wrong hash happens to collide by luck.
    /// (2) a direct `hashmap_key_hash` equality check — deterministic
    ///     regardless of actual heap addresses (content hash is
    ///     allocation-independent by construction; a pointer-identity hash
    ///     of two DIFFERENT allocations is virtually never congruent mod a
    ///     6-digit prime-ish cap).
    #[test]
    #[allow(unsafe_code)]
    #[allow(clippy::cast_possible_wrap)] // test-only String::len() as i64, values tiny
    fn adr0080_km_p1a_content_hash_hit_across_allocations() {
        let s1 = "alice";
        let s2 = String::from("alice"); // distinct backing allocation, same content
        let p1 = __triet_string_from_bytes(s1.as_ptr() as i64, s1.len() as i64);
        let p2 = __triet_string_from_bytes(s2.as_ptr() as i64, s2.len() as i64);
        assert_ne!(
            p1, p2,
            "test setup: source Strings must be distinct allocations"
        );

        let key1 = [p1, s1.len() as i64, s1.len() as i64]; // {ptr,len,cap}
        let key2 = [p2, s2.len() as i64, s2.len() as i64];

        let idx1 = hashmap_key_hash(24, key1.as_ptr() as i64, 1_000_003);
        let idx2 = hashmap_key_hash(24, key2.as_ptr() as i64, 1_000_003);
        assert_eq!(
            idx1, idx2,
            "ADR-0080 tooth #5: content hash must be allocation-independent \
             (equal-content keys must hash to the same slot regardless of pointer)"
        );

        let m = __triet_hashmap_alloc(0, 1_000_003, 24, 8); // key_stride=24 (String), value=Integer
        let m = __triet_hashmap_insert(m, key1.as_ptr() as i64, 42, 0);
        let got = __triet_hashmap_get(m, key2.as_ptr() as i64);
        assert_eq!(
            got, 42,
            "ADR-0080 tooth #5: equal-content different-allocation key must HIT"
        );

        // Cleanup: __triet_hashmap_free only frees the buffer (key-free is
        // JIT-emitted, not exercised on this Rust-level-only path) — free
        // the resident key (p1) and the never-inserted probe (p2) by hand.
        __triet_string_free(p1, s1.len() as i64);
        __triet_string_free(p2, s2.len() as i64);
        __triet_hashmap_free(m);
    }

    /// ADR-0080 tooth #7: rehash must move a String key's bytes by
    /// `key_stride` (24B memcpy), NOT an i64-only read/write — corrupting
    /// `len`/`cap` on the far side would silently break future lookups.
    /// Rust-level (bypasses JIT). Insert 4 keys on cap=4 (triggers realloc
    /// to cap=8 on the 4th insert, mirroring `hashmap_rehash_on_realloc`),
    /// then `get` with a FRESH-allocation same-content probe for one of the
    /// rehashed (not the newest) keys — must still HIT with the right value.
    /// Poison: rehash's key move → i64-only 8B copy (pre-ADR-0080 shape) →
    /// `slot_len`/`slot_cap` corrupted → content-eq reads garbage → MISS.
    #[test]
    #[allow(unsafe_code)]
    #[allow(clippy::cast_possible_wrap)] // test-only str::len() as i64, values tiny
    fn adr0080_km_p1a_rehash_preserves_key_content() {
        let names = ["ann", "bob", "cid", "don"];
        let mut ptrs = Vec::new();
        let mut m = __triet_hashmap_alloc(0, 4, 24, 8);
        for (i, name) in names.iter().enumerate() {
            let p = __triet_string_from_bytes(name.as_ptr() as i64, name.len() as i64);
            ptrs.push(p);
            let key = [p, name.len() as i64, name.len() as i64];
            // 4th insert (i==3) triggers realloc cap4→8, rehashing "ann"/"bob"/"cid".
            m = __triet_hashmap_insert(m, key.as_ptr() as i64, (i as i64) * 10, 0);
        }

        // Probe "cid" (rehashed, index 2) with a FRESH allocation.
        let probe_owned = String::from("cid");
        let probe_ptr = __triet_string_from_bytes(probe_owned.as_ptr() as i64, 3);
        let probe_key = [probe_ptr, 3_i64, 3_i64];
        let got = __triet_hashmap_get(m, probe_key.as_ptr() as i64);
        assert_eq!(
            got, 20,
            "ADR-0080 tooth #7: rehashed String key must survive content-intact \
             (memcpy by key_stride, not truncated i64 read/write)"
        );

        for p in ptrs {
            __triet_string_free(p, 3);
        }
        __triet_string_free(probe_ptr, 3);
        __triet_hashmap_free(m);
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
            let mut b = MirBuilder::new("add_test", MirType::Integer);
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
            let mut b = MirBuilder::new("sub_test", MirType::Integer);
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
            let mut b = MirBuilder::new("mul_test", MirType::Integer);
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

    /// E1b/ADR-0044: `__triet_string_append` with byte > 255 must TRAP
    /// (SIGABRT), not silently truncate to the low byte. Counterpart to the
    /// 8 overflow N7 tests — no silent data loss for an out-of-range byte.
    ///
    /// Teeth: poison the `0..=255` guard (`mir_lower` append shim) and this
    /// child stops aborting (writes low-byte 0, returns) → signal becomes
    /// `None` → the assertion fails. So the guard is load-bearing.
    #[test]
    #[allow(unsafe_code)]
    fn n7_string_append_byte_above_255() {
        n7_child_guard("n7_string_append_byte_above_255", || {
            // Valid empty String slot: cap=4, len=0 (no realloc on first append).
            let body = __triet_string_alloc(0, 4);
            let mut slot = FatStr {
                ptr: body,
                len: 0,
                cap: 4,
            };
            let _ = __triet_string_append(std::ptr::addr_of_mut!(slot) as i64, 256);
        });
        let status = spawn_n7_child("n7_string_append_byte_above_255");
        assert_n7_signal("n7_string_append_byte_above_255", status, 6);
    }

    /// E1b/ADR-0044: `__triet_string_append` with a negative byte must TRAP.
    /// `i64_low_byte(-1)` would otherwise silently write 0xFF — rejecting it
    /// keeps the shim strict per ADR-0044 (no garbage-in pass-through).
    #[test]
    #[allow(unsafe_code)]
    fn n7_string_append_byte_negative() {
        n7_child_guard("n7_string_append_byte_negative", || {
            let body = __triet_string_alloc(0, 4);
            let mut slot = FatStr {
                ptr: body,
                len: 0,
                cap: 4,
            };
            let _ = __triet_string_append(std::ptr::addr_of_mut!(slot) as i64, -1);
        });
        let status = spawn_n7_child("n7_string_append_byte_negative");
        assert_n7_signal("n7_string_append_byte_negative", status, 6);
    }
}
