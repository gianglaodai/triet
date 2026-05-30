//! v0.10.x.thread.3 — Multi-worker `Atomic<Integer>` cross-thread share
//! per [ADR-0028] §5 + [ADR-0026 v2] §3.2 + [ADR-0031] §10.2.
//!
//! Builds on v0.10.x.thread.2 infrastructure (`RuntimeValue::Atomic` as
//! `Arc<Mutex<…>>` — `Send + Sync`). Spawns 3 OS threads via
//! `std::thread::spawn`; each thread runs its own `Vm` instance over a
//! tiny IR program that calls `sys.atomic.fetch_add(&+ counter, 1, …)`
//! exactly once, sharing the same `Arc<Mutex<Atomic>>` cell.
//! After all 3 threads join, the main-thread `Vm` loads the counter and
//! asserts the final value is exactly 3.
//!
//! **Honest scope (cliff per `feedback_cham_ma_chac_pattern`):**
//!
//! - The Triết-source `atomic_counter.tri` demo stays at single-worker
//!   shape per [ADR-0031] §10.2 — multi-worker pattern via Triết
//!   `spawn(closure)` requires Send-bound closure types that defer to
//!   v0.11+ (no syntactic site for Send-boundary codegen exists yet).
//! - This test exercises the **runtime layer** (Arc<Mutex> share +
//!   OS-thread cross + VM dispatch of `fetch_add`) without depending
//!   on language-surface features that defer.
//! - Cross-thread synchronization correctness comes from `Arc<Mutex>`,
//!   not from `Ordering` (v0.10 VM dev tier is no-op semantically per
//!   [ADR-0028] §9). Real `Ordering` enforcement lands with v2.0 LLVM
//!   AOT.
//!
//! [ADR-0028]: ../../../../docs/decisions/0028-atomic-primitive.md
//! [ADR-0026 v2]: ../../../../docs/decisions/0026-actor-boundary-send-rules.md
//! [ADR-0031]: ../../../../docs/decisions/0031-borrow-expression-syntax.md

use triet_ir::{
    BasicBlock, BlockId, BuiltinName, Constant, ConstantPool, FuncId, Function, Instruction,
    IrModule, IrProgram, Operand, RuntimeValue, TypeTag, ValueId, Vm,
};

/// Build a minimal IR program with one function that takes an
/// `Atomic<Integer>` + an Integer placeholder for `Ordering` (matches
/// the canonical signature `fetch_add(&+ Atomic<Integer>, delta,
/// ordering) -> Integer`; v0.10 VM ignores ordering at runtime per
/// ADR-0028 §9).
///
/// The function calls `sys.atomic.fetch_add(counter, 1, ord)` exactly
/// once and returns the previous value. Sharing the same `Atomic`
/// `Arc<Mutex>` across 3 separately-constructed `Vm` instances + one
/// invocation each = 3 `fetch_add` ops, sequenced by the mutex.
fn build_fetch_add_worker_program() -> IrProgram {
    let mut constants = ConstantPool::new();
    // Pool slot 0: integer 1 (the delta we're adding).
    let one_idx = constants.intern(Constant::Integer(
        triet_core::Integer::new(1).expect("delta literal in range"),
    ));
    // Pool slot 1: integer 0 (placeholder for Ordering — VM doesn't
    // inspect it on the no-op dev-tier path per ADR-0028 §9).
    let ord_idx = constants.intern(Constant::Integer(
        triet_core::Integer::new(0).expect("ordering placeholder in range"),
    ));

    // ValueId(0): the counter parameter.
    let counter = ValueId(0);
    // ValueId(1): the constant 1 (delta).
    let delta = ValueId(1);
    // ValueId(2): the constant 0 (placeholder ordering).
    let ord = ValueId(2);
    // ValueId(3): the fetch_add return value (previous).
    let prev = ValueId(3);

    let func = Function {
        id: FuncId(0),
        name: Some("fetch_add_worker".to_owned()),
        params: vec![(
            "counter".to_owned(),
            TypeTag::Atomic(Box::new(TypeTag::Integer)),
        )],
        return_type: TypeTag::Integer,
        blocks: vec![BasicBlock {
            id: BlockId(0),
            name: Some("entry".to_owned()),
            instructions: vec![
                Instruction::Const {
                    dest: delta,
                    constant: one_idx,
                },
                Instruction::Const {
                    dest: ord,
                    constant: ord_idx,
                },
                Instruction::CallBuiltin {
                    dest: Some(prev),
                    name: BuiltinName::AtomicFetchAdd,
                    args: vec![
                        Operand::Value(counter),
                        Operand::Value(delta),
                        Operand::Value(ord),
                    ],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(prev)),
                },
            ],
        }],
    };

    IrProgram {
        modules: vec![IrModule {
            path: triet_modules::AbsolutePath::new(
                triet_modules::ModulePath::new(vec!["test".to_owned()]),
                String::new(),
            ),
            functions: vec![func],
        }],
        constants,
        witness_tables: Vec::new(),
    }
}

