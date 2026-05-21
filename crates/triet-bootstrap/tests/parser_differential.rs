//! v0.7.5.6b — `parser_differential` test (closes the v0.7.5
//! parser umbrella per [ADR-0019 §A7.5]).
//!
//! For each corpus source, runs the Rust impl
//! [`triet_parser::parse`] and the Triết-in-Triết port at
//! `compiler/parser.tri::dump_program_ndjson` over the same input.
//! Both sides emit the same line-delimited JSON shape (one node per
//! line, pre-order traversal) and the test asserts byte-equality.
//!
//! ## Format
//!
//! ```text
//! {"k":"Program","items":N}
//! [N item subtrees]
//! ```
//!
//! Each subtree is a pre-order DFS. The parent line carries its
//! span + leaf metadata + child counts; children appear immediately
//! after in declared source order. Full per-kind spec is in the
//! NDJSON dump comment block at the head of
//! `compiler/parser.tri`'s dump section.
//!
//! On parse failure (one or more `ParseError` accumulated) each
//! error emits a trailing sibling line:
//!
//! ```text
//! {"e":"<Kind>","s":[<start>,<end>],"v":"<expected>"[,"f":"<found>"]}
//! ```
//!
//! ## Corpus discipline
//!
//! The Triết-side parser at v0.7.5.6 supports the surface
//! shipped through v0.7.5.{1..5b}: 17 `Expr` variants (no
//! `Lambda` / `Match` / `If` / `Block` / `Tuple` /
//! `StructLiteral` / `EnumLiteral` / `TupleIndex` / `SafeAccess`
//! / `FStringLiteral`), 10 `Stmt` variants, 9 `Pattern` variants,
//! 7 `TypeExpr` variants, 8 `Item` variants. Corpus sources stay
//! inside that envelope so the Rust mirror has a well-defined
//! output for every kind the Triết side can produce.
//!
//! ## Char-indexed spans
//!
//! Triết-side scanner uses char-indexed spans (per Q3-A); the Rust
//! impl produces byte spans. [`byte_to_char_index`] (re-used shape
//! from `lexer_differential.rs`) translates every emitted span.
//!
//! ## Transient bridge
//!
//! NDJSON is a transient bridge format per [ADR-0019 §A2] — dropped
//! at v0.7.9 when Triết-side data flows in-memory. It exists solely
//! to make a byte-diff a tractable gate while the bootstrap is
//! incomplete.
//!
//! [ADR-0019 §A2]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md
//! [ADR-0019 §A7.5]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::OnceLock;

use miette::Diagnostic;
use triet_ir::{FuncId, IrProgram, RuntimeValue, Vm, lower_program, read_program, write_program};
use triet_modules::load_program;
use triet_parser::{ParseError, parse as rust_parse};
use triet_syntax::{
    Arena, BinaryOperator, Block, EnumDef, Expr, ExprId, FunctionBody, FunctionDef, Item,
    LiteralPattern, ModuleContent, NumericSuffix, OutcomeArm, ParameterPassing, Pattern, PatternId,
    Program, Spanned, Stmt, StmtId, StructDef, TrileanValue, TypeExpr, TypeId, UnaryOperator,
    Visibility,
};
use triet_typecheck::check_resolved;

// ─────────────────────────────────────────────────────────────────
// Triết-side: compile `compiler/parser.tri` once + run
// `dump_program_ndjson(source)`. Mirrors lexer_differential's
// `lexer_ir()` cache.
// ─────────────────────────────────────────────────────────────────

fn compiler_parser_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("compiler")
        .join("parser.tri")
}

fn parser_ir() -> &'static IrProgram {
    static IR: OnceLock<IrProgram> = OnceLock::new();
    IR.get_or_init(|| {
        let path = compiler_parser_path();
        assert!(
            path.is_file(),
            "missing compiler/parser.tri at {}",
            path.display()
        );
        let resolved = load_program(&path).expect("load_program");
        let diagnostics = check_resolved(&resolved);
        let blocking: Vec<_> = diagnostics
            .iter()
            .filter(|err| err.severity() != Some(miette::Severity::Warning))
            .collect();
        assert!(
            blocking.is_empty(),
            "type errors in compiler/parser.tri: {blocking:#?}",
        );
        let ir = lower_program(&resolved);
        let bytes = write_program(&ir);
        read_program(&bytes).expect("read .triv round-trip")
    })
}

fn lookup_func(ir: &IrProgram, name: &str) -> FuncId {
    ir.modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing function `{name}` in compiler/parser.tri"))
        .id
}

