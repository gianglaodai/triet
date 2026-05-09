//! Name resolution and visibility checking.
//!
//! After the file loader has built the module tree and cycle detection
//! has passed, this pass:
//!
//! 1. **Binds each module's own definitions** — every function, struct,
//!    enum, const, and type alias declared in a module goes into its
//!    `bindings` map keyed by the item's name, with an
//!    [`AbsolutePath`] rooted at the module's path.
//!
//! 2. **Resolves imports** — rewrites `from X import Y` and `import X`
//!    to absolute paths, resolving `crate.`, `self.`, and `super.`
//!    path keywords relative to the importing module. For `from`
//!    imports, each imported name is bound into the importing module's
//!    scope. For whole-module `import`, the module path is bound under
//!    its last segment.
//!
//! 3. **Checks visibility** — an imported name must be `public` (or
//!    `public(package)` within the same crate). Private items cannot
//!    be imported from outside their defining module.
//!
//! 4. **Binds synthetic stdlib exports** — `from std.io import println`
//!    resolves through the stdlib registry (v0.2.x.6 synthetic;
//!    v0.2.x.7 swaps to real files).
//!
//! Errors emitted: [`LoaderError::UnresolvedImport`] (E2104),
//! [`LoaderError::VisibilityViolation`] (E2103),
//! [`LoaderError::ReservedNamespace`] (E2102).

use triet_syntax::{Item, Visibility};

use crate::{
    error::LoaderError,
    module::{ModuleId, ResolvedProgram},
    path::{AbsolutePath, ModulePath},
};

/// Run name resolution on every module in the program.
///
/// Populates each module's `bindings` map. Returns a (possibly empty)
/// list of errors for unresolved imports and visibility violations.
pub(crate) fn resolve_names(program: &mut ResolvedProgram) -> Vec<LoaderError> {
    let mut errors = Vec::new();

    // Phase 1: Bind each module's own definitions.
    // We need to collect the bindings first, then apply them, because
    // we can't borrow `program` mutably while iterating.
    let own_bindings: Vec<Vec<(String, AbsolutePath)>> = program
        .modules
        .iter()
        .map(|module| {
            let mut bindings = Vec::new();
            for item in &module.items {
                if let Some((name, visibility)) = item_name_and_visibility(&item.node) {
                    let abs_path = AbsolutePath::new(module.path.clone(), name.clone());
                    let _ = visibility; // Visibility is used when *importing*, not when binding own defs.
                    bindings.push((name, abs_path));
                }
            }
            // Also bind child modules — `module foo` creates a name
            // `foo` in the parent's scope pointing at `crate.foo`.
            for &child_id in &module.children {
                let child = &program.modules[child_id.raw()];
                let child_name = child
                    .path
                    .segments()
                    .last()
                    .cloned()
                    .unwrap_or_default();
                // Module bindings use the module path as-is (the module
                // itself, not an item inside it). We represent this as an
                // AbsolutePath where the name *is* the last segment and
                // the module is the parent path.
                let abs_path = AbsolutePath::new(
                    module.path.clone(),
                    child_name.clone(),
                );
                bindings.push((child_name, abs_path));
            }
            bindings
        })
        .collect();

    for (idx, bindings) in own_bindings.into_iter().enumerate() {
        for (name, abs_path) in bindings {
            program.modules[idx].bindings.insert(name, abs_path);
        }
    }

    // Phase 2: Resolve imports.
    // Collect import info for each module, then process.
    let import_infos: Vec<Vec<ImportInfo>> = program
        .modules
        .iter()
        .map(|module| collect_imports(module.path.clone(), &module.items))
        .collect();

    for (idx, infos) in import_infos.into_iter().enumerate() {
        for info in infos {
            resolve_single_import(program, ModuleId(idx), info, &mut errors);
        }
    }

    errors
}

/// Information about a single import statement, extracted from the AST.
struct ImportInfo {
    /// The kind of import.
    kind: ImportKind,
    /// Span of the import statement for error reporting.
    span: triet_syntax::Span,
}

/// Differentiate between `import X` and `from X import Y`.
enum ImportKind {
    /// `import std.io` or `import std.io.println`.
    Whole {
        segments: Vec<String>,
    },
    /// `from std.io import println, print as p`.
    From {
        source: Vec<String>,
        names: Vec<(String, Option<String>)>,
    },
}

