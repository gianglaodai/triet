# Demo: Capability System (v0.6)

This folder illustrates the v0.6 capability layer — the third bản sắc
trụ cột of Triết per [VISION §5][vision-5]:

1. **Trit-level capability** — `Grant` / `Ambient` / `Deny` map to
   `Trit::Positive` / `Zero` / `Negative`. Plus a fourth state
   `Defer` encoding `Trilean::Unknown` for runtime-resolved policy.
2. **Łukasiewicz Ł3 runtime resolution** — `Defer` slots are decided
   at load time by a `dao.policy` file or an interactive TTY prompt,
   per [ADR-0017][adr-0017].
3. **OS-Native Capability Namespaces** — `sys.*` / `dev.*` / `usr.*`
   enforced at compile, link, and runtime stages.

The files in this folder are **illustrative only** — `triet check
app.tri` will NOT succeed yet, because:

- `sys.*` / `dev.*` / `usr.*` are reserved namespaces (per
  [ADR-0005][adr-0005]); the loader refuses to resolve them until
  v0.6 wires real backing modules (`sys.io` etc. are conceptual at
  this stage).
- No `triet build` step currently emits `.tripack`s with the `caps
  section` populated from `dao.package`. The wire format is locked
  ([ADR-0016 §4][adr-0016]) and round-trip tested, but the lowerer
  side that feeds it is post-v0.6 work.

v0.6 ships the type-checker, linker, and resolver machinery; the CLI
integration (running this demo end-to-end with one `triet check`
invocation) is deferred to v0.7 self-hosting. The executable proof
lives in
[`crates/triet-typecheck/tests/capability_pipeline.rs`][test-file]
— a Rust integration test that threads the full pipeline on synthetic
data matching the shape of this demo.

## Files

| File | Purpose |
|---|---|
| [`dao.package`](dao.package) | Per-package source manifest (ADR-0018 §1). Declares the capabilities the source uses. |
| [`dao.policy`](dao.policy) | Per-deploy resolution rules (ADR-0017 §3). Decides what `Defer` caps resolve to. |
| [`app.tri`](app.tri) | Illustrative source with three cross-root imports. |

## The three ROADMAP §v0.6 gate scenarios

### Gate 1 — Compile-time error rõ ràng khi `usr.*` import `dev.*` không có capability

If [`app.tri`](app.tri) is edited to add:

```triet
from dev.disk import read_block
```

…without a corresponding `requires dev.disk grant` (or `defer`) in
[`dao.package`](dao.package), the type-checker's
[`check_capabilities`][check-caps] pass refuses with **E2200
`MissingCapabilityClaim`**:

```
package `myapp` imports `dev.disk` but `dao.package` has no
matching `requires` entry
   help: add `requires dev.disk grant` (or `defer`) to `dao.package`.
         ADR-0016 §5 rule 1.
```

The current `dao.package` declares `requires dev.disk deny`, so a
`from dev.disk import …` would instead trigger **E2201
`SelfContradictoryCapability`** — distinct code because the diagnostic
points at the contradicting `deny` line rather than asking the user
to add something they already wrote.

Test reference: `compile_usr_imports_dev_without_claim_fires_e2200`
and `compile_usr_imports_dev_with_deny_fires_e2201` in
[`capability_pipeline.rs`][test-file].

### Gate 2 — Runtime policy hook hoạt động cho `Trilean::Unknown`

`requires sys.net.dns defer` in [`dao.package`](dao.package) means
"don't decide at compile time; ask the deploy-time policy". When the
loader reaches `lookup("example.com")` at runtime, the resolver
([ADR-0017 §4][adr-0017]) finds the matching rule in
[`dao.policy`](dao.policy):

```
rule sys.net.dns   fresh    prompt
```

…and falls through to the TTY prompt (ADR-0018 §4 mock UI). The user
sees a box-formatted summary with the requesting package, full
BLAKE3 `iface_hash` + `impl_hash`, dep chain, and four choices:

