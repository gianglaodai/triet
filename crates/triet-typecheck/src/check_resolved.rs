//! Type-check a multi-module [`ResolvedProgram`].
//!
//! This is the v0.2.x module-aware counterpart to [`crate::check::check`],
//! which operates on a flat single-file `Program`. The algorithm:
//!
//! 1. **Pass 1 — Collect signatures.** Walk every module and extract
//!    declared types (function signatures, struct fields, etc.) into
//!    a per-module type table.
//!
//! 2. **Pass 2 — Check per module.** For each module, build a
//!    `TypeEnvironment` pre-seeded with imported types (resolved via
//!    the binding map + pass 1 type tables), then run the standard
//!    single-module checker on that module's items.
//!
//! The returned error list is empty on success.

use triet_modules::ResolvedProgram;
use triet_syntax::Item;

use crate::{check::check_with_env, env::TypeEnvironment, error::TypeError, types::Type};

/// Type-check every module in a [`ResolvedProgram`].
///
/// This is the primary entry point for the v0.2.x pipeline. Single-file
/// programs still work — they produce a `ResolvedProgram` with one
/// module (the crate root) and zero imports.
///
/// Returns an empty `Vec` on success. Errors from all modules are
/// merged so the user sees the full failure surface in one run.
#[must_use]
pub fn check_resolved(program: &ResolvedProgram) -> Vec<TypeError> {
    let mut all_errors = Vec::new();

    // Pass 1: Collect declared types per module.
    let module_types: Vec<Vec<(String, Type)>> = program
        .modules
        .iter()
        .map(|module| {
            let arena = program.arena(module);
            collect_declared_types(arena, &module.items)
        })
        .collect();

    // Pass 2: For each module, build env with imports, then check.
    for (idx, module) in program.modules.iter().enumerate() {
        let arena = program.arena(module);

        // Build a single-module Program view.
        let single_program = triet_syntax::Program {
            arena: arena.clone(),
            items: module.items.clone(),
        };

        // Pre-seed the root module's environment with the standard
        // prelude (print, println, to_string, etc.) for backward-
        // compat with v0.1.x single-file programs. Child modules and
        // stdlib modules do not get the prelude — they must use
        // explicit `from std.io import …` declarations.
        let mut env = if idx == program.root.raw() {
            TypeEnvironment::with_prelude()
        } else {
            TypeEnvironment::default()
        };
        for (local_name, abs_path) in &module.bindings {
            let source_path = abs_path.module_path();
            let item_name = abs_path.name();

            // Skip own definitions — the checker's declare pass handles them.
            if *source_path == module.path {
                continue;
            }

            // Try user module first.
            if let Some(source_id) = program.find_module(source_path) {
                if source_id.raw() == idx {
                    continue; // self-import
                }
                let source_types = &module_types[source_id.raw()];
                if let Some((_, ty)) = source_types.iter().find(|(n, _)| n == item_name) {
                    env.declare(local_name, ty.clone());
                }
            }
        }

        let errors = check_with_env(&single_program, env);
        all_errors.extend(errors);
    }

    all_errors
}

/// Walk a module's items and extract declared types for each named item.
fn collect_declared_types(
    arena: &triet_syntax::Arena,
    items: &[triet_syntax::Spanned<Item>],
) -> Vec<(String, Type)> {
    let mut result = Vec::new();

    for item in items {
        match &item.node {
            Item::Function(def) => {
                let parameters: Vec<Type> = def
                    .parameters
                    .iter()
                    .map(|p| resolve_type_expr(arena, p.type_annotation))
                    .collect();
                let return_type = def
                    .return_type
                    .map_or(Type::Unit, |id| resolve_type_expr(arena, id));
                result.push((
                    def.name.clone(),
                    Type::Function {
                        parameters,
                        return_type: Box::new(return_type),
                    },
                ));
            }
            Item::Const {
                name,
                type_annotation,
                ..
            } => {
                let ty = type_annotation.map_or(Type::Unknown, |id| resolve_type_expr(arena, id));
                result.push((name.clone(), ty));
            }
            Item::Struct(def) => {
                let fields: Vec<(String, Type)> = def
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), resolve_type_expr(arena, f.type_annotation)))
                    .collect();
                result.push((
                    def.name.clone(),
                    Type::UserStruct {
                        name: def.name.clone(),
                        type_params: def.type_params.clone(),
                        fields,
                    },
                ));
            }
            Item::Enum(def) => {
                let variants: Vec<(String, Option<Box<Type>>)> = def
                    .variants
                    .iter()
                    .map(|v| {
                        let payload = v.payload.map(|tid| Box::new(resolve_type_expr(arena, tid)));
                        (v.name.clone(), payload)
                    })
                    .collect();
                result.push((
                    def.name.clone(),
                    Type::UserEnum {
                        name: def.name.clone(),
                        type_params: def.type_params.clone(),
                        variants,
                    },
                ));
            }
            Item::TypeAlias { .. } | Item::Import(_) | Item::ImportFrom(_) | Item::Module(_) => {}
        }
    }

    result
}

