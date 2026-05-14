//! Triết CLI — entry point for the `triet` binary.
//!
//! Subcommands:
//! - `triet run <path>` — run a .tri source or .triv bytecode file.
//! - `triet check <path>` — parse + type-check only, no execution.
//! - `triet build <path>` — compile .tri source to .triv bytecode.
//! - `triet info` — version and project info.
//!
//! Global flags:
//! - `--json` — machine-readable JSON diagnostics.
//! - `--color <auto|always|never>` — control terminal color (default: auto).

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::Path;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use miette::Report;
use triet_interpreter::RuntimeError;
use triet_typecheck::TypeError;

#[derive(Parser)]
#[command(
    name = "triet",
    version,
    about = "Triết — AI-first balanced-ternary language"
)]
struct Cli {
    /// Output diagnostics as JSON instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,

    /// Control whether output uses color.
    #[arg(long, global = true, default_value = "auto")]
    color: ColorArg,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum ColorArg {
    Auto,
    Always,
    Never,
}

#[derive(Subcommand)]
enum Command {
    /// Run a Triết program (.tri source or .triv bytecode).
    Run {
        /// Path to .tri or .triv file.
        path: String,
    },
    /// Parse and type-check a Triết program without running it.
    Check {
        /// Path to .tri source file.
        path: String,
    },
    /// Compile a .tri source file to .triv bytecode.
    Build {
        /// Path to .tri source file.
        path: String,
        /// Output path for .triv bytecode (default: <input>.triv).
        #[arg(short = 'o', long)]
        output: Option<String>,
    },
    /// Print version and build info.
    Info,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Install miette error hook for graphical diagnostics.
    match cli.color {
        ColorArg::Never => {
            miette::set_hook(Box::new(|_| {
                Box::new(miette::MietteHandlerOpts::new().color(false).build())
            }))
            .expect("miette hook");
        }
        ColorArg::Always => {
            miette::set_hook(Box::new(|_| {
                Box::new(miette::MietteHandlerOpts::new().color(true).build())
            }))
            .expect("miette hook");
        }
        ColorArg::Auto => {} // miette auto-detects by default
    }

    match cli.command {
        Command::Run { path } => {
            // Convention: source files end in `.tri`, bytecode in `.triv`.
            // Both are case-sensitive lowercase per ADR-0008.
            if Path::new(&path).extension().and_then(|e| e.to_str()) == Some("triv") {
                run_bytecode(&path, cli.json)
            } else {
                run_program(&path, cli.json)
            }
        }
        Command::Check { path } => check_program(&path, cli.json),
        Command::Build { path, output } => build_program(&path, output, cli.json),
        Command::Info => {
            println!("Triết — balanced ternary, AI-first programming language");
            println!("Language SPEC:     v0.3");
            println!("Implementation:    v{}", env!("CARGO_PKG_VERSION"));
            println!("Spec doc:          SPEC.md");
            println!("Vision:            VISION.md");
            println!("Roadmap:           ROADMAP.md");
            ExitCode::SUCCESS
        }
    }
}

// read_source removed

// ── Diagnostic rendering ─────────────────────────────────────────────

// render removed

// ── JSON output ──────────────────────────────────────────────────────

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

struct JsonEmitter {
    first: bool,
}

impl JsonEmitter {
    fn new() -> Self {
        println!("{{\n  \"errors\": [");
        Self { first: true }
    }

    fn emit(&mut self, message: &str, code: &str, span: &std::ops::Range<usize>, path: &str) {
        if !self.first {
            println!(",");
        }
        self.first = false;
        print!(
            "    {{\"severity\":\"error\",\"message\":{},\"code\":{},\"path\":{},\"span\":{{\"start\":{},\"end\":{}}}}}",
            json_escape(message),
            json_escape(code),
            json_escape(path),
            span.start,
            span.end,
        );
    }