fn triet_dump(source: &str) -> String {
    let ir = parser_ir().clone();
    let func_id = lookup_func(&ir, "dump_program_ndjson");
    let mut vm = Vm::new(ir);
    let result = vm
        .execute(func_id, vec![RuntimeValue::String(source.to_owned())])
        .expect("compiler/parser.tri::dump_program_ndjson must execute without VM error");
    match result {
        RuntimeValue::String(s) => s,
        other => panic!("expected String from dump_program_ndjson, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────
// Rust-side mirror — walks triet_syntax AST in the same pre-order
// the Triết-side dump uses, emitting byte-identical NDJSON.
// ─────────────────────────────────────────────────────────────────

fn byte_to_char_index(source: &str) -> Vec<usize> {
    let mut idx = vec![0_usize; source.len() + 1];
    let mut char_count = 0_usize;
    let mut last = 0_usize;
    for (byte_pos, slot) in idx.iter_mut().enumerate().take(source.len()) {
        if source.is_char_boundary(byte_pos) {
            last = char_count;
            char_count += 1;
        }
        *slot = last;
    }
    idx[source.len()] = char_count;
    idx
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

fn quote_string(s: &str) -> String {
    format!("\"{}\"", json_escape(s))
}

const fn suffix_name(s: NumericSuffix) -> &'static str {
    match s {
        NumericSuffix::Trit => "trit",
        NumericSuffix::Tryte => "tryte",
        NumericSuffix::Integer => "integer",
        NumericSuffix::Long => "long",
    }
}

const fn visibility_name(v: Visibility) -> &'static str {
    match v {
        Visibility::Private => "private",
        Visibility::Public => "public",
        Visibility::PublicPackage => "package",
    }
}

const fn passing_name(p: ParameterPassing) -> &'static str {
    match p {
        ParameterPassing::Borrowed => "borrowed",
        ParameterPassing::Mutable => "mutable",
        ParameterPassing::Owned => "owned",
    }
}

const fn arm_name(a: OutcomeArm) -> &'static str {
    match a {
        OutcomeArm::Positive => "positive",
        OutcomeArm::Zero => "zero",
        OutcomeArm::Negative => "negative",
    }
}

const fn trilean_lit_name(v: TrileanValue) -> &'static str {
    match v {
        TrileanValue::True => "true",
        TrileanValue::False => "false",
        TrileanValue::Unknown => "unknown",
    }
}

const fn binary_op_name(op: BinaryOperator) -> &'static str {
    match op {
        BinaryOperator::Add => "Add",
        BinaryOperator::Subtract => "Subtract",
        BinaryOperator::Multiply => "Multiply",
        BinaryOperator::Divide => "Divide",
        BinaryOperator::Modulo => "Modulo",
        BinaryOperator::Power => "Power",
        BinaryOperator::Equal => "Equal",
        BinaryOperator::NotEqual => "NotEqual",
        BinaryOperator::LessThan => "LessThan",
        BinaryOperator::LessEqual => "LessEqual",
        BinaryOperator::GreaterThan => "GreaterThan",
        BinaryOperator::GreaterEqual => "GreaterEqual",
        BinaryOperator::And => "And",
        BinaryOperator::Or => "Or",
        BinaryOperator::Xor => "Xor",
        BinaryOperator::Iff => "Iff",
        BinaryOperator::Implies => "Implies",
        BinaryOperator::KleeneXor => "KleeneXor",
        BinaryOperator::KleeneIff => "KleeneIff",
        BinaryOperator::KleeneImplies => "KleeneImplies",
    }
}

const fn unary_op_name(op: UnaryOperator) -> &'static str {
    match op {
        UnaryOperator::Negate => "Negate",
    }
}

const fn bool_repr(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

struct Ctx<'a> {
    arena: &'a Arena,
    byte_to_char: &'a [usize],
}

impl Ctx<'_> {
    fn span(&self, s: &std::ops::Range<usize>) -> (usize, usize) {
        (self.byte_to_char[s.start], self.byte_to_char[s.end])
    }
}

fn header(kind: &str, start: usize, end: usize) -> String {
    format!("{{\"k\":{},\"s\":[{start},{end}]", quote_string(kind))
}

// ── Type dump ──────────────────────────────────────────────────

fn dump_type(out: &mut String, ctx: &Ctx<'_>, id: TypeId) {
    let sp = ctx.arena.type_expression(id);
    let (start, end) = ctx.span(&sp.span);
    match &sp.node {
        TypeExpr::Named(name) => {
            let _ = writeln!(
                out,
                "{},\"name\":{}}}",
                header("NamedType", start, end),
                quote_string(name)
            );
        }
        TypeExpr::Generic { name, arguments } => {
            let _ = writeln!(
                out,
                "{},\"name\":{},\"args\":{}}}",
                header("GenericType", start, end),
                quote_string(name),
                arguments.len()
            );
            for arg in arguments {
                dump_type(out, ctx, *arg);
            }
        }
        TypeExpr::Tuple(elems) => {
            let _ = writeln!(
                out,
                "{},\"elems\":{}}}",
                header("TupleType", start, end),
                elems.len()
            );
            for elem in elems {
                dump_type(out, ctx, *elem);
            }
        }
        TypeExpr::Nullable(inner) => {
            let _ = writeln!(out, "{}}}", header("NullableType", start, end));
            dump_type(out, ctx, *inner);
        }
        TypeExpr::Function {
            parameters,
            return_type,
        } => {
            let _ = writeln!(
                out,
                "{},\"params\":{}}}",
                header("FunctionType", start, end),
                parameters.len()
            );
            for p in parameters {
                dump_type(out, ctx, *p);
            }
            dump_type(out, ctx, *return_type);
        }
        TypeExpr::Outcome {
            value_type,
            error_type,
            allow_null_state,
        } => {
            let _ = writeln!(
                out,
                "{},\"null\":{}}}",
                header("OutcomeType", start, end),
                bool_repr(*allow_null_state)
            );
            dump_type(out, ctx, *value_type);
            dump_type(out, ctx, *error_type);
        }
        TypeExpr::RefinedTrilean => {
            let _ = writeln!(out, "{}}}", header("RefinedTrileanType", start, end));
        }
    }
}

