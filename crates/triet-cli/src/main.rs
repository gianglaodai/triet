//! Triết CLI — entry point for the `dao` binary.
//!
//! Subcommands (primary: English, alias: Vietnamese per ADR-0024):
//! - `dao run <path>` / `chay` — run a .tri source or .triv bytecode file.
//! - `dao check <path>` / `kiem` — parse + type-check only, no execution.
//! - `dao build <path>` / `tao` — compile .tri source to .khi package.
//! - `dao store import <path>` / `kho` — manage CAS package store (~/.triet/store/).
//! - `dao fmt --migrate-null [--write] <path>` — apply source-level
//!   migrations (currently: ADR-0020 `null` → `~0`).
//! - `dao info` — version and project info.
//!
//! Global flags:
//! - `--json` — machine-readable JSON diagnostics.
//! - `--color <auto|always|never>` — control terminal color (default: auto).

#![allow(clippy::print_stdout, clippy::print_stderr)]

mod fmt;

use std::path::Path;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use miette::Report;
use triet_interpreter::RuntimeError;
use triet_pack::{AbiMetadata, PackageManifest, SemVer, check_cap_divergence, read_khi, write_khi};
use triet_typecheck::{CapabilityError, TypeError};

use crate::fmt::fmt_command;

#[derive(Parser)]
#[command(
    name = "dao",
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
    /// Run a Triết program (.tri source, .triv bytecode, or .khi package).
    #[command(alias = "chay")]
    Run {
        /// Path to .tri, .triv, or .khi file.
        path: String,

        /// Arguments passed to the program.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Parse and type-check a Triết program without running it.
    #[command(alias = "kiem")]
    Check {
        /// Path to .tri source file.
        path: String,
    },
    /// Compile a .tri source file to a .khi package.
    #[command(alias = "tao")]
    Build {
        /// Path to .tri source file.
        path: String,
        /// Output path for .khi package (default: <input>.khi).
        #[arg(short = 'o', long)]
        output: Option<String>,
    },
    /// Manage the local CAS package store (~/.triet/store/).
    #[command(alias = "kho")]
    Store {
        #[command(subcommand)]
        subcommand: StoreCommand,
    },
    /// Source-level formatter / migrator (v0.7.4.3-error.4a).
    ///
    /// Currently exposes a single migration rule via `--migrate-null`
    /// (ADR-0020 §10): rewrite the deprecated `null` keyword to its
    /// canonical `~0` form. Other rules will land alongside future
    /// deprecations and reuse the same `dao fmt` entry point.
    Fmt {
        /// Apply the `null` → `~0` migration (ADR-0020 §10).
        #[arg(long)]
        migrate_null: bool,
        /// Write changes back to disk. Without this flag, the command
        /// prints the unified diff to stdout (dry-run).
        #[arg(long)]
        write: bool,
        /// File or directory to operate on. Directories are walked
        /// recursively; only `*.tri` files are touched.
        path: String,
    },
    /// Print version and build info.
    Info,
}

