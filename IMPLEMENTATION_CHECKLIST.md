# IMPLEMENTATION_CHECKLIST.md — task `cross-call.b`

> **READ `HANDOFF_PROTOCOL.md` FIRST.** This checklist is binding and refines it.
> You (DeepSeek, the Implementer) write ONLY implementation code to make the
> Architect-authored test in §5 pass. You may not edit any test (§2 golden rule).
> Trial run of the tiered workflow — follow the protocol literally; if anything is
> ambiguous or hits an escalation trigger, STOP and write `ESCALATION_LOG.md`.

---

## 1. Task id + goal

**`cross-call.b`** — let an **unboxed** caller cross-mode call a **boxed** callee when
the boundary is **all primitive scalars** (Integer / Trilean / Trit / Tryte). This is
the mirror of `cross-call.a` (which did boxed→unboxed). Box each scalar arg (raw→ptr),
call the boxed callee, unbox the scalar result (ptr→raw); drop every box created.

Everything else (any composite / `Unit` / `Long` arg or return) MUST tier down — i.e.
return `Err(JitError::UnsupportedOpcode { .. })` so that function falls back to the VM.

---

## 2. File whitelist — edit ONLY these

- `crates/triet-jit/src/codegen.rs` — `translate_call` + its 3 call sites + (test in §5).
- `crates/triet-jit/src/lib.rs` — paste the §5 test verbatim into the `mod tests` block,
  next to `jit4_crosscall_boxed_to_unboxed_scalar_value_parity`.

Do NOT touch any other file. Do NOT add a new shim, opcode, ABI, or `unsafe` block —
you reuse existing helpers only. (If you think you need a new shim → STOP, escalate.)

---

## 3. Reference pattern — mirror this exactly

`translate_boxed_call` in `codegen.rs` is the **near-identical** sibling (it does the
box/drop/sentinel bookkeeping in the other direction). Read it fully. cross-call.b is
the same shape with box↔unbox swapped. **All helpers already exist** — reuse them, do
not write new ones:

- `ctx.func_sigs.get(&callee)` → `(Vec<TypeTag>, TypeTag)` (params + return).
- `boundary_class(ty)` → `Option<BoundaryClass>`; `BoundaryClass::Scalar { kind, clif }`
  for the 4 scalars, `BoundaryClass::PassThrough` for composites, `None` for Unit/Long.
- `emit_box_scalar(builder, module, raw, kind)` → boxed ptr (fresh Rc, ref 1, owned).
- `emit_unbox_scalar(builder, module, ptr, kind, clif)` → raw scalar `Value`.
- `emit_drop_arc(builder, module, ptr)` → balance a box.
- `emit_shim_sentinel_check(builder, module, fn_state)` → propagate a callee failure.
- `resolve_operand(builder, value_map, ctx, op)` → the unboxed caller's raw `Value`.

---

## 4. Exact implementation steps

**Step A — thread `fn_state` into `translate_call`** (pre-approved internal signature
change; this is NOT an ABI/wire change so §4.4 does not forbid it):

1. Add a parameter `fn_state: &mut FnState` to `fn translate_call(...)`. Put it in the
   same position the other helpers use (after `ctx`), so the call becomes
   `translate_call(builder, module, value_map, ctx, fn_state, dest, callee, args)`.
2. Update the **3 call sites** in `translate_instruction` (the `CallLocal`,
   `CallCrossModule`, `WitnessCall` arms) to pass `fn_state`. `translate_instruction`
   already has `fn_state` in scope — just forward it.
3. Add `#[allow(clippy::too_many_arguments)]` above `translate_call` if clippy asks.

**Step B — replace the tier-down guard with scalar-only cross-mode marshaling.**
Currently `translate_call` starts with:
```rust
if ctx.boxed_funcs.contains(&callee) {
    return Err(JitError::UnsupportedOpcode { /* cross-mode ABI (defer) */ });
}
```
Replace the **body** of that `if` (keep the `if ctx.boxed_funcs.contains(&callee)`
condition) with this logic (translate the pseudocode to Cranelift, mirroring
`translate_boxed_call`):

```
// Cross-mode: unboxed caller -> boxed callee. Scalar-only.
let (params, ret) = ctx.func_sigs.get(&callee)            // else Err (tier down)
let cl_callee = ctx.func_id_map.get(&callee)              // else Err (tier down)
if params.len() != args.len()                             // -> Err (tier down)

// Box each scalar arg into a temp box; a non-scalar boundary tiers down.
let mut temp_boxes = Vec::new()
let mut boxed_args = Vec::new()
for (op, pty) in args.iter().zip(params):
    match boundary_class(pty):
        Some(BoundaryClass::Scalar { kind, .. }) =>
            let raw   = resolve_operand(builder, value_map, ctx, *op)?
            let boxed = emit_box_scalar(builder, module, raw, kind)?
            boxed_args.push(boxed); temp_boxes.push(boxed)
        _ => return Err(UnsupportedOpcode "cross-mode arg type {pty:?} not scalar (unboxed->boxed)")

// The return must be scalar too, else tier down.
let ret_scalar = match boundary_class(ret):
    Some(BoundaryClass::Scalar { kind, clif }) => (kind, clif)
    _ => return Err(UnsupportedOpcode "cross-mode return type {ret:?} not scalar (unboxed->boxed)")

let func_ref   = module.declare_func_in_func(cl_callee, builder.func)
let call_inst  = builder.ins().call(func_ref, &boxed_args)
let result_ptr = builder.inst_results(call_inst).first().copied()   // owned boxed ptr

// Unbox the scalar result into `dest` (read BEFORE dropping the box), then drop it.
if let Some(r) = result_ptr:
    if let Some(dest_id) = dest:
        let raw = emit_unbox_scalar(builder, module, r, ret_scalar.0, ret_scalar.1)?
        value_map.insert(dest_id, raw)
    emit_drop_arc(builder, module, r)?      // we own the result box; done after unbox

// Drop the temp arg boxes (the boxed callee only borrowed them).
for b in temp_boxes: emit_drop_arc(builder, module, b)?

// Propagate a callee-internal failure (mirrors translate_boxed_call's tail).
emit_shim_sentinel_check(builder, module, fn_state)?
return Ok(())
```

