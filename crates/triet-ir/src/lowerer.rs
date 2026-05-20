//! AST → IR lowerer — walks a [`triet_modules::ResolvedProgram`] and
//! produces an [`IrProgram`] in register-based SSA form.
//!
//! Per [ADR-0007], this is the bridge between the AST (parsed + resolved)
//! and every backend (VM, JIT, AOT, trytecode).

use std::collections::{BTreeMap, HashMap};

use triet_core::{Integer, Long, Trit, Tryte};
use triet_logic::Trilean;
use triet_modules::{AbsolutePath, Module, ResolvedProgram};
use triet_syntax::{
    Arena,
    expr::{BinaryOperator, Expr, MatchArm, OutcomeArm},
    item::{FunctionBody, FunctionDef, Item},
    numeric::{NumericSuffix, TrileanValue},
    stmt::{Block, Stmt},
    type_ast::TypeExpr,
};

use crate::constant::{Constant, ConstantPool};
use crate::instr::{BuiltinName, Instruction, Operand, PhiIncoming};
use crate::module::{BasicBlock, Function, IrModule, IrProgram};
use crate::types::{BlockId, ConstId, FuncId, TypeTag, ValueId};

// ── Entry point ────────────────────────────────────────────────────

/// Lower a resolved program to IR form.
///
/// The lowerer makes two passes:
/// 1. Register all functions (assign `FuncId`s).
/// 2. Lower each function body to IR instructions.
#[must_use]
pub fn lower_program(program: &ResolvedProgram) -> IrProgram {
    let mut ctx = LowerCtx::new(program);
    // Pass 1a: register all struct + enum definitions FIRST. Done
    // before function declaration so `declare_function` can resolve
    // user-struct return types (per `func_return_struct` doc).
    for (module_idx, module) in program.modules.iter().enumerate() {
        ctx.current_module_idx = module_idx;
        for item in &module.items {
            match &item.node {
                Item::Struct(sd) => {
                    let field_names: Vec<String> =
                        sd.fields.iter().map(|f| f.name.clone()).collect();
                    ctx.struct_fields.insert(sd.name.clone(), field_names);
                }
                Item::Enum(ed) => {
                    let variants: Vec<String> =
                        ed.variants.iter().map(|v| v.name.clone()).collect();
                    for (idx, vname) in variants.iter().enumerate() {
                        ctx.variant_index.insert(
                            vname.clone(),
                            (ed.name.clone(), u32::try_from(idx).unwrap_or(u32::MAX)),
                        );
                    }
                    ctx.enum_variants.insert(ed.name.clone(), variants);
                }
                _ => {}
            }
        }
    }
    // Pass 1a.2 — resolve which enum variants carry a struct payload so
    // `bind_pattern_vars` (Pattern::EnumVariant) can propagate the
    // struct identity onto the post-`EnumPayload` SSA value. Without
    // this, `match e { Bin(p) => p.left }` always reads field slot 0
    // because `value_struct_types[payload_val]` is empty and
    // `FieldAccess` falls back to its placeholder. Parallel to the
    // OutcomeArm propagation in `bind_pattern_vars` ([v0.7.4.3-debt.2]).
    // Requires struct_fields populated by Pass 1a (above) so we can
    // distinguish struct-typed Named payloads from primitives.
    for module in &program.modules {
        let arena = &program.arenas[module.arena_id.0];
        for item in &module.items {
            if let Item::Enum(ed) = &item.node {
                for variant in &ed.variants {
                    if let Some(payload_id) = variant.payload
                        && let TypeExpr::Named(name) = &arena.type_expression(payload_id).node
                        && ctx.struct_fields.contains_key(name)
                    {
                        ctx.variant_payload_struct
                            .insert(variant.name.clone(), name.clone());
                    }
                }
            }
            // Walk each struct's fields to record which carry a named-
            // struct type. `FieldGet` propagates that name onto its dest
            // so chained accesses like `step.state.arena` keep tracking
            // struct identity through every link. Parallel to
            // `variant_payload_struct` above. Requires `struct_fields`
            // populated by Pass 1a so we can confirm a Named ref is a
            // real struct (not a primitive shadowed by the same name).
            if let Item::Struct(sd) = &item.node {
                for field in &sd.fields {
                    if let TypeExpr::Named(name) = &arena.type_expression(field.type_annotation).node
                        && ctx.struct_fields.contains_key(name)
                    {
                        ctx.struct_field_types
                            .insert((sd.name.clone(), field.name.clone()), name.clone());
                    }
                }
            }
        }
    }
    // Pass 1b: assign FuncIds. Struct table is now complete, so
    // `declare_function` correctly identifies struct-returning
    // functions for downstream call-site type propagation.
    for (module_idx, module) in program.modules.iter().enumerate() {
        ctx.current_module_idx = module_idx;
        for item in &module.items {
            if let Item::Function(fd) = &item.node {
                ctx.declare_function(module, fd);
            }
        }
    }
    // Pass 2: lower bodies
    ctx.current_module_idx = 0;
    let mut ir_modules: Vec<IrModule> = Vec::new();
    for (module_idx, module) in program.modules.iter().enumerate() {
        ctx.current_module_idx = module_idx;
        let path = AbsolutePath::new(module.path.clone(), String::new());
        let mut functions = Vec::new();
        for item in &module.items {
            if let Item::Function(fd) = &item.node {
                let arena = &program.arenas[module.arena_id.0];
                if let Some(ir_func) = ctx.lower_function(module, arena, fd) {
                    functions.push(ir_func);
                }
            }
        }
        if !functions.is_empty() {
            ir_modules.push(IrModule { path, functions });
        }
    }
    IrProgram {
        modules: ir_modules,
        constants: ctx.constants,
        // ADR-0012 — populated by the linker (v0.4.8 demo) when a
        // multi-package compile resolves cross-package generic call
        // sites. Single-package builds emit `CallLocal` /
        // `CallCrossModule` only and never need a table.
        witness_tables: Vec::new(),
    }
}

// ── Loop context ────────────────────────────────────────────────────

struct LoopContext {
    /// Block to branch to on `break`.
    break_target: BlockId,
    /// Block to branch to on `continue`.
    continue_target: BlockId,
}

// ── Lowering context ────────────────────────────────────────────────

struct LowerCtx<'a> {
    /// The resolved program being lowered.
    program: &'a ResolvedProgram,
    /// Current module index (set before each module pass).
    current_module_idx: usize,
    /// Shared constant pool.
    constants: ConstantPool,

    // Function registry (pass 1 → pass 2)
    /// Maps `AbsolutePath` → `FuncId`.
    func_table: HashMap<AbsolutePath, FuncId>,

    /// Enum variant table: enum name → ordered variant names. Variant
    /// index = position in this Vec. Populated in pass 1.
    enum_variants: HashMap<String, Vec<String>>,

    /// Reverse index for unqualified variant resolution: variant name →
    /// (enum name, `variant_idx`). When two enums share a variant name
    /// (e.g. `Some` in `Option` and `MaybeInt`), the last-registered wins;
    /// proper resolution still requires the enum name from the type-checker.
    variant_index: HashMap<String, (String, u32)>,

    /// Variant name → struct name when the variant's payload is a named
    /// struct (e.g. `enum Expr { BinaryOp(BinaryOpPayload) }` registers
    /// `BinaryOp → BinaryOpPayload`). Populated in Pass 1a.2 so
    /// `bind_pattern_vars` can propagate struct identity onto the SSA
    /// value bound by `match Variant(p) => ...`, fixing the v0.7.5.1
    /// repro where `p.field` always reads slot 0. Parallel to
    /// `value_outcome_value_struct` (which covers the `~?` /
    /// `OutcomeArm` path).
    variant_payload_struct: HashMap<String, String>,

    /// Struct definitions table: struct name → ordered field names in
    /// declaration order. Field index for a `name.field` access is the
    /// position of `field` in this Vec. Populated in pass 1.
    ///
    /// Fixes the v0.3.5 placeholder TODO (originally `field_name_to_idx`
    /// always returned 0 — only single-field structs worked correctly).
    struct_fields: HashMap<String, Vec<String>>,

    /// (struct name, field name) → field's struct type name when the
    /// field is itself a named user struct. Lets `FieldGet` propagate
    /// struct identity onto its dest so chained accesses like
    /// `step.state.arena` keep resolving the correct field slot.
    /// Surfaced by the v0.7.5.2 parser.tri port — `ParseStep { state,
    /// expr_id }` returned from a function then unwrapped via match-arm
    /// `~+ step =>` correctly tracked `step: ParseStep`, but the
    /// intermediate `step.state` (of type `ParserState`) lost identity
    /// and the next `.arena` field access fell back to slot 0,
    /// triggering E2201 at the VM. Populated in Pass 1a alongside
    /// `struct_fields`.
    struct_field_types: HashMap<(String, String), String>,

    /// Per-function map of which SSA values carry struct payloads,
    /// indexed by struct name. Populated as struct values flow through
    /// `StructLiteral`, function params, calls returning structs, and
    /// pattern destructuring. `FieldAccess` consults this map to find
    /// the correct field index. Reset for each function (same lifecycle
    /// as `value_counter` / `blocks`).
    value_struct_types: HashMap<ValueId, String>,

    /// Per-function map for SSA values that hold an Outcome whose
    /// success-arm payload is a known struct. Distinct from
    /// `value_struct_types` because the outcome itself is not the
    /// struct — only the unwrapped payload is. Lets `~?` / `~:` /
    /// match-arm `OutcomeUnwrapValue` propagate struct tracking onto
    /// the unwrap result so field access through the post-unwrap
    /// value resolves the correct field index per
    /// [v0.7.4.3-debt.2 / WA-2].
    value_outcome_value_struct: HashMap<ValueId, String>,

    /// Function-level return-type registry: `FuncId` → struct name (if
    /// the function returns a `UserStruct`). Populated in pass 1
    /// alongside `func_table`. `CallLocal` / `CallCrossModule` consult
    /// this to propagate struct-typing into the dest `ValueId`.
    func_return_struct: HashMap<FuncId, String>,

    /// Function-level registry for the success-arm struct of functions
    /// declared `-> T~E` / `-> T?~E` where T is a known struct.
    /// Populated in pass 1; consulted at call sites to seed
    /// `value_outcome_value_struct`. Parallel to `func_return_struct`.
    func_return_outcome_value_struct: HashMap<FuncId, String>,

    // Per-function state (reset for each function)
    value_counter: u32,
    block_counter: u32,
    blocks: BTreeMap<BlockId, BasicBlock>,
    current_block: BlockId,
    /// Lexical scope stack. Each `push_scope` creates a new frame.
    scopes: Vec<HashMap<String, ValueId>>,
    /// Loop context stack for break/continue.
    loop_stack: Vec<LoopContext>,
    /// Function parameters: name → `ValueId`.
    params: HashMap<String, ValueId>,
    /// Current function's return type (from signature).
    current_return_type: TypeTag,
    /// Current function's `FuncId`.
    current_func_id: FuncId,
}

impl<'a> LowerCtx<'a> {
    fn new(program: &'a ResolvedProgram) -> Self {
        Self {
            program,
            current_module_idx: 0,
            constants: ConstantPool::new(),
            func_table: HashMap::new(),
            enum_variants: HashMap::new(),
            variant_index: HashMap::new(),
            variant_payload_struct: HashMap::new(),
            struct_fields: HashMap::new(),
            struct_field_types: HashMap::new(),
            value_struct_types: HashMap::new(),
            value_outcome_value_struct: HashMap::new(),
            func_return_struct: HashMap::new(),
            func_return_outcome_value_struct: HashMap::new(),
            value_counter: 0,
            block_counter: 0,
            blocks: BTreeMap::new(),
            current_block: BlockId(0),
            scopes: Vec::new(),
            loop_stack: Vec::new(),
            params: HashMap::new(),
            current_return_type: TypeTag::Unit,
            current_func_id: FuncId(0),
        }
    }

    // ── Helpers ─────────────────────────────────────────────────

    const fn fresh_value(&mut self) -> ValueId {
        let id = ValueId(self.value_counter);
        self.value_counter += 1;
        id
    }

    const fn fresh_block(&mut self) -> BlockId {
        let id = BlockId(self.block_counter);
        self.block_counter += 1;
        id
    }

    fn intern_constant(&mut self, c: Constant) -> ConstId {
        self.constants.intern(c)
    }

    /// Emit an instruction into the current block.
    fn emit(&mut self, instr: Instruction) {
        self.blocks
            .entry(self.current_block)
            .or_insert_with(|| BasicBlock::new(self.current_block, None))
            .instructions
            .push(instr);
    }

    /// Create a new block and switch to it. The previous block MUST have
    /// been terminated.
    fn start_block(&mut self, id: BlockId, name: Option<String>) {
        self.blocks.insert(id, BasicBlock::new(id, name));
        self.current_block = id;
    }

