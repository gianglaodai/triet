//! Triết CLI — entry point cho binary `triet`.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "triet", version, about = "Triết — AI-first balanced-ternary language")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a Triết program (.tt file)
    Run {
        /// Path to .tt source file
        path: String,
    },
    /// Type-check a Triết program without running it
    Check {
        /// Path to .tt source file
        path: String,
    },
    /// Print version and build info
    Info,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Run { path } => {
            eprintln!("triet run {path}: not yet implemented (v0.1 in progress)");
            std::process::exit(1);
        }
        Command::Check { path } => {
            eprintln!("triet check {path}: not yet implemented (v0.1 in progress)");
            std::process::exit(1);
        }
        Command::Info => {
            println!("Triết v{}", env!("CARGO_PKG_VERSION"));
            println!("Balanced ternary, AI-first programming language");
            println!("Spec: SPEC.md");
        }
    }
}