The existing same-mode (callee NOT boxed) path below the `if` stays unchanged.

**Drop order matters** (get this exactly right — a wrong order double-frees, the §5 test
will abort): unbox the result BEFORE `emit_drop_arc(r)`; drop temp boxes AFTER the call.

---

## 5. Acceptance test — Architect-authored, paste VERBATIM, DO NOT EDIT

Paste into `crates/triet-jit/src/lib.rs` `mod tests`, right after
`jit4_crosscall_boxed_to_unboxed_scalar_value_parity`:

```rust
    #[test]
    fn jit4_crosscall_b_unboxed_to_boxed_scalar_value_parity() {
        // cross-call.b: an UNBOXED caller cross-mode calls a BOXED callee
        // with all-scalar boundary. Args box (raw->ptr), result unboxes
        // (ptr->raw); temp arg boxes + the result box are dropped (a wrong
        // drop order double-frees → this test aborts during dispatch).
        //   make_and_sum(a, b: Integer) -> Integer  (BOXED: builds a struct)
        //   caller(n: Integer) -> Integer = make_and_sum(n, n)  (UNBOXED)
        let make_and_sum = make_function_at(
            FuncId(0),
            "make_and_sum",
            vec![
                ("a".into(), TypeTag::Integer),
                ("b".into(), TypeTag::Integer),
            ],
            TypeTag::Integer,
            vec![
                Instruction::StructNew {
                    dest: ValueId(2),
                    fields: vec![Operand::Value(ValueId(0)), Operand::Value(ValueId(1))],
                },
                Instruction::FieldGet {
                    dest: ValueId(3),
                    object: Operand::Value(ValueId(2)),
                    field_idx: 0,
                },
                Instruction::FieldGet {
                    dest: ValueId(4),
                    object: Operand::Value(ValueId(2)),
                    field_idx: 1,
                },
                Instruction::Add {
                    dest: ValueId(5),
                    lhs: Operand::Value(ValueId(3)),
                    rhs: Operand::Value(ValueId(4)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(5))),
                },
            ],
        );
        let caller = make_function_at(
            FuncId(1),
            "caller",
            vec![("n".into(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![
                Instruction::CallLocal {
                    dest: Some(ValueId(1)),
                    callee: FuncId(0),
                    args: vec![Operand::Value(ValueId(0)), Operand::Value(ValueId(0))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![make_and_sum, caller])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        // make_and_sum is boxed; caller is unboxed and cross-mode calls it.
        // Both must JIT now (caller no longer tiers down).
        assert!(jit.lookup(FuncId(0)).is_some(), "boxed make_and_sum JITs");
        assert!(
            jit.lookup(FuncId(1)).is_some(),
            "unboxed caller cross-mode calling a boxed callee must JIT"
        );
        // caller is unboxed → raw i64 in/out via dispatch_integer.
        // caller(5) = make_and_sum(5,5) = struct{5,5} -> 5+5 = 10.
        assert_eq!(dispatch_integer(&jit, FuncId(1), &[5]), Some(10));
        // VM parity.
        let mut vm = triet_ir::Vm::new(program);
        let vm_r = vm.execute(FuncId(1), vec![integer(5)]).expect("vm caller");
        assert_rv_eq(&vm_r, &integer(10));
    }
```

---

## 6. Done = ALL of these, with ZERO §4-prohibited actions

Run verbatim (HANDOFF_PROTOCOL §7):
```bash
cargo test -p triet-jit jit4_crosscall_b_unboxed_to_boxed_scalar_value_parity   # passes
cargo test --workspace                                                           # >= 1676 total
cargo clippy --workspace --all-targets -- -D warnings                            # zero warnings
cargo fmt --all
cargo test -p triet-bootstrap --test jit_tier_down_audit -- --ignored --nocapture
#   ^ JIT-able count MUST be >= 1622 (the pre-task baseline; must not regress).
#     A small increase is expected + fine; report the exact number.
```

Report: the test result, the workspace test count, the JIT-able audit number, and confirm
no test was edited and no prohibition was hit. Leave changes uncommitted for author review.

## 7. Escalate (STOP + `ESCALATION_LOG.md`) if

- The §5 test still fails after 3 distinct honest attempts (do NOT hack it green).
- You find you need an `unsafe` block, a new shim, or any change outside the §2 whitelist.
- The drop-order / ownership reasoning is unclear and you cannot make the test pass without
  guessing at memory management.
