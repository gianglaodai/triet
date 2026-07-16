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
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    if !type_errors.is_empty() {
        let src = NamedSource::new(path, source.clone());
        for err in &type_errors {
            let report = Report::new(err.clone()).with_source_code(src.clone());
            eprintln!("{report:?}");
        }
        return ExitCode::from(3);
    }

    // ── Phase 3: Lower to MIR ──
    let bodies =
        match triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions) {
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
    // ADR-0045 §2/B4: collect callee signatures for cross-call loan
    // propagation (PropagatedLoan) and M3+ move-mark by passing mode.
    let callee_sigs: std::collections::BTreeMap<String, triet_mir::FunctionSignature> = bodies
        .iter()
        .map(|b| (b.signature.name.clone(), b.signature.clone()))
        .collect();
    let mut has_errors = false;
    let src = NamedSource::new(path, source.clone());
    for body in &bodies {
        let result = triet_borrowck::checker::check_body_with(body, &callee_sigs);
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

    // ADR-0040 §3.3: String literal bytes live in ConstValue::String within Body.
    // Body must outlive the JIT module. `bodies` stays alive until process exit.
    let shims = &[
        ShimSymbol::fn_2_1("__triet_pow", mir_lower::__triet_pow),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", mir_lower::__triet_string_free),
        ShimSymbol::fn_5_0("__triet_string_concat", mir_lower::__triet_string_concat),
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
        // ADR-0080 Mũi B: content hash for String HashMap keys.
        ShimSymbol::fn_2_1("__triet_string_hash", mir_lower::__triet_string_hash),
        // Vector shims (ADR-0040 §5)
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", mir_lower::__triet_vector_free),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_2_1("__triet_vector_get", mir_lower::__triet_vector_get),
        ShimSymbol::fn_2_1("__triet_vector_pop", mir_lower::__triet_vector_pop),
        ShimSymbol::fn_2_1(
            "__triet_vector_pop_front",
            mir_lower::__triet_vector_pop_front,
        ),
        // HashMap shims (ADR-0043; ADR-0080 key-typed P1 bumped alloc/insert/
        // remove to fn_4_1 — +key_stride / +is_update_out / +key_out_ptr).
        ShimSymbol::fn_6_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", mir_lower::__triet_hashmap_free),
        ShimSymbol::fn_1_1("__triet_hashmap_len", mir_lower::__triet_hashmap_len),
        ShimSymbol::fn_4_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_2_1("__triet_hashmap_get", mir_lower::__triet_hashmap_get),
        ShimSymbol::fn_4_1("__triet_hashmap_remove", mir_lower::__triet_hashmap_remove),
        // ADR-0079: get_ref shims (zero-copy borrow)
        ShimSymbol::fn_2_1("__triet_vector_get_ref", mir_lower::__triet_vector_get_ref),
        ShimSymbol::fn_2_1(
            "__triet_hashmap_get_ref",
            mir_lower::__triet_hashmap_get_ref,
        ),
        // ADR-0082 §AMEND-3: get-by-value COPY shims for Copy-aggregate
        // elements. Reuse the `_get_ref` Rust functions verbatim under a
        // second registered symbol name (same locate-the-cell logic,
        // returns cell_ptr/NULL_SENTINEL) — the JIT dispatch for THIS name
        // additionally memcpy's `stride` bytes out at the call site (no
        // Rust-side change). Kept as a distinct MIR name (not a literal
        // reuse of "__triet_..._get_ref") so borrowck's `returns_borrow_of`
        // stays `None` for get-by-value (see triet-mir builtin_shim_meta).
        ShimSymbol::fn_2_1("__triet_vector_get_copy", mir_lower::__triet_vector_get_ref),
        ShimSymbol::fn_2_1(
            "__triet_hashmap_get_copy",
            mir_lower::__triet_hashmap_get_ref,
        ),
        // ADR-0079 §AMEND (Slice 2, composes ADR-0084): get_ref shims for an
        // AGGREGATE element/value (Struct/Enum, including heap-bearing).
        // Distinct Rust functions (mir_lower.rs `__triet_vector_get_ref_agg`/
        // `__triet_hashmap_get_ref_agg`) — NOT a reuse of `_get_ref` under a
        // second symbol name like `_get_copy` above, because the underlying
        // logic differs: `_get_ref` derefs the cell for stride<=8 (matching
        // a heap-scalar/container-handle element's representation), which
        // would be WRONG for an aggregate element (its cell holds the
        // struct's bits, not a handle) — see the Rust doc comment on
        // `__triet_vector_get_ref_agg` for the full hazard.
        ShimSymbol::fn_2_1(
            "__triet_vector_get_ref_agg",
            mir_lower::__triet_vector_get_ref_agg,
        ),
        ShimSymbol::fn_2_1(
            "__triet_hashmap_get_ref_agg",
            mir_lower::__triet_hashmap_get_ref_agg,
        ),
        // ADR-0047: contains shims
        ShimSymbol::fn_4_1(
            "__triet_string_contains",
            mir_lower::__triet_string_contains,
        ),
        ShimSymbol::fn_2_1(
            "__triet_vector_contains",
            mir_lower::__triet_vector_contains,
        ),
        ShimSymbol::fn_2_1(
            "__triet_hashmap_contains",
            mir_lower::__triet_hashmap_contains,
        ),
        // ADR-0048/0049: mutable borrow — clear + append
        ShimSymbol::fn_1_1("__triet_string_clear", mir_lower::__triet_string_clear),
        ShimSymbol::fn_2_1("__triet_string_append", mir_lower::__triet_string_append),
        // ADR-0069 Lát 3: capability runtime policy hook (defer mint gate).
        ShimSymbol::fn_1_1("__triet_cap_check", mir_lower::__triet_cap_check),
    ];
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
                // Bậc A: main() must have 0 parameters — the JIT only
                // supports calling with 0 arguments today.
                if !body.signature.parameters.is_empty() {
                    eprintln!(
                        "{}: main() has {} parameter(s) — \
                         Bậc A JIT does not support arguments to main()",
                        path,
                        body.signature.parameters.len()
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
