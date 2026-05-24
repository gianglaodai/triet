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
    collections::{HashMap, HashSet},
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
    let root_chain = vec![root_dir];
    state.load_from_source(
        &ModulePath::crate_root(),
        Some(root_path.to_path_buf()),
        Some(&root_chain),
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
    /// Canonicalized paths of files currently on the loading stack.
    /// Used by [`Self::resolve_external`] to short-circuit when a
    /// `module foo` declaration resolves to a file that is already
    /// being loaded — without this guard the v0.7.x sibling-fallback
    /// chain (e.g. `a.tri` declares `module b`, `b.tri` declares
    /// `module a` and finds the root `a.tri` again) would recurse
    /// indefinitely. The cycle is still reported via the existing
    /// import-graph detector in [`crate::cycle`].
    loading_files: HashSet<PathBuf>,
}

impl LoaderState {
    fn new() -> Self {
        Self {
            program: ResolvedProgram {
                arenas: Vec::new(),
                modules: Vec::new(),
                root: ModuleId(0),
            },
            errors: Vec::new(),
            loading_files: HashSet::new(),
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

        let std_chain = vec![std_dir];
        self.load_from_source(&std_path, None, Some(&std_chain), source, None);
    }

    /// Parse `source`, allocate a fresh arena, slot a [`Module`] into
    /// the program, and recurse on its `module` declarations. Returns
    /// the new module's id, or `None` if parsing failed.
    ///
    /// `source_path` is `Some` for file-bound modules and `None` for
    /// in-memory roots. `search_chain` is `Some` whenever the module's
    /// children may be resolved against the filesystem; a child of an
    /// in-memory root cannot be external, so it stays `None` in that
    /// case. The chain is ordered most-specific first — see
    /// [`Self::resolve_external`] for the resolution algorithm.
    fn load_from_source(
        &mut self,
        path: &ModulePath,
        source_path: Option<PathBuf>,
        search_chain: Option<&[PathBuf]>,
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

        // Push the canonicalized file path onto the loading stack so
        // any descendant `module foo` declaration that re-resolves to
        // this same file is intercepted by `resolve_external`. Falls
        // back to the original `PathBuf` if canonicalization fails
        // (e.g. the file was moved between read and now); the guard
        // still catches the textual identity in that case.
        let guard_path = source_path.as_ref().map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()));
        if let Some(ref gp) = guard_path {
            self.loading_files.insert(gp.clone());
        }

        let arena_id = ArenaId(self.program.arenas.len());
        self.program.arenas.push(parsed.arena);

        let module_id = self.allocate_module(path.clone(), source_path, arena_id, parent);

        let (items, children) =
            self.process_items(path, arena_id, search_chain, module_id, parsed.items);

        let module = self.program.module_mut(module_id);
        module.items = items;
        module.children = children;

        if let Some(gp) = guard_path {
            self.loading_files.remove(&gp);
        }

