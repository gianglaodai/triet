# ADR 0033 — AOT cache via `cranelift-object` (backend hybrid + relocation discipline)

**Trạng thái:** **Locked** (v0.10.0.2, author sign-off pending). Refines [ADR-0030 §13](0030-jit-cranelift-integration.md) — locks the 5 design constraints from §13.4 plus backend-hybrid shape so v0.10.x.jit.3 (AOT cache implementation) can ship against settled design. Second v0.10 ADR; depends on [ADR-0032](0032-builtin-shim-abi.md) (shim registry is the symbol resolution source of truth at cache load time).

**Issue:** v0.9 ships in-process JIT only — every `dao run` re-compiles every hot function from scratch (~1-3s per function × ≥100-call-threshold-exceeded set). For self-host bootstrap (~3000 functions), cold-start cost is prohibitive: 9000-9000s ≈ 2.5h on first compile. [ADR-0030 §13.5](0030-jit-cranelift-integration.md) deferred AOT cache to v0.10 because the backend swap (`cranelift-jit` → `cranelift-object`) is structural, not additive. v0.10 closes that.

[ADR-0030 §14](0030-jit-cranelift-integration.md) further chained `.7` (Stage 2 ≡ Stage 3 byte-identical gate lift) and `.8` (perf bench ≥10× v0.3 baseline) to AOT cache — without persistence, bootstrap-loop measurement is dominated by JIT compile cost, masking the actual JIT-vs-VM execution delta.

The 5 constraints ADR-0030 §13.4 surfaced as needing a coherent design pass:

1. **Cranelift version pinning** — cache invalidates on Cranelift bump; how to detect mismatch + recover?
2. **Libcall symbol resolution at load** — shim symbols (`__triet_*` per [ADR-0032 §6](0032-builtin-shim-abi.md)) and Cranelift internal libcalls (`__truncdfsf2` etc.) need re-binding to host process addresses at AOT load time. Mechanism: `libloading`/`dlsym` vs direct registry lookup?
3. **`dao store gc` mark-and-sweep root tracking** — AOT cache directories are children of `impl_hash`; how do they participate in [ADR-0015 §6](0015-package-store-layout.md) GC?
4. **Cross-machine portability** — per-`target_triple` separation per [ADR-0030 §13.3](0030-jit-cranelift-integration.md); on host-mismatch, refuse or fallback?
5. **Determinism preservation** — cache hit/miss state across runs vs [ADR-0007](0007-ir-design.md) IR determinism contract. How is non-determinism scoped?

Open questions ADR-0033 phải lock cùng:

6. **Backend hybrid shape** — keep `cranelift-jit` for fresh compile + add `cranelift-object` for serialize, or fully swap?
7. **Cache write lifecycle** — write-on-compile (synchronous) vs write-on-shutdown (batched)?
8. **Cache corruption recovery** — partial writes, truncated .o files, manifest/object mismatch?
9. **Test gates** — minimum to ship v0.10.x.jit.3 safely.
10. **Self-host port plan** per [ADR-0029 §5](0029-self-host-port-policy.md) — Layer classification?

---

## §1 — Backend hybrid: `cranelift-jit` for fresh, `cranelift-object` for persist

**Decision:** v0.10 adds `cranelift-object` alongside the v0.9 `cranelift-jit` dep — NOT a full swap. Two execution paths share the same Triết IR translator (per [ADR-0030 §3](0030-jit-cranelift-integration.md)):

- **Path A — Cache hit:** AOT load via `cranelift-object` artifact + `object` crate ELF parser + custom relocation patcher. Zero codegen cost; just mmap + relocation + symbol resolve.
- **Path B — Cache miss:** Fresh `cranelift-jit` compile (existing v0.9 path). On success, **also** emit the same IR through `cranelift-object` and persist `functions.o` + `manifest.bin` to `~/.triet/store/jit/{target_triple}/{impl_hash}/`. Next run's cache hit comes from this write.

**Why hybrid, not full swap to `cranelift-object`:**

- **`cranelift-jit` is faster for the fresh-compile path** because it skips ELF emission + relocation table generation — just produces mmap'd RX pages directly. Cache-miss cost matters: bootstrap first-run still happens, and any added per-function overhead multiplies by ~3000 functions.
- **`cranelift-object` is mandatory for serialize** because `cranelift-jit`'s output is in-process-only (raw mmap pointers, no relocation table). Per [ADR-0030 §13.1](0030-jit-cranelift-integration.md) original analysis.
- **Compile twice on cache miss isn't redundant in practice** — Cranelift's IR-to-codegen pipeline is the dominant cost; emitting both an in-process mmap (cranelift-jit) and an .o file (cranelift-object) from the same IR reuses the codegen stage. We pay the ELF/relocation overhead once on cache miss; cache hit on next run skips fresh compile entirely. Net: cache-miss is ~10-20% slower than v0.9 pure-jit (acceptable), cache-hit is ~100× faster (the win).

**Alternative considered — full swap to `cranelift-object` + always-relocate:**
- Pros: single backend, simpler dep tree.
- Cons: every fresh JIT pays relocation patch overhead, even for one-shot programs that never benefit from cache. Slower v0.9-equivalent fresh path; bootstrap first-run regression.
- Rejected per project "don't slow the existing path for a deferred win" stance.