- `g` grant once (this session)
- `d` deny once (this session)
- `G` grant permanent — writes a fresh rule to `dao.policy`
- `D` deny permanent — writes a fresh rule to `dao.policy`

In headless mode (CI, no TTY available), the resolver fails closed
with **E2205 `NonTTYDefer`** — refuse over guess per [VISION §6][vision-6].

Test reference: `resolve_defer_with_grant_callback_returns_positive`
and `resolve_defer_without_callback_fails_closed_nontty`.

### Gate 3 — Demo capability-restricted program chạy được + bị reject khi capability sai

The capstone scenario walks all three stages on the same fixture:

**Accept path** — manifest grants `sys.io`, defers `sys.net.dns`, the
stdlib pack exports both modules, the policy rule grants
`sys.net.dns` on `Fresh` origin → compile + link + resolve all
accept. The Defer cap resolves to `Trit::Positive` via the policy
rule (no prompt fires; the rule is decisive).

**Refuse path** — same `app.tri` but the manifest forgets to declare
`dev.disk` while the source uses it. Compile stage refuses with
E2200 before link or resolve get a chance — fail fast.

Test reference: `full_pipeline_capstone_happy_path` and
`full_pipeline_capstone_refuse_path`.

## Why the CLI doesn't run this yet

The CLI's `triet check <file.tri>` reads a single source file. Wiring
caps end-to-end needs:

1. Project-layout discovery — locate `dao.package` from a source
   file path.
2. Cap-aware build pipeline — `triet build` must emit `.tripack`s
   with the `caps section` populated from manifest `requires` lines.
3. Loader integration — iterate `CapabilityLinkReport.deferrals` and
   call `CapabilityResolver::resolve` with the loader's
   `DevTtyPrompt`.

All three pieces are non-trivial structural changes that land cleaner
with v0.7 self-hosting (where the compiler itself is written in
Triết and can pick a project layout that matches its own ergonomics).
v0.6 ships the machinery and the gate proofs; v0.7+ wires the user
experience.

## What the resolver does NOT do (defer per ADR)

- **Path inheritance** — `requires sys.io grant` does NOT cover
  `sys.io.async`. Each path is a separate declaration (ADR-0016 §2).
- **Wildcard claims** — no `sys.* grant`. Explicit > implicit
  (VISION §6). Use `default` in `dao.policy` for blanket policies.
- **Auto-promotion through deps** — root manifest is the sole
  authority (ADR-0016 §7). A dep claiming `sys.io grant` does NOT
  grant root the same capability.
- **TOML/YAML/JSON manifest syntax** — hand-rolled line format mirrors
  `dao.lock` precedent (ADR-0015 §6).
- **Hash truncation in TTY prompts** — full 64 hex always
  (ADR-0018 §4). Short-SHA is a collision attack surface for
  typosquatting.

## ADR cross-reference

- [ADR-0016 — Capability type system][adr-0016] — namespace +
  manifest, Trit-level grant/deny/ambient, Trilean::Unknown defer.
- [ADR-0017 — Trilean policy hook protocol][adr-0017] —
  `dao.policy` grammar, resolution algorithm, monotonicity
  invariant, E2205 sub-variants.
- [ADR-0018 — Capability loader semantics][adr-0018] —
  `dao.package` grammar, eager link-time check (Step 6a),
  TTY provenance prompt UX, E2208 sub-variants.

[vision-5]: ../../VISION.md
[vision-6]: ../../VISION.md
[adr-0005]: ../../docs/decisions/0005-module-system.md
[adr-0016]: ../../docs/decisions/0016-capability-type-system.md
[adr-0017]: ../../docs/decisions/0017-trilean-policy-hook.md
[adr-0018]: ../../docs/decisions/0018-capability-loader-semantics.md
[check-caps]: ../../crates/triet-typecheck/src/capability_check.rs
[test-file]: ../../crates/triet-typecheck/tests/capability_pipeline.rs
