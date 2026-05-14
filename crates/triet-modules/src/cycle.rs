//! Cycle detection on the import dependency graph.
//!
//! After the file loader has populated every [`Module`] with its items,
//! this pass scans `Item::Import` and `Item::ImportFrom` in each module
//! to build a directed graph of import edges, then runs DFS with
//! white/gray/black coloring to find cycles. Each detected cycle emits
//! a [`LoaderError::CyclicImport`] with a human-readable trace
//! (`foo → bar → baz → foo`) per ADR-0005 §"Cyclic imports".
//!
//! **Stdlib edges are skipped.** Synthetic stdlib modules (`std.*`) are
//! virtual at v0.2.x.6 — they have no inbound edges and cannot
//! participate in user-authored cycles.

use std::collections::HashMap;

use triet_syntax::{Item, Span};

use crate::{
    error::LoaderError,
    module::{ModuleId, ResolvedProgram},
    path::ModulePath,
};

/// DFS color for the three-state cycle detection algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Color {
    /// Not yet visited.
    White,
    /// Currently on the DFS stack (visiting descendants).
    Gray,
    /// Fully explored — all descendants finished.
    Black,
}

/// An import edge: the importing module, the target module path, and
/// the span of the import statement that created the edge.
struct ImportEdge {
    target: ModulePath,
    span: Span,
}

/// Detect cyclic imports in a fully-loaded [`ResolvedProgram`].
///
/// Returns a (possibly empty) list of [`LoaderError::CyclicImport`]
/// errors — one per cycle found. Multiple independent cycles each get
/// their own error.
pub(crate) fn detect_cycles(program: &ResolvedProgram) -> Vec<LoaderError> {
    // 1. Build adjacency list: ModuleId → Vec<ImportEdge>.
    let adjacency = build_import_graph(program);

    // 2. Build ModulePath → ModuleId index for target resolution.
    let path_index: HashMap<&ModulePath, ModuleId> = program
        .modules
        .iter()
        .enumerate()
        .map(|(i, m)| (&m.path, ModuleId(i)))
        .collect();

    // 3. DFS with coloring.
    let mut color: Vec<Color> = vec![Color::White; program.modules.len()];
    let mut stack: Vec<ModuleId> = Vec::new();
    let mut errors: Vec<LoaderError> = Vec::new();

    for idx in 0..program.modules.len() {
        let id = ModuleId(idx);
        if color[idx] == Color::White {
            dfs(
                id,
                &adjacency,
                &path_index,
                program,
                &mut color,
                &mut stack,
                &mut errors,
            );
        }
    }

    errors
}

/// Build the adjacency list by scanning each module's items for
/// `Item::Import` and `Item::ImportFrom`.
fn build_import_graph(program: &ResolvedProgram) -> Vec<Vec<ImportEdge>> {
    let mut adj: Vec<Vec<ImportEdge>> = Vec::with_capacity(program.modules.len());

    for module in &program.modules {
        let mut edges = Vec::new();

        for item in &module.items {
            match &item.node {
                Item::Import(import_path) => {
                    if let Some(target) = resolve_import_target(&import_path.segments) {
                        // Skip stdlib edges — synthetic, cannot cycle.
                        if !target.is_reserved_root() {
                            edges.push(ImportEdge {
                                target,
                                span: item.span.clone(),
                            });
                        }
                    }
                }
                Item::ImportFrom(import_from) => {
                    if let Some(target) = resolve_import_target(&import_from.source)
                        && !target.is_reserved_root()
                    {
                        edges.push(ImportEdge {
                            target,
                            span: item.span.clone(),
                        });
                    }
                }
                // Items that never create import edges.
                Item::Function { .. }
                | Item::Struct { .. }
                | Item::Enum { .. }
                | Item::Const { .. }
                | Item::Module { .. }
                | Item::TypeAlias { .. } => {}
            }
        }

        adj.push(edges);
    }

    adj
}

/// Map an import path's segments to the [`ModulePath`] of the target
/// module.
///
/// - `import std.io.println` → segments `["std", "io", "println"]`.
///   The target *module* is `std.io` (drop the terminal item name).
///   But if the full path matches a known module, use it as-is.
///
/// - `from crate.foo import bar` → source `["crate", "foo"]`. The
///   target module is `crate.foo`.
///
/// - `self.X` expands relative to the current module. At this stage
///   the import edges use the raw segments; `self` as a single-segment
///   import path points at the importing module itself.
///
/// For cycle detection we only care about the *module-level* dependency,
/// so we treat the segments as a module path directly. If the path
/// doesn't resolve to a known module, we try dropping the last segment
/// (the terminal item name for `import` form). If neither works, we
/// return the full path — the name resolver (#36.4) will catch
/// genuinely invalid paths later.
fn resolve_import_target(segments: &[String]) -> Option<ModulePath> {
    if segments.is_empty() {
        return None;
    }
    // For cycle detection, treat the entire segment list as the module
    // path. The name resolver will sort out module-vs-item ambiguity.
    Some(ModulePath::new(segments.to_vec()))
}

