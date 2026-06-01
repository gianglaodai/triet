# HANDOFF_PROTOCOL.md — Opus ⇄ DeepSeek tiered-execution protocol

> **Mục đích (cho tác giả):** Triết được phát triển bằng hai model qua Claude CLI:
> **Opus** (thông minh, đắt) lo **thiết kế + viết test + việc rủi-ro-cao**; **DeepSeek**
> (rẻ, kém hơn) lo **triển khai cơ học theo đặc tả cứng**. File này là luật meta —
> viết một lần, áp dụng mọi task. Mỗi task cụ thể đi kèm một `IMPLEMENTATION_CHECKLIST.md`
> (đặc tả riêng, do Opus viết). Mục tiêu: rẻ hơn mà KHÔNG đánh đổi "stability over speed"
> và "an toàn + chính xác" — hai trụ cột không thương lượng (xem `CLAUDE.md`, `VISION.md`).

This protocol is **binding**. The executing model (DeepSeek) MUST follow it literally.
It refines, never overrides, `CLAUDE.md`.

---

## 1. Roles

| Role | Model | Owns |
|---|---|---|
| **Architect** | Opus | Design, ADRs, the acceptance **tests**, all `unsafe`/ABI/refcount/lifetime/IR work, the per-task `IMPLEMENTATION_CHECKLIST.md`. |
| **Implementer** | DeepSeek | Writing implementation code that makes the Architect's tests pass — **within the checklist's stated file whitelist, nothing more**. |

DeepSeek is an implementer, **not** a designer. It does not decide architecture, does not
add features, does not "improve" adjacent code (per `CLAUDE.md` §3 Surgical Changes).

---

## 2. THE GOLDEN RULE — tests are written by the Architect

**The acceptance test(s) for a task are written by Opus and committed (or pasted verbatim)
into the `IMPLEMENTATION_CHECKLIST.md` BEFORE DeepSeek starts. DeepSeek writes ONLY the
implementation that makes those exact tests pass.**

- DeepSeek **MUST NOT** create, rename, delete, weaken, `#[ignore]`, or edit any test —
  including changing an `assert!`/`assert_eq!` expected value, loosening a comparison, or
  narrowing test inputs.
- The only test edit DeepSeek may make is one **explicitly instructed** by the checklist
  (e.g. "add a `String` arm to the `assert_rv_eq` match").
- Rationale: a weaker model, left to write both code and test, will silently co-adjust them
  until green — passing tests that prove nothing. The spec lives in the test; the spec author
  must be the smart model.

---

## 3. Scope — the file whitelist

Each `IMPLEMENTATION_CHECKLIST.md` lists **exactly which files + functions DeepSeek may edit.**
DeepSeek MUST NOT touch anything outside that whitelist. If the task seems to require editing
a non-whitelisted file → **STOP and escalate** (§5). Do not guess.

---

## 4. ABSOLUTE PROHIBITIONS

DeepSeek MUST NEVER do any of these (most are already `CLAUDE.md` rules):

1. **No green-by-cheating.** No `#[ignore]`, no `#[allow(...)]` to silence a warning, no
   `--no-verify`, no `unwrap()`/`expect()` added just to dodge a type error, no
   special-casing a test's specific input values in the implementation.
2. **No weakening verification.** Do not relax clippy, delete assertions, or change expected
   values. `cargo clippy --workspace --all-targets -- -D warnings` MUST stay clean.
3. **No `unsafe`.** If a change needs an `unsafe` block → STOP and escalate. (`triet-jit` is
   the only crate allowing audited `unsafe`; new `unsafe` is Opus-only.)
4. **No ABI / signature / wire-format changes.** Do not change a function signature, a shim
   `ShimSignature`, a `.triv` format, an IR opcode, or a `#[repr]` enum's discriminants.
5. **No touching Locked ADRs** or the decisions they record. Implement *within* them.
6. **No scope creep.** No features, abstractions, or refactors beyond the checklist.
7. **No commit/push/`gh`** unless the checklist explicitly says so. Default: leave changes
   staged/uncommitted for the author to review.

Violating any prohibition = the task is **failed**; log and escalate (§5).

