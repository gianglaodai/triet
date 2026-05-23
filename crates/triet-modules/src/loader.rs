//! File loader — turns a root `.tri` file (or in-memory source) into a
//! [`ResolvedProgram`] populated with every reachable module.
//!
//! The loader walks `module foo` declarations starting from the root,
//! resolves each external declaration against the filesystem (per
//! ADR-0005 §"File resolution": `foo.tri` first, `foo/foo.tri`
//! fallback), recurses into inline `module foo { … }` bodies, and
//! produces one [`Module`] per declared scope. Inline submodules share
//! their parent's arena; file-bound submodules each get a fresh one.
//!
//! After all modules are loaded, the loader runs cycle detection
//! (#36.3) on the import graph. Name resolution + visibility checking
//! happen later (#36.4).

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use triet_parser::parse;
use triet_syntax::{Item, ModuleContent, ModuleDecl, Span, Spanned};

use crate::{
    cycle::detect_cycles,
    error::LoaderError,
    module::{ArenaId, Module, ModuleId, ResolvedProgram},
    path::ModulePath,
    resolver::resolve_names,
};

/// Filesystem-driven entry point. See [`crate::load_program`].
pub(crate) fn load_filesystem(root_path: &Path) -> Result<ResolvedProgram, Vec<LoaderError>> {
    let source = match fs::read_to_string(root_path) {
        Ok(text) => text,
        Err(error) => {
            return Err(vec![LoaderError::IoError {
                path: root_path.display().to_string(),
                message: error.to_string(),
                span: 0..0,
            }]);
        }
    };

    let root_dir = root_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    let mut state = LoaderState::new();
    state.load_stdlib();
    state.load_from_source(
        &ModulePath::crate_root(),
        Some(root_path.to_path_buf()),
        Some(&root_dir),
        &source,
        None,
    );
    state.finish()
}

/// In-memory entry point. See [`crate::load_program_from_source`].
pub(crate) fn load_in_memory(source: &str) -> Result<ResolvedProgram, Vec<LoaderError>> {
    let mut state = LoaderState::new();
    state.load_stdlib();
    state.load_from_source(&ModulePath::crate_root(), None, None, source, None);
    state.finish()
}

/// Mutable state threaded through the recursive load.
struct LoaderState {
    program: ResolvedProgram,
    errors: Vec<LoaderError>,
}

impl LoaderState {
    const fn new() -> Self {
        Self {
            program: ResolvedProgram {
                arenas: Vec::new(),
                modules: Vec::new(),
                root: ModuleId(0),
            },
            errors: Vec::new(),
        }
    }

    fn finish(mut self) -> Result<ResolvedProgram, Vec<LoaderError>> {
        // Run cycle detection on the import graph before returning.
        // This catches cyclic imports (E2100) even if loading itself
        // succeeded. Cycle errors are appended to any existing errors.
        let cycle_errors = detect_cycles(&self.program);
        self.errors.extend(cycle_errors);

        // If there are load/cycle errors, bail before name resolution
        // — resolving imports in a broken program is meaningless.
        if !self.errors.is_empty() {
            return Err(self.errors);
        }

        // Phase 3: Name resolution — bind definitions, resolve imports,
        // validate visibility. Runs only on cycle-free programs.
        let resolve_errors = resolve_names(&mut self.program);
        self.errors.extend(resolve_errors);

        if self.errors.is_empty() {
            Ok(self.program)
        } else {
            Err(self.errors)
        }
    }

