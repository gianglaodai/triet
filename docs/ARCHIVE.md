# Triết — Legacy Archive (v0.2–v0.10 compiler)

> **What this is.** On 2026-06-04 the v0.2–v0.10 compiler backend was deleted and
> the project restarted from the backend up (see `CLAUDE.md` → "What this is").
> This single file is the **reference digest** of the deleted architecture and a
> **classified catalog of all 36 ADRs**. It replaces the old `docs/ARCHITECTURE.md`
> and `docs/plans/` (both removed — full text remains in git history).
>
> **Read this only for reference.** It describes code that no longer exists.
> The **language semantics** are NOT legacy — the rewrite preserves the language.
> ADRs tagged **LIVE** below remain authoritative and stay in `docs/decisions/`.

---

## 1. The deleted compiler — architecture digest

The shipped pipeline (all of this is GONE except the reused frontend):

```
.tri → triet-lexer → triet-parser → triet-modules → triet-typecheck
     → triet-ir (register-SSA IR + bytecode VM)   [DELETED]
     → triet-interpreter (tree-walking, dev tier)  [DELETED]
     → triet-pack (.khi + linker)                  [survives, unwired]
     → triet-cli (binary `dao`)                    [DELETED]
```

**Reused (still live):** `triet-core` (Trit/Tryte/Integer/Long), `triet-logic`
(Trilean Ł3/K3), `triet-syntax` (arena AST), `triet-lexer`, `triet-parser`,
`triet-modules`, `triet-typecheck`.

**Per-phase history (deleted code — for intent only):**

- **v0.2.x Module system** (ADR-0005, LIVE) — multi-arena `ResolvedProgram`, dot
  paths, Python-style imports, stdlib from filesystem. Single-file = crate root;
  inline ≡ file-bound. *Language rules survive; the `ResolvedProgram` impl is reused.*
- **v0.3 IR + VM** (ADR-0007/0008/0010) — register-SSA IR, 53 opcodes, `.triv`
  wire format v5, `BrTrilean` 3-way branch, `Eq`/`Ne` Ł3-aware, `Constant::Null`
  = `Trit::Zero`. VM was dev-tier only. *IR + VM DELETED; the ternary branching
  SEMANTICS (3-way, Ł3 Eq/Ne, strict-if→E1033) are preserved in the new MIR + typecheck.*
- **v0.4 Crate-Pack** (ADR-0011/0012/0013) — `.khi` container, BLAKE3 two-level
  hash (`iface_hash`+`impl_hash`), linker `plan_link`, E2300–E2399 semver matrix.
  *`triet-pack` survives but is not wired into the rewrite pipeline.*
- **v0.5 CAS Packaging** (ADR-0014/0015) — 3-level hash tree (term+module+package),
  `~/.triet/store/`, atomic install, mark-sweep GC, `dao.lock`. *Tooling, unwired.*
- **v0.6 Capability System** (ADR-0016/0017/0018, LIVE semantics) — namespace
  attribute in `dao.package`, 4-state Grant/Ambient/Deny/Defer, `dao.policy`
  resolution, `/dev/tty` prompt, E22XX. Root manifest = sole decision-maker, no
  path inheritance. *Capability MODEL is language (LIVE); the loader impl is gone.*
- **v0.7 Self-hosting Compiler** (ADR-0019/0020/0021/0024) — `compiler/` ~23K LOC
  Triết-in-Triết, 3-stage bootstrap. **`compiler/` was deleted 2026-06-04** (it
  targeted the deleted IR/VM and no longer parsed under the current frontend).
  *Outcome (0020) + Trilean! (0021) semantics are LIVE; bootstrap machinery is gone.*
- **v0.8 Ownership + BYOS** (ADR-0022/0025/0026/0027, LIVE) — S6 5-form reference,
  `ObjectHeader`, Send derivation (13 categories), BYOS keyword refuse-list, E24XX/
  E25XX, AI-first diagnostic format. *All LIVE — these define the language.*
- **v0.9–v0.10** (ADR-0028–0033) — Atomic primitive, borrow-expression syntax,
  Cranelift JIT (delegate-to-VM shims), AOT cache. *JIT/AOT impl DELETED; Atomic +
  borrow-expression SYNTAX are LIVE.*
