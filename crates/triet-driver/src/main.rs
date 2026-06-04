//! Triết Track B pipeline driver — end-to-end compilation + execution.
//!
//! Reads a `.tri` source file and runs the full Track B pipeline:
//! lexer → parser → typecheck → lower → MIR → borrow check → JIT → execute.
//!
//! Usage:
//!   triet-driver <file.tri>          check only (parse + typecheck + lower + borrowck)
//!   triet-driver run <file.tri>      compile + execute via JIT

#![warn(missing_docs)]

use std::process::ExitCode;

use miette::{NamedSource, Report};
use triet_jit::mir_lower::{CompiledFunction, JitContext, ShimSymbol};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let run_mode = if args.first().map(|s| s.as_str()) == Some("run") {
        args.remove(0);
        true
    } else {
        false
    };

    let path = args.first().unwrap_or_else(|| {
        if run_mode {
            eprintln!("Usage: triet-driver run <file.tri>");
        } else {
            eprintln!("Usage: triet-driver <file.tri>");
        }
        std::process::exit(2);
    });

    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };

    // ── Phase 1: Parse ──
    let (program, parse_errors) = triet_parser::parse(&source);
    if !parse_errors.is_empty() {
        let src = NamedSource::new(path, source.clone());
        for err in &parse_errors {
            let report = Report::new(err.clone()).with_source_code(src.clone());
            eprintln!("{report:?}");
        }
        return ExitCode::from(2);
    }

    // ── Phase 2: Typecheck ──
    // Type errors are FATAL — the pipeline must not feed invalid AST
    // to the lowerer/borrowck/JIT layers.
    let type_errors = triet_typecheck::check(&program);
    if !type_errors.is_empty() {
        let src = NamedSource::new(path, source.clone());
        for err in &type_errors {
            let report = Report::new(err.clone()).with_source_code(src.clone());
            eprintln!("{report:?}");
        }
        return ExitCode::from(3);
    }

    // ── Phase 3: Lower to MIR ──
    let bodies = match triet_lower::lower_program(&program) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{path}: lowerer error: {e}");
            return ExitCode::from(3);
        }
    };

    if bodies.is_empty() {
        eprintln!("{path}: no functions to compile");
        return ExitCode::from(2);
    }

    // ── Phase 3.5: MIR verification ──
    // Run BEFORE borrowck and JIT so they can assume well-formed MIR.
    for body in &bodies {
        if let Err(e) = body.verify() {
            eprintln!("{path}: MIR verification error: {e}");
            return ExitCode::from(3);
        }
        println!("{}", body);
    }

    // ── Phase 4: Borrow check ──
    let mut has_errors = false;
    let src = NamedSource::new(path, source.clone());
    for body in &bodies {
        let result = triet_borrowck::checker::check_body(body);
        if !result.is_ok() {
            has_errors = true;
            for err in &result.errors {
                let report = Report::new(err.clone()).with_source_code(src.clone());
                eprintln!("{report:?}");
            }
        }
    }

    if has_errors {
        return ExitCode::from(3);
    }

    if !run_mode {
        eprintln!("{path}: OK (no borrow errors)");
        return ExitCode::SUCCESS;
    }

    // ── Phase 5: JIT compile + execute ──
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();

    // Register runtime shims. The type-safe factory methods
    // (`fn_2_1` = 2 args → 1 return) enforce correct signatures
    // at compile time — no manual arity counting needed.
    use triet_jit::mir_lower;

    let shims = &[ShimSymbol::fn_2_1("__triet_pow", mir_lower::__triet_pow)];
    let mut ctx = JitContext::with_shims(shims);
    let compiled = match ctx.compile_multi(&body_refs) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("JIT compilation error: {e}");
            return ExitCode::from(4);
        }
    };

    // Find and call `main` function (arity 0 → return value)
    let main_entry = bodies
        .iter()
        .find(|b| b.signature.name == "main")
        .or_else(|| bodies.first());

    match main_entry {
        Some(body) => match compiled.get(&body.signature.name) {
            Some(func) => {
                // Bậc A: main() must have 0 params — the JIT only
                // supports calling with 0 arguments today.
                if !body.signature.params.is_empty() {
                    eprintln!(
                        "{}: main() has {} parameter(s) — \
                         Bậc A JIT does not support arguments to main()",
                        path,
                        body.signature.params.len()
                    );
                    return ExitCode::from(3);
                }
                let result = execute_main(func);
                println!("{result}");
                ExitCode::SUCCESS
            }
            None => {
                eprintln!(
                    "Function `{}` not found in compiled output",
                    body.signature.name
                );
                ExitCode::from(4)
            }
        },
        None => {
            eprintln!("No functions to execute");
            ExitCode::from(4)
        }
    }
}

/// Call the compiled `main` function with the appropriate number of args.
///
/// # Safety
/// The JIT module that produced `func` must still be alive.
#[allow(unsafe_code)]
fn execute_main(func: &CompiledFunction) -> i64 {
    unsafe { func.call_i64_0() }
}
