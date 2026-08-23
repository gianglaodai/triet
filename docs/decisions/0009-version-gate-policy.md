# ADR 0009 — Version gate policy: v0.4 entry requirements

**Status:** Decision. Applies to all version bumps v0.x → v0.(x+1) starting from v0.3 → v0.4. Locked as a project principle.

**Issue:** When closing the v0.3 phase (commit `28e7da0`), several gates remained in a partial state:

- Differential VM ≡ interpreter: **3/11** (8 ignored with `#[ignore]`)
- VM bench: **1.26×** interpreter (gate set at 3×)
- Cargo workspace version: still `0.1.0` (out of sync with v0.3 SPEC)
- Accumulated Clippy warnings: 109+ in `triet-ir/lib` despite `CLAUDE.md` requiring *"fix every new warning"*
- TODO comments `TODO(v0.3.4)`, `TODO
