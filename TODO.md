# TODO

Sub-task tracking — short-term work in progress.

- Long-term phasing: [`ROADMAP.md`](ROADMAP.md)
- Architectural decisions: [`docs/decisions/`](docs/decisions/)
- Language semantics: [`SPEC.md`](SPEC.md), [`VISION.md`](VISION.md)

This file tracks the **current phase** only. When a phase finishes, its summary archives to `ROADMAP.md` and detailed checkboxes are deleted from here.

---

## v0.2 — v0.10.x archived

All shipped phases now live in [`ROADMAP.md`](ROADMAP.md):

| Phase | ADRs | Final test count |
|---|---|---|
| v0.2.x Module system | 0005, 0006 | 700+ |
| v0.3 Bytecode VM + Stable IR | 0007, 0008 | 835 |
| v0.3.x.cleanup | 0009 | 835 |
| v0.3.x.ternary | 0010 | 838 |
| v0.4 Crate-Pack + Stable ABI | 0011, 0012, 0013 | 867 |
| v0.5 CAS Packaging | 0014, 0015 | 918 |
| v0.5.x.review | 0015 Addendum | 924 |
| v0.6 Capability System | 0016, 0017, 0018 | 1079 |
| v0.6.x.review | 0018 Addendum | 1085 |
| v0.7 Self-hosting Compiler | 0019, 0020, 0021, 0024 | 1345 |
| v0.8 Ownership Foundation + BYOS | 0022, 0025, 0026 v2, 0027 | 1425 |
| v0.8.x.review (audit fixes) | — | 1425 |
| v0.8.x.docs-reorg (token + ADR thematic + ROADMAP compress + SPEC expand) | — | 1425 |
| v0.8.x.cadence-fix (process enforcement automation) | 0009 Addendum | 1425 |
| v0.8.x.completion (trục 2 implementation gap closure) | — | 1436 |
| v0.9.0 (Design phase — ADR-0028/0029/0030) | 0028, 0029, 0030 | 1436 |
| v0.9.x.atomic (Atomic Primitive Implementation) | 0028, 0031 | 1506 |
| v0.9.x.jit (Cranelift JIT — partial; .4/.6/.7/.8 deferred v0.10) | 0030 | 1536 |
| v0.9.final (version bump + archive) | — | 1536 |
| v0.10.0 (Design phase — ADR-0032/0033) | 0032, 0033 | 1536 |
| v0.10.x.interp (Interpreter atomic parity) | 0031 | 1549 |
| v0.10.x.jit (JIT builtin-shim layer — 36/43; .3/.4 deferred v0.11) | 0032 | ~1620 |
| v0.10.x.thread (Multi-thread Atomic — spawn + Arc<Mutex>) | 0026 v2, 0028 | ~1635 |
| v0.10.x.borrow (NLL enforcement — E2440/E2400/E2411/E2403) | 0025, 0031 | 1637 |
| v0.10.final (version bump + archive) | — | 1637 |

---

## v0.11 — JIT AOT cache + bootstrap gate lift 🔄 in progress

**Scope decision 2026-05-31 (author):** v0.11 prioritizes the **JIT AOT cache** first (over `std.concurrency.*` or low-risk cleanup). Rationale: it lifts the bootstrap byte-identical gate that's been `#[ignore]`'d since v0.7 + delivers the headline ≥10× perf win. The other v0.11 backlog items (varargs shims, borrow corpus, concurrency closures, `std.concurrency.*`) trail the AOT cache or move to a later phase.

**Pre-v0.11 baseline audit:** ✅ `scripts/release-check.sh` PASSED per ADR-0009 Addendum §C. 1637 tests, all 4 gates green (run 2026-05-31). Safe to open phase.

### v0.11.0 — Design phase (loader-approach resolution)