// ── Pattern dump ────────────────────────────────────────────────

fn literal_pattern_fields(lit: &LiteralPattern) -> String {
    match lit {
        LiteralPattern::Integer { value, suffix } => {
            let mut s = format!(",\"lit\":\"int\",\"v\":{value}");
            if let Some(suf) = suffix {
                let _ = write!(s, ",\"u\":{}", quote_string(suffix_name(*suf)));
            }
            s
        }
        LiteralPattern::Ternary(v) => format!(",\"lit\":\"ternary\",\"v\":{v}"),
        LiteralPattern::String(t) => {
            format!(",\"lit\":\"string\",\"v\":{}", quote_string(t))
        }
        LiteralPattern::Trilean(v) => {
            format!(",\"lit\":\"trilean\",\"v\":{}", quote_string(trilean_lit_name(*v)))
        }
    }
}

fn dump_pattern(out: &mut String, ctx: &Ctx<'_>, id: PatternId) {
    let sp = ctx.arena.pattern(id);
    let (start, end) = ctx.span(&sp.span);
    match &sp.node {
        Pattern::Wildcard => {
            let _ = writeln!(out, "{}}}", header("WildcardPattern", start, end));
        }
        Pattern::Null => {
            let _ = writeln!(out, "{}}}", header("NullPat", start, end));
        }
        Pattern::Variable(name) => {
            let _ = writeln!(
                out,
                "{},\"name\":{}}}",
                header("IdentifierPattern", start, end),
                quote_string(name)
            );
        }
        Pattern::Literal(lit) => {
            let _ = writeln!(
                out,
                "{}{}}}",
                header("LiteralPat", start, end),
                literal_pattern_fields(lit)
            );
        }
        Pattern::Tuple(elems) => {
            let _ = writeln!(
                out,
                "{},\"elems\":{}}}",
                header("TuplePat", start, end),
                elems.len()
            );
            for elem in elems {
                dump_pattern(out, ctx, *elem);
            }
        }
        Pattern::Or(alts) => {
            let _ = writeln!(
                out,
                "{},\"alts\":{}}}",
                header("OrPat", start, end),
                alts.len()
            );
            for alt in alts {
                dump_pattern(out, ctx, *alt);
            }
        }
        Pattern::Range {
            start: s,
            end: e,
            inclusive,
        } => {
            let _ = writeln!(
                out,
                "{},\"start\":{{{}}},\"end\":{{{}}},\"incl\":{}}}",
                header("RangePat", start, end),
                literal_pattern_fields(s),
                literal_pattern_fields(e),
                bool_repr(*inclusive),
            );
        }
        Pattern::EnumVariant {
            variant_name,
            payload,
            ..
        } => {
            let _ = writeln!(
                out,
                "{},\"variant\":{},\"payload\":{}}}",
                header("EnumVariantPat", start, end),
                quote_string(variant_name),
                bool_repr(payload.is_some()),
            );
            if let Some(p) = payload {
                dump_pattern(out, ctx, *p);
            }
        }
        Pattern::OutcomeArm { arm, payload } => {
            let _ = writeln!(
                out,
                "{},\"arm\":{},\"payload\":{}}}",
                header("OutcomeArmPat", start, end),
                quote_string(arm_name(*arm)),
                bool_repr(payload.is_some()),
            );
            if let Some(p) = payload {
                dump_pattern(out, ctx, *p);
            }
        }
    }
}

// ── Expression dump ─────────────────────────────────────────────

