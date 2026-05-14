//! Module-aware tree-walking interpreter.
//!
//! Evaluates a [`ResolvedProgram`] by building a global binding map of
//! all functions and constants across all modules, then delegating to
//! the core interpreter to run `main`.

use std::{collections::HashMap, rc::Rc};

use triet_modules::{AbsolutePath, ResolvedProgram};
use triet_syntax::Item;

use crate::{
    error::RuntimeError,
    interpret,
    value::{FunctionRef, Value},
};

/// Run a multi-module program by calling `main` in the root module.
///
/// # Errors
///
/// Returns a [`RuntimeError`] on runtime failure (panic, missing main).
pub fn run_resolved(program: &ResolvedProgram) -> Result<Value, RuntimeError> {
    // 1. Evaluate all constants and collect all functions into a
    //    global absolute-path → Value map.
    let globals = evaluate_all_globals(program)?;

    // 2. We need an interpreter. The standard `Interpreter` executes
    //    a single `arena` and `items` slice. But function calls cross
    //    module boundaries. We will adapt the standard interpreter
    //    so that `invoke_function` can switch the `arena` and `items`
    //    if the function has a `module_id`.
    //
    // However, `interpret.rs`'s `Interpreter` struct is private. Let's
    // modify `Interpreter` to accept a `resolved_program` and `globals`
    // map.

    // Instead of duplicating `Interpreter`, we'll modify it in `interpret.rs`.
    // We just need to expose a new entry point there.

    interpret::run_resolved_internal(program, globals)
}

/// Pre-evaluate all constants and collect all functions across all modules.
fn evaluate_all_globals(
    program: &ResolvedProgram,
) -> Result<HashMap<AbsolutePath, Value>, RuntimeError> {
    let mut globals = HashMap::new();

    // Pass 1: Collect functions. They don't need evaluation.
    for module in &program.modules {
        let mod_id = program.find_module(&module.path).unwrap();
        for item in &module.items {
            if let Item::Function(def) = &item.node {
                let path = AbsolutePath::new(module.path.clone(), def.name.clone());

                let value = if path.module_path().root() == Some("std") {
                    if let Some(builtin) = crate::builtins::get_builtin(&module.path, &def.name) {
                        Value::Builtin(builtin)
                    } else {
                        Value::Function(Rc::new(FunctionRef {
                            def: def.clone(),
                            module_id: Some(mod_id),
                        }))
                    }
                } else {
                    Value::Function(Rc::new(FunctionRef {
                        def: def.clone(),
                        module_id: Some(mod_id),
                    }))
                };

                globals.insert(path, value);
            }
        }
    }

    // Pass 2: Evaluate constants. A constant in module A might use
    // functions/constants from module B, so we need a module-aware
    // evaluation environment.
    //
    // For simplicity in v0.2.x, we'll evaluate constants by setting up
    // an interpreter for that specific module, pre-seeded with all
    // imported names (which might trigger recursive const evaluation
    // if we aren't careful, but Triết doesn't support cyclic consts).
    // Actually, we can just do a topological evaluation or just evaluate
    // them sequentially, since cycle detection already prevents cyclic
    // imports.

    for module in &program.modules {
        let mod_id = program.find_module(&module.path).unwrap();

        for item in &module.items {
            if let Item::Const { name, value, .. } = &item.node {
                let path = AbsolutePath::new(module.path.clone(), name.clone());

                // Set up a temporary interpreter for this module to
                // evaluate the constant.
                let mut temp_interp =
                    interpret::Interpreter::new_for_module(program, mod_id, &globals);

                let const_val = temp_interp.evaluate_expression_public(*value)?;
                globals.insert(path, const_val);
            }
        }
    }

    Ok(globals)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_in_memory(source: &str) -> Result<Value, RuntimeError> {
        let program = triet_modules::load_program_from_source(source).unwrap();
        run_resolved(&program)
    }

    fn run_filesystem(files: &[(&str, &str)]) -> Result<Value, RuntimeError> {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path();
        let mut root_path = None;

        for (rel_path, contents) in files {
            let full = base.join(rel_path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, contents).unwrap();
            if root_path.is_none() {
                root_path = Some(full);
            }
        }

        let program = triet_modules::load_program(root_path.as_ref().unwrap()).unwrap();
        run_resolved(&program)
    }

    #[test]
    fn single_module_happy_path() {
        let value = run_in_memory("function main() -> Integer = 42").unwrap();
        assert_eq!(value, Value::Integer(triet_core::Integer::new(42).unwrap()));
    }

    #[test]
    fn cross_module_function_call() {
        let value = run_filesystem(&[
            (
                "main.tri",
                "module helper\nfrom crate.helper import greet\nfunction main() -> Integer = greet()",
            ),
            ("helper.tri", "public function greet() -> Integer = 42"),
        ]).unwrap();
        assert_eq!(value, Value::Integer(triet_core::Integer::new(42).unwrap()));
    }

    #[test]
    fn cross_module_const_evaluation() {
        let value = run_filesystem(&[
            (
                "main.tri",
                "module helper\nfrom crate.helper import ANSWER\nfunction main() -> Integer = ANSWER",
            ),
            ("helper.tri", "public constant ANSWER: Integer = 100"),
        ]).unwrap();
        assert_eq!(
            value,
            Value::Integer(triet_core::Integer::new(100).unwrap())
        );
    }

    #[test]
    fn stdlib_print_does_not_panic() {
        let value = run_in_memory(
            r#"from std.io import println
function main() = println("hello")"#,
        )
        .unwrap();
        assert_eq!(value, Value::Unit);
    }
}
