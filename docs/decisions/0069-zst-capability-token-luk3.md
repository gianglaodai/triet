# ADR 0069 — ZST Capability Token with Ł3-Trit (rewrite-era, borrowck-enforced)

> # ⚖️🩸 CORE PRINCIPLE (G carved in stone 2026-06-25)
> # Capability = ownership. The gatekeeper (Borrow Checker) MUST hold the leash.
> A capability that the borrowck does not enforce via memory-safety is **mere window dressing**.
> Grant/Ambient/Deny must be checked **at compile-time**; only `Defer` (Unknown)
> may trigger a runtime hook. No bypass is possible.

**Status:** Proposed (white-paper scaffold — recon-before file:line, NOT yet coded; awaiting G's signature).
Applying C+ Tier. This is the **strategic front to complete COHERENCE VISION §8**: a single Ł3 algebra spanning **null (PA-3c) / logic (Trilean) / capability** — the missing third leg.

**Issue — two disconnected worlds, neither of which fulfills VISION §8:**
- **World 1 (package-manifest, ADR-0016/0017/0018):** 4-state Ł3 algebra ALREADY coded
  (`CapabilityLevel{Deny=Trit::Negative, Ambient=Trit::Zero, Grant=Trit::Positive, Defer=Trilean::Unknown}`
  at `crates/triet-pack/src/types.rs:297`; `check_capabilities()` at
  `crates/triet-typecheck/src/capability_check.rs:131`) — **BUT orphaned**: `grep capability
  crates/triet-driver/src` = EMPTY. Enforcement dies in the actual pipeline. The `.khi`/`dao`/
  cross-package-linker mechanism behind it has largely been removed or unwired → **G deprecates 0016/0017/0018**.
- **World 2 (Hardware-Token ZST, `spec/plans/phase6` + schema §10):** capability = ownership +
  move enforced by borrowck, ZERO runtime overhead, coherent with the recently closed No-Box (Axis B) —
  **BUT "design only", and intentionally lacks Ł3-level** (binary "has token or not").

**Strategic Decision (G finalized DIRECTION C — Synthesis, 2026-06-25):** merge the two worlds.
**Rescue the Ł3 algebra of World 1, discard the package-manifest mechanism; build upon the ownership/move
engine of World 2.** The ZST-token **tightly binds** the Ł3-Trit: Grant/Ambient/Deny enforced by the blood of the borrowck at compile-time; Defer triggers a runtime policy hook + trap upon violation.

**ADR Relationships:**
- **DEPRECATED:** ADR-0016 (package capability manifest), 0017 (policy resolution), 0018 (provenance
  prompt) — package-manifest era, not applicable to rewrite single-file/JIT. Do not exhume mummies.
- **Inherits:** S6 ownership (ADR-0022 §2, 5-form reference), borrowck NLL E2420 (ADR-0025),
  No-Box move/Deinit machinery (ADR-0066/0067), trap-on-violation 2-signal (ADR-0044).
- **Amend:** schema §10 `HardwareToken` (`spec/schema/triet-schema.yaml:1684`) — see §8.

---

## 0. Current Reality (recon O measured file:line, 2026-06-25 — carved in stone, no guessing)

| # | Finding | Evidence (file:line) | Design Implications |
|---|---|---|---|
| 1 | ZST construction `Cap {}` (empty struct) **does not lower** — falls into variable branch → "undefined local variable: Cap". Empty structs with **DECL** parse/typecheck/lower OK. | probe `triet-driver run` (struct-construct path, `triet-lower`) | **Phase-0 prerequisite**: ZST must be constructible before everything else. |
| 2 | `is_copy` for `Struct` = `s.fields.iter().all(\|f\| f.ty.is_copy(...))`. Empty struct → `all()` on EMPTY iterator → **`true` → COPY**. | `crates/triet-mir/src/lib.rs:666` | ZST tokens are by default classified as **Copy** → borrowck DOES NOT track moves → silent bypass. **Tokens MUST be forced non-copy.** |
| 3 | Substrate enforcement **ALREADY EXISTS**: E2420 use-after-move gated on `is_copy(Some(body))`; `ParameterPassing::Move`; 5-form `ReferenceForm`. Test `write_twice(vga: &+ mutable VgaBuffer)` + `consume(vga)` follows the correct ZST-move pattern. | `crates/triet-borrowck/src/checker.rs:388,618,690,948,981` + `lib.rs:339` | **move = transfer + revocation = E2420 — already built.** No rework required. |
| 4 | 4-state Ł3 algebra IS coded but orphaned from the driver pipeline. | `triet-pack/src/types.rs:297`; `triet-typecheck/src/capability_check.rs:131`; driver = ∅ | **Rescue the Trit↔level mapping; discard the manifest mechanism.** |
| 5 | Trap infra: `TrapCode::unwrap_user(1)` (trapnz → SIGILL family), range-check ADR-0044 is ready for use. | `crates/triet-jit/src/mir_lower.rs:2509` | `Defer` runtime-hook **reuses** this infra (see §5). |
| 6 | Schema §10: *"No special AST node ... No ACL, no syscall, **no runtime check** — capability = ownership."* | `spec/schema/triet-schema.yaml:1684,1742` | Synthesis **breaks** the "no runtime check" assumption (Defer requires a check) + adds syntax declarations → **ADR amends §10**, not silently. |

---

## 1. Mapping Ł3-Trit ↔ capability lifecycle (core coherence)

Ł3 algebra (Łukasiewicz 3-valued logic) maps directly onto the lifecycle of a capability token. This IS the third leg of coherence — the same `Trit{Positive, Zero, Negative} + Unknown` used in null (PA-3c sentinel) and logic (Trilean):

| Ł3 value | Level | Capability semantics | Enforcement point | Runtime cost |
|---|---|---|---|---|
| `Trit::Positive` | **Grant** | Token can be **minted** freely; possession = authority. Pure W2 ownership. | typecheck (