#[allow(clippy::too_many_lines)] // 17 Expr variants — splitting hurts the read flow
fn dump_expr(out: &mut String, ctx: &Ctx<'_>, id: ExprId) {
    let sp = ctx.arena.expression(id);
    let (start, end) = ctx.span(&sp.span);
    match &sp.node {
        Expr::IntegerLiteral { value, suffix } => {
            let _ = write!(out, "{},\"v\":{value}", header("IntegerLiteralExpr", start, end));
            if let Some(suf) = suffix {
                let _ = write!(out, ",\"u\":{}", quote_string(suffix_name(*suf)));
            }
            let _ = writeln!(out, "}}");
        }
        Expr::TernaryLiteral { value } => {
            let _ = writeln!(
                out,
                "{},\"v\":{value}}}",
                header("TernaryLiteralExpr", start, end)
            );
        }
        Expr::TrileanLiteral(v) => {
            let _ = writeln!(
                out,
                "{},\"v\":{}}}",
                header("TrileanLiteralExpr", start, end),
                quote_string(trilean_lit_name(*v))
            );
        }
        Expr::StringLiteral(t) => {
            let _ = writeln!(
                out,
                "{},\"v\":{}}}",
                header("StringLiteralExpr", start, end),
                quote_string(t)
            );
        }
        Expr::NullLiteral => {
            let _ = writeln!(out, "{}}}", header("NullLiteralExpr", start, end));
        }
        Expr::Identifier(name) => {
            let _ = writeln!(
                out,
                "{},\"v\":{}}}",
                header("IdentifierExpr", start, end),
                quote_string(name)
            );
        }
        Expr::UnaryOp { operator, operand } => {
            let _ = writeln!(
                out,
                "{},\"op\":{}}}",
                header("UnaryOpExpr", start, end),
                quote_string(unary_op_name(*operator))
            );
            dump_expr(out, ctx, *operand);
        }
        Expr::BinaryOp {
            operator,
            left,
            right,
        } => {
            let _ = writeln!(
                out,
                "{},\"op\":{}}}",
                header("BinaryOpExpr", start, end),
                quote_string(binary_op_name(*operator))
            );
            dump_expr(out, ctx, *left);
            dump_expr(out, ctx, *right);
        }
        Expr::ForceUnwrap(inner) => {
            let _ = writeln!(out, "{}}}", header("ForceUnwrapExpr", start, end));
            dump_expr(out, ctx, *inner);
        }
        Expr::Call { callee, arguments } => {
            let _ = writeln!(
                out,
                "{},\"args\":{}}}",
                header("CallExpr", start, end),
                arguments.len()
            );
            dump_expr(out, ctx, *callee);
            for arg in arguments {
                dump_expr(out, ctx, *arg);
            }
        }
        Expr::FieldAccess { object, field } => {
            let _ = writeln!(
                out,
                "{},\"field\":{}}}",
                header("FieldAccessExpr", start, end),
                quote_string(field)
            );
            dump_expr(out, ctx, *object);
        }
        Expr::MethodCall {
            receiver,
            method,
            arguments,
        } => {
            let _ = writeln!(
                out,
                "{},\"method\":{},\"args\":{}}}",
                header("MethodCallExpr", start, end),
                quote_string(method),
                arguments.len()
            );
            dump_expr(out, ctx, *receiver);
            for arg in arguments {
                dump_expr(out, ctx, *arg);
            }
        }
        Expr::OutcomeConstructor { arm, payload } => {
            let _ = writeln!(
                out,
                "{},\"arm\":{},\"payload\":{}}}",
                header("OutcomeConstructorExpr", start, end),
                quote_string(arm_name(*arm)),
                bool_repr(payload.is_some()),
            );
            if let Some(p) = payload {
                dump_expr(out, ctx, *p);
            }
        }
        Expr::OutcomePropagate {
            inner,
            capture_name,
            early_return,
        } => {
            let _ = write!(out, "{}", header("OutcomePropagateExpr", start, end));
            if let Some(name) = capture_name {
                let _ = write!(out, ",\"capture\":{}", quote_string(name));
            }
            let _ = writeln!(out, "}}");
            dump_expr(out, ctx, *inner);
            dump_expr(out, ctx, *early_return);
        }
        Expr::OutcomeDefault { inner, default } => {
            let _ = writeln!(out, "{}}}", header("OutcomeDefaultExpr", start, end));
            dump_expr(out, ctx, *inner);
            dump_expr(out, ctx, *default);
        }
        Expr::ElvisOp { object, default } => {
            let _ = writeln!(out, "{}}}", header("ElvisOpExpr", start, end));
            dump_expr(out, ctx, *object);
            dump_expr(out, ctx, *default);
        }
        Expr::Range {
            start: s,
            end: e,
            inclusive,
        } => {
            let _ = writeln!(
                out,
                "{},\"incl\":{}}}",
                header("RangeExpr", start, end),
                bool_repr(*inclusive)
            );
            dump_expr(out, ctx, *s);
            dump_expr(out, ctx, *e);
        }
        // Variants the Triết-side parser doesn't yet produce. Any
        // corpus source whose Rust-parse outputs these would diverge
        // by definition — the corpus is curated to avoid them.
        other => panic!(
            "parser_differential corpus produced unsupported Rust Expr variant: {other:?}"
        ),
    }
}

// ── Statement / Block dump ─────────────────────────────────────

fn dump_block(out: &mut String, ctx: &Ctx<'_>, block: &Block) {
    let _ = writeln!(
        out,
        "{{\"k\":\"Block\",\"stmts\":{},\"final\":{}}}",
        block.statements.len(),
        bool_repr(block.final_expression.is_some()),
    );
    for sid in &block.statements {
        dump_stmt(out, ctx, *sid);
    }
    if let Some(eid) = block.final_expression {
        dump_expr(out, ctx, eid);
    }
}

