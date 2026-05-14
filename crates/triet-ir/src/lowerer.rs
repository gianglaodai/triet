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
    expr::{BinaryOperator, Expr, MatchArm},
    item::{FunctionBody, FunctionDef, Item},
    numeric::{NumericSuffix, TrileanValue},
    stmt::{Block, Stmt},
    Arena,
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
    // Pass 1: assign FuncIds
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

    /// Get the current module's arena.
    fn arena(&self) -> &'a Arena {
        let module = &self.program.modules[self.current_module_idx];
        &self.program.arenas[module.arena_id.0]
    }

    /// Get the current module.
    fn current_module(&self) -> &'a Module {
        &self.program.modules[self.current_module_idx]
    }

    // ── Function registry ───────────────────────────────────────

    fn declare_function(&mut self, module: &Module, fd: &FunctionDef) {
        let path = AbsolutePath::new(module.path.clone(), fd.name.clone());
        let id = FuncId(self.func_table.len() as u32);
        self.func_table.insert(path, id);
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
                _ => TypeTag::Unit, // user-defined type — placeholder
            },
            triet_syntax::type_ast::TypeExpr::Nullable(inner) => {
                TypeTag::Nullable(Box::new(self.type_expr_to_tag(*inner)))
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
        self.current_func_id = func_id;
        self.current_return_type = fd
            .return_type
            .map_or(TypeTag::Unit, |t| self.type_expr_to_tag(t));

        // Push the outermost function scope.
        self.push_scope();

        // Entry block.
        let entry_id = self.fresh_block();
        self.start_block(entry_id, Some("entry".into()));

        // Allocate parameter ValueIds.
        let mut param_specs: Vec<(String, TypeTag)> = Vec::new();
        for p in &fd.parameters {
            let v = self.fresh_value();
            let pty = self.type_expr_to_tag(p.type_annotation);
            self.params.insert(p.name.clone(), v);
            self.bind_var(p.name.clone(), v);
            param_specs.push((p.name.clone(), pty));
        }

        // Lower the function body.
        let block_result = match &fd.body {
            FunctionBody::Block(block) => {
                Some(self.lower_block(block))
            }
            FunctionBody::Expression(expr_id) => {
                Some(self.lower_expr(*expr_id))
            }
        };

        // If the current block doesn't have a terminator yet, emit Ret.
        // (The body may have already emitted Ret/Return via an explicit
        // `return` statement.)
        if let Some(v) = block_result {
            if self.blocks[&self.current_block]
                .terminator()
                .is_none()
            {
                self.emit(Instruction::Ret {
                    value: Some(Operand::Value(v)),
                });
            }
        } else if self.blocks[&self.current_block]
            .terminator()
            .is_none()
        {
            self.emit(Instruction::Ret { value: None });
        }

        self.pop_scope();

        let mut func = Function::new(func_id, Some(fd.name.clone()), param_specs, self.current_return_type.clone());
        func.blocks = std::mem::take(&mut self.blocks)
            .into_values()
            .collect();
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
                // Check if target is in scope; if mutable, rebind.
                if let Some(_old) = self.resolve_var(target) {
                    self.bind_var(target.clone(), new_val);
                }
                // else: should have been caught by typecheck; ignore.
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
                    TypeTag::Long => {
                        Constant::Long(Long::from_i128(*value))
                    }
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
                let const_id = self.intern_constant(Constant::Integer(Integer::new(i64_val).unwrap()));
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
                // Null is lowered as a NullCheck-tagged Unit; the consumer
                // knows from context that this is nullable.
                let const_id = self.intern_constant(Constant::Unit);
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
                // Look up as a function reference — defer to call sites.
                // For now, return a placeholder.
                // This should have been caught by typecheck; for correct IR
                // the identifier should only appear in Call callee position.
                
                self.fresh_value()
            }

            Expr::FieldAccess { object, field } => {
                let obj_val = self.lower_expr(*object);
                let dest = self.fresh_value();
                self.emit(Instruction::FieldGet {
                    dest,
                    object: Operand::Value(obj_val),
                    field_idx: field_name_to_idx(field),
                });
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
            Expr::Call { callee, arguments } => {
                self.lower_call(*callee, arguments)
            }

            Expr::MethodCall {
                receiver,
                method: _method,
                arguments: _args,
            } => {
                // Method calls deferred to v0.3.3.
                self.lower_expr(*receiver)
            }

            // ── Arithmetic ───────────────────────────────────────
            Expr::BinaryOp { operator, left, right } => {
                let lhs = self.lower_expr(*left);
                let rhs = self.lower_expr(*right);
                let dest = self.fresh_value();
                let instr = Self::lower_binary_op(*operator, dest, lhs, rhs);
                self.emit(instr);
                dest
            }

            Expr::UnaryOp { operator: _op, operand } => {
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
                let dest = self.fresh_value();
                self.emit(Instruction::FieldGet {
                    dest,
                    object: Operand::Value(unwrapped),
                    field_idx: field_name_to_idx(field),
                });
                dest
            }

            Expr::SafeMethodCall {
                receiver,
                method: _method,
                arguments: _args,
            } => {
                // Simplified: just return receiver for now.
                self.lower_expr(*receiver)
            }

            Expr::ElvisOp { object, default } => {
                let obj_val = self.lower_expr(*object);
                let check = self.fresh_value();
                self.emit(Instruction::NullCheck {
                    dest: check,
                    nullable: Operand::Value(obj_val),
                });
                let unwrapped = self.fresh_value();
                self.emit(Instruction::NullUnwrap {
                    dest: unwrapped,
                    nullable: Operand::Value(obj_val),
                });
                // For now, simplified: always return the unwrapped value.
                // Full elvis lowering needs conditional blocks.
                let _default_val = self.lower_expr(*default);
                unwrapped
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
            } => {
                self.lower_if_expr(*condition, then_branch, else_branch.as_ref(), *treat_unknown_as_false)
            }

            Expr::Match { scrutinee, arms } => {
                self.lower_match_expr(*scrutinee, arms)
            }

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

            Expr::StructLiteral { name: _name, fields } => {
                let field_values: Vec<Operand> = fields
                    .iter()
                    .map(|(_n, e)| Operand::Value(self.lower_expr(*e)))
                    .collect();
                let dest = self.fresh_value();
                self.emit(Instruction::StructNew {
                    dest,
                    fields: field_values,
                });
                dest
            }

            Expr::EnumLiteral {
                name: _name,
                variant_name: _vname,
                payload,
            } => {
                let payload_op = payload.map(|e| Operand::Value(self.lower_expr(e)));
                let dest = self.fresh_value();
                // variant_idx = 0 placeholder; real index from typechecker.
                self.emit(Instruction::EnumNew {
                    dest,
                    variant_idx: 0,
                    payload: payload_op,
                });
                dest
            }
        }
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
            self.emit(Instruction::Const {
                dest,
                constant: c,
            });
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

        let then_block_id = self.fresh_block();
        let else_block_id = self.fresh_block();
        let merge_block_id = self.fresh_block();

        // Both `if` and `if?` currently lower to `BrIf` (unknown → else branch).
        // SPEC distinguishes them: `if?` treats unknown-as-false; plain `if`
        // should raise when the condition is unknown. The strict check for
        // plain `if` will be added when `trilean_assert_known` lands (deferred
        // to a later phase; tracked via the `treat_unknown_as_false` flag
        // already plumbed through here so the call sites don't need to change).
        let _ = treat_unknown_as_false;
        self.emit(Instruction::BrIf {
            cond: Operand::Value(cond_val),
            then_block: then_block_id,
            else_block: else_block_id,
        });

        // Then block.
        self.start_block(then_block_id, Some("then".into()));
        let then_val = self.lower_block(then_branch);
        // Only branch if the block didn't already terminate.
        if self.blocks[&self.current_block]
            .terminator()
            .is_none()
        {
            self.emit(Instruction::Br {
                target: merge_block_id,
            });
        }
        let then_end = self.current_block;

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
        if self.blocks[&self.current_block]
            .terminator()
            .is_none()
        {
            self.emit(Instruction::Br {
                target: merge_block_id,
            });
        }
        let else_end = self.current_block;

        // Merge block with phi.
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

        // Branch to the header.
        self.emit(Instruction::Br {
            target: header_id,
        });

        // Loop header: evaluate condition.
        self.start_block(header_id, Some("while_header".into()));
        let cond_val = self.lower_expr(condition);

        // Both `while` and `while?` lower to `BrIf` (unknown → exit). The
        // strict-`while` check on unknown will be added with the same
        // `trilean_assert_known` helper used for plain `if` (deferred); the
        // distinction is preserved via `treat_unknown_as_false` so callers
        // remain stable when the strict check lands.
        let _ = treat_unknown_as_false;
        self.emit(Instruction::BrIf {
            cond: Operand::Value(cond_val),
            then_block: body_id,
            else_block: exit_id,
        });

        // Loop body.
        self.start_block(body_id, Some("while_body".into()));
        self.loop_stack.push(LoopContext {
            break_target: exit_id,
            continue_target: header_id,
        });
        self.push_scope();
        self.lower_block(body);
        self.pop_scope();
        self.loop_stack.pop();
        // Only branch back if not already terminated.
        if self.blocks[&self.current_block]
            .terminator()
            .is_none()
        {
            self.emit(Instruction::Br {
                target: header_id,
            });
        }

        // Exit block.
        self.start_block(exit_id, Some("while_exit".into()));
    }

    fn lower_loop_stmt(&mut self, body: &Block) {
        let body_id = self.fresh_block();
        let exit_id = self.fresh_block();

        self.emit(Instruction::Br {
            target: body_id,
        });

        self.start_block(body_id, Some("loop_body".into()));
        self.loop_stack.push(LoopContext {
            break_target: exit_id,
            continue_target: body_id,
        });
        self.push_scope();
        self.lower_block(body);
        self.pop_scope();
        self.loop_stack.pop();
        if self.blocks[&self.current_block]
            .terminator()
            .is_none()
        {
            self.emit(Instruction::Br {
                target: body_id,
            });
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

        // Check if the iterable is a Range expression for proper counted loop.
        let spanned = &self.arena().expression(iterable);
        let is_range = matches!(&spanned.node, Expr::Range { .. });

        if is_range {
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
                self.emit(Instruction::Br {
                    target: header_id,
                });

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
                self.emit(Instruction::BrIf {
                    cond: Operand::Value(cmp_dest),
                    then_block: body_id,
                    else_block: exit_id,
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
                if self.blocks[&self.current_block]
                    .terminator()
                    .is_none()
                {
                    // Increment loop var.
                    let inc_dest = self.fresh_value();
                    let c1 = self.intern_constant(Constant::Integer(triet_core::Integer::new(1).unwrap()));
                    self.emit(Instruction::Add {
                        dest: inc_dest,
                        lhs: Operand::Value(phi_val),
                        rhs: Operand::Const(c1),
                    });
                    self.emit(Instruction::Br {
                        target: header_id,
                    });
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
            self.emit(Instruction::Br {
                target: header_id,
            });

            self.start_block(header_id, Some("for_header".into()));
            // Always enter body once.
            self.emit(Instruction::Br {
                target: body_id,
            });

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
            if self.blocks[&self.current_block]
                .terminator()
                .is_none()
            {
                self.emit(Instruction::Br {
                    target: exit_id,
                });
            }
        }

        self.start_block(exit_id, Some("for_exit".into()));
    }

    fn lower_match_expr(
        &mut self,
        scrutinee: triet_syntax::arena::ExprId,
        arms: &[MatchArm],
    ) -> ValueId {
        let _scrutee_val = self.lower_expr(scrutinee);
        if arms.is_empty() {
            let c = self.intern_constant(Constant::Unit);
            let d = self.fresh_value();
            self.emit(Instruction::Const {
                dest: d,
                constant: c,
            });
            return d;
        }

        let merge_block_id = self.fresh_block();
        let merge_dest = self.fresh_value();
        let mut phi_incoming: Vec<PhiIncoming> = Vec::new();

        for arm in arms {
            let arm_block_id = self.fresh_block();

            // For now, simplified match lowering: each arm is unconditional
            // (pattern exhaustiveness checked by typechecker).
            // TODO(v0.3.4): emit tag checks and conditional branches.

            self.start_block(arm_block_id, Some("match_arm".into()));
            self.push_scope();
            // Bind pattern variables (simplified).
            let arm_val = self.lower_expr(arm.body);
            self.pop_scope();

            phi_incoming.push(PhiIncoming {
                value: arm_val,
                block: arm_block_id,
            });

            if self.blocks[&self.current_block]
                .terminator()
                .is_none()
            {
                self.emit(Instruction::Br {
                    target: merge_block_id,
                });
            }
        }

        // Merge block.
        self.start_block(merge_block_id, Some("match_merge".into()));
        self.emit(Instruction::Phi {
            dest: merge_dest,
            incoming: phi_incoming,
        });
        merge_dest
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

                // Check function table.
                if let Some(func_id) = self.resolve_func(name) {
                    let dest = self.fresh_value();
                    self.emit(Instruction::CallLocal {
                        dest: Some(dest),
                        callee: func_id,
                        args,
                    });
                    return dest;
                }

                // Cross-module call via bindings.
                if let Some(abs_path) = self.current_module().bindings.get(name) {
                    let dest = self.fresh_value();
                    self.emit(Instruction::CallCrossModule {
                        dest: Some(dest),
                        path: abs_path.clone(),
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
        _ => None,
    }
}

/// Convert a field name to a deterministic index. In practice, the
/// lowerer looks up struct field indices from the typechecker. For now,
/// this is a placeholder; the typechecker wiring (v0.3.5) provides the
/// real index.
const fn field_name_to_idx(_name: &str) -> u32 {
    // Placeholder — always 0. Real index comes from struct definition
    // when typechecker integration lands.
    0
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use triet_modules::{Module, ModuleId, ModulePath};
    use triet_syntax::{
        arena::Arena,
        arena::ExprId,
        expr::Expr,
        item::{FunctionBody, FunctionDef, FunctionParam, Item},
        numeric::TrileanValue,
        stmt::{Block, Stmt},
        Spanned,
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
            parameters: params,
            return_type,
            body,
        })
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
        let tlit = arena.alloc_expression(Spanned::new(
            Expr::TernaryLiteral { value: 42 },
            0..1,
        ));
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
        assert!(entry.instructions.iter().any(|i| matches!(i, Instruction::Add { .. })));
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
            assert!(ir.modules[0].functions[0].is_well_formed(), "failed for op {op:?}");
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
        assert!(func.blocks[0].instructions.iter().any(|i| matches!(i, Instruction::Neg { .. })));
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
            assert!(ir.modules[0].functions[0].is_well_formed(), "failed for op {op:?}");
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
            assert!(ir.modules[0].functions[0].is_well_formed(), "failed for op {op:?}");
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
            assert!(ir.modules[0].functions[0].is_well_formed(), "failed for op {op:?}");
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
        assert!(func.blocks.len() >= 3, "expected >= 3 blocks, got {}", func.blocks.len());
        assert!(func.blocks.iter().any(|b| b.instructions.iter().any(super::super::instr::Instruction::is_phi)));
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
        assert!(func.blocks.len() >= 3, "expected >= 3 blocks, got {}", func.blocks.len());
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
        assert!(func.blocks[0].instructions.iter().any(|i| matches!(i, Instruction::CallBuiltin { .. })));
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
        assert!(func.blocks[0].instructions.iter().any(|i| matches!(i, Instruction::Ret { .. })));
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
        assert!(func.blocks.len() >= 6, "expected >= 6 blocks, got {}", func.blocks.len());
        let result = crate::verify::verify_function(func);
        assert!(result.is_ok(), "violations: {:?}", result.violations);
    }

    // ── Edge case: early return ───────────────────────────────────

    #[test]
    fn lower_early_return_from_if() {
        let mut arena = Arena::new();
        let cond = trilean_lit(&mut arena, TrileanValue::True);
        let ret_val = int_lit(&mut arena, -1);
        let ret_stmt = arena.alloc_statement(Spanned::new(
            Stmt::Return(Some(ret_val)),
            0..10,
        ));
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

        let prog = make_program(
            arena,
            vec![
                Spanned::new(f1, 0..1),
                Spanned::new(f2, 0..1),
            ],
        );
        let ir = lower_program(&prog);
        assert_eq!(ir.function_count(), 2);
        // Both functions should be well-formed.
        for func in &ir.modules[0].functions {
            assert!(func.is_well_formed(), "{} not well-formed", func.name.as_deref().unwrap_or("?"));
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
        let unwrap = arena.alloc_expression(Spanned::new(
            Expr::ForceUnwrap(inner),
            0..3,
        ));
        let body = FunctionBody::Expression(unwrap);
        let item = make_function_def("unwrap_test", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        assert!(func.blocks[0].instructions.iter().any(|i| matches!(i, Instruction::NullUnwrap { .. })));
    }

    #[test]
    fn lower_null_literal() {
        let mut arena = Arena::new();
        let null_expr = arena.alloc_expression(Spanned::new(
            Expr::NullLiteral,
            0..4,
        ));
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
        assert!(func.blocks[0].instructions.iter().any(|i| matches!(i, Instruction::StructNew { .. })));
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
        assert!(func.blocks[0].instructions.iter().any(|i| matches!(i, Instruction::EnumNew { .. })));
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
        let stmt = arena.alloc_statement(Spanned::new(
            Stmt::ExprStmt(lit),
            0..5,
        ));
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
            Stmt::Assign { target: "n".to_owned(), value: sub },
            0..10,
        ));
        let while_stmt = arena.alloc_statement(Spanned::new(
            Stmt::While {
                condition: cond,
                body: Block { statements: vec![assign_stmt], final_expression: None },
                treat_unknown_as_false: true, // while?
            },
            0..50,
        ));
        let init = int_lit(&mut arena, 5);
        let let_stmt = arena.alloc_statement(Spanned::new(
            Stmt::Let { name: "n".to_owned(), mutable: true, type_annotation: None, value: init },
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
        let inner_loop = arena.alloc_statement(Spanned::new(
            Stmt::Loop(inner_body),
            0..20,
        ));
        // outer: loop { inner_loop; break }
        let outer_break = arena.alloc_statement(Spanned::new(Stmt::Break(None), 0..5));
        let outer_body = Block {
            statements: vec![inner_loop, outer_break],
            final_expression: None,
        };
        let outer_loop = arena.alloc_statement(Spanned::new(
            Stmt::Loop(outer_body),
            0..30,
        ));
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
        let break_stmt = arena.alloc_statement(Spanned::new(
            Stmt::Break(Some(val)),
            0..10,
        ));
        let loop_body = Block {
            statements: vec![break_stmt],
            final_expression: None,
        };
        let loop_stmt = arena.alloc_statement(Spanned::new(
            Stmt::Loop(loop_body),
            0..20,
        ));
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
        assert!(func.blocks[0].instructions.iter().any(|i| matches!(i, Instruction::CallCrossModule { .. })));
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
        let values = [TrileanValue::True, TrileanValue::False, TrileanValue::Unknown];
        let ops = [
            BinaryOperator::And, BinaryOperator::Or, BinaryOperator::Xor,
            BinaryOperator::Iff, BinaryOperator::Implies,
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
                    assert!(result.is_ok(), "verifier failed for {op:?} with {a:?} {b:?}: {:?}", result.violations);
                }
            }
        }
    }

    // ── Edge case: all comparison ops ─────────────────────────────

    #[test]
    fn lower_all_comparison_ops_with_equal_values() {
        for op in [
            BinaryOperator::Equal, BinaryOperator::NotEqual,
            BinaryOperator::LessThan, BinaryOperator::LessEqual,
            BinaryOperator::GreaterThan, BinaryOperator::GreaterEqual,
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
            assert!(result.is_ok(), "verifier failed for {op:?}: {:?}", result.violations);
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
        let has_null_check = func.blocks[0].instructions.iter().any(|i| matches!(i, Instruction::NullCheck { .. }));
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
            Stmt::Let { name: "sum".to_owned(), mutable: true, type_annotation: None, value: init },
            0..10,
        ));
        let sum_ref = ident(&mut arena, "sum");
        let one = int_lit(&mut arena, 1);
        let add = binary_op(&mut arena, BinaryOperator::Add, sum_ref, one);
        let assign_stmt = arena.alloc_statement(Spanned::new(
            Stmt::Assign { target: "sum".to_owned(), value: add },
            0..10,
        ));
        let println_id = ident(&mut arena, "println");
        let call_expr = call_expr(&mut arena, println_id, vec![add]);
        let expr_stmt = arena.alloc_statement(Spanned::new(
            Stmt::ExprStmt(call_expr),
            0..5,
        ));
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
                then_branch: Block { statements: vec![], final_expression: Some(then_val) },
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
            Stmt::Const { name: "MAX".to_owned(), type_annotation: None, value: val },
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
        let tuple = arena.alloc_expression(Spanned::new(
            Expr::Tuple(vec![a, b, c]),
            0..1,
        ));
        let body = FunctionBody::Expression(tuple);
        let item = make_function_def("make_tuple", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        assert!(func.blocks[0].instructions.iter().any(|i| matches!(i, Instruction::StructNew { .. })));
    }

    // ── Edge case: field access ────────────────────────────────────

    #[test]
    fn lower_field_access() {
        let mut arena = Arena::new();
        let obj = ident(&mut arena, "point");
        let field_access = arena.alloc_expression(Spanned::new(
            Expr::FieldAccess { object: obj, field: "x".to_owned() },
            0..1,
        ));
        let body = FunctionBody::Expression(field_access);
        let item = make_function_def("get_x", vec![], None, body);
        let prog = make_program(arena, vec![Spanned::new(item, 0..1)]);
        let ir = lower_program(&prog);
        let func = &ir.modules[0].functions[0];
        assert!(func.is_well_formed());
        assert!(func.blocks[0].instructions.iter().any(|i| matches!(i, Instruction::FieldGet { .. })));
    }

    // ── Edge case: method call ─────────────────────────────────────

    #[test]
    fn lower_method_call() {
        let mut arena = Arena::new();
        let receiver = ident(&mut arena, "s");
        let method_call = arena.alloc_expression(Spanned::new(
            Expr::MethodCall { receiver, method: "len".to_owned(), arguments: vec![] },
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
            Expr::Range { start, end, inclusive: false },
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
        let helper_item = make_function_def("helper", vec![], None, FunctionBody::Expression(helper_val));

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
