//! Benchmark: bytecode VM vs tree-walking interpreter.
//!
//! Gate (per ROADMAP § v0.3): VM must be ≥3× faster than the interpreter
//! across the 11 example programs.
//!
//! Only execution time is measured; load/typecheck/lower are done once
//! during setup and excluded from the timing.

// `criterion::bench_function` returns a `&mut BenchmarkGroup` whose Drop runs
// the actual measurement and report writing — that is the intended pattern,
// not a leaked temporary. `criterion_group!` and `criterion_main!` expand into
// macro-generated functions that we cannot add doc comments to.
#![allow(clippy::significant_drop_tightening, missing_docs)]

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use std::path::Path;
use triet_ir::Vm;
use triet_modules::ResolvedProgram;

/// Pre-load and typecheck a program (not measured).
fn setup(path: &Path) -> ResolvedProgram {
    let resolved = triet_modules::load_program(path).expect("load");
    let errors = triet_typecheck::check_resolved(&resolved);
    assert!(errors.is_empty(), "type errors: {errors:?}");
    resolved
}

/// Measure only interpreter execution.
fn bench_interp(resolved: &ResolvedProgram) {
    black_box(triet_interpreter::run_resolved(resolved).expect("interpret"));
}

/// Measure only VM execution (lowering is done once during setup).
fn bench_vm_only(ir: &triet_ir::IrProgram, entry: triet_ir::FuncId) {
    let mut vm = Vm::new(ir.clone());
    black_box(vm.execute(entry, vec![]).expect("vm execute"));
}

macro_rules! bench_example {
    ($c:expr, $name:expr) => {
        let workspace = std::env::current_dir()
            .unwrap()
            .to_str()
            .unwrap()
            .trim_end_matches("crates/triet-cli")
            .to_string();
        let path = std::path::Path::new(&workspace).join($name);
        let resolved = setup(&path);
        // Lower once (not measured).
        let ir = triet_ir::lower_program(&resolved);
        let entry = ir
            .modules
            .iter()
            .flat_map(|m| m.functions.iter())
            .find(|f| f.name.as_deref() == Some("main"))
            .map(|f| f.id)
            .or_else(|| {
                ir.modules
                    .iter()
                    .flat_map(|m| m.functions.iter())
                    .map(|f| f.id)
                    .next()
            })
            .expect("no entry function");
        let mut group = $c.benchmark_group($name);
        group.bench_function("interpreter", |b| b.iter(|| bench_interp(&resolved)));
        group.bench_function("vm", |b| b.iter(|| bench_vm_only(&ir, entry)));
        group.finish();
    };
}

fn benchmarks(c: &mut Criterion) {
    bench_example!(c, "examples/factorial.tri");
    bench_example!(c, "examples/fizzbuzz.tri");
    bench_example!(c, "examples/lukasiewicz_vs_kleene.tri");
    bench_example!(c, "examples/measles_risk.tri");
    bench_example!(c, "examples/nullable.tri");
    bench_example!(c, "examples/maybe.tri");
    bench_example!(c, "examples/generic.tri");
    bench_example!(c, "examples/long_arithmetic.tri");
    bench_example!(c, "examples/counter.tri");
    bench_example!(c, "examples/enumerate.tri");
    bench_example!(c, "examples/while_polling.tri");
}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