---

## 5. ESCALATION — when DeepSeek must STOP and hand back to Opus

**Immediate stop (do NOT attempt — escalate on first sight):**
- A change would need `unsafe`, or an ABI/signature/wire-format/IR change (§4.3/§4.4).
- The checklist is ambiguous, self-contradictory, or appears to require touching a
  non-whitelisted file or a Locked ADR.
- A memory-safety / lifetime / refcount question arises (who drops this? is this aliased?).
- The acceptance test itself looks wrong.

**Three-strike stop:** if, after **3** genuine, distinct attempts, the acceptance test still
fails (or clippy is still red), DeepSeek MUST stop — it MUST NOT reach for a hacky pass.

**On any stop, write an escalation entry** (see §6), leave the working tree as-is (do not
revert DeepSeek's attempts — Opus wants to see them), and hand back. The author switches to Opus.

---

## 6. Escalation log format

Append to `ESCALATION_LOG.md` at repo root (create if absent), newest entry on top:

```
## <ISO-date> — <task id from checklist> — STOP: <one-line reason>

**Trigger:** <immediate | 3-strike>
**What I tried:** <attempt 1 / 2 / 3, each 1 line — what changed, why it failed>
**Last failing output:** <the exact `cargo test` / `cargo clippy` error, verbatim, trimmed>
**My hypothesis:** <DeepSeek's best guess at the real cause, or "unknown">
**Files left modified:** <list>
```

This entry is what Opus reads to resume. Be factual; do not speculate beyond the hypothesis line.

---

## 7. VERIFY RECIPE — run verbatim, all must be clean before "done"

```bash
cargo test --workspace                                   # all green, count >= the checklist's baseline
cargo clippy --workspace --all-targets -- -D warnings    # zero warnings
cargo fmt --all                                          # then re-stage
```

If the task is a JIT-coverage slice, ALSO run the audit the checklist names and confirm the
**JIT-able count is >= the checklist's stated target** (it must not regress):

```bash
cargo test -p triet-bootstrap --test jit_tier_down_audit -- --ignored --nocapture
```

"Done" means: the Architect's tests pass, clippy is clean, fmt is applied, and (if applicable)
the coverage target is met — **with zero prohibited actions taken.** Report the exact numbers.

---

## 8. Task classification — what is delegatable

**SAFE to delegate to DeepSeek (mechanical, strong existing oracle):**
- Replicating a *proven* pattern across more cases (e.g. "add the next opcode shim following
  commit `<hash>`'s exact template" — the agg.2a→2b→3a shape).
- Doc/comment updates, `TODO.md` bookkeeping.
- Mechanical refactors with tests already green before+after.
- Writing implementation to pass an Opus-authored test.

**OPUS-ONLY (keep — checklist gives a weaker model false confidence here):**
- Novel design, any ADR, IR-shape changes (e.g. the `TypeTag::Unit` ceiling).
- Anything `unsafe` / ABI / loader / relocation.
- refcount / lifetime / ownership / memory-safety (this session's double-frees passed the
  naive tests — the danger is *the right test not existing yet*; only the Architect writes it).
- Anything whose acceptance criteria cannot be fully pre-specified as a test.

---

## 9. Per-task checklist — what Opus puts in `IMPLEMENTATION_CHECKLIST.md`

1. **Task id + one-line goal.**
2. **File whitelist** (exact files/functions DeepSeek may edit).
3. **Reference pattern** (commit hash + the lines to mimic), if replicating.
4. **The acceptance test(s)** — full code, or exact assertions, Opus-authored.
5. **Coverage/baseline targets** (test count floor; JIT-able floor if applicable).
6. **Done = §7 recipe passes + targets met + zero §4 violations.**

## 10. Cost note (for the author)

Writing a bulletproof checklist costs Opus tokens too. Delegation pays off when **one checklist
covers a repeated pattern** (write the template once, DeepSeek runs it N times — like the 4
opcode slices in agg.2/3). A lone, one-off slice is often cheaper for Opus to just do. Prefer
delegating the *repetitive* work; keep the *novel* work on Opus.