#[allow(clippy::too_many_lines)] // 10 Stmt variants — splitting hurts the read flow
fn dump_stmt(out: &mut String, ctx: &Ctx<'_>, id: StmtId) {
    let sp = ctx.arena.statement(id);
    let (start, end) = ctx.span(&sp.span);
    match &sp.node {
        Stmt::Continue => {
            let _ = writeln!(out, "{}}}", header("ContinueStmt", start, end));
        }
        Stmt::Let {
            name,
            mutable,
            type_annotation,
            value,
        } => {
            let _ = writeln!(
                out,
                "{},\"name\":{},\"mut\":{},\"ann\":{}}}",
                header("LetStmt", start, end),
                quote_string(name),
                bool_repr(*mutable),
                bool_repr(type_annotation.is_some()),
            );
            if let Some(ty) = type_annotation {
                dump_type(out, ctx, *ty);
            }
            dump_expr(out, ctx, *value);
        }
        Stmt::Const {
            name,
            type_annotation,
            value,
        } => {
            let _ = writeln!(
                out,
                "{},\"name\":{},\"ann\":{}}}",
                header("ConstantStmt", start, end),
                quote_string(name),
                bool_repr(type_annotation.is_some()),
            );
            if let Some(ty) = type_annotation {
                dump_type(out, ctx, *ty);
            }
            dump_expr(out, ctx, *value);
        }
        Stmt::Return(value) => {
            let _ = writeln!(
                out,
                "{},\"value\":{}}}",
                header("ReturnStmt", start, end),
                bool_repr(value.is_some()),
            );
            if let Some(eid) = value {
                dump_expr(out, ctx, *eid);
            }
        }
        Stmt::Break(value) => {
            let _ = writeln!(
                out,
                "{},\"value\":{}}}",
                header("BreakStmt", start, end),
                bool_repr(value.is_some()),
            );
            if let Some(eid) = value {
                dump_expr(out, ctx, *eid);
            }
        }
        Stmt::For {
            variable,
            iterable,
            body,
        } => {
            let _ = writeln!(out, "{}}}", header("ForStmt", start, end));
            dump_pattern(out, ctx, *variable);
            dump_expr(out, ctx, *iterable);
            dump_block(out, ctx, body);
        }
        Stmt::While {
            condition,
            body,
            treat_unknown_as_false,
        } => {
            let _ = writeln!(
                out,
                "{},\"q\":{}}}",
                header("WhileStmt", start, end),
                bool_repr(*treat_unknown_as_false),
            );
            dump_expr(out, ctx, *condition);
            dump_block(out, ctx, body);
        }
        Stmt::Loop(body) => {
            let _ = writeln!(out, "{}}}", header("LoopStmt", start, end));
            dump_block(out, ctx, body);
        }
        Stmt::Assign { target, value } => {
            let _ = writeln!(
                out,
                "{},\"target\":{}}}",
                header("AssignStmt", start, end),
                quote_string(target),
            );
            dump_expr(out, ctx, *value);
        }
        Stmt::ExprStmt(eid) => {
            let _ = writeln!(out, "{}}}", header("ExpressionStmt", start, end));
            dump_expr(out, ctx, *eid);
        }
    }
}

// ── Item dump ───────────────────────────────────────────────────

fn dump_string_list(out: &mut String, kind: &str, items: &[String]) {
    for item in items {
        let _ = writeln!(
            out,
            "{{\"k\":{},\"name\":{}}}",
            quote_string(kind),
            quote_string(item),
        );
    }
}

fn dump_function_body(out: &mut String, ctx: &Ctx<'_>, body: &FunctionBody) {
    match body {
        FunctionBody::Block(block) => {
            out.push_str("{\"k\":\"FunctionBodyBlock\"}\n");
            dump_block(out, ctx, block);
        }
        FunctionBody::Expression(eid) => {
            out.push_str("{\"k\":\"FunctionBodyExpression\"}\n");
            dump_expr(out, ctx, *eid);
        }
    }
}

fn dump_function(out: &mut String, ctx: &Ctx<'_>, span: (usize, usize), f: &FunctionDef) {
    let _ = writeln!(
        out,
        "{},\"name\":{},\"vis\":{},\"gen\":{},\"params\":{},\"ret\":{}}}",
        header("FunctionItem", span.0, span.1),
        quote_string(&f.name),
        quote_string(visibility_name(f.visibility)),
        f.type_params.len(),
        f.parameters.len(),
        bool_repr(f.return_type.is_some()),
    );
    dump_string_list(out, "GenericParam", &f.type_params);
    for p in &f.parameters {
        let _ = writeln!(
            out,
            "{{\"k\":\"FunctionParam\",\"name\":{},\"pass\":{}}}",
            quote_string(&p.name),
            quote_string(passing_name(p.passing)),
        );
        dump_type(out, ctx, p.type_annotation);
    }
    if let Some(rt) = f.return_type {
        dump_type(out, ctx, rt);
    }
    dump_function_body(out, ctx, &f.body);
}

