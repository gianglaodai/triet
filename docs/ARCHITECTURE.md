# Triết — Kiến trúc chi tiết

> Tài liệu này mô tả chi tiết kiến trúc từng phase đã ship của Triết (v0.2.x → v0.8). Dành cho AI agents + contributors muốn đào sâu một area cụ thể trước khi chỉnh code.
>
> **Khi nào đọc file này:** trước khi modify code trong area tương ứng (e.g., trước khi sửa `triet-pack` đọc §Crate-Pack + §CAS Packaging).
>
> **Tài liệu liên quan:**
> - [`CLAUDE.md`](../CLAUDE.md) — orientation mỗi conversation (state hiện tại + conventions + cadence).
> - [`SPEC.md`](../SPEC.md) — semantics ngôn ngữ (authoritative).
> - [`VISION.md`](../VISION.md) — 5 trụ cột + OS-capable trajectory.
> - [`ROADMAP.md`](../ROADMAP.md) — phasing v0.2.x → v3.0+.
> - [`docs/decisions/`](decisions/) — 27 ADRs (xem cả [chronological README](decisions/README.md) và [thematic by-topic.md](decisions/by-topic.md)).

---

## Compilation pipeline

```
.tri source
    │
    ▼  triet-lexer        tokens (logos-based)
    ▼  triet-parser       AST (recursive descent + Pratt)
    ▼  triet-modules      ResolvedProgram (loader + resolver)
    ▼  triet-typecheck    type errors
    ▼  triet-ir           register-SSA IR + lowerer + bytecode VM
    ▼  triet-interpreter  tree-walking runtime values (dev tier)
    ▼  triet-pack         .khi format + cross-package linker
    ▼  triet-cli          binary, miette diagnostics, JSON output
```

Foundation crates: `triet-core` (Trit/Tryte/Integer/Long arithmetic), `triet-logic` (Trilean Łukasiewicz Ł3 / Kleene K3), `triet-syntax` (AST types + arena).

---

## Arena-based AST

`triet-syntax` allocates recursive nodes (`Expr`, `Stmt`, `Pattern`, `TypeExpr`) in typed sub-arenas inside `Arena`. AST nodes hold `*Id` handles (`u32`-sized) instead of `Box<T>`. Always go through `arena.expression(id)` etc. — never fabricate IDs.

---

## Module system (shipped v0.2.x; ADR-0005 locked)

`triet-modules` produces `ResolvedProgram` instead of a bare `Program`:

- **Multi-arena**: `Vec<Arena>` — one arena per parsed source file. Inline `module foo { … }` shares the parent's arena via `arena_id`; file-bound `module foo` gets a fresh arena. This sidesteps cross-file ID remapping.
- **Flat module list**: `Vec<Module>` indexed by `ModuleId`. Each `Module` has `bindings: HashMap<String, AbsolutePath>` populated by name resolution.
- **Stdlib as real files**: `std/io.tri`, `std/text.tri`, `std/assert.tri`, `std/result.tri` resolved from filesystem (loader walks from `CARGO_MANIFEST_DIR/../../std` or `./std`). Earlier "synthetic registry" approach replaced in v0.2.x.7.
- **Locked architecture decisions** (per ADR-0005, do not change):
  - Single-file = crate root (Python/Go pattern)
  - Inline ≡ file-bound for path resolution (Rust/OCaml precedent)

---

## IR + bytecode VM (shipped v0.3; ADR-0007/0008/0010)

`triet-ir` lowers AST to a register-SSA IR (53 opcodes) and runs it on a stack-of-frames VM. `.triv` is the wire format (currently v5 — bumped at ADR-0010 for `BR_TRILEAN`, ADR-0012 for `WITNESS_CALL`, ADR-0020 for outcome operators). The VM is **development tier only** per [VISION §4.3](../VISION.md); production target is AOT (v2.0) and trytecode (v∞).

