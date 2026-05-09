//! Triết CLI — entry point for the `triet` binary.
//!
//! Subcommands:
//! - `triet run <path>` — parse, type-check, and run the program.
//! - `triet check <path>` — parse + type-check only, no execution.
//! - `triet info` — version and project info.
//!
//! Global flags:
//! - `--json` — machine-readable JSON diagnostics.
//! - `--color <auto|always|never>` — control terminal color (default: auto).

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::{fs, process::ExitCode};

use clap::{Parser, Subcommand, ValueEnum};
use miette::{NamedSource, Report};
use triet_interpreter::RuntimeError;
use triet_parser::ParseError;
use triet_typecheck::TypeError;

#[derive(Parser)]
#[command(name = "triet", version, about = "Triết — AI-first balanced-ternary language")]
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
    /// Run a Triết program (.tri file).
    Run {
        /// Path to .tri source file.
        path: String,
    },
    /// Parse and type-check a Triết program without running it.
    Check {
        /// Path to .tri source file.
        path: String,
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
        Command::Run { path } => run_program(&path, cli.json),
        Command::Check { path } => check_program(&path, cli.json),
        Command::Info => {
            println!("Triết v{}", env!("CARGO_PKG_VERSION"));
            println!("Balanced ternary, AI-first programming language");
            println!("Spec: SPEC.md");
            ExitCode::SUCCESS
        }
    }
}

fn read_source(path: &str) -> Result<(String, String), ExitCode> {
    fs::read_to_string(path)
        .map(|src| (src, path.to_owned()))
        .map_err(|error| {
            eprintln!("could not read {path}: {error}");
            ExitCode::from(1)
        })
}

// ── Diagnostic rendering ─────────────────────────────────────────────

fn render<E: miette::Diagnostic + Send + Sync + 'static>(
    error: E,
    source: &str,
    path: &str,
) {
    let src = NamedSource::new(path, source.to_owned());
    let report = Report::new(error).with_source_code(src);
    eprintln!("{report:?}");
}

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

// ── run ──────────────────────────────────────────────────────────────

fn run_program(path: &str, json: bool) -> ExitCode {
    let (source, display_path) = match read_source(path) {
        Ok(s) => s,
        Err(code) => return code,
    };

    let (program, parse_errors) = triet_parser::parse(&source);
    if !parse_errors.is_empty() {
        if json {
            let mut emitter = JsonEmitter::new();
            for error in &parse_errors {
                let msg = error.to_string();
                let code = parse_error_code(error);
                emitter.emit(&msg, &code, &error.span(), &display_path);
            }
            emitter.finish();
        } else {
            for error in parse_errors {
                render(error, &source, &display_path);
            }
        }
        return ExitCode::from(2);
    }

    let type_errors = triet_typecheck::check(&program);
    if !type_errors.is_empty() {
        if json {
            let mut emitter = JsonEmitter::new();
            for error in &type_errors {
                let msg = error.to_string();
                let code = type_error_code(error);
                emitter.emit(&msg, &code, &error.span(), &display_path);
            }
            emitter.finish();
        } else {
            for error in type_errors {
                render(error, &source, &display_path);
            }
        }
        return ExitCode::from(3);
    }

    match triet_interpreter::run(&program) {
        Ok(value) => {
            if !json
                && !matches!(
                    value,
                    triet_interpreter::Value::Unit | triet_interpreter::Value::Null
                ) {
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
                emitter.emit(&msg, &code, &span, &display_path);
                emitter.finish();
            } else {
                render(error, &source, &display_path);
            }
            ExitCode::from(4)
        }
    }
}

// ── check ────────────────────────────────────────────────────────────

fn check_program(path: &str, json: bool) -> ExitCode {
    let (source, display_path) = match read_source(path) {
        Ok(s) => s,
        Err(code) => return code,
    };

    let (program, parse_errors) = triet_parser::parse(&source);
    if !parse_errors.is_empty() {
        if json {
            let mut emitter = JsonEmitter::new();
            for error in &parse_errors {
                let msg = error.to_string();
                let code = parse_error_code(error);
                emitter.emit(&msg, &code, &error.span(), &display_path);
            }
            emitter.finish();
        } else {
            for error in parse_errors {
                render(error, &source, &display_path);
            }
        }
        return ExitCode::from(2);
    }

    let type_errors = triet_typecheck::check(&program);
    if !type_errors.is_empty() {
        if json {
            let mut emitter = JsonEmitter::new();
            for error in &type_errors {
                let msg = error.to_string();
                let code = type_error_code(error);
                emitter.emit(&msg, &code, &error.span(), &display_path);
            }
            emitter.finish();
        } else {
            for error in type_errors {
                render(error, &source, &display_path);
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

fn parse_error_code(error: &ParseError) -> String {
    match error {
        ParseError::UnexpectedToken { .. } => "triet::parse::E0001",
        ParseError::UnexpectedEof { .. } => "triet::parse::E0002",
        ParseError::ChainedNoChainOperator { .. } => "triet::parse::E0003",
        ParseError::InvalidInterpolation { .. } => "triet::parse::E0004",
        ParseError::InvalidLiteral { .. } => "triet::parse::E0005",
        ParseError::BreakValueOutsideLoop { .. } => "triet::parse::E0006",
        ParseError::InvalidAssignmentTarget { .. } => "triet::parse::E0007",
        ParseError::Lex(_) => "triet::lex::E0000",
    }
    .to_owned()
}

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