#[derive(Subcommand)]
enum StoreCommand {
    /// Install a .khi into the CAS store.
    Import {
        /// Path to .khi file.
        path: String,
    },
    /// List packages currently installed in the store.
    List {
        /// Show full 64-char hashes instead of the 12-char abbreviation.
        #[arg(long)]
        full: bool,
    },
    /// Garbage-collect unreachable packs / modules / terms.
    Gc,
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
        Command::Run { path, args } => {
            // Convention: source files end in `.tri`. Bytecode can be
            // `.triv` (v0.3 wire format) or `.khi` (v0.4+ package format
            // wrapping `.triv` code inside ABI metadata).
            let ext = Path::new(&path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if ext == "triv" || ext == "khi" {
                run_bytecode(&path, &args, cli.json)
            } else {
                run_program(&path, &args, cli.json)
            }
        }
        Command::Check { path } => check_program(&path, cli.json),
        Command::Build { path, output } => build_program(&path, output, cli.json),
        Command::Store { subcommand } => store_command(subcommand, cli.json),
        Command::Fmt {
            migrate_null,
            write,
            path,
        } => fmt_command(&path, migrate_null, write),
        Command::Info => {
            println!("Triết — balanced ternary, AI-first programming language");
            println!("Language SPEC:     v0.7");
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

fn run_program(path: &str, _args: &[String], json: bool) -> ExitCode {
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

    let type_diagnostics = triet_typecheck::check_resolved(&resolved_program);
    if !type_diagnostics.is_empty() {
        use miette::Diagnostic;
        // v0.7.4.3-error.2: split into hard errors (block exit) and
        // warnings (display but continue per ADR-0020 §10.3 W2001 contract).
        let has_hard_errors = type_diagnostics
            .iter()
            .any(|err| err.severity() != Some(miette::Severity::Warning));
        if json {
            let mut emitter = JsonEmitter::new();
            for error in &type_diagnostics {
                let msg = error.to_string();
                let code = type_error_code(error);
                // The span is in some file, but JSON emitter just takes display_path.
                // For a proper multi-file JSON, we should extract the file from the module.
                emitter.emit(&msg, &code, &error.span(), display_path);
            }
            emitter.finish();
        } else {
            for error in &type_diagnostics {
                let report = Report::new(error.clone());
                eprintln!("{report:?}");
            }
        }
        if has_hard_errors {
            return ExitCode::from(3);
        }
        // Only warnings — continue.
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

    // v0.8.11: discover dao.package for capability checking in `dao check`.
    let source_path = std::path::Path::new(path);
    let _manifest = source_path
        .parent()
        .and_then(triet_pack::PackageManifest::discover)
        .and_then(|p| triet_pack::PackageManifest::load(&p).ok());

    let resolved_program = match triet_modules::load_program(source_path) {
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

    let type_diagnostics = triet_typecheck::check_resolved(&resolved_program);
    if !type_diagnostics.is_empty() {
        use miette::Diagnostic;
        let has_hard_errors = type_diagnostics
            .iter()
            .any(|err| err.severity() != Some(miette::Severity::Warning));
        if json {
            let mut emitter = JsonEmitter::new();
            for error in &type_diagnostics {
                let msg = error.to_string();
                let code = type_error_code(error);
                emitter.emit(&msg, &code, &error.span(), display_path);
            }
            emitter.finish();
        } else {
            for error in &type_diagnostics {
                let report = Report::new(error.clone());
                eprintln!("{report:?}");
            }
        }
        if has_hard_errors {
            return ExitCode::from(3);
        }
    }

    // v0.8.11: run capability check with discovered manifest or empty one if missing.
    let manifest_path = source_path
        .parent()
        .and_then(triet_pack::PackageManifest::discover);

    let empty_manifest =
        triet_pack::PackageManifest::new("unknown", triet_pack::SemVer::new(0, 0, 0));
    let manifest = match manifest_path {
        Some(p) => match triet_pack::PackageManifest::load(&p) {
            Ok(m) => Some(m),
            Err(triet_pack::StoreError::PackageManifest(e)) => {
                if json {
                    let mut emitter = JsonEmitter::new();
                    emitter.emit(
                        &e.to_string(),
                        package_manifest_error_code(&e),
                        &(0..0),
                        display_path,
                    );
                    emitter.finish();
                } else {
                    eprintln!("{}: {}", package_manifest_error_code(&e), e);
                }
                return ExitCode::from(5);
            }
            Err(e) => {
                if !json {
                    eprintln!("triet::capability::E2208: {e}");
                }
                return ExitCode::from(5);
            }
        },
        None => None,
    };

    let m = manifest.as_ref().unwrap_or(&empty_manifest);
    if !run_capability_check(&resolved_program, m, display_path, json) {
        return ExitCode::from(5);
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
        // v0.7.4.3-error.2 (ADR-0020 §9) — outcome error codes:
        TypeError::NullableErrorInOutcomeType { .. } => "triet::typecheck::E1024",
        TypeError::NullStateInBinaryOutcome { .. } => "triet::typecheck::E1025",
        TypeError::NonExhaustiveOutcomeMatch { .. } => "triet::typecheck::E1026",
        TypeError::OutcomeTypeMismatch { .. } => "triet::typecheck::E1027",
        TypeError::PropagateInNonFallibleContext { .. } => "triet::typecheck::E1028",
        TypeError::ErrorTypeMismatch { .. } => "triet::typecheck::E1029",
        TypeError::OutcomePropagateMissingCapture { .. } => "triet::typecheck::E1030",
        TypeError::OutcomePropagateMalformedReturn { .. } => "triet::typecheck::E1031",
        TypeError::PatternMissingExplicitConstructor { .. } => "triet::typecheck::E1032",
        TypeError::PossiblyUnknownCondition { .. } => "triet::typecheck::E1033",
        TypeError::TrileanReturnNotRefined { .. } => "triet::typecheck::E1034",
        // Warning code (ADR-0020 §10.3 — promotes to E2002 at v1.0):
        TypeError::NullDeprecated { .. } => "triet::typecheck::W2001",
        TypeError::Concurrency(err) => match err {
            triet_typecheck::ConcurrencyError::NotSendCannotCrossBoundary { .. } => {
                "triet::borrow::E2500"
            }
            triet_typecheck::ConcurrencyError::ScopeRefLeakage { .. } => "triet::borrow::E2510",
            triet_typecheck::ConcurrencyError::MutableShareAntiPattern { .. } => {
                "triet::borrow::E2520"
            }
        },
        TypeError::Borrow(err) => match err {
            triet_typecheck::BorrowError::BorrowLifetimeInferenceFailed { .. } => {
                "triet::borrow::E2400"
            }
            triet_typecheck::BorrowError::BorrowInStructField { .. } => "triet::borrow::E2402",
            triet_typecheck::BorrowError::EscapingBorrow { .. } => "triet::borrow::E2403",
            triet_typecheck::BorrowError::CannotMutateFrozenOwner { .. } => "triet::borrow::E2410",
            triet_typecheck::BorrowError::CannotPromoteFrozenToMutable { .. } => {
                "triet::borrow::E2411"
            }
            triet_typecheck::BorrowError::UseAfterMove { .. } => "triet::borrow::E2420",
            triet_typecheck::BorrowError::SelfOwnershipParadox { .. } => "triet::borrow::E2421",
            triet_typecheck::BorrowError::NonTerminatingConstruction { .. } => {
                "triet::borrow::E2422"
            }
            triet_typecheck::BorrowError::NamespaceInferenceFailed { .. } => "triet::borrow::E2430",
            triet_typecheck::BorrowError::BorrowExclusivityViolation { .. } => {
                "triet::borrow::E2440"
            }
        },
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

#[allow(clippy::too_many_lines)]
fn build_program(path: &str, output: Option<String>, json: bool) -> ExitCode {
    let display_path = path;

    // v0.7.10: discover dao.package by walking up from the source file's
    // parent directory. Mirrors `cargo` convention per ADR-0019 §8.
    let source_path = Path::new(path);
    let manifest_path = source_path.parent().and_then(PackageManifest::discover);

    let empty_manifest = PackageManifest::new("unknown", triet_pack::SemVer::new(0, 0, 0));
    let manifest = match manifest_path {
        Some(p) => match PackageManifest::load(&p) {
            Ok(m) => Some(m),
            Err(triet_pack::StoreError::PackageManifest(e)) => {
                if json {
                    let mut emitter = JsonEmitter::new();
                    emitter.emit(
                        &e.to_string(),
                        package_manifest_error_code(&e),
                        &(0..0),
                        display_path,
                    );
                    emitter.finish();
                } else {
                    eprintln!("{}: {}", package_manifest_error_code(&e), e);
                }
                return ExitCode::from(5);
            }
            Err(e) => {
                if !json {
                    eprintln!("triet::capability::E2208: {e}");
                }
                return ExitCode::from(5);
            }
        },
        None => None,
    };

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

    let type_diagnostics = triet_typecheck::check_resolved(&resolved);
    if !type_diagnostics.is_empty() {
        use miette::Diagnostic;
        let has_hard_errors = type_diagnostics
            .iter()
            .any(|err| err.severity() != Some(miette::Severity::Warning));
        if json {
            let mut emitter = JsonEmitter::new();
            for error in &type_diagnostics {
                emitter.emit(
                    &error.to_string(),
                    &type_error_code(error),
                    &error.span(),
                    display_path,
                );
            }
            emitter.finish();
        } else {
            for error in &type_diagnostics {
                let report = Report::new(error.clone());
                eprintln!("{report:?}");
            }
        }
        if has_hard_errors {
            return ExitCode::from(3);
        }
    }

    let m = manifest.as_ref().unwrap_or(&empty_manifest);
    if !run_capability_check(&resolved, m, display_path, json) {
        return ExitCode::from(5);
    }

    let ir = triet_ir::lower_program(&resolved);
    let code_section = triet_ir::write_program(&ir);

    // v0.7.11.5: build AbiMetadata and emit `.khi` (not raw `.triv`).
    // Caps section is populated from `dao.package`'s `requires` claims
    // when a manifest is found; absent = empty caps (leaf library).
    let meta = manifest.as_ref().map_or_else(
        || AbiMetadata::empty(path, SemVer::new(0, 0, 0)),
        |m| {
            let mut meta = AbiMetadata::empty(&m.name, m.version);
            meta.caps.clone_from(&m.requires);
            meta
        },
    );

    // v0.7.11.6: verify the manifest's `requires` and the `.khi`'s
    // caps section agree. Divergence indicates writer corruption or
    // a stale binary; emit E2208 and refuse to write.
    if let Some(ref m) = manifest
        && let Some(divergence) = check_cap_divergence(&m.requires, &meta.caps, &meta.pkg_name)
    {
        emit_link_error(&divergence, "triet::capability::E2208", display_path, json);
        return ExitCode::from(5);
    }

    let bytes = write_khi(&meta, &code_section);

    let output_path = output.unwrap_or_else(|| {
        let p = Path::new(path);
        if p.extension().and_then(|e| e.to_str()) == Some("tri") {
            p.with_extension("khi").to_string_lossy().into_owned()
        } else {
            format!("{path}.khi")
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

/// v0.7.10: run capability check against the discovered manifest.
/// Returns `true` if no errors, `false` if errors were emitted.
fn run_capability_check(
    program: &triet_modules::ResolvedProgram,
    manifest: &PackageManifest,
    display_path: &str,
    json: bool,
) -> bool {
    let cap_errors = triet_typecheck::check_capabilities(program, manifest);
    if cap_errors.is_empty() {
        return true;
    }
    if json {
        let mut emitter = JsonEmitter::new();
        for error in &cap_errors {
            emitter.emit(
                &error.to_string(),
                capability_error_code(error),
                &(0..0),
                display_path,
            );
        }
        emitter.finish();
    } else {
        for error in cap_errors {
            let report = miette::Report::new(error);
            eprintln!("{report:?}");
        }
    }
    false
}

/// Emit a cap-link error as JSON or human-readable text.
/// v0.7.11.6 extracted from the divergence check.
fn emit_link_error(err: &triet_pack::CapabilityLinkError, code: &str, path: &str, json: bool) {
    if json {
        let mut emitter = JsonEmitter::new();
        emitter.emit(&err.to_string(), code, &(0..0), path);
        emitter.finish();
    } else {
        // `CapabilityLinkError: miette::Diagnostic` but not Clone,
        // so format the string and render via miette's display.
        eprintln!("{code}: {err}");
    }
}

/// Stable JSON error code for each `CapabilityError` variant. CLAUDE.md
/// keep-in-sync rule: every new variant must extend this match.
const fn capability_error_code(error: &CapabilityError) -> &'static str {
    match error {
        CapabilityError::MissingCapabilityClaim { .. } => "triet::capability::E2200",
        CapabilityError::SelfContradictoryCapability { .. } => "triet::capability::E2201",
    }
}

const fn package_manifest_error_code(error: &triet_pack::PackageManifestError) -> &'static str {
    use triet_pack::PackageManifestError;
    match error {
        PackageManifestError::UnsupportedFormatVersion { .. } => "triet::capability::E2208",
        PackageManifestError::Malformed { .. } => "triet::capability::E2204",
        PackageManifestError::InvalidCapabilityRoot { .. } => "triet::capability::E2206",
        PackageManifestError::UnknownStandardCapability { .. } => "triet::capability::E2209",
    }
}

// ── run bytecode (.triv or .khi) ──────────────────────────────────────

fn run_bytecode(path: &str, args: &[String], json: bool) -> ExitCode {
    let file_bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: cannot read {path}: {e}");
            return ExitCode::from(5);
        }
    };

    // v0.7.11.5: `.khi` files are package containers wrapping `.triv`
    // code + ABI metadata. Extract the embedded code section before
    // feeding to `read_program`. `.triv` files remain unchanged.
    let code_bytes = if Path::new(path).extension().and_then(|e| e.to_str()) == Some("khi") {
        match read_khi(&file_bytes) {
            Ok((_meta, code)) => code,
            Err(e) => {
                if json {
                    let mut emitter = JsonEmitter::new();
                    emitter.emit(&e.to_string(), "triet::pack::E23XX", &(0..0), path);
                    emitter.finish();
                } else {
                    eprintln!("Error: {e}");
                }
                return ExitCode::from(5);
            }
        }
    } else {
        file_bytes
    };

    let ir = match triet_ir::read_program(&code_bytes) {
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

    // v0.8.12: Check if the entry function expects an argument.
    // If it expects 1 argument (Vector<String>), map `args` and pass it in.
    let func = ir
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.id == func_to_run)
        .unwrap();
    let vm_args = if func.params.len() == 1 {
        let argv_values: Vec<triet_ir::RuntimeValue> = args
            .iter()
            .map(|s| triet_ir::RuntimeValue::String(s.clone()))
            .collect();
        vec![triet_ir::RuntimeValue::Vector(argv_values)]
    } else {
        vec![]
    };

    let mut vm = triet_ir::Vm::new(ir);
    match vm.execute(func_to_run, vm_args) {
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

// ── `dao store` subcommands ───────────────────────────────────────

/// Top-level handler for `dao store <subcommand>`.
fn store_command(cmd: StoreCommand, json: bool) -> ExitCode {
    let root = match resolve_store_root() {
        Ok(p) => p,
        Err(msg) => {
            if json {
                let mut emitter = JsonEmitter::new();
                emitter.emit(&msg, "triet::pack::E2360", &(0..0), "");
                emitter.finish();
            } else {
                eprintln!("Error: {msg}");
            }
            return ExitCode::from(5);
        }
    };

    let store = match triet_pack::Store::open(&root) {
        Ok(s) => s,
        Err(e) => {
            return emit_store_error(&e, &root.display().to_string(), json);
        }
    };

    match cmd {
        StoreCommand::Import { path } => store_import(&store, &path, json),
        StoreCommand::List { full } => store_list(&store, full, json),
        StoreCommand::Gc => store_gc(&store, json),
    }
}

/// Resolve the store root from `$TRIET_STORE` or `$HOME/.triet/store`.
/// Returned path is not created here — `Store::open` handles `mkdir`.
fn resolve_store_root() -> Result<std::path::PathBuf, String> {
    if let Ok(env_path) = std::env::var("TRIET_STORE")
        && !env_path.is_empty()
    {
        return Ok(std::path::PathBuf::from(env_path));
    }
    let home = std::env::var("HOME")
        .map_err(|_| "HOME env var not set — set TRIET_STORE explicitly".to_owned())?;
    Ok(std::path::PathBuf::from(home).join(".triet").join("store"))
}

fn store_import(store: &triet_pack::Store, path: &str, json: bool) -> ExitCode {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            if json {
                let mut emitter = JsonEmitter::new();
                let msg = format!("can't read {path}: {e}");
                emitter.emit(&msg, "triet::pack::E2360", &(0..0), path);
                emitter.finish();
            } else {
                eprintln!("Error reading {path}: {e}");
            }
            return ExitCode::from(5);
        }
    };
    match store.install_pack(&bytes) {
        Ok(hash) => {
            if !json {
                let hex = hash.0.iter().take(6).fold(String::new(), |mut s, b| {
                    use std::fmt::Write;
                    let _ = write!(&mut s, "{b:02x}");
                    s
                });
                println!("Installed {path} → pkg/{hex}…");
            }
            ExitCode::SUCCESS
        }
        Err(e) => emit_store_error(&e, path, json),
    }
}

fn store_list(store: &triet_pack::Store, full: bool, json: bool) -> ExitCode {
    // Walk names/ to enumerate (pkg_name, version, impl_hash) triples.
    let names_dir = store.root().join("names");
    let entries = match std::fs::read_dir(&names_dir) {
        Ok(it) => it,
        Err(e) => {
            if json {
                let mut emitter = JsonEmitter::new();
                let msg = format!("can't read names/: {e}");
                emitter.emit(
                    &msg,
                    "triet::pack::E2360",
                    &(0..0),
                    &names_dir.display().to_string(),
                );
                emitter.finish();
            } else {
                eprintln!("Error: can't read {}: {e}", names_dir.display());
            }
            return ExitCode::from(5);
        }
    };

    let mut rows: Vec<(String, triet_pack::SemVer, triet_pack::ImplHash)> = Vec::new();
    for entry in entries.flatten() {
        let pkg_path = entry.path();
        if !pkg_path.is_dir() {
            continue;
        }
        let Some(pkg_name) = pkg_path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let pkg_name = pkg_name.to_owned();
        match store.list_versions(&pkg_name) {
            Ok(versions) => {
                for (ver, hash) in versions {
                    rows.push((pkg_name.clone(), ver, hash));
                }
            }
            Err(e) => return emit_store_error(&e, &pkg_path.display().to_string(), json),
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(cmp_semver(&a.1, &b.1)));

    if json {
        // Minimal JSON: one object per row, line-separated.
        for (name, ver, hash) in &rows {
            let hex = full_hex(&hash.0);
            println!(
                "{{\"pkg\":\"{name}\",\"version\":\"{}.{}.{}\",\"impl_hash\":\"{hex}\"}}",
                ver.major, ver.minor, ver.patch
            );
        }
        return ExitCode::SUCCESS;
    }

    if rows.is_empty() {
        println!("(store is empty)");
        return ExitCode::SUCCESS;
    }

    let pkg_w = rows
        .iter()
        .map(|(n, _, _)| n.len())
        .max()
        .unwrap_or(7)
        .max(7);
    println!(
        "{:<pkg_w$}  {:<8}  impl_hash",
        "pkg",
        "version",
        pkg_w = pkg_w
    );
    for (name, ver, hash) in &rows {
        let hex = if full {
            full_hex(&hash.0)
        } else {
            short_hex(&hash.0)
        };
        let ver_str = format!("{}.{}.{}", ver.major, ver.minor, ver.patch);
        println!("{name:<pkg_w$}  {ver_str:<8}  {hex}");
    }
    ExitCode::SUCCESS
}

fn store_gc(store: &triet_pack::Store, json: bool) -> ExitCode {
    match store.gc() {
        Ok(report) => {
            if json {
                println!(
                    "{{\"swept_pkgs\":{},\"swept_modules\":{},\"swept_terms\":{},\"swept_name_links\":{}}}",
                    report.swept_pkgs,
                    report.swept_modules,
                    report.swept_terms,
                    report.swept_name_links,
                );
            } else {
                println!("Garbage-collected:");
                println!("  {} pkg dirs", report.swept_pkgs);
                println!("  {} module dirs", report.swept_modules);
                println!("  {} term dirs", report.swept_terms);
                println!("  {} dangling name links", report.swept_name_links);
            }
            ExitCode::SUCCESS
        }
        Err(e) => emit_store_error(&e, &store.root().display().to_string(), json),
    }
}

fn emit_store_error(e: &triet_pack::StoreError, path: &str, json: bool) -> ExitCode {
    if json {
        let mut emitter = JsonEmitter::new();
        let code = match e {
            triet_pack::StoreError::Io { .. } => "triet::pack::E2360",
            triet_pack::StoreError::Pack(_) => "triet::pack::E2302",
            triet_pack::StoreError::Lockfile(_) => "triet::pack::E2371",
            triet_pack::StoreError::PackageManifest(_) => "triet::capability::E2208",
            triet_pack::StoreError::Policy(_) => "triet::capability::E2205",
        };
        emitter.emit(&e.to_string(), code, &(0..0), path);
        emitter.finish();
    } else {
        eprintln!("Error: {e}");
    }
    ExitCode::from(5)
}

fn cmp_semver(a: &triet_pack::SemVer, b: &triet_pack::SemVer) -> std::cmp::Ordering {
    a.major
        .cmp(&b.major)
        .then(a.minor.cmp(&b.minor))
        .then(a.patch.cmp(&b.patch))
}

fn full_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

fn short_hex(bytes: &[u8]) -> String {
    let mut s = full_hex(&bytes[..6]);
    s.push('…');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v0.7.10 keep-in-sync gate (CLAUDE.md): every `CapabilityError`
    /// variant must map to its specific E22XX code in the JSON emitter,
    /// not a placeholder. If a new variant is added, this test forces
    /// the mapper to extend.
    #[test]
    fn capability_error_code_maps_variants_to_stable_codes() {
        let missing = CapabilityError::MissingCapabilityClaim {
            requester_pkg: "myapp".into(),
            cap_path: "sys.io".into(),
            span: 0..0,
        };
        assert_eq!(capability_error_code(&missing), "triet::capability::E2200");

        let contradictory = CapabilityError::SelfContradictoryCapability {
            requester_pkg: "myapp".into(),
            cap_path: "sys.fs".into(),
            span: 0..0,
        };
        assert_eq!(
            capability_error_code(&contradictory),
            "triet::capability::E2201"
        );
    }
}
