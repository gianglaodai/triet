//! Disassembly display — renders IR in a human-readable format for
//! debugging and the `triet disasm` command (future).
//!
//! Output format per [ADR-0007 § Hình thức cụ thể]:
//! ```text
//! function @factorial(%n: Integer) -> Integer {
//! entry:
//!     %0 = eq %n, const Integer 0_integer
//!     br_if %0, base_case, recursive_case
//!
//! base_case:
//!     ret const Integer 1_integer
//!
//! recursive_case:
//!     %1 = sub %n, const Integer 1_integer
//!     ...
//! }
//! ```

use std::fmt;

use crate::constant::Constant;
use crate::instr::{BuiltinName, Instruction, Operand};
use crate::module::{BasicBlock, Function, IrModule, IrProgram};

// ── Operand display ───────────────────────────────────────────────

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(v) => write!(f, "{v}"),
            Self::Const(c) => {
                // We don't have access to the pool here, so print the ID.
                // The full disassembly context will resolve it.
                write!(f, "const c{}", c.0)
            }
        }
    }
}

// ── Instruction display ───────────────────────────────────────────

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lhs = self
            .destination()
            .map_or_else(String::new, |d| format!("{d} = "));
        match self {
            Self::Const { constant, .. } => {
                write!(f, "{lhs}const c{}", constant.0)
            }
            Self::Add { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}add {l}, {r}")
            }
            Self::Sub { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}sub {l}, {r}")
            }
            Self::Mul { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}mul {l}, {r}")
            }
            Self::Div { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}div {l}, {r}")
            }
            Self::Mod { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}mod {l}, {r}")
            }
            Self::Pow { base, exp, .. } => {
                write!(f, "{lhs}pow {base}, {exp}")
            }
            Self::Neg { operand, .. } => {
                write!(f, "{lhs}neg {operand}")
            }
            Self::LukAnd { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}luk_and {l}, {r}")
            }
            Self::LukOr { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}luk_or {l}, {r}")
            }
            Self::LukImplies { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}luk_implies {l}, {r}")
            }
            Self::LukXor { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}luk_xor {l}, {r}")
            }
            Self::LukIff { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}luk_iff {l}, {r}")
            }
            Self::KleeneImplies { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}kleene_implies {l}, {r}")
            }
            Self::KleeneXor { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}kleene_xor {l}, {r}")
            }
            Self::KleeneIff { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}kleene_iff {l}, {r}")
            }
            Self::Eq { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}eq {l}, {r}")
            }
            Self::Ne { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}ne {l}, {r}")
            }
            Self::Lt { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}lt {l}, {r}")
            }
            Self::Le { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}le {l}, {r}")
            }
            Self::Gt { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}gt {l}, {r}")
            }
            Self::Ge { lhs: l, rhs: r, .. } => {
                write!(f, "{lhs}ge {l}, {r}")
            }
            Self::ToInteger { operand, .. } => {
                write!(f, "{lhs}to_integer {operand}")
            }
            Self::ToTryte { operand, .. } => {
                write!(f, "{lhs}to_tryte {operand}")
            }
            Self::ToLong { operand, .. } => {
                write!(f, "{lhs}to_long {operand}")
            }
            Self::ToTrit { operand, .. } => {
                write!(f, "{lhs}to_trit {operand}")
            }
            Self::ToTrilean { operand, .. } => {
                write!(f, "{lhs}to_trilean {operand}")
            }
            Self::StructNew { fields, .. } => {
                write!(f, "{lhs}struct_new [")?;
                for (i, fld) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{fld}")?;
                }
                write!(f, "]")
            }
            Self::FieldGet {
                object, field_idx, ..
            } => {
                write!(f, "{lhs}field_get {object}, {field_idx}")
            }
            Self::FieldSet {
                object,
                field_idx,
                value,
                ..
            } => {
                write!(f, "{lhs}field_set {object}, {field_idx}, {value}")
            }
            Self::EnumNew {
                variant_idx,
                payload,
                ..
            } => {
                write!(f, "{lhs}enum_new v{variant_idx}")?;
                if let Some(p) = payload {
                    write!(f, ", {p}")?;
                }
                Ok(())
            }
            Self::EnumTag { scrutinee, .. } => {
                write!(f, "{lhs}enum_tag {scrutinee}")
            }
            Self::EnumPayload { scrutinee, .. } => {
                write!(f, "{lhs}enum_payload {scrutinee}")
            }
            Self::NullWrap { value: v, .. } => {
                write!(f, "{lhs}null_wrap {v}")
            }
            Self::NullUnwrap { nullable, .. } => {
                write!(f, "{lhs}null_unwrap {nullable}")
            }
            Self::NullCheck { nullable, .. } => {
                write!(f, "{lhs}null_check {nullable}")
            }
            Self::CallLocal { callee, args, .. } => {
                write!(f, "{lhs}call @f{}", callee.0)?;
                write_args(f, args)
            }
            Self::CallCrossModule { path, args, .. } => {
                write!(f, "{lhs}call {path}")?;
                write_args(f, args)
            }
            Self::WitnessCall {
                path,
                witness_idx,
                args,
                ..
            } => {
                write!(f, "{lhs}witness_call {path}[w{witness_idx}]")?;
                write_args(f, args)
            }
            Self::CallBuiltin { name, args, .. } => {
                write!(f, "{lhs}call builtin.{name}")?;
                write_args(f, args)
            }
            Self::ClosureNew {
                lambda, captures, ..
            } => {
                write!(f, "{lhs}closure_new @f{}", lambda.0)?;
                if !captures.is_empty() {
                    write!(f, ", [")?;
                    for (i, c) in captures.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{c}")?;
                    }
                    write!(f, "]")?;
                }
                Ok(())
            }
            Self::ClosureCall { closure, args, .. } => {
                write!(f, "{lhs}closure_call {closure}")?;
                write_args(f, args)
            }
            Self::Br { target } => {
                write!(f, "br {target}")
            }
            Self::BrIf {
                cond,
                then_block,
                else_block,
            } => {
                write!(f, "br_if {cond}, {then_block}, {else_block}")
            }
            Self::BrTrilean {
                cond,
                true_block,
                unknown_block,
                false_block,
            } => {
                write!(
                    f,
                    "br_trilean {cond}, +{true_block}, ?{unknown_block}, -{false_block}"
                )
            }
            Self::Ret { value } => {
                write!(f, "ret")?;
                if let Some(v) = value {
                    write!(f, " {v}")?;
                }
                Ok(())
            }
            Self::Unreachable => {
                write!(f, "unreachable")
            }
            Self::Phi { incoming, .. } => {
                write!(f, "{lhs}φ [")?;
                for (i, edge) in incoming.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{} from b{}", edge.value, edge.block.0)?;
                }
                write!(f, "]")
            }
            Self::OutcomeNewPositive { payload, .. } => {
                write!(f, "{lhs}outcome_new_positive {payload}")
            }
            Self::OutcomeNewNegative { payload, .. } => {
                write!(f, "{lhs}outcome_new_negative {payload}")
            }
            Self::OutcomeNewNull { .. } => {
                write!(f, "{lhs}outcome_new_null")
            }
            Self::OutcomeDiscriminant { source, .. } => {
                write!(f, "{lhs}outcome_discriminant {source}")
            }
            Self::OutcomeUnwrapValue { source, .. } => {
                write!(f, "{lhs}outcome_unwrap_value {source}")
            }
            Self::OutcomeUnwrapError { source, .. } => {
                write!(f, "{lhs}outcome_unwrap_error {source}")
            }
        }
    }
}

