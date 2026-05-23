//! v0.7.x.runtime-fix.const-items — top-level `constant NAME: T = LITERAL`
//! lowering smoke. Pre-fix, references to a module-level constant fell
//! through `lower_identifier` and silently produced `Unit` at runtime
//! (cycle detection in `compiler/modules.tri` hit infinite recursion
//! when its color sentinels read back as Unit instead of the declared
//! Integer). Pass 1c now interns each literal initializer into the
//! constant pool and routes `Identifier(name)` to a `Const` instruction.

use triet_ir::{Vm, lower_program};
use triet_modules::load_program_from_source;
use triet_typecheck::check_resolved;

use miette::Diagnostic;

fn run_and_get_main(source: &str) -> triet_ir::RuntimeValue {
    let resolved = load_program_from_source(source).expect("load");
    let diagnostics = check_resolved(&resolved);
    let blocking: Vec<_> = diagnostics
        .iter()
        .filter(|err| err.severity() != Some(miette::Severity::Warning))
        .collect();
    assert!(blocking.is_empty(), "type errors: {blocking:?}");
    let ir = lower_program(&resolved);
    let main_id = ir
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("main"))
        .expect("missing main()")
        .id;
    let mut vm = Vm::new(ir);
    vm.execute(main_id, vec![]).expect("vm execute")
}

#[test]
fn integer_constant_reads_declared_value() {
    let source = r"
        constant OPCODE_CONST: Integer = 42
        function main() -> Integer = OPCODE_CONST
    ";
    let result = run_and_get_main(source);
    match result {
        triet_ir::RuntimeValue::Integer(i) => assert_eq!(i.to_i64(), 42),
        other => panic!("expected Integer(42), got {other:?}"),
    }
}

#[test]
fn integer_constant_via_arithmetic() {
    let source = r"
        constant BASE: Integer = 10
        function main() -> Integer = BASE + BASE
    ";
    let result = run_and_get_main(source);
    match result {
        triet_ir::RuntimeValue::Integer(i) => assert_eq!(i.to_i64(), 20),
        other => panic!("expected Integer(20), got {other:?}"),
    }
}

#[test]
fn string_constant_reads_declared_value() {
    let source = r#"
        constant GREETING: String = "hello"
        function main() -> String = GREETING
    "#;
    let result = run_and_get_main(source);
    match result {
        triet_ir::RuntimeValue::String(s) => assert_eq!(s, "hello"),
        other => panic!("expected String(\"hello\"), got {other:?}"),
    }
}

#[test]
fn multiple_constants_distinct_pool_entries() {
    let source = r"
        constant OPCODE_CONST: Integer = 0
        constant OPCODE_ADD: Integer = 1
        constant OPCODE_SUB: Integer = 2
        function main() -> Integer = OPCODE_CONST + OPCODE_ADD + OPCODE_SUB
    ";
    let result = run_and_get_main(source);
    match result {
        triet_ir::RuntimeValue::Integer(i) => assert_eq!(i.to_i64(), 3),
        other => panic!("expected Integer(3), got {other:?}"),
    }
}

#[test]
fn constant_distinct_from_local_shadow() {
    // Local `let X = 99` should shadow the top-level constant inside its
    // scope. Outside the scope the constant value is used.
    let source = r"
        constant X: Integer = 1
        function inner() -> Integer = {
            let X: Integer = 99
            X
        }
        function outer() -> Integer = X
        function main() -> Integer = inner() + outer()
    ";
    let result = run_and_get_main(source);
    match result {
        triet_ir::RuntimeValue::Integer(i) => assert_eq!(i.to_i64(), 100),
        other => panic!("expected Integer(100), got {other:?}"),
    }
}
