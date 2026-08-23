# ADR 0087 — Builtin Print — Overloads & I/O Shim

**Status:** Approved (Signed by Mentor G on 2026-07-25). Applies to Tier C+.

**Issue:** `print`/`println` represent the FIRST stdout write operations designed for the rewrite backend. Typecheck already declares both (`crates/triet-typecheck/src/env.rs:144` `print`, `:152` `println`, both `String → Unit`), but the lowerer (`crates/triet-lower/src/lib.rs`) LACKS a builtin arm for them: `match callee_name.as_str()` at `:2661` lists `concat`/`len`/`vector_new`/`push`/… but omits `"print"`/`"println"`. Calls therefore fall through to the default arm `_ => { /* fall through to user-defined function dispatch */ }` at `:3241` — the lowerer treats `print`/`println` as user-defined functions, cannot locate their definitions, and the JIT fails with `callee 'println' not found`, exit code 4. This is NOT a silent miscompile — purely a feature gap: typecheck promised what the backend had not yet lowered.

## Decision

### 1. Four Overload Signatures

`print(String)`, `print(&0 String)`, `println(String)`, `println(&0 String)`.

- **Owned `String`** (by-value) = MOVE = consumes the value — caller cannot reuse the variable following the call (mirrors the ownership rule of all existing functions taking `String` by-value, ADR-0042).
- **`&0 String`** (borrow, read-only reference) = Reference is Copy (S6, SPEC §10) → reusable post-print.

Both forms are required: printing literals/temporaries (natural move, no need to retain) and printing variables that will be used subsequently (requires borrowing) are equally common in practice; supporting only one forces developers into artificial `concat`/clone workarounds merely to print output.

### 2. Four Separate Extern-C Shims by Symbol Name (No `is_owned` Flag)

| Signature | Shim | Arity | `arg_consumes` | Behavior |
|---|---|---|---|---|
| `print(String)` | `__triet_print` | 3 (ptr, len, cap) | `[true]` | write ptr..len to stdout → `free(ptr, cap)` |
| `print(&0 String)` | `__triet_print_ref` | 2 (ptr, len) | `[false]` | write only, no free |
| `println(String)` | `__triet_println` | 3 | `[true]` | write + `\n` → `free` |
| `println(&0 String)` | `__triet_println_ref` | 2 | `[false]` | write + `\n`, no free |

Memory management responsibility is hardcoded into the symbol NAME (4 distinct symbols), rather than passing a runtime `is_owned` flag for a shared shim to branch on.

- **Owned = `arg_consumes: [true]`:** Moving in transfers ownership to the callee (shim) ⇒ shim handles `free`. Caller-side slot is zeroed by M3 (move-tracking active for all consuming calls) ⇒ caller Deinit observes an empty slot ⇒ `free(0)` is a no-op ⇒ exactly one free across the value's lifecycle. Precedent: `__triet_vector_push` (when pushing an owned `String` into a `Vector<String>`).
- **Ref = `arg_consumes: [false]`:** Owner retains ownership; `free` occurs within owner scope normally (Deinit tombstone, ADR-0042), not inside the shim.

### 3. Proper `Unit` Returns — No Throwaway i64

Add a `Unit`-return branch to `emit_shim_call` (`crates/triet-lower/src/lib.rs:1669`): when a shim has no meaningful return value, DO NOT allocate a `dest` local and bind it as if receiving throwaway i64 data — allocate no dest, bind no return value. JIT `ShimSymbol` (`crates/triet-jit/src/mir_lower.rs:104`) already provides void templates (`has_return: false`, used by `fn_1_0`/`fn_2_0`/`fn_5_0`) — the 4 new print/println shims reuse these templates (`fn_3_0`/`fn_2_0` depending on arity). No new `ShimSymbol` registration variants required, but **`emit_shim_call` in the lowerer must be updated** because it previously ALWAYS allocated `dest` + assigned `return_shape: ReturnShape::Scalar` regardless of whether the shim returned values (confirmed at `:1685`/`:1698`).

