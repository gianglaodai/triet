//! v0.11.x (Hướng A) — JIT-coverage measurement for the self-host
//! compiler.
//!
//! Lowers `compiler/main.tri` to IR and runs a **dry JIT** (attempt to
//! translate every function, no execution, no finalize) to report which
//! functions tier down to the VM and **why**. This bounds the
//! JIT-coverage work needed to make the bootstrap fully JIT-able so the
//! `stage2_eq_stage3_main_tri_byte_identical` gate can be lifted at a
//! CI-acceptable wall time (ROADMAP v0.11 / ADR-0033 §9.5 chain).
//!
//! It does NOT run the compiler (no 30-minute VM bootstrap) — only
//! lowering + opcode translation, a few seconds.
//!
//! `#[ignore]` because it is a **measurement**, not a pass/fail gate.
//! Run on demand:
//!
//! ```text
//! cargo test -p triet-bootstrap --test jit_tier_down_audit \
//!     -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use miette::Diagnostic;
use triet_ir::lower_program;
use triet_jit::audit_jit_coverage;
use triet_modules::load_program;
use triet_typecheck::check_resolved;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("canonicalize workspace root")
}

/// Bucket a tier-down reason into a coarse coverage-gap category. Keyed
/// off stable substrings of the `UnsupportedOpcode` messages emitted by
/// `triet-jit`'s codegen.
fn category(reason: &str) -> &'static str {
    // Order matters: most specific first.
    if reason.contains("translator PANIC") {
        "translator PANIC (Cranelift assertion — bug, must become clean tier-down)"
    } else if reason.contains("field_get") || reason.contains("struct_new") {
        "struct ops (struct_new / field_get)"
    } else if reason.contains("enum_tag") || reason.contains("enum_new") {
        "enum ops (enum_new / enum_tag)"
    } else if reason.contains("outcome_discriminant") || reason.contains("outcome") {
        "Outcome ops (outcome_discriminant / wrap)"
    } else if reason.contains("null_unwrap") || reason.contains("null_wrap") {
        "Nullable ops (null_wrap / null_unwrap)"
    } else if reason.contains("Constant variant String") {
        "String constant"
    } else if reason.contains("Constant variant Null") {
        "Null constant"
    } else if reason.contains("Constant variant Long") || reason.contains("unsupported signature") {
        "Long (i128) constant / signature"
    } else if reason.contains("multi-block function") {
        "shim call inside multi-block fn (if/match/loop)"
    } else if reason.contains("no shim implemented") {
        "builtin has no JIT shim (incl. varargs f-string / concat)"
    } else if reason.contains("arity") {
        "builtin shim arity mismatch"
    } else if reason.contains("referenced before def") {
        "SSA value referenced before def"
    } else if reason.contains("CallCrossModule") || reason.contains("WitnessCall") {
        "unresolved cross-module / witness path"
    } else {
        "other (unhandled opcode)"
    }
}

/// Extract the IR opcode mnemonic from an "unsupported IR opcode … = <op> …"
/// reason, for an opcode-level breakdown of the "other" bucket.
fn opcode_mnemonic(reason: &str) -> Option<&str> {
    let body = reason.strip_prefix("unsupported IR opcode for JIT backend: ")?;
    // Forms: "%2 = enum_tag %0"  |  "ret …"  |  "struct_new […]"
    let after_eq = body.split_once(" = ").map_or(body, |(_, rhs)| rhs);
    after_eq.split([' ', ',']).next()
}

/// Extract the builtin name from a `CallBuiltin(<name>) …` reason, for
/// the per-builtin breakdown.
fn builtin_name(reason: &str) -> Option<&str> {
    let start = reason.find("CallBuiltin(")? + "CallBuiltin(".len();
    let rest = &reason[start..];
    let end = rest.find(')')?;
    Some(&rest[..end])
}