    /// Load the stdlib tree into the program.
    ///
    /// v0.2.x.7: stdlib children are resolved from real `.tri` files in the
    /// `std/` directory via the normal filesystem path (replacing the v0.2.x.6
    /// `include_str!` hack). The `std` container module itself is still
    /// synthetic — it declares `module io`, `module text`, `module assert` as
    /// external children.
    ///
    /// The stdlib directory is resolved relative to the workspace root (via
    /// `CARGO_MANIFEST_DIR` at compile time) and also checked relative to the
    /// current working directory at runtime.
    fn load_stdlib(&mut self) {
        let std_path = ModulePath::new(vec!["std".to_owned()]);
        // `std.result` carries the `Result<T, E>` enum per SPEC §2.5
        // (v0.4): primary error-handling type when `T?` isn't enough.
        //
        // v0.7.4.2 (ADR-0019 Addendum §A7) — added `collections`,
        // `path`, `string` modules to surface Vector/HashMap/IO/path
        // builtins shipped in v0.7.3. Function names within these
        // modules follow existing precedent (`std.io.println` not
        // `std.io.io_println`) — no module-name repetition.
        let source = "module io\nmodule text\nmodule assert\nmodule result\n\
             module collections\nmodule path\nmodule string\nmodule crypto";

        // Resolve std/ relative to the workspace root (for dev/production).
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // triet-modules crate is at <workspace>/crates/triet-modules
        let workspace_std = manifest_dir.join("..").join("..").join("std");

        let std_dir = if workspace_std.is_dir() {
            workspace_std
        } else {
            // Fall back to CWD-relative (for installed toolchains).
            PathBuf::from("std")
        };

        self.load_from_source(&std_path, None, Some(&std_dir), source, None);
    }

    /// Parse `source`, allocate a fresh arena, slot a [`Module`] into
    /// the program, and recurse on its `module` declarations. Returns
    /// the new module's id, or `None` if parsing failed.
    ///
    /// `source_path` is `Some` for file-bound modules and `None` for
    /// in-memory roots. `search_dir` is `Some` whenever the module's
    /// children may be resolved against the filesystem; a child of an
    /// in-memory root cannot be external, so it stays `None` in that
    /// case.
    fn load_from_source(
        &mut self,
        path: &ModulePath,
        source_path: Option<PathBuf>,
        search_dir: Option<&Path>,
        source: &str,
        parent: Option<ModuleId>,
    ) -> Option<ModuleId> {
        let (parsed, parse_errors) = parse(source);
        if !parse_errors.is_empty() {
            for error in parse_errors {
                self.errors.push(LoaderError::ChildParseError {
                    module: path.to_string(),
                    message: error.to_string(),
                    span: error.span(),
                });
            }
            return None;
        }

        let arena_id = ArenaId(self.program.arenas.len());
        self.program.arenas.push(parsed.arena);

        let module_id = self.allocate_module(path.clone(), source_path, arena_id, parent);

        let (items, children) =
            self.process_items(path, arena_id, search_dir, module_id, parsed.items);

        let module = self.program.module_mut(module_id);
        module.items = items;
        module.children = children;

        Some(module_id)
    }

    /// Inline submodule: items are already parsed in the parent's
    /// arena, so we just slot a [`Module`] entry that points at the
    /// same arena and recurse on its items.
    fn load_inline(
        &mut self,
        path: &ModulePath,
        parent_arena: ArenaId,
        search_dir: Option<&Path>,
        parent: ModuleId,
        inline_items: Vec<Spanned<Item>>,
    ) -> ModuleId {
        let module_id = self.allocate_module(path.clone(), None, parent_arena, Some(parent));

        let (items, children) =
            self.process_items(path, parent_arena, search_dir, module_id, inline_items);

        let module = self.program.module_mut(module_id);
        module.items = items;
        module.children = children;

        module_id
    }

    /// Reserve a [`ModuleId`] and push a placeholder [`Module`]. The
    /// caller fills in `items` and `children` after recursing.
    fn allocate_module(
        &mut self,
        path: ModulePath,
        source_path: Option<PathBuf>,
        arena_id: ArenaId,
        parent: Option<ModuleId>,
    ) -> ModuleId {
        let module_id = ModuleId(self.program.modules.len());
        if parent.is_none() {
            self.program.root = module_id;
        }
        self.program.modules.push(Module {
            path,
            source_path,
            arena_id,
            items: Vec::new(),
            bindings: HashMap::new(),
            parent,
            children: Vec::new(),
        });
        module_id
    }

