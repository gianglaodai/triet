# ADR 0005 — Module system: Java JPMS aesthetic, dot paths, Python imports

**Status:** Decision. Applicable to v0.2.x. This is pillar #2 in [VISION.md](../../VISION.md).

> ⚠️ **Import syntax superseded by [ADR-0071](0071-path-separator-and-module-import.md) (2026-06-25).**
> The `import std.io` / `from std.io import …` keywords + dot-separated import
> paths described below are replaced by `use std::io::{a, b as c}` with `::`
> paths. The module *semantics* (hierarchical tree, visibility ladder, cyclic
> refusal, `khi`/`self`/`super` roots, stdlib resolution) remain authoritative —
> only the surface import syntax changed. The body below is kept verbatim as the
> historical record; read it for the module model, not the import keywords.

**Issue:** Triet has reached the limit of single-file programs. The demo codebase already contains 11 `.tri` files within a single flat namespace. Internal libraries and codebase separation require a true module system. This is also the architectural prerequisite for a stable ABI (v0.4), CAS packaging (v0.5), and capability namespaces (v0.6)—a poorly designed module system would break these three future pillars.

## Decision

Triet adopts a **hierarchical module tree following the Java JPMS style**, utilizing **verbose keywords** + **dot-separated paths** + **Python-style imports**, **explicit `public` exports**, and **no hard binding to the filesystem**.

### Syntax

```triet
// In file `pkg.tri` (root of crate `pkg`):
module foo                            // declares submodule, compiler looks for `foo.tri`
module bar                            // declares submodule `bar`

// In file `foo.tri`:
public function hello() -> String =   // exported
    "hello"

function helper() -> Integer =        // private (default)
    42

module inline {                       // inline submodule
    public function nested() -> Integer = 1
}
```

### Path syntax

Triet uses the dot `.` as a path separator (similar to Java/Python, unlike Rust/C++). It does not use `::`.

| Path | Meaning |
|---|---|
| `crate.foo.bar` | Absolute path from crate root |
| `self.foo` | Relative — current module |
| `super.foo` | Relative — parent module |
| `std.io.println` | Stdlib path |
| `sys.*`, `dev.*`, `usr.*` | **Reserved** in v0.2.x (not yet enforced; v0.6 will enforce capabilities) |

`crate`, `self`, and `super` are reserved path keywords (see ADR-0005 §"Reserved top-level namespaces") and cannot be used as identifiers.

### Visibility levels

```triet
public function open() = ...          // visible everywhere
public(package) function detail() = ... // visible within the same crate-pack only
function helper() = ...               // private to current module (default)
```

**Triet SIMPLIFIES Rust's visibility:** only 3 levels (`public`, `public(package)`, and private). We omit `public(super)` and `public(in path)` to keep the ABI surface simple. These may be added in v1.0+ if strictly necessary.

### Imports — Python style

Triet uses Python syntax `from ... import ...` for selective imports, and `import ...` for whole-module imports.

```triet
from crate.foo import bar             // single name
from crate.foo import a, b, c         // multi
from crate.foo import bar as baz      // rename
from std.io import println, print
import std.io                         // import whole module (use as `std.io.println`)
```

**NOT supported:**
- Glob imports (`from foo import *`) — violates the explicit export principle and makes the ABI surface ambiguous. This may be revisited in v1.0+ if a compelling use case arises.
- Re-exports (`public from X import Y` or equivalent) — deferred to v0.3+ when the requirement is clear.

### File resolution

The compiler searches for files in the following order:
1. `module foo` in `path/to/parent.tri` $\rightarrow$ looks for `path/to/foo.tri` **or** `path/to/foo/foo.tri`.
2. Inline `module foo { ... }` $\rightarrow$ does not look for a file.

A module with a submodule is represented as a directory containing both the `foo.tri` file (the module itself) and its children in `foo/bar.tri`. This is simpler than Rust 2018 (no `mod.rs` required, avoiding multiple files with the same name).

```
mypkg/
├── mypkg.tri              # crate root: declares `module foo; module bar`
├── foo.tri                # module `foo` content
├── foo/                   # foo's children
│   ├── inner.tri          # foo.inner
│   └── helper.tri         # foo.helper
└── bar.tri                # module `bar`, no children
```

**Note:** The filesystem layout is a **convention**, not a semantic requirement. The compiler resolves based solely on `module` declarations. The mapping is designed so that:
- New developers can immediately understand the structure by reading the filesystem (helpful).
- Refactoring (renaming, moving) only requires updating `module` declarations and renaming files (flexible).

### Cyclic imports

**Forbidden.** The compiler will trigger an error during name resolution. The diagnostic will explicitly identify the cycle:

```
error[E2100]: cyclic module dependency
   ┌─ crate/foo.tri:3:1
   │
3  │ from crate.bar import B
   │ ^^^^^^^^^^^^^^^^^^^^^^^ creates cycle: foo → bar → baz → foo
```

### Reserved top-level namespaces

In v0.2.x, the following root namespaces are **reserved** (the compiler will reject user declarations):