        Some(module_id)
    }

    /// Inline submodule: items are already parsed in the parent's
    /// arena, so we just slot a [`Module`] entry that points at the
    /// same arena and recurse on its items.
    fn load_inline(
        &mut self,
        path: &ModulePath,
        parent_arena: ArenaId,
        search_chain: Option<&[PathBuf]>,
        parent: ModuleId,
        inline_items: Vec<Spanned<Item>>,
    ) -> ModuleId {
        let module_id = self.allocate_module(path.clone(), None, parent_arena, Some(parent));

        let (items, children) =
            self.process_items(path, parent_arena, search_chain, module_id, inline_items);

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
        parent_search_chain: Option<&[PathBuf]>,
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
                        parent_search_chain,
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
        parent_search_chain: Option<&[PathBuf]>,
        parent_id: ModuleId,
        decl: ModuleDecl,
        decl_span: Span,
    ) -> Option<ModuleId> {
        let child_path = parent_path.child(&decl.name);

        match decl.content {
            ModuleContent::Inline(inline_items) => {
                // Inline `module inner` carves a virtual nested dir
                // (`<primary>/<inner>/`) atop the parent chain so any
                // external child of the inline scope resolves under
                // that subdir first, then walks the chain.
                let child_chain: Option<Vec<PathBuf>> = parent_search_chain.map(|chain| {
                    let primary = chain
                        .first()
                        .cloned()
                        .unwrap_or_else(|| PathBuf::from("."));
                    let mut new_chain = Vec::with_capacity(chain.len() + 1);
                    new_chain.push(primary.join(&decl.name));
                    new_chain.extend(chain.iter().cloned());
                    new_chain
                });
                Some(self.load_inline(
                    &child_path,
                    parent_arena,
                    child_chain.as_deref(),
                    parent_id,
                    inline_items,
                ))
            }
            ModuleContent::External => {
                if let Some(chain) = parent_search_chain {
                    self.resolve_external(&child_path, &decl.name, chain, parent_id, decl_span)
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

    /// Resolve an external `module foo;` declaration against an
    /// ordered chain of search dirs. For each chain slot the loader
    /// tries `<slot>/<name>.tri` (flat) then `<slot>/<name>/<name>.tri`
    /// (nested); the first hit wins.
    ///
    /// The chain is built lazily as the loader descends: each child's
    /// chain starts with its own nested dir (so its own children stay
    /// in their conventional location) and then inherits the slots
    /// from whichever ancestor's chain matched, so siblings further
    /// up the tree remain reachable. This unlocks the
    /// `compiler/main.tri` → `compiler/pack_writer.tri` → sibling
    /// `compiler/ir_lowerer.tri` import pattern that pre-v0.7.9.4
    /// would have failed at the second hop because the previous
    /// loader nested unconditionally.
    fn resolve_external(
        &mut self,
        child_path: &ModulePath,
        name: &str,
        parent_chain: &[PathBuf],
        parent: ModuleId,
        decl_span: Span,
    ) -> Option<ModuleId> {
        let mut matched: Option<(usize, PathBuf)> = None;
        let mut first_flat: Option<PathBuf> = None;
        let mut first_nested: Option<PathBuf> = None;

        for (idx, search_dir) in parent_chain.iter().enumerate() {
            let flat = search_dir.join(format!("{name}.tri"));
            let nested = search_dir.join(name).join(format!("{name}.tri"));
            if first_flat.is_none() {
                first_flat = Some(flat.clone());
            }
            if first_nested.is_none() {
                first_nested = Some(nested.clone());
            }
            if flat.is_file() {
                matched = Some((idx, flat));
                break;
            }
            if nested.is_file() {
                matched = Some((idx, nested));
                break;
            }
        }

        let Some((matched_idx, source_path)) = matched else {
            self.errors.push(LoaderError::FileNotFound {
                module_name: name.to_owned(),
                searched_primary: first_flat.unwrap_or_default().display().to_string(),
                searched_nested: first_nested.unwrap_or_default().display().to_string(),
                span: decl_span,
            });
            return None;
        };

        // If the resolved file is already loading we would recurse
        // forever — skip silently and let the import-graph cycle
        // detector in [`crate::cycle`] surface the actual cycle via
        // `from crate.x import …` / `import crate.x` edges instead.
        // A `module foo;` decl that maps back to an ancestor file is
        // structurally the same cycle the from-import edges express.
        let canonical_resolved = source_path
            .canonicalize()
            .unwrap_or_else(|_| source_path.clone());
        if self.loading_files.contains(&canonical_resolved) {
            return None;
        }

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

        // Build the child's chain from the slot that matched. Drop the
        // shorter slots ahead of `matched_idx`: they failed for this
        // child, so its grandchildren should not waste a stat on them
        // either. Children of `name` live first in the conventional
        // `<matched_dir>/<name>/` subdir, then inherit the chain from
        // `matched_dir` onwards.
        let matched_dir = &parent_chain[matched_idx];
        let mut child_chain: Vec<PathBuf> = Vec::with_capacity(parent_chain.len() - matched_idx + 1);
        child_chain.push(matched_dir.join(name));
        child_chain.extend(parent_chain[matched_idx..].iter().cloned());

        self.load_from_source(
            child_path,
            Some(source_path),
            Some(&child_chain),
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

    /// Regression for v0.7.x.runtime-fix.loader-nested-search. A flat
    /// sibling that declares `module other` (another flat sibling)
    /// used to fail because the loader hard-nested the search dir to
    /// `<parent>/sibling/` even though `sibling` itself was found at
    /// `<parent>/sibling.tri`. The fix walks an ordered chain of
    /// search dirs and inherits siblings from whichever ancestor's
    /// chain matched.
    #[test]
    fn filesystem_sibling_imports_other_sibling() {
        let result = load_files(&[
            ("main.tri", "module pack_writer"),
            ("pack_writer.tri", "module ir_lowerer\nmodule typecheck"),
            ("ir_lowerer.tri", "public function lower() = 1"),
            ("typecheck.tri", "public function check() = 2"),
        ])
        .unwrap();
        let pack_writer = result.module(result.root_module().children[0]);
        assert_eq!(pack_writer.path.to_string(), "crate.pack_writer");
        assert_eq!(pack_writer.children.len(), 2);
        let ir_lowerer = result.module(pack_writer.children[0]);
        assert_eq!(ir_lowerer.path.to_string(), "crate.pack_writer.ir_lowerer");
        let typecheck = result.module(pack_writer.children[1]);
        assert_eq!(typecheck.path.to_string(), "crate.pack_writer.typecheck");
    }

    /// The conventional nested layout (`std/collections.tri` with
    /// children at `std/collections/vector.tri`) must keep winning
    /// over the new sibling-fallback path — the chain prefers the
    /// most-specific dir first.
    #[test]
    fn filesystem_nested_layout_still_preferred_over_sibling() {
        let result = load_files(&[
            ("main.tri", "module pkg"),
            ("pkg.tri", "module child"),
            ("pkg/child.tri", "public function nested() = 1"),
            // Decoy sibling at the parent level — the conventional
            // nested location should be preferred.
            ("child.tri", "public function decoy() = 99"),
        ])
        .unwrap();
        let pkg = result.module(result.root_module().children[0]);
        let child = result.module(pkg.children[0]);
        assert_eq!(child.path.to_string(), "crate.pkg.child");
        // The picked file is the nested one, not the decoy sibling.
        assert!(
            child
                .source_path
                .as_ref()
                .unwrap()
                .to_string_lossy()
                .ends_with("pkg/child.tri"),
            "expected pkg/child.tri, got {:?}",
            child.source_path
        );
    }
}