/// Resolve a type expression to a Type. Handles built-in names, tuples,
/// nullables, and function types. User-defined types resolve to Unknown
/// at this stage (they're handled during full checking).
fn resolve_type_expr(arena: &triet_syntax::Arena, id: triet_syntax::TypeId) -> Type {
    use triet_syntax::TypeExpr;
    match &arena.type_expression(id).node {
        TypeExpr::Named(name) => match name.as_str() {
            "Trit" => Type::Trit,
            "Tryte" => Type::Tryte,
            "Integer" => Type::Integer,
            "Long" => Type::Long,
            "Trilean" => Type::Trilean,
            "String" => Type::String,
            "Unit" => Type::Unit,
            _ => Type::Unknown,
        },
        TypeExpr::Tuple(elements) => Type::Tuple(
            elements
                .iter()
                .map(|t| resolve_type_expr(arena, *t))
                .collect(),
        ),
        TypeExpr::Nullable(inner) => Type::Nullable(Box::new(resolve_type_expr(arena, *inner))),
        TypeExpr::Function {
            parameters,
            return_type,
        } => Type::Function {
            parameters: parameters
                .iter()
                .map(|t| resolve_type_expr(arena, *t))
                .collect(),
            return_type: Box::new(resolve_type_expr(arena, *return_type)),
        },
        TypeExpr::Generic { .. } => Type::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ─────────────────────────────────────────────────────

    fn check_in_memory(source: &str) -> Vec<TypeError> {
        let program = triet_modules::load_program_from_source(source).expect("load should succeed");
        check_resolved(&program)
    }

    fn check_filesystem(files: &[(&str, &str)]) -> Vec<TypeError> {
        let temp = tempfile::tempdir().expect("tempdir");
        let base = temp.path();
        let mut root_path = None;

        for (rel_path, contents) in files {
            let full = base.join(rel_path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("create_dir_all");
            }
            std::fs::write(&full, contents).expect("write");
            if root_path.is_none() {
                root_path = Some(full);
            }
        }

        let program = triet_modules::load_program(root_path.as_ref().expect("at least one file"))
            .expect("load should succeed");
        check_resolved(&program)
    }

    // ── Single-module (backward compat) ─────────────────────────────

    #[test]
    fn single_module_happy_path() {
        let errors = check_in_memory("function main() -> Integer = 42");
        assert!(errors.is_empty(), "no errors expected: {errors:?}");
    }

    #[test]
    fn single_module_type_error() {
        let errors = check_in_memory(r#"function main() -> Integer = "oops""#);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TypeError::Mismatch { .. })),
            "expected Mismatch: {errors:?}"
        );
    }

    // ── Cross-module ────────────────────────────────────────────────

    #[test]
    fn cross_module_function_call() {
        let errors = check_filesystem(&[
            (
                "main.tri",
                "module helper\nfrom crate.helper import greet\nfunction main() -> Integer = greet()",
            ),
            ("helper.tri", "public function greet() -> Integer = 42"),
        ]);
        assert!(errors.is_empty(), "no errors expected: {errors:?}");
    }

    #[test]
    fn cross_module_type_mismatch() {
        let errors = check_filesystem(&[
            (
                "main.tri",
                "module helper
from crate.helper import greet
function main() -> String = greet()",
            ),
            ("helper.tri", "public function greet() -> Integer = 42"),
        ]);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TypeError::Mismatch { .. })),
            "expected Mismatch from cross-module call: {errors:?}"
        );
    }

    // ── Stdlib import ───────────────────────────────────────────────

    #[test]
    fn stdlib_import_type_checks() {
        let errors = check_in_memory(
            r#"from std.io import println
function main() = println("hello")"#,
        );
        assert!(errors.is_empty(), "no errors expected: {errors:?}");
    }

    // ── Inline module ───────────────────────────────────────────────

    #[test]
    fn inline_module_checks_independently() {
        let errors =
            check_in_memory("module helper {\n    public function aid() -> Integer = 42\n}");
        assert!(errors.is_empty(), "no errors expected: {errors:?}");
    }
}
