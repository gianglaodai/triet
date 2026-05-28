# TODO

Sub-task tracking — short-term work in progress.

- Long-term phasing: [`ROADMAP.md`](ROADMAP.md)
- Architectural decisions: [`docs/decisions/`](docs/decisions/)
- Language semantics: [`SPEC.md`](SPEC.md), [`VISION.md`](VISION.md)

This file tracks the **current phase** only. When a phase finishes, its summary archives to `ROADMAP.md` and detailed checkboxes are deleted from here.

---

## v0.2 — v0.8 archived

All shipped phases now live in [`ROADMAP.md`](ROADMAP.md):

| Phase | ADRs | Final test count |
|---|---|---|
| v0.2.x Module system | 0005, 0006 | 700+ |
| v0.3 Bytecode VM + Stable IR | 0007, 0008 | 835 |
| v0.3.x.cleanup | 0009 | 835 |
| v0.3.x.ternary | 0010 | 838 |
| v0.4 Crate-Pack + Stable ABI | 0011, 0012, 0013 | 867 |
| v0.5 CAS Packaging | 0014, 0015 | 918 |
| v0.5.x.review | 0015 Addendum | 924 |
| v0.6 Capability System | 0016, 0017, 0018 | 1079 |
| v0.6.x.review | 0018 Addendum | 1085 |
| v0.7 Self-hosting Compiler | 0019, 0020, 0021, 0024 | 1345 |
| v0.8 Ownership Foundation + BYOS | 0022, 0025, 0026 v2, 0027 | 1425 |

---

## v0.8.x.review — Post-v0.8 audit fixes 🔄 in progress

**Trigger:** Whole-project audit (AI) sau Release v0.8.0 commit `78f2402` phát hiện 4 BLOCKERS + 6 HIGH drift findings. Author confirmed "tất cả các lựa chọn phải tuân thủ chặt chẽ stability over speed" — phase mở để fix tất cả trước khi v0.9 mở.

**Quyết định kiến trúc:** Không thay đổi spec. Phases tuân theo cadence cũ v0.5.x.review / v0.6.x.review pattern.

- [x] **v0.8.x.review.1** — Close ADR-0009 gate B Hygiene leftover (3 clippy errors `resolver.rs` ambient-module fallback + 21 `cargo fmt` files) — `e8d797a`
- [x] **v0.8.x.review.2** — E25XX namespace correction `triet::borrow::` → `triet::actor::` (6 chỗ ở `error.rs` + `cli/main.rs` JSON mapper) per ADR-0026 v2 + CLAUDE.md namespace table — `fcc18fd`
- [x] **v0.8.x.review.3** — Port ownership reference tokens to self-host lexer — `compiler/parser/lexer.tri` thêm `Ampersand*` variants + dispatch + smoke check assert. Closes v0.8.12 paperwork-vs-reality gap (lexer-only; parser AST `ReferenceForm` port defer v0.9+) — `46c8722`
- [x] **v0.8.x.review.4** — Doc sync — CLAUDE.md (state + 2 arch sections + anchor + trit table + cadence + examples + audit history), README.md (v0.8 highlight + structure + tests), docs/decisions/README.md (§v0.8 add 0022/0025/0026/0027, remove "Future research"), ADR status Draft → Locked × 4 — `ebdbd15`
- [x] **v0.8.x.review.5** — ROADMAP §v0.8 SHIPPED marker + archive sub-tasks với commit hash + add §v0.8.x.review section + TODO.md v0.8 archive — this commit
- [x] **v0.8.x.review.6** — Cleanup root scratch files (16 untracked: 9 `fix_*.py` + `parse_test`/`parse_test.rs` + `run_dev_root.sh`/`run_dup.sh` + `demo.tri`/`hello.tri`/`out.khi` + `crates/triet-cli/target/` build artifact) + tighten `.gitignore` (`/target` → `target/` un-anchor, add `/*.khi` + `/parse_test`) — this commit