**Cargo deps:**

```toml
# crates/triet-jit/Cargo.toml v0.10:
cranelift-jit    = "..."   # KEEP — Path B fresh compile path
cranelift-object = "..."   # NEW — Path B persistence emission
object           = "..."   # NEW — Path A ELF parse + relocation read
memmap2          = "..."   # NEW — Path A RW→RX mmap for loaded code
```

Cranelift version pinned in workspace `Cargo.toml` per §2.

---

## §2 — Cranelift version pinning + cache invalidation

**Decision:** Each cache entry's `manifest.bin` records two version strings:

```rust
struct AotCacheManifest {
    cranelift_version: String,   // cranelift_codegen::VERSION at write time
    shim_abi_version: u32,       // bump on ADR-0032 ABI break
    target_triple: String,       // redundant with path; defensive cross-check
    function_table: Vec<FunctionEntry>,  // FuncId → symbol name + sig
    libcall_set: Vec<String>,    // sorted libcalls this .o references (§3)
}
```

**On load (Path A):** before patching relocations, read `manifest.bin` and check:

1. `cranelift_version` exact-string-match against current `cranelift_codegen::VERSION`. Mismatch → refuse-load + fall back to Path B fresh compile (overwrites stale .o on success).
2. `shim_abi_version` exact-integer-match against `triet_jit::SHIM_ABI_VERSION` constant. Mismatch → refuse-load + fall back to Path B.
3. `target_triple` exact-string-match against current host triple per §5. Mismatch → refuse-load (should never happen — path enforces this, but defense-in-depth).

**Failure mode unified:** any mismatch → silent fallback to Path B fresh compile. NOT user-visible error. NEW .o overwrites stale (per [ADR-0015 §3](0015-package-store-layout.md) atomic install pattern — tmp/ + rename).

**Why version-in-manifest, not version-in-path:**

- Version-in-path (e.g., `jit/{cranelift_ver}/{triple}/{impl_hash}/`) means concurrent Triết toolchain versions on the same machine accumulate stale caches indefinitely. GC would need version-awareness.
- Version-in-manifest gives a clean upgrade path: new toolchain reads old cache, sees mismatch, overwrites in place. Old cache directories don't accumulate.
- `dao store gc` (§4) still sweeps `jit/{triple}/{impl_hash}/` for any `impl_hash` not in live module set, version-agnostic.

**`SHIM_ABI_VERSION` constant** — single integer in `crates/triet-jit/src/shims.rs`, bumped manually on any ADR-0032 ABI break (e.g., adding new BuiltinName variants, changing hybrid ABI table). Bump = global cache invalidation for any program using affected builtins; matches the semver-style discipline of [ADR-0013](0013-semver-linking-policy.md) `iface_hash`.

**Rejected — silent cache load with potential ABI skew:** would crash randomly in production. Refuse-and-recompile is cheap; safety bar matters more.

---

## §3 — Libcall symbol resolution: direct registry lookup, no `dlsym`

**Decision:** AOT load path resolves undefined symbols from the .o file via **direct `SHIM_TABLE` lookup** ([ADR-0032 §6](0032-builtin-shim-abi.md)) for `__triet_*` shim symbols, plus a parallel `LIBCALL_TABLE` for Cranelift's internal libcalls. NO `libloading`/`dlsym` usage. Symbols stay in the static `triet-jit` binary; the registry holds their Rust function addresses at link time.

**Mechanism:**

```rust
// Path A — AOT load (pseudo):
let object_bytes = std::fs::read(path.join("functions.o"))?;
let obj = object::File::parse(&object_bytes[..])?;

for (sym, reloc_addr) in obj.dynamic_relocations() {
    let target_addr = if let Some(entry) = SHIM_TABLE.iter().find(|e| e.symbol == sym.name()?) {
        entry.addr as usize                              // __triet_* per ADR-0032 §6
    } else if let Some(addr) = LIBCALL_TABLE.lookup(sym.name()?) {
        addr                                              // Cranelift libcall (__truncdfsf2 etc.)
    } else {
        return Err(JitError::UnresolvedSymbol { name: sym.name()? });
    };
    patch_relocation(reloc_addr, target_addr)?;
}
```

**Why direct lookup, not `dlsym`:**

- **`dlsym(RTLD_DEFAULT, "__triet_println")`** requires the symbol to be exported in the host binary's dynamic symbol table. Rust's `#[unsafe(no_mangle)]` exports to the static symbol table but NOT the dynamic table by default — needs `-rdynamic` (`-Wl,--export-dynamic` on Linux) or per-symbol `#[unsafe(export_name = ...)]` + linker flags. Build-system fragility, OS-specific quirks.
- **`dlsym` on Windows** requires `GetProcAddress(GetModuleHandle(NULL), ...)`, with different symbol-name decoration rules.
- **Static `SHIM_TABLE`** (ADR-0032 §6) already exists for the fresh-compile Path B. Reusing it for Path A keeps one source of truth and works portably on Linux/macOS/Windows without build-flag plumbing.