/// Recursive DFS with gray-cycle detection.
fn dfs(
    node: ModuleId,
    adj: &[Vec<ImportEdge>],
    path_index: &HashMap<&ModulePath, ModuleId>,
    program: &ResolvedProgram,
    color: &mut [Color],
    stack: &mut Vec<ModuleId>,
    errors: &mut Vec<LoaderError>,
) {
    color[node.0] = Color::Gray;
    stack.push(node);

    for edge in &adj[node.0] {
        // Resolve target ModulePath → ModuleId.
        let target_id = match path_index.get(&edge.target) {
            Some(&id) => id,
            None => {
                // Target module not in the program — might be an item
                // import like `import crate.foo.bar` where `crate.foo`
                // is the module. Try dropping the last segment.
                if edge.target.len() > 1 {
                    if let Some(parent) = edge.target.parent() {
                        match path_index.get(&parent) {
                            Some(&id) => {
                                // If fallback resolves to the same
                                // module, it's not a real dependency
                                // edge — skip. E.g. `from crate.X
                                // import Y` in root falls back to
                                // `crate` = self.
                                if id == node {
                                    continue;
                                }
                                id
                            }
                            None => continue, // unresolved — resolver catches this
                        }
                    } else {
                        continue;
                    }
                } else {
                    continue;
                }
            }
        };

        match color[target_id.0] {
            Color::White => {
                dfs(target_id, adj, path_index, program, color, stack, errors);
            }
            Color::Gray => {
                // Back-edge found → cycle!
                let trace = build_cycle_trace(stack, target_id, program);
                errors.push(LoaderError::CyclicImport {
                    trace,
                    span: edge.span.clone(),
                });
            }
            Color::Black => {
                // Already fully explored — no cycle through this node.
            }
        }
    }

    stack.pop();
    color[node.0] = Color::Black;
}

/// Build the human-readable cycle trace from the DFS stack.
///
/// The stack contains the path from the DFS root to the current node.
/// `cycle_target` is the node we just found a back-edge to. We extract
/// the sub-path from `cycle_target` to the current node (top of stack),
/// then append `cycle_target` again to close the cycle.
///
/// Example: stack = `[crate, crate.foo, crate.bar, crate.baz]`,
/// `cycle_target` = `crate.foo` → trace = `foo → bar → baz → foo`.
fn build_cycle_trace(
    stack: &[ModuleId],
    cycle_target: ModuleId,
    program: &ResolvedProgram,
) -> String {
    let start = stack
        .iter()
        .position(|&id| id == cycle_target)
        .expect("cycle target must be on the stack");

    let cycle_nodes = &stack[start..];
    let mut parts: Vec<String> = cycle_nodes
        .iter()
        .map(|&id| display_module_name(program, id))
        .collect();
    // Close the cycle.
    parts.push(display_module_name(program, cycle_target));

    parts.join(" → ")
}

