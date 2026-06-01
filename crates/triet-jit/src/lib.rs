//! Triết JIT — Cranelift-backed Tier 2 backend per [ADR-0030].
//!
//! v0.9 JIT subsystem. Sub-task progression per [ADR-0030 §11]:
//!
//! - `.2` — opcode-by-opcode translation (arithmetic / comparisons /
//!   control flow `BrIf` + `BrTrilean` per ADR-0010 backend).
//! - `.3` — call dispatch (`CallLocal` / `CallCrossModule` /
//!   `WitnessCall` per ADR-0012).
//! - `.4` — builtin shim integration → **deferred v0.10** per
//!   [ADR-0030 §12] (`RuntimeValue` ABI marshaling complexity).
//!   Ships only structured diagnostic for tier-down.
//! - `.5` — VM dispatcher integration (call-count trigger + JIT
//!   compile path + native call thunk per [ADR-0030 §2]).
//! - `.6` — AOT cache filesystem layout → **deferred v0.10** per
//!   [ADR-0030 §13] (cranelift-jit → cranelift-object backend
//!   swap). Ships ADR backlog only.
//! - `.7` — bootstrap gate lift → **deferred v0.10** per
//!   [ADR-0030 §14] (chained from §13.5: no AOT cache → 3000-fn
//!   self-host × cold JIT cost prohibitive).
//! - `.8` — perf bench → **deferred v0.10** per [ADR-0030 §14]
//!   (chained from §12: most builtins tier-down → benchmark
//!   understates architectural value; defer to alongside `.4`
//!   completion for honest measurement).
//!
//! # Public API surface
//!
//! [`JitCompiler`] is the primary entry point — compile a
//! [`triet_ir::Function`] into native code, return a pointer suitable
//! for thunking from the VM dispatcher. [`JitError`] enumerates
//! compile failures; on error the VM falls back to bytecode dispatch
//! permanently for that `FuncId` (no retry — per ADR-0030 §2
//! "Tier-down on failure").
//!
//! # Capability gate (per [ADR-0030 Addendum Gap 1])
//!
//! JIT codegen requires `dev.jit_codegen` capability. Default ambient
//! for `usr.*` programs (free JIT). Kernel/embedded programs
//! explicitly `deny` — runtime detects + falls back to VM-only mode.
//! Capability enforcement lands in `.5` VM-dispatcher integration; this
//! scaffold layer doesn't gate.
//!
//! # `unsafe_code` policy
//!
//! Cranelift's `JITModule::finalize_definitions` returns raw function
//! pointers that the dispatcher casts to `extern "C"` callables — this
//! requires `unsafe`. As of `.5`, `dispatch_integer` localizes the
//! transmute + call in a single `#[allow(unsafe_code)]` block with
//! the safety contract documented at the function level. Workspace-
//! wide `unsafe_code = "forbid"` is overridden to `deny` only at this
//! crate's `Cargo.toml` `[lints.rust]` table (not propagated to other
//! crates).
//!
//! [ADR-0030]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §2]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §5]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §11]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §12]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §13]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §14]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 Addendum Gap 1]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0019 §7 Addendum]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md

#![warn(missing_docs)]

mod aot;
mod codegen;
mod loader;
mod shims;

use std::collections::HashMap;

use thiserror::Error;
use triet_ir::{BuiltinName, FuncId, VmError};

use crate::codegen::JitBackend;
use crate::loader::CodeLoader;
pub use crate::shims::SHIM_ABI_VERSION;

/// JIT compiler instance per Triết runtime.
///
/// Owns the Cranelift JIT module + a cache of compiled function
/// pointers indexed by [`FuncId`]. One instance per
/// [`triet_ir::Vm`] (created lazily on first JIT trigger per
/// [ADR-0030 §5] dispatcher integration).
///
/// **v0.9.x.jit.4 status:** `compile` (single-fn) covers arithmetic,
/// comparison, and control-flow opcodes. `compile_program` (multi-fn)
/// additionally resolves `CallLocal`, `CallCrossModule`, and
/// `WitnessCall` (the latter dispatches identically to
/// `CallCrossModule` per ADR-0012 v0.4 semantics), and materializes
/// inline `Operand::Const` against the program's constant pool.
/// `CallBuiltin` raises a name-bearing tier-down diagnostic per
/// ADR-0030 §12 (full shim layer defers v0.10 — `RuntimeValue` ABI
/// marshaling complexity). Closures, aggregates, nullable / outcome
/// wrappers, and the `Long` type also raise
/// [`JitError::UnsupportedOpcode`] so the caller tiers down to
/// VM-only dispatch per ADR-0030 §2.
///
/// [ADR-0030 §5]: ../../../docs/decisions/0030-jit-cranelift-integration.md
pub struct JitCompiler {
    /// Cache of native-code pointers keyed by `FuncId`. Populated on
    /// successful `compile()`; consulted by `lookup()` on dispatch.
    /// The pointer is opaque at this layer — `.5` integration casts
    /// it to the appropriate `extern "C"` calling convention.
    function_cache: HashMap<FuncId, NativeCodePtr>,
    /// Lazily-initialized Cranelift JIT backend. `None` until the
    /// first `compile()` call so failed ISA detection doesn't break
    /// callers that never JIT.
    backend: Option<JitBackend>,
    /// v0.11.x.jit.3 Step 4b — AOT-cache-loaded modules (Path A). Owns
    /// the RX mappings so the addresses in `function_cache` stay valid
    /// for the process. Empty unless a cache hit populated it.
    aot_loaded: Vec<loader::LoadedProgram>,
}

/// Opaque pointer to native machine code.
///
/// Wraps a `*const u8` to keep the public API trait-bound clean
/// (no raw pointer leakage at the type-system level). Dereferenced
/// into a calling-convention-matching `fn` pointer by the VM
/// dispatcher in `.5`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeCodePtr {
    /// Underlying machine-code address. `usize` rather than `*const u8`
    /// to avoid `Send`/`Sync` autoderivation concerns at scaffold layer.
    pub addr: usize,
}

/// JIT compilation errors per [ADR-0030 §2] tier-down policy.
///
/// On error, the VM marks the `FuncId` as JIT-failed and continues
/// dispatching via bytecode. No retry — failure is permanent for the
/// session.
///
/// [ADR-0030 §2]: ../../../docs/decisions/0030-jit-cranelift-integration.md
#[derive(Debug, Error)]
pub enum JitError {
    /// Reserved for future sub-phases that wire entire feature areas
    /// in one shot (e.g. `.4` builtin shim integration, `.5` VM
    /// dispatcher with capability gate). Currently no codegen path
    /// returns this variant — per-opcode failures use
    /// [`JitError::UnsupportedOpcode`] instead. Kept for forward
    /// compatibility with `.4`/`.5` integration.
    #[error("JIT compilation not yet implemented (see ADR-0030 §11)")]
    Unimplemented,

    /// The function uses an IR opcode that the current backend doesn't
    /// handle. Triggers permanent VM-only dispatch for this `FuncId`.
    #[error("unsupported IR opcode for JIT backend: {opcode}")]
    UnsupportedOpcode {
        /// Human-readable opcode name (per `Display for Instruction`).
        opcode: String,
    },

    /// Cranelift-internal error (verification failure, type mismatch
    /// in generated IR, target unsupported, etc.). Treated as
    /// JIT-failed for this session.
    #[error("Cranelift backend error: {message}")]
    Cranelift {
        /// Source error message from Cranelift, opaquely.
        message: String,
    },

    /// Capability gate `dev.jit_codegen` denied — kernel/embedded
    /// program declared `deny` per ADR-0030 Addendum Gap 1. Runtime
    /// falls back to VM-only mode entirely (not just this function).
    #[error("dev.jit_codegen capability denied — running in VM-only mode")]
    CapabilityDenied,

    /// v0.10.x.jit.1 — a `CallBuiltin` opcode names a builtin whose
    /// capability namespace is denied by the program's
    /// capability set. Per [ADR-0032 §3] this is a **compile-time
    /// defense-in-depth** check (the authoritative gate runs at
    /// program-load time per ADR-0016 §5). On this error the function
    /// tiers down to VM dispatch, where the same denial surfaces.
    ///
    /// [ADR-0032 §3]: ../../../docs/decisions/0032-builtin-shim-abi.md
    #[error("builtin `{builtin}` requires capability namespace `{namespace}` (denied)")]
    BuiltinCapabilityDenied {
        /// The builtin whose namespace was denied (Display form).
        builtin: String,
        /// The capability namespace required + denied.
        namespace: String,
    },

    /// v0.11.x.jit.3 (ADR-0033 §8) — AOT cache fault on the Path-A
    /// load side: corrupt/truncated manifest, ELF parse failure,
    /// version-pin mismatch, unresolved symbol, or relocation refusal.
    /// **Never user-visible** — the dispatcher treats it as a cache
    /// miss and silently falls back to a fresh compile, overwriting the
    /// stale entry. `reason` is logged for observability only. Object
    /// **emission** failures (Path-B persist) reuse
    /// [`JitError::Cranelift`]; this variant is the load/parse side.
    #[error("AOT cache unusable ({reason}) — falling back to fresh compile")]
    Cache {
        /// Human-readable cause, for `tracing::warn` + post-mortem.
        reason: String,
    },
}

/// v0.10.x.jit.1 — Look up a [`BuiltinName`]'s capability namespace +
/// test it against a denied-namespace set per [ADR-0032 §3].
///
/// Returns `Err(JitError::BuiltinCapabilityDenied { .. })` when the
/// builtin's namespace appears in `denied`. The JIT codegen consults
/// this before emitting a builtin-shim call (defense-in-depth; the
/// authoritative gate is upstream at program-load time per
/// ADR-0016 §5). Empty `denied` (the production default — capabilities
/// already resolved at load) always returns `Ok`.
///
/// [ADR-0032 §3]: ../../../docs/decisions/0032-builtin-shim-abi.md
pub(crate) fn check_builtin_capability(
    builtin: BuiltinName,
    denied: &[&str],
) -> Result<(), JitError> {
    let namespace = shims::builtin_namespace(builtin);
    if denied.contains(&namespace) {
        return Err(JitError::BuiltinCapabilityDenied {
            builtin: format!("{builtin}"),
            namespace: namespace.to_owned(),
        });
    }
    Ok(())
}

impl JitCompiler {
    /// Construct an empty JIT compiler. Cranelift JIT module is
    /// initialized lazily on first `compile()` call.
    #[must_use]
    pub fn new() -> Self {
        Self {
            function_cache: HashMap::new(),
            backend: None,
            aot_loaded: Vec::new(),
        }
    }

    /// v0.11.x.jit.3 Step 4b (ADR-0033 §2/§3) — Path A: load a whole
    /// program from the AOT cache instead of compiling it fresh.
    ///
    /// `modules` is one `(object_bytes, manifest_bytes)` pair per module
    /// (in `program.modules` order). Every manifest is version-checked
    /// first (§2: `cranelift_version` + [`SHIM_ABI_VERSION`] +
    /// `target_triple`, all exact-match); any mismatch — or any link
    /// failure — returns [`JitError::Cache`] **before** the native cache
    /// is touched, so the caller falls back cleanly to a fresh compile
    /// (§8). On success the per-module objects are linked together (the
    /// load-time linker resolves cross-module symbols + shims) and every
    /// function in their manifests is inserted into `function_cache`.
    ///
    /// # Errors
    /// [`JitError::Cache`] on a corrupt/mismatched manifest or a load
    /// failure — always recoverable via Path B.
    pub(crate) fn load_from_aot(
        &mut self,
        modules: &[(Vec<u8>, Vec<u8>)],
        host_triple: &str,
    ) -> Result<(), JitError> {
        // §2 version pins — check ALL manifests before populating.
        let mut manifests = Vec::with_capacity(modules.len());
        for (_object, manifest_bytes) in modules {
            let manifest = crate::aot::AotCacheManifest::deserialize(manifest_bytes)?;
            if manifest.cranelift_version != cranelift_codegen::VERSION {
                return Err(JitError::Cache {
                    reason: format!(
                        "cranelift version mismatch (cache {}, current {})",
                        manifest.cranelift_version,
                        cranelift_codegen::VERSION
                    ),
                });
            }
            if manifest.shim_abi_version != SHIM_ABI_VERSION {
                return Err(JitError::Cache {
                    reason: format!(
                        "shim ABI version mismatch (cache {}, current {SHIM_ABI_VERSION})",
                        manifest.shim_abi_version
                    ),
                });
            }
            if manifest.target_triple != host_triple {
                return Err(JitError::Cache {
                    reason: format!(
                        "target triple mismatch (cache {}, host {host_triple})",
                        manifest.target_triple
                    ),
                });
            }
            manifests.push(manifest);
        }

        // Link every module object together (cross-module + shim
        // resolution), then map each manifest's functions to addresses.
        let object_refs: Vec<&[u8]> = modules.iter().map(|(o, _)| o.as_slice()).collect();
        // Through the `CodeLoader` trait so the eventual ternary backend
        // swaps in without touching this dispatcher (Addendum constraint 1).
        let loaded =
            loader::ElfX86_64Loader.load_program(&object_refs, &loader::shim_symbol_resolver)?;
        for manifest in &manifests {
            for entry in &manifest.function_table {
                if let Some(addr) = loaded.function_addr(&entry.symbol) {
                    self.function_cache
                        .insert(FuncId(entry.func_id), NativeCodePtr { addr });
                }
            }
        }
        self.aot_loaded.push(loaded);
        Ok(())
    }

    /// Attempt to JIT-compile `func` standalone and return the native
    /// code pointer on success.
    ///
    /// **Use [`Self::compile_program`] instead** when the function
    /// has cross-function calls, witness-table generics, or inline
    /// constants — those need program-level wiring. The single-fn
    /// path uses an empty pool + empty func map and will reject any
    /// such opcode.
    ///
    /// On success, the pointer is also stored in the cache so
    /// [`Self::lookup`] returns the same address.
    ///
    /// # Errors
    ///
    /// - [`JitError::UnsupportedOpcode`] for any IR opcode outside
    ///   the .2 supported set.
    /// - [`JitError::Cranelift`] if Cranelift backend rejects the
    ///   emitted IR (verifier failure, target unsupported, etc.).
    ///
    /// # Panics
    ///
    /// Panics only if backend initialization reports success but the
    /// field is `None` — defensively unreachable via the immediately-
    /// preceding `Some(...)` assignment.
    pub fn compile(&mut self, func: &triet_ir::Function) -> Result<NativeCodePtr, JitError> {
        if self.backend.is_none() {
            self.backend = Some(JitBackend::new()?);
        }
        let backend = self.backend.as_mut().expect("backend just initialized");
        let addr = backend.compile_function(func)?;
        let ptr = NativeCodePtr { addr };
        self.function_cache.insert(func.id, ptr);
        Ok(ptr)
    }

    /// JIT-compile every function in `program` with full
    /// cross-function dispatch wiring per ADR-0030 §3. Functions
    /// that fail (per-function `JitError::UnsupportedOpcode`) are
    /// silently skipped (tier-down per ADR-0030 §2); the cache
    /// only contains successfully-compiled entries.
    ///
    /// Use this entry point (instead of [`Self::compile`]) whenever
    /// the program has cross-function calls, witness-table generics,
    /// or inline constant operands — the single-function path lacks
    /// the program context to resolve any of those.
    ///
    /// # Errors
    ///
    /// - [`JitError::Cranelift`] if the pre-pass (function signature
    ///   declarations) or final `finalize_definitions` fails.
    ///
    /// # Panics
    ///
    /// Panics only if backend initialization reports success but the
    /// field is `None` — defensively unreachable.
    pub fn compile_program(&mut self, program: &triet_ir::IrProgram) -> Result<(), JitError> {
        if self.backend.is_none() {
            self.backend = Some(JitBackend::new()?);
        }
        let backend = self.backend.as_mut().expect("backend just initialized");
        // Production path: empty denied-set. Capability gating is
        // authoritative at program-load time (ADR-0016 §5); the JIT's
        // §3 check is defense-in-depth and only fires when a non-empty
        // denied set is threaded through (see the test-only
        // `compile_program_denied`).
        backend.compile_program(program, &mut self.function_cache, &[])
    }

    /// v0.10.x.jit.1 (test-support) — compile with an explicit
    /// denied-namespace set, exercising the ADR-0032 §3
    /// `BuiltinCapabilityDenied` defense-in-depth path. Production
    /// callers use [`Self::compile_program`] (empty denied-set).
    #[cfg(test)]
    pub(crate) fn compile_program_denied(
        &mut self,
        program: &triet_ir::IrProgram,
        denied: &[&str],
    ) -> Result<(), JitError> {
        if self.backend.is_none() {
            self.backend = Some(JitBackend::new()?);
        }
        let backend = self.backend.as_mut().expect("backend just initialized");
        backend.compile_program(program, &mut self.function_cache, denied)
    }

    /// Return a previously-compiled native code pointer for `id`, or
    /// `None` if not yet JIT'd. Always `None` at scaffold layer.
    #[must_use]
    pub fn lookup(&self, id: FuncId) -> Option<NativeCodePtr> {
        self.function_cache.get(&id).copied()
    }