/// Extract import statements from a module's items.
fn collect_imports(
    _module_path: ModulePath,
    items: &[triet_syntax::Spanned<Item>],
) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    for item in items {
        match &item.node {
            Item::Import(import_path) => {
                imports.push(ImportInfo {
                    kind: ImportKind::Whole {
                        segments: import_path.segments.clone(),
                    },
                    span: item.span.clone(),
                });
            }
            Item::ImportFrom(import_from) => {
                let names = import_from
                    .names
                    .iter()
                    .map(|n| (n.name.clone(), n.alias.clone()))
                    .collect();
                imports.push(ImportInfo {
                    kind: ImportKind::From {
                        source: import_from.source.clone(),
                        names,
                    },
                    span: item.span.clone(),
                });
            }
            _ => {}
        }
    }
    imports
}

/// Resolve a single import statement and bind names into the importing
/// module's scope.
fn resolve_single_import(
    program: &mut ResolvedProgram,
    importer_id: ModuleId,
    info: ImportInfo,
    errors: &mut Vec<LoaderError>,
) {
    match info.kind {
        ImportKind::Whole { segments } => {
            resolve_whole_import(program, importer_id, &segments, info.span, errors);
        }
        ImportKind::From { source, names } => {
            resolve_from_import(program, importer_id, &source, &names, info.span, errors);
        }
    }
}

/// Resolve `import X.Y.Z` — bind the last segment as a name in the
/// importing module's scope.
fn resolve_whole_import(
    program: &mut ResolvedProgram,
    importer_id: ModuleId,
    segments: &[String],
    span: triet_syntax::Span,
    errors: &mut Vec<LoaderError>,
) {
    if segments.is_empty() {
        return;
    }

    let resolved = resolve_path_keywords(program, importer_id, segments);

    // Check reserved namespace (sys/dev/usr — not yet usable).
    if let Some(root) = resolved.first()
        && matches!(root.as_str(), "sys" | "dev" | "usr")
    {
        errors.push(LoaderError::ReservedNamespace {
            root: root.clone(),
            span,
        });
        return;
    }

    let target_path = ModulePath::new(resolved.clone());

    // Try as module path first.
    if program.find_module(&target_path).is_some() {
        // Bind under the last segment name.
        let bind_name = segments.last().unwrap().clone();
        let abs_path = AbsolutePath::new(target_path, bind_name.clone());
        program.modules[importer_id.raw()].bindings.insert(bind_name, abs_path);
        return;
    }

    // Try as module.item — drop last segment as item name.
    if resolved.len() > 1 {
        let module_segments = &resolved[..resolved.len() - 1];
        let item_name = resolved.last().unwrap();
        let module_path = ModulePath::new(module_segments.to_vec());



        // Check user module.
        if let Some(target_mod_id) = program.find_module(&module_path) {
            // Visibility check.
            if let Some(vis) = find_item_visibility(program, target_mod_id, item_name) {
                if !is_visible(vis, importer_id, target_mod_id, program) {
                    errors.push(LoaderError::VisibilityViolation {
                        name: item_name.clone(),
                        actual_visibility: vis.to_string(),
                        span,
                    });
                    return;
                }
                let abs_path = AbsolutePath::new(module_path, item_name.clone());
                let bind_name = segments.last().unwrap().clone();
                program.modules[importer_id.raw()].bindings.insert(bind_name, abs_path);
                return;
            }
        }
    }

    errors.push(LoaderError::UnresolvedImport {
        path: segments.join("."),
        span,
    });
}

/// Resolve `from X import a, b as c` — resolve source path, then
/// look up each name in the target module and bind it.
fn resolve_from_import(
    program: &mut ResolvedProgram,
    importer_id: ModuleId,
    source: &[String],
    names: &[(String, Option<String>)],
    span: triet_syntax::Span,
    errors: &mut Vec<LoaderError>,
) {
    if source.is_empty() {
        return;
    }

    let resolved = resolve_path_keywords(program, importer_id, source);

    // Check reserved namespace.
    if let Some(root) = resolved.first()
        && matches!(root.as_str(), "sys" | "dev" | "usr")
    {
        errors.push(LoaderError::ReservedNamespace {
            root: root.clone(),
            span,
        });
        return;
    }

    let target_path = ModulePath::new(resolved);



    // User module.
    let Some(target_mod_id) = program.find_module(&target_path) else {
        errors.push(LoaderError::UnresolvedImport {
            path: source.join("."),
            span,
        });
        return;
    };

    for (name, alias) in names {
        // Check the name exists in the target module.
        let Some(vis) = find_item_visibility(program, target_mod_id, name) else {
            // Also check child modules.
            let child_path = target_path.child(name);
            if program.find_module(&child_path).is_some() {
                let abs_path = AbsolutePath::new(target_path.clone(), name.clone());
                let bind_name = alias.as_ref().unwrap_or(name).clone();
                program.modules[importer_id.raw()].bindings.insert(bind_name, abs_path);
                continue;
            }
            errors.push(LoaderError::UnresolvedImport {
                path: format!("{}.{}", source.join("."), name),
                span: span.clone(),
            });
            continue;
        };

        // Visibility check.
        if !is_visible(vis, importer_id, target_mod_id, program) {
            errors.push(LoaderError::VisibilityViolation {
                name: name.clone(),
                actual_visibility: vis.to_string(),
                span: span.clone(),
            });
            continue;
        }

        let abs_path = AbsolutePath::new(target_path.clone(), name.clone());
        let bind_name = alias.as_ref().unwrap_or(name).clone();
        program.modules[importer_id.raw()].bindings.insert(bind_name, abs_path);
    }
}