This cleanly handles all future `Unit`-returning builtins through a unified lowerer path, avoiding throwaway local allocations.

### 4. Capabilities = Compile-Time Only, No Runtime Gate

Relies on existing capability infrastructure (`crates/triet-typecheck/src/capability_check.rs`, E2200 `MissingCapabilityClaim` / E2201): `std` is ambient (bypassing namespace checks), `sys.io` requires explicit grant declarations. No `__triet_cap_check` runtime calls are added before invoking shims, adhering to VISION (capabilities are static design constraints, without runtime overhead at this layer).

## Alternatives Considered

| # | Alternative | Pros | Cons | Conclusion |
|---|---|---|---|---|
| 1 | Consume-only: support only owned `print(String)`, forcing caller moves/clones on every print | Minimal shims (2 instead of 4), simpler lowerer | Poor ergonomics: printing a variable and continuing to use it requires artificial `concat`/cloning; inconsistent with how `&0` is used for other read builtins (`len(&0 String)`) | Rejected (G ruled out) |
| 2 | Single shim `has_return: true` returning fixed i64 (e.g. 0) as throwaway Unit value | Reuses `emit_shim_call` unchanged, least effort | Tech debt: all future `Unit`-returning builtins repeat the anti-pattern of returning unread 0s; `dest` locals allocated and marked StorageLive for non-existent semantic values | Rejected (G ruled out; update `emit_shim_call` once cleanly) |
| 3 | Two shims (`print`/`println`) + runtime `is_owned` flag branching inside C shim | Fewer symbols (2 instead of 4) | Introduces runtime branching inside C shims based on lowerer data — less clean than encoding memory responsibility in symbol names at compile-time | Rejected (G ruled out 4-symbol clarity preferred) |
| 4 | Four overload signatures, 4 separate shims by symbol name, update `emit_shim_call` with Unit branch (Selected) | Complete ergonomics (move + borrow), compile-time memory responsibility without runtime flags, clean path for future `Unit` builtins | Four new shims added to `builtin_shim_meta` table — expands table surface area managed by ADR-0085 | **SELECTED** |

## Consequences

### Positive
- Stdout output functional for the first time on the rewrite backend (Tier C) — `.tri` programs can print results directly to console rather than relying solely on exit codes or test harnesses.
- `emit_shim_call` gains a clean `Unit` path, reusable for future `Unit` builtins (e.g. side-effect-only operations) without ad-hoc call-site patching.
- Both move and borrow modes supported, consistent with existing read builtins (`len`, `eq`, `concat` on `&0 String`).

### Negative
- `builtin_shim_meta` table adds 4 new entries — expanding the surface area guarded by ADR-0085.
- 4 symbols for 2 source functions (`print`/`println`) — higher symbol count than theoretical minimum (2), in exchange for eliminating runtime branching flags.

### Risks to Mitigate
- Four new `arg_consumes` entries for `print`/`print_ref`/`println`/`println_ref` must be verified using bidirectional FREE-count teeth per ADR-0085 discipline — MUST NOT merge without canaries for these 4 entries.
- The new `Unit`/void branch in `emit_shim_call` must be verified not to disrupt existing call sites (`concat`/`len`/`push`/…) that route through the legacy `Scalar` path.

## Out of Scope (Deferred)

- `read_line` (input, out of scope for this WO though declared in `env.rs`).
- f-strings / runtime formatting (string interpolation) — scoped exclusively to literal/variable `String`/`&0 String`.
- Buffering policy (line-buffered vs unbuffered stdout) — unconstrained by this ADR, preserving existing runtime write defaults.

## Effective Date

- Tier C+ — takes effect when the WO implementing this ADR merges.
- Non-retroactive — no pre-existing print/println code existed prior to this ADR (pure feature gap, no legacy behavior modified).
