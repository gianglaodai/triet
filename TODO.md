# TODO

Sub-task tracking — short-term work in progress.

- Long-term phasing: [`ROADMAP.md`](ROADMAP.md)
- Architectural decisions: [`docs/decisions/`](docs/decisions/)
- Language semantics: [`SPEC.md`](SPEC.md), [`VISION.md`](VISION.md)

This file is updated as tasks complete. When a phase finishes (e.g. v0.2.x),
the summary is archived into `ROADMAP.md` and the detailed checkboxes
removed from here.

---

## v0.2.x — Module system (in progress)

Per [ADR-0005](docs/decisions/0005-module-system.md). Goal: hierarchical
module tree with explicit `public` export, dot paths, Python-style
imports, Java JPMS-aligned `module` declarations.

### Done

- [x] **v0.2.x.0** — SPEC.md align with VISION (5 pillars + OS-capable)
- [x] **v0.2.x.1** — Drop SIMD/Tensor/DType §10.5 from SPEC
- [x] **v0.2.x.2** — Visibility AST + parser capture (3 levels: `public`, `public(package)`, private)
- [x] **v0.2.x.3** — Verbose keyword sweep (`fn`→`function`, `pub`→`public`, `mut`→`mutable`, `const`→`constant`, `mod`→`module`) + dot path commitment
- [x] **v0.2.x.3.1** — Post-sweep drift cleanup (ADR-0005 rewrite, SPEC, README, doc-comments) — commit `fa89622`
- [x] **v0.2.x.4** — Reserved keywords validation (`std`/`sys`/`dev`/`usr`/`core` cannot be user-declared item names)
- [x] **v0.2.x.5** — Module declarations + Python import syntax (parser-only) — commit `e6e7e51`
  - `module foo` (file-bound) and `module foo { items }` (inline)
  - `from std.io import println, print as p`
  - `import std.io.println` (existing form retained)
  - Path keywords `crate.`, `self.`, `super.` accepted as roots
  - Glob `from X import *` rejected with ADR-0005 citation

### In progress

- [ ] **v0.2.x.6** — Module loader + name resolver + cyclic detection

  Architecture (chốt 2026-05-09): new `triet-modules` crate sits between
  parse and typecheck. Output is `ResolvedProgram` = flat list of
  `Module`s, each with own AST + `bindings: HashMap<String, AbsolutePath>`.
  Stdlib (`std.*`) handled as synthetic modules at v0.2.x.6; v0.2.x.7
  swaps source to real files. CLI: file passed to `triet run` is crate
  root (Python/Go pattern). Inline `module foo { … }` ≡ file-bound
  `module foo` for path resolution (Rust/OCaml pattern).

  **Sub-tasks (per-step commits):**

  - [x] **#36.1** — Scaffold `triet-modules` crate — commit `35dc88f`
    - Types: `ModulePath`, `AbsolutePath`, `ModuleId`, `Module`, `ResolvedProgram`
    - `LoaderError` enum with E2100–E2106 + miette diagnostics
    - Synthetic stdlib registry (crate-private, used by #36.4)
    - 19 unit tests, clippy clean
  - [x] **#36.2** — File loader _(uncommitted)_
    - Refactored `Module` / `ResolvedProgram` to multi-arena shape: one
      arena per parsed file, inline submodules share parent arena
      (avoids cross-file ID remapping)
    - `load_program(&Path)`: read root, recurse on `module foo` decls,
      resolve `foo.tri` (flat) → `foo/foo.tri` (nested fallback);
      children of `foo` searched in `<dir>/foo/` regardless of layout
    - `load_program_from_source(&str)`: in-memory mode; rejects external
      `module` decls with `FileNotFound` (no filesystem context)
    - Inline modules nested arbitrarily; external children of inline
      parents work iff parent has filesystem context
    - 15 new tests (in-memory + tempfile-driven filesystem) — covers
      empty root, function-only, single inline, nested inline, deep
      filesystem tree, missing file, parse error attribution, IO error
  - [x] **#36.3** — Cycle detection (E2100): DFS coloring on import graph, emit cycle trace `foo → bar → baz → foo` per ADR-0005 — commit `28b0ca3`
  - [ ] **#36.4** — Name resolution + visibility check: rewrite `from X import Y` to absolute, bind into module scope, validate `public`/`public(package)`/private; bind synthetic stdlib exports
  - [ ] **#36.5** — Typecheck integration: `check(&ResolvedProgram)` per-module with bindings, cross-module type lookup via absolute path
  - [ ] **#36.6** — Interpreter integration: `run(&ResolvedProgram)`, main lookup at root, cross-module call via per-module bindings
  - [ ] **#36.7** — CLI rewire (`triet run`/`check` through loader, single-file backward-compat) + integration tests (multi-file, cycle, visibility violation, file not found, reserved namespace)

### Pending

- [ ] **v0.2.x.7** — Stdlib reorganize as nested module structure
  - Convert flat `std.io.println` baseline into proper modules with
    `module` declarations under a `std/` directory.
  - Targets: `std.io` (print/println/read_line), `std.text` (len/concat/from_integer), `std.assert` (assert).
  - Update prelude binding in `triet-typecheck` and `triet-interpreter`.

- [ ] **v0.2.x.8** — Demo lớn + snapshot tests for module system
  - One demo program (~500 lines) split across 5+ modules — exercises
    `module`, `from X import Y`, `import X`, visibility, nested submodules.
  - Snapshot tests for diagnostics: cyclic import (E2100), visibility
    violation, unresolved path, reserved namespace abuse.
  - Acceptance gate: all existing demos still pass, large demo runs
    correctly, all snapshot tests stable.

---

## How to update this file

- Mark a task `[x]` and move it to **Done** when its commit lands on `main`.
- Add the commit short-hash next to completed tasks for quick git reference.
- Keep the order: **Done** → **In progress** → **Pending**.
- When a whole phase (e.g. v0.2.x) ships, archive its summary into
  `ROADMAP.md` (under the changelog section) and delete the detailed
  checkboxes from this file.