    /// Walk a module's items: recurse into `module` declarations,
    /// retain everything else as the module's own item list.
    fn process_items(
        &mut self,
        parent_path: &ModulePath,
        parent_arena: ArenaId,
        parent_search_dir: Option<&Path>,
        parent_id: ModuleId,
        items: Vec<Spanned<Item>>,
    ) -> (Vec<Spanned<Item>>, Vec<ModuleId>) {
        let mut retained = Vec::new();
        let mut children = Vec::new();

        for item in items {
            match item.node {
                Item::Module(decl) => {
                    if let Some(child_id) = self.process_module_decl(
                        parent_path,
                        parent_arena,
                        parent_search_dir,
                        parent_id,
                        decl,
                        item.span.clone(),
                    ) {
                        children.push(child_id);
                    }
                }
                _ => retained.push(item),
            }
        }

        (retained, children)
    }

    fn process_module_decl(
        &mut self,
        parent_path: &ModulePath,
        parent_arena: ArenaId,
        parent_search_dir: Option<&Path>,
        parent_id: ModuleId,
        decl: ModuleDecl,
        decl_span: Span,
    ) -> Option<ModuleId> {
        let child_path = parent_path.child(&decl.name);
        let child_search_dir = parent_search_dir.map(|directory| directory.join(&decl.name));

        match decl.content {
            ModuleContent::Inline(inline_items) => Some(self.load_inline(
                &child_path,
                parent_arena,
                child_search_dir.as_deref(),
                parent_id,
                inline_items,
            )),
            ModuleContent::External => {
                if let Some(directory) = parent_search_dir {
                    self.resolve_external(&child_path, &decl.name, directory, parent_id, decl_span)
                } else {
                    self.errors.push(LoaderError::FileNotFound {
                        module_name: decl.name,
                        searched_primary: "(no filesystem context — root loaded from in-memory source or inside an inline parent)".to_owned(),
                        searched_nested: String::new(),
                        span: decl_span,
                    });
                    None
                }
            }
        }
    }