    #[allow(clippy::unused_self)]
    fn finish(self) {
        println!("\n  ]\n}}");
    }
}

fn run_program(path: &str, json: bool) -> ExitCode {
    let display_path = path;

    let resolved_program = match triet_modules::load_program(std::path::Path::new(path)) {
        Ok(p) => p,
        Err(loader_errors) => {
            if json {
                let mut emitter = JsonEmitter::new();
                for error in &loader_errors {
                    let msg = error.to_string();
                    let code = error.code();
                    emitter.emit(&msg, code, &error.span(), display_path);
                }
                emitter.finish();
            } else {
                for error in loader_errors {
                    // We don't have the source for each child file easily accessible here,
                    // so we render the loader error without source code snippets using miette.
                    let report = Report::new(error);
                    eprintln!("{report:?}");
                }
            }
            return ExitCode::from(2);
        }
    };

    let type_errors = triet_typecheck::check_resolved(&resolved_program);
    if !type_errors.is_empty() {
        if json {
            let mut emitter = JsonEmitter::new();
            for error in &type_errors {
                let msg = error.to_string();
                let code = type_error_code(error);
                // The span is in some file, but JSON emitter just takes display_path.
                // For a proper multi-file JSON, we should extract the file from the module.
                emitter.emit(&msg, &code, &error.span(), display_path);
            }
            emitter.finish();
        } else {
            for error in type_errors {
                let report = Report::new(error);
                eprintln!("{report:?}");
            }
        }
        return ExitCode::from(3);
    }

    match triet_interpreter::run_resolved(&resolved_program) {
        Ok(value) => {
            if !json
                && !matches!(
                    value,
                    triet_interpreter::Value::Unit | triet_interpreter::Value::Null
                )
            {
                println!("{value}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            if json {
                let mut emitter = JsonEmitter::new();
                let msg = error.to_string();
                let code = runtime_error_code(&error);
                let span = runtime_error_span(&error);
                emitter.emit(&msg, &code, &span, display_path);
                emitter.finish();
            } else {
                let report = Report::new(error);
                eprintln!("{report:?}");
            }
            ExitCode::from(4)
        }
    }
}

// ── check ────────────────────────────────────────────────────────────

fn check_program(path: &str, json: bool) -> ExitCode {
    let display_path = path;

    let resolved_program = match triet_modules::load_program(std::path::Path::new(path)) {
        Ok(p) => p,
        Err(loader_errors) => {
            if json {
                let mut emitter = JsonEmitter::new();
                for error in &loader_errors {
                    let msg = error.to_string();
                    let code = error.code();
                    emitter.emit(&msg, code, &error.span(), display_path);
                }
                emitter.finish();
            } else {
                for error in loader_errors {
                    let report = Report::new(error);
                    eprintln!("{report:?}");
                }
            }
            return ExitCode::from(2);
        }
    };

    let type_errors = triet_typecheck::check_resolved(&resolved_program);
    if !type_errors.is_empty() {
        if json {
            let mut emitter = JsonEmitter::new();
            for error in &type_errors {
                let msg = error.to_string();
                let code = type_error_code(error);
                emitter.emit(&msg, &code, &error.span(), display_path);
            }
            emitter.finish();
        } else {
            for error in type_errors {
                let report = Report::new(error);
                eprintln!("{report:?}");
            }
        }
        return ExitCode::from(3);
    }

    if !json {
        println!("{display_path}: OK");
    }
    ExitCode::SUCCESS
}

// ── Error code extractors ────────────────────────────────────────────

// parse_error_code removed

fn type_error_code(error: &TypeError) -> String {
    match error {
        TypeError::UnknownType { .. } => "triet::typecheck::E1001",
        TypeError::UndefinedName { .. } => "triet::typecheck::E1002",
        TypeError::Mismatch { .. } => "triet::typecheck::E1003",
        TypeError::InvalidOperands { .. } => "triet::typecheck::E1004",
        TypeError::InvalidUnary { .. } => "triet::typecheck::E1005",
        TypeError::WrongArity { .. } => "triet::typecheck::E1006",
        TypeError::NotCallable { .. } => "triet::typecheck::E1007",
        TypeError::AmbiguousCondition { .. } => "triet::typecheck::E1008",
        TypeError::NonTrileanCondition { .. } => "triet::typecheck::E1009",
        TypeError::DuplicateName { .. } => "triet::typecheck::E1010",
        TypeError::NullLiteralInNonNullableContext { .. } => "triet::typecheck::E1011",
        TypeError::NotNullable { .. } => "triet::typecheck::E1012",
        TypeError::MatchArmMismatch { .. } => "triet::typecheck::E1013",
        TypeError::TupleIndexOutOfRange { .. } => "triet::typecheck::E1014",
        TypeError::UnknownMember { .. } => "triet::typecheck::E1015",
        TypeError::AssignToImmutable { .. } => "triet::typecheck::E1016",
    }
    .to_owned()
}

fn runtime_error_code(error: &RuntimeError) -> String {
    match error {
        RuntimeError::NoMainFunction => "triet::runtime::E2001",
        RuntimeError::UndefinedName { .. } => "triet::runtime::E2002",
        RuntimeError::UnknownCondition { .. } => "triet::runtime::E2003",
        RuntimeError::NonExhaustiveMatch { .. } => "triet::runtime::E2004",
        RuntimeError::Panic { .. } => "triet::runtime::E2005",
        RuntimeError::WrongArity { .. } => "triet::runtime::E2006",
        RuntimeError::TypeError { .. } => "triet::runtime::E2007",
    }
    .to_owned()
}

fn runtime_error_span(error: &RuntimeError) -> std::ops::Range<usize> {
    match error {
        RuntimeError::NoMainFunction => 0..0,
        RuntimeError::UndefinedName { span, .. }
        | RuntimeError::UnknownCondition { span }
        | RuntimeError::NonExhaustiveMatch { span }
        | RuntimeError::Panic { span, .. }
        | RuntimeError::WrongArity { span, .. }
        | RuntimeError::TypeError { span, .. } => span.clone(),
    }
}

// ── build ──────────────────────────────────────────────────────────────

fn build_program(path: &str, output: Option<String>, json: bool) -> ExitCode {
    let display_path = path;

    let resolved = match triet_modules::load_program(Path::new(path)) {
        Ok(p) => p,
        Err(errors) => {
            if json {
                let mut emitter = JsonEmitter::new();
                for error in &errors {
                    emitter.emit(
                        &error.to_string(),
                        error.code(),
                        &error.span(),
                        display_path,
                    );
                }
                emitter.finish();
            } else {
                for error in errors {
                    let report = Report::new(error);
                    eprintln!("{report:?}");
                }
            }
            return ExitCode::from(2);
        }
    };

    let type_errors = triet_typecheck::check_resolved(&resolved);
    if !type_errors.is_empty() {
        if json {
            let mut emitter = JsonEmitter::new();
            for error in &type_errors {
                emitter.emit(
                    &error.to_string(),
                    &type_error_code(error),
                    &error.span(),
                    display_path,
                );
            }
            emitter.finish();
        } else {
            for error in type_errors {
                let report = Report::new(error);
                eprintln!("{report:?}");
            }
        }
        return ExitCode::from(3);
    }

    let ir = triet_ir::lower_program(&resolved);
    let bytes = triet_ir::write_program(&ir);

    let output_path = output.unwrap_or_else(|| {
        let p = Path::new(path);
        if p.extension().and_then(|e| e.to_str()) == Some("tri") {
            p.with_extension("triv").to_string_lossy().into_owned()
        } else {
            format!("{path}.triv")
        }
    });

    match std::fs::write(&output_path, &bytes) {
        Ok(()) => {
            if !json {
                let funcs = ir.function_count();
                let size = bytes.len();
                eprintln!("Compiled {path} → {output_path} ({funcs} functions, {size} bytes)");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: could not write {output_path}: {e}");
            ExitCode::from(5)
        }
    }
}

// ── run bytecode (.triv) ───────────────────────────────────────────────

fn run_bytecode(path: &str, json: bool) -> ExitCode {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: cannot read {path}: {e}");
            return ExitCode::from(5);
        }
    };

    let ir = match triet_ir::read_program(&bytes) {
        Ok(program) => program,
        Err(e) => {
            if json {
                let mut emitter = JsonEmitter::new();
                emitter.emit(&e.to_string(), "triet::modules::E2103", &(0..0), path);
                emitter.finish();
            } else {
                eprintln!("Error: {e}");
            }
            return ExitCode::from(5);
        }
    };

    let func_to_run = find_entry_function(&ir);

    let mut vm = triet_ir::Vm::new(ir);
    match vm.execute(func_to_run, vec![]) {
        Ok(value) => {
            if !json && !matches!(value, triet_ir::RuntimeValue::Unit) {
                println!("{value}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            if json {
                let mut emitter = JsonEmitter::new();
                let msg = error.to_string();
                emitter.emit(&msg, "triet::runtime::E2200", &(0..0), path);
                emitter.finish();
            } else {
                eprintln!("Error: {error}");
            }
            ExitCode::from(4)
        }
    }
}

/// Find the best entry function in the IR program.
/// Prefers a function named "main". Falls back to the first function.
fn find_entry_function(ir: &triet_ir::IrProgram) -> triet_ir::FuncId {
    let mut first: Option<triet_ir::FuncId> = None;

    for module in &ir.modules {
        for func in &module.functions {
            if first.is_none() {
                first = Some(func.id);
            }
            if func.name.as_deref() == Some("main") {
                return func.id;
            }
        }
    }

    first.unwrap_or(triet_ir::FuncId(0))
}