fn dump_struct(out: &mut String, ctx: &Ctx<'_>, span: (usize, usize), sd: &StructDef) {
    let _ = writeln!(
        out,
        "{},\"name\":{},\"vis\":{},\"gen\":{},\"fields\":{}}}",
        header("StructItem", span.0, span.1),
        quote_string(&sd.name),
        quote_string(visibility_name(sd.visibility)),
        sd.type_params.len(),
        sd.fields.len(),
    );
    dump_string_list(out, "GenericParam", &sd.type_params);
    for field in &sd.fields {
        let _ = writeln!(
            out,
            "{{\"k\":\"StructField\",\"name\":{}}}",
            quote_string(&field.name),
        );
        dump_type(out, ctx, field.type_annotation);
    }
}

fn dump_enum(out: &mut String, ctx: &Ctx<'_>, span: (usize, usize), ed: &EnumDef) {
    let _ = writeln!(
        out,
        "{},\"name\":{},\"vis\":{},\"gen\":{},\"variants\":{}}}",
        header("EnumItem", span.0, span.1),
        quote_string(&ed.name),
        quote_string(visibility_name(ed.visibility)),
        ed.type_params.len(),
        ed.variants.len(),
    );
    dump_string_list(out, "GenericParam", &ed.type_params);
    for variant in &ed.variants {
        let _ = writeln!(
            out,
            "{{\"k\":\"EnumVariant\",\"name\":{},\"payload\":{}}}",
            quote_string(&variant.name),
            bool_repr(variant.payload.is_some()),
        );
        if let Some(p) = variant.payload {
            dump_type(out, ctx, p);
        }
    }
}

#[allow(clippy::too_many_lines)] // 8 Item variants × multi-field — splitting hurts the read flow
fn dump_item(out: &mut String, ctx: &Ctx<'_>, item: &Spanned<Item>) {
    let (start, end) = ctx.span(&item.span);
    let span = (start, end);
    match &item.node {
        Item::Function(f) => dump_function(out, ctx, span, f),
        Item::Const {
            visibility,
            name,
            type_annotation,
            value,
        } => {
            let _ = writeln!(
                out,
                "{},\"name\":{},\"vis\":{},\"ann\":{}}}",
                header("ConstantItem", start, end),
                quote_string(name),
                quote_string(visibility_name(*visibility)),
                bool_repr(type_annotation.is_some()),
            );
            if let Some(ty) = type_annotation {
                dump_type(out, ctx, *ty);
            }
            dump_expr(out, ctx, *value);
        }
        Item::TypeAlias {
            visibility,
            name,
            target,
        } => {
            let _ = writeln!(
                out,
                "{},\"name\":{},\"vis\":{}}}",
                header("TypeAliasItem", start, end),
                quote_string(name),
                quote_string(visibility_name(*visibility)),
            );
            dump_type(out, ctx, *target);
        }
        Item::Struct(sd) => dump_struct(out, ctx, span, sd),
        Item::Enum(ed) => dump_enum(out, ctx, span, ed),
        Item::Import(path) => {
            let _ = writeln!(
                out,
                "{},\"segments\":{}}}",
                header("ImportItem", start, end),
                path.segments.len(),
            );
            dump_string_list(out, "PathSegment", &path.segments);
        }
        Item::ImportFrom(im) => {
            let _ = writeln!(
                out,
                "{},\"src\":{},\"names\":{}}}",
                header("ImportFromItem", start, end),
                im.source.len(),
                im.names.len(),
            );
            dump_string_list(out, "PathSegment", &im.source);
            for nm in &im.names {
                let _ = writeln!(
                    out,
                    "{{\"k\":\"ImportName\",\"name\":{},\"alias\":{}}}",
                    quote_string(&nm.name),
                    bool_repr(nm.alias.is_some()),
                );
                if let Some(alias) = &nm.alias {
                    let _ = writeln!(
                        out,
                        "{{\"k\":\"ImportAlias\",\"name\":{}}}",
                        quote_string(alias),
                    );
                }
            }
        }
        Item::Module(m) => {
            let content_label = match &m.content {
                ModuleContent::External => "external",
                ModuleContent::Inline(_) => "inline",
            };
            let item_count = match &m.content {
                ModuleContent::External => 0,
                ModuleContent::Inline(items) => items.len(),
            };
            let _ = writeln!(
                out,
                "{},\"name\":{},\"vis\":{},\"content\":{},\"items\":{}}}",
                header("ModuleItem", start, end),
                quote_string(&m.name),
                quote_string(visibility_name(m.visibility)),
                quote_string(content_label),
                item_count,
            );
            if let ModuleContent::Inline(items) = &m.content {
                let inner_ctx = Ctx {
                    arena: ctx.arena,
                    byte_to_char: ctx.byte_to_char,
                };
                for inner in items {
                    dump_item(out, &inner_ctx, inner);
                }
            }
        }
    }
}