ADR-0010 ternary-native IR locks: `BrTrilean` 3-way branch, `Eq`/`Ne` propagate Trilean::Unknown per Ł3, `Constant::Null` is the canonical encoding of `Trit::Zero` discriminator (not a separate "thing"). **Post-ADR-0021** (v0.7.4.3-error.3c), strict `if cond` Unknown-handling moved from *runtime panic via BrTrilean unknown_block* (primary safety) to *compile-time E1033 `PossiblyUnknownCondition`* (primary safety); the unknown_block panic stays as defense-in-depth for `if?`/match/untrusted `.triv` consumers — see [ADR-0010 Addendum §C](decisions/0010-ternary-native-ir.md#addendum-c--v0743-error3c-brtrilean-unknown_block-demoted-to-defense-in-depth).

---

## Crate-Pack distribution (shipped v0.4; ADR-0011/0012/0013)

`triet-pack` defines `.khi` (container: ABI metadata + IR code + reserved sections for witness tables + manifest) and the cross-package linker (`plan_link`). Two-level hash at pack level: `iface_hash` (ABI surface) + `impl_hash` (covers code bytes). BLAKE3, canonicalized via sort-by-name so identical surfaces produce identical bytes.

Linker decisions land in the E2300–E2399 namespace: `MajorVersionMismatch` (E2320), `VersionBelowMinimum` (E2321), `IfaceHashDrift` (E2310 advisory). `iface_hash_pin` is the final arbiter — semver triple is *declaration*, hash is *proof*. Auto-shim is explicitly NOT promised.

---

## CAS Packaging (shipped v0.5; ADR-0014/0015)

Extends the pack-level hash from v0.4 into a **3-cấp hash tree**: term + module + package. Each level has its own `iface_hash` (signature-only) + `impl_hash` (covers body bytes), with 16-byte ASCII domain separators per level to prevent cross-level collisions. `abi_version` bumped 1 → 2 (additive — `.khi` v=1 explicitly refused per ADR-0014 §5, no shim).

Package store lives at `~/.triet/store/` (override via `$TRIET_STORE`). Three branches mirror the hash tree: `term/<impl_hash>/{iface.bin, body.bin}`, `mod/<impl_hash>/index.bin`, `pkg/<impl_hash>/{pack.khi, manifest.bin}`. Plus `names/<pkg>/<semver>.link` (symbolic alias → hash), `roots/<project_id>.root` (GC roots), `tmp/<uid>/` (atomic install staging). Atomic install protocol: write to tmp dir → `rename()` (POSIX atomic; EEXIST = race-lost = success). Manual `dao store gc` (mark-and-sweep). E2360–E2382 namespace covers store I/O + lockfile + resolver errors.

`dao.lock` hand-rolled line format (`format_version 1` + `pkg <name> <maj>.<min>.<pat> <iface_hex> <impl_hash_hex>`) — sort-by-name canonical, diff-friendly, no serde dep. `Resolver` (lockfile authoritative when present + still in store; dep `iface_hash_pin` overrides cache).

CLI: `dao store {import,list,gc}` (lossy v=1 migration deferred until v=1 packs exist in the wild). Body-level RAM dedup (`body.bin`) chờ lowerer per-term IR body split — iface-level dedup proven via `tests/shared_loading.rs`.

---

## Capability System (shipped v0.6; ADR-0016/0017/0018)

Trụ cột bản sắc #5 ([VISION §3.5 + §5](../VISION.md)). Capability is a **namespace attribute** declared in `dao.package` source manifest (ADR-0018 §1) — phương án C picked over capability-as-runtime-token (Pony) and capability-as-effect-annotation (Koka). 4-state `CapabilityLevel`: `Grant`/`Ambient`/`Deny` (Trit) + `Defer` (`Trilean::Unknown`). Wire format reuses `caps section` reserved since v0.4 ABI metadata; `abi_version` stays `2` (ADR-0016 §4 promise: populate-not-bump).

Three-stage enforcement, three-file contract:

- **`dao.package`** — per-package source manifest. Textual level tokens (`grant`/`ambient`/`deny`/`defer`). Parsed by `PackageManifest::parse`; strict whitelist (BOM/CRLF/inline-`#`/oversize-line/file rejected per ADR-0017 Addendum §A).
- **`dao.policy`** — per-deploy resolution rules + default. Numeric tokens (`+1`/`0`/`-1`/`prompt`) for sysadmin audit. Parsed by `PolicyRules::parse`; same shared `strict_parser`. Exact-origin > wildcard `*` precedence. `default prompt` rejected.
- **`dao.lock`** — pre-existing pinned resolution from v0.5 (informs `ResolutionOrigin` of each dep).

Compile-stage `check_capabilities(ResolvedProgram, &PackageManifest)` fires E2200 `MissingCapabilityClaim` + E2201 `SelfContradictoryCapability`. Link-stage `check_link_capabilities(root, available)` fires E2200 (root authority gap) + E2202 `UnresolvedCapabilityPath` + E2203 `CapabilityRefused`. Runtime `CapabilityResolver::resolve(req)` returns `CachedDecision { outcome: Trit, source: DecisionSource }` per ADR-0017 §4; `Defer` unresolved at link goes to `dao.policy` rules → TTY prompt → fail-closed. Per-session cache, monotonicity invariant (ADR-0017 §5).

TTY prompt (`DevTtyPrompt`, ADR-0018 §4 + ADR-0017 Addendum §B): opens `/dev/tty` paired I/O on POSIX (bypasses stdin/stderr — anti-spoofing); full 64-hex hashes never truncated (security boundary); ASCII `!!` markers (universally renderable); `G`/`D` permanent-write via atomic `PolicyRules::save`. Windows ConPTY = `io::ErrorKind::Unsupported` stub.

Root authority semantics (ADR-0016 §7): root package's manifest is the **sole decision-maker**. Dep claims are *requests*, never auto-promoted. No path inheritance (`sys.io grant` does NOT cover `sys.io.async`).

Demo + capstone test: `demos/04-capability-system/` (illustrative) + `crates/triet-typecheck/tests/capability_pipeline.rs` (executable proof for ROADMAP §v0.6 gates).

---

## Self-hosting Compiler (shipped v0.7; ADR-0019/0020/0021/0024)

`compiler/` holds the Triết-in-Triết compiler — 7 `.tri` files (~23K LOC) mirroring crate boundaries: `parser/lexer.tri`, `parser/parser.tri`, `modules.tri`, `typecheck.tri`, `ir_lowerer.tri`, `pack_writer.tri`, `main.tri` (+ `driver.tri` thin entry, `factorial.tri` byte-identical gate fixture). 3-stage bootstrap chain Stage 1 (Rust) → Stage 2 (Triết-built-by-Stage-1) → Stage 3 (Triết-built-by-Stage-2) wired via `crates/triet-bootstrap/tests/bootstrap_loop.rs`. Stage 2 ≡ Stage 3 byte-identical gate `#[ignore]`'d per ADR-0019 §7 Addendum (VM dev tier >15 min per main.tri compile; lifts at v0.9 JIT).

ADR-0020 Outcome (`T~E` binary + `T?~E` ternary with `~+`/`~0`/`~-` constructors + `~?` propagate + `~:` default postfix ops) and ADR-0021 Trilean! refinement (strict `if cond` requires non-`unknown`, raises E1033 otherwise) are the v0.7 design locks now baked into typecheck + lowerer + self-host. ADR-0024 renames `.tri.bin` → `.khi` (pack file, deterministic compression) + canonical `dao` binary identity.

---

## Ownership Foundation + Concurrency Primitives (BYOS, shipped v0.8; ADR-0022/0025/0026 v2/0027)

Trụ cột bản sắc memory model. ADR-0022 locks **S6 — Single-form Ownership with 6 trits** as the 5-form reference family: `&+` strong owner (frozen), `&0` neutral borrow, `&-` weak observer (upgrade-tracked), bare `&` (parser disambiguates), plus `owned` for transfer semantics. Compound `&+`/`&0`/`&-` are lexer tokens (longest-match before `&&`). `triet-core::memory::ObjectHeader` is the 8-byte (binary) / 54-trit (ternary) per-allocation header with refcount atomic ops + sentinels.

**BYOS — Bring Your Own Scheduler** (ADR-0026 v2): Triết core provides Send rules + Atomic primitives + capability gates **without** baking a scheduler into the language. `actor`/`spawn`/`receive`/`send`/`async`/`await` are NOT keywords (explicit refuse-list ADR-0026 v2 §6). Scheduler lives in stdlib (`std.concurrency.*` planned v0.10) or external (kernel-mode crates use raw capability + FFI). Same compile-time safety regardless of scheduler choice. The 2026-05-26 pivot from ADR-0026 v1 actor-model addressed kernel-writability: Linux Rust modules cannot use async runtime (defer to C scheduler) — same problem applied to v1 Triết.

Send derivation auto-classifies 13 type categories per ADR-0026 v2 §2.1 via `triet-typecheck::types::Type::is_send()`. E2500 `NotSendCannotCrossBoundary` fires on generic `Send` bound violations. ADR-0025 borrow checker rules + E24XX lifetime/move/exclusivity diagnostics are **specced + skeleton-emitted**; enforcement deferred to v0.9 (need real-world corpus first). ADR-0027 diagnostic format standard locks AI-first format (header `EXXXX ErrorName` + body + `[Fix N]` blocks, ASCII, no diff `-/+`) across all error/warning text — retroactive to ADR-0020/0025.

Capability schema (`dao.package`) extends with concurrency caps: `sys.raw_thread`, `sys.atomic`, `dev.ffi`, `dev.raw_memory`, `dev.reinterpret`; ownership caps `dev.self_ref`, `dev.custom_drop`. Resolver fix: ambient capability modules bypass filesystem checks. Demo `examples/atomic_counter/` is aspirational pseudo-code (parser-side ReferenceForm not yet ported — tracked v0.8.x.review).

---

## Error code namespace (full map)

- `triet::lex::E0000` — lexer
- `triet::parse::E000X` — parser
- `triet::typecheck::E10XX` — type checker (E1024-E1032 + E1037 + E1038 + E1039 ADR-0020 Outcome ternary operator family với auto-wrap §3.0; E1033 `PossiblyUnknownCondition` + E1034 `TrileanReturnNotRefined` per ADR-0021)
- `triet::runtime::E20XX` — interpreter
- `triet::modules::E21XX` — loader / resolver (E2100 = cyclic, E2101 = file-not-found, etc.)
- `triet::capability::E22XX` — capability system (E2200 missing claim / E2201 self-contradictory / E2202 unresolved path / E2203 refused / E2204 dup decl / E2205 policy runtime / E2206 invalid root / E2207 invalid level / E2208 loader)
- `triet::pack::E23XX` — semver linker (existing v0.4)
- `triet::borrow::E24XX` — borrow checker (E2400 lifetime inference / E2410 mutability / E2420 move + use-after-move / E2430 namespace inference / E2440 NLL exclusivity / E2450+ reserved drop+custom-drop) per ADR-0025
- `triet::actor::E25XX` — actor boundary + concurrency (E2500 Send derivation / E2510 scope-ref leakage / E2520 mutable-share anti-pattern / E2530+ reserved reply channel / supervision) per ADR-0026

All errors implement `miette::Diagnostic`. The CLI's `--json` flag also needs each variant in `parse_error_code` / `type_error_code` / `runtime_error_code` mappers in `crates/triet-cli/src/main.rs` — keep them in sync when adding variants.

**Diagnostic format:** all error/warning text follows the canonical AI-first format locked in [ADR-0027](decisions/0027-diagnostic-format-standard.md) — header `EXXXX ErrorName` + body + optional span block + optional `[Fix N]` numbered fix blocks with imperative `Change/Wrap/Use/Add/Replace/Move X to Y`. Pure ASCII, no diff `-/+`. Retroactive scope: ADR-0020 + ADR-0025 already comply; ADRs introducing new diagnostics must follow §2 of ADR-0027.
