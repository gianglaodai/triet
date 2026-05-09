//! Synthetic stdlib registry for v0.2.x.6.
//!
//! At this phase, stdlib functions (`std.io.println`, `std.text.len`,
//! `std.assert.assert`) are still backed by interpreter-side built-ins
//! rather than real `.tri` source files. The loader treats `std.*`
//! paths as **virtual modules** so import resolution has a single,
//! uniform code path. v0.2.x.7 will replace the registry with real
//! files declared via `module` (per ADR-0005 §"Implementation
//! roadmap" step 9).
//!
//! The registry is an explicit list — no magic introspection. Adding a
//! new stdlib export is a one-line edit here, then the export shows up
//! to the resolver exactly as a real module's would.
//!
//! Items here are crate-private — consumed by the name resolver
//! (#36.4) and visible to external crates only through the resolved
//! module's binding map.

#![allow(dead_code)] // consumed by resolver in #36.4

use crate::path::{AbsolutePath, ModulePath};

/// Returns the synthetic exports of a stdlib module path, or `None` if
/// the path does not name a known stdlib module.
///
/// The returned [`AbsolutePath`]s use `path` itself as the module
/// component, so callers can bind them straight into a module's
/// resolver scope.
#[must_use]
pub(crate) fn stdlib_exports(path: &ModulePath) -> Option<Vec<AbsolutePath>> {
    let names = stdlib_export_names(path)?;
    Some(
        names
            .iter()
            .map(|name| AbsolutePath::new(path.clone(), (*name).to_owned()))
            .collect(),
    )
}

/// True if `name` is a recognized export of the stdlib module at `path`.
#[must_use]
pub(crate) fn stdlib_has_export(path: &ModulePath, name: &str) -> bool {
    stdlib_export_names(path).is_some_and(|names| names.contains(&name))
}

/// True if `path` names a known synthetic stdlib module.
#[must_use]
pub(crate) fn is_known_stdlib_module(path: &ModulePath) -> bool {
    stdlib_export_names(path).is_some()
}

/// Internal: the export list for each known stdlib module.
///
/// Keep this aligned with `triet-interpreter`'s `builtins::install` and
/// `triet-typecheck`'s prelude. Mismatch here = unresolved import even
/// though the runtime would handle it, or vice versa.
fn stdlib_export_names(path: &ModulePath) -> Option<&'static [&'static str]> {
    let segs: Vec<&str> = path.segments().iter().map(String::as_str).collect();
    match segs.as_slice() {
        ["std", "io"] => Some(&["println", "print", "read_line"]),
        ["std", "text"] => Some(&["len", "concat", "from_integer"]),
        ["std", "assert"] => Some(&["assert"]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(segments: &[&str]) -> ModulePath {
        ModulePath::new(segments.iter().map(|s| (*s).to_owned()).collect())
    }

    #[test]
    fn std_io_has_println() {
        assert!(stdlib_has_export(&path(&["std", "io"]), "println"));
    }

    #[test]
    fn std_text_has_len() {
        assert!(stdlib_has_export(&path(&["std", "text"]), "len"));
    }

    #[test]
    fn unknown_module_has_no_exports() {
        assert!(stdlib_exports(&path(&["std", "nope"])).is_none());
        assert!(!is_known_stdlib_module(&path(&["std", "nope"])));
    }

    #[test]
    fn missing_export_in_known_module() {
        assert!(!stdlib_has_export(&path(&["std", "io"]), "frobnicate"));
    }

    #[test]
    fn exports_use_supplied_path() {
        let exports = stdlib_exports(&path(&["std", "io"])).unwrap();
        assert!(exports.iter().any(|p| p.to_string() == "std.io.println"));
    }
}