fn dump_parse_error(out: &mut String, ctx: &Ctx<'_>, err: &ParseError) {
    use triet_parser::ParseError as PE;
    match err {
        PE::UnexpectedToken {
            expected,
            found,
            span,
        } => {
            let (s, e) = ctx.span(span);
            let _ = writeln!(
                out,
                "{{\"e\":\"UnexpectedToken\",\"s\":[{s},{e}],\"v\":{},\"f\":{}}}",
                quote_string(expected),
                quote_string(found),
            );
        }
        PE::UnexpectedEof { expected, span } => {
            let (s, e) = ctx.span(span);
            let _ = writeln!(
                out,
                "{{\"e\":\"UnexpectedEof\",\"s\":[{s},{e}],\"v\":{}}}",
                quote_string(expected),
            );
        }
        PE::ReservedItemName { name, span } => {
            let (s, e) = ctx.span(span);
            let _ = writeln!(
                out,
                "{{\"e\":\"ReservedItemName\",\"s\":[{s},{e}],\"v\":{}}}",
                quote_string(name),
            );
        }
        other => {
            // Variants the Triết-side parser doesn't yet surface
            // (e.g. ChainedNoChainOperator, InvalidAssignmentTarget,
            // Lex). The corpus is curated so this branch never
            // triggers on a passing test, but if a corpus addition
            // ever does, the panic message points at the gap.
            panic!("parser_differential: unsupported Rust ParseError variant in dump: {other:?}");
        }
    }
}

fn dump_program(out: &mut String, program: &Program, byte_to_char: &[usize]) {
    let ctx = Ctx {
        arena: &program.arena,
        byte_to_char,
    };
    let _ = writeln!(out, "{{\"k\":\"Program\",\"items\":{}}}", program.items.len());
    for item in &program.items {
        dump_item(out, &ctx, item);
    }
}

fn rust_dump(source: &str) -> String {
    let byte_to_char = byte_to_char_index(source);
    let (program, errors) = rust_parse(source);
    let mut out = String::new();
    dump_program(&mut out, &program, &byte_to_char);
    let ctx = Ctx {
        arena: &program.arena,
        byte_to_char: &byte_to_char,
    };
    for err in &errors {
        dump_parse_error(&mut out, &ctx, err);
    }
    out
}

// ─────────────────────────────────────────────────────────────────
// Differential driver
// ─────────────────────────────────────────────────────────────────

fn assert_equal(label: &str, source: &str) {
    let rust = rust_dump(source);
    let triet = triet_dump(source);
    if rust == triet {
        return;
    }
    let first_diff = rust
        .bytes()
        .zip(triet.bytes())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| rust.len().min(triet.len()));
    panic!(
        "parser differential mismatch for `{label}`\n\
         Rust length:  {}\n\
         Triết length: {}\n\
         first diverging byte: {first_diff}\n\
         ── Rust output (truncated) ──\n{}\n\
         ── Triết output (truncated) ──\n{}",
        rust.len(),
        triet.len(),
        rust.chars().take(800).collect::<String>(),
        triet.chars().take(800).collect::<String>(),
    );
}

// ─────────────────────────────────────────────────────────────────
// Corpus — kept inside the v0.7.5.6 surface envelope (see header).
// ─────────────────────────────────────────────────────────────────

#[test]
fn empty_program() {
    assert_equal("empty", "");
}

#[test]
fn single_function_with_expression_body() {
    assert_equal("fn_expr", "function main() = 0");
}

#[test]
fn function_with_param_and_return_type() {
    assert_equal(
        "fn_double",
        "function double(n: Integer) -> Integer = n * 2",
    );
}

#[test]
fn function_with_block_body() {
    assert_equal("fn_block", "function main() { 0 }");
}

#[test]
fn function_with_public_visibility() {
    assert_equal("fn_public", "public function main() = 0");
}

#[test]
fn function_with_package_visibility() {
    assert_equal("fn_package", "public(package) function helper() = 1");
}

#[test]
fn function_with_generic_params() {
    assert_equal("fn_generic", "function identity<T>(x: T) -> T = x");
}

#[test]
fn function_with_passing_modes() {
    assert_equal(
        "fn_passing",
        "function take(owned n: Integer) = n function mutate(mutable m: Integer) = m",
    );
}

#[test]
fn constant_with_and_without_annotation() {
    assert_equal(
        "consts",
        "constant PI = 3 public constant E: Integer = 2",
    );
}

#[test]
fn type_alias() {
    assert_equal("type_alias", "type Username = String public type Age = Integer");
}

#[test]
fn struct_with_fields_and_generics() {
    assert_equal(
        "struct_gen",
        "struct Empty { } public struct Box<T> { value: T } struct Point { x: Integer, y: Integer }",
    );
}

#[test]
fn enum_with_payloads_and_generics() {
    assert_equal(
        "enum_gen",
        "enum Color { Red, Green, Blue } public enum Option<T> { Some(T), None }",
    );
}

#[test]
fn imports_direct_and_from() {
    assert_equal(
        "imports",
        "import std.io\nimport crate.lexer\nfrom std.io import println, print\nfrom std.io import println as p",
    );
}

