# ADR 0024 — Khi + Dao Identity Naming (Dao De Jing)

**Status:** Decided. Applies to the v0.7.x.identity sub-task series (5 commits + this 1 ADR commit, shipped before v0.7.10 opens).

**Origin:** Author 2026-05-24 (after v0.7.9.5 closed and the byte-identical gate passed). Before opening v0.7.10 CLI wiring, the author asked: *"Currently we are using `crate` and `cargo` following Rust conventions. I want them to have dedicated names aligned with the naming identity of the language, Triet."*

## §1 — Problem: Rust-inherited Surface Naming

As of v0.7.9.5, 5 terms inherited from Rust remained on the **user-facing surface** of Triet:

| Element | Current | Visibility |
|---|---|---|
| Path keyword | `crate.foo.bar` | User-facing — ~114 occurrences in `.tri` source (std/examples/demos/compiler) |
| Compiled artifact | `.khi` | User-facing — `dao build` output |
| CLI tool binary | `triet` | User-facing — all user commands |
| Manifest filename | `dao.package` | User-facing — root of project |
| Lockfile | `dao.lock` | User-facing — committed to VCS |

(Note: `Cargo.toml` / `Cargo.lock` at workspace root are Rust development artifacts used to build the Rust-implemented compiler, NOT on the Triet user surface — out of scope for this ADR.)

**Identity Issues:**

1. Triet VISION (§3) emphasizes Vietnamese-rooted philosophical depth. Surface terms inheriting English logistics metaphors (`crate`, `cargo`, `pack`) dilute identity.
2. The half-renamed state created inconsistency: manifests had the `triet.` prefix, wire format used `.khi`, but the path keyword remained `crate` — causing cognitive split.
3. SPEC §"Path keywords" + CLAUDE.md "Reserved namespace roots" listed `crate` alongside `std/sys/dev/usr/core` — the only root that was not an ASCII transliteration of a Vietnamese concept.

## §2 — Decision: Khi (器) + Dao (道)

Adopt the core paired concepts from the **Dao De Jing** (Laozi) as the naming framework:

- **Khi (器)** — vessel, utensil, instrument. Per §28: *"Phac tan tac vi khi"* (樸散則為器) — when the uncarved block (phac, raw essence) is dispersed, it becomes vessels (khi). Compilation mapping: source `.tri` (phac) → compile (disperse) → artifact `.khi` (vessel containing philosophical content).
- **Dao (道)** — the Way, principle, process. Per §42: *"Dao sinh nhat, nhat sinh nhi, nhi sinh tam, tam sinh van vat"* (道生一, 一生二, 二生三, 三生萬物) — Dao generates One, One generates Two, Two generates Three, Three generates the ten thousand things. Direct alignment with the balanced ternary identity of Triet (Trit::Negative / Zero / Positive — ternary is a consequence of Dao).

**Why these two concepts, and not others:**

| Considered Pair | Rejection Reason |
|---|---|
| `package` + `triet` (keep Rust naming, change only path keyword) | Pragmatic but lacks philosophical depth; fails to express Vietnamese identity |
| `treatise/corpus/volume/opus` + retain `triet` CLI | English Latin/academic — remains a Western framework, not Vietnamese |
| `niem` (念) + `hanh` (行) — Wang Yangming "unity of knowledge and action" | Valuable but epistemological (knowing/acting) rather than ontological (becoming); weak ternary connection; Neo-Confucian scholarly level rather than foundational |
| `khi` + `phap` (法 — method/dharma) | `phap build` is ASCII-clear compared to `dao build`, but "phap" has Buddhist Sanskrit roots rather than native Dao De Jing; loses direct Laozi reference |
| `phac` (樸) + `dao` (source extension `.phac`) | Extremely strong metaphor (source = phac), but loses the `tri` prefix signifying a ternary language; renaming 1000+ `.tri` files causes excessive churn |

Reasons for choosing **`khi + dao`**:

1. **Dao De Jing is foundational philosophy in Vietnam** — universally recognized from general education. Laozi does not require a "scholar tier" to recognize.
2. **Dao §42 directly justifies balanced ternary** — no metaphor is stronger than *"Two generates Three, Three generates all things"* for a ternary programming language. ADR-0010 (ternary-native IR) can quote this passage as an epigraph.
3. **Khi §28 perfectly maps compilation** — phac (raw source) → khi (compiled vessel). The compiler is the transformer; the output is the vessel holding content. No English-rooted metaphor (`pack`, `bundle`, `archive`) achieves this depth.
4. **CLI `dao build` SIGNALS identity** — `dao` is 3 characters, concise to type. Concerns about confusion with English "dao" (knife) are resolved in documentation headers. Compared to `cargo build` (English logistics, zero depth) → `dao build` (Vietnamese philosophical core) is a feature, not a bug.

## §3 — Naming Matrix (9 Cells)