    /// Push a new lexical scope frame.
    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// Pop the innermost scope frame.
    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Look up a variable name in the scope stack (innermost first).
    fn resolve_var(&self, name: &str) -> Option<ValueId> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(*v);
            }
        }
        self.params.get(name).copied()
    }

    /// Bind a name in the current innermost scope.
    fn bind_var(&mut self, name: String, value: ValueId) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, value);
        }
    }

    /// Rebind a previously-declared name with a new SSA value. Searches
    /// the scope chain from innermost outward and updates the binding at
    /// the level where it was originally declared — so an `x = ...`
    /// inside `if?` keeps mutating the loop-body `x`, not a shadow that
    /// dies when the if's scope pops. Falls back to the innermost scope
    /// when the name isn't found (e.g. first assignment to a fresh var).
    fn rebind_var(&mut self, name: &str, value: ValueId) {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), value);
                return;
            }
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), value);
        }
    }

    /// Get the current module's arena.
    fn arena(&self) -> &'a Arena {
        let module = &self.program.modules[self.current_module_idx];
        &self.program.arenas[module.arena_id.0]
    }

    /// Get the current module.
    fn current_module(&self) -> &'a Module {
        &self.program.modules[self.current_module_idx]
    }

    /// Walk a block and return the names of every variable mutated by
    /// `Stmt::Assign`. Used by the loop lowerer to know which bindings
    /// need phi-node plumbing at the loop header.
    ///
    /// Recurses into nested `if`/`while`/`for`/`match` arms inside the
    /// loop body so an assignment guarded by a conditional still gets a
    /// phi (e.g. `while? cond { if? p { x = ... } }`).
    ///
    /// Returned names preserve first-write order and contain no duplicates.
    fn collect_assigned_vars(&self, block: &Block) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        self.walk_block_for_assigns(block, &mut out);
        out
    }

    fn walk_block_for_assigns(&self, block: &Block, out: &mut Vec<String>) {
        for stmt_id in &block.statements {
            let stmt = &self.arena().statement(*stmt_id).node;
            if std::env::var("TRIET_DEBUG_LOOP").is_ok() {
                eprintln!("[DEBUG]   stmt: {stmt:?}");
            }
            self.walk_stmt_for_assigns(stmt, out);
        }
        if let Some(final_expr) = block.final_expression {
            self.walk_expr_for_assigns(final_expr, out);
        }
    }

    fn walk_stmt_for_assigns(&self, stmt: &Stmt, out: &mut Vec<String>) {
        match stmt {
            Stmt::Assign { target, value } => {
                if !out.iter().any(|n| n == target) {
                    out.push(target.clone());
                }
                self.walk_expr_for_assigns(*value, out);
            }
            Stmt::Let { value, .. } | Stmt::Const { value, .. } | Stmt::ExprStmt(value) => {
                self.walk_expr_for_assigns(*value, out);
            }
            Stmt::Return(Some(v)) | Stmt::Break(Some(v)) => self.walk_expr_for_assigns(*v, out),
            Stmt::Return(None) | Stmt::Break(None) | Stmt::Continue => {}
            Stmt::While {
                condition, body, ..
            } => {
                self.walk_expr_for_assigns(*condition, out);
                self.walk_block_for_assigns(body, out);
            }
            Stmt::For { iterable, body, .. } => {
                self.walk_expr_for_assigns(*iterable, out);
                self.walk_block_for_assigns(body, out);
            }
            Stmt::Loop(body) => self.walk_block_for_assigns(body, out),
        }
    }

    fn walk_expr_for_assigns(&self, expr_id: triet_syntax::arena::ExprId, out: &mut Vec<String>) {
        let expr = &self.arena().expression(expr_id).node;
        match expr {
            Expr::If {
                then_branch,
                else_branch,
                ..
            } => {
                self.walk_block_for_assigns(then_branch, out);
                if let Some(eb) = else_branch.as_ref() {
                    self.walk_block_for_assigns(eb, out);
                }
            }
            Expr::Match { arms, .. } => {
                for arm in arms {
                    self.walk_expr_for_assigns(arm.body, out);
                }
            }
            Expr::Block(b) => self.walk_block_for_assigns(b, out),
            _ => {}
        }
    }

    // ── Function registry ───────────────────────────────────────

    fn declare_function(&mut self, module: &Module, fd: &FunctionDef) {
        let path = AbsolutePath::new(module.path.clone(), fd.name.clone());
        let id = FuncId(self.func_table.len() as u32);
        self.func_table.insert(path, id);
        // Track which functions return a user-defined struct so call
        // sites can propagate struct-typing into the dest `ValueId`
        // (needed for `FieldAccess` resolution per ADR-0007 §field).
        if let Some(rt) = fd.return_type {
            if let Some(struct_name) = self.type_expr_to_struct_name(rt, module) {
                self.func_return_struct.insert(id, struct_name);
            }
            // v0.7.4.3-debt.2 (WA-2): functions returning `T~E` /
            // `T?~E` where T is a known struct also need tracking so
            // the success-arm unwrap (~? / ~: / match-arm) can
            // propagate the struct identity onto the unwrapped value.
            if let Some(struct_name) = self.outcome_value_struct_name(rt, module) {
                self.func_return_outcome_value_struct
                    .insert(id, struct_name);
            }
        }
    }

    /// Peer into a `TypeExpr::Outcome { value_type, .. }` and return
    /// the success-arm struct name when `value_type` is a known
    /// user-struct. Returns `None` for any other shape (non-Outcome,
    /// Outcome wrapping a primitive, etc.). Mirrors
    /// `type_expr_to_struct_name` but unwraps one Outcome layer
    /// first. Per [v0.7.4.3-debt.2 / WA-2].
    fn outcome_value_struct_name(
        &self,
        type_id: triet_syntax::arena::TypeId,
        module: &Module,
    ) -> Option<String> {
        let arena = &self.program.arenas[module.arena_id.0];
        let type_expr = &arena.type_expression(type_id).node;
        if let triet_syntax::type_ast::TypeExpr::Outcome { value_type, .. } = type_expr {
            return self.type_expr_to_struct_name(*value_type, module);
        }
        None
    }

    /// Look up the field index for `field_name` on a `ValueId` known
    /// to carry a struct payload. Falls back to `0` for values whose
    /// struct type isn't tracked (e.g. struct flowed through a path
    /// the lowerer doesn't yet propagate through, or the struct's
    /// definition isn't in scope) — preserves legacy single-field
    /// behavior so we don't regress existing tests.
    fn resolve_struct_field_idx(&self, value: ValueId, field_name: &str) -> u32 {
        let Some(struct_name) = self.value_struct_types.get(&value) else {
            return 0;
        };
        let Some(fields) = self.struct_fields.get(struct_name) else {
            return 0;
        };
        fields
            .iter()
            .position(|n| n == field_name)
            .and_then(|i| u32::try_from(i).ok())
            .unwrap_or(0)
    }

    /// Resolve a `TypeId` to its struct name if it refers to a known
    /// user-defined struct. Returns `None` for primitive types,
    /// nullables, generics, etc. Pass 1 is permitted to call this
    /// during function declaration since `struct_fields` is registered
    /// alongside (the iteration order across `Item`s within a module
    /// may interleave structs and functions, but both register first
    /// before any function bodies are lowered).
    fn type_expr_to_struct_name(
        &self,
        type_id: triet_syntax::arena::TypeId,
        module: &Module,
    ) -> Option<String> {
        let arena = &self.program.arenas[module.arena_id.0];
        let type_expr = &arena.type_expression(type_id).node;
        if let triet_syntax::type_ast::TypeExpr::Named(name) = type_expr
            && self.struct_fields.contains_key(name)
        {
            return Some(name.clone());
        }
        None
    }

    fn resolve_func(&self, name: &str) -> Option<FuncId> {
        let module = self.current_module();
        // Check module bindings for the full path.
        if let Some(abs_path) = module.bindings.get(name) {
            return self.func_table.get(abs_path).copied();
        }
        // Try as local function: current_module.path.name
        let local_path = AbsolutePath::new(module.path.clone(), name.to_owned());
        self.func_table.get(&local_path).copied()
    }

    // ── Type inference helpers ──────────────────────────────────

    const fn suffix_type_tag(suffix: Option<NumericSuffix>) -> TypeTag {
        match suffix {
            Some(NumericSuffix::Trit) => TypeTag::Trit,
            Some(NumericSuffix::Tryte) => TypeTag::Tryte,
            Some(NumericSuffix::Integer) | None => TypeTag::Integer,
            Some(NumericSuffix::Long) => TypeTag::Long,
        }
    }

    fn type_expr_to_tag(&self, type_id: triet_syntax::arena::TypeId) -> TypeTag {
        let type_expr = &self.arena().type_expression(type_id).node;
        match type_expr {
            triet_syntax::type_ast::TypeExpr::Named(name) => match name.as_str() {
                "Trit" => TypeTag::Trit,
                "Tryte" => TypeTag::Tryte,
                "Integer" => TypeTag::Integer,
                "Long" => TypeTag::Long,
                "Trilean" => TypeTag::Trilean,
                "String" => TypeTag::String,
                "Unit" => TypeTag::Unit,
                _ => TypeTag::Unit, // user-defined type OR generic type param — placeholder
            },
            triet_syntax::type_ast::TypeExpr::Nullable(inner) => {
                TypeTag::Nullable(Box::new(self.type_expr_to_tag(*inner)))
            }
            // Generic types — `Vector<T>`, `HashMap<K, V>` (v0.7.4.1
            // via ADR-0019 Addendum §A7 stdlib stub support). Other
            // generic user types (e.g. `Option<Integer>`) erase to
            // Unit per existing user-defined-type placeholder rule.
            triet_syntax::type_ast::TypeExpr::Generic { name, arguments } => {
                match (name.as_str(), arguments.as_slice()) {
                    ("Vector", [element]) => {
                        TypeTag::Vector(Box::new(self.type_expr_to_tag(*element)))
                    }
                    ("HashMap", [key, value]) => TypeTag::HashMap(
                        Box::new(self.type_expr_to_tag(*key)),
                        Box::new(self.type_expr_to_tag(*value)),
                    ),
                    _ => TypeTag::Unit,
                }
            }
            _ => TypeTag::Unit,
        }
    }

    // ── Function lowering ───────────────────────────────────────

    fn lower_function(
        &mut self,
        module: &Module,
        _arena: &Arena,
        fd: &FunctionDef,
    ) -> Option<Function> {
        let path = AbsolutePath::new(module.path.clone(), fd.name.clone());
        let func_id = *self.func_table.get(&path)?;

        // Reset per-function state.
        self.value_counter = 0;
        self.block_counter = 0;
        self.blocks = BTreeMap::new();
        self.scopes = Vec::new();
        self.loop_stack = Vec::new();
        self.params = HashMap::new();
        self.value_struct_types = HashMap::new();
        self.value_outcome_value_struct = HashMap::new();
        self.current_func_id = func_id;
        self.current_return_type = fd
            .return_type
            .map_or(TypeTag::Unit, |t| self.type_expr_to_tag(t));

        // Push the outermost function scope.
        self.push_scope();

        // Entry block.
        let entry_id = self.fresh_block();
        self.start_block(entry_id, Some("entry".into()));

        // Allocate parameter ValueIds. Record struct-typed parameters in
        // `value_struct_types` so `FieldAccess` inside the function body
        // can resolve `param.field` to the correct field index.
        let mut param_specs: Vec<(String, TypeTag)> = Vec::new();
        for p in &fd.parameters {
            let v = self.fresh_value();
            let pty = self.type_expr_to_tag(p.type_annotation);
            if let Some(struct_name) = self.type_expr_to_struct_name(p.type_annotation, module) {
                self.value_struct_types.insert(v, struct_name);
            }
            self.params.insert(p.name.clone(), v);
            self.bind_var(p.name.clone(), v);
            param_specs.push((p.name.clone(), pty));
        }

        // Lower the function body.
        let block_result = match &fd.body {
            FunctionBody::Block(block) => Some(self.lower_block(block)),
            FunctionBody::Expression(expr_id) => Some(self.lower_expr(*expr_id)),
        };

        // If the current block doesn't have a terminator yet, emit Ret.
        // (The body may have already emitted Ret/Return via an explicit
        // `return` statement.)
        if let Some(v) = block_result {
            if self.blocks[&self.current_block].terminator().is_none() {
                self.emit(Instruction::Ret {
                    value: Some(Operand::Value(v)),
                });
            }
        } else if self.blocks[&self.current_block].terminator().is_none() {
            self.emit(Instruction::Ret { value: None });
        }

        self.pop_scope();

        let mut func = Function::new(
            func_id,
            Some(fd.name.clone()),
            param_specs,
            self.current_return_type.clone(),
        );
        func.blocks = std::mem::take(&mut self.blocks).into_values().collect();
        Some(func)
    }

    // ── Statement lowering ──────────────────────────────────────

    fn lower_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let {
                name,
                mutable: _mutable,
                type_annotation: _ty,
                value,
            } => {
                let val = self.lower_expr(*value);
                self.bind_var(name.clone(), val);
            }

            Stmt::Assign { target, value } => {
                let new_val = self.lower_expr(*value);
                // Rebind at the scope where the var was originally
                // declared, not the innermost (so assignments inside
                // `if?` / `match` arms actually mutate the loop-body or
                // outer binding instead of dying with the inner scope).
                self.rebind_var(target, new_val);
            }

            Stmt::Const {
                name,
                type_annotation: _ty,
                value,
            } => {
                let val = self.lower_expr(*value);
                self.bind_var(name.clone(), val);
            }

            Stmt::Return(opt_expr) => {
                let val = opt_expr.map(|e| self.lower_expr(e));
                self.emit(Instruction::Ret {
                    value: val.map(Operand::Value),
                });
            }

            Stmt::Break(opt_expr) => {
                let val = opt_expr.map(|e| self.lower_expr(e));
                if let Some(ctx) = self.loop_stack.last() {
                    // TODO: pass break value via phi at target
                    let _ = val;
                    self.emit(Instruction::Br {
                        target: ctx.break_target,
                    });
                }
                // else: outside loop — typechecker catches this.
            }

            Stmt::Continue => {
                if let Some(ctx) = self.loop_stack.last() {
                    self.emit(Instruction::Br {
                        target: ctx.continue_target,
                    });
                }
            }

            Stmt::For {
                variable: pat,
                iterable,
                body,
            } => {
                let pattern = &self.arena().pattern(*pat).node;
                self.lower_for_loop(pattern, *iterable, body);
            }

            Stmt::While {
                condition,
                body,
                treat_unknown_as_false,
            } => {
                self.lower_while_loop(*condition, body, *treat_unknown_as_false);
            }

            Stmt::Loop(body) => {
                self.lower_loop_stmt(body);
            }

            Stmt::ExprStmt(expr_id) => {
                // Evaluate expression, discard result.
                self.lower_expr(*expr_id);
            }
        }
    }

    // ── Expression lowering ─────────────────────────────────────
    ///
    /// Lower an expression and return the `ValueId` holding its result.
    fn lower_expr(&mut self, expr_id: triet_syntax::arena::ExprId) -> ValueId {
        let spanned = &self.arena().expression(expr_id);
        let expr = &spanned.node;
        match expr {
            // ── Literals ──────────────────────────────────────────
            Expr::IntegerLiteral { value, suffix } => {
                let tag = Self::suffix_type_tag(*suffix);
                let const_id = self.intern_constant(match tag {
                    TypeTag::Trit => {
                        let t = Trit::from_i8(*value as i8).unwrap_or(Trit::Zero);
                        Constant::Trit(t)
                    }
                    TypeTag::Tryte => {
                        let i16_val = *value as i16;
                        let tryte_val = Tryte::new(i16_val).unwrap_or(Tryte::ZERO);
                        Constant::Tryte(tryte_val)
                    }
                    TypeTag::Integer => {
                        let i64_val = i64::try_from(*value).unwrap_or(0);
                        Constant::Integer(Integer::new(i64_val).unwrap())
                    }
                    TypeTag::Long => Constant::Long(Long::from_i128(*value)),
                    _ => {
                        let i64_val = i64::try_from(*value).unwrap_or(0);
                        Constant::Integer(Integer::new(i64_val).unwrap())
                    }
                });
                let dest = self.fresh_value();
                self.emit(Instruction::Const {
                    dest,
                    constant: const_id,
                });
                dest
            }

            Expr::TernaryLiteral { value } => {
                let i64_val = i64::try_from(*value).unwrap_or(0);
                let const_id =
                    self.intern_constant(Constant::Integer(Integer::new(i64_val).unwrap()));
                let dest = self.fresh_value();
                self.emit(Instruction::Const {
                    dest,
                    constant: const_id,
                });
                dest
            }

            Expr::TrileanLiteral(tv) => {
                let tl = match tv {
                    TrileanValue::True => Trilean::True,
                    TrileanValue::False => Trilean::False,
                    TrileanValue::Unknown => Trilean::Unknown,
                };
                let const_id = self.intern_constant(Constant::Trilean(tl));
                let dest = self.fresh_value();
                self.emit(Instruction::Const {
                    dest,
                    constant: const_id,
                });
                dest
            }

            Expr::StringLiteral(s) => {
                let const_id = self.intern_constant(Constant::String(s.clone()));
                let dest = self.fresh_value();
                self.emit(Instruction::Const {
                    dest,
                    constant: const_id,
                });
                dest
            }

            Expr::NullLiteral => {
                // Materialize a runtime `Null` marker so `NullCheck` can
                // distinguish it from a genuine `Unit` expression result.
                let const_id = self.intern_constant(Constant::Null);
                let dest = self.fresh_value();
                self.emit(Instruction::Const {
                    dest,
                    constant: const_id,
                });
                dest
            }

            Expr::FStringLiteral(fstr) => {
                // Lower f-string by collecting all parts (text as Const String,
                // interpolations as lowered expressions) and concatenating via
                // the FStringConcat builtin.
                let mut parts: Vec<Operand> = Vec::new();
                for part in &fstr.parts {
                    match part {
                        triet_syntax::expr::FStringPart::Text(s) => {
                            let c = self.intern_constant(Constant::String(s.clone()));
                            let d = self.fresh_value();
                            self.emit(Instruction::Const {
                                dest: d,
                                constant: c,
                            });
                            parts.push(Operand::Value(d));
                        }
                        triet_syntax::expr::FStringPart::Interpolation {
                            expression,
                            format_spec: _fs,
                        } => {
                            let val = self.lower_expr(*expression);
                            parts.push(Operand::Value(val));
                        }
                    }
                }
                let dest = self.fresh_value();
                self.emit(Instruction::CallBuiltin {
                    dest: Some(dest),
                    name: BuiltinName::FStringConcat,
                    args: parts,
                });
                dest
            }

            // ── Variables ─────────────────────────────────────────
            Expr::Identifier(name) => {
                // Function parameters (shadowed by local vars).
                if let Some(v) = self.resolve_var(name) {
                    return v;
                }
                // Bare identifier referring to a unit enum variant (e.g.
                // `None` for `enum MaybeInt { Some(Integer), None }`).
                // The parser leaves these as `Expr::Identifier`; the
                // type-checker resolves variant identity but the AST node
                // itself is not rewritten. We rebuild the enum value here.
                if let Some((_enum_name, variant_idx)) = self.variant_index.get(name).cloned() {
                    let dest = self.fresh_value();
                    self.emit(Instruction::EnumNew {
                        dest,
                        variant_idx,
                        payload: None,
                    });
                    return dest;
                }
                // Look up as a function reference — defer to call sites.
                // For now, return a placeholder.
                // This should have been caught by typecheck; for correct IR
                // the identifier should only appear in Call callee position.

                self.fresh_value()
            }

            Expr::FieldAccess { object, field } => {
                let obj_val = self.lower_expr(*object);
                let field_idx = self.resolve_struct_field_idx(obj_val, field);
                let dest = self.fresh_value();
                self.emit(Instruction::FieldGet {
                    dest,
                    object: Operand::Value(obj_val),
                    field_idx,
                });
                // v0.7.5.2: propagate struct identity onto the FieldGet
                // dest when the accessed field is itself a named struct.
                // Without this, chained accesses like `step.state.arena`
                // lose track at the intermediate `step.state` value.
                if let Some(obj_struct) = self.value_struct_types.get(&obj_val).cloned()
                    && let Some(field_struct) = self
                        .struct_field_types
                        .get(&(obj_struct, field.clone()))
                        .cloned()
                {
                    self.value_struct_types.insert(dest, field_struct);
                }
                dest
            }

            Expr::TupleIndex { tuple, index } => {
                let tup_val = self.lower_expr(*tuple);
                let dest = self.fresh_value();
                self.emit(Instruction::FieldGet {
                    dest,
                    object: Operand::Value(tup_val),
                    field_idx: *index as u32,
                });
                dest
            }

            // ── Calls ─────────────────────────────────────────────
            Expr::Call { callee, arguments } => self.lower_call(*callee, arguments),

            Expr::MethodCall {
                receiver,
                method,
                arguments,
            } => {
                // Specific method dispatch — covers the v0.2 stdlib surface
                // (no full method-table lookup yet; see `lower_for_loop` for
                // the `.enumerate()` adapter handling).
                if method == "length" && arguments.is_empty() {
                    let receiver_val = self.lower_expr(*receiver);
                    let dest = self.fresh_value();
                    self.emit(Instruction::CallBuiltin {
                        dest: Some(dest),
                        name: BuiltinName::TextLen,
                        args: vec![Operand::Value(receiver_val)],
                    });
                    return dest;
                }
                // Outcome unwrap methods (ADR-0020 §3 — verbose strict
                // unwrap pairs). The message argument is lowered for
                // side-effect parity with the source contract but is
                // not consumed by the VM tier; the runtime E2210 panic
                // carries a hardcoded label. Source-level message
                // satisfies `feedback_explicit_strictness`.
                if (method == "unwrap_value" || method == "unwrap_error") && arguments.len() == 1 {
                    let receiver_val = self.lower_expr(*receiver);
                    let _msg_val = self.lower_expr(arguments[0]);
                    let dest = self.fresh_value();
                    let instr = if method == "unwrap_value" {
                        Instruction::OutcomeUnwrapValue {
                            dest,
                            source: Operand::Value(receiver_val),
                        }
                    } else {
                        Instruction::OutcomeUnwrapError {
                            dest,
                            source: Operand::Value(receiver_val),
                        }
                    };
                    self.emit(instr);
                    return dest;
                }
                self.lower_expr(*receiver)
            }

            // ── Arithmetic ───────────────────────────────────────
            Expr::BinaryOp {
                operator,
                left,
                right,
            } => {
                let lhs = self.lower_expr(*left);
                let rhs = self.lower_expr(*right);
                let dest = self.fresh_value();
                let instr = Self::lower_binary_op(*operator, dest, lhs, rhs);
                self.emit(instr);
                dest
            }

            Expr::UnaryOp {
                operator: _op,
                operand,
            } => {
                let val = self.lower_expr(*operand);
                let dest = self.fresh_value();
                self.emit(Instruction::Neg {
                    dest,
                    operand: Operand::Value(val),
                });
                dest
            }

            // ── Nullable ops ──────────────────────────────────────
            Expr::SafeFieldAccess { object, field } => {
                let obj_val = self.lower_expr(*object);
                // %check = null_check %obj
                let check = self.fresh_value();
                self.emit(Instruction::NullCheck {
                    dest: check,
                    nullable: Operand::Value(obj_val),
                });
                // If non-null, unwrap and access.
                let unwrapped = self.fresh_value();
                self.emit(Instruction::NullUnwrap {
                    dest: unwrapped,
                    nullable: Operand::Value(obj_val),
                });
                // The unwrapped value carries the same struct typing as
                // the original (Nullable peels off cleanly). Look up the
                // field index via the original object — `value_struct_types`
                // entry was populated when the struct was constructed.
                let field_idx = self.resolve_struct_field_idx(obj_val, field);
                let dest = self.fresh_value();
                self.emit(Instruction::FieldGet {
                    dest,
                    object: Operand::Value(unwrapped),
                    field_idx,
                });
                dest
            }

            Expr::SafeMethodCall {
                receiver,
                method,
                arguments,
            } => {
                // `recv?.length()` — null check + branch:
                // null → null; non-null → method on unwrapped value.
                let receiver_val = self.lower_expr(*receiver);
                let null_check = self.fresh_value();
                self.emit(Instruction::NullCheck {
                    dest: null_check,
                    nullable: Operand::Value(receiver_val),
                });
                let then_id = self.fresh_block();
                let else_id = self.fresh_block();
                let merge_id = self.fresh_block();
                // ADR-0010: NullCheck returns a Trit (Positive=non-null,
                // Zero=null, Negative=reserved-definitely-missing). Map
                // via Trilean for ternary-native dispatch:
                //   True (non-null) → then
                //   Unknown (null)  → else (propagate null)
                //   False (reserved) → else (also propagate, conservative)
                self.emit(Instruction::BrTrilean {
                    cond: Operand::Value(null_check),
                    true_block: then_id,
                    unknown_block: else_id,
                    false_block: else_id,
                });
                // `then`: non-null path. Unwrap and apply the method.
                self.start_block(then_id, Some("safe_method_some".into()));
                let unwrapped = self.fresh_value();
                self.emit(Instruction::NullUnwrap {
                    dest: unwrapped,
                    nullable: Operand::Value(receiver_val),
                });
                let then_val = if method == "length" && arguments.is_empty() {
                    let d = self.fresh_value();
                    self.emit(Instruction::CallBuiltin {
                        dest: Some(d),
                        name: BuiltinName::TextLen,
                        args: vec![Operand::Value(unwrapped)],
                    });
                    d
                } else {
                    unwrapped
                };
                self.emit(Instruction::Br { target: merge_id });
                let then_end = self.current_block;
                // `else`: null path — re-use the receiver's null value as
                // the result of the chain (its formatted display is `null`).
                self.start_block(else_id, Some("safe_method_none".into()));
                self.emit(Instruction::Br { target: merge_id });
                let else_end = self.current_block;
                let else_val = receiver_val;
                // Merge.
                self.start_block(merge_id, Some("safe_method_merge".into()));
                let merge_dest = self.fresh_value();
                self.emit(Instruction::Phi {
                    dest: merge_dest,
                    incoming: vec![
                        PhiIncoming {
                            value: then_val,
                            block: then_end,
                        },
                        PhiIncoming {
                            value: else_val,
                            block: else_end,
                        },
                    ],
                });
                merge_dest
            }

            Expr::ElvisOp { object, default } => {
                // `obj ?: default` — null check + branch:
                // null → default; non-null → unwrapped.
                let obj_val = self.lower_expr(*object);
                let null_check = self.fresh_value();
                self.emit(Instruction::NullCheck {
                    dest: null_check,
                    nullable: Operand::Value(obj_val),
                });
                let then_id = self.fresh_block();
                let else_id = self.fresh_block();
                let merge_id = self.fresh_block();
                // ADR-0010: ternary-native — null (Trit::Zero) maps to
                // Trilean::Unknown which routes to else (use default).
                self.emit(Instruction::BrTrilean {
                    cond: Operand::Value(null_check),
                    true_block: then_id,
                    unknown_block: else_id,
                    false_block: else_id,
                });
                self.start_block(then_id, Some("elvis_some".into()));
                let unwrapped = self.fresh_value();
                self.emit(Instruction::NullUnwrap {
                    dest: unwrapped,
                    nullable: Operand::Value(obj_val),
                });
                self.emit(Instruction::Br { target: merge_id });
                let then_end = self.current_block;
                self.start_block(else_id, Some("elvis_default".into()));
                let default_val = self.lower_expr(*default);
                self.emit(Instruction::Br { target: merge_id });
                let else_end = self.current_block;
                self.start_block(merge_id, Some("elvis_merge".into()));
                let merge_dest = self.fresh_value();
                self.emit(Instruction::Phi {
                    dest: merge_dest,
                    incoming: vec![
                        PhiIncoming {
                            value: unwrapped,
                            block: then_end,
                        },
                        PhiIncoming {
                            value: default_val,
                            block: else_end,
                        },
                    ],
                });
                merge_dest
            }

            Expr::ForceUnwrap(inner) => {
                let val = self.lower_expr(*inner);
                let dest = self.fresh_value();
                self.emit(Instruction::NullUnwrap {
                    dest,
                    nullable: Operand::Value(val),
                });
                dest
            }

            // ── Control flow expressions ──────────────────────────
            Expr::If {
                condition,
                then_branch,
                else_branch,
                treat_unknown_as_false,
            } => self.lower_if_expr(
                *condition,
                then_branch,
                else_branch.as_ref(),
                *treat_unknown_as_false,
            ),

            Expr::Match { scrutinee, arms } => self.lower_match_expr(*scrutinee, arms),

            Expr::Block(block) => {
                self.push_scope();
                let result = self.lower_block(block);
                self.pop_scope();
                result
            }

            // ── Composite ─────────────────────────────────────────
            Expr::Tuple(elements) => {
                let fields: Vec<Operand> = elements
                    .iter()
                    .map(|e| Operand::Value(self.lower_expr(*e)))
                    .collect();
                let dest = self.fresh_value();
                self.emit(Instruction::StructNew { dest, fields });
                dest
            }

            Expr::Lambda {
                parameters: _params,
                return_type: _rt,
                body: _body,
            } => {
                // Lambdas are lowered as separate synthetic functions.
                // Deferred to v0.3.4.

                self.fresh_value()
            }

            Expr::Range {
                start: _start,
                end: _end,
                inclusive: _inclusive,
            } => {
                // Range lowered as struct { start, end, inclusive }.

                self.fresh_value()
            }

            Expr::StructLiteral { name, fields } => {
                // Lower each field's value expression FIRST (preserves
                // source-order side effects), then reorder according to
                // the struct's declared field order so `FieldGet { idx }`
                // sees a consistent layout across literal sites.
                //
                // Pre-fix (v0.3.5 placeholder era): fields emitted in
                // literal order. Two literals of the same struct could
                // produce divergent layouts. This is the layout bug
                // surfaced by `v0.7.4.3-error.5` capstone authoring.
                let mut by_name: std::collections::HashMap<&str, Operand> =
                    std::collections::HashMap::new();
                for (field_name, expr_id) in fields {
                    let value = self.lower_expr(*expr_id);
                    by_name.insert(field_name.as_str(), Operand::Value(value));
                }
                let ordered = self.struct_fields.get(name).cloned().unwrap_or_else(|| {
                    // Unknown struct (recovery path): fall back to
                    // literal order so existing tests that omit the
                    // struct declaration still typecheck-and-erase.
                    fields.iter().map(|(n, _)| n.clone()).collect()
                });
                let field_values: Vec<Operand> = ordered
                    .iter()
                    .filter_map(|field_name| by_name.get(field_name.as_str()).copied())
                    .collect();
                let dest = self.fresh_value();
                self.emit(Instruction::StructNew {
                    dest,
                    fields: field_values,
                });
                // Track which struct the dest carries so downstream
                // `FieldAccess` can resolve `name.field` to the right
                // index in the declared field list.
                self.value_struct_types.insert(dest, name.clone());
                dest
            }

            Expr::EnumLiteral {
                name,
                variant_name,
                payload,
            } => {
                // Resolve the variant index. Prefer enum-qualified lookup
                // (parser may leave `name` empty when the literal comes from
                // a bare `Some(x)` / `None` — fall back to the reverse index
                // by variant name in that case).
                let variant_idx = self
                    .enum_variants
                    .get(name)
                    .and_then(|vs| vs.iter().position(|v| v == variant_name))
                    .or_else(|| {
                        self.variant_index
                            .get(variant_name)
                            .map(|(_, i)| *i as usize)
                    })
                    .map_or(0, |i| u32::try_from(i).unwrap_or(0));
                let payload_op = payload.map(|e| Operand::Value(self.lower_expr(e)));
                let dest = self.fresh_value();
                self.emit(Instruction::EnumNew {
                    dest,
                    variant_idx,
                    payload: payload_op,
                });
                dest
            }

            // v0.7.4.3-error.3b (ADR-0020): full lowering for the
            // three Outcome AST shapes. Constructors map 1:1 to the
            // 0xC1-0xC3 opcodes; default / propagate emit three-way
            // BrTrilean dispatch over the discriminator.
            Expr::OutcomeConstructor { arm, payload } => {
                self.lower_outcome_constructor(*arm, *payload)
            }
            Expr::OutcomeDefault { inner, default } => self.lower_outcome_default(*inner, *default),
            Expr::OutcomePropagate {
                inner,
                capture_name,
                early_return,
            } => self.lower_outcome_propagate(*inner, capture_name.as_deref(), *early_return),
        }
    }

    // ── Outcome lowering (v0.7.4.3-error.3b, ADR-0020) ───────────

    /// Lower `~+ value` / `~0` / `~- error` to one of the three
    /// outcome constructor opcodes. Type-erased — the surrounding
    /// typecheck context determines which arm is valid (E1024 etc.).
    fn lower_outcome_constructor(
        &mut self,
        arm: OutcomeArm,
        payload: Option<triet_syntax::arena::ExprId>,
    ) -> ValueId {
        let dest = self.fresh_value();
        match arm {
            OutcomeArm::Positive => {
                let payload_val =
                    self.lower_expr(payload.expect("typecheck guarantees ~+ has a payload"));
                self.emit(Instruction::OutcomeNewPositive {
                    dest,
                    payload: Operand::Value(payload_val),
                });
            }
            OutcomeArm::Negative => {
                let payload_val =
                    self.lower_expr(payload.expect("typecheck guarantees ~- has a payload"));
                self.emit(Instruction::OutcomeNewNegative {
                    dest,
                    payload: Operand::Value(payload_val),
                });
            }
            OutcomeArm::Zero => {
                // ADR-0010 Addendum §D (v0.7.4.3-error.6a): source `~0`
                // and source `null` produce byte-identical IR — both
                // emit Constant::Null. The OutcomeNewNull opcode (0xC3)
                // is retained for backward `.triv` compat + dynamic-
                // construction paths but is no longer the source-level
                // target for `~0`. Cross-tolerant VM dispatch (also §D)
                // ensures `OutcomeDiscriminant` and pattern-match arms
                // continue to recognize this value as the Zero state.
                let null_const = self.intern_constant(Constant::Null);
                self.emit(Instruction::Const {
                    dest,
                    constant: null_const,
                });
            }
        }
        dest
    }

    /// Lower `inner ~: default`. Reads the discriminator; branches
    /// three ways. Success → `unwrap_value` flows into the merge block;
    /// failure / null → evaluate `default`. The merge block carries a
    /// phi node selecting whichever path produced the result.
    fn lower_outcome_default(
        &mut self,
        inner: triet_syntax::arena::ExprId,
        default: triet_syntax::arena::ExprId,
    ) -> ValueId {
        let inner_val = self.lower_expr(inner);
        let disc = self.fresh_value();
        self.emit(Instruction::OutcomeDiscriminant {
            dest: disc,
            source: Operand::Value(inner_val),
        });

        let success_block = self.fresh_block();
        let fallback_block = self.fresh_block();
        let merge_block = self.fresh_block();

        // BrTrilean reads the discriminator as: Positive → True,
        // Negative → False, Zero → Unknown. Map success arm to
        // `true_block`; both null and failure arms go to fallback.
        self.emit(Instruction::BrTrilean {
            cond: Operand::Value(disc),
            true_block: success_block,
            unknown_block: fallback_block,
            false_block: fallback_block,
        });

        // Success path — unwrap and continue.
        self.start_block(success_block, None);
        let success_val = self.fresh_value();
        self.emit(Instruction::OutcomeUnwrapValue {
            dest: success_val,
            source: Operand::Value(inner_val),
        });
        // v0.7.4.3-debt.2 (WA-2): propagate success-arm struct identity.
        if let Some(s) = self.value_outcome_value_struct.get(&inner_val).cloned() {
            self.value_struct_types.insert(success_val, s);
        }
        self.emit(Instruction::Br {
            target: merge_block,
        });
        let success_pred = success_block;

        // Fallback path — evaluate default.
        self.start_block(fallback_block, None);
        let fallback_val = self.lower_expr(default);
        self.emit(Instruction::Br {
            target: merge_block,
        });
        let fallback_pred = self.current_block;

        // Merge — phi between the two paths.
        self.start_block(merge_block, None);
        let merged = self.fresh_value();
        self.emit(Instruction::Phi {
            dest: merged,
            incoming: vec![
                PhiIncoming {
                    value: success_val,
                    block: success_pred,
                },
                PhiIncoming {
                    value: fallback_val,
                    block: fallback_pred,
                },
            ],
        });
        // v0.7.4.3-debt.2 (WA-2): if both branches resolve to the same
        // struct identity, the merge inherits it. When the fallback
        // produces an unrelated type (Integer default for a struct
        // success arm), we leave the merge untracked — caller must
        // not access struct fields on a value of mixed type.
        if let (Some(s1), Some(s2)) = (
            self.value_struct_types.get(&success_val).cloned(),
            self.value_struct_types.get(&fallback_val).cloned(),
        ) && s1 == s2
        {
            self.value_struct_types.insert(merged, s1);
        }
        merged
    }

    /// Lower `inner ~? |capture| early_return`.
    ///
    /// Semantics per ADR-0020 §3.1: on the failure arm, the
    /// `early_return` expression is **divergent** — typecheck E1031
    /// enforces it must be a `return` / panic / re-propagate. The
    /// lowerer treats it as such by emitting `Ret <early_return>`
    /// rather than branching into the merge block, so the failure
    /// arm terminates the surrounding function. If the user's
    /// `early_return` is the re-wrap form `~- err`, the resulting
    /// Outcome value is returned directly — matching the source-
    /// level expectation that the outer function is fallible.
    ///
    /// Null arm (only reachable when the inner is `T?~E`) propagates
    /// the null marker through to the merge block; merging with the
    /// success arm's unwrapped value yields a `T?` typed expression.
    /// For binary `T~E` outcomes typecheck guarantees the Zero arm
    /// is statically unreachable, but the IR still emits the block
    /// for verifier well-formedness.
    fn lower_outcome_propagate(
        &mut self,
        inner: triet_syntax::arena::ExprId,
        capture_name: Option<&str>,
        early_return: triet_syntax::arena::ExprId,
    ) -> ValueId {
        let inner_val = self.lower_expr(inner);
        let disc = self.fresh_value();
        self.emit(Instruction::OutcomeDiscriminant {
            dest: disc,
            source: Operand::Value(inner_val),
        });

        let success_block = self.fresh_block();
        let null_block = self.fresh_block();
        let failure_block = self.fresh_block();
        let merge_block = self.fresh_block();

        self.emit(Instruction::BrTrilean {
            cond: Operand::Value(disc),
            true_block: success_block,
            unknown_block: null_block,
            false_block: failure_block,
        });

        // Success path — unwrap and continue to merge.
        self.start_block(success_block, None);
        let success_val = self.fresh_value();
        self.emit(Instruction::OutcomeUnwrapValue {
            dest: success_val,
            source: Operand::Value(inner_val),
        });
        // v0.7.4.3-debt.2 (WA-2): if the inner outcome's success arm
        // was a known struct, propagate that identity onto the
        // unwrapped value so subsequent field access resolves the
        // correct index.
        if let Some(s) = self.value_outcome_value_struct.get(&inner_val).cloned() {
            self.value_struct_types.insert(success_val, s);
        }
        self.emit(Instruction::Br {
            target: merge_block,
        });
        let success_pred = success_block;

        // Null path — propagate null marker through to merge.
        self.start_block(null_block, None);
        let null_const = self.intern_constant(Constant::Null);
        let null_val = self.fresh_value();
        self.emit(Instruction::Const {
            dest: null_val,
            constant: null_const,
        });
        self.emit(Instruction::Br {
            target: merge_block,
        });
        let null_pred = null_block;

        // Failure path — bind capture, evaluate the (divergent)
        // early-return expression, then `Ret` its value so the
        // surrounding function terminates here.
        self.start_block(failure_block, None);
        self.push_scope();
        let captured_payload = self.fresh_value();
        self.emit(Instruction::OutcomeUnwrapError {
            dest: captured_payload,
            source: Operand::Value(inner_val),
        });
        if let Some(name) = capture_name {
            self.bind_var(name.to_owned(), captured_payload);
        }
        let early_return_val = self.lower_expr(early_return);
        self.emit(Instruction::Ret {
            value: Some(Operand::Value(early_return_val)),
        });
        self.pop_scope();

        // Merge — only success + null reach here (failure arm Ret-ed).
        self.start_block(merge_block, None);
        let merged = self.fresh_value();
        self.emit(Instruction::Phi {
            dest: merged,
            incoming: vec![
                PhiIncoming {
                    value: success_val,
                    block: success_pred,
                },
                PhiIncoming {
                    value: null_val,
                    block: null_pred,
                },
            ],
        });
        // v0.7.4.3-debt.2 (WA-2): merged carries the same struct as
        // the success arm. The null branch can never reach a field
        // access without an upstream null-check, so attributing the
        // struct identity is correct on the success-reachable subset.
        if let Some(s) = self.value_struct_types.get(&success_val).cloned() {
            self.value_struct_types.insert(merged, s);
        }
        merged
    }

    // ── Binary operator lowering ─────────────────────────────────

    const fn lower_binary_op(
        op: BinaryOperator,
        dest: ValueId,
        lhs: ValueId,
        rhs: ValueId,
    ) -> Instruction {
        let lo = Operand::Value(lhs);
        let ro = Operand::Value(rhs);
        match op {
            BinaryOperator::Add => Instruction::Add {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::Subtract => Instruction::Sub {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::Multiply => Instruction::Mul {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::Divide => Instruction::Div {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::Modulo => Instruction::Mod {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::Power => Instruction::Pow {
                dest,
                base: lo,
                exp: ro,
            },
            BinaryOperator::Equal => Instruction::Eq {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::NotEqual => Instruction::Ne {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::LessThan => Instruction::Lt {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::LessEqual => Instruction::Le {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::GreaterThan => Instruction::Gt {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::GreaterEqual => Instruction::Ge {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::And => Instruction::LukAnd {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::Or => Instruction::LukOr {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::Xor => Instruction::LukXor {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::Iff => Instruction::LukIff {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::Implies => Instruction::LukImplies {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::KleeneXor => Instruction::KleeneXor {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::KleeneIff => Instruction::KleeneIff {
                dest,
                lhs: lo,
                rhs: ro,
            },
            BinaryOperator::KleeneImplies => Instruction::KleeneImplies {
                dest,
                lhs: lo,
                rhs: ro,
            },
        }
    }

    // ── Control flow lowering ────────────────────────────────────

    fn lower_block(&mut self, block: &Block) -> ValueId {
        for stmt_id in &block.statements {
            let stmt = &self.arena().statement(*stmt_id).node;
            self.lower_stmt(stmt);
        }
        if let Some(final_expr) = block.final_expression {
            self.lower_expr(final_expr)
        } else {
            // Block has no final expression → yields Unit.
            let c = self.intern_constant(Constant::Unit);
            let dest = self.fresh_value();
            self.emit(Instruction::Const { dest, constant: c });
            dest
        }
    }

    fn lower_if_expr(
        &mut self,
        condition: triet_syntax::arena::ExprId,
        then_branch: &Block,
        else_branch: Option<&Block>,
        treat_unknown_as_false: bool,
    ) -> ValueId {
        let cond_val = self.lower_expr(condition);

        // Collect names of every var either branch may mutate so the
        // merge block can phi them together; otherwise the post-if scope
        // sees only the last branch's writes (statically), violating
        // dynamic-execution semantics.
        let mut mutated: Vec<String> = self.collect_assigned_vars(then_branch);
        if let Some(eb) = else_branch {
            for n in self.collect_assigned_vars(eb) {
                if !mutated.iter().any(|x| x == &n) {
                    mutated.push(n);
                }
            }
        }
        // Snapshot the pre-if value of each so we can phi against branches
        // that don't write to a given var.
        let pre_if_vals: Vec<(String, Option<ValueId>)> = mutated
            .iter()
            .map(|n| (n.clone(), self.resolve_var(n)))
            .collect();

        let then_block_id = self.fresh_block();
        let else_block_id = self.fresh_block();
        let merge_block_id = self.fresh_block();

        // ADR-0010: dispatch ternary-native via `BrTrilean`. For `if?`
        // the Unknown trit follows False (treat-as-false). For plain `if`
        // the Unknown trit lands in an `Unreachable` block per SPEC §7.1.1
        // (must panic, not silently take a branch).
        let unknown_block_id = if treat_unknown_as_false {
            else_block_id
        } else {
            self.fresh_block()
        };
        self.emit(Instruction::BrTrilean {
            cond: Operand::Value(cond_val),
            true_block: then_block_id,
            unknown_block: unknown_block_id,
            false_block: else_block_id,
        });
        // For plain `if`, materialise the panic block now (after the
        // BrTrilean is on record, before lowering the then/else paths).
        if !treat_unknown_as_false {
            self.start_block(unknown_block_id, Some("if_unknown_panic".into()));
            self.emit(Instruction::Unreachable);
        }

        // Then block.
        self.start_block(then_block_id, Some("then".into()));
        let then_val = self.lower_block(then_branch);
        // Snapshot the live SSA value for each mutated name AT THE END of
        // the then-branch (this is the value the then-side phi-incoming
        // contributes).
        let then_vals: Vec<Option<ValueId>> = mutated.iter().map(|n| self.resolve_var(n)).collect();
        // Only branch if the block didn't already terminate.
        if self.blocks[&self.current_block].terminator().is_none() {
            self.emit(Instruction::Br {
                target: merge_block_id,
            });
        }
        let then_end = self.current_block;

        // Restore pre-if bindings before lowering the else branch — both
        // branches start from the same dominator state.
        for (name, pre) in &pre_if_vals {
            if let Some(v) = *pre {
                self.rebind_var(name, v);
            }
        }

        // Else block.
        self.start_block(else_block_id, Some("else".into()));
        let else_val = if let Some(eb) = else_branch {
            self.lower_block(eb)
        } else {
            let c = self.intern_constant(Constant::Unit);
            let d = self.fresh_value();
            self.emit(Instruction::Const {
                dest: d,
                constant: c,
            });
            d
        };
        let else_vals: Vec<Option<ValueId>> = mutated.iter().map(|n| self.resolve_var(n)).collect();
        if self.blocks[&self.current_block].terminator().is_none() {
            self.emit(Instruction::Br {
                target: merge_block_id,
            });
        }
        let else_end = self.current_block;

        // Merge block with phi for the if's result value.
        self.start_block(merge_block_id, Some("merge".into()));
        let merge_dest = self.fresh_value();
        self.emit(Instruction::Phi {
            dest: merge_dest,
            incoming: vec![
                PhiIncoming {
                    value: then_val,
                    block: then_end,
                },
                PhiIncoming {
                    value: else_val,
                    block: else_end,
                },
            ],
        });
        // Emit one phi per mutated variable so the post-if scope sees
        // a single merged SSA value for each name. Skip names that the
        // pre-if scope didn't define (typecheck rejects those anyway).
        for (i, (name, pre)) in pre_if_vals.iter().enumerate() {
            let Some(pre_val) = *pre else { continue };
            let then_v = then_vals[i].unwrap_or(pre_val);
            let else_v = else_vals[i].unwrap_or(pre_val);
            // No need to phi if both branches leave the value untouched.
            if then_v == pre_val && else_v == pre_val {
                self.rebind_var(name, pre_val);
                continue;
            }
            let phi_dest = self.fresh_value();
            self.emit(Instruction::Phi {
                dest: phi_dest,
                incoming: vec![
                    PhiIncoming {
                        value: then_v,
                        block: then_end,
                    },
                    PhiIncoming {
                        value: else_v,
                        block: else_end,
                    },
                ],
            });
            self.rebind_var(name, phi_dest);
        }
        merge_dest
    }

    fn lower_while_loop(
        &mut self,
        condition: triet_syntax::arena::ExprId,
        body: &Block,
        treat_unknown_as_false: bool,
    ) {
        let header_id = self.fresh_block();
        let body_id = self.fresh_block();
        let exit_id = self.fresh_block();

        // Identify variables mutated by `Stmt::Assign` inside the body so
        // we can plumb a phi node at the header for each — otherwise the
        // SSA values bound to those names never advance across iterations
        // and the loop diverges.
        let mutated: Vec<String> = self.collect_assigned_vars(body);
        let pre_loop_block = self.current_block;
        // Snapshot the value bound to each mutated name BEFORE the loop —
        // this is the "from pre-header" phi incoming edge.
        let pre_loop_vals: Vec<(String, Option<ValueId>)> = mutated
            .iter()
            .map(|n| (n.clone(), self.resolve_var(n)))
            .collect();

        // Branch to the header.
        self.emit(Instruction::Br { target: header_id });

        // Loop header: emit phi node placeholders for each mutated var,
        // then evaluate the condition (which may read the phi values).
        self.start_block(header_id, Some("while_header".into()));
        let mut phi_dests: Vec<(String, ValueId)> = Vec::new();
        for (name, pre_val) in &pre_loop_vals {
            let Some(pre) = *pre_val else { continue };
            let phi_dest = self.fresh_value();
            self.emit(Instruction::Phi {
                dest: phi_dest,
                // Second incoming (from body end) is patched once the
                // body's final SSA value is known.
                incoming: vec![PhiIncoming {
                    value: pre,
                    block: pre_loop_block,
                }],
            });
            // Bind the phi as the new live value for `name` inside the loop.
            // v0.7.4.4: rebind into the declaring scope (not innermost),
            // so an intermediate `Expr::Block` scope around the while —
            // e.g. when the while sits in a match-arm body which the
            // parser wraps as `Expr::Block` — doesn't drop the phi-dest
            // on its `pop_scope`, leaving `lower_match_expr`'s post-arm
            // snapshot to read the stale pre-loop value from an outer
            // scope. The body-scope shadow at line ~1846 keeps in-body
            // reads/writes pointing at this phi-dest via the innermost
            // entry, so loop-local SSA tracking is unchanged.
            self.rebind_var(name, phi_dest);
            phi_dests.push((name.clone(), phi_dest));
        }
        let cond_val = self.lower_expr(condition);

        // ADR-0010: dispatch ternary-native. `while?` treats Unknown as
        // False (exit); plain `while` panics on Unknown per SPEC §7.1.1.
        let unknown_block_id = if treat_unknown_as_false {
            exit_id
        } else {
            self.fresh_block()
        };
        self.emit(Instruction::BrTrilean {
            cond: Operand::Value(cond_val),
            true_block: body_id,
            unknown_block: unknown_block_id,
            false_block: exit_id,
        });
        if !treat_unknown_as_false {
            // Materialise the panic block. Lowerer leaves current_block
            // on this block; body lowering below switches back to body_id.
            self.start_block(unknown_block_id, Some("while_unknown_panic".into()));
            self.emit(Instruction::Unreachable);
        }

        // Loop body.
        self.start_block(body_id, Some("while_body".into()));
        self.loop_stack.push(LoopContext {
            break_target: exit_id,
            continue_target: header_id,
        });
        self.push_scope();
        // Re-bind the phi values in the body scope so name lookups inside
        // the body resolve to the phi dest, not the pre-loop value.
        for (name, phi_dest) in &phi_dests {
            self.bind_var(name.clone(), *phi_dest);
        }
        self.lower_block(body);
        // After the body, each mutated name may resolve to a different
        // SSA value than the phi — that's the "from body" incoming edge.
        let body_end_block = self.current_block;
        let post_body_vals: Vec<(String, ValueId, ValueId)> = phi_dests
            .iter()
            .filter_map(|(name, phi_dest)| {
                self.resolve_var(name).map(|v| (name.clone(), *phi_dest, v))
            })
            .collect();
        self.pop_scope();
        self.loop_stack.pop();
        // Patch the phi nodes in the header block with the body-end edge.
        if let Some(header_block) = self.blocks.get_mut(&header_id) {
            for (_, phi_dest, body_val) in &post_body_vals {
                for instr in &mut header_block.instructions {
                    if let Instruction::Phi { dest, incoming } = instr
                        && *dest == *phi_dest
                    {
                        incoming.push(PhiIncoming {
                            value: *body_val,
                            block: body_end_block,
                        });
                    }
                }
            }
        }
        // Only branch back if not already terminated.
        if self.blocks[&self.current_block].terminator().is_none() {
            self.emit(Instruction::Br { target: header_id });
        }

        // Exit block.
        self.start_block(exit_id, Some("while_exit".into()));
    }

    fn lower_loop_stmt(&mut self, body: &Block) {
        let body_id = self.fresh_block();
        let exit_id = self.fresh_block();

        self.emit(Instruction::Br { target: body_id });

        self.start_block(body_id, Some("loop_body".into()));
        self.loop_stack.push(LoopContext {
            break_target: exit_id,
            continue_target: body_id,
        });
        self.push_scope();
        self.lower_block(body);
        self.pop_scope();
        self.loop_stack.pop();
        if self.blocks[&self.current_block].terminator().is_none() {
            self.emit(Instruction::Br { target: body_id });
        }

        self.start_block(exit_id, Some("loop_exit".into()));
    }

    fn lower_for_loop(
        &mut self,
        pattern: &triet_syntax::pattern::Pattern,
        iterable: triet_syntax::arena::ExprId,
        body: &Block,
    ) {
        let header_id = self.fresh_block();
        let body_id = self.fresh_block();
        let exit_id = self.fresh_block();

        // Check if the iterable is a Range, or `(Range).enumerate()`, for
        // proper counted-loop lowering. The enumerate adapter is special-
        // cased here because its iteration plan is statically known —
        // generic iterator protocol lowering is deferred (would need
        // closures or a state machine value).
        let spanned = &self.arena().expression(iterable);
        let is_range = matches!(&spanned.node, Expr::Range { .. });
        let enumerate_range: Option<triet_syntax::arena::ExprId> = match &spanned.node {
            Expr::MethodCall {
                receiver,
                method,
                arguments,
            } if method == "enumerate" && arguments.is_empty() => {
                let inner = &self.arena().expression(*receiver).node;
                if matches!(inner, Expr::Range { .. }) {
                    Some(*receiver)
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(range_expr_id) = enumerate_range {
            self.lower_for_enumerate(pattern, range_expr_id, body, header_id, body_id, exit_id);
        } else if is_range {
            if let Expr::Range {
                start,
                end,
                inclusive: _,
            } = &spanned.node
            {
                let start_val = self.lower_expr(*start);
                let end_val = self.lower_expr(*end);
                let phi_val = self.fresh_value();

                // Bind the loop variable to the phi value.
                if let triet_syntax::pattern::Pattern::Variable(var_name) = pattern {
                    self.bind_var(var_name.clone(), phi_val);
                }
                self.emit(Instruction::Br { target: header_id });

                // Header: phi merges start (entry) and incremented (body).
                let pre_header_id = self.current_block;
                self.start_block(header_id, Some("for_header".into()));
                self.emit(Instruction::Phi {
                    dest: phi_val,
                    incoming: vec![
                        PhiIncoming {
                            value: start_val,
                            block: pre_header_id,
                        },
                        // Second incoming will be patched after body lowering.
                    ],
                });
                let cmp_dest = self.fresh_value();
                self.emit(Instruction::Le {
                    dest: cmp_dest,
                    lhs: Operand::Value(phi_val),
                    rhs: Operand::Value(end_val),
                });
                // ADR-0010: Le on Integer never yields Unknown, but route
                // Unknown→exit defensively so a future widening of the
                // loop var type cannot silently change loop semantics.
                self.emit(Instruction::BrTrilean {
                    cond: Operand::Value(cmp_dest),
                    true_block: body_id,
                    unknown_block: exit_id,
                    false_block: exit_id,
                });

                // Body.
                self.start_block(body_id, Some("for_body".into()));
                self.loop_stack.push(LoopContext {
                    break_target: exit_id,
                    continue_target: header_id,
                });
                self.push_scope();
                self.lower_block(body);
                self.pop_scope();
                self.loop_stack.pop();
                if self.blocks[&self.current_block].terminator().is_none() {
                    // Increment loop var.
                    let inc_dest = self.fresh_value();
                    let c1 = self
                        .intern_constant(Constant::Integer(triet_core::Integer::new(1).unwrap()));
                    self.emit(Instruction::Add {
                        dest: inc_dest,
                        lhs: Operand::Value(phi_val),
                        rhs: Operand::Const(c1),
                    });
                    self.emit(Instruction::Br { target: header_id });
                    // Patch the phi: add the second incoming edge from body block.
                    let body_block_id = self.current_block;
                    // Go back and patch the phi in the header block.
                    if let Some(header_block) = self.blocks.get_mut(&header_id) {
                        for instr in &mut header_block.instructions {
                            if let Instruction::Phi { incoming, .. } = instr {
                                incoming.push(PhiIncoming {
                                    value: inc_dest,
                                    block: body_block_id,
                                });
                                break;
                            }
                        }
                    }
                }
            }
        } else {
            // Non-range iterable: execute body once (minimal viable).
            let iter_val = self.lower_expr(iterable);
            self.emit(Instruction::Br { target: header_id });

            self.start_block(header_id, Some("for_header".into()));
            // Always enter body once.
            self.emit(Instruction::Br { target: body_id });

            self.start_block(body_id, Some("for_body".into()));
            self.loop_stack.push(LoopContext {
                break_target: exit_id,
                continue_target: header_id,
            });
            self.push_scope();
            if let triet_syntax::pattern::Pattern::Variable(var_name) = pattern {
                self.bind_var(var_name.clone(), iter_val);
            }
            self.lower_block(body);
            self.pop_scope();
            self.loop_stack.pop();
            if self.blocks[&self.current_block].terminator().is_none() {
                self.emit(Instruction::Br { target: exit_id });
            }
        }

        self.start_block(exit_id, Some("for_exit".into()));
    }

    /// Lower `for (idx, item) in (start..=end).enumerate() { body }`.
    /// Generates two parallel phi nodes — one for the 0-based index, one
    /// for the range value — and binds tuple-destructure pattern names
    /// directly to them (no intermediate tuple struct).
    fn lower_for_enumerate(
        &mut self,
        pattern: &triet_syntax::pattern::Pattern,
        range_expr_id: triet_syntax::arena::ExprId,
        body: &Block,
        header_id: BlockId,
        body_id: BlockId,
        exit_id: BlockId,
    ) {
        let range_node = &self.arena().expression(range_expr_id).node;
        let Expr::Range {
            start,
            end,
            inclusive: _,
        } = range_node
        else {
            return;
        };
        let start_val = self.lower_expr(*start);
        let end_val = self.lower_expr(*end);
        let item_phi = self.fresh_value();
        let idx_phi = self.fresh_value();
        let zero_const =
            self.intern_constant(Constant::Integer(triet_core::Integer::new(0).unwrap()));
        let idx_init = self.fresh_value();
        self.emit(Instruction::Const {
            dest: idx_init,
            constant: zero_const,
        });

        // Bind the pattern. For `(idx, item)` tuple pattern → bind each
        // sub-pattern; for a single Variable pattern fall back to binding
        // the item (matches the non-enumerate range path).
        match pattern {
            triet_syntax::pattern::Pattern::Tuple(elems) if elems.len() == 2 => {
                for (i, pat_id) in elems.iter().enumerate() {
                    let inner = &self.arena().pattern(*pat_id).node;
                    if let triet_syntax::pattern::Pattern::Variable(name) = inner {
                        let v = if i == 0 { idx_phi } else { item_phi };
                        self.bind_var(name.clone(), v);
                    }
                }
            }
            triet_syntax::pattern::Pattern::Variable(name) => {
                self.bind_var(name.clone(), item_phi);
            }
            _ => {}
        }
        self.emit(Instruction::Br { target: header_id });

        // Header: phi merges (start, 0) on entry and (item+1, idx+1) from body.
        let pre_header_id = self.current_block;
        self.start_block(header_id, Some("for_enum_header".into()));
        self.emit(Instruction::Phi {
            dest: item_phi,
            incoming: vec![PhiIncoming {
                value: start_val,
                block: pre_header_id,
            }],
        });
        self.emit(Instruction::Phi {
            dest: idx_phi,
            incoming: vec![PhiIncoming {
                value: idx_init,
                block: pre_header_id,
            }],
        });
        let cmp_dest = self.fresh_value();
        self.emit(Instruction::Le {
            dest: cmp_dest,
            lhs: Operand::Value(item_phi),
            rhs: Operand::Value(end_val),
        });
        // ADR-0010: ternary-native — Le on Integer is never Unknown, but
        // route Unknown→exit defensively.
        self.emit(Instruction::BrTrilean {
            cond: Operand::Value(cmp_dest),
            true_block: body_id,
            unknown_block: exit_id,
            false_block: exit_id,
        });

        // Body.
        self.start_block(body_id, Some("for_enum_body".into()));
        self.loop_stack.push(LoopContext {
            break_target: exit_id,
            continue_target: header_id,
        });
        self.push_scope();
        // Re-bind the pattern names inside the body scope so user code
        // sees the phi values, not stale outer-scope bindings.
        match pattern {
            triet_syntax::pattern::Pattern::Tuple(elems) if elems.len() == 2 => {
                for (i, pat_id) in elems.iter().enumerate() {
                    let inner = &self.arena().pattern(*pat_id).node;
                    if let triet_syntax::pattern::Pattern::Variable(name) = inner {
                        let v = if i == 0 { idx_phi } else { item_phi };
                        self.bind_var(name.clone(), v);
                    }
                }
            }
            triet_syntax::pattern::Pattern::Variable(name) => {
                self.bind_var(name.clone(), item_phi);
            }
            _ => {}
        }
        self.lower_block(body);
        self.pop_scope();
        self.loop_stack.pop();
        if self.blocks[&self.current_block].terminator().is_none() {
            let one_const =
                self.intern_constant(Constant::Integer(triet_core::Integer::new(1).unwrap()));
            let item_next = self.fresh_value();
            self.emit(Instruction::Add {
                dest: item_next,
                lhs: Operand::Value(item_phi),
                rhs: Operand::Const(one_const),
            });
            let idx_next = self.fresh_value();
            self.emit(Instruction::Add {
                dest: idx_next,
                lhs: Operand::Value(idx_phi),
                rhs: Operand::Const(one_const),
            });
            self.emit(Instruction::Br { target: header_id });
            let body_end = self.current_block;
            if let Some(header_block) = self.blocks.get_mut(&header_id) {
                let mut patched_item = false;
                let mut patched_idx = false;
                for instr in &mut header_block.instructions {
                    if let Instruction::Phi { dest, incoming } = instr {
                        if *dest == item_phi && !patched_item {
                            incoming.push(PhiIncoming {
                                value: item_next,
                                block: body_end,
                            });
                            patched_item = true;
                        } else if *dest == idx_phi && !patched_idx {
                            incoming.push(PhiIncoming {
                                value: idx_next,
                                block: body_end,
                            });
                            patched_idx = true;
                        }
                    }
                }
            }
        }

        self.start_block(exit_id, Some("for_enum_exit".into()));
    }

    fn lower_match_expr(
        &mut self,
        scrutinee: triet_syntax::arena::ExprId,
        arms: &[MatchArm],
    ) -> ValueId {
        let scrutee_val = self.lower_expr(scrutinee);
        if arms.is_empty() {
            let c = self.intern_constant(Constant::Unit);
            let d = self.fresh_value();
            self.emit(Instruction::Const {
                dest: d,
                constant: c,
            });
            return d;
        }

        // v0.7.4.3-debt.5 (WA-1): collect every variable that any arm
        // body reassigns so the merge block can phi-merge them. Without
        // this, all arms write to the same outer-scope binding and the
        // LAST arm's SSA value wins statically — at runtime any executed
        // arm overwrites the others' state. Pre-fix, this corrupted a
        // mutable `Vector<T>` (or anything else) that arms rebound
        // because the loop header's phi was patched with the
        // statically-last arm's value, regardless of which arm actually
        // ran.
        let mut mutated: Vec<String> = Vec::new();
        for arm in arms {
            for n in self.walk_expr_for_assigns_into(arm.body) {
                if !mutated.iter().any(|x| x == &n) {
                    mutated.push(n);
                }
            }
        }
        // Snapshot pre-match bindings so every arm starts from the same
        // dominator state and arms that DON'T touch a given var still
        // contribute the pre-match value to the merge phi.
        let pre_match_vals: Vec<(String, Option<ValueId>)> = mutated
            .iter()
            .map(|n| (n.clone(), self.resolve_var(n)))
            .collect();

        let merge_block_id = self.fresh_block();
        let merge_dest = self.fresh_value();
        let mut phi_incoming: Vec<PhiIncoming> = Vec::new();
        // For each arm: store (end_block, [post-arm value for each mutated var]).
        let mut arm_mutated_vals: Vec<(BlockId, Vec<Option<ValueId>>)> =
            Vec::with_capacity(arms.len());

        // Each arm becomes: `test_block` → if-match → `arm_block` → merge,
        // else fall through to next `test_block`. The first arm's test
        // block is the current block (we don't allocate a fresh one for
        // it — the caller is already in a sensible position).
        let mut next_test_block = self.current_block;

        for (i, arm) in arms.iter().enumerate() {
            let is_last = i + 1 == arms.len();
            let arm_body_block = self.fresh_block();

            // Ensure we are in the test block for this arm.
            if self.current_block != next_test_block {
                self.start_block(next_test_block, Some(format!("match_test_{i}")));
            }

            // Extract the arm pattern.
            let pat_node = &self.arena().pattern(arm.pattern).node;

            // Emit tag check (if needed) and branch to the arm body or
            // the next test. The last arm is unconditional (exhaustive
            // by typechecker invariant).
            let next_block = if is_last {
                arm_body_block
            } else {
                self.fresh_block()
            };

            // Synthesise the match-test predicate as an optional ValueId.
            // `None` means "pattern matches everything" (wildcard / bare
            // variable). The last arm is unconditional regardless — the
            // typechecker enforces exhaustiveness.
            if is_last {
                self.emit(Instruction::Br {
                    target: arm_body_block,
                });
            } else {
                let test = self.lower_pattern_test(pat_node, scrutee_val);
                match test {
                    None => {
                        // Wildcard/variable: always matches, drop straight in.
                        self.emit(Instruction::Br {
                            target: arm_body_block,
                        });
                    }
                    Some(cmp_val) => {
                        // ADR-0010: match arm test is a Trilean from Eq /
                        // LukAnd / NullCheck. Unknown means "we cannot
                        // confirm this arm matches" → skip to next test;
                        // False means "definitely doesn't match" → same.
                        self.emit(Instruction::BrTrilean {
                            cond: Operand::Value(cmp_val),
                            true_block: arm_body_block,
                            unknown_block: next_block,
                            false_block: next_block,
                        });
                    }
                }
            }

            // Restore pre-match bindings so this arm sees the same
            // dominator state as every other arm (mirrors `lower_if_expr`
            // where the then/else branches both start from pre-if).
            for (name, pre) in &pre_match_vals {
                if let Some(v) = *pre {
                    self.rebind_var(name, v);
                }
            }

            // Arm body block.
            self.start_block(arm_body_block, Some(format!("match_arm_{i}")));
            self.push_scope();

            // Bind pattern variables (enum payload, tuple element vars,
            // or whole-scrutinee variable).
            self.bind_pattern_vars(pat_node, scrutee_val);

            let arm_val = self.lower_expr(arm.body);

            // Snapshot the post-arm SSA value for every mutated var
            // BEFORE popping the arm scope — pop drops pattern-bound
            // names but leaves the outer-scope rebindings intact via
            // `rebind_var`'s walk-to-declaring-scope contract.
            let post_vals: Vec<Option<ValueId>> =
                mutated.iter().map(|n| self.resolve_var(n)).collect();

            self.pop_scope();

            phi_incoming.push(PhiIncoming {
                value: arm_val,
                block: self.current_block,
            });
            arm_mutated_vals.push((self.current_block, post_vals));

            if self.blocks[&self.current_block].terminator().is_none() {
                self.emit(Instruction::Br {
                    target: merge_block_id,
                });
            }

            next_test_block = next_block;
        }

        // Merge block.
        self.start_block(merge_block_id, Some("match_merge".into()));
        self.emit(Instruction::Phi {
            dest: merge_dest,
            incoming: phi_incoming,
        });

        // v0.7.4.3-debt.5: emit one phi per mutated outer-scope var so
        // post-match reads see a single merged SSA value. Skip vars that
        // every arm left untouched (pre-match value identical at every
        // arm-end).
        for (i, (name, pre)) in pre_match_vals.iter().enumerate() {
            let Some(pre_val) = *pre else { continue };
            // Build per-arm incoming pairs, defaulting arms that didn't
            // touch the var to the pre-match value.
            let incoming: Vec<PhiIncoming> = arm_mutated_vals
                .iter()
                .map(|(arm_end_block, post)| PhiIncoming {
                    value: post[i].unwrap_or(pre_val),
                    block: *arm_end_block,
                })
                .collect();
            // No phi needed if every arm left the value untouched.
            if incoming.iter().all(|p| p.value == pre_val) {
                self.rebind_var(name, pre_val);
                continue;
            }
            let phi_dest = self.fresh_value();
            self.emit(Instruction::Phi {
                dest: phi_dest,
                incoming,
            });
            self.rebind_var(name, phi_dest);
        }
        merge_dest
    }

    /// Helper: collect the names of variables assigned by `Stmt::Assign`
    /// reachable from a single expression. Used by match-arm body
    /// scanning so we can phi-merge mutations across arms. Wrapper over
    /// `walk_expr_for_assigns` for ergonomics.
    fn walk_expr_for_assigns_into(&self, expr: triet_syntax::arena::ExprId) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        self.walk_expr_for_assigns(expr, &mut out);
        out
    }

    /// Synthesise a Trilean-valued match test for `pattern` against
    /// `scrutinee`. Returns `None` when the pattern matches anything
    /// (Wildcard, bare Variable) — the caller treats this as an
    /// unconditional jump. Returns `Some(val)` for patterns that need a
    /// runtime check (literals, tuple destructure, enum variant).
    fn lower_pattern_test(
        &mut self,
        pattern: &triet_syntax::pattern::Pattern,
        scrutinee: ValueId,
    ) -> Option<ValueId> {
        match pattern {
            triet_syntax::pattern::Pattern::Wildcard => None,
            // v0.7.4.3-debt.5: bare identifier patterns whose name
            // matches a known enum variant must be tested as
            // `EnumVariant` (tag check), NOT treated as a catch-all
            // variable binding. The parser leaves them as
            // `Pattern::Variable` because it doesn't carry a variant
            // table — the lowerer disambiguates here. Mirrors the
            // existing `Expr::Identifier`-as-variant resolution at
            // line 815-833. Without this, `match e { A => ..., B =>
            // ... }` always dispatches to arm 0 because Variable
            // returns None (catch-all). Pre-fix this latent bug was
            // hidden by `lower_match_expr`'s static-last-write
            // semantics around mutable rebinds — fixing match-arm
            // phi-merging in this same sub-task exposed it.
            triet_syntax::pattern::Pattern::Variable(name)
                if self.variant_index.contains_key(name) =>
            {
                let target_idx = self.variant_index.get(name).map_or(0, |(_, i)| *i);
                let tag = self.fresh_value();
                self.emit(Instruction::EnumTag {
                    dest: tag,
                    scrutinee: Operand::Value(scrutinee),
                });
                // v0.7.4.3-debt.7: EnumTag now returns Integer.
                let const_id = self.intern_constant(Constant::Integer(
                    triet_core::Integer::new(i64::from(target_idx)).unwrap_or_default(),
                ));
                let const_val = self.fresh_value();
                self.emit(Instruction::Const {
                    dest: const_val,
                    constant: const_id,
                });
                let cmp = self.fresh_value();
                self.emit(Instruction::Eq {
                    dest: cmp,
                    lhs: Operand::Value(tag),
                    rhs: Operand::Value(const_val),
                });
                Some(cmp)
            }
            triet_syntax::pattern::Pattern::Variable(_) => None,
            triet_syntax::pattern::Pattern::Null => {
                // null pattern: scrutinee is null iff NullCheck returns Zero.
                let check = self.fresh_value();
                self.emit(Instruction::NullCheck {
                    dest: check,
                    nullable: Operand::Value(scrutinee),
                });
                // Compare `check` against Trit::Zero — equal → matched.
                let zero_const = self.intern_constant(Constant::Trit(Trit::Zero));
                let zero_val = self.fresh_value();
                self.emit(Instruction::Const {
                    dest: zero_val,
                    constant: zero_const,
                });
                let cmp = self.fresh_value();
                self.emit(Instruction::Eq {
                    dest: cmp,
                    lhs: Operand::Value(check),
                    rhs: Operand::Value(zero_val),
                });
                Some(cmp)
            }
            triet_syntax::pattern::Pattern::Literal(lit) => {
                let const_id = match lit {
                    triet_syntax::pattern::LiteralPattern::Integer { value, .. }
                    | triet_syntax::pattern::LiteralPattern::Ternary(value) => {
                        let n = i64::try_from(*value).unwrap_or(0);
                        self.intern_constant(Constant::Integer(
                            triet_core::Integer::new(n).unwrap_or_default(),
                        ))
                    }
                    triet_syntax::pattern::LiteralPattern::String(s) => {
                        self.intern_constant(Constant::String(s.clone()))
                    }
                    triet_syntax::pattern::LiteralPattern::Trilean(tv) => {
                        let tl = match tv {
                            triet_syntax::numeric::TrileanValue::True => Trilean::True,
                            triet_syntax::numeric::TrileanValue::False => Trilean::False,
                            triet_syntax::numeric::TrileanValue::Unknown => Trilean::Unknown,
                        };
                        self.intern_constant(Constant::Trilean(tl))
                    }
                };
                let lit_val = self.fresh_value();
                self.emit(Instruction::Const {
                    dest: lit_val,
                    constant: const_id,
                });
                let cmp = self.fresh_value();
                self.emit(Instruction::Eq {
                    dest: cmp,
                    lhs: Operand::Value(scrutinee),
                    rhs: Operand::Value(lit_val),
                });
                Some(cmp)
            }
            triet_syntax::pattern::Pattern::Tuple(elems) => {
                // For each sub-pattern at index i, extract scrutinee.i via
                // FieldGet and recurse. Conjoin all non-wildcard tests.
                let mut acc: Option<ValueId> = None;
                for (i, sub_id) in elems.iter().enumerate() {
                    let sub_pat = &self.arena().pattern(*sub_id).node.clone();
                    let field_val = self.fresh_value();
                    self.emit(Instruction::FieldGet {
                        dest: field_val,
                        object: Operand::Value(scrutinee),
                        field_idx: u32::try_from(i).unwrap_or(0),
                    });
                    if let Some(test) = self.lower_pattern_test(sub_pat, field_val) {
                        acc = Some(acc.map_or(test, |prev| {
                            let and_dest = self.fresh_value();
                            self.emit(Instruction::LukAnd {
                                dest: and_dest,
                                lhs: Operand::Value(prev),
                                rhs: Operand::Value(test),
                            });
                            and_dest
                        }));
                    }
                }
                acc
            }
            triet_syntax::pattern::Pattern::EnumVariant { variant_name, .. } => {
                let target_idx = self.variant_index.get(variant_name).map_or(0, |(_, i)| *i);
                let tag = self.fresh_value();
                self.emit(Instruction::EnumTag {
                    dest: tag,
                    scrutinee: Operand::Value(scrutinee),
                });
                // v0.7.4.3-debt.7: EnumTag returns Integer (variant
                // index), so compare against the Integer constant.
                let const_id = self.intern_constant(Constant::Integer(
                    triet_core::Integer::new(i64::from(target_idx)).unwrap_or_default(),
                ));
                let const_val = self.fresh_value();
                self.emit(Instruction::Const {
                    dest: const_val,
                    constant: const_id,
                });
                let cmp = self.fresh_value();
                self.emit(Instruction::Eq {
                    dest: cmp,
                    lhs: Operand::Value(tag),
                    rhs: Operand::Value(const_val),
                });
                Some(cmp)
            }
            // Outcome arm pattern (ADR-0020 §5): test the
            // discriminator trit against the expected arm. Payload
            // binding happens in `bind_pattern_vars`.
            triet_syntax::pattern::Pattern::OutcomeArm { arm, .. } => {
                let disc = self.fresh_value();
                self.emit(Instruction::OutcomeDiscriminant {
                    dest: disc,
                    source: Operand::Value(scrutinee),
                });
                let expected_trit = match arm {
                    OutcomeArm::Positive => Trit::Positive,
                    OutcomeArm::Zero => Trit::Zero,
                    OutcomeArm::Negative => Trit::Negative,
                };
                let const_id = self.intern_constant(Constant::Trit(expected_trit));
                let const_val = self.fresh_value();
                self.emit(Instruction::Const {
                    dest: const_val,
                    constant: const_id,
                });
                let cmp = self.fresh_value();
                self.emit(Instruction::Eq {
                    dest: cmp,
                    lhs: Operand::Value(disc),
                    rhs: Operand::Value(const_val),
                });
                Some(cmp)
            }
            // Or/Range patterns deferred (not exercised by current examples).
            _ => None,
        }
    }

    /// Walk a pattern and bind every Variable sub-pattern to a freshly
    /// extracted SSA value from `scrutinee`. Used inside match arm
    /// bodies so the body can refer to the destructured names.
    fn bind_pattern_vars(&mut self, pattern: &triet_syntax::pattern::Pattern, scrutinee: ValueId) {
        match pattern {
            // v0.7.4.3-debt.5: a bare-identifier pattern whose name
            // matches a known enum variant is NOT a variable binding —
            // it's a tag check. Skip the bind so `match e { A => ...,
            // B => ... }` doesn't pollute the arm scope with stale
            // bindings of A/B. Mirrors the `lower_pattern_test`
            // disambiguation.
            triet_syntax::pattern::Pattern::Variable(name)
                if self.variant_index.contains_key(name) => {}
            triet_syntax::pattern::Pattern::Variable(name) => {
                self.bind_var(name.clone(), scrutinee);
            }
            triet_syntax::pattern::Pattern::EnumVariant {
                variant_name,
                payload: Some(payload_pat),
                ..
            } => {
                let inner_pat = self.arena().pattern(*payload_pat).node.clone();
                let payload_val = self.fresh_value();
                self.emit(Instruction::EnumPayload {
                    dest: payload_val,
                    scrutinee: Operand::Value(scrutinee),
                });
                // v0.7.5.1: propagate the variant's struct payload
                // identity onto the bound SSA value so field access
                // through `p.field` in the arm body resolves the right
                // slot (mirrors the OutcomeArm propagation below).
                if let Some(s) = self.variant_payload_struct.get(variant_name).cloned() {
                    self.value_struct_types.insert(payload_val, s);
                }
                self.bind_pattern_vars(&inner_pat, payload_val);
            }
            triet_syntax::pattern::Pattern::Tuple(elems) => {
                for (i, sub_id) in elems.iter().enumerate() {
                    let sub_pat = self.arena().pattern(*sub_id).node.clone();
                    let field_val = self.fresh_value();
                    self.emit(Instruction::FieldGet {
                        dest: field_val,
                        object: Operand::Value(scrutinee),
                        field_idx: u32::try_from(i).unwrap_or(0),
                    });
                    self.bind_pattern_vars(&sub_pat, field_val);
                }
            }
            // Outcome arm pattern — extract the payload via the
            // matching unwrap opcode then recurse into the sub-pattern.
            // `~0` has no payload, so its `payload` is always `None`
            // (typecheck guarantees this).
            triet_syntax::pattern::Pattern::OutcomeArm {
                arm,
                payload: Some(payload_pat),
            } => {
                let inner_pat = self.arena().pattern(*payload_pat).node.clone();
                let payload_val = self.fresh_value();
                let unwrap = match arm {
                    OutcomeArm::Positive => Instruction::OutcomeUnwrapValue {
                        dest: payload_val,
                        source: Operand::Value(scrutinee),
                    },
                    OutcomeArm::Negative => Instruction::OutcomeUnwrapError {
                        dest: payload_val,
                        source: Operand::Value(scrutinee),
                    },
                    OutcomeArm::Zero => {
                        // ~0 has no payload — sub-pattern presence is a
                        // typecheck violation; silently skip rather than
                        // emit a malformed unwrap.
                        return;
                    }
                };
                self.emit(unwrap);
                // v0.7.4.3-debt.2 (WA-2): propagate success-arm struct
                // identity onto the bound payload so the arm body can
                // resolve `payload.field` correctly.
                if matches!(arm, OutcomeArm::Positive)
                    && let Some(s) = self.value_outcome_value_struct.get(&scrutinee).cloned()
                {
                    self.value_struct_types.insert(payload_val, s);
                }
                self.bind_pattern_vars(&inner_pat, payload_val);
            }
            _ => {}
        }
    }

    // ── Call lowering ────────────────────────────────────────────

    fn lower_call(
        &mut self,
        callee: triet_syntax::arena::ExprId,
        arguments: &[triet_syntax::arena::ExprId],
    ) -> ValueId {
        let args: Vec<Operand> = arguments
            .iter()
            .map(|a| Operand::Value(self.lower_expr(*a)))
            .collect();

        // Resolve callee.
        let callee_expr = &self.arena().expression(callee).node;
        match callee_expr {
            Expr::Identifier(name) => {
                // Enum tuple variant: `Some(42)` is parsed as Call with
                // Identifier("Some"). Promote to EnumNew with payload.
                if let Some((_enum_name, variant_idx)) = self.variant_index.get(name).cloned() {
                    let payload = args.into_iter().next();
                    let dest = self.fresh_value();
                    self.emit(Instruction::EnumNew {
                        dest,
                        variant_idx,
                        payload,
                    });
                    return dest;
                }
                // Check for builtins.
                if let Some(builtin) = resolve_builtin(name) {
                    let dest = self.fresh_value();
                    self.emit(Instruction::CallBuiltin {
                        dest: Some(dest),
                        name: builtin,
                        args,
                    });
                    return dest;
                }

                // Cross-module call via bindings — issue this BEFORE the
                // local-function lookup so the VM can intercept calls to
                // stdlib stub functions (`std.text.len`, etc.) by their
                // absolute path. Otherwise `resolve_func` would resolve
                // the import to a `FuncId` and `CallLocal` would run the
                // placeholder body that ships in `std/*.tri`.
                if let Some(abs_path) = self.current_module().bindings.get(name).cloned() {
                    let dest = self.fresh_value();
                    // Propagate struct-typing if the imported callee
                    // returns a struct. The callee's `FuncId` is in
                    // `func_table`; check `func_return_struct` for the
                    // associated struct name.
                    if let Some(callee_id) = self.func_table.get(&abs_path).copied() {
                        if let Some(s) = self.func_return_struct.get(&callee_id).cloned() {
                            self.value_struct_types.insert(dest, s);
                        }
                        // v0.7.4.3-debt.2 (WA-2): also seed the
                        // outcome-payload tracker for callees returning
                        // `T~E` / `T?~E`.
                        if let Some(s) = self
                            .func_return_outcome_value_struct
                            .get(&callee_id)
                            .cloned()
                        {
                            self.value_outcome_value_struct.insert(dest, s);
                        }
                    }
                    self.emit(Instruction::CallCrossModule {
                        dest: Some(dest),
                        path: abs_path,
                        args,
                    });
                    return dest;
                }

                // Check function table for local functions defined in the
                // current module (not imported).
                if let Some(func_id) = self.resolve_func(name) {
                    let dest = self.fresh_value();
                    if let Some(s) = self.func_return_struct.get(&func_id).cloned() {
                        self.value_struct_types.insert(dest, s);
                    }
                    // v0.7.4.3-debt.2 (WA-2): outcome-payload tracker.
                    if let Some(s) = self.func_return_outcome_value_struct.get(&func_id).cloned() {
                        self.value_outcome_value_struct.insert(dest, s);
                    }
                    self.emit(Instruction::CallLocal {
                        dest: Some(dest),
                        callee: func_id,
                        args,
                    });
                    return dest;
                }

                // Fallback: treat as local function reference.

                self.fresh_value()
            }
            _ => {
                // Indirect call (closure, etc.) — simplified.

                self.fresh_value()
            }
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Map a builtin name string to the IR `BuiltinName`.
fn resolve_builtin(name: &str) -> Option<BuiltinName> {
    match name {
        "println" => Some(BuiltinName::Println),
        "print" => Some(BuiltinName::Print),
        "assert" => Some(BuiltinName::Assert),
        "assert_eq" => Some(BuiltinName::AssertEq),
        // Global stdlib helpers — bound to integer/value formatters in the
        // interpreter; we route them through `TextFromInteger` here so the
        // VM produces the same string representation.
        "to_string" | "tryte_to_string" => Some(BuiltinName::TextFromInteger),
        _ => None,
    }
}

// (Removed v0.7.4.3-error.fix: `field_name_to_idx` placeholder.
// Field resolution now uses `LowerCtx::resolve_struct_field_idx`
// which consults the per-function `value_struct_types` map +
// global `struct_fields` table populated in Pass 1a.)

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use triet_modules::{Module, ModuleId, ModulePath};
    use triet_syntax::{
        Spanned,
        arena::Arena,
        arena::ExprId,
        expr::Expr,
        item::{FunctionBody, FunctionDef, FunctionParam, Item},
        numeric::TrileanValue,
        stmt::{Block, Stmt},
    };

    /// Build a minimal `ResolvedProgram` with one module and one function.
    fn make_program(arena: Arena, items: Vec<Spanned<Item>>) -> ResolvedProgram {
        let root_path = ModulePath::crate_root();
        let module = Module {
            path: root_path,
            source_path: None,
            arena_id: triet_modules::ArenaId(0),
            items,
            bindings: HashMap::new(),
            parent: None,
            children: Vec::new(),
        };
        ResolvedProgram {
            arenas: vec![arena],
            modules: vec![module],
            root: ModuleId(0),
        }
    }

    /// Helper: alloc an integer literal expression.
    fn int_lit(arena: &mut Arena, value: i128) -> ExprId {
        arena.alloc_expression(Spanned::new(
            Expr::IntegerLiteral {
                value,
                suffix: None,
            },
            0..1,
        ))
    }

    /// Helper: alloc a trilean literal expression.
    fn trilean_lit(arena: &mut Arena, tv: TrileanValue) -> ExprId {
        arena.alloc_expression(Spanned::new(Expr::TrileanLiteral(tv), 0..1))
    }

    /// Helper: alloc a string literal expression.
    fn string_lit(arena: &mut Arena, s: &str) -> ExprId {
        arena.alloc_expression(Spanned::new(Expr::StringLiteral(s.to_owned()), 0..1))
    }

    /// Helper: alloc a binary op expression.
    fn binary_op(arena: &mut Arena, op: BinaryOperator, left: ExprId, right: ExprId) -> ExprId {
        arena.alloc_expression(Spanned::new(
            Expr::BinaryOp {
                operator: op,
                left,
                right,
            },
            0..1,
        ))
    }

    /// Helper: alloc an Identifier expression.
    fn ident(arena: &mut Arena, name: &str) -> ExprId {
        arena.alloc_expression(Spanned::new(Expr::Identifier(name.to_owned()), 0..1))
    }

    /// Helper: alloc a call expression.
    fn call_expr(arena: &mut Arena, callee: ExprId, arguments: Vec<ExprId>) -> ExprId {
        arena.alloc_expression(Spanned::new(Expr::Call { callee, arguments }, 0..1))
    }

    /// Helper: alloc a negate expression.
    fn negate(arena: &mut Arena, operand: ExprId) -> ExprId {
        arena.alloc_expression(Spanned::new(
            Expr::UnaryOp {
                operator: triet_syntax::expr::UnaryOperator::Negate,
                operand,
            },
            0..1,
        ))
    }

    fn make_function_def(
        name: &str,
        params: Vec<FunctionParam>,
        return_type: Option<triet_syntax::arena::TypeId>,
        body: FunctionBody,
    ) -> Item {
        Item::Function(FunctionDef {
            visibility: triet_syntax::visibility::Visibility::Private,
            name: name.to_owned(),
            type_params: Vec::new(),
            parameters: params,
            return_type,
            body,
        })
    }

    /// Helper: alloc a `Named(name)` type expression.
    fn named_type(arena: &mut Arena, name: &str) -> triet_syntax::arena::TypeId {
        arena.alloc_type(Spanned::new(
            triet_syntax::type_ast::TypeExpr::Named(name.to_owned()),
            0..1,
        ))
    }

    /// Helper: build a `FunctionParam` with default Mojo-style passing.
    fn param(name: &str, type_annotation: triet_syntax::arena::TypeId) -> FunctionParam {
        FunctionParam {
            name: name.to_owned(),
            type_annotation,
            passing: triet_syntax::ParameterPassing::Borrowed,
        }
    }

    // ── Struct field-index regression (v0.7.4.3-error.fix) ─────────
    //
    // Pre-fix, `field_name_to_idx` returned 0 for every field name,
    // so `point.y` lowered to `FieldGet { field_idx: 0 }` — silently
    // returning `point.x` instead. The fix wires struct definitions
    // into the lowerer via `LowerCtx::struct_fields` and a per-value
    // `value_struct_types` map populated at StructLiteral / param /
    // call sites. These tests pin the corrected behavior.

    /// Build a `Point { x: Integer, y: Integer }` struct definition.
    fn point_struct_item(arena: &mut Arena) -> Item {
        let x_ty = named_type(arena, "Integer");
        let y_ty = named_type(arena, "Integer");
        Item::Struct(triet_syntax::item::StructDef {
            visibility: triet_syntax::visibility::Visibility::Public,
            name: "Point".to_owned(),
            type_params: Vec::new(),
            fields: vec![
                triet_syntax::item::StructField {
                    name: "x".to_owned(),
                    type_annotation: x_ty,
                },
                triet_syntax::item::StructField {
                    name: "y".to_owned(),
                    type_annotation: y_ty,
                },
            ],
        })
    }

    /// `function get_y(p: Point) -> Integer = p.y` lowers `p.y` to
    /// `FieldGet { field_idx: 1 }`, not 0.
    #[test]
    fn struct_field_access_uses_correct_index_for_second_field() {
        let mut arena = Arena::new();
        let struct_item = point_struct_item(&mut arena);
        let param_type = named_type(&mut arena, "Point");
        let ret_type = named_type(&mut arena, "Integer");
        let p_ident = ident(&mut arena, "p");
        let field_access = arena.alloc_expression(Spanned::new(
            Expr::FieldAccess {
                object: p_ident,
                field: "y".to_owned(),
            },
            0..1,
        ));
        let body = FunctionBody::Expression(field_access);
        let params = vec![param("p", param_type)];
        let func_item = make_function_def("get_y", params, Some(ret_type), body);
        let prog = make_program(
            arena,
            vec![
                Spanned::new(struct_item, 0..1),
                Spanned::new(func_item, 0..1),
            ],
        );
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        let field_idx = func.blocks[0]
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::FieldGet { field_idx, .. } => Some(*field_idx),
                _ => None,
            })
            .expect("FieldGet should be emitted for p.y");
        assert_eq!(
            field_idx, 1,
            "p.y must lower to field_idx=1, got {field_idx} (pre-fix would return 0)",
        );
    }

    /// `function get_x(p: Point) -> Integer = p.x` lowers `p.x` to
    /// `FieldGet { field_idx: 0 }`. Pin the happy-zero case too —
    /// previously the placeholder coincidentally produced this same
    /// result, but for the wrong reason.
    #[test]
    fn struct_field_access_uses_correct_index_for_first_field() {
        let mut arena = Arena::new();
        let struct_item = point_struct_item(&mut arena);
        let param_type = named_type(&mut arena, "Point");
        let ret_type = named_type(&mut arena, "Integer");
        let p_ident = ident(&mut arena, "p");
        let field_access = arena.alloc_expression(Spanned::new(
            Expr::FieldAccess {
                object: p_ident,
                field: "x".to_owned(),
            },
            0..1,
        ));
        let body = FunctionBody::Expression(field_access);
        let params = vec![param("p", param_type)];
        let func_item = make_function_def("get_x", params, Some(ret_type), body);
        let prog = make_program(
            arena,
            vec![
                Spanned::new(struct_item, 0..1),
                Spanned::new(func_item, 0..1),
            ],
        );
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        let field_idx = func.blocks[0]
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::FieldGet { field_idx, .. } => Some(*field_idx),
                _ => None,
            })
            .expect("FieldGet should be emitted for p.x");
        assert_eq!(field_idx, 0);
    }

    // ── Literal tests ──────────────────────────────────────────────

    #[test]
    fn lower_integer_literal() {
        let mut arena = Arena::new();
        let lit = int_lit(&mut arena, 42);
        let body = FunctionBody::Expression(lit);
        let item = make_function_def("main", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);

        let ir = lower_program(&prog);
        assert_eq!(ir.function_count(), 1);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    #[test]
    fn lower_trilean_literals() {
        let mut arena = Arena::new();
        let t = trilean_lit(&mut arena, TrileanValue::True);
        let body = FunctionBody::Expression(t);
        let item = make_function_def("test_true", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert_eq!(ir.function_count(), 1);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    #[test]
    fn lower_string_literal() {
        let mut arena = Arena::new();
        let s = string_lit(&mut arena, "hello");
        let body = FunctionBody::Expression(s);
        let item = make_function_def("greet", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert_eq!(ir.function_count(), 1);
    }

    #[test]
    fn lower_ternary_literal() {
        let mut arena = Arena::new();
        let tlit = arena.alloc_expression(Spanned::new(Expr::TernaryLiteral { value: 42 }, 0..1));
        let body = FunctionBody::Expression(tlit);
        let item = make_function_def("tern", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert_eq!(ir.function_count(), 1);
    }

    // ── Arithmetic tests ────────────────────────────────────────────

    #[test]
    fn lower_addition() {
        let mut arena = Arena::new();
        let a = int_lit(&mut arena, 10);
        let b = int_lit(&mut arena, 20);
        let add = binary_op(&mut arena, BinaryOperator::Add, a, b);
        let body = FunctionBody::Expression(add);
        let item = make_function_def("add_10_20", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);

        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        // Entry block should have: const, const, add, ret
        let entry = func.entry_block().unwrap();
        assert!(entry.instructions.len() >= 2);
        assert!(
            entry
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Add { .. }))
        );
    }

    #[test]
    fn lower_all_arithmetic_ops() {
        for op in [
            BinaryOperator::Add,
            BinaryOperator::Subtract,
            BinaryOperator::Multiply,
            BinaryOperator::Divide,
            BinaryOperator::Modulo,
            BinaryOperator::Power,
        ] {
            let mut arena = Arena::new();
            let a = int_lit(&mut arena, 6);
            let b = int_lit(&mut arena, 2);
            let expr = binary_op(&mut arena, op, a, b);
            let body = FunctionBody::Expression(expr);
            let item = make_function_def("arith", vec![], None, body);
            let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
            let ir = lower_program(&prog);
            assert!(
                ir.modules[0].functions[0].is_well_formed(),
                "failed for op {op:?}"
            );
        }
    }

    #[test]
    fn lower_negate() {
        let mut arena = Arena::new();
        let a = int_lit(&mut arena, 5);
        let neg = negate(&mut arena, a);
        let body = FunctionBody::Expression(neg);
        let item = make_function_def("negate", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        assert!(
            func.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Neg { .. }))
        );
    }

    // ── Comparison tests ────────────────────────────────────────────

    #[test]
    fn lower_comparisons() {
        for op in [
            BinaryOperator::Equal,
            BinaryOperator::NotEqual,
            BinaryOperator::LessThan,
            BinaryOperator::LessEqual,
            BinaryOperator::GreaterThan,
            BinaryOperator::GreaterEqual,
        ] {
            let mut arena = Arena::new();
            let a = int_lit(&mut arena, 3);
            let b = int_lit(&mut arena, 5);
            let expr = binary_op(&mut arena, op, a, b);
            let body = FunctionBody::Expression(expr);
            let item = make_function_def("cmp", vec![], None, body);
            let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
            let ir = lower_program(&prog);
            assert!(
                ir.modules[0].functions[0].is_well_formed(),
                "failed for op {op:?}"
            );
        }
    }

    // ── Logic tests ─────────────────────────────────────────────────

    #[test]
    fn lower_lukasiewicz_logic_ops() {
        for op in [
            BinaryOperator::And,
            BinaryOperator::Or,
            BinaryOperator::Xor,
            BinaryOperator::Iff,
            BinaryOperator::Implies,
        ] {
            let mut arena = Arena::new();
            let a = trilean_lit(&mut arena, TrileanValue::True);
            let b = trilean_lit(&mut arena, TrileanValue::Unknown);
            let expr = binary_op(&mut arena, op, a, b);
            let body = FunctionBody::Expression(expr);
            let item = make_function_def("logic", vec![], None, body);
            let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
            let ir = lower_program(&prog);
            assert!(
                ir.modules[0].functions[0].is_well_formed(),
                "failed for op {op:?}"
            );
        }
    }

    #[test]
    fn lower_kleene_logic_ops() {
        for op in [
            BinaryOperator::KleeneImplies,
            BinaryOperator::KleeneXor,
            BinaryOperator::KleeneIff,
        ] {
            let mut arena = Arena::new();
            let a = trilean_lit(&mut arena, TrileanValue::False);
            let b = trilean_lit(&mut arena, TrileanValue::Unknown);
            let expr = binary_op(&mut arena, op, a, b);
            let body = FunctionBody::Expression(expr);
            let item = make_function_def("kleene", vec![], None, body);
            let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
            let ir = lower_program(&prog);
            assert!(
                ir.modules[0].functions[0].is_well_formed(),
                "failed for op {op:?}"
            );
        }
    }

    // ── Let + identifier tests ──────────────────────────────────────

    #[test]
    fn lower_let_binding_and_reference() {
        let mut arena = Arena::new();
        let val = int_lit(&mut arena, 100);
        let id = ident(&mut arena, "x");
        let stmt_id = arena.alloc_statement(Spanned::new(
            Stmt::Let {
                name: "x".to_owned(),
                mutable: false,
                type_annotation: None,
                value: val,
            },
            0..10,
        ));
        let block = Block {
            statements: vec![stmt_id],
            final_expression: Some(id),
        };
        let body = FunctionBody::Block(block);
        let item = make_function_def("let_test", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
    }

    #[test]
    fn lower_assignment() {
        let mut arena = Arena::new();
        let init = int_lit(&mut arena, 0);
        let new_val = int_lit(&mut arena, 99);
        let let_stmt = arena.alloc_statement(Spanned::new(
            Stmt::Let {
                name: "n".to_owned(),
                mutable: true,
                type_annotation: None,
                value: init,
            },
            0..10,
        ));
        let assign_stmt = arena.alloc_statement(Spanned::new(
            Stmt::Assign {
                target: "n".to_owned(),
                value: new_val,
            },
            11..20,
        ));
        let id = ident(&mut arena, "n");
        let block = Block {
            statements: vec![let_stmt, assign_stmt],
            final_expression: Some(id),
        };
        let body = FunctionBody::Block(block);
        let item = make_function_def("assign_test", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    // ── Control flow tests ─────────────────────────────────────────

    #[test]
    fn lower_if_else() {
        let mut arena = Arena::new();
        let cond = trilean_lit(&mut arena, TrileanValue::True);
        let then_val = int_lit(&mut arena, 1);
        let else_val = int_lit(&mut arena, 0);
        let if_expr = arena.alloc_expression(Spanned::new(
            Expr::If {
                condition: cond,
                then_branch: Block {
                    statements: vec![],
                    final_expression: Some(then_val),
                },
                else_branch: Some(Block {
                    statements: vec![],
                    final_expression: Some(else_val),
                }),
                treat_unknown_as_false: false,
            },
            0..1,
        ));
        let body = FunctionBody::Expression(if_expr);
        let item = make_function_def("if_else", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        // Should have at least 4 blocks: entry, then, else, merge.
        assert!(
            func.blocks.len() >= 3,
            "expected >= 3 blocks, got {}",
            func.blocks.len()
        );
        assert!(func.blocks.iter().any(|b| {
            b.instructions
                .iter()
                .any(super::super::instr::Instruction::is_phi)
        }));
    }

    #[test]
    fn lower_if_question() {
        let mut arena = Arena::new();
        let cond = trilean_lit(&mut arena, TrileanValue::Unknown);
        let then_val = int_lit(&mut arena, 1);
        let ifq_expr = arena.alloc_expression(Spanned::new(
            Expr::If {
                condition: cond,
                then_branch: Block {
                    statements: vec![],
                    final_expression: Some(then_val),
                },
                else_branch: None,
                treat_unknown_as_false: true,
            },
            0..1,
        ));
        let body = FunctionBody::Expression(ifq_expr);
        let item = make_function_def("if_question", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    #[test]
    fn lower_while_loop() {
        let mut arena = Arena::new();
        let n_ref = ident(&mut arena, "n");
        let one = int_lit(&mut arena, 1);
        let cond = binary_op(&mut arena, BinaryOperator::GreaterThan, n_ref, one);
        // while n > 1: n = n - 1
        let sub = binary_op(&mut arena, BinaryOperator::Subtract, n_ref, one);
        let assign_stmt = arena.alloc_statement(Spanned::new(
            Stmt::Assign {
                target: "n".to_owned(),
                value: sub,
            },
            0..10,
        ));
        let while_stmt = arena.alloc_statement(Spanned::new(
            Stmt::While {
                condition: cond,
                body: Block {
                    statements: vec![assign_stmt],
                    final_expression: None,
                },
                treat_unknown_as_false: false,
            },
            0..50,
        ));
        let init = int_lit(&mut arena, 5);
        let let_stmt = arena.alloc_statement(Spanned::new(
            Stmt::Let {
                name: "n".to_owned(),
                mutable: true,
                type_annotation: None,
                value: init,
            },
            0..10,
        ));
        let final_ref = ident(&mut arena, "n");
        let block = Block {
            statements: vec![let_stmt, while_stmt],
            final_expression: Some(final_ref),
        };
        let body = FunctionBody::Block(block);
        let item = make_function_def("countdown", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        // Should have header, body, exit blocks.
        assert!(
            func.blocks.len() >= 3,
            "expected >= 3 blocks, got {}",
            func.blocks.len()
        );
    }

    #[test]
    fn lower_loop_with_break() {
        let mut arena = Arena::new();
        let break_stmt = arena.alloc_statement(Spanned::new(Stmt::Break(None), 0..5));
        let body_block = Block {
            statements: vec![break_stmt],
            final_expression: None,
        };
        let loop_stmt = arena.alloc_statement(Spanned::new(Stmt::Loop(body_block), 0..20));
        let block = Block {
            statements: vec![loop_stmt],
            final_expression: None,
        };
        let body = FunctionBody::Block(block);
        let item = make_function_def("infinite", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    // ── Function call tests ────────────────────────────────────────

    #[test]
    fn lower_builtin_call() {
        let mut arena = Arena::new();
        let callee = ident(&mut arena, "println");
        let arg = string_lit(&mut arena, "hi");
        let call = call_expr(&mut arena, callee, vec![arg]);
        let body = FunctionBody::Expression(call);
        let item = make_function_def("say_hi", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        assert!(
            func.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::CallBuiltin { .. }))
        );
    }

    // ── Return tests ────────────────────────────────────────────────

    #[test]
    fn lower_return_stmt() {
        let mut arena = Arena::new();
        let val = int_lit(&mut arena, 42);
        let ret_stmt = arena.alloc_statement(Spanned::new(Stmt::Return(Some(val)), 0..10));
        let block = Block {
            statements: vec![ret_stmt],
            final_expression: None,
        };
        let body = FunctionBody::Block(block);
        let item = make_function_def("ret42", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        assert!(
            func.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Ret { .. }))
        );
    }

    // ── Verifier integration ────────────────────────────────────────

    #[test]
    fn lowered_program_passes_verifier() {
        let mut arena = Arena::new();
        let a = int_lit(&mut arena, 10);
        let b = int_lit(&mut arena, 3);
        let add = binary_op(&mut arena, BinaryOperator::Add, a, b);
        let body = FunctionBody::Expression(add);
        let item = make_function_def("add", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);

        let result = crate::verify::verify_program(&ir);
        assert!(
            result.is_ok(),
            "verifier violations: {:?}",
            result.violations
        );
    }

    #[test]
    fn lowered_if_else_passes_verifier() {
        let mut arena = Arena::new();
        let cond = trilean_lit(&mut arena, TrileanValue::True);
        let tval = int_lit(&mut arena, 1);
        let fval = int_lit(&mut arena, 0);
        let if_expr = arena.alloc_expression(Spanned::new(
            Expr::If {
                condition: cond,
                then_branch: Block {
                    statements: vec![],
                    final_expression: Some(tval),
                },
                else_branch: Some(Block {
                    statements: vec![],
                    final_expression: Some(fval),
                }),
                treat_unknown_as_false: false,
            },
            0..1,
        ));
        let body = FunctionBody::Expression(if_expr);
        let item = make_function_def("branch", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);

        let result = crate::verify::verify_program(&ir);
        assert!(
            result.is_ok(),
            "verifier violations: {:?}",
            result.violations
        );
    }

    // ── Edge case: nested blocks + shadowing ──────────────────────

    #[test]
    fn lower_nested_blocks_with_shadowing() {
        let mut arena = Arena::new();
        let outer_val = int_lit(&mut arena, 1);
        let stmt_outer = arena.alloc_statement(Spanned::new(
            Stmt::Let {
                name: "x".to_owned(),
                mutable: false,
                type_annotation: None,
                value: outer_val,
            },
            0..10,
        ));
        // Inner block: `{ let x = 2; x }` — shadows outer x
        let inner_val = int_lit(&mut arena, 2);
        let inner_let = arena.alloc_statement(Spanned::new(
            Stmt::Let {
                name: "x".to_owned(),
                mutable: false,
                type_annotation: None,
                value: inner_val,
            },
            0..10,
        ));
        let inner_ref = ident(&mut arena, "x");
        let inner_block_expr = arena.alloc_expression(Spanned::new(
            Expr::Block(Block {
                statements: vec![inner_let],
                final_expression: Some(inner_ref),
            }),
            0..1,
        ));
        let block = Block {
            statements: vec![stmt_outer],
            final_expression: Some(inner_block_expr),
        };
        let body = FunctionBody::Block(block);
        let item = make_function_def("shadow", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        let result = crate::verify::verify_function(func);
        assert!(result.is_ok(), "violations: {:?}", result.violations);
    }

    #[test]
    fn lower_nested_if_else_deep() {
        let mut arena = Arena::new();
        // if true { if false { 1 } else { 2 } } else { 3 }
        let inner_then = int_lit(&mut arena, 1);
        let inner_else = int_lit(&mut arena, 2);
        let cond_false = trilean_lit(&mut arena, TrileanValue::False);
        let inner_if = arena.alloc_expression(Spanned::new(
            Expr::If {
                condition: cond_false,
                then_branch: Block {
                    statements: vec![],
                    final_expression: Some(inner_then),
                },
                else_branch: Some(Block {
                    statements: vec![],
                    final_expression: Some(inner_else),
                }),
                treat_unknown_as_false: false,
            },
            0..1,
        ));
        let cond_true = trilean_lit(&mut arena, TrileanValue::True);
        let outer_else_val = int_lit(&mut arena, 3);
        let outer_if = arena.alloc_expression(Spanned::new(
            Expr::If {
                condition: cond_true,
                then_branch: Block {
                    statements: vec![],
                    final_expression: Some(inner_if),
                },
                else_branch: Some(Block {
                    statements: vec![],
                    final_expression: Some(outer_else_val),
                }),
                treat_unknown_as_false: false,
            },
            0..1,
        ));
        let body = FunctionBody::Expression(outer_if);
        let item = make_function_def("nested_if", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        // Deep nesting creates multiple merge blocks.
        assert!(
            func.blocks.len() >= 6,
            "expected >= 6 blocks, got {}",
            func.blocks.len()
        );
        let result = crate::verify::verify_function(func);
        assert!(result.is_ok(), "violations: {:?}", result.violations);
    }

    // ── Edge case: early return ───────────────────────────────────

    #[test]
    fn lower_early_return_from_if() {
        let mut arena = Arena::new();
        let cond = trilean_lit(&mut arena, TrileanValue::True);
        let ret_val = int_lit(&mut arena, -1);
        let ret_stmt = arena.alloc_statement(Spanned::new(Stmt::Return(Some(ret_val)), 0..10));
        let else_val = int_lit(&mut arena, 0);
        let if_expr = arena.alloc_expression(Spanned::new(
            Expr::If {
                condition: cond,
                then_branch: Block {
                    statements: vec![ret_stmt],
                    final_expression: None,
                },
                else_branch: Some(Block {
                    statements: vec![],
                    final_expression: Some(else_val),
                }),
                treat_unknown_as_false: false,
            },
            0..1,
        ));
        let body = FunctionBody::Expression(if_expr);
        let item = make_function_def("early_ret", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        let result = crate::verify::verify_function(func);
        assert!(result.is_ok(), "violations: {:?}", result.violations);
    }

    // ── Edge case: multi-param function ───────────────────────────

    #[test]
    fn lower_function_with_multiple_params() {
        let mut arena = Arena::new();
        let a = ident(&mut arena, "a");
        let b = ident(&mut arena, "b");
        let add = binary_op(&mut arena, BinaryOperator::Add, a, b);
        let body = FunctionBody::Expression(add);
        let item = make_function_def(
            "sum",
            vec![
                FunctionParam {
                    name: "a".to_owned(),
                    type_annotation: arena.alloc_type(Spanned::new(
                        triet_syntax::type_ast::TypeExpr::Named("Integer".to_owned()),
                        0..7,
                    )),
                    passing: triet_syntax::item::ParameterPassing::Borrowed,
                },
                FunctionParam {
                    name: "b".to_owned(),
                    type_annotation: arena.alloc_type(Spanned::new(
                        triet_syntax::type_ast::TypeExpr::Named("Integer".to_owned()),
                        0..7,
                    )),
                    passing: triet_syntax::item::ParameterPassing::Borrowed,
                },
            ],
            None,
            body,
        );
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert_eq!(func.params.len(), 2);
        assert!(func.is_well_formed());
        let result = crate::verify::verify_function(func);
        assert!(result.is_ok(), "violations: {:?}", result.violations);
    }

    // ── Edge case: forward function reference ─────────────────────

    #[test]
    fn lower_forward_function_reference() {
        // Two functions in the same module. The first calls the second.
        let mut arena = Arena::new();
        // Function 2: just returns 42
        let f2_val = int_lit(&mut arena, 42);
        let f2 = make_function_def("helper", vec![], None, FunctionBody::Expression(f2_val));

        // Function 1: calls helper()
        let callee = ident(&mut arena, "helper");
        let call = call_expr(&mut arena, callee, vec![]);
        let f1 = make_function_def("main", vec![], None, FunctionBody::Expression(call));

        let prog = make_program(arena, vec![Spanned::new(f1, 0..1), Spanned::new(f2, 0..1)]);
        let ir = lower_program(&prog);
        assert_eq!(ir.function_count(), 2);
        // Both functions should be well-formed.
        for func in &ir.modules[0].functions {
            assert!(
                func.is_well_formed(),
                "{} not well-formed",
                func.name.as_deref().unwrap_or("?")
            );
        }
        let result = crate::verify::verify_program(&ir);
        assert!(result.is_ok(), "violations: {:?}", result.violations);
    }

    // ── Edge case: null operations ────────────────────────────────

    #[test]
    fn lower_null_operations() {
        let mut arena = Arena::new();
        // ForceUnwrap: value!!
        let inner = int_lit(&mut arena, 5);
        let unwrap = arena.alloc_expression(Spanned::new(Expr::ForceUnwrap(inner), 0..3));
        let body = FunctionBody::Expression(unwrap);
        let item = make_function_def("unwrap_test", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        assert!(
            func.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::NullUnwrap { .. }))
        );
    }

    #[test]
    fn lower_null_literal() {
        let mut arena = Arena::new();
        let null_expr = arena.alloc_expression(Spanned::new(Expr::NullLiteral, 0..4));
        let body = FunctionBody::Expression(null_expr);
        let item = make_function_def("null_test", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    // ── Edge case: struct & enum literals ─────────────────────────

    #[test]
    fn lower_struct_literal_with_fields() {
        let mut arena = Arena::new();
        let f1 = int_lit(&mut arena, 10);
        let f2 = string_lit(&mut arena, "hello");
        let struct_lit = arena.alloc_expression(Spanned::new(
            Expr::StructLiteral {
                name: "Point".to_owned(),
                fields: vec![("x".to_owned(), f1), ("label".to_owned(), f2)],
            },
            0..1,
        ));
        let body = FunctionBody::Expression(struct_lit);
        let item = make_function_def("make_point", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        assert!(
            func.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::StructNew { .. }))
        );
    }

    #[test]
    fn lower_enum_literal_with_payload() {
        let mut arena = Arena::new();
        let payload = int_lit(&mut arena, 42);
        let enum_lit = arena.alloc_expression(Spanned::new(
            Expr::EnumLiteral {
                name: "Option".to_owned(),
                variant_name: "Some".to_owned(),
                payload: Some(payload),
            },
            0..1,
        ));
        let body = FunctionBody::Expression(enum_lit);
        let item = make_function_def("make_some", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        assert!(
            func.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::EnumNew { .. }))
        );
    }

    #[test]
    fn lower_enum_literal_unit_variant() {
        let mut arena = Arena::new();
        let enum_lit = arena.alloc_expression(Spanned::new(
            Expr::EnumLiteral {
                name: "Option".to_owned(),
                variant_name: "None".to_owned(),
                payload: None,
            },
            0..1,
        ));
        let body = FunctionBody::Expression(enum_lit);
        let item = make_function_def("make_none", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    // ── Edge case: block without final expression ─────────────────

    #[test]
    fn lower_block_without_final_expression_yields_unit() {
        let mut arena = Arena::new();
        let lit = int_lit(&mut arena, 1);
        let stmt = arena.alloc_statement(Spanned::new(Stmt::ExprStmt(lit), 0..5));
        let block_expr = arena.alloc_expression(Spanned::new(
            Expr::Block(Block {
                statements: vec![stmt],
                final_expression: None,
            }),
            0..1,
        ));
        let body = FunctionBody::Expression(block_expr);
        let item = make_function_def("unit_block", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        let result = crate::verify::verify_function(func);
        assert!(result.is_ok(), "violations: {:?}", result.violations);
    }

    // ── Edge case: while? loop ────────────────────────────────────

    #[test]
    fn lower_while_question_loop() {
        let mut arena = Arena::new();
        let n_ref = ident(&mut arena, "n");
        let one = int_lit(&mut arena, 1);
        let cond = binary_op(&mut arena, BinaryOperator::GreaterThan, n_ref, one);
        let sub = binary_op(&mut arena, BinaryOperator::Subtract, n_ref, one);
        let assign_stmt = arena.alloc_statement(Spanned::new(
            Stmt::Assign {
                target: "n".to_owned(),
                value: sub,
            },
            0..10,
        ));
        let while_stmt = arena.alloc_statement(Spanned::new(
            Stmt::While {
                condition: cond,
                body: Block {
                    statements: vec![assign_stmt],
                    final_expression: None,
                },
                treat_unknown_as_false: true, // while?
            },
            0..50,
        ));
        let init = int_lit(&mut arena, 5);
        let let_stmt = arena.alloc_statement(Spanned::new(
            Stmt::Let {
                name: "n".to_owned(),
                mutable: true,
                type_annotation: None,
                value: init,
            },
            0..10,
        ));
        let block = Block {
            statements: vec![let_stmt, while_stmt],
            final_expression: Some(ident(&mut arena, "n")),
        };
        let body = FunctionBody::Block(block);
        let item = make_function_def("whileq_test", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        let result = crate::verify::verify_function(func);
        assert!(result.is_ok(), "violations: {:?}", result.violations);
    }

    // ── Edge case: nested loops ───────────────────────────────────

    #[test]
    fn lower_nested_loops() {
        let mut arena = Arena::new();
        // inner: loop { break }
        let inner_break = arena.alloc_statement(Spanned::new(Stmt::Break(None), 0..5));
        let inner_body = Block {
            statements: vec![inner_break],
            final_expression: None,
        };
        let inner_loop = arena.alloc_statement(Spanned::new(Stmt::Loop(inner_body), 0..20));
        // outer: loop { inner_loop; break }
        let outer_break = arena.alloc_statement(Spanned::new(Stmt::Break(None), 0..5));
        let outer_body = Block {
            statements: vec![inner_loop, outer_break],
            final_expression: None,
        };
        let outer_loop = arena.alloc_statement(Spanned::new(Stmt::Loop(outer_body), 0..30));
        let block = Block {
            statements: vec![outer_loop],
            final_expression: None,
        };
        let body = FunctionBody::Block(block);
        let item = make_function_def("nested_loop", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    // ── Edge case: break with value ───────────────────────────────

    #[test]
    fn lower_break_with_value() {
        let mut arena = Arena::new();
        let val = int_lit(&mut arena, 42);
        let break_stmt = arena.alloc_statement(Spanned::new(Stmt::Break(Some(val)), 0..10));
        let loop_body = Block {
            statements: vec![break_stmt],
            final_expression: None,
        };
        let loop_stmt = arena.alloc_statement(Spanned::new(Stmt::Loop(loop_body), 0..20));
        let block = Block {
            statements: vec![loop_stmt],
            final_expression: None,
        };
        let body = FunctionBody::Block(block);
        let item = make_function_def("break_val", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    // ── Edge case: cross-module call ──────────────────────────────

    #[test]
    fn lower_cross_module_call_via_bindings() {
        let mut arena = Arena::new();
        let callee = ident(&mut arena, "external_func");
        let arg = int_lit(&mut arena, 1);
        let call = call_expr(&mut arena, callee, vec![arg]);
        let body = FunctionBody::Expression(call);
        let item = make_function_def("wrapper", vec![], None, body);

        let root_path = ModulePath::crate_root();
        let mut bindings = HashMap::new();
        bindings.insert(
            "external_func".to_owned(),
            AbsolutePath::new(
                ModulePath::new(vec!["std".to_owned(), "lib".to_owned()]),
                "external_func".to_owned(),
            ),
        );
        let module = Module {
            path: root_path,
            source_path: None,
            arena_id: triet_modules::ArenaId(0),
            items: vec![Spanned::new(item, 0..1)],
            bindings,
            parent: None,
            children: Vec::new(),
        };
        let prog = ResolvedProgram {
            arenas: vec![arena],
            modules: vec![module],
            root: ModuleId(0),
        };
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        assert!(
            func.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::CallCrossModule { .. }))
        );
    }

    // ── Edge case: arithmetic with edge values ────────────────────

    #[test]
    fn lower_large_integer_values() {
        let mut arena = Arena::new();
        let big = int_lit(&mut arena, i128::MAX); // 2^127 - 1
        let neg_big = int_lit(&mut arena, i128::MIN); // -2^127
        let add = binary_op(&mut arena, BinaryOperator::Add, big, neg_big);
        let body = FunctionBody::Expression(add);
        let item = make_function_def("big_add", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    #[test]
    fn lower_negative_integer_literal() {
        let mut arena = Arena::new();
        let neg = int_lit(&mut arena, -5);
        let body = FunctionBody::Expression(neg);
        let item = make_function_def("negative", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    // ── Edge case: all logic ops on mixed Trilean values ──────────

    #[test]
    fn lower_all_lukasiewicz_ops_mixed_truth_values() {
        // Test every Ł3 op with all 9 combinations of (True, False, Unknown)
        let values = [
            TrileanValue::True,
            TrileanValue::False,
            TrileanValue::Unknown,
        ];
        let ops = [
            BinaryOperator::And,
            BinaryOperator::Or,
            BinaryOperator::Xor,
            BinaryOperator::Iff,
            BinaryOperator::Implies,
        ];
        for op in &ops {
            for &a in &values {
                for &b in &values {
                    let mut arena = Arena::new();
                    let lhs = trilean_lit(&mut arena, a);
                    let rhs = trilean_lit(&mut arena, b);
                    let expr = binary_op(&mut arena, *op, lhs, rhs);
                    let body = FunctionBody::Expression(expr);
                    let item = make_function_def("l3_op", vec![], None, body);
                    let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
                    let ir = lower_program(&prog);
                    let func = &ir.modules[0].functions[0];
                    assert!(func.is_well_formed(), "failed for {op:?} with {a:?} {b:?}");
                    let result = crate::verify::verify_function(func);
                    assert!(
                        result.is_ok(),
                        "verifier failed for {op:?} with {a:?} {b:?}: {:?}",
                        result.violations
                    );
                }
            }
        }
    }

    // ── Edge case: all comparison ops ─────────────────────────────

    #[test]
    fn lower_all_comparison_ops_with_equal_values() {
        for op in [
            BinaryOperator::Equal,
            BinaryOperator::NotEqual,
            BinaryOperator::LessThan,
            BinaryOperator::LessEqual,
            BinaryOperator::GreaterThan,
            BinaryOperator::GreaterEqual,
        ] {
            let mut arena = Arena::new();
            let a = int_lit(&mut arena, 7);
            let b = int_lit(&mut arena, 7);
            let expr = binary_op(&mut arena, op, a, b);
            let body = FunctionBody::Expression(expr);
            let item = make_function_def("cmp_eq", vec![], None, body);
            let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
            let ir = lower_program(&prog);
            let func = &ir.modules[0].functions[0];
            assert!(func.is_well_formed(), "failed for {op:?} with 7 vs 7");
            let result = crate::verify::verify_function(func);
            assert!(
                result.is_ok(),
                "verifier failed for {op:?}: {:?}",
                result.violations
            );
        }
    }

    // ── Edge case: safe field access ──────────────────────────────

    #[test]
    fn lower_safe_field_access() {
        let mut arena = Arena::new();
        let obj = ident(&mut arena, "obj");
        let safe_access = arena.alloc_expression(Spanned::new(
            Expr::SafeFieldAccess {
                object: obj,
                field: "value".to_owned(),
            },
            0..1,
        ));
        let body = FunctionBody::Expression(safe_access);
        let item = make_function_def("safe_get", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        // Should have NullCheck + NullUnwrap + FieldGet.
        let has_null_check = func.blocks[0]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::NullCheck { .. }));
        assert!(has_null_check, "SafeFieldAccess should emit NullCheck");
    }

    // ── Edge case: elvis operator ─────────────────────────────────

    #[test]
    fn lower_elvis_operator() {
        let mut arena = Arena::new();
        let obj = int_lit(&mut arena, 10);
        let default = int_lit(&mut arena, 0);
        let elvis = arena.alloc_expression(Spanned::new(
            Expr::ElvisOp {
                object: obj,
                default,
            },
            0..1,
        ));
        let body = FunctionBody::Expression(elvis);
        let item = make_function_def("elvis_test", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    // ── Edge case: complex block with multiple statement types ────

    #[test]
    fn lower_complex_block_with_mixed_statements() {
        let mut arena = Arena::new();
        let init = int_lit(&mut arena, 0);
        let let_stmt = arena.alloc_statement(Spanned::new(
            Stmt::Let {
                name: "sum".to_owned(),
                mutable: true,
                type_annotation: None,
                value: init,
            },
            0..10,
        ));
        let sum_ref = ident(&mut arena, "sum");
        let one = int_lit(&mut arena, 1);
        let add = binary_op(&mut arena, BinaryOperator::Add, sum_ref, one);
        let assign_stmt = arena.alloc_statement(Spanned::new(
            Stmt::Assign {
                target: "sum".to_owned(),
                value: add,
            },
            0..10,
        ));
        let println_id = ident(&mut arena, "println");
        let call_expr = call_expr(&mut arena, println_id, vec![add]);
        let expr_stmt = arena.alloc_statement(Spanned::new(Stmt::ExprStmt(call_expr), 0..5));
        let final_ref = ident(&mut arena, "sum");
        let block = Block {
            statements: vec![let_stmt, assign_stmt, expr_stmt],
            final_expression: Some(final_ref),
        };
        let body = FunctionBody::Block(block);
        let item = make_function_def("complex_block", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    // ── Edge case: if without else ────────────────────────────────

    #[test]
    fn lower_if_without_else_produces_unit() {
        let mut arena = Arena::new();
        let cond = trilean_lit(&mut arena, TrileanValue::True);
        let then_val = int_lit(&mut arena, 1);
        let if_expr = arena.alloc_expression(Spanned::new(
            Expr::If {
                condition: cond,
                then_branch: Block {
                    statements: vec![],
                    final_expression: Some(then_val),
                },
                else_branch: None,
                treat_unknown_as_false: false,
            },
            0..1,
        ));
        let body = FunctionBody::Expression(if_expr);
        let item = make_function_def("if_no_else", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        let result = crate::verify::verify_function(func);
        assert!(result.is_ok(), "violations: {:?}", result.violations);
    }

    // ── Edge case: const statement ────────────────────────────────

    #[test]
    fn lower_const_statement() {
        let mut arena = Arena::new();
        let val = int_lit(&mut arena, 100);
        let const_stmt = arena.alloc_statement(Spanned::new(
            Stmt::Const {
                name: "MAX".to_owned(),
                type_annotation: None,
                value: val,
            },
            0..10,
        ));
        let id = ident(&mut arena, "MAX");
        let block = Block {
            statements: vec![const_stmt],
            final_expression: Some(id),
        };
        let body = FunctionBody::Block(block);
        let item = make_function_def("use_const", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    // ── Edge case: tuple literal ───────────────────────────────────

    #[test]
    fn lower_tuple_literal() {
        let mut arena = Arena::new();
        let a = int_lit(&mut arena, 1);
        let b = int_lit(&mut arena, 2);
        let c = int_lit(&mut arena, 3);
        let tuple = arena.alloc_expression(Spanned::new(Expr::Tuple(vec![a, b, c]), 0..1));
        let body = FunctionBody::Expression(tuple);
        let item = make_function_def("make_tuple", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        assert!(
            func.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::StructNew { .. }))
        );
    }

    // ── Edge case: field access ────────────────────────────────────

    #[test]
    fn lower_field_access() {
        let mut arena = Arena::new();
        let obj = ident(&mut arena, "point");
        let field_access = arena.alloc_expression(Spanned::new(
            Expr::FieldAccess {
                object: obj,
                field: "x".to_owned(),
            },
            0..1,
        ));
        let body = FunctionBody::Expression(field_access);
        let item = make_function_def("get_x", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        assert!(
            func.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::FieldGet { .. }))
        );
    }

    // ── Edge case: method call ─────────────────────────────────────

    #[test]
    fn lower_method_call() {
        let mut arena = Arena::new();
        let receiver = ident(&mut arena, "s");
        let method_call = arena.alloc_expression(Spanned::new(
            Expr::MethodCall {
                receiver,
                method: "len".to_owned(),
                arguments: vec![],
            },
            0..1,
        ));
        let body = FunctionBody::Expression(method_call);
        let item = make_function_def("get_len", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    // ── Edge case: range expression ────────────────────────────────

    #[test]
    fn lower_range_expression() {
        let mut arena = Arena::new();
        let start = int_lit(&mut arena, 0);
        let end = int_lit(&mut arena, 10);
        let range = arena.alloc_expression(Spanned::new(
            Expr::Range {
                start,
                end,
                inclusive: false,
            },
            0..1,
        ));
        let body = FunctionBody::Expression(range);
        let item = make_function_def("make_range", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        assert!(ir.modules[0].functions[0].is_well_formed());
    }

    // ── Edge case: multiple modules ───────────────────────────────

    #[test]
    fn lower_multiple_modules() {
        // Module 0: crate root with main()
        let mut arena0 = Arena::new();
        let val = int_lit(&mut arena0, 42);
        let main_item = make_function_def("main", vec![], None, FunctionBody::Expression(val));

        let root_path = ModulePath::crate_root();
        let module0 = Module {
            path: root_path.clone(),
            source_path: None,
            arena_id: triet_modules::ArenaId(0),
            items: vec![Spanned::new(main_item, 0..1)],
            bindings: HashMap::new(),
            parent: None,
            children: vec![ModuleId(1)],
        };

        // Module 1: crate.utils with helper()
        let mut arena1 = Arena::new();
        let helper_val = int_lit(&mut arena1, 7);
        let helper_item =
            make_function_def("helper", vec![], None, FunctionBody::Expression(helper_val));

        let module1 = Module {
            path: root_path.child("utils"),
            source_path: None,
            arena_id: triet_modules::ArenaId(1),
            items: vec![Spanned::new(helper_item, 0..1)],
            bindings: HashMap::new(),
            parent: Some(ModuleId(0)),
            children: Vec::new(),
        };

        let prog = ResolvedProgram {
            arenas: vec![arena0, arena1],
            modules: vec![module0, module1],
            root: ModuleId(0),
        };
        let ir = lower_program(&prog);
        assert_eq!(ir.modules.len(), 2);
        assert_eq!(ir.function_count(), 2);
        let result = crate::verify::verify_program(&ir);
        assert!(result.is_ok(), "violations: {:?}", result.violations);
    }

    // ── Edge case: function with type annotation ──────────────────

    #[test]
    fn lower_function_with_return_type_annotation() {
        let mut arena = Arena::new();
        let val = int_lit(&mut arena, 0);
        let ret_type = arena.alloc_type(Spanned::new(
            triet_syntax::type_ast::TypeExpr::Named("Integer".to_owned()),
            0..7,
        ));
        let item = Item::Function(FunctionDef {
            visibility: triet_syntax::visibility::Visibility::Private,
            name: "get_zero".to_owned(),
            type_params: Vec::new(),
            parameters: vec![],
            return_type: Some(ret_type),
            body: FunctionBody::Expression(val),
        });
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert_eq!(func.return_type, TypeTag::Integer);
        assert!(func.is_well_formed());
    }

    // ── Edge case: empty program ───────────────────────────────────

    #[test]
    fn lower_empty_program_yields_empty_ir() {
        let root_path = ModulePath::crate_root();
        let module = Module {
            path: root_path,
            source_path: None,
            arena_id: triet_modules::ArenaId(0),
            items: vec![],
            bindings: HashMap::new(),
            parent: None,
            children: Vec::new(),
        };
        let prog = ResolvedProgram {
            arenas: vec![Arena::new()],
            modules: vec![module],
            root: ModuleId(0),
        };
        let ir = lower_program(&prog);
        assert!(ir.is_empty());
        assert_eq!(ir.function_count(), 0);
    }
}