/// Resolve `crate.`, `self.`, `super.` path keywords to absolute
/// segments.
fn resolve_path_keywords(
    program: &ResolvedProgram,
    importer_id: ModuleId,
    segments: &[String],
) -> Vec<String> {
    if segments.is_empty() {
        return Vec::new();
    }

    let importer = program.module(importer_id);

    match segments[0].as_str() {
        "crate" => {
            // Already absolute — return as-is.
            segments.to_vec()
        }
        "self" => {
            // Replace `self` with the importer's module path segments.
            let mut result = importer.path.segments().to_vec();
            result.extend_from_slice(&segments[1..]);
            result
        }
        "super" => {
            // Replace `super` with the parent's path segments.
            importer.path.parent().map_or_else(
                // Already at root — `super` is invalid. Return as-is;
                // the resolver will emit UnresolvedImport.
                || segments.to_vec(),
                |parent_path| {
                    let mut result = parent_path.segments().to_vec();
                    result.extend_from_slice(&segments[1..]);
                    result
                },
            )
        }
        _ => {
            // No path keyword prefix — return as-is (could be `std.*`
            // or bare name).
            segments.to_vec()
        }
    }
}

/// Extract name and visibility from an item, if it defines a named entity.
fn item_name_and_visibility(item: &Item) -> Option<(String, Visibility)> {
    match item {
        Item::Function(f) => Some((f.name.clone(), f.visibility)),
        Item::Const { name, visibility, .. }
        | Item::TypeAlias { name, visibility, .. } => Some((name.clone(), *visibility)),
        Item::Struct(s) => Some((s.name.clone(), s.visibility)),
        Item::Enum(e) => Some((e.name.clone(), e.visibility)),
        Item::Import(_) | Item::ImportFrom(_) | Item::Module(_) => None,
    }
}

/// Find the visibility of a named item in a module.
fn find_item_visibility(
    program: &ResolvedProgram,
    module_id: ModuleId,
    name: &str,
) -> Option<Visibility> {
    let module = program.module(module_id);
    for item in &module.items {
        if let Some((item_name, vis)) = item_name_and_visibility(&item.node)
            && item_name == name
        {
            return Some(vis);
        }
    }
    None
}

