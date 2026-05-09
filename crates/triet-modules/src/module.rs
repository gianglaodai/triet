//! Resolved module representation.
//!
//! After loading + name resolution, the program is a flat list of
//! [`Module`]s indexed by [`ModuleId`]. Each module carries its own
//! AST [`Program`], a binding map (local name → [`AbsolutePath`]) for
//! imports, and parent/children links. Typecheck and interpreter
//! consume [`ResolvedProgram`] instead of a bare `Program`.

use std::{collections::HashMap, path::PathBuf};

use triet_syntax::Program;

use crate::path::{AbsolutePath, ModulePath};

/// Index handle for a module within a [`ResolvedProgram`].
///
/// Stable for the lifetime of the program. The crate root is at the
/// reserved index returned by [`ResolvedProgram::root`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModuleId(pub(crate) usize);

impl ModuleId {
    /// Internal index. Exposed for diagnostics, not for arithmetic.
    #[must_use]
    pub const fn raw(self) -> usize {
        self.0
    }
}

/// One module within the resolved program.
///
/// Inline modules (`module foo { … }`) and file-bound modules
/// (`module foo` with `foo.tri` on disk) are both represented as
/// `Module` — the only difference is whether `source_path` is set.
/// Synthetic stdlib modules also appear here with `source_path = None`
/// and an empty `program`; their bindings are populated by the resolver
/// from a hard-coded export list.
#[derive(Clone, Debug)]
pub struct Module {
    /// Fully-qualified path of this module — e.g. `crate.foo.bar`.
    pub path: ModulePath,
    /// Source file backing this module, if any. `None` for inline and
    /// synthetic modules.
    pub source_path: Option<PathBuf>,
    /// AST owned by this module — items declared lexically in its body.
    /// Empty for synthetic modules.
    pub program: Program,
    /// Local-name → fully-qualified path. Populated during name
    /// resolution from `import` and `from … import …` declarations.
    /// Items declared inside this module also appear here so callers
    /// can look up every visible name uniformly.
    pub bindings: HashMap<String, AbsolutePath>,
    /// Parent module — `None` only for the crate root.
    pub parent: Option<ModuleId>,
    /// Direct submodules in declaration order.
    pub children: Vec<ModuleId>,
}

/// The output of [`crate::load_program`] / [`crate::load_program_from_source`].
///
/// Replaces a bare `Program` as the input to typecheck and interpreter
/// once the module system is wired up. Holds every module that
/// participates in the program — local crate modules first, then any
/// referenced stdlib modules.
#[derive(Clone, Debug)]
pub struct ResolvedProgram {
    /// All modules, indexed by [`ModuleId`].
    pub modules: Vec<Module>,
    /// The crate root module — where `main` is looked up.
    pub root: ModuleId,
}

impl ResolvedProgram {
    /// Look up a module by id.
    #[must_use]
    pub fn module(&self, id: ModuleId) -> &Module {
        &self.modules[id.0]
    }

    /// Look up a module by id, mutably. Used by the resolver during
    /// construction.
    #[allow(dead_code)] // consumed by resolver in #36.4
    pub(crate) fn module_mut(&mut self, id: ModuleId) -> &mut Module {
        &mut self.modules[id.0]
    }

    /// The crate root module.
    #[must_use]
    pub fn root_module(&self) -> &Module {
        self.module(self.root)
    }

    /// Find a module by its absolute path. `O(n)`; used during
    /// resolution and diagnostic rendering.
    #[must_use]
    pub fn find_module(&self, path: &ModulePath) -> Option<ModuleId> {
        self.modules
            .iter()
            .position(|m| m.path == *path)
            .map(ModuleId)
    }

    /// Number of modules in the program (local + synthetic stdlib).
    #[must_use]
    pub const fn len(&self) -> usize {
        self.modules.len()
    }

    /// True if the program has no modules — only possible during
    /// construction; a successful `load_program` always produces at
    /// least the crate root.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_module(path: ModulePath, parent: Option<ModuleId>) -> Module {
        Module {
            path,
            source_path: None,
            program: Program::empty(),
            bindings: HashMap::new(),
            parent,
            children: Vec::new(),
        }
    }

    #[test]
    fn root_module_lookup() {
        let root_path = ModulePath::crate_root();
        let program = ResolvedProgram {
            modules: vec![empty_module(root_path.clone(), None)],
            root: ModuleId(0),
        };
        assert_eq!(program.root_module().path, root_path);
    }

    #[test]
    fn find_module_by_path() {
        let root_path = ModulePath::crate_root();
        let foo_path = root_path.child("foo");
        let program = ResolvedProgram {
            modules: vec![
                empty_module(root_path, None),
                empty_module(foo_path.clone(), Some(ModuleId(0))),
            ],
            root: ModuleId(0),
        };
        let found = program.find_module(&foo_path).unwrap();
        assert_eq!(found, ModuleId(1));
    }

    #[test]
    fn find_module_returns_none_for_missing() {
        let program = ResolvedProgram {
            modules: vec![empty_module(ModulePath::crate_root(), None)],
            root: ModuleId(0),
        };
        let missing = ModulePath::crate_root().child("nope");
        assert!(program.find_module(&missing).is_none());
    }
}