fn write_args(f: &mut fmt::Formatter<'_>, args: &[Operand]) -> fmt::Result {
    write!(f, "(")?;
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{a}")?;
    }
    write!(f, ")")
}

impl fmt::Display for BuiltinName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Println => write!(f, "println"),
            Self::Print => write!(f, "print"),
            Self::Assert => write!(f, "assert"),
            Self::AssertEq => write!(f, "assert_eq"),
            Self::FStringConcat => write!(f, "fstring_concat"),
            Self::TextLen => write!(f, "text_len"),
            Self::TextConcat => write!(f, "text_concat"),
            Self::TextFromInteger => write!(f, "text_from_integer"),
            Self::VectorNew => write!(f, "vector_new"),
            Self::VectorPush => write!(f, "vector_push"),
            Self::VectorGet => write!(f, "vector_get"),
            Self::VectorLength => write!(f, "vector_length"),
            Self::HashMapNew => write!(f, "hashmap_new"),
            Self::HashMapInsert => write!(f, "hashmap_insert"),
            Self::HashMapGet => write!(f, "hashmap_get"),
            Self::HashMapKeys => write!(f, "hashmap_keys"),
            Self::HashMapContains => write!(f, "hashmap_contains"),
            Self::ReadFile => write!(f, "read_file"),
            Self::WriteFile => write!(f, "write_file"),
            Self::WriteFileBytes => write!(f, "write_file_bytes"),
            Self::FileExists => write!(f, "file_exists"),
            Self::PathJoin => write!(f, "path_join"),
            Self::PathParent => write!(f, "path_parent"),
            Self::PathBasename => write!(f, "path_basename"),
            Self::StringSubstring => write!(f, "string_substring"),
            Self::StringSplit => write!(f, "string_split"),
            Self::StringIndexOf => write!(f, "string_index_of"),
            Self::ParseInteger => write!(f, "parse_integer"),
            Self::TextIntoBytes => write!(f, "into_bytes"),
            Self::TextFromBytes => write!(f, "from_bytes"),
            Self::Blake3Hash => write!(f, "blake3_hash"),
            Self::GetEnv => write!(f, "get_env"),
            Self::ReadDirRecursive => write!(f, "read_dir_recursive"),
            // v0.9.x.atomic.2 — atomic primitive builtins per ADR-0028 §1.
            Self::AtomicNew => write!(f, "atomic_new"),
            Self::AtomicLoad => write!(f, "atomic_load"),
            Self::AtomicStore => write!(f, "atomic_store"),
            Self::AtomicSwap => write!(f, "atomic_swap"),
            Self::AtomicCompareExchange => write!(f, "atomic_compare_exchange"),
            Self::AtomicFetchAdd => write!(f, "atomic_fetch_add"),
            Self::AtomicFetchSub => write!(f, "atomic_fetch_sub"),
            Self::AtomicFetchBitwiseAnd => write!(f, "atomic_fetch_bitwise_and"),
            Self::AtomicFetchBitwiseOr => write!(f, "atomic_fetch_bitwise_or"),
            Self::AtomicFetchBitwiseXor => write!(f, "atomic_fetch_bitwise_xor"),
            // v0.10.x.thread.1 — raw OS thread primitives per ADR-0026 v2 §3.
            Self::RawThreadSpawn => write!(f, "raw_thread_spawn"),
            Self::RawThreadJoin => write!(f, "raw_thread_join"),
        }
    }
}