    /// Resolve an external `module foo;` declaration: look for
    /// `<dir>/<name>.tri`, then `<dir>/<name>/<name>.tri`. Read and
    /// recurse on whichever exists.
    fn resolve_external(
        &mut self,
        child_path: &ModulePath,
        name: &str,
        search_dir: &Path,
        parent: ModuleId,
        decl_span: Span,
    ) -> Option<ModuleId> {
        let flat = search_dir.join(format!("{name}.tri"));
        let nested = search_dir.join(name).join(format!("{name}.tri"));

        let source_path = if flat.is_file() {
            flat
        } else if nested.is_file() {
            nested
        } else {
            self.errors.push(LoaderError::FileNotFound {
                module_name: name.to_owned(),
                searched_primary: flat.display().to_string(),
                searched_nested: nested.display().to_string(),
                span: decl_span,
            });
            return None;
        };

        let source = match fs::read_to_string(&source_path) {
            Ok(text) => text,
            Err(error) => {
                self.errors.push(LoaderError::IoError {
                    path: source_path.display().to_string(),
                    message: error.to_string(),
                    span: decl_span,
                });
                return None;
            }
        };

        // Children of `name` live in `<search_dir>/<name>/`, regardless
        // of whether `name` itself is backed by `<search_dir>/<name>.tri`
        // (flat) or `<search_dir>/<name>/<name>.tri` (nested).
        let child_search_dir = search_dir.join(name);

        self.load_from_source(
            child_path,
            Some(source_path),
            Some(&child_search_dir),
            &source,
            Some(parent),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── In-memory tests ─────────────────────────────────────────────

    /// Stdlib module count baseline. Updated by v0.7.4.2 from 5 → 11
    /// (added: collections, collections.vector, collections.hashmap,
    /// io.fs, path, string). Crate root contributes +1 → 12 modules
    /// for an empty program. v0.7.9.3 adds `crypto` (carrying the
    /// `blake3_hash` stub) for the .tripack writer's iface/impl
    /// hash chain → 13 modules. Centralized here so future stdlib
    /// expansions only touch one place.
    const STDLIB_MODULE_COUNT_WITH_CRATE_ROOT: usize = 13;

    #[test]
    fn empty_root_creates_one_module() {
        let result = load_in_memory("").unwrap();
        assert_eq!(result.modules.len(), STDLIB_MODULE_COUNT_WITH_CRATE_ROOT);
        assert_eq!(result.root_module().path, ModulePath::crate_root());
        assert!(result.root_module().items.is_empty());
        assert!(result.root_module().children.is_empty());
    }

    #[test]
    fn root_with_function_only_keeps_item() {
        let source = "function main() { }";
        let result = load_in_memory(source).unwrap();
        assert_eq!(result.modules.len(), STDLIB_MODULE_COUNT_WITH_CRATE_ROOT);
        assert_eq!(result.root_module().items.len(), 1);
        assert!(result.root_module().children.is_empty());
    }

    #[test]
    fn inline_module_creates_child() {
        let source = r"
            module helper {
                function aid() = 1
            }
        ";
        let result = load_in_memory(source).unwrap();
        assert_eq!(
            result.modules.len(),
            STDLIB_MODULE_COUNT_WITH_CRATE_ROOT + 1
        );

        let root = result.root_module();
        assert!(root.items.is_empty(), "module decl should be lifted out");
        assert_eq!(root.children.len(), 1);

        let child_id = root.children[0];
        let child = result.module(child_id);
        assert_eq!(child.path.to_string(), "crate.helper");
        assert_eq!(child.items.len(), 1);
        assert_eq!(child.parent, Some(result.root));
        // Inline child shares parent's arena.
        assert_eq!(child.arena_id, root.arena_id);
    }

    #[test]
    fn nested_inline_modules() {
        let source = r"
            module outer {
                module inner {
                    function ping() = 1
                }
            }
        ";
        let result = load_in_memory(source).unwrap();
        assert_eq!(
            result.modules.len(),
            STDLIB_MODULE_COUNT_WITH_CRATE_ROOT + 2
        );

        let outer_id = result.root_module().children[0];
        let outer = result.module(outer_id);
        assert_eq!(outer.path.to_string(), "crate.outer");
        assert_eq!(outer.children.len(), 1);

        let inner_id = outer.children[0];
        let inner = result.module(inner_id);
        assert_eq!(inner.path.to_string(), "crate.outer.inner");
        assert_eq!(inner.parent, Some(outer_id));
    }

    #[test]
    fn external_in_memory_root_errors() {
        // No filesystem context → external child cannot be resolved.
        let source = "module foo";
        let errors = load_in_memory(source).unwrap_err();
        assert!(matches!(errors[0], LoaderError::FileNotFound { .. }));
    }

    #[test]
    fn parse_error_propagates_with_module_attribution() {
        let source = "function this is not valid syntax";
        let errors = load_in_memory(source).unwrap_err();
        assert!(matches!(errors[0], LoaderError::ChildParseError { .. }));
        if let LoaderError::ChildParseError { module, .. } = &errors[0] {
            assert_eq!(module, "crate");
        }
    }

    /// Stdlib arenas: 1 (std synthetic root) + 11 (one per stdlib
    /// .tri file: io, io/fs, text, assert, result, collections,
    /// collections/vector, collections/hashmap, path, string, crypto).
    /// Crate root contributes +1 = 13 total when inline modules
    /// share the crate's arena.
    const STDLIB_ARENA_COUNT_WITH_CRATE_ROOT: usize = 13;

    #[test]
    fn inline_modules_share_root_arena() {
        let source = r"
            function root_fn() = 1
            module helper {
                function aid() = 2
            }
        ";
        let result = load_in_memory(source).unwrap();
        let root = result.root_module();
        let helper = result.module(root.children[0]);
        assert_eq!(root.arena_id, helper.arena_id);
        // Single arena allocated for the whole inline tree —
        // see STDLIB_ARENA_COUNT_WITH_CRATE_ROOT above for breakdown.
        assert_eq!(result.arenas.len(), STDLIB_ARENA_COUNT_WITH_CRATE_ROOT);
    }

    // ── Filesystem tests ───────────────────────────────────────────

    /// Lay out a temp directory with a set of files, run the loader,
    /// and return the result. The first file in `files` is taken as
    /// the root.
    fn load_files(files: &[(&str, &str)]) -> Result<ResolvedProgram, Vec<LoaderError>> {
        let temp = tempfile::tempdir().expect("tempdir");
        let base = temp.path();

        let mut root_path: Option<PathBuf> = None;
        for (rel_path, contents) in files {
            let full = base.join(rel_path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).expect("create_dir_all");
            }
            fs::write(&full, contents).expect("write");
            if root_path.is_none() {
                root_path = Some(full);
            }
        }

        let result = load_filesystem(root_path.as_ref().expect("at least one file"));
        // Keep the tempdir alive until result is consumed.
        drop(temp);
        result
    }

    #[test]
    fn filesystem_root_with_no_modules() {
        let result = load_files(&[("main.tri", "function main() { }")]).unwrap();
        assert_eq!(result.modules.len(), STDLIB_MODULE_COUNT_WITH_CRATE_ROOT);
        assert_eq!(result.root_module().items.len(), 1);
        assert!(result.root_module().source_path.is_some());
    }

    #[test]
    fn filesystem_resolves_flat_child() {
        let result = load_files(&[
            ("main.tri", "module helper"),
            ("helper.tri", "public function aid() = 7"),
        ])
        .unwrap();
        assert_eq!(
            result.modules.len(),
            STDLIB_MODULE_COUNT_WITH_CRATE_ROOT + 1
        );
        let helper = result.module(result.root_module().children[0]);
        assert_eq!(helper.path.to_string(), "crate.helper");
        assert_eq!(helper.items.len(), 1);
        // External child gets its own arena.
        assert_ne!(helper.arena_id, result.root_module().arena_id);
        // Stdlib arenas (12: std synthetic + 11 .tri files) + crate
        // root arena + external child file arena = 14.
        assert_eq!(result.arenas.len(), 14);
    }

    #[test]
    fn filesystem_resolves_nested_child() {
        let result = load_files(&[
            ("main.tri", "module helper"),
            ("helper/helper.tri", "module inner"),
            ("helper/inner.tri", "public function ping() = 1"),
        ])
        .unwrap();
        assert_eq!(
            result.modules.len(),
            STDLIB_MODULE_COUNT_WITH_CRATE_ROOT + 2
        );
        let helper = result.module(result.root_module().children[0]);
        assert_eq!(helper.children.len(), 1);
        let inner = result.module(helper.children[0]);
        assert_eq!(inner.path.to_string(), "crate.helper.inner");
    }

    #[test]
    fn filesystem_missing_file_errors() {
        let errors = load_files(&[("main.tri", "module ghost")]).unwrap_err();
        assert!(matches!(errors[0], LoaderError::FileNotFound { .. }));
        if let LoaderError::FileNotFound { module_name, .. } = &errors[0] {
            assert_eq!(module_name, "ghost");
        }
    }

    #[test]
    fn filesystem_child_parse_error_attributed() {
        let errors = load_files(&[
            ("main.tri", "module broken"),
            ("broken.tri", "function this is invalid"),
        ])
        .unwrap_err();
        assert!(matches!(errors[0], LoaderError::ChildParseError { .. }));
        if let LoaderError::ChildParseError { module, .. } = &errors[0] {
            assert_eq!(module, "crate.broken");
        }
    }

    #[test]
    fn deep_filesystem_tree() {
        let result = load_files(&[
            ("main.tri", "module a"),
            ("a/a.tri", "module b"),
            ("a/b/b.tri", "module c"),
            ("a/b/c/c.tri", "function leaf() = 0"),
        ])
        .unwrap();
        assert_eq!(
            result.modules.len(),
            STDLIB_MODULE_COUNT_WITH_CRATE_ROOT + 3
        );
        let leaf = result
            .find_module(&ModulePath::new(
                ["crate", "a", "b", "c"]
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
            ))
            .unwrap();
        assert_eq!(result.module(leaf).items.len(), 1);
    }

    #[test]
    fn missing_root_file_io_error() {
        let temp = tempfile::tempdir().unwrap();
        let nonexistent = temp.path().join("nope.tri");
        let errors = load_filesystem(&nonexistent).unwrap_err();
        assert!(matches!(errors[0], LoaderError::IoError { .. }));
    }
}
