//! Linker + pack diagnostics in the `E2300` namespace (ADR-0013 §3).
//!
//! These cover both wire-format problems (bad magic, truncated input)
//! and policy outcomes (version mismatch, refuse-to-link). Every
//! variant implements `miette::Diagnostic` so the CLI can render
//! them uniformly with everything else (ADR-0009 § C).
//!
//! `StoreError` (v0.5.4+) wraps both [`PackError`] and `io::Error`
//! for the CAS store API in `store.rs`.

use std::io;

use miette::Diagnostic;
use thiserror::Error;

/// Shorthand for `Result<T, PackError>`.
pub type PackResult<T> = Result<T, PackError>;

/// Errors raised by `triet-pack` — both format-level (corrupted file)
/// and policy-level (linker refusing).
#[derive(Clone, Debug, Diagnostic, Error, PartialEq, Eq)]
pub enum PackError {
    /// The file isn't a `.khi` (magic bytes don't match).
    #[error("not a .khi file: magic bytes mismatch")]
    #[diagnostic(
        code(triet::pack::E2300),
        help("the file may be corrupted or it's a `.triv` IR file (not a packaged crate)")
    )]
    BadMagic,

    /// The pack format version is newer than this reader supports.
    #[error("unsupported pack format version {found} (max supported: {supported})")]
    #[diagnostic(
        code(triet::pack::E2301),
        help(
            "update the Triết toolchain — this `.khi` was produced by a newer compiler that knows fields this reader does not"
        )
    )]
    UnsupportedAbiVersion {
        /// Version found in the file header.
        found: u32,
        /// Maximum version this reader understands.
        supported: u32,
    },

    /// Structural corruption: truncated section, bad UTF-8, varint
    /// overflow, etc. Free-form message because the cause varies.
    #[error("corrupted .khi: {0}")]
    #[diagnostic(
        code(triet::pack::E2302),
        help(
            "the bytes describe an invalid layout — re-build the package, or inspect with `triet pack inspect`"
        )
    )]
    Corrupted(String),

    /// An unknown discriminant byte for a typed enum field
    /// (TypeRef kind, TypeKind, Visibility). Catches forward-compat
    /// drift without conflating with general corruption.
    #[error("unknown discriminant 0x{discriminant:02X} for {field}")]
    #[diagnostic(
        code(triet::pack::E2303),
        help("this enum variant didn't exist when the reader was built")
    )]
    UnknownDiscriminant {
        /// Which field carried the bad byte (e.g. "TypeRef", "Visibility").
        field: &'static str,
        /// The byte value that wasn't recognised.
        discriminant: u8,
    },
}

/// Shorthand for `Result<T, StoreError>`.
pub type StoreResult<T> = Result<T, StoreError>;

/// Errors raised by the CAS package store (`store.rs`). Wraps the
/// pure format errors from [`PackError`] and the filesystem errors
/// from [`std::io::Error`]. Reserved error-code namespace E2360–E2369
/// per ADR-0015.
#[derive(Debug, Diagnostic, Error)]
pub enum StoreError {
    /// Filesystem error during install / resolve / gc.
    #[error("store I/O error at {path}: {source}")]
    #[diagnostic(
        code(triet::pack::E2360),
        help("verify the store directory exists and is writable")
    )]
    Io {
        /// Path that triggered the failure (best-effort — set to "" if
        /// the failing op didn't touch a specific path).
        path: String,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// The pack bytes didn't parse — wraps a [`PackError`] surfaced
    /// while reading metadata before installing.
    #[error("invalid .khi handed to store: {0}")]
    #[diagnostic(transparent)]
    Pack(#[from] PackError),

    /// Lockfile (`triet.lock`) format or version mismatch.
    #[error("lockfile error: {0}")]
    #[diagnostic(transparent)]
    Lockfile(#[from] crate::lockfile::LockfileError),

    /// Package manifest (`triet.package`) format or version mismatch
    /// (v0.6.5+, ADR-0018 §1).
    #[error("package manifest error: {0}")]
    #[diagnostic(transparent)]
    PackageManifest(#[from] crate::package_manifest::PackageManifestError),

    /// Policy file (`triet.policy`) format or version mismatch
    /// (v0.6.6+, ADR-0017 §3).
    #[error("policy error: {0}")]
    #[diagnostic(transparent)]
    Policy(#[from] crate::policy::PolicyError),
}

impl StoreError {
    /// Convenience constructor for IO errors with a path context.
    pub(crate) fn io(path: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