#[test]
fn module_external_and_inline() {
    assert_equal(
        "modules",
        "module foo\nmodule bar { function helper() = 0 } public(package) module data { struct Cell { v: Integer } }",
    );
}

#[test]
fn block_body_with_let_and_final_expression() {
    assert_equal(
        "block_let",
        "function main() { let x = 5 let mutable y = 0 y = y + x y }",
    );
}

#[test]
fn block_body_with_return_break_continue() {
    assert_equal(
        "block_flow",
        "function main() { return 0 } function loop_demo() { loop { break 1 } } function infinite() { loop { continue } }",
    );
}

#[test]
fn for_and_while_loops() {
    assert_equal(
        "loops",
        "function main() { for i in 0..10 { } for _ in xs { } while flag { } while? maybe_known { } }",
    );
}

#[test]
fn pratt_arithmetic_and_precedence() {
    assert_equal(
        "pratt_arith",
        "function f() = 1 + 2 * 3 - 4 / 5 %% 2 ** 3",
    );
}

#[test]
fn pratt_comparison_and_logic() {
    assert_equal(
        "pratt_logic",
        "function f() = a == b && c != d || e < f && g >= h",
    );
}

#[test]
fn unary_operators() {
    assert_equal(
        "unary",
        "function f() = -x function g() = !p function h() = not q",
    );
}

#[test]
fn postfix_operators() {
    assert_equal(
        "postfix",
        "function f() = obj.field function g() = obj.method(1, 2) function h() = x!! function k() = call(a, b, c)",
    );
}

#[test]
fn outcome_constructors_and_propagation() {
    assert_equal(
        "outcome",
        "function ok() -> Integer~Error = ~+ 5\nfunction zero() -> Integer?~Error = ~0\nfunction err() -> Integer~Error = ~- 1\nfunction try() -> Integer~Error = compute() ~? |e| ~- e\nfunction default() = compute() ~: 0",
    );
}

#[test]
fn elvis_and_range() {
    assert_equal(
        "elvis_range",
        "function f() = x ?: 0 function g() = 0..100 function h() = 0..=10",
    );
}

#[test]
fn outcome_types_in_signatures() {
    assert_equal(
        "outcome_types",
        "function a() -> Integer~ParseError = ~+ 5\nfunction b() -> Vector<Token>?~LexError = ~0",
    );
}

#[test]
fn nullable_and_refined_trilean_types() {
    assert_equal(
        "type_nullable",
        "function f(x: Integer?) -> Trilean! = true\nfunction g(y: Integer??) = y",
    );
}

#[test]
fn function_types_and_tuples() {
    assert_equal(
        "fn_type_tuple",
        "constant CB: (Integer, Integer) -> Long = handler\nconstant PAIR: (Integer, String) = something\nconstant SINGLE: (Integer,) = stuff",
    );
}

#[test]
fn nested_generics() {
    assert_equal(
        "nested_gen",
        "function f(m: HashMap<String, Vector<Integer>>) = m",
    );
}

#[test]
fn realistic_function_with_let_outcome_propagation() {
    // Use the `function … { … }` block-body form (not `= { … }`
    // which the Rust parser folds into Expr::Block — a variant the
    // Triết-side surface doesn't produce yet).
    //
    // The body parses as a Stmt::Let (with `~?` propagation) plus a
    // final ExpressionStmt of an OutcomeConstructorExpr, which is
    // exactly the shape the bootstrap parser needs to produce for
    // its own recursive helpers. Struct-literal expressions don't
    // exist in the v0.7.5.* Expr surface, so the OutcomeConstructor
    // payload uses an identifier instead.
    assert_equal(
        "real_outcome",
        "function consume(c: Counter) -> Counter~Error {\n    let step: Counter = increment(c) ~? |e| ~- e\n    ~+ step\n}",
    );
}

// Error recovery is intentionally NOT in the differential corpus.
// Two implementation-level divergences make a byte-diff infeasible
// without flattening Rust's behavior to Triết's (or vice versa):
//
// 1. Synchronization granularity. Rust's mutable-cursor parser
//    consumes tokens during the failed parse_item attempt, then
//    synchronizes from that mid-item position. Triết's functional
//    parser passes immutable ParserState through every step, so a
//    `~-` propagation discards any intermediate advancement — the
//    driver re-synchronizes from the PRE-call cursor. This means
//    Triết-side typically records more errors per source-level
//    mistake than Rust does (defensive force-advance lands on
//    sync-stop tokens that fail another parse_item attempt). The
//    behavior is documented in compiler/parser.tri's v0.7.5.6a
//    header comment.
//
// 2. "found" token label format. Rust formats the offending token
//    via `format!("{:?}", token)` which yields a variant name like
//    "Assign". Triết-side uses `token_kind_label` which yields the
//    user-facing symbol like "=". The latter is friendlier in
//    diagnostics but doesn't byte-equal Rust's representation.
//
// Both divergences are concerns about error reporting, not AST
// shape. The Triết-side recovery layer is pinned end-to-end by
// `parser_recovery_smoke.rs`; the differential here focuses on
// byte-identity of the well-formed AST output.