| # | Element | Before | After | Note |
|---|---|---|---|---|
| 1 | Language name | Triet | Triet | UNCHANGED — language identity is a stable invariant |
| 2 | Source file extension | `.tri` | `.tri` | UNCHANGED — `tri` signifies (a) association with "Triet", (b) ternary language |
| 3 | IR bytecode | `.triv` | `.triv` | UNCHANGED — internal artifact, not on user surface |
| 4 | Compiled package artifact | `.khi` | **`.khi`** | Per §28 phac tan tac vi khi |
| 5 | CLI tool binary | `triet` | **`dao`** | Dao (the Way) — tool performing the phac→khi transformation |
| 6 | Manifest filename | `dao.package` | **`dao.package`** | Cohesive with CLI tool name |
| 7 | Lockfile | `dao.lock` | **`dao.lock`** | Cohesive with CLI tool name |
| 8 | Path keyword | `crate.foo.bar` | **`khi.foo.bar`** | Reference "this khi" — when a file resides in a khi, paths begin with khi |
| 9 | Reserved namespace roots | `std/sys/dev/usr/core/crate/self/super` | `std/sys/dev/usr/core/`**`khi`**`/self/super` | Per CLAUDE.md reserved-root list |

**CLI subcommands** — mixed primary + Vietnamese aliases:

| Primary (English) | Vietnamese alias | Origin |
|---|---|---|
| `dao build` | `dao tao` | tao (create) |
| `dao check` | `dao kiem` | kiem (verify) |
| `dao run` | `dao chay` | chay (execute) |
| `dao store ...` | `dao kho ...` | kho (warehouse) |
| `dao fmt` | (no alias — `fmt` is already 3 chars) | — |

ASCII non-diacritic aliases for CLI usability (`dao tao` typeable on any keyboard layout). Implementation: `dao` argument parser accepts both, dispatching to the same handler. Documentation lists primary and alias side-by-side.

## §4 — Implementation: 5-stage Commit Series (Per-step Cadence)

Hard cutover (no transition period — v0.7 has no external users yet). Independent stages, each with green test suites.

| Stage | Scope | Estimated Files Touched |
|---|---|---|
| **A** | Path keyword `crate` → `khi` (lexer token + parser dispatch + SPEC §"Path keywords" + ~114 user-source + ~30 snapshots) | ~150 |
| **B** | Wire format `.khi` → `.khi` (pack serde + store paths + CLI args + docs) | ~50 |
| **C** | CLI binary `triet` → `dao` (Cargo.toml `[[bin]]` + README + all snapshots matching `triet …`) | ~80 |
| **D** | Manifest `dao.package` → `dao.package` + lock `dao.lock` → `dao.lock` (loader filename matcher + all demo manifests) | ~30 |
| **E** | Vietnamese subcommand aliases (`dao tao/kiem/chay/kho`) — additive feature on top of stage C | ~10 |

Total = ~320 files modified across 5 commits + 1 ADR commit = 6 commits shipped before v0.7.10 opens.

**Stage ordering rationale:**

- **A first** because the path keyword is most user-facing and affects Triet source code; if rollback is required, rolling back A alone is inexpensive.
- **B before C** because the `.khi` wire format does not depend on the CLI binary name; `dao build -o foo.khi` can be tested in an intermediate state.
- **C + D** often ship together (CLI rename + manifest rename are closely linked) but are separated to keep diffs reviewable.
- **E last** because aliases are additive (do not modify primary commands).

**Test invariant**: After each stage, full `cargo test --workspace` + `cargo clippy --workspace --all-targets -- -D warnings` must pass. Per CLAUDE.md "Tests must be green before any commit".

## §5 — Backward Compatibility: Hard Cutover

v0.7 has no external users, package registry, or installed toolchains outside the author's machine. A hard cutover is appropriate:

- No support for legacy `crate.foo.bar` imports — typechecker rejects with E2207-equivalent.
- No support for legacy `triet build` command — `dao build` is exclusive.
- No support for legacy `triet.package` manifest — loader only looks for `dao.package`.
- No support for legacy `.tripack` reader — `read_tripack` removed or renamed to `read_khi`.

Migration tool (`dao fmt --migrate-khi`) for user-facing source: optional, deferred post-v0.7. Currently only touched by the author, manual search/replace or a sed script is sufficient.

## §6 — Related

- [ADR-0005](0005-module-system.md) — Module system + path keywords (superseded regarding the `crate` keyword)
- [ADR-0010](0010-ternary-native-ir.md) — Ternary-native IR — quote Dao §42 as epigraph (deferred, optional cleanup)
- [ADR-0011](0011-abi-metadata-format.md) — ABI metadata format (rename "crate-pack" → "khi" in description, no semantic change)
- [ADR-0014](0014-hash-scheme-refinement.md) — CAS packaging (terminology sweep "crate-pack" → "khi" pkg level)
- [SPEC.md §"Path keywords"](../../SPEC.md) — update reserved-roots list
- [VISION.md §3](../../VISION.md) — potential addition of a paragraph on the Dao De Jing framework grounding the naming

## §7 — Notes

- CLI confusion concerns between `dao build` and English "dao" (knife): evaluated and embraced as part of Vietnamese identity. Documentation headers in README + `dao --help` explicitly state: "`dao` (Dao, the Way) — Triet's build and package manager". Adoption precedent demonstrated by `cargo` / `gem` / `pip`.
- ASCII aliases (`tao` not diacritic form): not all users can type diacritics. Latin-only command aliases represent a pragmatic choice. Documentation displays the accented version for recognition.
- Future: if Triet develops a package registry, the registry URL can be `dao.<TLD>` (e.g. `dao.triet.dev`), establishing cohesive branding.

---

*This decision locks identity-level naming for phase v0.7 onward. Breaking changes to any cell in §3 require a superseding ADR. Implementation v0.7.x.identity (5 stages) commences immediately following this ADR commit.*
