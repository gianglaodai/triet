# ADR 0032 — Builtin shim ABI (`extern "C-unwind"` registry, hybrid RuntimeValue marshaling)

**Trạng thái:** **Locked** (v0.10.0.1, author sign-off pending). Refines [ADR-0030 §12](0030-jit-cranelift-integration.md) — locks the 5 design constraints surfaced by v0.9.x.jit.4 deferral so that v0.10.x.jit.1 (shim infrastructure) and v0.10.x.jit.2 (43 builtin implementations) can ship without re-litigating ABI shape. First ADR opened in the v0.10 cycle; mandatory unblock for jit.1.

> **2026-05-31 Addendum (v0.10.x.jit.1 implementation) — §4 error-propagation cliff + redesign options.**
>
> v0.10.x.jit.1 implementation discovered that **§4's locked mechanism
> (`extern "C-unwind"` shim panic + dispatcher `catch_unwind` across the
> JIT'd frame) does NOT work on `cranelift-jit 0.132`** — the version
> pinned in `triet-jit/Cargo.toml`. The framework smoke test
> `framework_shim_panic_to_vm_error` (§7.1) aborts with `failed to
> initiate panic, error 5` + SIGABRT.
>
> **Root cause:** `cranelift-jit 0.132` does not register **system DWARF
> unwind tables** (`.eh_frame` via `__register_frame`) for JIT-compiled
> functions. (It has a `wasmtime-unwinder` cargo feature, but that wires
> Wasmtime's *custom* exception model, not the system unwinder that
> Rust's `panic!`/`catch_unwind` use.) When a shim panics and the
> unwinder tries to traverse the Cranelift-compiled caller frame, it
> finds no unwind info → fatal abort, never reaching the dispatcher's
> `catch_unwind`. §4 assumed (incorrectly) that `extern "C-unwind"` +
> `catch_unwind` would compose with Cranelift JIT out of the box.
>
> **What jit.1 shipped (option-agnostic, all green):** shim registry
> (`ShimEntry`/`production_shim_entries`/`JITBuilder::symbol` wiring,
> §6), lifetime `__triet_drop_arc` shim (§2), `BuiltinName → namespace`
> capability table + `check_builtin_capability` defense-in-depth (§3),
> hybrid-ABI scalar/signature converters (§1), the `unsafe_code = deny`
> crate-local override (§5), and 3 of 4 framework tests
> (`framework_shim_call_returns_value`,
> `framework_drop_arc_balances_refcount`,
> `framework_capability_denied_tiers_down`).
>
> **What deferred:** the §4 error-propagation mechanism — the
> thread-local error slot + dispatch `catch_unwind` wrapper
> (`dispatch_integer_caught`) + `framework_shim_panic_to_vm_error` test +
> the `VmError::JitShimFault` fallback variant. These were removed from
> jit.1 (not shipped half-working) pending the redesign decision below.
>
> **Three redesign options for the author (decide before v0.10.x.jit.2
> wires the 43 shims, which all need a working error path):**
>
> 1. **Shim-internal catch + boundary TLS check.** Each shim wraps its
>    fallible body in `catch_unwind` internally; on failure it records a
>    `VmError` to a thread-local slot and returns a sentinel — never
>    unwinding out through the JIT frame. The dispatcher reads the slot
>    after the (normal) native return. *Caveat:* a JIT'd function calling
>    multiple shims would keep executing subsequent shims after one
>    fails (the failing shim returned a sentinel, not an abort) — wrong
>    for side-effecting shims (`println`) unless paired with option 2's
>    per-call check. Lowest codegen complexity, weakest semantics.
> 2. **Per-call sentinel check in codegen.** Shims record-to-TLS +
>    return a sentinel; the JIT `CallBuiltin` codegen emits a
>    check-and-early-return after each shim call. Correct abort-on-first-
>    error; this is the path §4 explicitly rejected as "verbose", but it
>    is robust and needs no Cranelift unwind tables. ~per-call branch
>    overhead + codegen complexity.
> 3. **Register Cranelift unwind tables.** Enable `unwind_info` + wire
>    system `__register_frame` (or adopt the `wasmtime-unwinder` feature
>    if it can target the system unwinder) so `catch_unwind` across the
>    JIT frame works — keeping §4 as originally locked. Uncertain
>    feasibility on `cranelift-jit 0.132`; needs research; most faithful
>    to §4 if achievable.
>
> **Recommendation (implementer):** option 2 — robust + Cranelift-
> version-independent, matching the project's "refuse over guess" +
> "stability over speed" stance. The §4 "verbose" objection is
> outweighed by the abort-on-first-error correctness that side-effecting
> shims require. Final choice is the author's; this Addendum records the
> cliff so jit.2 does not build 43 shims on an unvalidated foundation.
>
> **RESOLVED 2026-05-31 (author sign-off): OPTION 2 — per-call sentinel
> check in codegen.** The §4 `extern "C-unwind"` + dispatcher
> `catch_unwind` mechanism is **superseded** for v0.10. The locked
> replacement:
>
> - **Shim ABI:** each fallible shim returns its result PLUS an
>   out-of-band failure signal. A shim records a structured `VmError`
>   into a thread-local slot (the §4 slot is RE-USED — it survives as
>   option-agnostic substrate) and returns a **sentinel** the codegen
>   recognizes. For pointer/composite returns the sentinel is a null
>   `i64 0`; for primitive returns (where 0 is a valid value) the shim
>   instead sets a dedicated thread-local `SHIM_FAILED` boolean flag
>   that the codegen-emitted check reads (chosen over poisoning a
>   primitive value-space).
> - **Codegen:** after every `call $shim`, the JIT emits a check of the
>   `SHIM_FAILED` flag (a `__triet_shim_failed() -> i8` probe shim) and,
>   if set, branches to a per-function `error_exit` block that returns a
>   sentinel up to the dispatcher. The dispatcher reads the TLS
>   `VmError` after the native call returns (NORMALLY — no unwinding
>   through the JIT frame).
> - **Shims declare `extern "C"` (not `"C-unwind"`)** — they never
>   unwind out; on internal panic-class failures they `catch_unwind`
>   INTERNALLY, record the `VmError`, set the flag, and return. The JIT
>   frame is never traversed by the unwinder. This sidesteps the
>   cranelift-jit 0.132 unwind-table gap entirely.
> - **`VmError::JitShimFault`** (the fallback variant reverted in jit.1)
>   is RE-ADDED in jit.2 for the "shim set flag without recording a
>   structured error" defensive case.
>
> jit.2 implements this. The §4 body below (C-unwind + catch_unwind) is
> preserved per immutability but is **non-normative** — this Addendum's
> option-2 resolution is authoritative for v0.10's error-propagation
> mechanism.

> **2026-05-31 Addendum (v0.10.x.jit.2b-iii) — shim coverage + varargs deferral.**
>
> v0.10 ships **36 of the 43** builtin JIT shims (jit.2b-i clean fixed-
> arity ×21, jit.2b-ii Atomic ×10, jit.2b-iii file-I/O ×5). All delegate
> their SEMANTICS to `triet_ir::dispatch_builtin` (the VM's own dispatch)
> — VM↔JIT divergence is impossible by construction. The remaining 7
> tier-down to VM (correct, just not JIT-accelerated):
>
> - **Varargs (2): `FStringConcat`, `TextConcat`.** Each `CallBuiltin`
>   has a concrete arity at the IR level, but it varies per call site,
>   so no fixed-arity `extern "C"` shim fits. §1's hybrid table flagged
>   this "Mixed/unresolved". Supporting it needs a NEW ABI shape — an
>   array-ptr + len (box each arg → stack array → shim reads N
>   `RuntimeValue`s). That is itself an ABI decision; **deferred to
>   v0.11** with its own ADR addendum rather than rushed into the v0.10
>   window. F-strings tier-down to VM meanwhile (often on cold I/O
>   paths anyway).
> - **`RawThreadSpawn` / `RawThreadJoin` (2):** need the VM's
>   thread-handle registry (not a pure `dispatch_builtin` call) — not
>   JIT-able through the shim ABI. Out of scope; tier-down forever
>   (or until a VM-thread-aware JIT path, undecided).
> - **Ordering-enum end-to-end (atomic functions):** the Atomic shims
>   work, but a real `fetch_add(c, 1, Synchronized)` function tier-downs
>   at the `Synchronized` `EnumNew` construction (no `Enum` `TypeTag`,
>   no enum codegen). Gated on a future enum-construction codegen.
>
> File I/O was initially flagged a "cliff" but is NOT — fixed-arity,
> delegates cleanly (the I/O happens in `dispatch_builtin`); only its
> parity tests need tempdir fixtures.

**Issue:** v0.9 ships partial Cranelift JIT (numeric arith / cmp / control flow / intra-program calls). `Instruction::CallBuiltin` tier-downs to VM dispatch per [ADR-0030 §12.3](0030-jit-cranelift-integration.md) — entire function reverts to VM when it touches *any* of the 43 stdlib builtins. Real programs use `println` / `Vector*` / `HashMap*` heavily, so v0.9 JIT acceleration is limited to numeric leaf functions. v0.10 closes the gap.

The 5 constraints ADR-0030 §12.2 surfaced as needing a coherent design pass:

1. **`RuntimeValue` ABI representation** — JIT registers hold raw primitives (`i64`/`i8`); Rust shims need to receive structured `RuntimeValue`. Boxed-by-default vs per-builtin specialization vs hybrid?
2. **Lifetime management** — `Rc::into_raw` leaks the refcount unless a matching `drop_arc` shim runs. How does JIT codegen know where to emit Drop?
3. **Capability gate enforcement** — VM dispatches builtins without per-call capability check (gate is compile-time at IR generation per [ADR-0016 §5](0016-capability-type-system.md)). JIT shim layer needs equivalent — compile-time elision or runtime check?
4. **Panic → `VmError` propagation** — Rust shims panic on `VmError`-class failures (`Assert` fail, `Vector::get` OOB, etc.). JIT-side: Cranelift trap blocks vs `extern "C-unwind"` ABI?
5. **`unsafe_code` policy** — Workspace-wide `forbid`; shim layer requires `#[unsafe(no_mangle)]` + raw pointer casts. Override scope?

Open questions ADR-0032 phải lock cùng:

6. **Shim registry shape** — how do 43 `extern "C"` functions get registered + symbol-resolved at JIT compile time?
7. **Test gates** — minimum infrastructure to ship v0.10.x.jit.1 (framework) and v0.10.x.jit.2 (43 implementations) safely.
8. **Self-host port plan** per [ADR-0029 §5](0029-self-host-port-policy.md) — does `compiler/parser/*.tri` need any same-phase mirror? Layer classification?

---

## §1 — `RuntimeValue` ABI: hybrid (primitives unboxed, composites Rc-boxed)

**Decision:** Triết runtime values cross the JIT↔shim ABI boundary in **hybrid** form. Primitive types pass as their natural Cranelift register type; composite types pass as raw `*const RuntimeValue` (or `*const RefCell<RuntimeValue>` for `Atomic<T>`) where the pointer is the result of `Rc::into_raw`.

| Triết type | Cranelift register | Shim arg/ret type | Marshaling |
|---|---|---|---|
| `Trit` | `i8` | `i8` | direct (POD, `-1`/`0`/`+1` encoding per [ADR-0030 §3](0030-jit-cranelift-integration.md)) |
| `Tryte` | `i16` | `i16` | direct |
| `Integer` | `i64` | `i64` | direct |
| `Long` | `i128` (Cranelift native, x86_64/aarch64) | `i128` | direct |
| `Trilean` | `i8` | `i8` | direct (same encoding as Trit per ADR-0010) |
| `Unit` | `i8` 0 | n/a | shim returning Unit returns `i8 0` placeholder; caller discards |
| `String` | `i64` ptr | `*const RuntimeValue` | `Rc<RuntimeValue::String(…)>` via `Rc::into_raw` |
| `Vector<T>` | `i64` ptr | `*const RuntimeValue` | same boxed pattern |
| `HashMap<K,V>` | `i64` ptr | `*const RuntimeValue` | same boxed pattern |
| `Struct{…}` / `Enum{…}` / `Closure{…}` / `Outcome{…}` | `i64` ptr | `*const RuntimeValue` | same boxed pattern |
| `Atomic<T>` | `i64` ptr | `*const RefCell<RuntimeValue>` | `Rc<RefCell<RuntimeValue>>` via `Rc::into_raw` — direct reuse of v0.9 VM repr ([ADR-0028 §3](0028-atomic-primitive.md)) |
| `T?` nullable (composite payload) | `i64` ptr | `*const RuntimeValue` | boxed; payload includes discriminator |
| `T?` nullable (primitive payload, e.g. `Integer?`) | `i64` ptr | `*const RuntimeValue` | boxed always — uniform discriminator handling beats per-shape specialization at v0.10 scale |

**Why hybrid, not all-boxed nor per-builtin specialized:**

- **All-boxed** (`*const RuntimeValue` for everything including `Integer`) means every arithmetic-adjacent shim allocates on argument prep — `assert_eq(x, y)` with `x: Integer` would box both. Brutal for `Vector_get(vec, i)` where `i: Integer` is 90% of OOB checks. Rejected.
- **Per-builtin specialized** (each of 43 builtins gets its own marshaling stub keyed by exact arg signature) means ~N × M variants in registry — `Println(Integer)` vs `Println(String)` vs `Println(Vector)` vs `Println(Struct)` etc. Combinatorial. Maintenance debt without payoff — the JIT translator already knows arg types from the IR-side `BuiltinName` + `args` ValueId types. Rejected.
- **Hybrid** matches what the JIT codegen already does for non-builtin code — primitives stay in `i8`/`i16`/`i64` registers, composites use heap-boxed `Rc<…>` pointers per [ADR-0028 §3](0028-atomic-primitive.md) precedent. Adds one decision per arg ("is this type a composite?") and zero combinatorial state.

**Implementation note for §3 codegen:** the JIT translator already classifies values via the unified `ValueKind` enum per [ADR-0023](0023-lowerer-ssa-struct-tracking.md) (Struct / Outcome / Nullable / Other). v0.10 extends this with explicit Composite-or-Primitive bit so the codegen-side argument marshaler knows whether to emit `iconst` (primitive pass-through) or `Rc::into_raw → ptr_as_i64` (composite box).

**Long type caveat:** Cranelift supports `i128` on x86_64 + aarch64 (the v0.10 target triples). On platforms where `i128` is unsupported (rare; not a v0.10 target), `Long` falls back to a 2×`i64` ABI pair. Defer formal multi-arch decision to v0.11 cross-compile work.

---

## §2 — Lifetime management: `Rc::into_raw` on box-out, `drop_arc` shim on last-use

**Decision:** Lifetime crossings between JIT registers and Rust shims follow Rust's `Rc::into_raw` / `Rc::from_raw` convention exactly, with two rules:

1. **At ABI boundary IN (JIT register → shim arg):** shim receives a **borrowed** raw pointer. Refcount unchanged. Shim does NOT call `Rc::from_raw` on the borrowed view — it would consume the refcount the caller still owns. To dereference, shim uses `unsafe { &*(ptr as *const RuntimeValue) }` and treats the borrow as living for the call duration only.
2. **At ABI boundary OUT (shim → JIT register):** shim returns a **fresh** owned pointer (refcount = 1) via `Rc::into_raw(Rc::new(…))`. JIT-side register now owns that +1 refcount.

**`drop_arc` shim — explicit Drop at SSA value last-use.** The JIT translator consults `ValueKind` (per [ADR-0023](0023-lowerer-ssa-struct-tracking.md)) at compile time to identify boxed values, and emits a call to a `__triet_drop_arc(ptr: i64)` shim at the SSA value's last use point. The shim does:

```rust
#[unsafe(no_mangle)]
extern "C-unwind" fn __triet_drop_arc(ptr: i64) {
    if ptr == 0 { return; }  // null-safe (T? null arm or sentinel)
    // SAFETY: ptr came from `Rc::into_raw` and is consumed exactly once
    // (lowerer guarantees last-use point per ADR-0023 ValueKind tracking).
    unsafe { let _ = Rc::from_raw(ptr as *const RuntimeValue); }
}
```

**Why explicit Drop at last-use, not implicit refcounting in IR:**

- **IR-level refcount opcodes** (`RetainArc` / `ReleaseArc` per SSA value) would bloat the register-SSA shape with non-semantic operations and violate [ADR-0007](0007-ir-design.md) "register-SSA is minimal". Rejected.
- **Cranelift `cleanup_block` / EH-style auto-Drop** is complex tooling (requires Cranelift's `frontend::Variable` lifetime tracking integration). Premature for v0.10 scope. Defer to v0.11 if leaks materialize.
- **Explicit Drop at last-use** mirrors how the lowerer already handles ownership transitions (per [ADR-0022 §4](0022-trit-balanced-ownership.md) "compiler tự động borrow"). JIT codegen adds one IR-walk pass that inserts `drop_arc` calls — same machinery as the lowerer's existing ValueKind annotations.

**Borrow-pattern rule** for shims accepting boxed args: if a shim needs to retain the value beyond the call (e.g., `Vector_push` stores into the vec, `HashMap_insert` keeps the value as a map entry), the shim **clones the Rc explicitly** before storing:

```rust
extern "C-unwind" fn __triet_vector_push(vec_ptr: i64, val_ptr: i64) -> i64 {
    // SAFETY: both ptrs are borrowed (refcount unchanged on entry)
    let vec_rc: &Rc<RuntimeValue> = unsafe { &*(vec_ptr as *const _) };
    let val_rc: &Rc<RuntimeValue> = unsafe { &*(val_ptr as *const _) };
    let new_vec = match &**vec_rc {
        RuntimeValue::Vector(v) => {
            let mut nv = v.clone();
            nv.push(RuntimeValue::clone(&*val_rc));  // explicit clone
            RuntimeValue::Vector(nv)
        }
        _ => panic!("vector_push: expected Vector"),
    };
    Rc::into_raw(Rc::new(new_vec)) as i64
}
```

**Caveat — Vector functional semantics.** Per existing user memory `triet_vector_functional.md`, `push(buf, x)` is functional (returns new Vector, not in-place mutate). Hybrid ABI preserves this exactly — the shim receives borrowed `vec_ptr`, clones-and-extends, returns new boxed Vector. JIT-side register that previously held `vec_ptr` is consumed (sees `drop_arc` from lowerer's last-use tracking).

---

## §3 — Capability gate: compile-time hoist; JIT refuses-to-emit on denied namespace

**Decision:** Builtin capability namespace checks (per [ADR-0016 §5](0016-capability-type-system.md) `sys.*`/`dev.*`/`usr.*` resolution) happen at **JIT compile time**, not per-call. JIT translator consults the program's frozen `CapabilitySet` snapshot when reaching a `CallBuiltin` opcode; if the builtin's required namespace is denied, JIT emits `JitError::BuiltinCapabilityDenied { builtin, namespace }` and the function tier-downs to VM dispatch (where the same diagnostic fires per existing VM behavior).

**Why compile-time, not per-call:**

- **VM precedent:** `execute_builtin` in `triet-ir::vm` (v0.9 inspection of `crates/triet-ir/src/vm.rs:2152`) does NOT gate per-call. Gate is at IR generation — if the user's `dao.package` doesn't grant `sys.io`, the lowerer refuses to emit a `CallBuiltin(Println)` opcode in the first place. By the time a `CallBuiltin` reaches the JIT, the capability is already granted (else the .triv wouldn't have that opcode). JIT inherits this invariant.
- **Trit-graded resolution** per [ADR-0017](0017-trilean-policy-hook.md) (Granted / Defer / Deny + TTY prompt) is resolved at program LOAD, not at codegen — so by JIT compile time (call ≥100 threshold), the capability has been pinned to Granted (else the program wouldn't have started executing the function in the first place).
- **Defense-in-depth path:** for the pathological case where a `dev.jit_codegen`-style runtime resolution change happens (e.g., capability hot-swap, not currently a feature but reserved per [ADR-0030 Addendum Gap 1](0030-jit-cranelift-integration.md)), JIT translator re-reads the snapshot once per `CallBuiltin` opcode at compile time. Cheap; not per-call.

**Concretely:** the v0.10 `JitCompiler::compile_program` receives `&CapabilitySet` along with the program. The codegen branch for `Instruction::CallBuiltin` looks up `BuiltinName → namespace` (a static table — `Println → sys.io`, `AtomicNew → sys.atomic`, etc.) and consults the snapshot. If denied → `JitError::BuiltinCapabilityDenied` → tier-down. Otherwise → emit `call $shim_symbol`.

**Rejected: per-call runtime check inside each shim.** Adds 43 × per-call overhead for a check whose outcome is already pinned at program-load time. Wasted instruction cache.

---

## §4 — Panic → `VmError` propagation: `extern "C-unwind"` + dispatcher `catch_unwind`

**Decision:** All builtin shims use Rust 2024-stable `extern "C-unwind"` ABI. On `VmError`-class failures (assertion, OOB, parse failure, etc.) the shim **panics with a structured `VmError` payload** via thread-local `CURRENT_VM_ERROR` slot. The outer JIT dispatcher in `triet-jit::lib::dispatch_integer` (and sibling dispatch variants) wraps the native call in `std::panic::catch_unwind`; on panic, retrieves the slot and returns as `Err(VmError::…)` to the VM.

**Pseudo-code:**

```rust
// In the shim:
#[unsafe(no_mangle)]
extern "C-unwind" fn __triet_assert(cond: i8, msg_ptr: i64) -> i8 {
    if cond != 1 /* Trit::Positive */ {
        let msg = if msg_ptr == 0 { None } else {
            Some(extract_string_from_ptr(msg_ptr))
        };
        CURRENT_VM_ERROR.with(|slot| {
            *slot.borrow_mut() = Some(VmError::AssertionFailed {
                message: msg,
                function: CURRENT_FUNC_NAME.with(|f| f.borrow().clone()),
            });
        });
        panic!("triet shim assertion failed");  // payload via TLS, msg is debug-only
    }
    0  // Unit placeholder
}

// In the dispatcher (triet-jit::lib):
let result = std::panic::catch_unwind(AssertUnwindSafe(|| native_call(args)));
match result {
    Ok(val) => Ok(reify(val)),
    Err(_payload) => {
        let err = CURRENT_VM_ERROR.with(|slot| slot.borrow_mut().take())
            .unwrap_or(VmError::JitInternal { reason: "shim panic without structured error".into() });
        Err(err)
    }
}
```

**Why `extern "C-unwind"`, not Cranelift trap blocks:**

- **Cranelift trap blocks** would require each shim to translate panic→trap, the JIT translator to emit per-call trap-block scaffolding (`trapnz` + `ehpad`-style cleanup), and the dispatcher to map trap codes to VmError variants. ~43 × per-shim integration, plus Cranelift's `TrapCode` extension table grows. High mechanical complexity.
- **`extern "C-unwind"`** is Rust 2024 stable, Cranelift-compatible (stable ABI declaration: `cranelift_codegen::isa::CallConv::SystemV` with the unwind variant). Standard `std::panic::catch_unwind` at the dispatcher boundary does the rest. No per-shim mechanism beyond `panic!()`.
- **Cargo `panic = "unwind"`** required. Workspace already defaults to unwind (no `panic = "abort"` set in any `Cargo.toml`). v0.10 documents this dependency in `triet-jit/Cargo.toml`.

**Thread-local error context.** Two TLS slots:

- `CURRENT_VM_ERROR: RefCell<Option<VmError>>` — populated by shim before `panic!()`, consumed by dispatcher after `catch_unwind`.
- `CURRENT_FUNC_NAME: RefCell<String>` — set by dispatcher BEFORE the native call (mirrors VM's `frame.func_name` per `vm.rs:2156`); read by shim when constructing `VmError::AssertionFailed { function, … }`.

Single-threaded v0.10 (per [ADR-0028 §9](0028-atomic-primitive.md) BYOS single-thread VM) makes TLS trivially correct. Multi-thread defer per v0.10.x.thread.* — `raw_thread.spawn` lands AFTER shim layer, so per-thread TLS slot semantics are correct by default.

**Performance note:** `catch_unwind` per-call has minor overhead (~10ns + setjmp/longjmp ~50ns on panic path). For the 99% non-error case, overhead is the `catch_unwind` setjmp guard — measurable but small compared to the shim work itself (string formatting, Rc clones). Acceptable for v0.10 dev tier; v1.0+ can revisit if profile shows it.

**Rejected — `extern "C"` non-unwind + manual error register:** would require every shim to return a tagged `Result<…, ErrorIdx>` union via 2 registers (value + error tag). JIT translator would have to emit per-call error-check branch. Verbose, error-prone, no real perf win over `catch_unwind`.

---

## §5 — `unsafe_code` policy: crate-local override in `triet-jit/Cargo.toml` only

**Decision:** `unsafe_code = "forbid"` (workspace default) → `unsafe_code = "deny"` ONLY in `triet-jit` crate. No other crate's policy changes. Each `unsafe` block in `triet-jit` carries a `// SAFETY: …` comment documenting the invariant (per [Rust safety convention](https://doc.rust-lang.org/std/keyword.unsafe.html)). Workspace audit ([release-check.sh](../../scripts/release-check.sh) per [ADR-0009 Addendum §C](0009-version-gate-policy.md)) reports an `unsafe` block count for `triet-jit` and flags any new file in any other crate that introduces `unsafe`.

**Concretely** — `crates/triet-jit/Cargo.toml`:

```toml
[lints.rust]
unsafe_code = "deny"   # override workspace `forbid` per ADR-0032 §5
```

Workspace-level `[workspace.lints.rust] unsafe_code = "forbid"` stays. Crate-local override fully suppresses the workspace value (Rust 2024 cargo lint inheritance rule).

**Why crate-local, not workspace-wide:**

- Other crates have zero reason to want `unsafe`. `triet-core` arithmetic, `triet-parser` tokens, `triet-typecheck` rules, `triet-ir` opcodes — all safe Rust. Widening would erode the workspace's safety story for no benefit.
- `triet-jit` is the **single** crate that legitimately needs `unsafe`: Cranelift's `JITModule::finalize_definitions` returns raw `*const u8` function pointers, and shim layer needs `Rc::into_raw` / `Rc::from_raw` / pointer casts. All `unsafe` is at the language↔runtime ABI boundary, exactly where it belongs.
- Per [ADR-0007](0007-ir-design.md) + project [VISION §6 "Refuse over guess"](../../VISION.md), keeping `unsafe` localized makes audit cheap: one `grep -rn "unsafe" crates/triet-jit/src/` lists every block, every block has a SAFETY comment.

**Each unsafe block — mandatory SAFETY comment template:**

```rust
// SAFETY: <invariant 1>; <invariant 2>; backed by <ADR section or lowerer rule>.
unsafe { /* op */ }
```

**Estimated count for v0.10:** ~50 unsafe blocks across the shim layer (43 builtins × ~1 deref each + ~7 housekeeping like `drop_arc`, transmute, dispatcher entry). Tractable for review.

**Rejected — `unsafe_code = "allow"` at workspace level** (e.g., to skip per-block annotation). Loses the safety bar across the whole project. The whole point of `forbid` workspace-wide is that unsafe is a deliberate, audited choice. Crate-local `deny` preserves that without disabling the bar elsewhere.

---

## §6 — Shim registry shape: static table + `JITBuilder::symbol()` wiring

**Decision:** Shim registration uses a single static table keyed by `BuiltinName` enum, registered via Cranelift's `JITBuilder::symbol(name: &str, addr: *const u8)`. The dispatcher's `compile_program` walks the table once at JIT initialization and pins each shim's Rust function address to a stable symbol name.

```rust
// crates/triet-jit/src/shims.rs (NEW v0.10.x.jit.1):

pub(crate) struct ShimEntry {
    pub builtin: BuiltinName,
    pub symbol: &'static str,           // "__triet_println", etc.
    pub addr: *const u8,                // shim function as raw addr
    pub signature: ShimSignature,       // arg/ret Cranelift types
    pub required_namespace: &'static str, // "sys.io", "sys.atomic", etc.
}

pub(crate) static SHIM_TABLE: &[ShimEntry] = &[
    ShimEntry {
        builtin: BuiltinName::Println,
        symbol: "__triet_println",
        addr: __triet_println as *const u8,
        signature: ShimSignature::VariadicComposites,
        required_namespace: "sys.io",
    },
    // ... 42 more entries
];
```

**Symbol-name discipline:**

- All shim symbols prefixed `__triet_` to namespace-isolate from system C library + Cranelift's own libcalls (per [ADR-0030 §13.4 constraint 2](0030-jit-cranelift-integration.md)).
- Snake-case mirroring `BuiltinName` PascalCase variant: `BuiltinName::FStringConcat` → `__triet_fstring_concat`. Mechanical 1:1 mapping.
- Symbol stability matters once AOT cache lands (v0.10.x.jit.3): shim symbols become libcalls in the persisted ELF object. Cache invalidation key MUST include the shim ABI version (recorded in `manifest.bin` per [ADR-0033](0033-aot-cache-cranelift-object.md) — separate ADR).

**Registry construction at JIT init:**

```rust
let mut builder = JITBuilder::new(cranelift_module::default_libcall_names())?;
for entry in SHIM_TABLE {
    // SAFETY: addr is the Rust function pointer of an #[unsafe(no_mangle)]
    // extern "C-unwind" shim defined in this crate; signature matches the
    // BuiltinName's declared ABI per §1 hybrid table.
    builder.symbol(entry.symbol, entry.addr);
}
```

**Cranelift translator side** (per [ADR-0030 §3](0030-jit-cranelift-integration.md) opcode table): `CallBuiltin { name, args }` emits a `call_indirect` (or declared-extern `call`) targeting the shim symbol, with argument marshaling per §1 hybrid table.

**Why a single static table, not per-category registration:**

- 43 entries fit in one file (`shims.rs`); flat table is the natural shape.
- Categorical splits (`io_shims.rs`, `vector_shims.rs`, `atomic_shims.rs`) would add file-boundary friction without semantic benefit — every shim has the same shape (declare `extern "C-unwind" fn`, register address).
- Single-file makes the `unsafe` audit per §5 a single `git diff` to review.

---

## §7 — Test gates: framework tests + per-builtin integration tests + ABI fuzz

**Decision:** v0.10.x.jit.1 + jit.2 ship with three layered test categories. Each layer must be green before commit.

### 7.1 — Layer A: framework smoke tests (v0.10.x.jit.1 ship gate)

- `framework_shim_call_returns_value` — register a no-op `__triet_test_identity(x: i64) -> i64`, call from JIT'd function, assert round-trip.
- `framework_shim_panic_to_vm_error` — register `__triet_test_panic` that always panics with `VmError::AssertionFailed`, call from JIT'd function, assert dispatcher returns `Err(VmError::AssertionFailed {…})` via `catch_unwind` path per §4.
- `framework_drop_arc_balances_refcount` — emit a synthetic boxed-value cycle (Rc::into_raw → drop_arc), instrument with strong_count probe, assert refcount returns to 1 (and 0 after drop_arc).
- `framework_capability_denied_tiers_down` — set up `CapabilitySet` denying `sys.io`, attempt to JIT a function calling `Println`, assert `JitError::BuiltinCapabilityDenied { builtin: Println, namespace: "sys.io" }`.

These exercise the §1–§5 mechanisms WITHOUT requiring any of the 43 production shims. jit.1 can ship framework infrastructure with just these.

### 7.2 — Layer B: per-builtin round-trip tests (v0.10.x.jit.2 ship gate)

For each of the 43 builtins, one test:

```rust
#[test]
fn jit_println_matches_vm() {
    let src = r#"
        from sys.io import println
        function main() -> Unit {
            println("hello jit")
        }
    "#;
    let (vm_out, jit_out) = run_both(src);
    assert_eq!(vm_out, jit_out);
}
```

`run_both` runs the same source twice — once with `--no-jit` (VM-only), once with JIT enabled and threshold forced to 1 (immediate graduation). Outputs (stdout, return value, error variant if any) compared byte-identical. This is the parity gate per [ADR-0029 §3 Layer C](0029-self-host-port-policy.md) — runtime layer doesn't require self-host port, but it DOES require VM↔JIT semantic equivalence.

43 tests; each ~30 LOC; ~1300 LOC total. Co-located in `crates/triet-jit/tests/shim_parity/`.

### 7.3 — Layer C: ABI fuzz (v0.10.x.jit.2 final gate)

For shims that accept composite args (Vector / HashMap / Atomic / String), one fuzz test per builtin generating random valid `RuntimeValue` inputs, comparing VM vs JIT result via `runtime_eq` (per existing `triet-ir::vm::runtime_eq` helper):

```rust
proptest! {
    #[test]
    fn vector_push_jit_matches_vm(elems in proptest::collection::vec(any::<i64>(), 0..100)) {
        let vec_value = build_vector(&elems);
        let pushed = pick_random_integer();
        assert!(jit_vs_vm_equivalent("vector_push", &[vec_value.clone(), pushed.clone()]));
    }
}
```

Estimated 20 proptest cases × 256 default iterations = 5120 randomized cases per CI run. Acceptable budget; runs in <30s on dev hardware.

### 7.4 — Regression gates

- `release-check.sh` `unsafe` count audit per §5 — script reports `triet-jit` block count and fails if any other crate gains an `unsafe` block.
- ADR-0009 Gate B Hygiene (`cargo clippy --workspace --all-targets`) — must be clean. Note: `triet-jit`'s `#![allow(unsafe_code)]` overrides hit clippy lints differently than other crates; document expected delta in `scripts/release-check.sh`.

---

## §8 — Self-host port plan (per ADR-0029 §5 template)

**Layer A surface changes:** **No.** Builtin shim ABI is internal runtime layer. No lexer, parser AST, SPEC grammar, or `BuiltinName` enum changes — the enum (43 variants) was finalized v0.9 per [ADR-0028](0028-atomic-primitive.md) Addendum (variants 33-42 for Atomic).

**Layer B internal changes:** **No.** Typecheck unchanged. Lowerer unchanged. IR shape unchanged (`Instruction::CallBuiltin { name, args, dest }` already exists). The shim layer consumes existing IR.

**Layer C runtime changes:** **Yes.** Crates affected:

- `triet-jit` (new file `shims.rs` ~1300-1800 LOC, plus framework code in `lib.rs` for TLS + catch_unwind + symbol registration).
- `triet-jit/Cargo.toml` lint table override per §5.
- `release-check.sh` `unsafe` audit step (new).

**Same-phase port required:** **No.** Per [ADR-0029 §3 Layer C independent](0029-self-host-port-policy.md) rule. The self-host compiler at `compiler/*.tri` emits `.khi` containing `Instruction::CallBuiltin` opcodes — the JIT side consumes those. Stage 2 source code is unaffected; self-host compiler doesn't see the shim layer.

**Bootstrap interaction:** Once v0.10.x.jit.3 AOT cache (per [ADR-0033]) lands and v0.10.x.jit.4 lifts Stage 2/3 byte-identical gate, the self-host compiler runtime BENEFITS from shim layer (e.g., heavy `HashMap` usage in lowerer will JIT instead of tier-down). But: Stage 2 source code does NOT change. Same Triết source, faster runtime.

---

## §9 — Implementation sub-task hooks (v0.10.x.jit.1 + jit.2)

ADR-0032 unblocks:

- **v0.10.x.jit.1** (Builtin shim infrastructure) — implements §3 codegen integration + §4 TLS + catch_unwind + §5 unsafe override + §6 registry skeleton + §7.1 Layer A framework tests. NO production shims. Ships ~500 LOC + 4 framework tests.
- **v0.10.x.jit.2** (43 builtin implementations) — implements §1 hybrid marshaling per type + §2 lifetime per shim + §6 SHIM_TABLE entries × 43 + §7.2 Layer B parity tests + §7.3 Layer C fuzz tests. Ships ~1300-1800 LOC + 43 parity tests + ~20 proptest cases.

[ADR-0033](0033-aot-cache-cranelift-object.md) (separate v0.10.0.2 ADR) covers the AOT cache layer that builds on top of these shims — symbol resolution at object load time per §6 naming discipline.

---

## §10 — Decision rationale + connection to ADR-0030

[ADR-0030 §12.4](0030-jit-cranelift-integration.md) deferred the shim layer to v0.10 because shipping skeleton-shims in v0.9 would have committed to ABI choices before the 5 constraints had been thought through coherently. ADR-0032 is the result of that thought — locking the constraints together so v0.10.x.jit.1 + .2 can be mechanical execution against a settled design.

**Author-facing summary** (per CLAUDE.md "present tradeoffs in terms the author cares about"):

- **§1 hybrid ABI** = "primitives stay fast (i64 register), composites use the existing Rc<RuntimeValue> shape that Atomic already uses". Re-uses what's there; no new shape.
- **§2 explicit Drop** = "JIT calls a `drop_arc` shim when SSA value last-used; same as how the lowerer already tracks ownership transitions". Mirrors borrow checker discipline.
- **§3 compile-time capability** = "VM already checks at lowerer time, JIT inherits the same invariant — if the .triv has the opcode, the capability is granted". One source of truth.
- **§4 `extern "C-unwind"`** = "Rust panics propagate naturally through JIT frames; dispatcher catches and converts to VmError". One mechanism, not 43 trap-block scaffolds.
- **§5 crate-local unsafe** = "only `triet-jit` gains unsafe; everything else stays `forbid`". Single-file audit.

Per [feedback_implementer_choice.md] precedent: 5 constraints are implementation-internal; author delegated 2026-05-30. ADR-0032 records the choice + reasoning so future-AI can reconstruct.

---

## Hệ quả

**Possible (positive):**

- v0.10.x.jit.1 + .2 unblocked — mechanical execution against a locked design.
- Real Triết programs (with `println` / `Vector*` / `HashMap*`) get JIT acceleration end-to-end, not just numeric leaves.
- Atomic builtin shims (33-42 per [ADR-0028 §3](0028-atomic-primitive.md)) get native dispatch through JIT — important for v0.10.x.thread.* multi-worker demos.
- VM↔JIT parity test infrastructure (§7.2) catches semantic divergence at commit time.
- AOT cache (v0.10.x.jit.3 per [ADR-0033]) can persist shim symbols by name — backward-compatible cache invalidation.

**Constrained (cost):**

- `triet-jit` crate gains ~50 `unsafe` blocks (§5). Each documented; reviewable in one `git grep`.
- Single-threaded TLS (§4) — multi-thread JIT (post-v0.10.x.thread.*) inherits per-thread slot semantics naturally; no architectural change needed.
- `catch_unwind` per-call overhead (~10ns setjmp guard, negligible) — measurable in microbenchmark; not measurable in real-program profile.
- 43 parity tests + 20 proptests add ~30s to test wall-clock. Acceptable.

**Costly (need verify in v0.10.x.jit.2):**

- Rc::into_raw / Rc::from_raw correctness across 43 shims — each shim's ownership protocol must match §2. Per-shim review burden; mitigated by §7.2 parity tests catching refcount imbalances (would manifest as memory leaks visible in stress tests).
- Cranelift `extern "C-unwind"` ABI support — confirmed stable Rust 2024, but Cranelift's `CallConv` declaration interaction with unwind has occasionally had bugs (last reviewed 2026-Q1). Test §7.1 `framework_shim_panic_to_vm_error` pins behavior.

---

## Không làm (explicitly rejected)

- **All-boxed `*const RuntimeValue` ABI** — every primitive becomes a heap allocation. Brutal for numeric leaf cases (Vector_get with Integer index). Rejected per §1.
- **Per-builtin specialized stubs** — combinatorial explosion (43 builtins × N arg-type combos). Maintenance debt. Rejected per §1.
- **IR-level RetainArc/ReleaseArc opcodes** — bloats register-SSA, violates [ADR-0007](0007-ir-design.md) minimality. Rejected per §2.
- **Per-call runtime capability check inside each shim** — 43 × overhead for a check whose outcome is pinned at program load. Rejected per §3.
- **Cranelift trap blocks for panic propagation** — per-shim scaffolding, TrapCode table growth, mapping logic. Rejected per §4 in favor of `extern "C-unwind"`.
- **Workspace-wide `unsafe_code = "allow"`** — erodes the safety story for the 99% of crates that don't need unsafe. Rejected per §5.
- **Categorical shim registration** (separate `io_shims.rs` / `vector_shims.rs` / etc.) — file-boundary friction, no semantic benefit. Rejected per §6 in favor of single static table.
- **Per-builtin documentation files** — registry-as-truth (§6 SHIM_TABLE) + ADR-0032 itself documents the patterns. No per-builtin Markdown.
- **Multi-thread TLS now** — v0.10 single-thread (BYOS per [ADR-0026 v2](0026-actor-boundary-send-rules.md)). Per-thread slot works correctly when `raw_thread.spawn` lands later in v0.10.x.thread; no design change. Rejected pre-emptive multi-thread.

---

## Prior art

| Source | What we copy | What we change |
|---|---|---|
| Rust 2024 `extern "C-unwind"` | Cross-ABI panic propagation | Triết: stable ABI + thread-local error context |
| Rust `Rc::into_raw` / `Rc::from_raw` | Refcount lifetime crossing FFI boundary | Triết: explicit `drop_arc` shim at SSA last-use rather than `Box::leak` indefinite-life |
| HotSpot JVM JNI | Native shim registry pattern | Triết: simpler — flat static table, no class loading complexity |
| WasmTime hostcalls | Cranelift `JITBuilder::symbol` wiring | Triết: shim symbols prefixed `__triet_` for namespace isolation |
| Cranelift libcalls | `default_libcall_names` registration | Triết: shim layer is parallel to libcalls; both go through `JITBuilder::symbol` |
| LuaJIT C bindings | Hybrid primitive/composite marshaling | Triết: register types match Cranelift native; composite uses Rc, not LuaJIT's GCobj |
| V8 Torque / CodeStubAssembler | Builtin call codegen | Triết: simpler — single `call $shim` instruction per BuiltinName, no Torque-style codegen DSL |

**What we invented:**

- **Hybrid ABI tied to Triết's RuntimeValue shape** — primitives unboxed to Cranelift native types, composites Rc-boxed reusing the existing v0.9 RuntimeValue heap representation. No conversion layer between VM and JIT for composites — the same `Rc<RuntimeValue>` instance can be addressed from both sides.
- **Capability gate at JIT compile time via frozen CapabilitySet snapshot** — leverages [ADR-0017](0017-trilean-policy-hook.md)'s "resolved at program load" invariant to skip per-call check.
- **Crate-local `unsafe_code = "deny"` with single-file audit surface** — preserves the workspace's `forbid` policy while enabling the one crate that legitimately needs unsafe ABI bridging.

---

## Tham chiếu

- [ADR-0007](0007-ir-design.md) — IR design (register-SSA minimality; reason §2 rejects IR-level refcount opcodes).
- [ADR-0009 Addendum §C](0009-version-gate-policy.md) — `release-check.sh` audit protocol (§7.4 regression gates).
- [ADR-0010](0010-ternary-native-ir.md) — BrTrilean → 2-cmp + 2-branch (Trit/Trilean i8 encoding per §1 hybrid table).
- [ADR-0014](0014-hash-scheme-refinement.md) — impl_hash (used by [ADR-0033](0033-aot-cache-cranelift-object.md) AOT cache; not directly by this ADR).
- [ADR-0016 §5](0016-capability-type-system.md) — Capability ambient resolution (§3 compile-time gate inherits this invariant).
- [ADR-0017](0017-trilean-policy-hook.md) — Trilean policy hook (§3 capability already-resolved-at-load rationale).
- [ADR-0022 §4](0022-trit-balanced-ownership.md) — "compiler tự động borrow" (§2 lifetime tracking precedent).
- [ADR-0023](0023-lowerer-ssa-struct-tracking.md) — Unified ValueKind enum (§1 hybrid uses for primitive-vs-composite classification; §2 drop_arc emission at last-use).
- [ADR-0026 v2 §6 BYOS](0026-actor-boundary-send-rules.md) — Single-threaded v0.10 (§4 TLS trivially correct).
- [ADR-0028 §3](0028-atomic-primitive.md) — `Rc<RefCell<RuntimeValue>>` Atomic representation (§1 ABI table reuses directly).
- [ADR-0029 §3 + §5](0029-self-host-port-policy.md) — Layer C runtime independent (§8 no same-phase port); Self-host port plan template (§8 format).
- [ADR-0030 §3 + §12](0030-jit-cranelift-integration.md) — Cranelift opcode table (§1 ABI built on top); §12 deferral chain ADR-0032 resolves.
- [ADR-0033](0033-aot-cache-cranelift-object.md) — AOT cache (built on §6 shim symbol naming).
- [VISION §6](../../VISION.md) — "Refuse over guess" (§5 unsafe localization).
- [SPEC §10](../../SPEC.md) — Reference forms (not directly used here; ADR-0031 covers borrow expressions which v0.10.x.borrow.* enforces).
- Rust RFC 2945 — `extern "C-unwind"` stabilization (§4 ABI choice).
- Cranelift docs — `JITBuilder::symbol` API for shim registration (§6 mechanism).