    /// Return the number of functions currently cached. Used by
    /// internal diagnostics + smoke tests.
    #[must_use]
    pub fn cached_function_count(&self) -> usize {
        self.function_cache.len()
    }

    /// v0.10.x.jit.1 (test-support) — build a backend with `extra_shims`
    /// registered, compile a synthetic caller forwarding to
    /// `shim_symbol`, and cache it under `func_id` so the framework
    /// tests can drive it through [`dispatch_integer_caught`].
    #[cfg(test)]
    fn cache_shim_caller(
        &mut self,
        func_id: FuncId,
        extra_shims: &[shims::ShimEntry],
        caller_sig: &shims::ShimSignature,
        shim_symbol: &str,
        shim_sig: &shims::ShimSignature,
    ) -> Result<(), JitError> {
        let mut backend = JitBackend::new_with_extra_shims(extra_shims)?;
        let addr = backend.compile_shim_caller(caller_sig, shim_symbol, shim_sig)?;
        self.backend = Some(backend);
        self.function_cache.insert(func_id, NativeCodePtr { addr });
        Ok(())
    }
}

impl Default for JitCompiler {
    fn default() -> Self {
        Self::new()
    }
}

/// v0.9.x.jit.5 — Per-process call count threshold for JIT graduation.
///
/// Per [ADR-0030 §2]. Functions hit 100 invocations → dispatcher
/// triggers Cranelift compilation of the entire program. Hotspot
/// JVM convention. Runtime-override via `TRIET_JIT_THRESHOLD` env
/// var (deferred to a follow-up commit; constant for now).
///
/// [ADR-0030 §2]: ../../../docs/decisions/0030-jit-cranelift-integration.md
pub const JIT_THRESHOLD: u32 = 100;

/// One function that tiers down (falls back to VM dispatch) instead of
/// JIT-compiling, with the reason — a single row of a
/// [`JitCoverageReport`].
#[derive(Clone, Debug)]
pub struct JitCoverageEntry {
    /// Raw `triet_ir::FuncId` value.
    pub func_id: u32,
    /// Function name (mangled-source name), if the IR carried one.
    pub name: Option<String>,
    /// Why it tiered down (the `UnsupportedOpcode` / unsupported-signature
    /// message). Embeds the offending opcode / builtin / constant.
    pub reason: String,
}

/// The JIT-coverage measurement for a whole program (v0.11.x Hướng A).
///
/// How many functions JIT, and which ones tier down + why — the
/// measurement that bounds the work to make a program (the self-host
/// compiler) fully JIT-able so the bootstrap byte-identical gate lifts.
#[derive(Clone, Debug)]
pub struct JitCoverageReport {
    /// Total functions attempted (every function in the program).
    pub total: usize,
    /// The functions that tiered down. `total - tier_downs.len()` JIT.
    pub tier_downs: Vec<JitCoverageEntry>,
}

impl JitCoverageReport {
    /// Number of functions that JIT-compile cleanly.
    #[must_use]
    pub const fn jit_able(&self) -> usize {
        self.total - self.tier_downs.len()
    }
}

/// Attempt to JIT every function in `program` and report which tier
/// down + why, **without** executing or finalizing (v0.11.x Hướng A).
///
/// Pure diagnostic: it runs only the opcode-translation stage to surface
/// the coverage gap (see [`crate::codegen::collect_tier_downs`]). On host
/// ISA-detection failure it returns an empty (no-tier-down) report — the
/// audit simply can't run on that host.
#[must_use]
pub fn audit_jit_coverage(program: &triet_ir::IrProgram) -> JitCoverageReport {
    let total = program.modules.iter().map(|m| m.functions.len()).sum();
    let mut report = JitCoverageReport {
        total,
        tier_downs: Vec::new(),
    };
    let Ok(mut backend) = JitBackend::new() else {
        return report;
    };
    for (id, name, reason) in backend.audit(program) {
        report.tier_downs.push(JitCoverageEntry {
            func_id: id.0,
            name,
            reason,
        });
    }
    report
}

/// v0.9.x.jit.5 — Runtime-side JIT integration façade.
///
/// Implements [`triet_ir::JitDispatch`] by wrapping a [`JitCompiler`]
/// plus per-`FuncId` call counters. The Vm installs this via
/// `Vm::set_jit_dispatcher` after construction. The CLI does this
/// when `--no-jit` is absent and `TRIET_JIT` env var doesn't request
/// disable, per ADR-0030 Addendum Gap 3.
///
/// Compilation is **whole-program once** semantics: the first
/// function to cross [`JIT_THRESHOLD`] triggers a single
/// `compile_program` pass that JIT-compiles every eligible function
/// in the program (per ADR-0030 §3 + §11.3 batched-compile model).
/// Subsequent threshold-crossings are no-ops; the cache is
/// populated once and consulted on every subsequent call.
pub struct JitDispatcher {
    /// Underlying Cranelift compiler holding the native code cache.
    compiler: JitCompiler,
    /// Per-`FuncId` call-count counters. Incremented by
    /// [`Self::record_call`]; the first counter to reach
    /// [`JIT_THRESHOLD`] triggers the whole-program compile.
    counters: HashMap<FuncId, u32>,
    /// One-shot guard. `false` until the first threshold-crossing
    /// fires `compile_program`; `true` after (subsequent
    /// `record_call`s skip the compile path).
    compiled: bool,
    /// v0.11.x.jit.3 Step 4b — optional AOT cache backend + per-module
    /// keys. `None` = caching disabled (v0.9/v0.10 behaviour: always
    /// fresh in-process compile). Set via [`Self::enable_aot_cache`].
    aot: Option<AotCache>,
    /// AOT cache hit/miss counters (observability only — ADR-0033 §6).
    cache_state: CacheStats,
}

/// Pluggable AOT cache backend (filesystem I/O) per ADR-0033 §7/§8.
///
/// Injected into the dispatcher so `triet-jit` stays **independent of
/// the package store** (`triet-pack`). Keys are opaque strings: the
/// caller — which owns the store + knows program provenance — derives
/// them (e.g. `hex(impl_hash_mod)` per v0.11.0.2), and a `None` key
/// disables caching for the whole program (refuse-over-guess; never a
/// fabricated key).
pub trait AotCacheStore {
    /// Load `(object_bytes, manifest_bytes)` for `key`, or `None` on a
    /// miss (absent / unreadable entry).
    fn load(&self, key: &str) -> Option<(Vec<u8>, Vec<u8>)>;
    /// Persist `object_bytes` + `manifest_bytes` under `key`.
    /// Best-effort (§8): a write failure must not surface to the running
    /// program — swallow it (the next run simply recompiles).
    fn store(&self, key: &str, object: &[u8], manifest: &[u8]);
}

/// AOT cache hit/miss counters for observability (ADR-0033 §6).
///
/// **Not** a correctness signal: cache state is outside the determinism
/// contract, so tests must never assert on `hits`/`misses` for
/// correctness — only for observability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Path-A whole-program cache hits.
    pub hits: usize,
    /// Compile triggers that fell back to a fresh in-process compile.
    pub misses: usize,
}

/// Injected AOT cache state: the backend, the per-module opaque keys
/// (index == `program.modules` index), and the host triple for the §2
/// version cross-check.
struct AotCache {
    store: Box<dyn AotCacheStore>,
    module_keys: Vec<Option<String>>,
    host_triple: String,
}

impl JitDispatcher {
    /// Construct a fresh dispatcher with no compiled functions and
    /// zeroed counters.
    #[must_use]
    pub fn new() -> Self {
        Self {
            compiler: JitCompiler::new(),
            counters: HashMap::new(),
            compiled: false,
            aot: None,
            cache_state: CacheStats::default(),
        }
    }

    /// Read access to the underlying compiler (for diagnostics, test
    /// inspection, or future capability gate hooks).
    #[must_use]
    pub const fn compiler(&self) -> &JitCompiler {
        &self.compiler
    }

    /// v0.11.x.jit.3 Step 4b — enable the AOT cache (ADR-0033). `store`
    /// is the I/O backend; `module_keys[i]` is the opaque cache key for
    /// `program.modules[i]` (or `None` to mark that module — hence the
    /// whole program — non-cacheable). The caller supplies keys it has
    /// derived from the canonical `impl_hash_mod` (v0.11.0.2); the
    /// dispatcher treats them as opaque.
    pub fn enable_aot_cache(
        &mut self,
        store: Box<dyn AotCacheStore>,
        module_keys: Vec<Option<String>>,
    ) {
        // Host triple for the §2 cross-check. If ISA detection fails the
        // triple is empty → every cached manifest mismatches → caching
        // degrades cleanly to always-fresh-compile.
        let host_triple = crate::codegen::build_host_isa(true)
            .map(|isa| isa.triple().to_string())
            .unwrap_or_default();
        self.aot = Some(AotCache {
            store,
            module_keys,
            host_triple,
        });
    }

    /// AOT cache hit/miss counters (observability — ADR-0033 §6).
    #[must_use]
    pub const fn cache_state(&self) -> CacheStats {
        self.cache_state
    }

    /// Whole-program compile honouring the AOT cache (ADR-0033 §7/§8).
    ///
    /// Path A (cache hit): if every module has a key, try loading all
    /// modules' cached objects; on success the load-time linker maps
    /// them + populates the native cache — zero codegen. Path B (miss,
    /// version mismatch, link failure, or non-cacheable): fresh
    /// in-process compile (the v0.9 path), and — if cacheable — persist
    /// each module's freshly-emitted object for next time. Any Path-A
    /// failure falls through to Path B silently (§8).
    fn compile_program_cached(&mut self, program: &triet_ir::IrProgram) {
        // Cacheable iff caching is on AND every module has a key.
        let keys: Option<Vec<String>> = match &self.aot {
            Some(cache)
                if cache.module_keys.len() == program.modules.len()
                    && cache.module_keys.iter().all(Option::is_some) =>
            {
                Some(
                    cache
                        .module_keys
                        .iter()
                        .map(|k| k.clone().expect("all keys Some"))
                        .collect(),
                )
            }
            _ => None,
        };
        let Some(keys) = keys else {
            // Caching disabled or this program isn't cacheable → fresh.
            let _ = self.compiler.compile_program(program);
            return;
        };

        // ── Path A: read every module's cached entry, then link. ──
        let host_triple = self
            .aot
            .as_ref()
            .expect("cache present")
            .host_triple
            .clone();
        let cached: Option<Vec<(Vec<u8>, Vec<u8>)>> = {
            let store = self.aot.as_ref().expect("cache present").store.as_ref();
            // `collect` into `Option<Vec<_>>` → `None` if any module misses.
            keys.iter().map(|k| store.load(k)).collect()
        };
        if let Some(modules) = cached
            && self.compiler.load_from_aot(&modules, &host_triple).is_ok()
        {
            self.cache_state.hits += 1;
            return;
        }

        // ── Path B: fresh compile, then persist for next run. ──
        let _ = self.compiler.compile_program(program);
        self.cache_state.misses += 1;

        // Only persist when EVERY function compiled. A tier-down
        // (UnsupportedOpcode) leaves a function undefined in its object,
        // so any caller's relocation to it — intra-module (PLT32) or
        // cross-module (GOTPCREL) — is an unresolved symbol the load-time
        // linker refuses; persisting such an object would cache a thing
        // that can never load, paying a parse+mmap+refuse cycle on every
        // future run (permanent churn). Skipping is safe + correct: a
        // not-fully-JIT-able program simply recompiles fresh each run, as
        // it did before the cache existed. (Partial-program warm cache —
        // caching the compiled subset while VM-dispatching the rest — is
        // a jit.4 refinement; it needs the linker to tolerate VM-resident
        // callees, which direct relocations cannot today.)
        let total_funcs: usize = program.modules.iter().map(|m| m.functions.len()).sum();
        if self.compiler.cached_function_count() != total_funcs {
            return;
        }
        let store = self.aot.as_ref().expect("cache present").store.as_ref();
        for (idx, key) in keys.iter().enumerate() {
            if let Ok((object, manifest)) = crate::aot::emit_module_object(program, idx) {
                store.store(key, &object, &manifest.serialize());
            }
        }
    }
}

impl Default for JitDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl triet_ir::JitDispatch for JitDispatcher {
    fn record_call(&mut self, func_id: FuncId, program: &triet_ir::IrProgram) {
        if self.compiled {
            // Counter still increments — useful for diagnostics — but
            // compile path is closed. No-op compile-wise.
            *self.counters.entry(func_id).or_insert(0) += 1;
            return;
        }
        let counter = self.counters.entry(func_id).or_insert(0);
        *counter += 1;
        if *counter >= JIT_THRESHOLD {
            // Best-effort whole-program compile honouring the AOT cache
            // (Path A load else Path B fresh compile + persist). Per-
            // function tier-down (UnsupportedOpcode) is silently absorbed
            // — those functions stay VM-only.
            self.compile_program_cached(program);
            self.compiled = true;
        }
    }

    fn try_dispatch(&self, func_id: FuncId, args: &[i64]) -> Option<i64> {
        dispatch_integer(&self.compiler, func_id, args)
    }
}

/// v0.9.x.jit.5 — Dispatch a JIT-compiled function whose signature is
/// **all-`Integer` params + `Integer` return** (arity 0–4) from the
/// VM. Returns `None` when:
///
/// - The function isn't in the JIT cache (not yet compiled, or
///   compilation failed for this `FuncId`).
/// - The argument count exceeds the supported arity range (0–4 inclusive).
///
/// This is the **single safe-API gateway** for the VM's JIT trigger
/// path. Cranelift returns raw `*const u8` for finalized code and any
/// transmute to an `extern "C" fn` pointer is fundamentally unsafe;
/// this function localizes that unsafe to one auditable site so the
/// VM crate stays under `unsafe_code = "forbid"`.
///
/// # Safety contract (internal — documented for auditability)
///
/// The internal `unsafe { mem::transmute(...) }` is sound iff:
///
/// 1. `jit.lookup(func_id)` returned a pointer to native code that
///    Cranelift compiled with a signature of N `i64` params + `i64`
///    return for some N ≤ 4. The codegen layer guarantees this via
///    [`codegen::map_type`]: `TypeTag::Integer` → `types::I64` always.
/// 2. The function's `JitCompiler::compile_program` succeeded
///    without an `UnsupportedOpcode` tier-down — already implied by
///    the cache hit (failed compiles are never cached).
/// 3. The host platform's calling convention matches Cranelift's
///    `CallConv::SystemV` (or the equivalent Win64) — set in the
///    Cranelift IR at codegen time.
///
/// VM-side caller MUST verify the callee's IR signature is
/// all-Integer before calling this. The
/// [`is_jit_integer_dispatchable`] helper exists for that pre-check.
///
/// [`codegen::map_type`]: crate::codegen
pub fn dispatch_integer(jit: &JitCompiler, func_id: FuncId, args: &[i64]) -> Option<i64> {
    let ptr = jit.lookup(func_id)?;
    if args.len() > 4 {
        return None;
    }
    // SAFETY: see fn-level doc-comment. The transmute is sound under
    // the three invariants enumerated; VM caller is responsible for
    // signature pre-check via `is_jit_integer_dispatchable`.
    #[allow(unsafe_code)]
    let result = unsafe {
        match args.len() {
            0 => {
                let f: extern "C" fn() -> i64 = std::mem::transmute(ptr.addr as *const ());
                f()
            }
            1 => {
                let f: extern "C" fn(i64) -> i64 = std::mem::transmute(ptr.addr as *const ());
                f(args[0])
            }
            2 => {
                let f: extern "C" fn(i64, i64) -> i64 = std::mem::transmute(ptr.addr as *const ());
                f(args[0], args[1])
            }
            3 => {
                let f: extern "C" fn(i64, i64, i64) -> i64 =
                    std::mem::transmute(ptr.addr as *const ());
                f(args[0], args[1], args[2])
            }
            4 => {
                let f: extern "C" fn(i64, i64, i64, i64) -> i64 =
                    std::mem::transmute(ptr.addr as *const ());
                f(args[0], args[1], args[2], args[3])
            }
            // Unreachable per the `if args.len() > 4` guard above.
            _ => return None,
        }
    };
    Some(result)
}