// ── Block display ─────────────────────────────────────────────────

impl fmt::Display for BasicBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = self
            .name
            .clone()
            .unwrap_or_else(|| format!("b{}", self.id.0));
        writeln!(f, "{label}:")?;
        for instr in &self.instructions {
            writeln!(f, "    {instr}")?;
        }
        Ok(())
    }
}

// ── Function display ──────────────────────────────────────────────

impl fmt::Display for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self
            .name
            .clone()
            .unwrap_or_else(|| format!("@f{}", self.id.0));
        write!(f, "function {name}(")?;
        for (i, (pname, pty)) in self.params.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{pname}: {pty}")?;
        }
        writeln!(f, ") -> {} {{", self.return_type)?;
        for block in &self.blocks {
            write!(f, "{block}")?;
        }
        writeln!(f, "}}")
    }
}

// ── Module display ────────────────────────────────────────────────

impl fmt::Display for IrModule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, ";; module {}", self.path)?;
        for func in &self.functions {
            write!(f, "{func}")?;
        }
        Ok(())
    }
}

// ── Program display ───────────────────────────────────────────────

impl fmt::Display for IrProgram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Print constant pool first.
        if !self.constants.is_empty() {
            writeln!(f, ";; ── Constant pool ({}) ──", self.constants.len())?;
            for (id, c) in self.constants.iter() {
                writeln!(f, ";; c{} = {}", id.0, display_constant(c))?;
            }
            writeln!(f)?;
        }

        // Print modules.
        for module in &self.modules {
            write!(f, "{module}")?;
        }
        Ok(())
    }
}

/// Display a constant value in human-readable form.
fn display_constant(c: &Constant) -> String {
    match c {
        Constant::Trit(t) => format!("{t}"),
        Constant::Tryte(t) => format!("{t:?}"),
        Constant::Integer(i) => format!("{i}"),
        Constant::Long(l) => format!("{l}"),
        Constant::Trilean(t) => format!("{t}"),
        Constant::String(s) => format!("\"{s}\""),
        Constant::Unit => "()".to_string(),
        Constant::Null => "null".to_string(),
    }
}