- **v0.11 (unshipped, deleted)** (ADR-0034/0035/0036) — JIT aggregate coverage via
  delegate-to-VM boxing (reached 96% before deletion), refcount discipline,
  `TypeTag::Opaque`. *All HISTORICAL — this is the work that was thrown away.*

The old `docs/plans/v0.7.9-implementation-plan.md` (Track A implementation plan)
is also archived here by reference — see git history if needed.

---

## 2. ADR catalog — all 36, classified

**LIVE** = still authoritative for language semantics; lives in `docs/decisions/`,
the rewrite must honor it. **TOOLING** = describes `triet-pack` (survives, unwired)
— revisit if/when packaging is rewired. **HISTORICAL** = describes deleted compiler
internals (IR/VM/bootstrap/old-JIT); reference only.

| ADR | Title | Status |
|---|---|---|
| 0001 | Nullable memory layout (`T?` discriminator) | **LIVE** (T? semantics) |
| 0002 | F-string format spec | **LIVE** |
| 0003 | Iterator protocol | **LIVE** |
| 0004 | Multi-line string indentation | **LIVE** |
| 0005 | Module system (dot paths, Python imports, verbose keywords) | **LIVE** |
| 0006 | Ternary packaging vision (informational) | **LIVE** (info) |
| 0007 | IR design (register-based SSA) | HISTORICAL |
| 0008 | `.triv` bytecode binary format | HISTORICAL |
| 0009 | Version gate policy (4-gate matrix) | **LIVE** (process) |
| 0010 | Ternary-native IR (`BrTrilean`, Ł3 `Eq`/`Ne`) | HISTORICAL vessel; **semantics LIVE in MIR** |
| 0011 | ABI metadata format (BLAKE3 two-level hash) | TOOLING |
| 0012 | Witness table dispatch | HISTORICAL |
| 0013 | Semver linking policy (E2300–E2399) | TOOLING |
| 0014 | Hash scheme refinement (3-level tree) | TOOLING |
| 0015 | Package store layout (`~/.triet/store/`) | TOOLING |
| 0016 | Capability type system (namespace + manifest, E22XX) | **LIVE** |
| 0017 | Trilean policy hook (`dao.policy`, TTY prompt) | **LIVE** (semantics) |
| 0018 | Capability loader semantics | **LIVE** semantics; loader impl HISTORICAL |
| 0019 | Self-hosting compiler bootstrap | HISTORICAL |
| 0020 | Outcome error handling (`T~E` / `T?~E`, `~+`/`~0`/`~-`) | **LIVE** (core) |
| 0021 | `Trilean!` refinement (strict `if`, E1033/E1034) | **LIVE** (core) |
| 0022 | Trit-balanced ownership (S6 5-form reference) | **LIVE** (core) |
| 0023 | Lowerer SSA struct-tracking (`ValueKind`) | HISTORICAL (old lowerer) |
| 0024 | Khí + Đạo identity naming | **LIVE** partial; `dao` binary deleted |
| 0025 | Borrow checker rules (NLL, E24XX) | **LIVE** (core) |
| 0026 | Concurrency & Send rules (BYOS, E25XX) | **LIVE** |
| 0027 | Diagnostic format standard (AI-first) | **LIVE** |
| 0028 | Atomic primitive (Ordering→Trit, API) | **LIVE** type/API; Rust-shim impl HISTORICAL |
| 0029 | Self-host port policy | HISTORICAL (no self-host now) |
| 0030 | JIT integration (Cranelift, 3-tier, delegate-to-VM) | HISTORICAL (old JIT) |
| 0031 | Borrow expression syntax (`&FORM operand`) | **LIVE** |
| 0032 | Builtin shim ABI | HISTORICAL (old JIT) |
| 0033 | AOT cache via `cranelift-object` | HISTORICAL (old AOT) |
| 0034 | JIT aggregate coverage (delegate-to-VM) | HISTORICAL (deleted v0.11) |
| 0035 | JIT boxed-value refcount discipline | HISTORICAL (deleted v0.11) |
| 0036 | `TypeTag::Opaque` aggregate disambiguation | HISTORICAL (deleted v0.11) |

> **Note on physical files.** The ADR files all remain in `docs/decisions/` for
> now — several HISTORICAL ones are cross-referenced by LIVE ones, so physical
> deletion is deferred to a deliberate, link-fixing pass. This catalog is the
> authoritative live/dead map until then.