/// v0.10.x.jit.2a — Dispatch an all-`Integer` JIT'd function that may
/// call builtin shims, propagating shim failures as `Err(VmError)` per
/// the [ADR-0032 §4 option-2 resolution].
///
/// Unlike [`dispatch_integer`], this clears the thread-local shim state
/// before the call + reads it after: a shim that fails records a
/// `VmError` + sets `SHIM_FAILED`, the JIT-emitted per-call probe
/// branches the function to its `error_exit` (returning a sentinel),
/// and this dispatcher converts the recorded error into `Err`. No
/// unwinding crosses the Cranelift frame — shims are plain `extern "C"`
/// and return normally.
///
/// Returns:
/// - `Some(Ok(value))` — clean run (no shim set the failure flag).
/// - `Some(Err(vm_error))` — a shim recorded a failure (the function's
///   sentinel return is discarded).
/// - `None` — function not in the JIT cache, or arity > 4.
///
/// # Safety contract
///
/// Identical to [`dispatch_integer`] — the inner transmute is sound
/// under the same three invariants (all-`i64` signature ≤ arity 4;
/// compile success implied by cache hit; host calling convention
/// matches Cranelift's `SystemV`). Shims declare `extern "C"` (never
/// unwind), so no `catch_unwind` is needed and the cranelift-jit 0.132
/// unwind-table gap is sidestepped (ADR-0032 §4 option-2).
///
/// [ADR-0032 §4 option-2]: ../../../docs/decisions/0032-builtin-shim-abi.md
pub fn dispatch_with_shim_errors(
    jit: &JitCompiler,
    func_id: FuncId,
    args: &[i64],
    func_name: &str,
) -> Option<Result<i64, VmError>> {
    let ptr = jit.lookup(func_id)?;
    if args.len() > 4 {
        return None;
    }
    shims::clear_shim_state();
    shims::set_func_name(func_name);
    // SAFETY: see fn-level doc-comment + `dispatch_integer`'s contract.
    // The function + every shim it calls is `extern "C"` and never
    // unwinds, so no panic crosses this frame. Backed by ADR-0032 §4
    // option-2.
    #[allow(unsafe_code)]
    let value = unsafe {
        match args.len() {
            0 => {
                let f: extern "C" fn() -> i64 = std::mem::transmute(ptr.addr as *const ());
                f()
            }
            1 => {
                let f: extern "C" fn(i64) -> i64 = std::mem::transmute(ptr.addr as *const ());
                f(args[0])
            }
            2 => {
                let f: extern "C" fn(i64, i64) -> i64 = std::mem::transmute(ptr.addr as *const ());
                f(args[0], args[1])
            }
            3 => {
                let f: extern "C" fn(i64, i64, i64) -> i64 =
                    std::mem::transmute(ptr.addr as *const ());
                f(args[0], args[1], args[2])
            }
            _ => {
                let f: extern "C" fn(i64, i64, i64, i64) -> i64 =
                    std::mem::transmute(ptr.addr as *const ());
                f(args[0], args[1], args[2], args[3])
            }
        }
    };
    // Shim failure (recorded in TLS) → Err; clean run → Ok(value).
    Some(shims::take_shim_failure().map_or(Ok(value), Err))
}