/// Build a minimal `load(counter, ordering) -> Integer` program — used
/// from the main thread to read the final counter value through normal
/// VM dispatch (rather than peeking the Arc directly), matching the
/// production read path.
fn build_load_program() -> IrProgram {
    let mut constants = ConstantPool::new();
    let ord_idx = constants.intern(Constant::Integer(
        triet_core::Integer::new(0).expect("ordering placeholder in range"),
    ));

    let counter = ValueId(0);
    let ord = ValueId(1);
    let result = ValueId(2);

    let func = Function {
        id: FuncId(0),
        name: Some("load_counter".to_owned()),
        params: vec![(
            "counter".to_owned(),
            TypeTag::Atomic(Box::new(TypeTag::Integer)),
        )],
        return_type: TypeTag::Integer,
        blocks: vec![BasicBlock {
            id: BlockId(0),
            name: Some("entry".to_owned()),
            instructions: vec![
                Instruction::Const {
                    dest: ord,
                    constant: ord_idx,
                },
                Instruction::CallBuiltin {
                    dest: Some(result),
                    name: BuiltinName::AtomicLoad,
                    args: vec![Operand::Value(counter), Operand::Value(ord)],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(result)),
                },
            ],
        }],
    };

    IrProgram {
        modules: vec![IrModule {
            path: triet_modules::AbsolutePath::new(
                triet_modules::ModulePath::new(vec!["test".to_owned()]),
                String::new(),
            ),
            functions: vec![func],
        }],
        constants,
        witness_tables: Vec::new(),
    }
}

#[test]
fn three_workers_share_atomic_counter_via_arc_mutex() {
    // Step 1: build an Atomic<Integer> = 0 via a tiny VM invocation
    // (proves the standard builtin dispatch produces the same Arc-
    // backed value the worker threads will see).
    let new_prog = {
        let mut constants = ConstantPool::new();
        let zero_idx = constants.intern(Constant::Integer(
            triet_core::Integer::new(0).expect("zero literal in range"),
        ));
        let init = ValueId(0);
        let atomic = ValueId(1);
        let func = Function {
            id: FuncId(0),
            name: Some("new_atomic".to_owned()),
            params: vec![],
            return_type: TypeTag::Atomic(Box::new(TypeTag::Integer)),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    Instruction::Const {
                        dest: init,
                        constant: zero_idx,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(atomic),
                        name: BuiltinName::AtomicNew,
                        args: vec![Operand::Value(init)],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(atomic)),
                    },
                ],
            }],
        };
        IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::new(vec!["test".to_owned()]),
                    String::new(),
                ),
                functions: vec![func],
            }],
            constants,
            witness_tables: Vec::new(),
        }
    };
    let mut new_vm = Vm::new(new_prog);
    let counter = new_vm
        .execute(FuncId(0), Vec::new())
        .expect("AtomicNew dispatches cleanly");

    // Step 2: spawn 3 OS threads, each running its own Vm + program
    // that calls fetch_add on the SHARED Atomic. The RuntimeValue::
    // Atomic variant holds Arc<Mutex<…>>; clone propagates the Arc.
    let mut join_handles = Vec::new();
    for worker_id in 0..3_usize {
        let worker_counter = counter.clone();
        join_handles.push(std::thread::spawn(move || -> i64 {
            let prog = build_fetch_add_worker_program();
            let mut vm = Vm::new(prog);
            let result = vm
                .execute(FuncId(0), vec![worker_counter])
                .unwrap_or_else(|err| panic!("worker {worker_id} VM error: {err:?}"));
            match result {
                RuntimeValue::Integer(i) => i.to_i64(),
                other => panic!("worker {worker_id} expected Integer, got {other:?}"),
            }
        }));
    }

    // Step 3: collect previous values returned by each worker. The
    // mutex serializes the fetch_add ops, so the three prevs are a
    // permutation of {0, 1, 2}.
    let mut prevs: Vec<i64> = Vec::new();
    for h in join_handles {
        prevs.push(h.join().expect("worker joined cleanly"));
    }
    prevs.sort_unstable();
    assert_eq!(
        prevs,
        vec![0, 1, 2],
        "expected permutation of {{0, 1, 2}} from 3 sequential fetch_add ops",
    );

    // Step 4: main thread loads the final counter value through the
    // same VM dispatch path — proves the Arc clone we passed to workers
    // refers to the SAME underlying cell.
    let mut load_vm = Vm::new(build_load_program());
    let final_value = load_vm
        .execute(FuncId(0), vec![counter])
        .expect("AtomicLoad dispatches cleanly");
    match final_value {
        RuntimeValue::Integer(i) => assert_eq!(
            i.to_i64(),
            3,
            "expected final counter == 3 after 3 fetch_add(+1) ops",
        ),
        other => panic!("expected Integer, got {other:?}"),
    }
}
