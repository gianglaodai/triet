//! Triết Track B pipeline driver — end-to-end borrow checking.
//!
//! Reads a `.tri` source file and runs the full Track B pipeline:
//! lexer → parser → typecheck → lower → MIR → borrow check.
//! Borrow-check errors are rendered with [`miette`] for rich source-level
//! diagnostics (line/column, labelled spans, help text).

#![warn(missing_docs)]

use std::process::ExitCode;

use miette::{NamedSource, Report};

fn main() -> ExitCode {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: triet-driver <file.tri>");
        std::process::exit(2);
    });

    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };

    // ── Phase 1: Parse ──
    let (program, parse_errors) = triet_parser::parse(&source);
    if !parse_errors.is_empty() {
        let src = NamedSource::new(&path, source.clone());
        for err in &parse_errors {
            let report = Report::new(err.clone()).with_source_code(src.clone());
            eprintln!("{report:?}");
        }
        return ExitCode::from(2);
    }

    // ── Phase 2: Typecheck (validator only — errors are reported but
    // do not block the pipeline; the lowerer/borrowck process the AST
    // independently) ──
    let type_errors = triet_typecheck::check(&program);
    let has_type_errors = !type_errors.is_empty();
    if has_type_errors {
        let src = NamedSource::new(&path, source.clone());
        for err in &type_errors {
            let report = Report::new(err.clone()).with_source_code(src.clone());
            eprintln!("{report:?}");
        }
    }

    // ── Phase 3: Lower to MIR ──
    let bodies = triet_lower::lower_program(&program);

    // ── Phase 4: Borrow check ──
    let mut has_errors = false;
    let src = NamedSource::new(&path, source.clone());
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

    if has_errors || has_type_errors {
        ExitCode::from(3)
    } else {
        eprintln!("{path}: OK (no borrow errors)");
        ExitCode::SUCCESS
    }
}
