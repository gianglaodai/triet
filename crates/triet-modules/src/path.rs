//! Module and item paths.
//!
//! Triết uses dot-separated paths (`crate.foo.bar`) per ADR-0005. This
//! module gives those paths a typed representation: [`ModulePath`] points
//! at a module, [`AbsolutePath`] at a specific item inside one. Both are
//! sequences of segments under the hood; the wrappers exist so the rest
//! of the loader can't accidentally mix them up.

use std::fmt;

/// A path identifying a module — sequence of segments from a root.
///
/// The first segment is significant: `"crate"` for the local crate root,
/// `"std"` / `"sys"` / `"dev"` / `"usr"` / `"core"` for reserved
/// namespaces (per ADR-0005). Empty paths are not permitted in the
/// resolved program — the root module is `["crate"]`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModulePath {
    segments: Vec<String>,
}

impl ModulePath {
    /// Build a path from raw segments. Caller is responsible for
    /// non-emptiness when used as a real module reference.
    #[must_use]
    pub const fn new(segments: Vec<String>) -> Self {
        Self { segments }
    }

    /// The root path of the local crate — `["crate"]`.
    #[must_use]
    pub fn crate_root() -> Self {
        Self {
            segments: vec!["crate".to_owned()],
        }
    }

    /// Borrow the underlying segments.
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// First segment, or `None` for an empty path.
    #[must_use]
    pub fn root(&self) -> Option<&str> {
        self.segments.first().map(String::as_str)
    }

    /// Return the parent path — drop the last segment. `None` if the
    /// path is at most one segment deep.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        if self.segments.len() <= 1 {
            None
        } else {
            Some(Self {
                segments: self.segments[..self.segments.len() - 1].to_vec(),
            })
        }
    }

    /// Append a child segment — `crate.foo`.child("bar") = `crate.foo.bar`.
    #[must_use]
    pub fn child(&self, name: &str) -> Self {
        let mut segments = self.segments.clone();
        segments.push(name.to_owned());
        Self { segments }
    }

    /// Number of segments.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.segments.len()
    }

    /// True if there are no segments — only meaningful for transient
    /// values during construction.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// True if the root segment names a reserved stdlib namespace
    /// (`std`/`sys`/`dev`/`usr`/`core`). Per ADR-0005, only `std` and
    /// `core` are usable in v0.2.x; the others are reserved for v0.6.
    #[must_use]
    pub fn is_reserved_root(&self) -> bool {
        matches!(
            self.root(),
            Some("std" | "sys" | "dev" | "usr" | "core")
        )
    }

    /// True if the path's root is the local crate (`crate.…`).
    #[must_use]
    pub fn is_local_crate(&self) -> bool {
        matches!(self.root(), Some("crate"))
    }
}

impl fmt::Display for ModulePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.segments.join("."))
    }
}

/// A fully-qualified path to an item — module path plus item name.
///
/// Example: in module `crate.foo`, the function `bar` has absolute path
/// `crate.foo.bar`. Used as the canonical identity of every named
/// entity after name resolution; binding maps in [`crate::Module`] map
/// local names to `AbsolutePath`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AbsolutePath {
    /// Module containing the item.
    pub module: ModulePath,
    /// Item name within the module.
    pub name: String,
}

impl AbsolutePath {
    /// Construct from a module path and item name.
    #[must_use]
    pub const fn new(module: ModulePath, name: String) -> Self {
        Self { module, name }
    }
}

impl fmt::Display for AbsolutePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.module, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_path_displays_with_dots() {
        let path = ModulePath::new(vec![
            "crate".to_owned(),
            "foo".to_owned(),
            "bar".to_owned(),
        ]);
        assert_eq!(path.to_string(), "crate.foo.bar");
    }

    #[test]
    fn crate_root_is_single_segment() {
        let root = ModulePath::crate_root();
        assert_eq!(root.to_string(), "crate");
        assert_eq!(root.len(), 1);
    }

    #[test]
    fn parent_of_root_is_none() {
        let root = ModulePath::crate_root();
        assert!(root.parent().is_none());
    }

    #[test]
    fn parent_drops_last_segment() {
        let path = ModulePath::crate_root().child("foo").child("bar");
        let parent = path.parent().unwrap();
        assert_eq!(parent.to_string(), "crate.foo");
    }

    #[test]
    fn child_appends_segment() {
        let path = ModulePath::crate_root().child("io");
        assert_eq!(path.to_string(), "crate.io");
    }

    #[test]
    fn reserved_roots_recognized() {
        for root in ["std", "sys", "dev", "usr", "core"] {
            let path = ModulePath::new(vec![root.to_owned()]);
            assert!(path.is_reserved_root(), "{root} should be reserved");
        }
        assert!(!ModulePath::crate_root().is_reserved_root());
    }

    #[test]
    fn local_crate_recognized() {
        assert!(ModulePath::crate_root().is_local_crate());
        assert!(!ModulePath::new(vec!["std".to_owned()]).is_local_crate());
    }

    #[test]
    fn absolute_path_displays_with_terminal_name() {
        let abs = AbsolutePath::new(
            ModulePath::new(vec!["std".to_owned(), "io".to_owned()]),
            "println".to_owned(),
        );
        assert_eq!(abs.to_string(), "std.io.println");
    }
}