| Root | Purpose | Phase of enforcement |
|---|---|---|
| `std` | Standard library | v0.2.x (already exists) |
| `sys` | Syscall surface | v0.6 (capability) |
| `dev` | Driver / hardware | v0.6 (capability) |
| `usr` | User application | v0.6 (capability)
| `core` | Minimal stdlib (no_std style) | v1.0+ |

Early reservation prevents breaking user code when v0.6 is released.

## Rationale

### Why verbose keywords?

Triet is an **AI-first language**. The goal is: LLMs generate correct code on the first attempt, and developers read code without needing a dictionary for abbreviations.

- `function` / `public` / `mutable` / `constant` / `module` — these are a few characters longer, but they provide zero ambiguity. `fn` could be a Function-key, `pub` could be publication, `mut` could be a mutex, and `mod` could be modulo.
- LLM context tokens are dense: verbose keywords consume only 1–2 BPE tokens, which is not significantly more expensive than symbols.
- Java has proven that large ecosystems are not hindered by long keywords.
- Following design principle #1 in VISION.md: "explicit > implicit, regular > exception, keyword > symbol when ambiguous, low ambiguity > terseness."

### Why dot paths, not `::`?

- `.` is already used for field access in Triet — providing a consistent experience for newcomers (especially those from Java/Python/JS).
- `::` is a C++ legacy used to distinguish namespaces from members; Triet uses a two-phase resolver (load $\rightarrow$ resolve), so there is no need for syntactic distinction during lexical analysis.
- Field access and path resolution are unambiguous in Triet: the parser decides based on context (after `import`/`from`/type annotation = path; after an expression = field). Module paths always appear in a deterministic position.
- Java/Python/Kotlin/Swift all use `.` for both module paths and field access without practical issues.

### Why Python-style `from X import Y`?

- Clearly separates "which module" from "which name" — making it easy to read and refactor.
- Compact multi-imports: `from std.io import println, print` is more concise than `import std.io.println; import std.io.print`.
- Aliasing uses the same syntax as selective imports: `from std.io import println as out` — no separate keyword required.
- Unlike `import std.io.println` (Java), it requires the terminal name to be written on the first mention, forcing developers to use explicit names.
- Every LLM has seen millions of lines of Python — it can generate correct code immediately.

### Why Java-style `module foo`?

- Java JPMS (Java 9+) has proven that module declaration is a first-class concept, decoupled from the filesystem.
- Triet adopts that spirit: `module foo` is an actual declaration in the source, not an implicit result of the directory structure.
- We accept that the filesystem layout is a convention (see §"File resolution"), but the semantics are determined by the `module` keyword — refactoring only requires updating the keyword and the filename.

### Why simplify visibility?

Rust has 5 levels: `pub`, `pub(crate)`, `pub(super)`, `pub(in path)`, and private. Most codebases use 80% `pub` + `pub(crate)`. `pub(super)` and `pub(in path)` are rare and complicate the ABI surface.

Triet v0.2.x only requires `public` (export) + `public(package)` (internal) + private (default). This is simple and easy to learn for LLM-generated code. It can be expanded in v1.0+ if necessary (following the "stability over speed" principle: adding is easier than removing).

### Why no glob imports?

Glob imports (`from foo import *`) violate the explicit export principle:
- Readers do not know which names are being imported.
- Refactoring in `foo` (adding a symbol) could cause accidental shadowing in the local scope.
- ABI metadata would require scanning the entirety of `foo` to determine the surface.

Forbidden in v0.2.x. **May be revisited in v1.0+** with strict constraints (e.g., only within test modules).

### Why use `crate.` instead of `pkg.` for paths?

"Crate" is already established terminology in Triet (workspaces contain `crates/`). Changing to `pkg.` simply to differ from Rust lacks sufficient reasoning. We reserve `pkg.` for the "Crate-Pack distributable" concept in v0.4 (which will be something else).

### Why reserve `sys`/`dev`/`usr` from v0.2.x?

These three namespaces are core to pillar #5 (the capability system, v0.6). Early reservation ensures:
- User code does not break when v0.6 is shipped.
- Library authors are guided: the stdlib system belongs in `sys`, applications in `usr`.
- Early typecheck warnings (v0.5+) can be issued if a user imports the wrong namespace.

### Cyclic imports — why a hard ban?

- Cycles break compile-time logic: the linker does not know the initialization order.
- Cycles are a sign of poor design (high coupling).
- All production-grade systems languages (Rust, Go, OCamle) either forbid or heavily warn against them.

Diagnostics that explicitly identify the cycle help developers fix the issue quickly.

## Alternatives considered

### A1. Filesystem-strict (Java pre-Jigsaw / Python 2)
**Reject.** Refactor-unfriendly. Java has already abandoned this.

### A2. First-class modules (OCaml functor)
**Defer.** Theoretically beautiful (parametric modules) but complex to implement and difficult for LLMs to learn. May be added in v2.0+ if truly needed.

### A3. ES modules (file = module, default exports)
**Reject.** Implicit namespace derived from the filesystem. Default exports are good for ergonomics but make the ABI surface ambiguous. Triet prioritizes explicitness.