**`LIBCALL_TABLE` for Cranelift internal libcalls.** Cranelift's codegen emits calls to runtime helpers like `__truncdfsf2` (f64→f32 truncation) when implementing certain operations. These come from a small fixed set (~20 functions in `cranelift_codegen::ir::LibCall`). Triết wires them via `JITBuilder::symbol()` at fresh-JIT time (Path B); for Path A, parallel table maps the same names to the same Rust function pointers (typically the compiler-builtins crate's exports, or libc equivalents).

**Concrete list (v0.10):**

```rust
pub(crate) static LIBCALL_TABLE: &[(&str, *const u8)] = &[
    // From cranelift_codegen::ir::LibCall variants — subset Triết IR actually uses.
    // Triết doesn't use floating-point at v0.10, so this list is small.
    // Sticky for v0.10; revisit when v0.11+ adds f64/f32 numerics.
    ("Probestack", probestack_handler as *const u8),
    // (extensible; documented in shims.rs)
];
```

**Symbol-name discipline at write time:** `cranelift-object`'s `ObjectModule::declare_function` accepts a `Linkage` parameter. v0.10 uses `Linkage::Import` for all `__triet_*` and libcall symbols, which marks them as relocation targets in the output .o. Path A's patcher fills these via the table lookup above.

**Rejected — `libloading::Library::new("libtriet_shims.so")`:** would require shipping a separate dynamic library for shims. Distribution complexity (versioning, locating, signing). The shims are part of `triet-jit` static binary; there's no reason to externalize them.

---

## §4 — `dao store gc` integration: jit dirs as `impl_hash` children

**Decision:** Extend [ADR-0015 §6](0015-package-store-layout.md) mark-and-sweep GC to cover `jit/{target_triple}/{impl_hash}/` directories. Mark phase unchanged (manifests give the live `impl_hash` set per existing code). Sweep phase adds a third top-level walk over `jit/` after the existing pkg/mod/term sweeps.

**Concrete extension to `Store::gc`** (per `crates/triet-pack/src/store.rs:413`):

```rust
// After existing mark phase produces live_mods: HashSet<[u8; IMPL_HASH_LEN]>
// (per store.rs:417), add:

if corrupt_pkgs.is_empty() {
    // Conservative mode skip: if any pkg manifest is corrupt, we can't
    // tell which module hashes were referenced, so we skip ALL deriv-
    // ative sweeps (mod, term, jit) per ADR-0015 §6 conservative rule.
    report.swept_jit_dirs += sweep_jit_tree(
        &self.root.join("jit"),
        |triple, hash_bytes| !live_mods.contains(&hash_bytes),
    )?;
}
```

**`sweep_jit_tree`** walks `jit/<triple>/<hash>/` two levels deep:

- Top level: each child is a target_triple directory (`x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, etc.). Walk all — multi-arch caches coexist if user runs on different machines sharing `~/.triet/store/` via NFS/sync.
- Second level: each child is a hash directory. If hex-decode → 32-byte `impl_hash` NOT in `live_mods` set → remove with `remove_path` (existing helper).
- Malformed entries (non-hex names, depth mismatch) best-effort skip per existing pattern.

**`GcReport` extension:**

```rust
pub struct GcReport {
    pub swept_pkgs: usize,
    pub swept_modules: usize,
    pub swept_terms: usize,
    pub swept_name_links: usize,
    pub swept_jit_dirs: usize,   // NEW v0.10
    pub corrupt_pkgs: Vec<ImplHash>,
}
```

**Conservative-on-corruption rule applies uniformly.** If `corrupt_pkgs` non-empty during mark, skip jit sweep along with mod + term sweeps. Same rationale: we can't be sure which JIT artifacts are still reachable.

**Why `impl_hash` keyed, not `iface_hash` keyed:**

- AOT cache contains compiled native code, which depends on impl details (function bodies). `iface_hash` (interface only per [ADR-0011](0011-abi-metadata-format.md)) can stay stable across body changes → cache would silently serve stale code on body-only changes.
- `impl_hash` per [ADR-0014](0014-hash-scheme-refinement.md) changes on any body byte change → cache invalidation aligns with semantic invalidation. Module-level granularity (one .o per module) batches related functions for cache locality + amortizes manifest overhead.

**Rejected — per-function jit cache (`jit/{triple}/{func_hash}/`):** function-level hashes don't exist in Triết's hash tree. Adding them would inflate the hash table without payoff — modules are the natural unit of cache invalidation since IR is already module-batched.

---

## §5 — Cross-machine portability: per-`target_triple` path separation

**Decision:** Cache entries live under `jit/{target_triple}/`, where `target_triple` is the Cranelift triple string for the host (e.g., `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`). Cross-arch load is refused at the path level — Path A only attempts load if `jit/{current_triple}/{impl_hash}/` exists. If user mounts a multi-machine shared `~/.triet/store/` (NFS, dropbox, etc.), each machine gets its own subtree; no cross-pollination.

**`target_triple` source:** computed once at `JitDispatcher::new()` via Cranelift's `cranelift_codegen::isa::lookup` for the host. Single source of truth; stored in `JitDispatcher::host_triple` field for Path A/B reuse.

```rust
let host_isa = cranelift_codegen::isa::lookup_by_name(
    target_lexicon::HOST.to_string().as_str()
)?.finish(settings::Flags::new(settings::builder()))?;
let host_triple_str = host_isa.triple().to_string();
```

**Why path-based, not manifest-based:**

- Path-based makes "does the cache exist for this host?" a single `std::path::Path::exists` check; no manifest parsing required for the negative case (90% of multi-arch shares are negative checks).
- GC traversal (§4) walks `jit/<triple>/<hash>/` two levels uniformly; per-arch separation is structural, not encoded in metadata.
- `manifest.bin` STILL records `target_triple` (§2) for defense-in-depth — catches the case where someone moves a `{impl_hash}/` dir between triple parents manually.

**No cross-arch loading attempted.** Triết does NOT try to "find a cache from another arch and rebuild from there" — fresh compile is cheaper than cross-arch translation. ARM cache on x86 host = ignored, treated as cache miss.

**Multi-machine NFS share scenarios:**

- Two Linux x86_64 machines sharing `~/.triet/store/` → both write to `jit/x86_64-unknown-linux-gnu/` → cache hits both ways. Cranelift version check (§2) ensures correctness if machines have different toolchain versions.
- Linux x86_64 + macOS aarch64 sharing store → separate subtrees (`x86_64-unknown-linux-gnu/` vs `aarch64-apple-darwin/`) → no conflict. Each gets cold cache miss on first run; subsequent runs hit own subtree.

**Rejected — universal IR cache (cache the optimized Cranelift IR before codegen, target-specific codegen on each machine):**
- Pros: cache shareable across arch.
- Cons: adds an IR layer between Triết IR and machine code; we'd cache Cranelift's intermediate, which isn't a stable serialization format (Cranelift doesn't promise IR stability across versions). Defers value to architecture-portability concern that doesn't matter for single-developer-machine usage.
- Rejected for v0.10. Defer post-v1.0 if multi-arch dev becomes common.

---

## §6 — Determinism preservation: cache state is NOT in the contract

**Decision:** Per [ADR-0007](0007-ir-design.md) the determinism contract is **IR-level**: same `.tri` source → same `.triv` bytes. Cache hit/miss state is **runtime-state** — varies across runs (cold first run misses, warm subsequent runs hit) and is NOT subject to determinism guarantees. Bootstrap byte-identical gates (per [ADR-0030 §9](0030-jit-cranelift-integration.md)) compare `.khi` artifact bytes, NOT cache contents or execution time.

**Concretely:**

| Property | Deterministic? | Where verified |
|---|---|---|
| `.tri` source → `.triv` IR bytes | YES — strict | `bootstrap_determinism` test (existing) |
| `.tri` source → Stage 2 ≡ Stage 3 `.khi` bytes | YES — strict | `stage2_eq_stage3_main_tri_byte_identical` (v0.10.x.jit.4 gate lift target) |
| `.tri` source → JIT'd machine code bytes | NO — varies across Cranelift versions, target arch | Not gated; documented per §2 |
| `.tri` source → cache hit/miss on Nth run | NO — runtime state | Not gated; documented per this §6 |
| Same machine, same toolchain version, two runs | Cache hit on run 2 (statistically) | Implicit; not strict-tested (timing-dependent) |
| User-observable output (stdout, stderr, exit code) | YES — strict | Existing program-level tests |

**Why this matters:**

- A reviewer might worry "cache state leaking into determinism" — e.g., "my CI run sometimes hits cache, sometimes misses, output differs?" Resolution: output is identical either way; only timing differs. CI determinism is about output equality, which is preserved.
- Bootstrap gate (`.4`) measures Stage 2 produces same `.khi` as Stage 3. Stage 2 itself runs faster on warm cache, but the `.khi` bytes are identical. Gate passes regardless of cache state.
- Perf benchmark (`.5` or v0.10.x.jit.4 sub-task) measures execution speed; cache state IS part of the measurement (we WANT to measure warm-cache speed). Bench scenarios:
  - "Cold cache" run: `rm -rf ~/.triet/store/jit/` before measure — captures fresh-compile cost.
  - "Warm cache" run: measure after at least one prior run — captures cache-hit cost.
  - Both reported; "warm cache" is the primary target for ≥10× v0.3 baseline claim.

**Test gate impact:** `.4` (Stage 2 ≡ Stage 3 byte-identical lift per ADR-0030 §9) uses `cmp` on `.khi` files. Cache state doesn't affect `.khi` output. Gate stays clean.

**Documentation surface:** `JitDispatcher::cache_state()` returns `Option<CacheStats { hits, misses, evictions }>` for observability — useful for profiling, NOT for correctness assertion. Test code should never depend on `cache_state().hits > 0`.

---

## §7 — Cache write lifecycle: synchronous-on-compile + atomic install

**Decision:** Path B fresh-compile writes its .o + manifest **synchronously immediately after successful JIT compile**, before native execution begins. Uses [ADR-0015 §3](0015-package-store-layout.md) atomic-install pattern (write to `tmp/` dir, rename into final location).

**Pseudo:**

```rust
fn compile_and_cache(&mut self, func_id: FuncId, ir: &IrProgram)
    -> Result<NativeCodePtr, JitError>
{
    // Step 1: Path B fresh JIT compile (existing v0.9 path).
    let native_ptr = self.jit_module.compile(func_id, ir)?;

    // Step 2: emit same IR through cranelift-object → bytes.
    let object_bytes = self.object_module.emit(ir, func_id)?;
    let manifest = AotCacheManifest::new(&ir, /* …per §2… */);
    let manifest_bytes = manifest.serialize();

    // Step 3: atomic install via store.
    self.store.install_aot_cache(
        &self.host_triple,
        &ir.module_impl_hash,
        &object_bytes,
        &manifest_bytes,
    )?;

    Ok(native_ptr)
}
```

**Why synchronous, not lazy/batched:**

- Synchronous keeps the cache eventually-consistent with no `Drop`-time work. Lazy write at `JitDispatcher::drop` would lose state on process crash or `kill -9`; we'd re-pay the compile cost on the next run anyway.
- Per-function write is small (~1-50 KB .o file). Atomic-install overhead is one mkdir + N writes + one rename ≈ <1ms. Negligible compared to the ~1-3s JIT compile.
- Async write to a background thread adds threading complexity (lock-free dispatcher state, write ordering) — premature for v0.10 single-thread VM scope (per [ADR-0026 v2](0026-actor-boundary-send-rules.md) BYOS).

**Write failure handling:**

- If atomic install fails (disk full, permission denied), log a warning + continue execution with the in-process compiled code. Cache write is best-effort: failure doesn't propagate to the user's program. Next run pays compile cost again.
- This matches ADR-0015's `dao install` failure semantics — cache is supplementary.

**Idempotency:** if two `dao run` processes JIT-compile the same module concurrently and race to write, atomic-rename ensures only one wins; the other's tmp dir gets cleaned by the next `dao store gc`'s tmp/ sweep (per `store.rs:474`).

**Rejected — write-on-shutdown via `Drop`:**
- Lost on crash; race conditions with multi-process invocations; complicates shutdown ordering.
- Defer per §7 if profile shows synchronous write becomes a bottleneck (unlikely at <1ms/function).

---

## §8 — Cache corruption recovery: refuse-and-recompile

**Decision:** Any failure during Path A AOT load — corrupt manifest, version mismatch (§2), missing symbols (§3), ELF parse error, RW→RX mprotect failure — triggers **silent fallback to Path B** (fresh compile). The stale cache directory gets overwritten on the next successful Path B persist. No user-visible error; log at `tracing::warn` level for observability.

**Failure modes covered:**

| Failure | Trigger | Recovery |
|---|---|---|
| `manifest.bin` truncated/missing | Disk corruption, killed mid-write | Path A errors → Path B + overwrite |
| `cranelift_version` mismatch | Toolchain upgrade | Path A refuses → Path B + overwrite |
| `shim_abi_version` mismatch | ADR-0032 ABI bump | Path A refuses → Path B + overwrite |
| `functions.o` truncated | Disk corruption | `object::File::parse` errors → Path B + overwrite |
| Unresolved symbol in `__triet_*` | SHIM_TABLE entry removed (would be a v0.10+ ABI break already covered by §2) | Path A errors → Path B + overwrite, on assumption ABI was just bumped |
| `target_triple` mismatch | Manual file copy across machines | Path A refuses → Path B + overwrite |
| RW→RX mprotect fails | Hardened userspace (grsecurity, SELinux) | Path A errors → Path B fresh JIT → also fails per ADR-0030 §6 W^X note → tier-down to VM permanently |

**Why silent fallback, not user-visible error:**

- Cache is supplementary — user's program correctness doesn't depend on it.
- A "cache corrupt" error message during `dao run foo.tri` would confuse non-engineer users (the explicit author audience per CLAUDE.md "Author–AI collaboration model").
- Path B fresh compile + overwrite IS the self-heal action; no manual intervention required.
- Log message at warn level (`tracing::warn!("AOT cache miss for {impl_hash}: {reason}; falling back to fresh JIT")`) lets ops folks/CI investigate without blocking execution.

**Logged reasons:**

- `version mismatch (cache: {old}, current: {new})`
- `corrupt manifest: {parse_error}`
- `unresolved symbol: {symbol_name}`
- `ELF parse failed: {object_error}`
- `mprotect RW→RX denied: {os_error}`

These power post-mortem analysis; structured enough to grep + count for release-check.sh patterns if needed.

**Rejected — strict mode (`TRIET_JIT_STRICT_CACHE=1` env var) that errors on cache miss:** could be added as v0.11 escape hatch for CI scenarios that want to test cache reliability. Not v0.10 scope.

---

## §9 — Test gates: Path A load + Path B persist + GC integration + version invalidation

**Decision:** v0.10.x.jit.3 ships with 4 test categories. Each gate must be green before commit.

### 9.1 — Path A round-trip (cache write + reload + execute)

```rust
#[test]
fn aot_cache_writes_then_reads_back() {
    let tmp_store = TempDir::new()?;
    let mut dispatcher = JitDispatcher::new(&tmp_store)?;
    let func = compile_test_function();

    // First run: cache miss, Path B.
    let result1 = dispatcher.run(&func, &args)?;
    let cache_dir = tmp_store.path().join("jit")
        .join(&dispatcher.host_triple)
        .join(hex_encode(&func.module_impl_hash));
    assert!(cache_dir.join("functions.o").exists());
    assert!(cache_dir.join("manifest.bin").exists());

    // Second run: cache hit, Path A.
    let mut dispatcher2 = JitDispatcher::new(&tmp_store)?;
    let result2 = dispatcher2.run(&func, &args)?;

    assert_eq!(result1, result2);
    assert_eq!(dispatcher2.cache_state().hits, 1);
    assert_eq!(dispatcher2.cache_state().misses, 0);
}
```

### 9.2 — Version mismatch refuses load

```rust
#[test]
fn aot_cache_refuses_on_version_mismatch() {
    // Write a manifest with bogus cranelift_version, attempt load.
    let bogus = AotCacheManifest {
        cranelift_version: "0.0.0-bogus".into(),
        // …other fields valid…
    };
    write_cache(&tmp_store, &bogus, &valid_object_bytes)?;

    let mut dispatcher = JitDispatcher::new(&tmp_store)?;
    let _ = dispatcher.run(&func, &args)?;  // succeeds via Path B fallback
    assert_eq!(dispatcher.cache_state().misses, 1);  // version mismatch counted as miss
    // After Path B, fresh manifest overwrote the bogus one:
    assert_eq!(read_manifest(&tmp_store)?.cranelift_version,
               cranelift_codegen::VERSION);
}
```

Parallel test for `shim_abi_version` mismatch.

### 9.3 — GC sweeps orphan JIT dirs

```rust
#[test]
fn gc_sweeps_orphan_jit_directories() {
    // Install pkg A, run program → creates jit/<triple>/<A_impl_hash>/.
    // Uninstall pkg A (remove from roots), gc.
    // Expect: jit/<triple>/<A_impl_hash>/ removed.

    install_pkg(&store, &pkg_a)?;
    let _ = run_to_warm_cache(&store, &pkg_a);
    remove_root(&store, &pkg_a)?;

    let report = store.gc()?;
    assert_eq!(report.swept_jit_dirs, 1);
    assert!(!jit_dir_for(&store, &pkg_a).exists());
}
```

Plus a conservative-mode test: corrupt manifest → jit sweep skipped along with mod/term.

### 9.4 — Cross-arch directory isolation

```rust
#[test]
fn cross_arch_cache_does_not_cross_pollinate() {
    // Pre-populate jit/<wrong-triple>/<impl_hash>/ with bogus data.
    // Run program on current triple → expect cache miss + Path B compile.
    // Assert: jit/<current-triple>/<impl_hash>/ created; jit/<wrong-triple>/ untouched.
}
```

Verifies §5 path-based separation: wrong-arch directories are invisible to current-arch load attempts.

### 9.5 — Bootstrap warm-cache gate (chained to v0.10.x.jit.4)

This is the gate-lift target per [ADR-0030 §9](0030-jit-cranelift-integration.md) — `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` from `#[ignore]` to CI-required. Requires:

1. v0.10.x.jit.1 (shim infrastructure per ADR-0032) ✓
2. v0.10.x.jit.2 (43 shim impls per ADR-0032) ✓
3. v0.10.x.jit.3 (AOT cache per THIS ADR) ✓
4. AOT-warm Stage 2/3 self-host compile loop completes in < 10 min on dev hardware.

Gate lifted in v0.10.x.jit.4 separate sub-task; ADR-0033 §9.5 documents the dependency chain.

---

## §10 — Self-host port plan (per ADR-0029 §5 template)

**Layer A surface changes:** **No.** AOT cache is internal runtime layer. No SPEC grammar, lexer, parser AST, or user-facing language semantics affected.

**Layer B internal changes:** **No.** Typecheck unchanged. Lowerer unchanged. IR shape unchanged. The `.khi` artifact format is unchanged (cache is separate from `.khi` per ADR-0030 §13.3 layout).

**Layer C runtime changes:** **Yes.** Crates affected:

- `triet-jit` — new file `aot.rs` for Path A loader + cache manifest type; `lib.rs` JitDispatcher integration.
- `triet-jit/Cargo.toml` — new deps `cranelift-object`, `object`, `memmap2`.
- `triet-pack/src/store.rs` — extend `Store::gc` with §4 jit sweep; extend `GcReport` with `swept_jit_dirs`; new `install_aot_cache` method.
- `crates/triet-cli/src/main.rs` — `dao store gc` command picks up the new field automatically; verify report formatting includes `swept_jit_dirs` count.

**Same-phase port required:** **No.** Per [ADR-0029 §3 Layer C independent](0029-self-host-port-policy.md). The self-host compiler at `compiler/*.tri` emits `.khi` containing `Instruction::*` opcodes — AOT cache consumes those at runtime, just like Path B JIT does. Self-host source unchanged.

**Bootstrap interaction:** This is what unlocks the Stage 2/3 byte-identical gate per §9.5. Self-host source code does NOT change; the same Stage 2 / Stage 3 self-compile runs with warm cache after first iteration → completes in <10 min target.

---

## §11 — Implementation sub-task hook (v0.10.x.jit.3)

ADR-0033 unblocks:

- **v0.10.x.jit.3** — AOT cache implementation per this ADR. Implements §1 backend hybrid + §2 manifest with version pinning + §3 SHIM_TABLE/LIBCALL_TABLE symbol resolution + §4 GC integration + §5 cross-arch path separation + §6 determinism documentation + §7 synchronous atomic-install lifecycle + §8 silent fallback + §9.1–9.4 test gates. ~800 LOC + 4 test categories.
- **v0.10.x.jit.4** — Bootstrap gate lift (per ADR-0030 §9 + §14) — depends on §9.5 chain. Lifts `stage2_eq_stage3_main_tri_byte_identical` from `#[ignore]` once §9.5 timing target meets <10 min. NOT this ADR's scope; covered separately.

[ADR-0032](0032-builtin-shim-abi.md) is a hard prerequisite — SHIM_TABLE (ADR-0032 §6) is the single symbol resolution source of truth at AOT load time per §3 of this ADR.

---

## §12 — Decision rationale + connection to ADR-0030

[ADR-0030 §13.6](0030-jit-cranelift-integration.md) deferred AOT cache to v0.10 because shipping it incrementally on top of `cranelift-jit` would have meant either (a) committing to a JIT-only architecture without persistence (which §13.5 already showed is a dead end for bootstrap), or (b) implementing skeleton-cache with v0.9's backend then redoing for v0.10 — exactly the "ship temporary code" anti-pattern author rejected. ADR-0033 is the result of that thought — backend hybrid + relocation discipline locked together so v0.10.x.jit.3 ships against settled design.

**Author-facing summary** (per CLAUDE.md "present tradeoffs in terms the author cares about"):

- **§1 Backend hybrid** = "keep the fast v0.9 fresh-compile path; add a side-channel that writes the same code to disk for next time". One IR, two emission targets.
- **§2 Version pinning** = "cache invalidates cleanly when Triết or Cranelift upgrades; new compile overwrites stale .o in place". No accumulating dead directories.
- **§3 No `dlsym`** = "symbols come from the same `SHIM_TABLE` ADR-0032 already locked; works portably on Linux/macOS/Windows without build-flag plumbing".
- **§4 GC integration** = "JIT cache lives under `~/.triet/store/jit/`; `dao store gc` sweeps it alongside pkg/mod/term — same mark-and-sweep, one more sweep walk".
- **§5 Per-arch directories** = "different machine architecture? different subdirectory. No cross-pollination risk".
- **§6 Determinism preserved** = "cache state doesn't affect program output — only timing. Bootstrap byte-identical gate stays clean".
- **§7–§8 Best-effort cache** = "any failure → fresh compile + overwrite. Cache never blocks a user program from running".

Per [feedback_implementer_choice.md] precedent: 5 constraints are implementation-internal; author delegated 2026-05-30. ADR-0033 records the choice + reasoning so future-AI can reconstruct.

---

## Hệ quả

**Possible (positive):**

- v0.10.x.jit.3 unblocked — mechanical execution against a locked design.
- Self-host bootstrap warm-cache runs in <10 min (vs. ~2.5h cold without cache) per [ADR-0030 §13.5](0030-jit-cranelift-integration.md).
- Stage 2 ≡ Stage 3 byte-identical gate lifted v0.10.x.jit.4 (chained via §9.5) — closes [ADR-0019 §7 Addendum](0019-self-hosting-compiler-bootstrap.md) perf gate that has waited since v0.7.
- Perf bench (v0.10.x.jit.4) can measure warm-cache execution speed honestly — ≥10× v0.3 baseline target per ROADMAP §v0.9 carries forward to v0.10.
- Multi-arch dev shares `~/.triet/store/` cleanly (per-triple subtrees coexist without conflict).
- AOT cache participates in existing GC infrastructure — no new ops procedures.

**Constrained (cost):**

- `triet-jit` adds ~800 LOC for AOT path (Path A loader + manifest type + relocation patcher + GC hook).
- 3 new deps: `cranelift-object`, `object`, `memmap2`. Workspace `Cargo.lock` grows.
- Path B compile cost ~10-20% slower than v0.9 (emits .o alongside in-process mmap). Acceptable; amortized by warm-cache hits on subsequent runs.
- Cache disk usage: ~3000 functions × ~10-50 KB per .o = ~30-150 MB self-host cache footprint. GC keeps this bounded to live modules.
- One new unsafe block in Path A (mprotect RW→RX). SAFETY-documented per [ADR-0032 §5](0032-builtin-shim-abi.md). Workspace `unsafe_code = "forbid"` stays elsewhere.

**Costly (need verify in v0.10.x.jit.3):**

- `cranelift-object` API stability — Cranelift versions 0.108→0.132 have had occasional ObjectModule API churn. Pin Cranelift version in workspace `Cargo.toml`; bump triggers cache invalidation per §2.
- Relocation patcher correctness — manual ELF/Mach-O/COFF relocation handling is error-prone. v0.10 ships ELF-only (Linux/POSIX); Mach-O (macOS) + COFF (Windows) defer to v0.11 cross-platform completion if author prioritizes. Document as POSIX-first per [ADR-0018](0018-capability-loader-semantics.md) precedent.
- Atomic-install concurrency — multiple `dao run` processes racing on cache write. Existing `Store` atomic-rename pattern is correct; verify under stress test in §9.

---

## Không làm (explicitly rejected)

- **Full swap to `cranelift-object` (drop `cranelift-jit`)** — slows fresh-compile path; bootstrap first-run regression. Rejected per §1.
- **Version-in-path cache directories** (`jit/{cranelift_ver}/{triple}/{impl_hash}/`) — accumulates stale directories indefinitely; GC complexity. Rejected per §2.
- **`libloading`/`dlsym` for shim resolution** — requires `-rdynamic` build flag, OS-specific decoration rules, fragile. Rejected per §3.
- **Universal IR cache (cache Cranelift IR pre-codegen, target-codegen per machine)** — defers value to cross-arch dev scenario that doesn't justify v0.10 scope. Rejected per §5.
- **Lazy/batched cache write at process Drop** — lost on crash; race conditions; threading complexity. Rejected per §7.
- **User-visible "cache corrupt" errors** — cache is supplementary; correctness-equivalent recovery via Path B exists. Rejected per §8.
- **Strict cache mode env var (`TRIET_JIT_STRICT_CACHE`)** — defer to v0.11+ if CI use case materializes. Rejected for v0.10 scope.
- **Per-function jit cache granularity** — function-level hashes don't exist in Triết's hash tree. Rejected per §4.
- **Async write to background thread** — premature; <1ms write cost vs. ~1-3s compile cost. Rejected per §7.
- **Mach-O + COFF relocation support v0.10** — POSIX/ELF first per [ADR-0018](0018-capability-loader-semantics.md) precedent; defer cross-platform to v0.11 if author prioritizes.

---

## Prior art

| Source | What we copy | What we change |
|---|---|---|
| `cranelift-object` (Bytecode Alliance) | ELF/.o emission API; relocation table generation | Triết: paired with `cranelift-jit` rather than as sole backend |
| `object` crate | ELF parse + symbol enumeration + relocation read | Triết: custom patcher rather than generic linker |
| Rust `rustc` incremental compilation cache | Per-`crate_hash` cache directory + version invalidation | Triết: per-`impl_hash` (module-level) cache + GC-integrated |
| WasmTime/`wasmer-jit` AOT cache | Object-file persistence + relocation patching | Triết: smaller scope — no Wasm linker, single-target output |
| Java AOT class data sharing (CDS) | Pre-compiled artifact reuse across JVM invocations | Triết: simpler — no class loading, single binary linkable .o |
| Nix store CAS | Content-addressed storage with hash invalidation + GC | Triết: reuses existing `~/.triet/store/` infrastructure per ADR-0015 |
| Ccache (C/C++ compiler cache) | Compiler-output cache keyed on input hash + tool version | Triết: similar version-pinning discipline (§2); narrower scope (one tool) |

**What we invented:**

- **Backend hybrid (Path A `cranelift-object` + Path B `cranelift-jit` sharing one IR translator)** — keeps fresh-compile fast while enabling persistence. Two backends, one codegen pipeline.
- **Shim-registry-as-symbol-table** — `SHIM_TABLE` (ADR-0032 §6) doubles as Path A symbol resolver. No separate symbol export step; no `dlsym` dependency.
- **GC-integrated cache lifecycle** — cache directories are first-class members of [ADR-0015 §6](0015-package-store-layout.md) mark-and-sweep. No separate cache-eviction policy.
- **Best-effort silent-fallback recovery** — cache corruption triggers transparent fresh-recompile + overwrite. Never user-visible.

---

## Tham chiếu

- [ADR-0007](0007-ir-design.md) — IR determinism contract (§6 cache state is NOT in the contract).
- [ADR-0011](0011-abi-metadata-format.md) — ABI metadata + `iface_hash` (§4 explains why `impl_hash` is the cache key, not iface).
- [ADR-0013](0013-semver-linking-policy.md) — Semver linking discipline (§2 SHIM_ABI_VERSION inherits the spirit).
- [ADR-0014](0014-hash-scheme-refinement.md) — 3-cấp hash tree (§4 module-level `impl_hash` is the cache key).
- [ADR-0015 §3 + §6](0015-package-store-layout.md) — Atomic install pattern (§7); mark-and-sweep GC (§4 extension).
- [ADR-0018](0018-capability-loader-semantics.md) — POSIX-first precedent (§Hệ quả costly note for Mach-O/COFF defer).
- [ADR-0019 §7 Addendum](0019-self-hosting-compiler-bootstrap.md) — Perf gate chain (§9.5 unlocks via .4).
- [ADR-0026 v2](0026-actor-boundary-send-rules.md) — BYOS single-threaded v0.10 (§7 rationale for synchronous write).
- [ADR-0029 §3 + §5](0029-self-host-port-policy.md) — Layer C runtime independent (§10 no same-phase port); Self-host port plan template (§10 format).
- [ADR-0030 §5 + §13 + §14](0030-jit-cranelift-integration.md) — Parent AOT cache decision deferred to this ADR; §14 deferral rollup for `.7` + `.8` chained to this ADR.
- [ADR-0032](0032-builtin-shim-abi.md) — Hard prerequisite. SHIM_TABLE (§6) is the symbol resolution table reused here per §3. SHIM_ABI_VERSION (§2) tracks ABI compatibility.
- [VISION §4.3](../../VISION.md) — Multi-backend execution model (JIT/AOT production tier).
- Cranelift docs — `cranelift-object` API + `JITBuilder::symbol` API.
- `object` crate docs — ELF parse + relocation enumeration.
- Bytecode Alliance security model — sandboxing patterns for AOT code load.