- [x] **v0.11.0.1** — [ADR-0033 Addendum] **Loader-approach decision** — locked **Path A (Triết owns its relocating loader)**, author sign-off. Path B (system-linker + `dlopen`) rejected: runtime C-toolchain dependency on the perf path + leans on `dlopen`/ELF dynamic linking, neither exists on balanced-ternary hardware. Path A honors OS-capable/from-scratch identity. 4 normative safety constraints for jit.3: CodeLoader trait, ELF/x86_64 POSIX-first, bounded reloc set + refuse-on-unknown, test regimen (round-trip value parity + proptest fuzz + W^X). Supersedes only ADR-0033 §3 framing; §3 symbol-resolution + §1/§2/§4–§10 unchanged. — `ee624ce`
- [ ] **v0.11.0.2** — [ADR-0033 Addendum] **Cache identity decision** — locked **key = canonical `impl_hash_mod` + per-module objects**, author sign-off. Rejects the content-hash shortcut (parallel identity space disjoint from the [ADR-0014] hash tree → would break `dao store gc` liveness; app-tier reasoning, wrong for OS-capable). GC aligns by construction (jit dir live iff `impl_hash_mod ∈ live_mods`). `triet-jit` stays `triet-pack`-independent: opaque key bytes + injected `trait AotCacheStore`; the caller computes `impl_hash_mod`. Loose runs without the canonical hash → not cached (refuse-over-guess, never fabricate a key). Supersedes Step 1's per-program `emit_object` → per-module (jit.4a).

### v0.11.x.jit — AOT cache implementation (depends on v0.11.0.1 + v0.11.0.2)

- [~] **v0.11.x.jit.3** — AOT cache implementation per ADR-0033. **Steps 0–3 shipped:**
  - Step 0 — generalize IR translator over `Module` trait — `3396f72`
  - Step 1 — object emission (`emit_object`) + version-pinned manifest (§2) — `cbeb102`
  - Step 2 — `Store::install_aot_cache` + `dao store gc` jit sweep (§4/§5/§7) — `2946ce4`
  - Step 3 — Path-A relocating loader `ElfX86_64Loader` (§3 + Addendum constraints 1–4; loader is unsafe-free) — `c47bd8f`
  - [x] **Step 4a** — per-**module** object emission + **load-time linker** (per v0.11.0.2 Entailment). `declare_and_define_module` (own `Export` / cross-module `Import`); `emit_module_object`; two-phase loader (`map_object` + `MappedObject::patch_and_exec`) + `load_program` building a global symbol table. Cross-module 2-module program links + executes → 7. Loader still zero-unsafe. — `52a1cba`
  - [ ] **Step 4b** — wire Path A into `JitDispatcher` via injected `trait AotCacheStore` (opaque `impl_hash_mod` key) + §2 version-check + §8 silent fallback + `cache_state()`; CLI `AotCacheStore` adapter over `Store` + computes `impl_hash_mod`. §9.1 value-parity (cache ≡ VM) + §9.2 version-mismatch refuse. Remove the staged `#![allow(dead_code)]` in `aot.rs` + `loader.rs`.
- [ ] **v0.11.x.jit.4** — Bootstrap gate lift + ≥10× perf bench per ADR-0030 §9 + §14. Lift `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` from `#[ignore]` once warm-cache self-host completes < 10 min (ADR-0033 §9.5 chain). `criterion` warm-vs-cold bench, ≥10× v0.3 baseline target.

### v0.11 backlog (trails AOT cache or later phase)

- **JIT shim gaps:** varargs `FStringConcat`/`TextConcat` (array-ptr+len ABI, ADR-0032 jit.2b-iii Addendum); multi-block-shim codegen (jit.2b-i single-block scope); Ordering-`EnumNew` codegen for end-to-end atomic-function JIT.
- **Borrow checker corpus-driven:** field-granular NLL base, inter-procedural borrow, closure captures, E2403 full owner-trail, Rule-2 elision (`self`-param parser), E2410 field-assign enforcement.
- **Concurrency closures:** `spawn(closure)` Send-bound closure types → real Send-boundary refcount-bump codegen (thread.2) + Triết-source multi-worker (thread.3).
- **`std.concurrency.*` stdlib** (Mutex, Channel, M:N green threads) per ADR-0028 §10 — feature-new scope, separate stdlib phase.

### Workflow note

Per ADR-0009 design-first, **v0.11.0.1 must lock before v0.11.x.jit.3 starts** — the loader approach determines the §3 symbol-resolution mechanism + the unsafe surface. ADR-0032's SHIM_TABLE is the symbol resolution source of truth either way (ADR-0033 §3).
