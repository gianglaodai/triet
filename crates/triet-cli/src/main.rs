//! Triết CLI — entry point for the `triet` binary.
//!
//! Subcommands:
//! - `triet run <path.tri>` — parse, type-check, and run the program.
//! - `triet check <path.tri>` — parse + type-check only, no execution.
//! - `triet info` — version and project info.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::{fs, process::ExitCode};

use clap::{Parser, Subcommand};
use triet_interpreter::run;
use triet_parser::parse;
use triet_typecheck::check;

#[derive(Parser)]
#[command(name = "triet", version, about = "Triết — AI-first balanced-ternary language")]
struct Cli {
    #[command(subcommand)]
    command: Command,
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
    match cli.command {
        Command::Run { path } => match run_program(&path) {
            Ok(()) => ExitCode::SUCCESS,
            Err(code) => code,
        },
        Command::Check { path } => match check_program(&path) {
            Ok(()) => ExitCode::SUCCESS,
            Err(code) => code,
        },
        Command::Info => {
            println!("Triết v{}", env!("CARGO_PKG_VERSION"));
            println!("Balanced ternary, AI-first programming language");
            println!("Spec: SPEC.md");
            ExitCode::SUCCESS
        }
    }
}

fn run_program(path: &str) -> Result<(), ExitCode> {
    let source = read_source(path)?;
    let (program, parse_errors) = parse(&source);
    if !parse_errors.is_empty() {
        for error in &parse_errors {
            eprintln!("parse error: {error}");
        }
        return Err(ExitCode::from(2));
    }

    let type_errors = check(&program);
    if !type_errors.is_empty() {
        for error in &type_errors {
            eprintln!("type error: {error}");
        }
        return Err(ExitCode::from(3));
    }

    match run(&program) {
        Ok(_) => Ok(()),
        Err(error) => {
            eprintln!("runtime error: {error}");
            Err(ExitCode::from(4))
        }
    }
}

fn check_program(path: &str) -> Result<(), ExitCode> {
    let source = read_source(path)?;
    let (program, parse_errors) = parse(&source);
    if !parse_errors.is_empty() {
        for error in &parse_errors {
            eprintln!("parse error: {error}");
        }
        return Err(ExitCode::from(2));
    }
    let type_errors = check(&program);
    if !type_errors.is_empty() {
        for error in &type_errors {
            eprintln!("type error: {error}");
        }
        return Err(ExitCode::from(3));
    }
    println!("{path}: OK");
    Ok(())
}

fn read_source(path: &str) -> Result<String, ExitCode> {
    fs::read_to_string(path).map_err(|error| {
        eprintln!("could not read {path}: {error}");
        ExitCode::from(1)
    })
}
