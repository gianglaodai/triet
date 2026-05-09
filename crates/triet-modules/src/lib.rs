//! Triết module loader, name resolver, and cyclic-dependency detector.
//!
//! Sits between parsing and type-checking. Walks `module foo`
//! declarations starting from a root file, loads sub-files, builds a
//! module dependency graph, detects cycles, and resolves
//! `import` / `from … import …` declarations to fully-qualified paths.
//!
//! Per [ADR-0005] ("Module system: Java JPMS aesthetic, dot paths,
//! Python imports"). Public surface mirrors the parser's:
//! [`load_program`] is the filesystem-driven entry point;
//! [`load_program_from_source`] is the in-memory equivalent for tests
//! and REPL usage where there is no surrounding directory.
//!
//! [ADR-0005]: ../../../docs/decisions/0005-module-system.md
//!
//! # Output
//!
//! Both entry points produce a [`ResolvedProgram`] — a flat list of
//! [`Module`]s, each carrying its own AST plus a binding map (local
//! name → [`AbsolutePath`]). Type-check and interpreter consume the
//! resolved program instead of a bare `Program`.

#![warn(missing_docs)]
// Internal helpers behind the public `load_program` / `load_program_from_source`
// surface are `pub(crate)`. `redundant_pub_crate` flags them as redundant
// (because their hosting modules are private), but `unreachable_pub`
// would flag them the other way if we made them `pub`. Silence the
// nursery lint to keep the trade-off consistent across the workspace
// (matches `triet-parser`, `triet-typecheck`, `triet-interpreter`).
#![allow(clippy::redundant_pub_crate)]

mod error;
mod module;
mod path;
mod stdlib;

use std::path::Path;

pub use error::LoaderError;
pub use module::{Module, ModuleId, ResolvedProgram};
pub use path::{AbsolutePath, ModulePath};

/// Load a Triết program starting from `root_path`.
///
/// The file at `root_path` is treated as the crate root. The loader
/// walks `module foo` declarations relative to `root_path`'s directory,
/// resolves each to `<dir>/foo.tri` or `<dir>/foo/foo.tri`, recurses
/// into inline `module foo { … }` bodies, builds the dependency graph,
/// detects cycles, and resolves every import to an absolute path.
///
/// # Errors
///
/// Returns a non-empty `Vec<LoaderError>` if any phase fails. The
/// loader accumulates errors across modules where possible so the user
/// sees the full failure surface in one run.
pub fn load_program(root_path: &Path) -> Result<ResolvedProgram, Vec<LoaderError>> {
    let _ = root_path;
    todo!("file loader — implemented in v0.2.x.6 step #36.2")
}

/// Load a single-file program directly from source text — no
/// filesystem access.
///
/// Used by tests, REPL, and any context where the program is fully
/// in-memory. Inline `module foo { … }` declarations are recursed
/// into as usual; **external** `module foo` declarations are rejected
/// with [`LoaderError::FileNotFound`] (no filesystem available).
///
/// # Errors
///
/// Same error semantics as [`load_program`].
pub fn load_program_from_source(source: &str) -> Result<ResolvedProgram, Vec<LoaderError>> {
    let _ = source;
    todo!("file loader — implemented in v0.2.x.6 step #36.2")
}
