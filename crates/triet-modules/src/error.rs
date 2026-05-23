//! Loader / resolver diagnostics.
//!
//! Error codes are reserved per ADR-0005 in the `triet::modules::E2100`
//! range. Cyclic-import diagnostics emit a trace
//! (`foo → bar → baz → foo`) directly in the message so the user sees
//! the cycle at a glance; the labelled span points at the import that
//! closes the cycle.

use miette::Diagnostic;
use thiserror::Error;
use triet_syntax::Span;

/// All errors produced during module loading and name resolution.
#[derive(Debug, Diagnostic, Error)]
pub enum LoaderError {
    /// E2100 — cyclic module dependency detected during DFS over imports.
    #[error("cyclic module dependency: {trace}")]
    #[diagnostic(
        code(triet::modules::E2100),
        help("modules cannot import each other; refactor shared definitions into a third module")
    )]
    CyclicImport {
        /// Pretty-printed cycle: `foo → bar → baz → foo`.
        trace: String,
        /// The import that closes the cycle.
        #[label("creates cycle")]
        span: Span,
    },

    /// E2101 — `module foo` declared but no `foo.tri` / `foo/foo.tri` found.
    #[error("module file not found for `{module_name}`")]
    #[diagnostic(
        code(triet::modules::E2101),
        help("create `{searched_primary}` or `{searched_nested}`")
    )]
    FileNotFound {
        /// Module name as written in the declaration.
        module_name: String,
        /// `<dir>/<name>.tri`.
        searched_primary: String,
        /// `<dir>/<name>/<name>.tri`.
        searched_nested: String,
        /// Span of the `module foo` declaration.
        #[label]
        span: Span,
    },

    /// E2102 — import path roots itself in a reserved namespace not yet
    /// usable in this version (`sys`/`dev`/`usr` per ADR-0005).
    #[error("namespace `{root}` is reserved and not yet usable")]
    #[diagnostic(
        code(triet::modules::E2102),
        help(
            "`sys`, `dev`, `usr` are reserved per ADR-0005 for v0.6 capability namespaces; use `std` for stdlib in v0.2.x"
        )
    )]
    ReservedNamespace {
        /// The reserved root segment that triggered the error.
        root: String,
        /// Span of the offending import path.
        #[label]
        span: Span,
    },

    /// E2103 — name exists in target module but is not visible from the
    /// importing module.
    #[error("`{name}` is not visible from this module")]
    #[diagnostic(
        code(triet::modules::E2103),
        help("declare `{name}` as `public` in its defining module to export it")
    )]
    VisibilityViolation {
        /// Name that failed visibility check.
        name: String,
        /// The visibility level of the target.
        actual_visibility: String,
        /// Span of the import.
        #[label]
        span: Span,
    },

    /// E2104 — import path or name does not resolve to anything.
    #[error("unresolved import: `{path}`")]
    #[diagnostic(code(triet::modules::E2104))]
    UnresolvedImport {
        /// Full import path as written.
        path: String,
        /// Span of the import.
        #[label]
        span: Span,
    },

    /// E2105 — child module file failed to parse. The inner parse error
    /// is rendered separately by the CLI; this variant lets the loader
    /// attribute the failure to a specific module.
    #[error("parse error in module `{module}`: {message}")]
    #[diagnostic(code(triet::modules::E2105))]
    ChildParseError {
        /// Full module path that failed to parse.
        module: String,
        /// Inner parse error message.
        message: String,
        /// Span of the `module foo` declaration that referenced the file.
        #[label]
        span: Span,
    },

    /// E2106 — I/O error reading a module source file.
    #[error("could not read module file `{path}`: {message}")]
    #[diagnostic(code(triet::modules::E2106))]
    IoError {
        /// Path the loader tried to read.
        path: String,
        /// Underlying I/O error message.
        message: String,
        /// Span of the `module foo` declaration.
        #[label]
        span: Span,
    },

    /// E2107 — `from X import Variant as Alias` for an enum variant.
    /// Variant aliasing isn't supported because the constructor's
    /// spelling is part of the value (typechecker matches variant by
    /// name within its enum). Import the parent enum instead.
    #[error("enum variant `{variant}` cannot be imported under an alias")]
    #[diagnostic(
        code(triet::modules::E2107),
        help(
            "either import the variant unaliased (e.g. `from X import {variant}`) or import the parent enum `{enum_name}` and use `{enum_name}.{variant}` at use sites"
        )
    )]
    AliasedVariantImport {
        /// The variant name in the import list.
        variant: String,
        /// The enum that owns the variant.
        enum_name: String,
        /// Span of the import.
        #[label]
        span: Span,
    },
}

impl LoaderError {
    /// The labelled span for this error — used by the CLI's JSON emitter.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::CyclicImport { span, .. }
            | Self::FileNotFound { span, .. }
            | Self::ReservedNamespace { span, .. }
            | Self::VisibilityViolation { span, .. }
            | Self::UnresolvedImport { span, .. }
            | Self::ChildParseError { span, .. }
            | Self::IoError { span, .. }
            | Self::AliasedVariantImport { span, .. } => span.clone(),
        }
    }

    /// Stable error code string — used by the CLI's JSON emitter.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::CyclicImport { .. } => "triet::modules::E2100",
            Self::FileNotFound { .. } => "triet::modules::E2101",
            Self::ReservedNamespace { .. } => "triet::modules::E2102",
            Self::VisibilityViolation { .. } => "triet::modules::E2103",
            Self::UnresolvedImport { .. } => "triet::modules::E2104",
            Self::ChildParseError { .. } => "triet::modules::E2105",
            Self::IoError { .. } => "triet::modules::E2106",
            Self::AliasedVariantImport { .. } => "triet::modules::E2107",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyclic_import_message_contains_trace() {
        let error = LoaderError::CyclicImport {
            trace: "foo → bar → foo".to_owned(),
            span: 0..10,
        };
        assert!(error.to_string().contains("foo → bar → foo"));
        assert_eq!(error.code(), "triet::modules::E2100");
    }

    #[test]
    fn reserved_namespace_code() {
        let error = LoaderError::ReservedNamespace {
            root: "sys".to_owned(),
            span: 0..3,
        };
        assert_eq!(error.code(), "triet::modules::E2102");
        assert!(error.to_string().contains("sys"));
    }

    #[test]
    fn span_extraction_uniform() {
        let error = LoaderError::FileNotFound {
            module_name: "foo".to_owned(),
            searched_primary: "foo.tri".to_owned(),
            searched_nested: "foo/foo.tri".to_owned(),
            span: 5..15,
        };
        assert_eq!(error.span(), 5..15);
    }
}