/// Check whether an item with the given visibility in `target_mod` is
/// visible to `importer_mod`.
fn is_visible(
    vis: Visibility,
    importer_id: ModuleId,
    target_mod_id: ModuleId,
    _program: &ResolvedProgram,
) -> bool {
    match vis {
        Visibility::Public | Visibility::PublicPackage => {
            // Public: visible everywhere.
            // PublicPackage: same crate — always visible. Cross-crate
            // packs are a v0.4+ feature; at v0.2.x everything is
            // single-crate.
            true
        }
        Visibility::Private => {
            // Private items are only visible within their own module.
            importer_id == target_mod_id
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::loader;

    use super::*;

    // ── Helpers ─────────────────────────────────────────────────────

    fn load_in_memory_result(source: &str) -> Result<ResolvedProgram, Vec<LoaderError>> {
        loader::load_in_memory(source)
    }

    fn load_filesystem_result(
        files: &[(&str, &str)],
    ) -> Result<ResolvedProgram, Vec<LoaderError>> {
        let temp = tempfile::tempdir().expect("tempdir");
        let base = temp.path();
        let mut root_path = None;

        for (rel_path, contents) in files {
            let full = base.join(rel_path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("create_dir_all");
            }
            std::fs::write(&full, contents).expect("write");
            if root_path.is_none() {
                root_path = Some(full);
            }
        }

        loader::load_filesystem(root_path.as_ref().expect("at least one file"))
    }

    // ── Own-definition binding ──────────────────────────────────────

    #[test]
    fn own_function_bound() {
        let program =
            load_in_memory_result("function main() { }").unwrap();
        let root = program.root_module();
        assert!(
            root.bindings.contains_key("main"),
            "own function should be bound: {:?}",
            root.bindings
        );
    }

    #[test]
    fn child_module_bound_in_parent() {
        let program =
            load_in_memory_result("module helper { function aid() = 1 }").unwrap();
        let root = program.root_module();
        assert!(
            root.bindings.contains_key("helper"),
            "child module should be bound in parent: {:?}",
            root.bindings
        );
    }

    // ── Stdlib import ───────────────────────────────────────────────

    #[test]
    fn stdlib_from_import_binds() {
        let program =
            load_in_memory_result("from std.io import println").unwrap();
        let root = program.root_module();
        assert!(
            root.bindings.contains_key("println"),
            "stdlib import should bind: {:?}",
            root.bindings
        );
        let path = &root.bindings["println"];
        assert_eq!(path.to_string(), "std.io.println");
    }

    #[test]
    fn stdlib_from_import_with_alias() {
        let program =
            load_in_memory_result("from std.io import println as out").unwrap();
        let root = program.root_module();
        assert!(
            root.bindings.contains_key("out"),
            "aliased import should bind under alias: {:?}",
            root.bindings
        );
        assert!(
            !root.bindings.contains_key("println"),
            "original name should not be bound when aliased"
        );
    }

    #[test]
    fn stdlib_whole_import_binds() {
        let program =
            load_in_memory_result("import std.io.println").unwrap();
        let root = program.root_module();
        assert!(
            root.bindings.contains_key("println"),
            "whole import terminal name should bind: {:?}",
            root.bindings
        );
    }

    // ── Cross-module import ─────────────────────────────────────────

    #[test]
    fn from_import_public_function() {
        let program = load_filesystem_result(&[
            ("main.tri", "module helper\nfrom crate.helper import greet"),
            ("helper.tri", "public function greet() = 1"),
        ])
        .unwrap();

        let root = program.root_module();
        assert!(
            root.bindings.contains_key("greet"),
            "imported public function should be bound: {:?}",
            root.bindings
        );
    }

    #[test]
    fn from_import_private_function_errors() {
        let errors = load_filesystem_result(&[
            ("main.tri", "module helper\nfrom crate.helper import secret"),
            ("helper.tri", "function secret() = 42"),
        ])
        .unwrap_err();

        assert!(
            errors.iter().any(|e| matches!(e, LoaderError::VisibilityViolation { name, .. } if name == "secret")),
            "private import should produce VisibilityViolation: {errors:?}"
        );
    }

    #[test]
    fn from_import_nonexistent_name_errors() {
        let errors = load_filesystem_result(&[
            ("main.tri", "module helper\nfrom crate.helper import nope"),
            ("helper.tri", "public function greet() = 1"),
        ])
        .unwrap_err();

        assert!(
            errors.iter().any(|e| matches!(e, LoaderError::UnresolvedImport { .. })),
            "missing name should produce UnresolvedImport: {errors:?}"
        );
    }

    #[test]
    fn from_import_nonexistent_module_errors() {
        let errors =
            load_in_memory_result("from crate.ghost import thing").unwrap_err();
        assert!(
            errors.iter().any(|e| matches!(e, LoaderError::UnresolvedImport { .. })),
            "missing module should produce UnresolvedImport: {errors:?}"
        );
    }

    // ── Path keyword resolution ─────────────────────────────────────

    #[test]
    fn self_import_resolves() {
        let program =
            load_in_memory_result("function helper() = 1\nfrom self import helper")
                .unwrap();
        let root = program.root_module();
        // `self` in the root module = `crate`, so `from self import helper`
        // binds `helper` (which is already bound as own def, but import
        // overwrites with same path — that's fine).
        assert!(root.bindings.contains_key("helper"));
    }

    // ── Reserved namespace ──────────────────────────────────────────

    #[test]
    fn reserved_namespace_errors() {
        let errors =
            load_in_memory_result("from sys.cap import read").unwrap_err();
        assert!(
            errors.iter().any(|e| matches!(e, LoaderError::ReservedNamespace { root, .. } if root == "sys")),
            "reserved namespace should error: {errors:?}"
        );
    }

    // ── Unresolved stdlib name ──────────────────────────────────────

    #[test]
    fn stdlib_nonexistent_export_errors() {
        let errors =
            load_in_memory_result("from std.io import frobnicate").unwrap_err();
        assert!(
            errors.iter().any(|e| matches!(e, LoaderError::UnresolvedImport { .. })),
            "unknown stdlib export should error: {errors:?}"
        );
    }

    // ── Multi-name from import ──────────────────────────────────────

    #[test]
    fn multi_name_from_import() {
        let program =
            load_in_memory_result("from std.io import println, print").unwrap();
        let root = program.root_module();
        assert!(root.bindings.contains_key("println"));
        assert!(root.bindings.contains_key("print"));
    }
}