/// Render a module name for the cycle trace. Uses the short name
/// (last segment) for readability — `crate.foo.bar` → `bar`. For the
/// crate root, use `crate`.
fn display_module_name(program: &ResolvedProgram, id: ModuleId) -> String {
    let module = program.module(id);
    let segments = module.path.segments();
    if segments.len() <= 1 {
        // Crate root or single-segment.
        segments
            .last()
            .cloned()
            .unwrap_or_else(|| "crate".to_owned())
    } else {
        segments.last().cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader;

    // ── Helpers ─────────────────────────────────────────────────────

    /// Load from in-memory source. Since the loader now runs cycle
    /// detection internally, acyclic programs return `Ok` and cyclic
    /// ones return `Err` containing the cycle errors. We flatten both
    /// cases into a `Vec<LoaderError>` for uniform assertion.
    fn load_in_memory_errors(source: &str) -> Vec<LoaderError> {
        match loader::load_in_memory(source) {
            Ok(_) => Vec::new(),
            Err(errors) => errors,
        }
    }

    /// Load from a filesystem tempdir. Same flattening as
    /// [`load_in_memory_errors`].
    fn load_filesystem_errors(files: &[(&str, &str)]) -> Vec<LoaderError> {
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

        match loader::load_filesystem(root_path.as_ref().expect("at least one file")) {
            Ok(_) => Vec::new(),
            Err(errors) => errors,
        }
    }

    // ── Tests ───────────────────────────────────────────────────────

    #[test]
    fn two_cycle_a_imports_b_imports_a() {
        let errors = load_filesystem_errors(&[
            ("main.tri", "module foo\nmodule bar"),
            ("foo.tri", "from crate.bar import something"),
            ("bar.tri", "from crate.foo import something"),
        ]);
        let traces: Vec<&str> = errors
            .iter()
            .filter_map(|e| match e {
                LoaderError::CyclicImport { trace, .. } => Some(trace.as_str()),
                _ => None,
            })
            .collect();
        assert!(!traces.is_empty(), "should detect a 2-cycle: {errors:?}");
        assert!(
            traces
                .iter()
                .any(|t| t.contains("foo") && t.contains("bar")),
            "trace should mention both foo and bar: {traces:?}"
        );
    }

    #[test]
    fn three_cycle_with_correct_trace() {
        let errors = load_filesystem_errors(&[
            ("main.tri", "module foo\nmodule bar\nmodule baz"),
            ("foo.tri", "from crate.bar import x"),
            ("bar.tri", "from crate.baz import y"),
            ("baz.tri", "from crate.foo import z"),
        ]);
        let trace = errors
            .iter()
            .find_map(|e| match e {
                LoaderError::CyclicImport { trace, .. } => Some(trace.clone()),
                _ => None,
            })
            .expect("should detect a 3-cycle");
        // Trace format: "foo → bar → baz → foo" (ADR-0005).
        assert!(
            trace.contains(" → "),
            "trace should contain arrow separator: {trace}"
        );
        // Must close the cycle — last name equals first name.
        let parts: Vec<&str> = trace.split(" → ").collect();
        assert!(
            parts.len() >= 4,
            "3-cycle trace should have ≥4 parts: {trace}"
        );
        assert_eq!(parts.first(), parts.last(), "cycle must close: {trace}");
    }

    #[test]
    fn self_import_not_panic() {
        // `self` as a path root doesn't match any module in the
        // program — the name resolver (#36.4) handles self-imports.
        // Cycle detector should not panic or false-positive.
        let errors = load_in_memory_errors("from self import something");
        let cycles: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, LoaderError::CyclicImport { .. }))
            .collect();
        assert!(
            cycles.is_empty(),
            "self-import should not produce cycle error: {cycles:?}"
        );
    }

    #[test]
    fn diamond_no_cycle() {
        // A → B, A → C, B → D, C → D — no cycle.
        let errors = load_filesystem_errors(&[
            ("main.tri", "module a\nmodule b\nmodule c\nmodule d"),
            ("a.tri", "from crate.b import fb\nfrom crate.c import fc"),
            ("b.tri", "public function fb() = 1\nfrom crate.d import fd"),
            ("c.tri", "public function fc() = 2\nfrom crate.d import fd"),
            ("d.tri", "public function fd() = 0"),
        ]);
        assert!(
            errors.is_empty(),
            "diamond should not produce cycle errors: {errors:?}"
        );
    }

    #[test]
    fn stdlib_import_not_flagged() {
        let errors = load_in_memory_errors("from std.io import println\nimport std.text.len");
        assert!(
            errors.is_empty(),
            "stdlib imports should not be flagged: {errors:?}"
        );
    }

    #[test]
    fn multiple_independent_cycles() {
        // Two separate cycles: foo↔bar and baz↔qux.
        let errors = load_filesystem_errors(&[
            ("main.tri", "module foo\nmodule bar\nmodule baz\nmodule qux"),
            ("foo.tri", "from crate.bar import x"),
            ("bar.tri", "from crate.foo import y"),
            ("baz.tri", "from crate.qux import a"),
            ("qux.tri", "from crate.baz import b"),
        ]);
        let cycle_count = errors
            .iter()
            .filter(|e| matches!(e, LoaderError::CyclicImport { .. }))
            .count();
        assert!(
            cycle_count >= 2,
            "should detect at least 2 independent cycles, got {cycle_count}: {errors:?}"
        );
    }

    #[test]
    fn no_imports_no_cycles() {
        let errors = load_filesystem_errors(&[
            ("main.tri", "module foo\nmodule bar"),
            ("foo.tri", "function f() = 1"),
            ("bar.tri", "function g() = 2"),
        ]);
        assert!(errors.is_empty());
    }
}
