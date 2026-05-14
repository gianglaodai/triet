//! Resolved module representation.
//!
//! After loading + name resolution the program is a flat list of
//! [`Module`]s indexed by [`ModuleId`]. Each module borrows one of the
//! [`Arena`]s held by [`ResolvedProgram`]; an arena maps roughly to one
//! parsed source file — inline submodules share their parent's arena,
//! file-bound submodules each get a fresh one. Lookups go through
//! `arenas[module.arena_id]`, so `*Id` handles inside `module.items`
//! always resolve to the same arena that produced them.
//!
//! This shape lets every module own its own slice of items without
//! the loader having to merge arenas across files.

use std::{collections::HashMap, path::PathBuf};

use triet_syntax::{Arena, Item, Spanned};

use crate::path::{AbsolutePath, ModulePath};

/// Index handle for a module within a [`ResolvedProgram`].
///
/// Stable for the lifetime of the program. The crate root sits at the
/// reserved index returned by [`ResolvedProgram::root`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModuleId(pub usize);

impl ModuleId {
    /// Internal index. Exposed for diagnostics, not for arithmetic.
    #[must_use]
    pub const fn raw(self) -> usize {
        self.0
    }
}

/// Index handle for an arena within a [`ResolvedProgram`].
///
/// One arena per parsed source file. Inline submodules share their
/// parent's `ArenaId`; file-bound submodules each have a unique one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ArenaId(pub usize);

impl ArenaId {
    /// Internal index. Exposed for diagnostics.
    #[must_use]
    pub const fn raw(self) -> usize {
        self.0
    }
}

/// One module within the resolved program.
///
/// Inline modules (`module foo { … }`) and file-bound modules
/// (`module foo` with `foo.tri` on disk) are both represented as
/// `Module`. The differences:
/// - `source_path` is `Some` only for file-bound and synthetic modules
///   tied to a real file (synthetic stdlib modules use `None`).
/// - `arena_id` is unique for file-bound; shared with parent for inline.
#[derive(Clone, Debug)]
pub struct Module {
    /// Fully-qualified path of this module — e.g. `crate.foo.bar`.
    pub path: ModulePath,
    /// Source file backing this module, if any. `None` for inline and
    /// synthetic modules.
    pub source_path: Option<PathBuf>,
    /// Arena holding every recursive AST node referenced by `items`.
    /// Index into [`ResolvedProgram::arenas`].
    pub arena_id: ArenaId,
    /// Items lexically belonging to this module — submodule
    /// declarations are *not* in this list (they live as separate
    /// [`Module`]s and appear in `children`).
    pub items: Vec<Spanned<Item>>,
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
    /// One arena per parsed source file. Modules reference an arena by
    /// [`ArenaId`].
    pub arenas: Vec<Arena>,
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

    /// Look up a module by id, mutably. Used by the loader / resolver
    /// during construction.
    pub(crate) fn module_mut(&mut self, id: ModuleId) -> &mut Module {
        &mut self.modules[id.0]
    }

    /// The crate root module.
    #[must_use]
    pub fn root_module(&self) -> &Module {
        self.module(self.root)
    }

    /// Borrow the arena that backs `module`'s items.
    #[must_use]
    pub fn arena(&self, module: &Module) -> &Arena {
        &self.arenas[module.arena_id.0]
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
            arena_id: ArenaId(0),
            items: Vec::new(),
            bindings: HashMap::new(),
            parent,
            children: Vec::new(),
        }
    }

    #[test]
    fn root_module_lookup() {
        let root_path = ModulePath::crate_root();
        let program = ResolvedProgram {
            arenas: vec![Arena::new()],
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
            arenas: vec![Arena::new(), Arena::new()],
            modules: vec![empty_module(root_path, None), {
                let mut m = empty_module(foo_path.clone(), Some(ModuleId(0)));
                m.arena_id = ArenaId(1);
                m
            }],
            root: ModuleId(0),
        };
        let found = program.find_module(&foo_path).unwrap();
        assert_eq!(found, ModuleId(1));
    }

    #[test]
    fn find_module_returns_none_for_missing() {
        let program = ResolvedProgram {
            arenas: vec![Arena::new()],
            modules: vec![empty_module(ModulePath::crate_root(), None)],
            root: ModuleId(0),
        };
        let missing = ModulePath::crate_root().child("nope");
        assert!(program.find_module(&missing).is_none());
    }

    #[test]
    fn arena_lookup_by_module() {
        use triet_syntax::{Spanned as Sp, TypeExpr};

        let a0 = Arena::new();
        let mut a1 = Arena::new();
        // distinguishable by length: stash a fake type into a1.
        a1.alloc_type(Sp::new(TypeExpr::Named("Marker".to_owned()), 0..6));
        let program = ResolvedProgram {
            arenas: vec![a0, a1],
            modules: vec![empty_module(ModulePath::crate_root(), None), {
                let mut m = empty_module(ModulePath::crate_root().child("foo"), Some(ModuleId(0)));
                m.arena_id = ArenaId(1);
                m
            }],
            root: ModuleId(0),
        };
        let foo = program.module(ModuleId(1));
        assert_eq!(program.arena(foo).type_count(), 1);
        let root = program.root_module();
        assert_eq!(program.arena(root).type_count(), 0);
    }
}