/// v0.9.x.jit.5 — Pre-check used by [`triet_ir::Vm`] to decide whether
/// a function's signature qualifies for the JIT native-dispatch path.
/// Mirrors the `Integer`-only ABI [`dispatch_integer`] supports.
///
/// Returns `true` iff:
/// - All parameters are `TypeTag::Integer`
/// - The return type is `TypeTag::Integer`
/// - Arity is ≤ 4
///
/// Wider type coverage (Trit / Tryte / Trilean / Long / composites)
/// defers v0.10+ per ADR-0030 §12 backlog (`RuntimeValue` ABI
/// marshaling complexity).
#[must_use]
pub fn is_jit_integer_dispatchable(func: &triet_ir::Function) -> bool {
    if func.params.len() > 4 {
        return false;
    }
    if !matches!(func.return_type, triet_ir::TypeTag::Integer) {
        return false;
    }
    func.params
        .iter()
        .all(|(_, t)| matches!(t, triet_ir::TypeTag::Integer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_new_returns_empty_compiler() {
        let jit = JitCompiler::new();
        assert_eq!(jit.cached_function_count(), 0);
    }

    #[test]
    fn scaffold_default_matches_new() {
        let a = JitCompiler::default();
        let b = JitCompiler::new();
        assert_eq!(a.cached_function_count(), b.cached_function_count());
    }

    #[test]
    fn scaffold_lookup_on_empty_cache_returns_none() {
        let jit = JitCompiler::new();
        assert!(jit.lookup(FuncId(0)).is_none());
    }

    // ===== v0.9.x.jit.2: end-to-end codegen tests =====
    // Build small synthetic IR functions, compile, assert success +
    // non-null pointer. Execution validation defers .5 (requires the
    // VM dispatcher integration + unsafe fn-pointer cast).

    use triet_ir::{BasicBlock, BlockId, Function, Instruction, Operand, TypeTag, ValueId};

    fn make_function(
        name: &str,
        params: Vec<(String, TypeTag)>,
        return_type: TypeTag,
        instructions: Vec<Instruction>,
    ) -> Function {
        let mut block = BasicBlock::new(BlockId(0), Some("entry".to_string()));
        block.instructions = instructions;
        let mut func = Function::new(FuncId(0), Some(name.to_string()), params, return_type);
        func.blocks = vec![block];
        func
    }

    #[test]
    fn jit2_compiles_identity_function() {
        // `id(x: Integer) -> Integer = x` — single Ret of param.
        let func = make_function(
            "id",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let mut jit = JitCompiler::new();
        let ptr = jit.compile(&func).expect("identity compile should succeed");
        assert_ne!(ptr.addr, 0, "native pointer must be non-null");
        assert_eq!(jit.cached_function_count(), 1);
        assert_eq!(jit.lookup(FuncId(0)), Some(ptr));
    }

    #[test]
    fn jit2_compiles_integer_add() {
        // `add(a, b: Integer) -> Integer = a + b`
        let func = make_function(
            "add",
            vec![
                ("a".to_string(), TypeTag::Integer),
                ("b".to_string(), TypeTag::Integer),
            ],
            TypeTag::Integer,
            vec![
                Instruction::Add {
                    dest: ValueId(2),
                    lhs: Operand::Value(ValueId(0)),
                    rhs: Operand::Value(ValueId(1)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let mut jit = JitCompiler::new();
        jit.compile(&func)
            .expect("integer add compile should succeed");
    }

    #[test]
    fn jit2_compiles_integer_sub_mul_neg() {
        // `mix(a, b: Integer) -> Integer = -(a * b - a)`
        let func = make_function(
            "mix",
            vec![
                ("a".to_string(), TypeTag::Integer),
                ("b".to_string(), TypeTag::Integer),
            ],
            TypeTag::Integer,
            vec![
                Instruction::Mul {
                    dest: ValueId(2),
                    lhs: Operand::Value(ValueId(0)),
                    rhs: Operand::Value(ValueId(1)),
                },
                Instruction::Sub {
                    dest: ValueId(3),
                    lhs: Operand::Value(ValueId(2)),
                    rhs: Operand::Value(ValueId(0)),
                },
                Instruction::Neg {
                    dest: ValueId(4),
                    operand: Operand::Value(ValueId(3)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(4))),
                },
            ],
        );
        let mut jit = JitCompiler::new();
        jit.compile(&func)
            .expect("sub/mul/neg compile should succeed");
    }

    #[test]
    fn jit2_compiles_integer_comparison_returns_trilean() {
        // `lt(a, b: Integer) -> Trilean = a < b`
        let func = make_function(
            "lt",
            vec![
                ("a".to_string(), TypeTag::Integer),
                ("b".to_string(), TypeTag::Integer),
            ],
            TypeTag::Trilean,
            vec![
                Instruction::Lt {
                    dest: ValueId(2),
                    lhs: Operand::Value(ValueId(0)),
                    rhs: Operand::Value(ValueId(1)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let mut jit = JitCompiler::new();
        jit.compile(&func)
            .expect("Lt → Trilean compile should succeed");
    }

    #[test]
    fn jit3_const_without_pool_fails_with_missing_const_error() {
        // Single-function path uses an empty constant pool — Const
        // instruction therefore looks up an absent entry. v0.9.x.jit.3
        // surfaces this as a Cranelift-class error. Programs needing
        // constants must use `compile_program`, which threads the
        // real pool.
        let func = make_function(
            "with_const",
            vec![],
            TypeTag::Integer,
            vec![
                Instruction::Const {
                    dest: ValueId(0),
                    constant: triet_ir::ConstId(0),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let mut jit = JitCompiler::new();
        match jit.compile(&func) {
            Err(JitError::Cranelift { message }) => {
                assert!(
                    message.contains("ConstId"),
                    "error must reference missing ConstId, got: {message}"
                );
            }
            other => panic!("expected Cranelift missing-const error, got {other:?}"),
        }
        assert_eq!(
            jit.cached_function_count(),
            0,
            "failed compile must not cache"
        );
    }

    fn make_multi_block_function(
        name: &str,
        params: Vec<(String, TypeTag)>,
        return_type: TypeTag,
        blocks: Vec<(BlockId, Vec<Instruction>)>,
    ) -> Function {
        let mut func = Function::new(FuncId(0), Some(name.to_string()), params, return_type);
        func.blocks = blocks
            .into_iter()
            .map(|(id, instructions)| {
                let mut b = BasicBlock::new(id, None);
                b.instructions = instructions;
                b
            })
            .collect();
        func
    }

    #[test]
    fn jit2_compiles_unconditional_branch() {
        // entry: Br tail
        // tail:  Ret x
        let func = make_multi_block_function(
            "br",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![
                (BlockId(0), vec![Instruction::Br { target: BlockId(1) }]),
                (
                    BlockId(1),
                    vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    }],
                ),
            ],
        );
        let mut jit = JitCompiler::new();
        jit.compile(&func).expect("Br compile should succeed");
    }

    #[test]
    fn jit2_compiles_brif() {
        // entry: BrIf cond, then, else
        // then:  Ret a
        // else:  Ret b
        let func = make_multi_block_function(
            "select",
            vec![
                ("cond".to_string(), TypeTag::Trilean),
                ("a".to_string(), TypeTag::Integer),
                ("b".to_string(), TypeTag::Integer),
            ],
            TypeTag::Integer,
            vec![
                (
                    BlockId(0),
                    vec![Instruction::BrIf {
                        cond: Operand::Value(ValueId(0)),
                        then_block: BlockId(1),
                        else_block: BlockId(2),
                    }],
                ),
                (
                    BlockId(1),
                    vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(1))),
                    }],
                ),
                (
                    BlockId(2),
                    vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(2))),
                    }],
                ),
            ],
        );
        let mut jit = JitCompiler::new();
        jit.compile(&func).expect("BrIf compile should succeed");
    }

    #[test]
    fn jit2_compiles_brtrilean_per_adr0010() {
        // Three-way branch per ADR-0010 §4 backend table.
        //   entry: BrTrilean cond, t, u, f
        //   t/u/f: each Ret with a distinct value
        let func = make_multi_block_function(
            "trit_select",
            vec![
                ("cond".to_string(), TypeTag::Trilean),
                ("vt".to_string(), TypeTag::Integer),
                ("vu".to_string(), TypeTag::Integer),
                ("vf".to_string(), TypeTag::Integer),
            ],
            TypeTag::Integer,
            vec![
                (
                    BlockId(0),
                    vec![Instruction::BrTrilean {
                        cond: Operand::Value(ValueId(0)),
                        true_block: BlockId(1),
                        unknown_block: BlockId(2),
                        false_block: BlockId(3),
                    }],
                ),
                (
                    BlockId(1),
                    vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(1))),
                    }],
                ),
                (
                    BlockId(2),
                    vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(2))),
                    }],
                ),
                (
                    BlockId(3),
                    vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(3))),
                    }],
                ),
            ],
        );
        let mut jit = JitCompiler::new();
        jit.compile(&func)
            .expect("BrTrilean compile should succeed");
    }

    #[test]
    fn jit3_single_fn_call_to_unknown_target_falls_back() {
        // Single-function path has empty func_id_map. CallLocal
        // to any callee fires UnsupportedOpcode "call target FuncId
        // not in program". Use `compile_program` for cross-call
        // dispatch.
        let func = make_function(
            "with_call",
            vec![],
            TypeTag::Integer,
            vec![
                Instruction::CallLocal {
                    dest: Some(ValueId(0)),
                    callee: FuncId(42),
                    args: vec![],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let mut jit = JitCompiler::new();
        match jit.compile(&func) {
            Err(JitError::UnsupportedOpcode { opcode }) => {
                assert!(
                    opcode.contains("FuncId(42)"),
                    "error should name the missing callee, got: {opcode}"
                );
            }
            other => panic!("expected UnsupportedOpcode, got {other:?}"),
        }
    }

    // ===== v0.9.x.jit.3: program-level compilation + call dispatch =====

    use triet_ir::{IrModule, IrProgram};
    use triet_modules::{AbsolutePath, ModulePath};

    fn make_program(modules: Vec<IrModule>, constants: triet_ir::ConstantPool) -> IrProgram {
        IrProgram {
            modules,
            constants,
            witness_tables: Vec::new(),
        }
    }

    fn make_ir_module(module_segments: &[&str], functions: Vec<Function>) -> IrModule {
        let path = AbsolutePath::new(
            ModulePath::new(module_segments.iter().map(|s| (*s).to_string()).collect()),
            String::new(),
        );
        IrModule { path, functions }
    }

    fn make_function_at(
        id: FuncId,
        name: &str,
        params: Vec<(String, TypeTag)>,
        return_type: TypeTag,
        instructions: Vec<Instruction>,
    ) -> Function {
        let mut block = BasicBlock::new(BlockId(0), Some("entry".to_string()));
        block.instructions = instructions;
        let mut func = Function::new(id, Some(name.to_string()), params, return_type);
        func.blocks = vec![block];
        func
    }

    #[test]
    fn jit3_program_with_const_integer() {
        // `function answer() -> Integer = 42` via Const + Ret.
        let mut pool = triet_ir::ConstantPool::new();
        let cid = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(42).unwrap(),
        ));
        let answer = make_function_at(
            FuncId(0),
            "answer",
            vec![],
            TypeTag::Integer,
            vec![
                Instruction::Const {
                    dest: ValueId(0),
                    constant: cid,
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let program = make_program(vec![make_ir_module(&["khi"], vec![answer])], pool);
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("program with Const should compile");
        assert_eq!(jit.cached_function_count(), 1);
        assert!(jit.lookup(FuncId(0)).is_some());
    }

    #[test]
    fn jit4_agg0_dead_code_after_terminator_does_not_panic() {
        // v0.11.x.jit.4.agg.0 — regression for the 10 self-host compiler
        // translator panics: the lowerer emits dead instructions AFTER a
        // terminator within one block (early `return` + appended lexical-
        // block continuation). Cranelift used to panic ("instruction added
        // to a block already filled"); codegen now stops at the
        // terminator. The function must JIT (the dead tail is skipped),
        // returning the value of the early `ret`.
        let mut pool = triet_ir::ConstantPool::new();
        let first = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(1).unwrap(),
        ));
        let dead = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(2).unwrap(),
        ));
        let func = make_function_at(
            FuncId(0),
            "early",
            vec![],
            TypeTag::Integer,
            vec![
                Instruction::Const {
                    dest: ValueId(0),
                    constant: first,
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
                // Dead code after the terminator — must be skipped, not
                // emitted into the now-filled block.
                Instruction::Const {
                    dest: ValueId(1),
                    constant: dead,
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let program = make_program(vec![make_ir_module(&["khi"], vec![func])], pool);
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("dead-code-after-terminator must compile, not panic");
        assert_eq!(jit.cached_function_count(), 1);
        // Executes to the FIRST ret (== 1), proving the dead tail (== 2)
        // was correctly skipped.
        assert_eq!(dispatch_integer(&jit, FuncId(0), &[]), Some(1));
    }

    // ===== v0.11.x.jit.4.agg.1b: boxed codegen mode (struct ops) =====

    /// Recursive structural equality (`RuntimeValue` has no `PartialEq`)
    /// — the value-parity assertion: JIT boxed result == VM result.
    fn assert_rv_eq(a: &triet_ir::RuntimeValue, b: &triet_ir::RuntimeValue) {
        use triet_ir::RuntimeValue as R;
        match (a, b) {
            (R::Integer(x), R::Integer(y)) => assert_eq!(x.to_i64(), y.to_i64(), "{a:?} vs {b:?}"),
            (R::Struct { fields: fa }, R::Struct { fields: fb }) => {
                assert_eq!(fa.len(), fb.len(), "struct arity {a:?} vs {b:?}");
                for (x, y) in fa.iter().zip(fb) {
                    assert_rv_eq(x, y);
                }
            }
            (R::Trilean(x), R::Trilean(y)) => assert_eq!(x, y, "{a:?} vs {b:?}"),
            (R::Trit(x), R::Trit(y)) => assert_eq!(x, y, "{a:?} vs {b:?}"),
            (R::String(x), R::String(y)) => assert_eq!(x, y, "{a:?} vs {b:?}"),
            (
                R::Enum {
                    variant: va,
                    payload: pa,
                },
                R::Enum {
                    variant: vb,
                    payload: pb,
                },
            ) => {
                assert_eq!(va, vb, "enum variant {a:?} vs {b:?}");
                match (pa, pb) {
                    (Some(x), Some(y)) => assert_rv_eq(x, y),
                    (None, None) => {}
                    _ => panic!("enum payload presence mismatch: {a:?} vs {b:?}"),
                }
            }
            (
                R::Outcome {
                    discriminator: da,
                    payload: pa,
                },
                R::Outcome {
                    discriminator: db,
                    payload: pb,
                },
            ) => {
                assert_eq!(da, db, "outcome discriminator {a:?} vs {b:?}");
                match (pa, pb) {
                    (Some(x), Some(y)) => assert_rv_eq(x, y),
                    (None, None) => {}
                    _ => panic!("outcome payload presence mismatch: {a:?} vs {b:?}"),
                }
            }
            (R::Unit, R::Unit) | (R::Null, R::Null) => {}
            _ => panic!("RuntimeValue mismatch: {a:?} vs {b:?}"),
        }
    }

    fn integer(n: i64) -> triet_ir::RuntimeValue {
        triet_ir::RuntimeValue::Integer(triet_core::Integer::new(n).unwrap())
    }

    /// Dispatch a boxed JIT function: box each arg, call via the shared
    /// `dispatch_integer` (the i64-ABI transmute — args/return are boxed
    /// ptrs here), read back the result, drop the arg boxes.
    fn dispatch_boxed(
        jit: &JitCompiler,
        func_id: FuncId,
        args: Vec<triet_ir::RuntimeValue>,
    ) -> triet_ir::RuntimeValue {
        let arg_ptrs: Vec<i64> = args.into_iter().map(shims::box_for_jit_test).collect();
        let result_ptr = dispatch_integer(jit, func_id, &arg_ptrs).expect("boxed dispatch");
        let result = shims::read_for_jit_test(result_ptr);
        shims::drop_for_jit_test(result_ptr);
        for p in arg_ptrs {
            shims::drop_for_jit_test(p);
        }
        result
    }

    #[test]
    fn jit4_agg1b_boxed_struct_ops_value_parity() {
        // make(x, y) -> Struct{x, y}     (StructNew)
        // first(p)   -> p.field(0)       (FieldGet)
        // set0(p, v) -> {v, p.field(1)}  (FieldSet)
        // All boxed (Bậc A). Assert the JIT result equals the VM result.
        let make = make_function_at(
            FuncId(0),
            "make",
            vec![
                ("x".into(), TypeTag::Integer),
                ("y".into(), TypeTag::Integer),
            ],
            TypeTag::Unit, // struct → Unit placeholder (the TypeTag lies)
            vec![
                Instruction::StructNew {
                    dest: ValueId(2),
                    fields: vec![Operand::Value(ValueId(0)), Operand::Value(ValueId(1))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let first = make_function_at(
            FuncId(1),
            "first",
            vec![("p".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                Instruction::FieldGet {
                    dest: ValueId(1),
                    object: Operand::Value(ValueId(0)),
                    field_idx: 0,
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let set0 = make_function_at(
            FuncId(2),
            "set0",
            vec![("p".into(), TypeTag::Unit), ("v".into(), TypeTag::Integer)],
            TypeTag::Unit,
            vec![
                Instruction::FieldSet {
                    dest: ValueId(2),
                    object: Operand::Value(ValueId(0)),
                    field_idx: 0,
                    value: Operand::Value(ValueId(1)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![make, first, set0])],
            triet_ir::ConstantPool::new(),
        );

        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("boxed struct functions must compile");
        // All three are boxed-JIT-able (no tier-down).
        assert_eq!(jit.cached_function_count(), 3);

        // make(7, 9) — JIT vs VM.
        let jit_made = dispatch_boxed(&jit, FuncId(0), vec![integer(7), integer(9)]);
        let mut vm = triet_ir::Vm::new(program.clone());
        let vm_made = vm
            .execute(FuncId(0), vec![integer(7), integer(9)])
            .expect("vm make");
        assert_rv_eq(&jit_made, &vm_made);
        // ...and it is the struct {7, 9}.
        assert_rv_eq(
            &jit_made,
            &triet_ir::RuntimeValue::Struct {
                fields: vec![integer(7), integer(9)],
            },
        );

        // first({7, 9}) == 7 — JIT vs VM.
        let point = triet_ir::RuntimeValue::Struct {
            fields: vec![integer(7), integer(9)],
        };
        let jit_first = dispatch_boxed(&jit, FuncId(1), vec![point.clone()]);
        let mut vm2 = triet_ir::Vm::new(program.clone());
        let vm_first = vm2
            .execute(FuncId(1), vec![point.clone()])
            .expect("vm first");
        assert_rv_eq(&jit_first, &vm_first);
        assert_rv_eq(&jit_first, &integer(7));

        // set0({7, 9}, 42) == {42, 9} — JIT vs VM.
        let jit_set = dispatch_boxed(&jit, FuncId(2), vec![point.clone(), integer(42)]);
        let mut vm3 = triet_ir::Vm::new(program);
        let vm_set = vm3
            .execute(FuncId(2), vec![point, integer(42)])
            .expect("vm set");
        assert_rv_eq(&jit_set, &vm_set);
        assert_rv_eq(
            &jit_set,
            &triet_ir::RuntimeValue::Struct {
                fields: vec![integer(42), integer(9)],
            },
        );
    }

    #[test]
    fn jit4_agg1c_boxed_arithmetic_and_comparison_value_parity() {
        // sum(p)  -> p.field(0) + p.field(1)   (FieldGet + Add, boxed)
        // gt(p)   -> p.field(0) > p.field(1)   (FieldGet + Gt → Trilean)
        // Boxed because of FieldGet; the arithmetic/comparison now JIT too
        // (agg.1c) instead of tiering down. Assert == VM.
        let sum = make_function_at(
            FuncId(0),
            "sum",
            vec![("p".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                Instruction::FieldGet {
                    dest: ValueId(1),
                    object: Operand::Value(ValueId(0)),
                    field_idx: 0,
                },
                Instruction::FieldGet {
                    dest: ValueId(2),
                    object: Operand::Value(ValueId(0)),
                    field_idx: 1,
                },
                Instruction::Add {
                    dest: ValueId(3),
                    lhs: Operand::Value(ValueId(1)),
                    rhs: Operand::Value(ValueId(2)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(3))),
                },
            ],
        );
        let gt = make_function_at(
            FuncId(1),
            "gt",
            vec![("p".into(), TypeTag::Unit)],
            TypeTag::Trilean,
            vec![
                Instruction::FieldGet {
                    dest: ValueId(1),
                    object: Operand::Value(ValueId(0)),
                    field_idx: 0,
                },
                Instruction::FieldGet {
                    dest: ValueId(2),
                    object: Operand::Value(ValueId(0)),
                    field_idx: 1,
                },
                Instruction::Gt {
                    dest: ValueId(3),
                    lhs: Operand::Value(ValueId(1)),
                    rhs: Operand::Value(ValueId(2)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(3))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![sum, gt])],
            triet_ir::ConstantPool::new(),
        );

        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("boxed arithmetic/comparison must compile");
        assert_eq!(
            jit.cached_function_count(),
            2,
            "both fns JIT (no tier-down)"
        );

        let point = || triet_ir::RuntimeValue::Struct {
            fields: vec![integer(7), integer(9)],
        };

        // sum({7,9}) == 16, == VM.
        let jit_sum = dispatch_boxed(&jit, FuncId(0), vec![point()]);
        let mut vm = triet_ir::Vm::new(program.clone());
        let vm_sum = vm.execute(FuncId(0), vec![point()]).expect("vm sum");
        assert_rv_eq(&jit_sum, &vm_sum);
        assert_rv_eq(&jit_sum, &integer(16));

        // gt({7,9}) == False (7 > 9), == VM.
        let jit_gt = dispatch_boxed(&jit, FuncId(1), vec![point()]);
        let mut vm2 = triet_ir::Vm::new(program);
        let vm_gt = vm2.execute(FuncId(1), vec![point()]).expect("vm gt");
        // Both are Trilean — compare via the boxed read.
        match (&jit_gt, &vm_gt) {
            (triet_ir::RuntimeValue::Trilean(a), triet_ir::RuntimeValue::Trilean(b)) => {
                assert_eq!(a, b, "gt parity");
                assert_eq!(*a, triet_logic::Trilean::False);
            }
            _ => panic!("expected Trilean from gt, got {jit_gt:?} / {vm_gt:?}"),
        }
    }

    #[test]
    fn jit4_agg1c_boxed_inline_constant_value_parity() {
        // add100(p) -> p.field(0) + 100   (FieldGet → boxed; inline Const
        // 100 materialized via __triet_box_const). Assert == VM.
        let mut pool = triet_ir::ConstantPool::new();
        let hundred = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(100).unwrap(),
        ));
        let add100 = make_function_at(
            FuncId(0),
            "add100",
            vec![("p".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                Instruction::FieldGet {
                    dest: ValueId(1),
                    object: Operand::Value(ValueId(0)),
                    field_idx: 0,
                },
                Instruction::Add {
                    dest: ValueId(2),
                    lhs: Operand::Value(ValueId(1)),
                    rhs: Operand::Const(hundred),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let program = make_program(vec![make_ir_module(&["khi"], vec![add100])], pool);

        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("boxed inline-constant fn must compile");
        assert_eq!(jit.cached_function_count(), 1);

        let point = || triet_ir::RuntimeValue::Struct {
            fields: vec![integer(7), integer(9)],
        };
        let jit_r = dispatch_boxed(&jit, FuncId(0), vec![point()]);
        let mut vm = triet_ir::Vm::new(program);
        let vm_r = vm.execute(FuncId(0), vec![point()]).expect("vm add100");
        assert_rv_eq(&jit_r, &vm_r);
        assert_rv_eq(&jit_r, &integer(107));
    }

    #[test]
    fn jit4_agg1c_cross_mode_call_tiers_down_caller() {
        // Safety guard (ADR-0034 Addendum): an unboxed (all-Integer)
        // caller calling a BOXED callee would pass a raw i64 where the
        // callee expects a boxed ptr (same width → Cranelift can't catch
        // it). The caller must tier down; the boxed callee still JITs.
        //   make(x) -> Struct{x}              (boxed)
        //   caller(x: Integer) -> Integer = { make(x); x }  (would call
        //     boxed make → must tier down; left as call + ret x)
        let make = make_function_at(
            FuncId(0),
            "make",
            vec![("x".into(), TypeTag::Integer)],
            TypeTag::Unit,
            vec![
                Instruction::StructNew {
                    dest: ValueId(1),
                    fields: vec![Operand::Value(ValueId(0))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let caller = make_function_at(
            FuncId(1),
            "caller",
            vec![("x".into(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![
                Instruction::CallLocal {
                    dest: Some(ValueId(1)),
                    callee: FuncId(0),
                    args: vec![Operand::Value(ValueId(0))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![make, caller])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        // `make` (boxed) JITs; `caller` tiers down on the cross-mode call.
        assert!(jit.lookup(FuncId(0)).is_some(), "boxed make must JIT");
        assert!(
            jit.lookup(FuncId(1)).is_none(),
            "unboxed caller of a boxed callee must tier down (no cross-mode miscompile)"
        );
    }

    #[test]
    fn jit4_agg1c_boxed_to_boxed_call_value_parity() {
        // agg.1c-iii: a boxed function calling another boxed function
        // (same-mode boxed→boxed). Both pass/return boxed ptrs.
        //   inner(p) -> p.field(0)                          (boxed: FieldGet)
        //   outer(p) -> { inner(p), p.field(1) }            (boxed: StructNew
        //                + CallLocal to inner + FieldGet)
        // outer({7,9}) == {7, 9}; assert JIT == VM.
        let inner = make_function_at(
            FuncId(0),
            "inner",
            vec![("p".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                Instruction::FieldGet {
                    dest: ValueId(1),
                    object: Operand::Value(ValueId(0)),
                    field_idx: 0,
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let outer = make_function_at(
            FuncId(1),
            "outer",
            vec![("p".into(), TypeTag::Unit)],
            TypeTag::Unit,
            vec![
                Instruction::CallLocal {
                    dest: Some(ValueId(1)),
                    callee: FuncId(0),
                    args: vec![Operand::Value(ValueId(0))],
                },
                Instruction::FieldGet {
                    dest: ValueId(2),
                    object: Operand::Value(ValueId(0)),
                    field_idx: 1,
                },
                Instruction::StructNew {
                    dest: ValueId(3),
                    fields: vec![Operand::Value(ValueId(1)), Operand::Value(ValueId(2))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(3))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![inner, outer])],
            triet_ir::ConstantPool::new(),
        );

        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("boxed→boxed call must compile");
        assert_eq!(
            jit.cached_function_count(),
            2,
            "both boxed fns JIT (boxed call, no tier-down)"
        );

        let point = || triet_ir::RuntimeValue::Struct {
            fields: vec![integer(7), integer(9)],
        };
        let jit_r = dispatch_boxed(&jit, FuncId(1), vec![point()]);
        let mut vm = triet_ir::Vm::new(program);
        let vm_r = vm.execute(FuncId(1), vec![point()]).expect("vm outer");
        assert_rv_eq(&jit_r, &vm_r);
        assert_rv_eq(
            &jit_r,
            &triet_ir::RuntimeValue::Struct {
                fields: vec![integer(7), integer(9)],
            },
        );
    }

    #[test]
    fn jit4_agg1c_boxed_brif_multiblock_value_parity() {
        // agg.1c-iv: a multi-block BOXED function branching on a boxed
        // Trilean cond (`__triet_trilean_tag` → icmp). No Phi (each arm
        // has its own Ret), so it JITs.
        //   max(p) -> { if p.field(0) > p.field(1) then p.field(0)
        //               else p.field(1) }
        let max = make_multi_block_function(
            "max",
            vec![("p".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                (
                    BlockId(0),
                    vec![
                        Instruction::FieldGet {
                            dest: ValueId(1),
                            object: Operand::Value(ValueId(0)),
                            field_idx: 0,
                        },
                        Instruction::FieldGet {
                            dest: ValueId(2),
                            object: Operand::Value(ValueId(0)),
                            field_idx: 1,
                        },
                        Instruction::Gt {
                            dest: ValueId(3),
                            lhs: Operand::Value(ValueId(1)),
                            rhs: Operand::Value(ValueId(2)),
                        },
                        Instruction::BrIf {
                            cond: Operand::Value(ValueId(3)),
                            then_block: BlockId(1),
                            else_block: BlockId(2),
                        },
                    ],
                ),
                (
                    BlockId(1),
                    vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(1))),
                    }],
                ),
                (
                    BlockId(2),
                    vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(2))),
                    }],
                ),
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![max])],
            triet_ir::ConstantPool::new(),
        );

        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("multi-block boxed BrIf must compile");
        assert_eq!(jit.cached_function_count(), 1, "boxed BrIf fn JITs");

        for (a, b, expected) in [(7, 9, 9), (9, 7, 9), (5, 5, 5)] {
            let point = triet_ir::RuntimeValue::Struct {
                fields: vec![integer(a), integer(b)],
            };
            let jit_r = dispatch_boxed(&jit, FuncId(0), vec![point.clone()]);
            let mut vm = triet_ir::Vm::new(program.clone());
            let vm_r = vm.execute(FuncId(0), vec![point]).expect("vm max");
            assert_rv_eq(&jit_r, &vm_r);
            assert_rv_eq(&jit_r, &integer(expected));
        }
    }

    #[test]
    fn jit4_agg1c_boxed_brtrilean_multiblock_value_parity() {
        // agg.1c-iv: a multi-block BOXED function with a three-way
        // BrTrilean on a boxed Trilean field. Each arm returns a distinct
        // boxed Integer const (no Phi). Exercises all three tag values.
        //   classify(p) -> match p.field(0): True=>1, Unknown=>0, False=>-1
        let mut pool = triet_ir::ConstantPool::new();
        let pos = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(1).unwrap(),
        ));
        let zero = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(0).unwrap(),
        ));
        let neg = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(-1).unwrap(),
        ));
        let classify = make_multi_block_function(
            "classify",
            vec![("p".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                (
                    BlockId(0),
                    vec![
                        Instruction::FieldGet {
                            dest: ValueId(1),
                            object: Operand::Value(ValueId(0)),
                            field_idx: 0,
                        },
                        Instruction::BrTrilean {
                            cond: Operand::Value(ValueId(1)),
                            true_block: BlockId(1),
                            unknown_block: BlockId(2),
                            false_block: BlockId(3),
                        },
                    ],
                ),
                (
                    BlockId(1),
                    vec![
                        Instruction::Const {
                            dest: ValueId(2),
                            constant: pos,
                        },
                        Instruction::Ret {
                            value: Some(Operand::Value(ValueId(2))),
                        },
                    ],
                ),
                (
                    BlockId(2),
                    vec![
                        Instruction::Const {
                            dest: ValueId(3),
                            constant: zero,
                        },
                        Instruction::Ret {
                            value: Some(Operand::Value(ValueId(3))),
                        },
                    ],
                ),
                (
                    BlockId(3),
                    vec![
                        Instruction::Const {
                            dest: ValueId(4),
                            constant: neg,
                        },
                        Instruction::Ret {
                            value: Some(Operand::Value(ValueId(4))),
                        },
                    ],
                ),
            ],
        );
        let program = make_program(vec![make_ir_module(&["khi"], vec![classify])], pool);

        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("multi-block boxed BrTrilean must compile");
        assert_eq!(jit.cached_function_count(), 1);

        for (tri, expected) in [
            (triet_logic::Trilean::True, 1),
            (triet_logic::Trilean::Unknown, 0),
            (triet_logic::Trilean::False, -1),
        ] {
            let point = triet_ir::RuntimeValue::Struct {
                fields: vec![triet_ir::RuntimeValue::Trilean(tri)],
            };
            let jit_r = dispatch_boxed(&jit, FuncId(0), vec![point.clone()]);
            let mut vm = triet_ir::Vm::new(program.clone());
            let vm_r = vm.execute(FuncId(0), vec![point]).expect("vm classify");
            assert_rv_eq(&jit_r, &vm_r);
            assert_rv_eq(&jit_r, &integer(expected));
        }
    }

    #[test]
    fn jit4_agg1c_boxed_phi_merge_value_parity() {
        // agg.1c-v: a boxed φ merging values computed in SIBLING blocks
        // (neither dominates the join → only Cranelift block params can
        // express it). Exercises BrIf + Br + Phi + boxed Add/Neg.
        //   f(p) -> { v = p.field(0);
        //             r = if v > 0 then v + 10 else -v;
        //             r }
        let mut pool = triet_ir::ConstantPool::new();
        let zero = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(0).unwrap(),
        ));
        let ten = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(10).unwrap(),
        ));
        let f = make_multi_block_function(
            "incr_or_negate",
            vec![("p".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                (
                    BlockId(0),
                    vec![
                        Instruction::FieldGet {
                            dest: ValueId(1),
                            object: Operand::Value(ValueId(0)),
                            field_idx: 0,
                        },
                        Instruction::Gt {
                            dest: ValueId(2),
                            lhs: Operand::Value(ValueId(1)),
                            rhs: Operand::Const(zero),
                        },
                        Instruction::BrIf {
                            cond: Operand::Value(ValueId(2)),
                            then_block: BlockId(1),
                            else_block: BlockId(2),
                        },
                    ],
                ),
                (
                    BlockId(1),
                    vec![
                        Instruction::Add {
                            dest: ValueId(3),
                            lhs: Operand::Value(ValueId(1)),
                            rhs: Operand::Const(ten),
                        },
                        Instruction::Br { target: BlockId(3) },
                    ],
                ),
                (
                    BlockId(2),
                    vec![
                        Instruction::Neg {
                            dest: ValueId(4),
                            operand: Operand::Value(ValueId(1)),
                        },
                        Instruction::Br { target: BlockId(3) },
                    ],
                ),
                (
                    BlockId(3),
                    vec![
                        Instruction::Phi {
                            dest: ValueId(5),
                            incoming: vec![
                                triet_ir::PhiIncoming {
                                    value: ValueId(3),
                                    block: BlockId(1),
                                },
                                triet_ir::PhiIncoming {
                                    value: ValueId(4),
                                    block: BlockId(2),
                                },
                            ],
                        },
                        Instruction::Ret {
                            value: Some(Operand::Value(ValueId(5))),
                        },
                    ],
                ),
            ],
        );
        let program = make_program(vec![make_ir_module(&["khi"], vec![f])], pool);

        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("boxed φ-merge fn must compile");
        assert_eq!(jit.cached_function_count(), 1, "boxed φ fn JITs");

        for (v, expected) in [(5, 15), (-3, 3), (0, 0)] {
            // v=0: 0 > 0 is False → else arm → -0 = 0.
            let point = triet_ir::RuntimeValue::Struct {
                fields: vec![integer(v)],
            };
            let jit_r = dispatch_boxed(&jit, FuncId(0), vec![point.clone()]);
            let mut vm = triet_ir::Vm::new(program.clone());
            let vm_r = vm.execute(FuncId(0), vec![point]).expect("vm φ");
            assert_rv_eq(&jit_r, &vm_r);
            assert_rv_eq(&jit_r, &integer(expected));
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn jit4_agg2a_boxed_enum_ops_value_parity() {
        // agg.2a: boxed enum ops. A struct payload exercises the
        // payload-preservation gotcha (triet_enum_struct_payload_identity).
        //   wrap(v)  -> EnumNew variant=1, payload=v   (Some payload)
        //   unit()   -> EnumNew variant=0              (no payload)
        //   tag(e)   -> EnumTag e                      (→ Integer)
        //   inner(e) -> EnumPayload e                  (→ payload)
        let wrap = make_function_at(
            FuncId(0),
            "wrap",
            vec![("v".into(), TypeTag::Integer)],
            TypeTag::Unit,
            vec![
                Instruction::EnumNew {
                    dest: ValueId(1),
                    variant_idx: 1,
                    payload: Some(Operand::Value(ValueId(0))),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let unit = make_function_at(
            FuncId(1),
            "unit",
            vec![],
            TypeTag::Unit,
            vec![
                Instruction::EnumNew {
                    dest: ValueId(0),
                    variant_idx: 0,
                    payload: None,
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let tag = make_function_at(
            FuncId(2),
            "tag",
            vec![("e".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                Instruction::EnumTag {
                    dest: ValueId(1),
                    scrutinee: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let inner = make_function_at(
            FuncId(3),
            "inner",
            vec![("e".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                Instruction::EnumPayload {
                    dest: ValueId(1),
                    scrutinee: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![wrap, unit, tag, inner])],
            triet_ir::ConstantPool::new(),
        );

        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("boxed enum ops must compile");
        assert_eq!(jit.cached_function_count(), 4, "all 4 boxed enum fns JIT");

        // wrap(42) == Enum{variant:1, payload:42} — JIT vs VM.
        let jit_wrap = dispatch_boxed(&jit, FuncId(0), vec![integer(42)]);
        let mut vm = triet_ir::Vm::new(program.clone());
        let vm_wrap = vm.execute(FuncId(0), vec![integer(42)]).expect("vm wrap");
        assert_rv_eq(&jit_wrap, &vm_wrap);
        match &jit_wrap {
            triet_ir::RuntimeValue::Enum {
                variant,
                payload: Some(p),
            } => {
                assert_eq!(*variant, 1);
                assert_rv_eq(p, &integer(42));
            }
            _ => panic!("expected Enum{{1, Some(42)}}, got {jit_wrap:?}"),
        }

        // unit() == Enum{variant:0, payload:None}.
        let jit_unit = dispatch_boxed(&jit, FuncId(1), vec![]);
        let mut vm2 = triet_ir::Vm::new(program.clone());
        let vm_unit = vm2.execute(FuncId(1), vec![]).expect("vm unit");
        assert_rv_eq(&jit_unit, &vm_unit);
        assert!(
            matches!(
                &jit_unit,
                triet_ir::RuntimeValue::Enum {
                    variant: 0,
                    payload: None
                }
            ),
            "got {jit_unit:?}"
        );

        // tag(wrap(42)) == 1; inner(wrap(42)) == 42 — JIT vs VM.
        let e = triet_ir::RuntimeValue::Enum {
            variant: 1,
            payload: Some(Box::new(integer(42))),
        };
        let jit_tag = dispatch_boxed(&jit, FuncId(2), vec![e.clone()]);
        let mut vm3 = triet_ir::Vm::new(program.clone());
        let vm_tag = vm3.execute(FuncId(2), vec![e.clone()]).expect("vm tag");
        assert_rv_eq(&jit_tag, &vm_tag);
        assert_rv_eq(&jit_tag, &integer(1));

        let jit_inner = dispatch_boxed(&jit, FuncId(3), vec![e.clone()]);
        let mut vm4 = triet_ir::Vm::new(program);
        let vm_inner = vm4.execute(FuncId(3), vec![e]).expect("vm inner");
        assert_rv_eq(&jit_inner, &vm_inner);
        assert_rv_eq(&jit_inner, &integer(42));
    }

    #[test]
    fn jit4_agg2a_boxed_enum_struct_payload_identity() {
        // The triet_enum_struct_payload_identity gotcha as a JIT==VM check:
        // an enum carrying a STRUCT payload must round-trip its fields
        // through EnumNew → EnumPayload without loss.
        //   rewrap(s) -> EnumPayload(EnumNew variant=2, payload=s)
        let rewrap = make_function_at(
            FuncId(0),
            "rewrap",
            vec![("s".into(), TypeTag::Unit)],
            TypeTag::Unit,
            vec![
                Instruction::EnumNew {
                    dest: ValueId(1),
                    variant_idx: 2,
                    payload: Some(Operand::Value(ValueId(0))),
                },
                Instruction::EnumPayload {
                    dest: ValueId(2),
                    scrutinee: Operand::Value(ValueId(1)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![rewrap])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        assert_eq!(jit.cached_function_count(), 1);

        let s = triet_ir::RuntimeValue::Struct {
            fields: vec![integer(7), integer(9)],
        };
        let jit_r = dispatch_boxed(&jit, FuncId(0), vec![s.clone()]);
        let mut vm = triet_ir::Vm::new(program);
        let vm_r = vm.execute(FuncId(0), vec![s.clone()]).expect("vm rewrap");
        assert_rv_eq(&jit_r, &vm_r);
        assert_rv_eq(&jit_r, &s); // struct fields preserved end-to-end
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn jit4_agg2b_boxed_outcome_ops_value_parity() {
        // agg.2b: boxed Outcome ops.
        //   ok(v)    -> ~+ v             (OutcomeNewPositive)
        //   err(v)   -> ~- v             (OutcomeNewNegative)
        //   nul()    -> ~0               (OutcomeNewNull)
        //   disc(o)  -> discriminant o   (→ Trit)
        //   val(o)   -> unwrap_value o
        //   errv(o)  -> unwrap_error o
        let ok = make_function_at(
            FuncId(0),
            "ok",
            vec![("v".into(), TypeTag::Integer)],
            TypeTag::Unit,
            vec![
                Instruction::OutcomeNewPositive {
                    dest: ValueId(1),
                    payload: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let err = make_function_at(
            FuncId(1),
            "err",
            vec![("v".into(), TypeTag::Integer)],
            TypeTag::Unit,
            vec![
                Instruction::OutcomeNewNegative {
                    dest: ValueId(1),
                    payload: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let nul = make_function_at(
            FuncId(2),
            "nul",
            vec![],
            TypeTag::Unit,
            vec![
                Instruction::OutcomeNewNull { dest: ValueId(0) },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let disc = make_function_at(
            FuncId(3),
            "disc",
            vec![("o".into(), TypeTag::Unit)],
            TypeTag::Trit,
            vec![
                Instruction::OutcomeDiscriminant {
                    dest: ValueId(1),
                    source: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let val = make_function_at(
            FuncId(4),
            "val",
            vec![("o".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                Instruction::OutcomeUnwrapValue {
                    dest: ValueId(1),
                    source: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let errv = make_function_at(
            FuncId(5),
            "errv",
            vec![("o".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                Instruction::OutcomeUnwrapError {
                    dest: ValueId(1),
                    source: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(
                &["khi"],
                vec![ok, err, nul, disc, val, errv],
            )],
            triet_ir::ConstantPool::new(),
        );

        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("boxed outcome ops must compile");
        assert_eq!(
            jit.cached_function_count(),
            6,
            "all 6 boxed outcome fns JIT"
        );

        let pos = || triet_ir::RuntimeValue::Outcome {
            discriminator: triet_core::Trit::Positive,
            payload: Some(Box::new(integer(42))),
        };
        let neg = || triet_ir::RuntimeValue::Outcome {
            discriminator: triet_core::Trit::Negative,
            payload: Some(Box::new(integer(7))),
        };

        // ok(42) == ~+42; err(7) == ~-7; nul() == ~0 — JIT vs VM.
        let mut vm = triet_ir::Vm::new(program.clone());
        assert_rv_eq(
            &dispatch_boxed(&jit, FuncId(0), vec![integer(42)]),
            &vm.execute(FuncId(0), vec![integer(42)]).expect("vm ok"),
        );
        assert_rv_eq(&dispatch_boxed(&jit, FuncId(0), vec![integer(42)]), &pos());
        let mut vm = triet_ir::Vm::new(program.clone());
        assert_rv_eq(
            &dispatch_boxed(&jit, FuncId(1), vec![integer(7)]),
            &vm.execute(FuncId(1), vec![integer(7)]).expect("vm err"),
        );
        assert_rv_eq(&dispatch_boxed(&jit, FuncId(1), vec![integer(7)]), &neg());
        let mut vm = triet_ir::Vm::new(program.clone());
        assert_rv_eq(
            &dispatch_boxed(&jit, FuncId(2), vec![]),
            &vm.execute(FuncId(2), vec![]).expect("vm nul"),
        );

        // disc(~+42) == Positive; disc(~0) == Zero — JIT vs VM.
        let mut vm = triet_ir::Vm::new(program.clone());
        let jit_disc = dispatch_boxed(&jit, FuncId(3), vec![pos()]);
        assert_rv_eq(
            &jit_disc,
            &vm.execute(FuncId(3), vec![pos()]).expect("vm disc"),
        );
        assert_rv_eq(
            &jit_disc,
            &triet_ir::RuntimeValue::Trit(triet_core::Trit::Positive),
        );

        // val(~+42) == 42; errv(~-7) == 7 — JIT vs VM.
        let mut vm = triet_ir::Vm::new(program.clone());
        let jit_val = dispatch_boxed(&jit, FuncId(4), vec![pos()]);
        assert_rv_eq(
            &jit_val,
            &vm.execute(FuncId(4), vec![pos()]).expect("vm val"),
        );
        assert_rv_eq(&jit_val, &integer(42));
        let mut vm = triet_ir::Vm::new(program);
        let jit_errv = dispatch_boxed(&jit, FuncId(5), vec![neg()]);
        assert_rv_eq(
            &jit_errv,
            &vm.execute(FuncId(5), vec![neg()]).expect("vm errv"),
        );
        assert_rv_eq(&jit_errv, &integer(7));
    }

    #[test]
    fn jit4_agg2b_boxed_outcome_unwrap_wrong_arm_fails() {
        // val(~-7): unwrap_value on a failure arm must FAIL at runtime —
        // the per-call sentinel converts the boxed JIT run to an error,
        // exactly as the VM raises InvalidOutcomeState.
        let val = make_function_at(
            FuncId(0),
            "val",
            vec![("o".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                Instruction::OutcomeUnwrapValue {
                    dest: ValueId(1),
                    source: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![val])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        assert_eq!(jit.cached_function_count(), 1);

        let neg = triet_ir::RuntimeValue::Outcome {
            discriminator: triet_core::Trit::Negative,
            payload: Some(Box::new(integer(7))),
        };
        // VM errors.
        let mut vm = triet_ir::Vm::new(program);
        assert!(
            vm.execute(FuncId(0), vec![neg.clone()]).is_err(),
            "VM must error"
        );
        // Boxed JIT errors via the per-call sentinel.
        let neg_ptr = shims::box_for_jit_test(neg);
        let r = dispatch_with_shim_errors(&jit, FuncId(0), &[neg_ptr], "val")
            .expect("function is cached");
        shims::drop_for_jit_test(neg_ptr);
        assert!(r.is_err(), "boxed JIT must surface the wrong-arm failure");
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn jit4_agg3a_boxed_nullable_ops_value_parity() {
        // agg.3a: boxed Nullable ops.
        //   wrap(v)  -> NullWrap v     (→ Enum{0, Some(v)})
        //   uw(n)    -> NullUnwrap n   (pass-through; Null panics)
        //   chk(n)   -> NullCheck n    (→ Trit)
        let wrap = make_function_at(
            FuncId(0),
            "wrap",
            vec![("v".into(), TypeTag::Integer)],
            TypeTag::Unit,
            vec![
                Instruction::NullWrap {
                    dest: ValueId(1),
                    value: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let uw = make_function_at(
            FuncId(1),
            "uw",
            vec![("n".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                Instruction::NullUnwrap {
                    dest: ValueId(1),
                    nullable: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let chk = make_function_at(
            FuncId(2),
            "chk",
            vec![("n".into(), TypeTag::Unit)],
            TypeTag::Trit,
            vec![
                Instruction::NullCheck {
                    dest: ValueId(1),
                    nullable: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![wrap, uw, chk])],
            triet_ir::ConstantPool::new(),
        );

        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("boxed nullable ops must compile");
        assert_eq!(
            jit.cached_function_count(),
            3,
            "all 3 boxed nullable fns JIT"
        );

        // wrap(5) == Enum{0, Some(5)} — JIT vs VM.
        let mut vm = triet_ir::Vm::new(program.clone());
        let jit_wrap = dispatch_boxed(&jit, FuncId(0), vec![integer(5)]);
        assert_rv_eq(
            &jit_wrap,
            &vm.execute(FuncId(0), vec![integer(5)]).expect("vm wrap"),
        );
        assert!(
            matches!(
                &jit_wrap,
                triet_ir::RuntimeValue::Enum {
                    variant: 0,
                    payload: Some(_)
                }
            ),
            "got {jit_wrap:?}"
        );

        // uw(42) == 42 (pass-through) — JIT vs VM.
        let mut vm = triet_ir::Vm::new(program.clone());
        let jit_uw = dispatch_boxed(&jit, FuncId(1), vec![integer(42)]);
        assert_rv_eq(
            &jit_uw,
            &vm.execute(FuncId(1), vec![integer(42)]).expect("vm uw"),
        );
        assert_rv_eq(&jit_uw, &integer(42));

        // chk(42) == Positive; chk(Null) == Zero — JIT vs VM.
        let mut vm = triet_ir::Vm::new(program.clone());
        let jit_chk = dispatch_boxed(&jit, FuncId(2), vec![integer(42)]);
        assert_rv_eq(
            &jit_chk,
            &vm.execute(FuncId(2), vec![integer(42)]).expect("vm chk"),
        );
        assert_rv_eq(
            &jit_chk,
            &triet_ir::RuntimeValue::Trit(triet_core::Trit::Positive),
        );

        let mut vm = triet_ir::Vm::new(program);
        let jit_chk_null = dispatch_boxed(&jit, FuncId(2), vec![triet_ir::RuntimeValue::Null]);
        assert_rv_eq(
            &jit_chk_null,
            &vm.execute(FuncId(2), vec![triet_ir::RuntimeValue::Null])
                .expect("vm chk null"),
        );
        assert_rv_eq(
            &jit_chk_null,
            &triet_ir::RuntimeValue::Trit(triet_core::Trit::Zero),
        );
    }

    #[test]
    fn jit4_agg3a_boxed_null_unwrap_panics_on_null() {
        // uw(Null): NullUnwrap on Null must FAIL at runtime — the boxed
        // JIT surfaces it via the per-call sentinel, matching the VM.
        let uw = make_function_at(
            FuncId(0),
            "uw",
            vec![("n".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                Instruction::NullUnwrap {
                    dest: ValueId(1),
                    nullable: Operand::Value(ValueId(0)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![uw])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");

        // VM errors on Null.
        let mut vm = triet_ir::Vm::new(program);
        assert!(
            vm.execute(FuncId(0), vec![triet_ir::RuntimeValue::Null])
                .is_err(),
            "VM must error"
        );
        // Boxed JIT errors via the per-call sentinel.
        let null_ptr = shims::box_for_jit_test(triet_ir::RuntimeValue::Null);
        let r = dispatch_with_shim_errors(&jit, FuncId(0), &[null_ptr], "uw")
            .expect("function is cached");
        shims::drop_for_jit_test(null_ptr);
        assert!(r.is_err(), "boxed JIT must surface the null-unwrap panic");
    }

    #[test]
    fn jit4_boxed_return_borrowed_param_no_double_free() {
        // Clone-on-return regression: a BOXED function (touches an
        // aggregate op → boxed) that returns a BORROWED composite param.
        // Without the clone-on-return discipline this double-frees (the
        // caller drops the result + the original owner drops the param =
        // same box freed twice → `malloc(): unaligned tcache` abort).
        //   keep(p, q: struct) -> struct {
        //       _ = p.field(0)   // FieldGet → boxed; result dropped at Ret
        //       ret q            // borrowed param → must be cloned
        //   }
        let keep = make_function_at(
            FuncId(0),
            "keep",
            vec![("p".into(), TypeTag::Unit), ("q".into(), TypeTag::Unit)],
            TypeTag::Unit,
            vec![
                Instruction::FieldGet {
                    dest: ValueId(2),
                    object: Operand::Value(ValueId(0)),
                    field_idx: 0,
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![keep])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        assert_eq!(jit.cached_function_count(), 1, "keep is boxed + JITs");

        let p = triet_ir::RuntimeValue::Struct {
            fields: vec![integer(1)],
        };
        let q = triet_ir::RuntimeValue::Struct {
            fields: vec![integer(2), integer(3)],
        };
        // dispatch_boxed drops both arg boxes + the result box. If `keep`
        // returns q without cloning, the result box aliases the q arg box
        // → double-free on the drops below. The clone makes it safe.
        let jit_r = dispatch_boxed(&jit, FuncId(0), vec![p.clone(), q.clone()]);
        let mut vm = triet_ir::Vm::new(program);
        let vm_r = vm.execute(FuncId(0), vec![p, q.clone()]).expect("vm keep");
        assert_rv_eq(&jit_r, &vm_r);
        assert_rv_eq(&jit_r, &q);
    }

    #[test]
    fn jit4_crosscall_boxed_to_unboxed_scalar_value_parity() {
        // agg.cross-call (chiều boxed→unboxed): a BOXED function (has a
        // struct op) calls an UNBOXED scalar helper. Args unbox (ptr→i64),
        // result re-boxes (i64→ptr).
        //   add(a, b: Integer) -> Integer = a + b          (unboxed)
        //   sum(p: struct)     -> Integer = add(p.0, p.1)  (boxed)
        let add = make_function_at(
            FuncId(0),
            "add",
            vec![
                ("a".into(), TypeTag::Integer),
                ("b".into(), TypeTag::Integer),
            ],
            TypeTag::Integer,
            vec![
                Instruction::Add {
                    dest: ValueId(2),
                    lhs: Operand::Value(ValueId(0)),
                    rhs: Operand::Value(ValueId(1)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let sum = make_function_at(
            FuncId(1),
            "sum",
            vec![("p".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                Instruction::FieldGet {
                    dest: ValueId(1),
                    object: Operand::Value(ValueId(0)),
                    field_idx: 0,
                },
                Instruction::FieldGet {
                    dest: ValueId(2),
                    object: Operand::Value(ValueId(0)),
                    field_idx: 1,
                },
                Instruction::CallLocal {
                    dest: Some(ValueId(3)),
                    callee: FuncId(0),
                    args: vec![Operand::Value(ValueId(1)), Operand::Value(ValueId(2))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(3))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![add, sum])],
            triet_ir::ConstantPool::new(),
        );

        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        // `add` is unboxed (pure scalar); `sum` is boxed and cross-mode
        // calls `add` — both must JIT now (no tier-down).
        assert!(jit.lookup(FuncId(0)).is_some(), "unboxed add JITs");
        assert!(
            jit.lookup(FuncId(1)).is_some(),
            "boxed sum cross-mode calling unboxed add must JIT"
        );

        let point = triet_ir::RuntimeValue::Struct {
            fields: vec![integer(7), integer(9)],
        };
        let jit_r = dispatch_boxed(&jit, FuncId(1), vec![point.clone()]);
        let mut vm = triet_ir::Vm::new(program);
        let vm_r = vm.execute(FuncId(1), vec![point]).expect("vm sum");
        assert_rv_eq(&jit_r, &vm_r);
        assert_rv_eq(&jit_r, &integer(16));
    }

    #[test]
    fn jit4_crosscall_composite_passthrough_value_parity() {
        // ADR-0035 §1+§2: composite cross-mode boundary now JITs safely.
        // The unboxed `id(s)=s` returns a borrowed composite param, so its
        // clone-on-return (§1 unboxed, TypeTag-guided) mints an owned box;
        // the boxed caller passes the composite arg through + records the
        // owned result (§2) — no double-free (the case that aborted with
        // `malloc(): unaligned tcache` before the discipline), no leak.
        //   id(s: String) -> String = s             (unboxed passthrough)
        //   pick(p struct) -> String = id(p.0)       (boxed cross-mode)
        let id = make_function_at(
            FuncId(0),
            "id",
            vec![("s".into(), TypeTag::String)],
            TypeTag::String,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let pick = make_function_at(
            FuncId(1),
            "pick",
            vec![("p".into(), TypeTag::Unit)],
            TypeTag::String,
            vec![
                Instruction::FieldGet {
                    dest: ValueId(1),
                    object: Operand::Value(ValueId(0)),
                    field_idx: 0,
                },
                Instruction::CallLocal {
                    dest: Some(ValueId(2)),
                    callee: FuncId(0),
                    args: vec![Operand::Value(ValueId(1))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![id, pick])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        // Both JIT now: `id` (unboxed composite passthrough) + `pick`
        // (boxed cross-mode composite call).
        assert!(jit.lookup(FuncId(0)).is_some(), "unboxed id JITs");
        assert!(
            jit.lookup(FuncId(1)).is_some(),
            "boxed cross-mode composite caller must JIT"
        );

        let point = triet_ir::RuntimeValue::Struct {
            fields: vec![
                triet_ir::RuntimeValue::String("triết".to_owned()),
                integer(9),
            ],
        };
        // dispatch_boxed drops the arg box + the result box; with the
        // clone-on-return discipline this is balanced (no double-free).
        let jit_r = dispatch_boxed(&jit, FuncId(1), vec![point.clone()]);
        let mut vm = triet_ir::Vm::new(program);
        let vm_r = vm.execute(FuncId(1), vec![point]).expect("vm pick");
        assert_rv_eq(&jit_r, &vm_r);
        assert_rv_eq(&jit_r, &triet_ir::RuntimeValue::String("triết".to_owned()));
    }

    #[test]
    fn jit4_unboxed_return_borrowed_composite_no_double_free() {
        // ADR-0035 §1 (unboxed): a same-mode unboxed→unboxed composite
        // passthrough. `outer` calls `id(s)=s` and returns the result; both
        // are unboxed (no aggregate op). Without unboxed clone-on-return
        // this double-frees the String box. The String is materialized by
        // a builtin so it's a real boxed composite flowing through unboxed
        // functions.
        let id = make_function_at(
            FuncId(0),
            "id",
            vec![("s".into(), TypeTag::String)],
            TypeTag::String,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![id])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        assert!(jit.lookup(FuncId(0)).is_some(), "unboxed id JITs");

        // Call id(s) directly: box a String arg, dispatch, read + drop the
        // result, drop the arg. Aliased without the §1 clone → double-free.
        let s_ptr = shims::box_for_jit_test(triet_ir::RuntimeValue::String("hi".to_owned()));
        let result_ptr = dispatch_integer(&jit, FuncId(0), &[s_ptr]).expect("id dispatch");
        let result = shims::read_for_jit_test(result_ptr);
        shims::drop_for_jit_test(result_ptr);
        shims::drop_for_jit_test(s_ptr);
        assert_rv_eq(&result, &triet_ir::RuntimeValue::String("hi".to_owned()));
    }

    #[test]
    fn jit4_crosscall_unit_boundary_tiers_down() {
        // A Unit boundary is ambiguous (true-Unit vs struct/enum-lowered),
        // so a cross-mode call with a Unit param/return must tier the
        // boxed caller down — never miscompile.
        //   sink(x: Unit) -> Integer = 0   (unboxed; Unit param)
        //   caller(p struct) -> Integer = sink(p.0)   (boxed → tier down)
        let mut pool = triet_ir::ConstantPool::new();
        let zero = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(0).unwrap(),
        ));
        let sink = make_function_at(
            FuncId(0),
            "sink",
            vec![("x".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                Instruction::Const {
                    dest: ValueId(1),
                    constant: zero,
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let caller = make_function_at(
            FuncId(1),
            "caller",
            vec![("p".into(), TypeTag::Unit)],
            TypeTag::Integer,
            vec![
                Instruction::FieldGet {
                    dest: ValueId(1),
                    object: Operand::Value(ValueId(0)),
                    field_idx: 0,
                },
                Instruction::CallLocal {
                    dest: Some(ValueId(2)),
                    callee: FuncId(0),
                    args: vec![Operand::Value(ValueId(1))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let program = make_program(vec![make_ir_module(&["khi"], vec![sink, caller])], pool);
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        // `caller` tiers down (Unit boundary) — `sink` itself still JITs.
        assert!(jit.lookup(FuncId(0)).is_some(), "unboxed sink JITs");
        assert!(
            jit.lookup(FuncId(1)).is_none(),
            "boxed caller with a Unit cross-mode boundary must tier down"
        );
    }

    #[test]
    fn jit3_program_with_call_local() {
        // main calls helper which returns 7.
        let mut pool = triet_ir::ConstantPool::new();
        let seven = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(7).unwrap(),
        ));
        let helper = make_function_at(
            FuncId(0),
            "helper",
            vec![],
            TypeTag::Integer,
            vec![
                Instruction::Const {
                    dest: ValueId(0),
                    constant: seven,
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let main = make_function_at(
            FuncId(1),
            "main",
            vec![],
            TypeTag::Integer,
            vec![
                Instruction::CallLocal {
                    dest: Some(ValueId(0)),
                    callee: FuncId(0),
                    args: vec![],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let program = make_program(vec![make_ir_module(&["khi"], vec![helper, main])], pool);
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("CallLocal program should compile");
        assert_eq!(jit.cached_function_count(), 2);
        assert!(jit.lookup(FuncId(0)).is_some());
        assert!(jit.lookup(FuncId(1)).is_some());
    }

    #[test]
    fn jit3_program_with_cross_module_call() {
        // main (module=khi) calls helper (module=khi.utils) via
        // CallCrossModule path resolution.
        let helper = make_function_at(
            FuncId(0),
            "helper",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let main = make_function_at(
            FuncId(1),
            "main",
            vec![("y".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![
                Instruction::CallCrossModule {
                    dest: Some(ValueId(1)),
                    path: AbsolutePath::new(
                        ModulePath::new(vec!["khi".to_string(), "utils".to_string()]),
                        "helper".to_string(),
                    ),
                    args: vec![Operand::Value(ValueId(0))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let program = make_program(
            vec![
                make_ir_module(&["khi"], vec![main]),
                make_ir_module(&["khi", "utils"], vec![helper]),
            ],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("CallCrossModule should compile");
        assert_eq!(jit.cached_function_count(), 2);
    }

    #[test]
    fn jit3_program_with_witness_call_dispatches_same_as_cross_module() {
        // WitnessCall lowers identically to CallCrossModule at v0.4
        // semantics per ADR-0012 §2. Verify it compiles.
        let helper = make_function_at(
            FuncId(0),
            "helper",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let main = make_function_at(
            FuncId(1),
            "main",
            vec![("y".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![
                Instruction::WitnessCall {
                    dest: Some(ValueId(1)),
                    path: AbsolutePath::new(
                        ModulePath::new(vec!["khi".to_string()]),
                        "helper".to_string(),
                    ),
                    witness_idx: 0,
                    args: vec![Operand::Value(ValueId(0))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![helper, main])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("WitnessCall should compile (v0.4 dispatch = CallCrossModule)");
        assert_eq!(jit.cached_function_count(), 2);
    }

    // ===== v0.9.x.jit.4: structured CallBuiltin tier-down diagnostic =====
    // Full builtin shim layer defers v0.10 per ADR-0030 §12 backlog
    // (RuntimeValue ABI marshaling complexity). This sub-task ships
    // ONLY the diagnostic improvement so functions calling builtins
    // tier-down with a name-bearing error instead of opaque Debug
    // dump. Real shim wiring lands v0.10.

    #[test]
    fn jit4_callbuiltin_without_shim_tiers_down_naming_builtin() {
        // A builtin WITHOUT an implemented shim (a jit.2b-iii cliff)
        // tier-downs with a diagnostic that names the builtin + the
        // jit.2b backlog. `FStringConcat` is a varargs cliff — no shim.
        use triet_ir::BuiltinName;
        let func = make_function(
            "with_fstring",
            vec![],
            TypeTag::Unit,
            vec![
                Instruction::CallBuiltin {
                    dest: None,
                    name: BuiltinName::FStringConcat,
                    args: vec![],
                },
                Instruction::Ret { value: None },
            ],
        );
        let mut jit = JitCompiler::new();
        match jit.compile(&func) {
            Err(JitError::UnsupportedOpcode { opcode }) => {
                assert!(
                    opcode.contains("CallBuiltin(fstring_concat)"),
                    "diagnostic must name the builtin via its Display impl, got: {opcode}"
                );
                assert!(
                    opcode.contains("jit.2b"),
                    "diagnostic must reference the jit.2b backlog, got: {opcode}"
                );
            }
            other => panic!("expected UnsupportedOpcode, got {other:?}"),
        }
    }

    #[test]
    fn jit4_callbuiltin_arity_mismatch_tiers_down() {
        // v0.10.x.jit.2a update: a builtin WITH a shim but called with
        // the wrong arity (e.g. `Println` with 0 args; the shim takes 1
        // composite ptr) tier-downs with an arity diagnostic rather
        // than miscompiling.
        use triet_ir::BuiltinName;
        let func = make_function(
            "println_wrong_arity",
            vec![],
            TypeTag::Unit,
            vec![
                Instruction::CallBuiltin {
                    dest: None,
                    name: BuiltinName::Println,
                    args: vec![],
                },
                Instruction::Ret { value: None },
            ],
        );
        let mut jit = JitCompiler::new();
        match jit.compile(&func) {
            Err(JitError::UnsupportedOpcode { opcode }) => {
                assert!(
                    opcode.contains("arity"),
                    "diagnostic must flag the arity mismatch, got: {opcode}"
                );
            }
            other => panic!("expected UnsupportedOpcode, got {other:?}"),
        }
    }

    #[test]
    fn jit4_program_with_builtin_caller_skipped_other_compiled() {
        // Program-level tier-down per ADR-0030 §2: function calling
        // builtin skipped, other function compiles. Same shape as
        // .jit.3's ClosureCall test but with CallBuiltin opcode.
        use triet_ir::BuiltinName;
        let pure_fn = make_function_at(
            FuncId(0),
            "pure",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let builtin_fn = make_function_at(
            FuncId(1),
            "uses_builtin",
            vec![],
            TypeTag::Unit,
            vec![
                Instruction::CallBuiltin {
                    dest: None,
                    name: BuiltinName::Println,
                    args: vec![],
                },
                Instruction::Ret { value: None },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![pure_fn, builtin_fn])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("program should compile (per-fn tier-down)");
        assert!(jit.lookup(FuncId(0)).is_some(), "pure fn must JIT");
        assert!(
            jit.lookup(FuncId(1)).is_none(),
            "builtin-using fn must tier-down (skipped from cache)"
        );
    }

    // ===== v0.9.x.jit.5: native dispatch end-to-end =====
    // First sub-task that actually executes JIT-compiled code (vs
    // just verifying compile succeeds). Uses safe wrapper
    // `dispatch_integer` to localize the unsafe transmute.

    #[test]
    fn jit5_dispatch_integer_signature_check() {
        // Integer-only signature qualifies.
        let int_fn = make_function_at(
            FuncId(0),
            "ok",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        assert!(super::is_jit_integer_dispatchable(&int_fn));

        // Trilean return disqualifies.
        let trilean_fn = make_function_at(
            FuncId(1),
            "trilean",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Trilean,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        assert!(!super::is_jit_integer_dispatchable(&trilean_fn));

        // 5-arg fn disqualifies (max 4).
        let five_arg_fn = make_function_at(
            FuncId(2),
            "five",
            (0..5)
                .map(|i| (format!("a{i}"), TypeTag::Integer))
                .collect(),
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        assert!(!super::is_jit_integer_dispatchable(&five_arg_fn));
    }

    #[test]
    fn jit5_dispatch_integer_identity() {
        // Compile + dispatch `id(x) = x`. Returns input unchanged.
        let id = make_function_at(
            FuncId(0),
            "id",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![id])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        let result = super::dispatch_integer(&jit, FuncId(0), &[42]);
        assert_eq!(result, Some(42), "identity must return its argument");
    }

    #[test]
    fn jit5_dispatch_integer_two_arg_add() {
        // `add(a, b) = a + b`. Result must match Rust integer add.
        let add = make_function_at(
            FuncId(0),
            "add",
            vec![
                ("a".to_string(), TypeTag::Integer),
                ("b".to_string(), TypeTag::Integer),
            ],
            TypeTag::Integer,
            vec![
                Instruction::Add {
                    dest: ValueId(2),
                    lhs: Operand::Value(ValueId(0)),
                    rhs: Operand::Value(ValueId(1)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![add])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        assert_eq!(
            super::dispatch_integer(&jit, FuncId(0), &[3, 4]),
            Some(7),
            "3 + 4 = 7 via JIT"
        );
        assert_eq!(
            super::dispatch_integer(&jit, FuncId(0), &[-10, 25]),
            Some(15),
            "-10 + 25 = 15 via JIT (negative arg handled)"
        );
    }

    #[test]
    fn jit5_dispatch_returns_none_on_uncached_fn() {
        // Empty JIT cache → dispatch is None.
        let jit = JitCompiler::new();
        assert_eq!(super::dispatch_integer(&jit, FuncId(999), &[]), None);
    }

    #[test]
    fn jit5_dispatch_returns_none_on_arity_overflow() {
        // 5+ args refused per supported-arity guard.
        let id = make_function_at(
            FuncId(0),
            "id",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![id])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        // Pass 5 args (signature has 1, but dispatch_integer
        // refuses by arity guard before invoking).
        assert_eq!(
            super::dispatch_integer(&jit, FuncId(0), &[1, 2, 3, 4, 5]),
            None,
            "arity > 4 must be refused"
        );
    }

    #[test]
    fn jit3_program_skips_function_with_unsupported_opcode() {
        // Per ADR-0030 §2 tier-down: a function with an unsupported
        // opcode is skipped, but the rest of the program compiles.
        let ok_fn = make_function_at(
            FuncId(0),
            "ok",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let bad_fn = make_function_at(
            FuncId(1),
            "bad",
            vec![],
            TypeTag::Unit,
            vec![
                // ClosureCall is not supported through .3.
                Instruction::ClosureCall {
                    dest: None,
                    closure: Operand::Value(ValueId(99)),
                    args: vec![],
                },
                Instruction::Ret { value: None },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![ok_fn, bad_fn])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("program should compile despite per-fn tier-down");
        // `ok` compiled; `bad` did not.
        assert!(
            jit.lookup(FuncId(0)).is_some(),
            "ok function should be cached"
        );
        assert!(
            jit.lookup(FuncId(1)).is_none(),
            "bad function should be skipped"
        );
    }

    // ===== v0.9.x.jit.5: JitDispatcher + Vm integration =====
    // End-to-end: install dispatcher → execute via Vm → counter
    // climbs → at threshold compile fires → subsequent calls run
    // native code.

    use triet_ir::{JitDispatch, RuntimeValue, Vm};

    fn make_increment_program() -> (IrProgram, FuncId) {
        // Two functions:
        //   helper(x) = x + 1     // FuncId(0), Integer-only signature
        //   main(seed) = helper(seed)  // FuncId(1)
        // main is what we drive in the loop so the Vm sees CallLocal
        // to helper repeatedly.
        let mut pool = triet_ir::ConstantPool::new();
        let one = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(1).unwrap(),
        ));
        let helper = make_function_at(
            FuncId(0),
            "helper",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![
                Instruction::Const {
                    dest: ValueId(1),
                    constant: one,
                },
                Instruction::Add {
                    dest: ValueId(2),
                    lhs: Operand::Value(ValueId(0)),
                    rhs: Operand::Value(ValueId(1)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let main = make_function_at(
            FuncId(1),
            "main",
            vec![("seed".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![
                Instruction::CallLocal {
                    dest: Some(ValueId(1)),
                    callee: FuncId(0),
                    args: vec![Operand::Value(ValueId(0))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let program = make_program(vec![make_ir_module(&["khi"], vec![helper, main])], pool);
        (program, FuncId(1))
    }

    // ===== v0.11.x.jit.3 Step 4b: AOT cache dispatcher integration =====

    /// `key → (object_bytes, manifest_bytes)`.
    type MockEntries = HashMap<String, (Vec<u8>, Vec<u8>)>;

    /// In-memory [`AotCacheStore`] for tests. `Clone` shares one backing
    /// map (via `Rc`), so a "cold" dispatcher's Path-B persist is visible
    /// to a later "warm" dispatcher's Path-A load.
    #[derive(Clone)]
    struct MockAotStore {
        map: std::rc::Rc<std::cell::RefCell<MockEntries>>,
    }

    impl MockAotStore {
        fn new() -> Self {
            Self {
                map: std::rc::Rc::new(std::cell::RefCell::new(HashMap::new())),
            }
        }
        fn is_empty(&self) -> bool {
            self.map.borrow().is_empty()
        }
        fn get(&self, key: &str) -> Option<(Vec<u8>, Vec<u8>)> {
            self.map.borrow().get(key).cloned()
        }
    }

    impl AotCacheStore for MockAotStore {
        fn load(&self, key: &str) -> Option<(Vec<u8>, Vec<u8>)> {
            self.map.borrow().get(key).cloned()
        }
        fn store(&self, key: &str, object: &[u8], manifest: &[u8]) {
            self.map
                .borrow_mut()
                .insert(key.to_string(), (object.to_vec(), manifest.to_vec()));
        }
    }

    #[test]
    fn aot_cache_path_a_matches_fresh_compile() {
        // §9.1 value parity: a cold run compiles fresh (Path B) + persists;
        // a warm run with the same store loads from cache (Path A). Both
        // must dispatch to the identical result — the cache must never
        // change observable behaviour, only avoid recompilation.
        let (program, main_id) = make_increment_program();
        let store = MockAotStore::new();
        let keys = vec![Some("mod0".to_string())];

        // Cold: cache empty → Path B fresh compile + persist.
        let mut cold = JitDispatcher::new();
        cold.enable_aot_cache(Box::new(store.clone()), keys.clone());
        for _ in 0..JIT_THRESHOLD {
            cold.record_call(main_id, &program);
        }
        assert_eq!(cold.try_dispatch(main_id, &[41]), Some(42));
        assert_eq!(cold.cache_state().misses, 1);
        assert_eq!(cold.cache_state().hits, 0);
        assert!(!store.is_empty(), "Path B must have persisted the object");

        // Warm: same store now populated → Path A cache hit.
        let mut warm = JitDispatcher::new();
        warm.enable_aot_cache(Box::new(store), keys);
        for _ in 0..JIT_THRESHOLD {
            warm.record_call(main_id, &program);
        }
        assert_eq!(
            warm.try_dispatch(main_id, &[41]),
            Some(42),
            "Path-A cache result must equal the fresh-compile result"
        );
        assert_eq!(warm.cache_state().hits, 1);
        assert_eq!(warm.cache_state().misses, 0);
    }

    #[test]
    fn aot_cache_refuses_version_mismatch_then_overwrites() {
        // §9.2: a cached entry whose manifest pins a different
        // `cranelift_version` must be refused (§2) → silent fallback to
        // Path B (§8) → still correct → and the stale entry overwritten.
        let (program, main_id) = make_increment_program();
        let store = MockAotStore::new();

        // Seed a valid object but tamper the manifest's cranelift version.
        let (object, mut manifest) = crate::aot::emit_module_object(&program, 0).expect("emit");
        manifest.cranelift_version = "0.0.0-bogus".to_string();
        store
            .map
            .borrow_mut()
            .insert("mod0".to_string(), (object, manifest.serialize()));

        let mut dispatcher = JitDispatcher::new();
        dispatcher.enable_aot_cache(Box::new(store.clone()), vec![Some("mod0".to_string())]);
        for _ in 0..JIT_THRESHOLD {
            dispatcher.record_call(main_id, &program);
        }
        // Version mismatch refused → Path B → correct result anyway.
        assert_eq!(dispatcher.try_dispatch(main_id, &[41]), Some(42));
        assert_eq!(dispatcher.cache_state().misses, 1);
        assert_eq!(dispatcher.cache_state().hits, 0);

        // Path B overwrote the stale entry with a current-version manifest.
        let (_object, manifest_bytes) = store.get("mod0").expect("entry present");
        let fresh = crate::aot::AotCacheManifest::deserialize(&manifest_bytes).expect("parse");
        assert_eq!(fresh.cranelift_version, cranelift_codegen::VERSION);
    }

    #[test]
    fn aot_cache_does_not_persist_when_a_function_tiers_down() {
        // A program with a tier-down function must NOT be persisted: its
        // object would have an undefined symbol that the load-time linker
        // refuses forever (permanent churn). Skipping is correct — the
        // program just recompiles fresh each run.
        let mut pool = triet_ir::ConstantPool::new();
        // `bad()` Consts a String → materialize_constant defers it →
        // emit_function_body errors → that function tiers down.
        let s = pool.intern(triet_ir::Constant::String("x".to_string()));
        let good = make_function_at(
            FuncId(0),
            "good",
            vec![],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Const(pool.intern(triet_ir::Constant::Integer(
                    triet_core::Integer::new(1).unwrap(),
                )))),
            }],
        );
        let bad = make_function_at(
            FuncId(1),
            "bad",
            vec![],
            TypeTag::Integer,
            vec![
                Instruction::Const {
                    dest: ValueId(0),
                    constant: s,
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let program = make_program(vec![make_ir_module(&["khi"], vec![good, bad])], pool);

        let store = MockAotStore::new();
        let mut dispatcher = JitDispatcher::new();
        dispatcher.enable_aot_cache(Box::new(store.clone()), vec![Some("mod0".to_string())]);
        for _ in 0..JIT_THRESHOLD {
            dispatcher.record_call(FuncId(0), &program);
        }
        // `good` compiled, `bad` tiered down → not fully JIT-able → no persist.
        assert!(
            dispatcher.compiler().cached_function_count() < 2,
            "bad() must tier down"
        );
        assert!(
            store.is_empty(),
            "a not-fully-JIT-able program must not persist an unloadable object"
        );
        assert_eq!(dispatcher.cache_state().misses, 1);
    }

    #[test]
    fn jit5_vm_with_dispatcher_returns_correct_result() {
        // Sanity: Vm with JitDispatcher installed produces the
        // correct numeric result on a single execute call. This is
        // a one-shot check — running execute in a tight loop on the
        // same Vm leaves stale frames per `Vm` semantics (entry
        // frame persists). End-to-end threshold-cross + native-
        // dispatch coverage lives in
        // `jit5_dispatcher_record_call_counts` +
        // `jit5_dispatcher_try_dispatch_returns_some_after_compile`
        // which exercise `JitDispatcher` directly.
        let (program, main_id) = make_increment_program();
        let mut vm = Vm::new(program);
        vm.set_jit_dispatcher(Box::new(JitDispatcher::new()));
        let seed = triet_core::Integer::new(7).unwrap();
        let result = vm
            .execute(main_id, vec![RuntimeValue::Integer(seed)])
            .expect("vm.execute");
        match result {
            RuntimeValue::Integer(out) => assert_eq!(out.to_i64(), 8),
            other => panic!("expected Integer, got {other:?}"),
        }
    }

    #[test]
    fn jit5_dispatcher_record_call_counts() {
        // Manual JitDispatcher test: feed record_call N times, verify
        // compile triggers exactly at threshold + later calls hit
        // cache.
        let (program, _) = make_increment_program();
        let mut dispatcher = JitDispatcher::new();

        // Pre-threshold: record_call doesn't compile.
        for _ in 0..(JIT_THRESHOLD - 1) {
            dispatcher.record_call(FuncId(0), &program);
        }
        assert_eq!(
            dispatcher.compiler().cached_function_count(),
            0,
            "no compile before threshold"
        );

        // Threshold crossing — compile fires.
        dispatcher.record_call(FuncId(0), &program);
        assert!(
            dispatcher.compiler().cached_function_count() >= 1,
            "compile must fire at threshold"
        );

        // Post-compile: subsequent record_calls increment counter but
        // don't re-compile.
        let cached_after = dispatcher.compiler().cached_function_count();
        dispatcher.record_call(FuncId(0), &program);
        assert_eq!(
            dispatcher.compiler().cached_function_count(),
            cached_after,
            "no re-compile after first threshold-crossing"
        );
    }

    #[test]
    fn jit5_dispatcher_try_dispatch_returns_some_after_compile() {
        // After threshold crossing, try_dispatch returns the native
        // result for an eligible function.
        let (program, _) = make_increment_program();
        let mut dispatcher = JitDispatcher::new();
        for _ in 0..JIT_THRESHOLD {
            dispatcher.record_call(FuncId(0), &program);
        }
        // helper(5) = 6.
        let result = dispatcher.try_dispatch(FuncId(0), &[5]);
        assert_eq!(result, Some(6));
    }

    #[test]
    fn jit5_vm_without_dispatcher_works_unchanged() {
        // Default Vm (no JIT installed) still works — JIT path is
        // strictly additive.
        let (program, main_id) = make_increment_program();
        let mut vm = Vm::new(program);
        let seed = triet_core::Integer::new(41).unwrap();
        let result = vm
            .execute(main_id, vec![RuntimeValue::Integer(seed)])
            .expect("vm.execute");
        match result {
            RuntimeValue::Integer(out) => assert_eq!(out.to_i64(), 42),
            other => panic!("expected Integer, got {other:?}"),
        }
    }

    #[test]
    fn jit5_disable_jit_clears_dispatcher() {
        // disable_jit removes the dispatcher → fall back to VM-only.
        let (program, main_id) = make_increment_program();
        let mut vm = Vm::new(program);
        vm.set_jit_dispatcher(Box::new(JitDispatcher::new()));
        vm.disable_jit();
        let seed = triet_core::Integer::new(99).unwrap();
        let result = vm
            .execute(main_id, vec![RuntimeValue::Integer(seed)])
            .expect("vm.execute");
        match result {
            RuntimeValue::Integer(out) => assert_eq!(out.to_i64(), 100),
            other => panic!("expected Integer, got {other:?}"),
        }
    }

    // ===== v0.10.x.jit.1: Layer A framework smoke tests (ADR-0032 §7.1) =====
    //
    // These exercise the shim-infrastructure mechanisms in isolation —
    // symbol registration + external-call codegen, catch_unwind → VmError,
    // drop_arc refcount balance, and capability-denied tier-down — WITHOUT
    // requiring any of the 43 production shims (those land in jit.2).

    use crate::shims::{self, AbiScalar, ShimEntry, ShimSignature};

    /// Framework self-test shim: identity over one `i64`. `extern
    /// "C-unwind"` per ADR-0032 §4. Referenced by address (registered
    /// via `JITBuilder::symbol`), so no `#[no_mangle]` needed. Never
    /// panics — exercises the symbol-registration + external-call
    /// codegen path without touching the deferred §4 unwind mechanism.
    extern "C-unwind" fn test_shim_identity(x: i64) -> i64 {
        x
    }

    fn entry(symbol: &'static str, addr: usize, sig: ShimSignature) -> ShimEntry {
        ShimEntry {
            builtin: None,
            symbol,
            addr,
            signature: sig,
        }
    }

    #[test]
    fn framework_shim_call_returns_value() {
        // Register `__triet_test_identity`, build a JIT caller that
        // forwards its i64 param to it, dispatch, assert round-trip.
        let sig = ShimSignature {
            params: &[AbiScalar::I64],
            ret: Some(AbiScalar::I64),
        };
        let shim = entry(
            "__triet_test_identity",
            test_shim_identity as *const () as usize,
            sig,
        );
        let mut jit = JitCompiler::new();
        jit.cache_shim_caller(FuncId(0), &[shim], &sig, "__triet_test_identity", &sig)
            .expect("compile shim caller");
        // Dispatch via the numeric `dispatch_integer` path — the shim
        // never panics, so the deferred §4 catch-unwind wrapper is not
        // needed to validate symbol-registration + external-call codegen.
        let result = dispatch_integer(&jit, FuncId(0), &[42]).expect("cache hit");
        assert_eq!(result, 42);
    }

    // NOTE: `framework_shim_panic_to_vm_error` (ADR-0032 §7.1) is
    // DEFERRED with the §4 error-propagation mechanism — it requires
    // catch_unwind across a Cranelift JIT frame, blocked on
    // cranelift-jit 0.132 (no system unwind-table registration). The
    // test lands when the ADR-0032 Addendum resolves the redesign.

    #[test]
    fn framework_drop_arc_balances_refcount() {
        use std::rc::Rc;
        use triet_ir::RuntimeValue;

        // Box a composite via Rc::into_raw, then hand the raw pointer
        // to __triet_drop_arc — the strong count must return to the
        // pre-box level, and the original handle stays valid.
        let original = Rc::new(RuntimeValue::String("framework".to_owned()));
        assert_eq!(Rc::strong_count(&original), 1);

        // Simulate a box-out: clone (count → 2), leak the clone as the
        // JIT-side owned pointer (into_raw consumes the +1 without
        // dropping → count stays 2).
        let boxed = Rc::clone(&original);
        let raw = Rc::into_raw(boxed) as i64;
        assert_eq!(Rc::strong_count(&original), 2);

        // drop_arc reconstitutes + drops the leaked Rc → count back to 1.
        shims::__triet_drop_arc(raw);
        assert_eq!(Rc::strong_count(&original), 1);

        // Null pointer is a no-op (does not panic, does not touch count).
        shims::__triet_drop_arc(0);
        assert_eq!(Rc::strong_count(&original), 1);
    }

    #[test]
    fn framework_capability_denied_tiers_down() {
        // A function calling AtomicNew, compiled with `sys.atomic`
        // denied, must surface BuiltinCapabilityDenied (not the
        // generic UnsupportedOpcode tier-down).
        use triet_ir::{
            BasicBlock, BlockId, BuiltinName, Constant, Function, IrModule, IrProgram, Operand,
            TypeTag, ValueId,
        };
        let mut constants = triet_ir::ConstantPool::new();
        let zero = constants.intern(Constant::Integer(triet_core::Integer::new(0).unwrap()));
        let func = Function {
            id: FuncId(0),
            name: Some("uses_atomic".to_owned()),
            params: vec![],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    Instruction::Const {
                        dest: ValueId(0),
                        constant: zero,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(1)),
                        name: BuiltinName::AtomicNew,
                        args: vec![Operand::Value(ValueId(0))],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    },
                ],
            }],
        };
        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::new(vec!["test".to_owned()]),
                    String::new(),
                ),
                functions: vec![func],
            }],
            constants,
            witness_tables: Vec::new(),
        };

        // compile_program_denied silently tiers-down per-function errors
        // (the function is dropped from the cache), so we assert via the
        // direct capability-check helper that the namespace maps + denies
        // correctly — the mechanism the codegen consults.
        let mut jit = JitCompiler::new();
        jit.compile_program_denied(&program, &["sys.atomic"])
            .expect("program-level compile (per-function tier-down is silent)");
        // The function must NOT be cached (it tiered down on the denied
        // builtin).
        assert!(
            jit.lookup(FuncId(0)).is_none(),
            "function calling a denied builtin must tier down (not cached)"
        );
        // And the check helper surfaces the precise diagnostic.
        let err = crate::check_builtin_capability(BuiltinName::AtomicNew, &["sys.atomic"])
            .expect_err("sys.atomic denied");
        match err {
            JitError::BuiltinCapabilityDenied { builtin, namespace } => {
                assert!(builtin.contains("atomic"), "builtin name: {builtin}");
                assert_eq!(namespace, "sys.atomic");
            }
            other => panic!("expected BuiltinCapabilityDenied, got {other:?}"),
        }
    }

    // ===== v0.10.x.jit.2a: composite-flow + shim end-to-end (ADR-0032) =====
    //
    // Build a single-function IR program whose body calls one builtin
    // shim, JIT-compile it, dispatch via `dispatch_with_shim_errors`,
    // and assert the result / error path. Validates the full composite
    // ABI (box-in / borrow / box-out) + §4 option-2 error propagation.

    // Most IR types are already imported elsewhere in the test module
    // (`BasicBlock`/`Function`/`Operand`/`TypeTag`/`ValueId` at ~630;
    // `RuntimeValue` at ~1457). `Constant` is the only new one here;
    // `IrModule`/`IrProgram` are referenced via `triet_ir::` paths.
    use triet_ir::Constant;

    fn single_fn_program(func: Function, constants: triet_ir::ConstantPool) -> triet_ir::IrProgram {
        triet_ir::IrProgram {
            modules: vec![triet_ir::IrModule {
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

    /// Build an `Atomic<Integer>` `RuntimeValue` for end-to-end atomic
    /// JIT tests (matches `dispatch_builtin(AtomicNew, ..)` shape).
    fn make_atomic(n: i64) -> RuntimeValue {
        RuntimeValue::Atomic(std::sync::Arc::new(std::sync::Mutex::new(
            RuntimeValue::Integer(triet_core::Integer::new(n).unwrap()),
        )))
    }

    #[test]
    fn jit_text_len_via_shim() {
        // `text_len_worker(s: String) -> Integer { TextLen(s) }`.
        // Composite arg (String ptr) → primitive return.
        let func = Function {
            id: FuncId(0),
            name: Some("text_len_worker".to_owned()),
            params: vec![("s".to_owned(), TypeTag::String)],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(1)),
                        name: BuiltinName::TextLen,
                        args: vec![Operand::Value(ValueId(0))],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(1))),
                    },
                ],
            }],
        };
        let program = single_fn_program(func, triet_ir::ConstantPool::new());
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        assert!(jit.lookup(FuncId(0)).is_some(), "text_len_worker must JIT");

        let s_ptr = shims::box_for_jit_test(RuntimeValue::String("Triết!".to_owned()));
        let result = dispatch_with_shim_errors(&jit, FuncId(0), &[s_ptr], "text_len_worker")
            .expect("cache hit")
            .expect("no shim failure");
        assert_eq!(result, 6, "char count of \"Triết!\""); // T r i ế t !
        shims::drop_for_jit_test(s_ptr);
    }

    #[test]
    fn jit_vector_new_via_shim() {
        // `make_vec() -> Vector { VectorNew() }`. Composite box-out
        // return; the i64 result is a boxed empty Vector ptr.
        let func = Function {
            id: FuncId(0),
            name: Some("make_vec".to_owned()),
            params: vec![],
            return_type: TypeTag::Vector(Box::new(TypeTag::Integer)),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(0)),
                        name: BuiltinName::VectorNew,
                        args: vec![],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    },
                ],
            }],
        };
        let program = single_fn_program(func, triet_ir::ConstantPool::new());
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");

        let result_ptr = dispatch_with_shim_errors(&jit, FuncId(0), &[], "make_vec")
            .expect("cache hit")
            .expect("no shim failure");
        // The returned i64 is a boxed empty Vector — verify + drop it.
        assert_ne!(result_ptr, 0);
        shims::drop_for_jit_test(result_ptr);
    }

    #[test]
    fn jit_assert_false_propagates_error() {
        // `assert_worker(x: Integer) -> Integer { Assert(False, null); x }`.
        // The failing Assert records a VmError + sets SHIM_FAILED; in
        // jit.2a's single-shim-call scope the function still runs to
        // `ret x`, and the dispatcher's boundary TLS check converts the
        // recorded error to `Err` (per ADR-0032 §4 option-2).
        let mut constants = triet_ir::ConstantPool::new();
        let false_c = constants.intern(Constant::Trilean(triet_logic::Trilean::False));
        // Integer 0 doubles as the null msg pointer (no message).
        let null_msg = constants.intern(Constant::Integer(triet_core::Integer::new(0).unwrap()));
        let func = Function {
            id: FuncId(0),
            name: Some("assert_worker".to_owned()),
            params: vec![("x".to_owned(), TypeTag::Integer)],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: None,
                        name: BuiltinName::Assert,
                        args: vec![Operand::Const(false_c), Operand::Const(null_msg)],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    },
                ],
            }],
        };
        let program = single_fn_program(func, constants);
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        assert!(jit.lookup(FuncId(0)).is_some(), "assert_worker must JIT");

        let result =
            dispatch_with_shim_errors(&jit, FuncId(0), &[7], "assert_worker").expect("cache hit");
        match result {
            Err(VmError::AssertionFailed { function, .. }) => {
                assert_eq!(function, "assert_worker");
            }
            other => panic!("expected AssertionFailed via boundary check, got {other:?}"),
        }
    }

    #[test]
    fn jit_assert_true_no_error() {
        // Same shape, cond=True — no failure, function returns x.
        let mut constants = triet_ir::ConstantPool::new();
        let true_c = constants.intern(Constant::Trilean(triet_logic::Trilean::True));
        let null_msg = constants.intern(Constant::Integer(triet_core::Integer::new(0).unwrap()));
        let func = Function {
            id: FuncId(0),
            name: Some("assert_ok".to_owned()),
            params: vec![("x".to_owned(), TypeTag::Integer)],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: None,
                        name: BuiltinName::Assert,
                        args: vec![Operand::Const(true_c), Operand::Const(null_msg)],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    },
                ],
            }],
        };
        let program = single_fn_program(func, constants);
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        let result = dispatch_with_shim_errors(&jit, FuncId(0), &[42], "assert_ok")
            .expect("cache hit")
            .expect("no shim failure");
        assert_eq!(result, 42);
    }

    #[test]
    fn jit_two_shim_calls_in_single_block_jit() {
        // v0.10.x.jit.2b-i: multiple shim calls in a single-block
        // function now JIT (per-call sentinel codegen). Two TextLen
        // calls on the same String, returning the second result.
        let func = Function {
            id: FuncId(0),
            name: Some("two_lens".to_owned()),
            params: vec![("s".to_owned(), TypeTag::String)],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(1)),
                        name: BuiltinName::TextLen,
                        args: vec![Operand::Value(ValueId(0))],
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(2)),
                        name: BuiltinName::TextLen,
                        args: vec![Operand::Value(ValueId(0))],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(2))),
                    },
                ],
            }],
        };
        let program = single_fn_program(func, triet_ir::ConstantPool::new());
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        assert!(jit.lookup(FuncId(0)).is_some(), "two-shim-call fn must JIT");

        let s_ptr = shims::box_for_jit_test(RuntimeValue::String("hello".to_owned()));
        let result = dispatch_with_shim_errors(&jit, FuncId(0), &[s_ptr], "two_lens")
            .expect("cache hit")
            .expect("no shim failure");
        assert_eq!(result, 5);
        shims::drop_for_jit_test(s_ptr);
    }

    #[test]
    fn jit_abort_on_first_shim_failure() {
        // v0.10.x.jit.2b-i abort-on-first-error: a function that first
        // calls a FAILING Assert then a Println must NOT run the Println
        // (the per-call sentinel branches to error_exit). We can't
        // directly observe stdout suppression, but we verify the
        // function returns Err (the failing Assert's VmError) — proving
        // the error_exit branch fired before the 2nd shim.
        let mut constants = triet_ir::ConstantPool::new();
        let false_c = constants.intern(Constant::Trilean(triet_logic::Trilean::False));
        let null_msg = constants.intern(Constant::Integer(triet_core::Integer::new(0).unwrap()));
        let func = Function {
            id: FuncId(0),
            name: Some("assert_then_print".to_owned()),
            params: vec![("msg".to_owned(), TypeTag::String)],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    // 1st shim: Assert(False) → records failure, sentinel.
                    Instruction::CallBuiltin {
                        dest: None,
                        name: BuiltinName::Assert,
                        args: vec![Operand::Const(false_c), Operand::Const(null_msg)],
                    },
                    // 2nd shim: Println(msg) — must NOT run (error_exit
                    // branch taken after the failing Assert).
                    Instruction::CallBuiltin {
                        dest: None,
                        name: BuiltinName::Println,
                        args: vec![Operand::Value(ValueId(0))],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Const(null_msg)),
                    },
                ],
            }],
        };
        let program = single_fn_program(func, constants);
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        assert!(
            jit.lookup(FuncId(0)).is_some(),
            "assert_then_print must JIT"
        );

        let msg_ptr = shims::box_for_jit_test(RuntimeValue::String("SHOULD NOT PRINT".to_owned()));
        let result = dispatch_with_shim_errors(&jit, FuncId(0), &[msg_ptr], "assert_then_print")
            .expect("cache hit");
        match result {
            Err(VmError::AssertionFailed { .. }) => {} // abort-on-first ✓
            other => panic!("expected AssertionFailed (abort before Println), got {other:?}"),
        }
        shims::drop_for_jit_test(msg_ptr);
    }

    #[test]
    fn jit_shim_in_multi_block_tiers_down() {
        // jit.2b-i scope: shim calls in a MULTI-block function tier
        // down (single-block shim scope). Build a 2-block function with
        // a shim call; assert it's not cached.
        let func = Function {
            id: FuncId(0),
            name: Some("multi_block_shim".to_owned()),
            params: vec![("s".to_owned(), TypeTag::String)],
            return_type: TypeTag::Integer,
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    name: Some("entry".to_owned()),
                    instructions: vec![Instruction::Br { target: BlockId(1) }],
                },
                BasicBlock {
                    id: BlockId(1),
                    name: Some("second".to_owned()),
                    instructions: vec![
                        Instruction::CallBuiltin {
                            dest: Some(ValueId(1)),
                            name: BuiltinName::TextLen,
                            args: vec![Operand::Value(ValueId(0))],
                        },
                        Instruction::Ret {
                            value: Some(Operand::Value(ValueId(1))),
                        },
                    ],
                },
            ],
        };
        let program = single_fn_program(func, triet_ir::ConstantPool::new());
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("compile (tier-down is silent)");
        assert!(
            jit.lookup(FuncId(0)).is_none(),
            "multi-block shim function must tier down in jit.2b-i"
        );
    }

    #[test]
    fn jit_atomic_fetch_add_end_to_end() {
        // jit.2b-ii end-to-end: `inc(counter: Atomic<Integer>, ord:
        // Ordering, delta: Integer) -> Integer { fetch_add(counter,
        // delta, ord) }`. `counter` + `ord` arrive as composite-ptr
        // PARAMS (boxed by the caller) — sidesteps the EnumNew gate
        // (we don't construct Ordering in-function). Proves the atomic
        // shim JITs + dispatches end-to-end through the composite ABI.
        let func = Function {
            id: FuncId(0),
            name: Some("inc".to_owned()),
            params: vec![
                (
                    "counter".to_owned(),
                    TypeTag::Atomic(Box::new(TypeTag::Integer)),
                ),
                // Ordering modeled as a Nullable here (composite ptr) so
                // the param maps to i64 without an Enum TypeTag; the VM
                // ignores the ordering value anyway.
                ("ord".to_owned(), TypeTag::Nullable(Box::new(TypeTag::Unit))),
                ("delta".to_owned(), TypeTag::Integer),
            ],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(3)),
                        name: BuiltinName::AtomicFetchAdd,
                        args: vec![
                            Operand::Value(ValueId(0)), // counter (Atomic ptr)
                            Operand::Value(ValueId(2)), // delta (i64)
                            Operand::Value(ValueId(1)), // ord (ptr, ignored)
                        ],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(3))),
                    },
                ],
            }],
        };
        let program = single_fn_program(func, triet_ir::ConstantPool::new());
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        assert!(jit.lookup(FuncId(0)).is_some(), "atomic fetch_add must JIT");

        // Box a counter=100 + a null ordering; call fetch_add(+5).
        let counter = shims::box_for_jit_test(make_atomic(100));
        let ord = 0_i64; // null ordering ptr (VM ignores)
        let prev = dispatch_with_shim_errors(&jit, FuncId(0), &[counter, ord, 5], "inc")
            .expect("cache hit")
            .expect("no shim failure");
        assert_eq!(prev, 100, "fetch_add returns previous value");
        // The shared Atomic now holds 105.
        shims::with_atomic_for_test(counter, |v| assert_eq!(v, 105));
        shims::drop_for_jit_test(counter);
    }
}
