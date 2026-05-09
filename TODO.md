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

(none)

### Pending

- [ ] **v0.2.x.6** — Module loader + name resolver + cyclic detection
  - New pass between parse and typecheck.
  - **Loader:** walk `module foo` declarations from root file; resolve
    each to `foo.tri` or `foo/foo.tri`; recurse into inline `module foo { ... }`.
  - **Resolver:** rewrite `from X import Y` and `import X` paths to
    absolute module paths; bind imported names into per-module scope;
    enforce visibility (`public` / `public(package)` / private).
  - **Cycle detection:** depth-first walk; emit E2100 diagnostic with
    cycle trace per ADR-0005 example.
  - **Typecheck integration:** typecheck runs per-module with resolved
    names instead of the flat symbol table.
  - **Interpreter integration:** runtime symbol table becomes module-aware
    (path-based lookup).
  - Affected crates: `triet-parser` (driver), new `triet-modules` crate
    (loader + resolver), `triet-typecheck`, `triet-interpreter`, `triet-cli`.

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
