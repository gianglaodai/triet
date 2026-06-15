//! Triết crate-pack format (`.khi`) and cross-package linker.
//!
//! Per [ADR-0011], this crate defines the binary format that carries an
//! ABI surface across crate boundaries. `.khi` is the unit of
//! distribution: it bundles an ABI metadata section, one or more `.triv`
//! IR code units (per [ADR-0008]), a dependency manifest, and a slot
//! reserved for capability claims (v0.6).
//!
//! Linker semantics live alongside: refuse-to-link policy per
//! [ADR-0013] and witness-table dispatch hooks per [ADR-0012].
//!
//! # Crate structure
//!
//! | Module | Purpose |
//! |---|---|
//! | [`types`] | `AbiMetadata`, `TypeDef`, `FunctionExport`, `Dep` |
//! | [`hash`] | BLAKE3 helpers for `iface_hash` + `impl_hash` |
//! | [`serde`] | `.khi` binary serializer/deserializer |
//! | [`error`] | E2300–E2399 linker diagnostics |
//!
//! [ADR-0008]: ../../../docs/decisions/0008-triv-binary-format.md
//! [ADR-0011]: ../../../docs/decisions/0011-abi-metadata-format.md
//! [ADR-0012]: ../../../docs/decisions/0012-witness-table-dispatch.md
//! [ADR-0013]: ../../../docs/decisions/0013-semver-linking-policy.md

#![warn(missing_docs)]
// Internal details behind the public types stay pub(crate); silence
// the redundant_pub_crate lint to keep the trade-off consistent
// across the workspace (matches `triet-ir`, `triet-parser`, etc.).
#![allow(clippy::redundant_pub_crate)]
// `.khi` uses fixed-width u32 fields for many index/length values
// derived from `usize` counters. A package with > 2^32 exports or
// dependencies is not a realistic input.
#![allow(clippy::cast_possible_truncation)]
// Doc comments reference ADR identifiers like "TypeRef", "TypeKind",
// "Visibility" which appear in narrative prose pointing at the matching
// Rust type. Wrapping every mention in backticks is noisy without
// adding clarity — disable the pedantic check at the crate level.
#![allow(clippy::doc_markdown)]
// Read/write dispatch tables (`write_type_def`, `read_export_table`)
// inherently match on a closed set of variants; arms with identical
// short bodies are an intentional dispatch pattern rather than a sign
// of duplication. Same trade-off the IR crate makes (ADR-0007).
#![allow(clippy::match_same_arms)]
// First-paragraph length is style preference — our ADR-driven doc
// comments often pack the rationale into the lead paragraph.
#![allow(clippy::too_long_first_doc_paragraph)]

mod capability_link;
mod capability_resolver;
mod error;
mod hash;
mod linker;
mod lockfile;
mod package_manifest;
mod policy;
mod resolver;
mod serde;
mod store;
mod strict_parser;
mod tty_prompt;
mod types;

pub use capability_link::{
    CapabilityLinkError, CapabilityLinkReport, DeferredCap, RootRefusalLevel, check_cap_divergence,
    check_link_capabilities,
};
pub use capability_resolver::{
    CachedDecision, CapabilityResolver, DecisionSource, PolicyRequest, ResolverError,
    resolve_deferrals,
};
pub use error::{PackError, PackResult, StoreError, StoreResult};
pub use hash::{
    IFACE_HASH_LEN, IMPL_HASH_LEN, IfaceHash, ImplHash, ModuleIfaceHash, ModuleImplHash,
    TermIfaceHash, TermImplHash, compute_iface_hash, compute_module_iface_hash,
    compute_module_impl_hash, compute_term_iface_hash, compute_term_impl_hash,
};
pub use linker::{LinkError, LinkPlan, LinkWarning, ResolvedDep, plan_link};
pub use lockfile::{LockEntry, Lockfile, LockfileError};
pub use package_manifest::{PackageManifest, PackageManifestError};
pub use policy::{Decision, OriginMatcher, PolicyError, PolicyRule, PolicyRules};
pub use resolver::{Resolution, ResolutionOrigin, ResolveError, ResolveResult, Resolver};
pub use store::{GcReport, RootEntry, Store};
pub use tty_prompt::{
    DepChainEntry, DevTtyPrompt, LockfileMatch, PackageInfo, PromptCallback, PromptChoice,
    PromptContext, context_from_request, non_interactive_callback, prompt_loop, render_prompt,
};
// `compute_impl_hash` is only needed inside `serde::write_khi` for
// now; we'll promote it to the public API when the linker (v0.4.5)
// needs to validate a downloaded pack against an externally-claimed
// hash. Until then, keeping it `pub(crate)` avoids advertising an
// API surface we haven't committed to.
pub use serde::{read_khi, write_khi};
pub use types::{
    AbiMetadata, CapabilityClaim, CapabilityLevel, Dep, EnumDefinition, FieldDef, FunctionExport,
    Module, Param, SemVer, StructDefinition, TypeDef, TypeKind, TypeRef, Visibility,
};
