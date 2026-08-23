# ADR 0069 — ZST Capability Token Bearing Ł3-Trit (Rewrite-Era, Borrowck-Enforced)

> # ⚖️🩸 CORE PRINCIPLE (Carved in stone by G on 2026-06-25)
> # Capability = ownership. The gatekeeper (Borrow Checker) MUST hold the noose.
> A capability not enforced by borrowck via memory safety is merely **ornamental**.
> Grant/Ambient/Deny must be checked **fatally at compile-time**; only `Defer` (Unknown)
> may be dispatched to a runtime hook. There is no bypass.

**Status:** Proposal (scaffold clean slate — recon file:line first, NO code yet; awaiting G's step-by-step signature). Applies to Tier C+. This is the **strategic campaign completing COHERENCE VISION §8**: a single Ł3 algebra spanning **null (PA-3c) / logic (Trilean) / capability** — the missing third leg.

**Issue — Two disjoint worlds, neither closing VISION §8:**
- **World 1 (package-manifest, ADR-0016/0017/0018):** 4-state Ł3 algebra WAS coded (`CapabilityLevel{Deny=Trit::Negative, Ambient=Trit::Zero, Grant=Trit::Positive, Defer=Trilean::Unknown}` at `crates/triet-pack/src/types.rs:297`; `check_capabilities()` at `crates/triet-typecheck/src/capability_check.rs:131`) — **BUT orphaned**: `grep capability crates/triet-driver/src` = EMPTY. Enforcement died in the actual pipeline. Underlying `.khi`/`dao`/cross-package-linker machinery was mostly deleted or unwired → **G buried 0016/0017/0018**.
- **World 2 (Hardware-Token ZST, `spec/plans/phase6` + schema §10):** capability = ownership + move enforced by borrowck, ZERO runtime overhead, coherent with recently closed No-Box (Axis B) — **BUT "design only", and deliberately LACKING Ł3-levels** (binary "token present or not").

**Strategic Decision (G locked DIRECTION C — Synthesis, 2026-06-25):** Merge the two worlds. **Rescue the Ł3 algebra of World 1, discard the package-manifest mechanism; build upon the ownership/move engine of World 2.** ZST-tokens **firmly embody** Ł3-Trit: Grant/Ambient/Deny enforced fatally by borrowck at compile-time; Defer dispatches to runtime policy hooks + traps upon violation.

**ADR Relationships:**
- **BURIED:** ADR-0016 (package capability manifest), 0017 (policy resolution), 0018 (provenance prompt) — package-manifest era, inapplicable to single-file/JIT rewrite. Do not exhume.
- **Inherited:** S6 ownership (ADR-0022 §2, 5-form references), borrowck NLL E2420 (ADR-0025), No-Box move/Deinit machinery (ADR-0066/0067), trap-on-violation 2-signal (ADR-0044).
- **Amended:** schema §10 `HardwareToken` (`spec/schema/triet-schema.yaml:1684`) — see §8.

---

## 0. Current Reality (Recon measured file:line by O, 2026-06-25 — carved in stone, not guessed)

| # | Finding | Evidence (file:line) | Design Consequence |
|---|---|---|---|
| 1 | ZST construction `Cap {}` (empty struct) **does not lower** — falls into variable branch → "undefined local variable: Cap". Empty struct **DECLARATION** parses/typechecks/lowers OK. | probe `triet-driver run` (struct-construct path, `triet-lower`) | **Slice-0 prerequisite**: must allow ZST construction before anything else. |
| 2 | `is_copy` for `Struct` = `s.fields.iter().all(\|f\| f.ty.is_copy(...))`. Empty struct → `all()` on EMPTY iterator → **`true` → COPY**. | `crates/triet-mir/src/lib.rs:666` | ZST tokens default to **Copy** → borrowck DOES NOT track moves → silent bypass. **Tokens MUST be forced non-copy.** |
| 3 | Enforcement substrate **ALREADY EXISTS**: E2420 use-after-move gated on `is_copy(Some(body))`; `ParameterPassing::Move`; 5-form `ReferenceForm`. Test `write_twice(vga: &+ mutable VgaBuffer)` + `consume(vga)` runs correct ZST-move pattern. | `crates/triet-borrowck/src/checker.rs:388,618,690,948,981` + `lib.rs:339` | **move = transfer + revocation = E2420 — already built.** No need to reinvent. |
| 4 | 4-state Ł3 algebra ALREADY coded but orphaned from driver pipeline. | `triet-pack/src/types.rs:297`; `triet-typecheck/src/capability_check.rs:131`; driver = ∅ | **Rescue Trit ↔ level mapping; discard manifest mechanism.** |
| 5 | Trap infra: `TrapCode::unwrap_user(1)` (trapnz → SIGILL family), range-check ADR-0044 already uses it. | `crates/triet-jit/src/mir_lower.rs:2509` | `Defer` runtime hook **reuses** this infra (see §5). |
| 6 | Schema §10: *"No special AST node ... No ACL, no syscall, **no runtime check** — capability = ownership."* | `spec/schema/triet-schema.yaml:1684,1742` | Synthesis **breaks** assumption of "no runtime check" (Defer has checks) + adds syntax declarations → **ADR amends §10**, explicitly. |

---

## 1. Mapping Ł3-Trit ↔ Capability Lifecycle (Core Coherence)

The Ł3 algebra (Łukasiewicz 3-valued logic) maps directly onto the lifecycle of a capability token. This IS the third leg of coherence — sharing the exact same `Trit{Positive, Zero, Negative} + Unknown` operational in nulls (PA-3c sentinel) and logic (Trilean):

| Ł3 Value | Level | Capability Semantics | Where Enforced | Runtime Cost |
|---|---|---|---|---|
| `Trit::Positive` | **Grant** | Token permitted to be **minted** freely; possession = authority. Pure W2 ownership. | typecheck (allows mint) + borrowck (move/E2420) | **0 bytes** |
| `Trit::Zero` | **Ambient** | **RECEIVE-ONLY (M1, signed by G on 2026-06-25 — see §amend-A).** File LOSES authority to `mint X` (E2211 like Deny). ONLY way to use `X` = caller passes token ZST via **parameter** (explicit signature). Token flows downward from outer boundary (entry-point), CANNOT be generated within the program. | mint → E2211; **possession via param/binding = VALID** | 0 bytes |
| `Trit::Negative` | **Deny** | Token **CANNOT be minted** + **STRICTLY PROHIBITS any possession** (even receiving via parameter → error). | mint → E2211; **possession (param/binding type) → E2212** | (non-existent) |
| `Trilean::Unknown` | **Defer** | Token can be minted **conditionally**; all guarded ops emit runtime policy-hook checks; deny → **trap**. | typecheck allows mint + JIT inserts runtime check | **1 check + trap** (the ONLY cost) |

**Coherence Property:** Three static values (Pos/Zero/Neg) resolve completely at compile-time = zero-cost (true to W2 schema §10). Only `Unknown` — the intrinsic "undetermined" state of Ł3 — defers to runtime. This is not a compromise; this is **exact Ł3 semantics**: Unknown signifies "cannot be proven statically", thus must consult runtime. Exactly analogous to `Trilean` Unknown being disallowed in `if` (E1033) until resolved.

---

## 2. Surface Syntax (Decided by Giang, 2026-06-25 — `capability` decl + `mint`)

```triet
capability VgaBuffer grant     // Grant: free minting, zero-cost
capability DiskWrite defer     // Defer: mint → runtime hook check
capability RawPort   deny      // Deny: mint = E2211, possession = E2212 (forbidden)
capability UartPort  ambient   // Ambient: mint = E2211; ONLY received via param (receive-only)

function kernel_main(hw: Hardware) -> Unit {
    let vga = mint VgaBuffer;   // OK (grant) — ZST, 0 runtime bytes
    vga_driver(vga);            // vga MOVED (authority transferred)
    // vga_driver(vga);         // E2420 UseAfterMove — authority revoked
}
```

- **New Keywords: `capability`** (item declaration) + **`mint`** (prefix operator instantiating token). Not on the refuse list of ADR-0026 v2 §6 (actor/spawn/receive/send/async/await) → valid.
- **`grant`/`ambient`/`deny`/`defer` = Contextual Keywords** (meaningful only in the level position following `capability Name`) → **NOT globally reserved**, users may use them as identifiers elsewhere.
- `capability X <level>` defines a **ZST type** `X` (sizeof = 0) bound to an Ł3-level. Differs from regular `struct X {}`: (a) always non-copy (see §6), (b) carries a level, (c) instantiated exclusively via `mint` (not `X {}`).

---

## 3. AST/HIR Hook Points (Answering G's Question a)

`capability X grant` → **new** AST item `Item::Capability { name, level, span }`, NOT stuffed into `UserStruct` (keeps `UserStruct` clean; capability ≠ data struct). Level = 4-state enum reusing World 1's Ł3 mapping.

- **Schema-First:** Add node `Capability` + enum `CapabilityLevel{Grant,Ambient,Deny,Defer}` to `spec/schema/triet-schema.yaml`, run codegen → `crates/triet-syntax/src/generated/`. **DO NOT hand-edit generated code** (Track B rule #2). `CapabilityLevel` in `triet-pack` (`types.rs:297`) is a proven Ł3 mapping — schema mirrors it, but this is an independent rewrite-era AST node (not dragging in 0016 wire-format/manifests).
- **Type Representation:** Typecheck `Type` (`crates/triet-typecheck/src/types.rs`) adds variant `Capability { name, level }` OR reuses empty `UserStruct` + flags. Locked based on Slice 1 recon.
- **`mint X`** → AST `Expr::Mint { capability_name, span }`. Lowerer → instantiates ZST local (0 bytes, similar to `_ = const ()` but typed Capability). Resolves Slice-0 (finding #1) for ZSTs.

---

## 4. Borrow Checker Enforcement Rules (Answering G's Question b — The Noose)

Capability tokens are **ZST non-copy**. All enforcement **reuses** existing machinery (finding #3), WITHOUT authoring new engines:

1. **Possession = Ownership.** Holding a token ⇔ retaining a local of Capability type not yet moved/dropped.
2. **Move = Authority Transfer.** `f(vga)` moves token into callee → caller loses authority. Reusing `vga` → **E2420 UseAfterMove** (already functional, `checker.rs`). This is the revocation G demanded.
3. **Non-Copy = No Authority Duplication.** Tokens **strictly enforce** `is_copy == false` (§6), preventing token duplication where two parties simultaneously retain authority. This is the vital soundness anchor.
4. **Deny Blocks Minting + FORBIDS Possession.** `mint X` (deny) → **E2211** at mint-site. In addition: `X` appearing as the TYPE of any binding/parameter/field → **E2212 CapabilityNotPossessable** (Deny forbids all forms of possession, including via signatures). No token can exist.
5. **Ambient = Receive-Only (M1).** `mint X` (ambient) → **E2211** (file lacks minting authority). HOWEVER, `X` appearing as a parameter/binding type = **VALID** — caller passes token down (pure O-Cap: authority flows through signatures, capability cannot materialize from thin air). Distinction from Deny: Ambient permits receiving, Deny forbids receiving. Resolves entirely at compile-time, never at runtime.
6. **Grant = Zero-Cost.** Valid mint → ZST token; runtime only observes hardware addresses hardcoded in drivers (original W2). 0 bytes copied.

> **Soundness Invariant (Teeth prove via real crashes):** A guarded resource is acquired **exactly once** — borrowck proves this via move-exactly-once. Poisoning `is_copy → true` for Capability → duplicate acquisition MUST slip through (E2420 does not fire) → RED. (Reuses No-Box poison protocol.)

---

## 5. Defer → Runtime Policy Hook + Trap (Answering G's Question c)

`Defer` (Ł3 Unknown) is the SOLE case touching runtime. When `mint X` occurs with `X` at level `defer`:

- Token can still be minted (ZST), borrowck tracks moves identically to Grant — **memory safety is never relaxed**.
- JIT inserts, **AT THE MINT-SITE** (G LOCKED 2026-06-25 — NOT at guarded-ops: ZST evaporates at runtime, placing checks at guarded-ops inserts runtime checks at every use-site, destroying ZST efficiency. `mint` = "forging the token", checked once at mint-site; approved → invisible token flows through all functions at ZERO-COST; rejected → trap), a call to the **runtime policy hook** `extern "C" fn __triet_cap_check(cap_id: i64) -> i64` (Rust shim, family of `__triet_*` in `mir_lower.rs`). Hook returns Ł3-Trit: `+1` allow / `-1` deny / `0` (Unknown → treat as deny, fail-closed).
- **Deny → Trap.** Hook returning ≤ 0 → `trapnz` (`TrapCode::unwrap_user(N)`, finding #5). Uses **DEDICATED trap code** (e.g. `unwrap_user(2)`) decoupled from range-checks (ADR-0044 uses `user(1)`) → distinguishes "capability denied" from "arithmetic overflow" during core dump analysis. SIGILL family.
- **Fail-Closed:** Missing policy hook / panic / returning Unknown → treated as deny → trap. Capabilities MUST NOT unlock unless authorization is definitively proven. (Aligns with refuse-over-guess.)

> This is where the ADR **amends** schema §10 "no runtime check". Rationale: §10 only described the static case (Grant). `Defer` is an EXPLICIT developer opt-in for dynamic decisions — intentionally paying the cost of 1 check. Grant/Ambient/Deny remain zero-cost. Zero-cost-by-default is preserved; runtime cost occurs only when declaring `defer`.

---

## 6. Fixing the `is_copy` Hole (Finding #2 — Soundness Pin)

`MirType` adds a Capability classification **always `is_copy == false`** — NEVER routing through `Struct` `all(empty)==true`. Two approaches:
- (a) `MirType::Capability(name)` new variant → match arm returns `false` directly at `lib.rs:666`.
- (b) Reusing `MirType::Struct` for ZSTs: add "is_capability" flag + short-circuit `false` BEFORE `all()`.

**Mandatory Teeth:** Poison this branch to `true` → token mint → move → reuse stops firing E2420 → test turns RED.

---

## 7. Slice Plan (Scaffold — each slice recon→WO→D codes→O verifies teeth→O signs→G signs)

- **Slice 0 ✅ CLOSED (`8b06a28`):** `capability X grant` decl + `mint X` ZST 0-byte + `is_copy==false` (2-classifier defense-in-depth) + non-grant refuse E2211 + `public capability` refuse. Fixes findings #1+#2. (Absorbs Grant/Deny-mint from Slice 1.)
- **Slice 2 ✅ CLOSED (`ca8272e`):** Ambient receive-only + Deny no-possession (M1). Possession check at `resolve_type` (chokepoint for all annotations) → Deny as type = **E2212**; ambient/grant possessable. `mint` ambient → E2211 "receive-only". Clean split between E2212(possess) and E2211(mint). O verified 3 teeth.
- **Slice 3 ✅ CLOSED (`2dd4d5f`):** Defer runtime hook + trap user(2), checked at mint-site, fail-closed. `Statement::CapabilityCheck` (MIR) → JIT `__triet_cap_check` → `icmp ≤0` → `trapnz user(2)`. CAP_POLICY AtomicI64 default 0=Unknown=fail-closed. O verified 4 teeth (R-fail-closed boundary `≤` is critical).
- **Slice 4 ✅ CLOSED (`A2`, demo via param):** End-to-end demo (`fixture 278`, EXPECT 30) — mint grant ×2 → move across interleaved driver functions → RUN. Demonstrates all 4 levels (grant runs · ambient receive-only typechecks · deny/defer documented + fixture verified). **G locked A2** (separate params) over full struct-aggregate: `struct Hardware{vga}` destructure-move requires **field-level partial-moves** = a core Borrow Checker challenge, NOT to be bundled into capability ADR (scope creep). → **"Partial-move & Struct-ZST" = INDEPENDENT campaign** (including B8 gate fix at `lib.rs:72` mistaking ZST capability fields for heap).

---

## Alternatives Considered

**Overall Direction (G locked C):**
- **A — Wire W1 orphan into pipeline:** cheap, 4-state Ł3 lives quickly. **Rejected:** capability decoupled from ownership; borrowck does not hold the noose → "ornamental" (G). Disjoint systems, not coherent with No-Box.
- **B — Pure W2 ZST-tokens:** coherent with No-Box. **Rejected:** abandons established 4-state Ł3 algebra; reinvents Trit levels → soundness risks (G).
- **C — Synthesis (LOCKED):** ZST-tokens embody Ł3-Trit, borrowck-enforced, Defer → runtime. Touches borrowck core — but the ONLY path closing VISION §8 with zero loopholes.

**Syntax (Decided by Giang — `capability` decl):**
- `capability X grant` + `mint` (LOCKED): decouples capability from standard structs; keeps `UserStruct` clean.
- `@grant struct X {}` annotation: preserves schema §10 "no AST node" but requires AST field on `UserStruct` + codegen; conflates capabilities with data structs. **Rejected.**
- Pure W2 discarding levels: simplest but **violates G's mandate** ("tokens firmly embody Ł3-Trit"). **Rejected.**

**Defer Trap Code:**
- Reusing `user(1)` (range-check): conflates capability-denial with arithmetic-overflow in core dumps.
- **Dedicated `user(2)` (Proposed):** clearly distinguishable — selected.

---

## Consequences

### Positive
- **Closes COHERENCE VISION §8** — capability pillar of the Ł3 algebra (sharing `Trit + Unknown` with nulls/logic).
- Capability = memory safety: borrowck proves resources acquired exactly-once, zero-cost for Grant.
- Reuses 100% of move/E2420 machinery (No-Box) + traps (ADR-0044) — no new engine.
- Conclusively buries 0016/0017/0018; capabilities live in the actual driver pipeline, no longer orphaned.

### Risks to Mitigate (Teeth)
- **R-copy-bypass (Critical):** Token classified as Copy → duplicate acquisition escapes E2420 → silent duplication of authority. Teeth: poison `is_copy → true` → reuse-after-move stops erroring.
- **R-deny-leak:** `mint deny` fails to emit E2211 → forbidden capability instantiates tokens. Teeth: poison Deny check.
- **R-defer-fail-open:** Missing/Unknown hook fails to trap → bypass. Teeth: poison fail-closed → minting defer with denying hook must SIGILL; removing trap must slip through.
- **R-ambient-collapse-error:** Ambient improperly resolves Deny → Grant. Teeth: ambient in deny scope.

## §amend-A — Ambient = Receive-only (M1, Signed by G on 2026-06-25)

Original §1 scaffold stated "Ambient = inherit caller's level → collapse Grant/Deny" — **ambiguous, lacking single-file JIT mechanism** (no package hierarchy like W1). G dissected 3 recon models, discarding 2:
- **M2 Possession-gated** (mint if holding token) — DISCARDED: allowing `mint` based on possession = self-duplicating non-copy tokens = breaks ZST move-only invariant from Slice 0.
- **M3 Call-graph reachability** — DISCARDED: action-at-a-distance, destroys local reasoning (function B fails compilation because function A calls it).
- **M1 Receive-only (LOCKED)** — Pure O-Cap, consistent with ZST move-only:
  1. `capability X ambient` → file LOSES minting authority: `mint X` = **E2211** (diagnostic "receive-only").
  2. "Inheriting from caller" SPECIFICALLY means caller passes ZST token via **parameter**; authority flows down from outer boundary (entry-point such as `kernel_main(hw)`), NEVER generated within the program ("capability cannot materialize from thin air").
  3. **Distinction from Deny:** Deny strictly forbids ALL possession (param/binding types of deny-cap → **E2212**); Ambient forbids minting but PERMITS receiving via signatures. ⇒ Ambient = "explicit on function signatures, no implicit magic".

## Amendment Schema §10 (`HardwareToken`)
This ADR **modifies** two sentences of §10:
1. *"No special AST node"* → `Item::Capability` + `Expr::Mint` exist (capability ≠ pure data struct; pure-W2 destructuring remains for Hardware aggregate in Slice 4).
2. *"No runtime check"* → holds for Grant/Ambient/Deny (static); **`Defer` adds runtime hooks + traps** (opt-in, the sole cost). Zero-cost-by-default is preserved.
(Schema patch accompanies Slice 1, codegen-driven, no hand-editing generated code.)

## Effective Date
- Tier C+: Slices 0 → 4 in sequence, each slice individually signed by G.
- Defer (Slice 3) touches runtime — reviewed most rigorously.

---

**ADR-0069 Signatures:** O ✍️ (recon + draft 2026-06-25) · **G ✅ (approved 2026-06-25 — Ł3 mapping approved with both hands, fail-closed is truth, dedicated trap code `user(2)` mandatory, slice ordering preserved)** · Giang ✅ (locked direction C + syntax `capability`/`mint`)

**§amend-A (Ambient = M1 Receive-only):** O ✍️ (packaged M1 2026-06-25) · **G ✅ (RULED M1, buried M2/M3 — "implicit magic destroys ZST move-only and local reasoning")** · Giang ⏳

**Slice 0 ✅ `8b06a28` · Slice 2 ✅ `ca8272e` · Slice 3 ✅ `2dd4d5f` · Slice 4 ✅ (demo A2) — all signed by O+G.**

🔒🏁 **ADR-0069 SEALED (2026-06-25).** The **Ł3 capability algebra IS CLOSED** — Grant(+)/Ambient(0)/Deny(−) static zero-cost + Defer(Unknown) runtime trap fail-closed. **COHERENCE VISION §8 COMPLETE — all three pillars of null(PA-3c) / logic(Trilean) / capability share a single Ł3 algebra.** No-Box engine + ZST move-only guarantee static safety; trap user(2) eliminates dynamic ambiguity. Capability axis locked shut.

**Open Independent Campaign:** **Partial-move & Struct-ZST** (`let v = hw.vga` field-level move-state = core Borrow Checker, separate ADR + poison struct-destructuring/half-move/half-use + fix B8 gate at `lib.rs:72`).
