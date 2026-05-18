# Demo: Error Handling (v0.7.4.3-error capstone)

This folder is the executable proof for [ADR-0020][adr-0020] (Outcome
error handling) + [ADR-0021][adr-0021] (Trilean! refinement) +
[ADR-0010 Addendum §D][adr-0010-d] (outcome-null runtime unification).
Every feature shipped under the `v0.7.4.3-error.*` sub-tasks appears
at least once across the four `.tri` files.

[adr-0020]: ../../docs/decisions/0020-outcome-error-handling.md
[adr-0021]: ../../docs/decisions/0021-trilean-refinement.md
[adr-0010-d]: ../../docs/decisions/0010-ternary-native-ir.md

## Pipeline

A toy "user registration" + "role lookup" service that demonstrates
the typical layered-error pattern: each stage produces a `T~ConfigError`
(or `T?~ConfigError`) outcome, and the orchestrator chains them with
`~?` so the first failure short-circuits the rest of the pipeline.

```
register_user(age, nick)
    │
    ├─ validate_age(age) ─────────► Integer~ConfigError
    │                                  └─ ~? propagate first failure
    ├─ validate_age_range(...) ──► Integer~ConfigError
    │
    └─ validate_nickname(nick) ──► String~ConfigError
       │
       └─► ~+ UserAccount { ... }  /  ~- err
```

```
find_user_role(user_id) ──► String?~ConfigError
    │
    ├─ ~+ "admin"        (id < 10)
    ├─ ~+ "moderator"    (id < 50)
    ├─ ~0                (id in [50, 100], no role assigned)
    └─ ~- ConfigError    (id ≤ 0 or > 100)
```

## File map

| File | Purpose | Features |
|---|---|---|
| [`errors.tri`](errors.tri) | Shared `ConfigError` struct + constructor | Multi-field struct (fixed by `0d4577e`) |
| [`validators.tri`](validators.tri) | Stage-1 validators returning `T~E` | `~+` / `~-` constructors, `Trilean!` refinement on `if age > 0` (ADR-0021 §2.2), Łukasiewicz `&&` preserving refinement (§2.3) |
| [`registry.tri`](registry.tri) | `T?~E` ternary outcome + `~:` default | All three Ł3 arms (`~+` / `~0` / `~-`), `~:` postfix default, `~0 ↔ null` unification (Addendum §D) |
| [`main.tri`](main.tri) | Pipeline orchestrator + `main()` entry | `~?` propagate with `\|err\|` capture, 2-arm + 3-arm `match` on outcomes |

## Running

The demo runs through the VM tier — `triet build` followed by
`triet run` on the resulting `.triv`:

```bash
cargo build --release
./target/release/triet build demos/05-error-handling/main.tri \
    -o /tmp/error-handling.triv
./target/release/triet run /tmp/error-handling.triv
```

Or as a single-step typecheck-+-run (works for source `.tri` too):

```bash
./target/release/triet run demos/05-error-handling/main.tri
```

Expected output:

```text
ok    : UserAccount{age=25, nick=alice}
age<=0: ConfigError[age]: must be strictly positive
old   : ConfigError[age_range]: value outside allowed [lo, hi] window
noname: ConfigError[nickname]: must not be empty
admin : role=admin
mod   : role=moderator
noroll: role=(none assigned)
bad   : ConfigError[registry]: invalid user id (must be > 0)
range : ConfigError[registry]: id out of range (max 100)
```

## Why VM-only?

The interpreter ([`triet run` on `.tri` source directly, no build
step] dispatches through `triet-interpreter`) only supports the `~0`
constructor today — full outcome operators (`~+` / `~-` / `~?` /
`~:`) are deferred under [ADR-0019 Addendum §A7][adr-0019-a7]
"interpreter parity". The `triet build` path lowers source to `.triv`
and runs through the VM (`triet-ir::Vm`), which has shipped the full
outcome opcode set since `v0.7.4.3-error.3a` (`0xC1`–`0xC6`).

Per VISION §4.3, both interpreter and VM are dev-tier; the production
tier is AOT (`v2.0`) + trytecode (`v∞`).

[adr-0019-a7]: ../../docs/decisions/0019-self-hosting-compiler-bootstrap.md

## Feature checklist

This demo covers every locked outcome-handling feature shipped through
the `v0.7.4.3-error.*` series:

- [x] **`T~E` binary outcome type** — `Integer~ConfigError`,
  `String~ConfigError`, `UserAccount~ConfigError` (ADR-0020 §1.1).
- [x] **`T?~E` ternary outcome type** — `String?~ConfigError` in
  `registry.tri` (ADR-0020 §1.2). `?~` parses as a lexer-level
  compound token (§1.3).
- [x] **`~+ value` success constructor** (ADR-0020 §2).
- [x] **`~- error` failure constructor** (§2).
- [x] **`~0` null constructor** for `T?~E` (§2 + §10 canonical form).
- [x] **`expr ~? |capture| early_return`** postfix propagate
  (§3.1) — `register_user` pipeline.
- [x] **`expr ~: default`** postfix default (§3.2) — `role_or_guest`.
- [x] **Pattern matching on outcome arms** (§5) — `match outcome
  { ~+ x => ..., ~- e => ... }` for binary, `~+`/`~0`/`~-` 3-arm for
  ternary.
- [x] **Trilean! refinement** (ADR-0021 §1–§3) — `age > 0` returns
  `Trilean!`, accepted by plain `if`. `age >= lo && age <= hi`
  preserves refinement through `&&`.
- [x] **`~0 ↔ null` unification at runtime** (ADR-0010 Addendum §B/§D)
  — `registry.tri` `~0` lowers to `Constant::Null`; pattern matching
  + Elvis remain cross-tolerant.

Not covered (deferred items, see [`TODO.md`][todo]):

- [ ] `.unwrap_value("msg")` / `.unwrap_error("msg")` verbose methods
  (ADR-0020 §3.3) — the pipeline uses `~?` propagate which is the
  preferred form when the caller is itself fallible; the verbose
  methods are intended for top-of-stack callers (e.g. test drivers)
  and so live in `crates/triet-cli/tests/error_handling_demo.rs`
  rather than the demo source itself.
- [ ] Generic outcome containers like `Vector<T~E>` — generic
  monomorphization is type-erased per ADR-0019 Addendum §A7 Q3-A
  deviation; outcomes inside generics work but aren't shown here.

[todo]: ../../TODO.md

## Related demos

- `demos/02-module-system/` — pre-v0.6 module + struct exercise.
- `demos/04-capability-system/` — v0.6 capability layer illustrative
  files.

The `04` demo is illustrative-only; this `05` demo is fully
executable through `triet run`.