### A4. Mojo modules
**Reference, but do not fully adopt.** Mojo follows the Python module model and has some points of reference (file = module), but Mojo is still evolving. We will wait for Mojo to settle before adopting details.

### A5. Single-file packages (Go)
**Reject.** Go merges all files in the same directory into a single namespace. While simple, it does not support natural nested namespaces.

### A6. Rust-style `::` paths + `mod`/`use`
**Reject.** A C++ legacy; in Triet, there is no ambiguity between namespace and member access, so a separate symbol is unnecessary. Verbose keywords + dot paths are more aesthetically pleasing and align better with the common Java/Python background.

## Consequences

**Positive:**
- The Triet codebase can scale to dozens or hundreds of modules without name collisions.
- Internal libraries can be separated into `crate.core`, `crate.utils`, and `crate.api`.
- The stdlib is reorganized from a flat structure (`std.io.println` in the v0.2 monolith) into a proper hierarchy (`std.io.println` via the module system).
- The ABI surface (v0.4) only needs to scan items marked `public` — making it fast.
- Capability enforcement (v0.6) has a clear anchor in the top-level namespace.

**Negative:**
- Verbose keywords are longer than symbols — an accepted tradeoff (see "Why verbose keywords?").
- Developers coming from Rust/C++ will find the lack of `::` noticeable — accepted (see "Why dot paths").
- v0.2 shipped the `import std.io.println` syntax (terminal name immediately following `import`), which differs from Python's `import std.io`. This will be standardized when the v0.2.x module system ships: the `from std.io import println` syntax will officially replace selective imports; `import std.io` (whole module) will remain.

**Migration strategy:**
- v0.2 baseline: only supports the `import std.io.println` form (dot-path with terminal name).
- v0.2.x: adds the `from X import Y` form. The `import std.io.println` form will be maintained as "import whole sub-path with named tail," equivalent to `from std.io import println`.
- v0.3: syntax is stabilized; no further changes.

## Implementation roadmap (v0.2.x)

1. **Lexer:** `module`, `public`, `import`, `crate`, `self`, and `super` keywords already exist (ADR-0005 commits). Need to add `from` and `as` keywords for Python-style imports.
2. **AST:**
   - `Item::Module { name, content: Either<Inline(Vec<Item>), External> }`.
   - `Item::Import { source: Path, names: Vec<(String, Option<String>)> }` — `from X import a, b as c`.
   - `Item::Import { whole: Path }` — `import X` (whole module).
   - `Item::*` already has the `visibility: Visibility { Public, PublicPackage, Private }` field (commit `7cb63e7`).
   - `Path` AST node distinguishes absolute (`crate.`), relative (`self.`/`super.`), and reserved (`std.`/`sys.`/...).
3. **Parser:** `parse_module`, `parse_import`, and `parse_visibility` already exist. Uses recursive descent + Pratt — easy to extend.
4. **Module loader:** A new pass before typechecking. Builds the module tree from the root file. Resolves `module foo` $\rightarrow$ finds file. Detects cycles.
5. **Name resolver:** A new pass before typechecking. Resolves `from X import Y` paths to absolute paths. Validates visibility.
6. **Typecheck:** Runs per-module with resolved names. Type definitions and functions are cross-module via the name resolver.
7. **Interpreter:** The runtime currently has a flat symbol table — must be extended to be module-aware (path-based lookup).
8. **CLI:** `dao check` + `dao run` must accept a root file and automatically load the module tree.
9. **Stdlib migration:** Transition `std.io` and `std.text` into proper modules with `module` declarations.
10. **Large-scale Demo:** Write a large demo (~500 lines) split across 5+ modules to validate end-to-end.

**Test gate:**
- All existing `.tri` demos must continue to run.
- The large module-split demo must run correctly.
- Snapshot tests for diagnostics: cyclic import, visibility violation, unresolved path, and reserved namespace abuse.
- 50+ new unit tests for the module loader + name resolver.

## References

- [Java Project Jigsaw / JEP 261](https://openjdk.org/projects/jigsaw/) — JPMS module model, the baseline for `module` declaration.
- [Python Language Reference — Imports](https://docs.python.org/3/reference/import.html) — `from X import Y` syntax.
- [Rust Reference — Items: Modules](https://doc.rust-lang.org/reference/items/modules.html) — reference for hierarchical module trees (visibility, no filesystem binding).
- [OCaml Module System](https://v2.ocaml.org/manual/moduleexamples.html) — first-class modules (deferred).
- [TypeScript Modules](https://www.typescriptlang.org/docs/handbook/modules.html) — ES modules (rejected pattern).
- [Mojo Modules](https://docs.modular.com/mojo/manual/packages) — reference.

## Related

- [ADR-0007](0007-ir-design.md) (written, v0.3): IR design — `AbsolutePath` from the module loader is the input for cross-module calls in the IR.
- ADR-0009 (upcoming, v0.4): ABI metadata format — module visibility is an input.
- ADR-0012 (upcoming, v0.5): Hash scheme — module structure affects `iface_hash`.
- ADR-0014 (upcoming, v0.6): Capability type system — top-level namespace is the anchor.

---

*This decision freezes the module model for v0.2.x. Any breaking changes from this phase forward require a separate ADR.*