#[test]
#[ignore = "measurement, not a gate: run with --ignored --nocapture to see the JIT-coverage report"]
fn jit_tier_down_audit_compiler_main() {
    let path = workspace_root().join("compiler").join("main.tri");
    assert!(path.is_file(), "missing main.tri at {}", path.display());

    let resolved = load_program(&path).expect("load_program(main.tri)");
    let blocking: Vec<_> = check_resolved(&resolved)
        .into_iter()
        .filter(|e| e.severity() != Some(miette::Severity::Warning))
        .collect();
    assert!(
        blocking.is_empty(),
        "type errors in main.tri: {blocking:#?}"
    );
    let program = lower_program(&resolved);

    // The audit catches per-function translator panics internally; mute
    // the default panic hook so its stderr spam doesn't drown the report.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let report = audit_jit_coverage(&program);
    std::panic::set_hook(prev_hook);
    let total = report.total;
    let jit_able = report.jit_able();
    let tier_downs = report.tier_downs.len();
    #[allow(clippy::cast_precision_loss)]
    let pct = |n: usize| {
        if total == 0 {
            0.0
        } else {
            100.0 * n as f64 / total as f64
        }
    };

    // ── Category breakdown ──
    let mut by_category: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut by_builtin: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_opcode: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_reason: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &report.tier_downs {
        *by_category.entry(category(&entry.reason)).or_default() += 1;
        if let Some(b) = builtin_name(&entry.reason) {
            *by_builtin.entry(b.to_string()).or_default() += 1;
        }
        if let Some(op) = opcode_mnemonic(&entry.reason) {
            *by_opcode.entry(op.to_string()).or_default() += 1;
        }
        *by_reason.entry(entry.reason.clone()).or_default() += 1;
    }

    eprintln!("\n=== JIT-coverage audit — compiler/main.tri (Hướng A) ===");
    eprintln!("total functions      : {total}");
    eprintln!("JIT-able             : {jit_able}  ({:.1}%)", pct(jit_able));
    eprintln!(
        "tier down to VM      : {tier_downs}  ({:.1}%)",
        pct(tier_downs)
    );

    eprintln!("\n--- tier-downs by category ---");
    let mut cats: Vec<_> = by_category.iter().collect();
    cats.sort_by(|a, b| b.1.cmp(a.1));
    for (cat, count) in cats {
        eprintln!("  {count:>5}  {cat}");
    }

    eprintln!("\n--- tier-downs by first-unsupported IR opcode ---");
    let mut ops: Vec<_> = by_opcode.iter().collect();
    ops.sort_by(|a, b| b.1.cmp(a.1));
    for (op, count) in ops {
        eprintln!("  {count:>5}  {op}");
    }

    if !by_builtin.is_empty() {
        eprintln!("\n--- tier-downs by builtin (CallBuiltin sites) ---");
        let mut bs: Vec<_> = by_builtin.iter().collect();
        bs.sort_by(|a, b| b.1.cmp(a.1));
        for (b, count) in bs {
            eprintln!("  {count:>5}  {b}");
        }
    }

    eprintln!("\n--- top 25 distinct raw reasons ---");
    let mut rs: Vec<_> = by_reason.iter().collect();
    rs.sort_by(|a, b| b.1.cmp(a.1));
    for (reason, count) in rs.into_iter().take(25) {
        eprintln!("  {count:>5}  {reason}");
    }

    // jit.4.agg.0 — list every translator PANIC by function, so the
    // fix targets the exact IR shapes (not a guess).
    eprintln!("\n--- translator PANICs (func_id, name → reason) ---");
    for entry in report
        .tier_downs
        .iter()
        .filter(|e| e.reason.contains("translator PANIC"))
    {
        eprintln!("  f{}  {:?}  {}", entry.func_id, entry.name, entry.reason);
    }

    eprintln!("=== end audit ===\n");

    // Always passes — it is a measurement. Sanity only:
    assert!(total > 0, "compiler/main.tri lowered to zero functions");
}
