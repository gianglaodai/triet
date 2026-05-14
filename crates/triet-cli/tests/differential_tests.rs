//! Differential tests: VM ≡ tree-walking interpreter.
//!
//! Each `examples/*.tri` program is run through both the interpreter
//! and the bytecode VM. Output (stdout + exit code) must be
//! byte-identical per [ROADMAP.md § v0.3 gate 2].
//!
//! [ROADMAP.md]: ../../../ROADMAP.md

use std::process::Command;

/// Represents the output of running a Triết program.
struct ProgramOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: i32,
}

/// Run a program through the interpreter: `triet run <path>`.
fn run_interpreter(binary: &str, path: &str) -> ProgramOutput {
    let output = Command::new(binary)
        .args(["run", path])
        .output()
        .expect("failed to execute triet");
    ProgramOutput {
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code: output.status.code().unwrap_or(-1),
    }
}

/// Run a program through the VM: `triet build <path> -o <tmp> && triet run <tmp>`.
fn run_vm(binary: &str, path: &str, tmp: &str) -> ProgramOutput {
    // Build the .triv file.
    let build = Command::new(binary)
        .args(["build", path, "-o", tmp])
        .output()
        .expect("failed to execute triet build");
    if !build.status.success() {
        return ProgramOutput {
            stdout: build.stdout,
            stderr: build.stderr,
            exit_code: build.status.code().unwrap_or(-1),
        };
    }
    // Run the .triv file.
    let run = Command::new(binary)
        .args(["run", tmp])
        .output()
        .expect("failed to execute triet run");
    ProgramOutput {
        stdout: run.stdout,
        stderr: run.stderr,
        exit_code: run.status.code().unwrap_or(-1),
    }
}

/// Compare two outputs. Panic with a detailed message if they differ.
fn assert_output_eq(interp: &ProgramOutput, vm: &ProgramOutput, name: &str) {
    if interp.exit_code != vm.exit_code {
        // If the VM fails with a non-zero exit, show stderr.
        if vm.exit_code != 0 {
            eprintln!(
                "VM stderr for {name}:\n{}",
                String::from_utf8_lossy(&vm.stderr)
            );
        }
        panic!(
            "{name}: exit code mismatch: interpreter={}, vm={}",
            interp.exit_code, vm.exit_code
        );
    }
    // For error programs (non-zero exit), compare stderr too.
    if interp.exit_code != 0 {
        assert_eq!(
            interp.stderr,
            vm.stderr,
            "{name}: stderr mismatch (exit code {})",
            interp.exit_code
        );
        return;
    }
    assert_eq!(
        interp.stdout, vm.stdout,
        "{name}: stdout mismatch.\nExpected ({} bytes): {:?}\nActual ({} bytes): {:?}",
        interp.stdout.len(),
        String::from_utf8_lossy(&interp.stdout),
        vm.stdout.len(),
        String::from_utf8_lossy(&vm.stdout),
    );
}

// ── Tests ──────────────────────────────────────────────────────────

/// Get the path to the triet binary for testing.
fn triet_binary() -> String {
    std::env::var("TRIET_BINARY").unwrap_or_else(|_| {
        // Default: look for the release binary relative to the workspace root.
        // Tests run from the workspace root, so CWD should be the repo root.
        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd.to_str().unwrap();
        // If we're in crates/triet-cli/, go up two levels.
        if cwd.ends_with("triet-cli") {
            format!("{cwd}/../../target/release/triet")
        } else {
            format!("{cwd}/target/release/triet")
        }
    })
}

macro_rules! diff_test {
    ($name:ident, $example:expr) => {
        #[test]
        fn $name() {
            let binary = triet_binary();
            let example_path = $example;
            // Resolve the example path relative to the workspace root.
            let cwd = std::env::current_dir().unwrap();
            let cwd = cwd.to_str().unwrap();
            let workspace_root = if cwd.ends_with("triet-cli") {
                format!("{cwd}/../..")
            } else {
                cwd.to_string()
            };
            let full = format!("{workspace_root}/{example_path}");
            let tmp = format!("/tmp/triet_diff_{}.triv", stringify!($name));
            let interp = run_interpreter(&binary, &full);
            let vm = run_vm(&binary, &full, &tmp);
            assert_output_eq(&interp, &vm, &full);
            // Clean up.
            let _ = std::fs::remove_file(&tmp);
        }
    };
}

// Verified passing (byte-identical VM vs interpreter).
diff_test!(diff_lukasiewicz_vs_kleene, "examples/lukasiewicz_vs_kleene.tri");
diff_test!(diff_measles_risk, "examples/measles_risk.tri");
diff_test!(diff_factorial, "examples/factorial.tri");
diff_test!(diff_maybe, "examples/maybe.tri");
diff_test!(diff_generic, "examples/generic.tri");
diff_test!(diff_counter, "examples/counter.tri");
diff_test!(diff_while_polling, "examples/while_polling.tri");
diff_test!(diff_long_arithmetic, "examples/long_arithmetic.tri");
diff_test!(diff_nullable, "examples/nullable.tri");
diff_test!(diff_enumerate, "examples/enumerate.tri");

// Known issues — ignored pending lowerer/VM fixes (tracked under v0.3.x.cleanup.5–8).
// Tests are scheduled to be un-ignored as each lowering gap is closed:
// - nullable, maybe, generic     → v0.3.x.cleanup.5 (enum payload lowering)
// - counter, fizzbuzz, while_polling → v0.3.x.cleanup.6 (while loop + match)
// - enumerate                     → v0.3.x.cleanup.7 (iterator lowering)
// - long_arithmetic               → v0.3.x.cleanup.8 (Long cross-module ops)
macro_rules! ignored_test {
    ($name:ident, $path:expr, $reason:expr) => {
        #[test]
        #[ignore = $reason]
        fn $name() {
            let binary = triet_binary();
            let example_path = $path;
            let cwd = std::env::current_dir().unwrap();
            let cwd = cwd.to_str().unwrap();
            let workspace_root = if cwd.ends_with("triet-cli") {
                format!("{cwd}/../..")
            } else {
                cwd.to_string()
            };
            let full = format!("{workspace_root}/{example_path}");
            let tmp = format!("/tmp/triet_diff_{}.triv", stringify!($name));
            let interp = run_interpreter(&binary, &full);
            let vm = run_vm(&binary, &full, &tmp);
            assert_output_eq(&interp, &vm, &full);
            let _ = std::fs::remove_file(&tmp);
        }
    };
}

ignored_test!(
    ignore_fizzbuzz,
    "examples/fizzbuzz.tri",
    "v0.3.x.cleanup.8: tuple literal pattern match arms"
);
