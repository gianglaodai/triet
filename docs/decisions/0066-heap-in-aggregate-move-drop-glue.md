# ADR 0066 — Heap-in-Aggregate: Move & Drop-glue (Flat, Slice 1)

> # ⚖️🩸 IRON LAW OF SOUNDNESS — ATOMIC INVARIANT (AXIS B PLEDGE)
> #  ⟶  MUST BE ATOMIC WITHIN THE SAME BASIC BLOCK.
> **ABSOLUTELY NO** function call / CFG branch / panic-or-trap point may be inserted into the gap between
> the byte-array copy phase and the writing of  to the source pointer. If any gap exists → panic → drop-glue triggers while
> BOTH variables point to the same heap allocation → **CATASTROPHIC DOUBLE-FREE**. Any Axis B WO/code violating this invariant =
> REJECT immediately without reading further. (Carved in stone by G, 2026-06-21.)

**Status:** ✅ **Decision — G SIGNED OFF on design blueprint 2026-06-21** (D is permitted to Usage: rustc [OPTIONS] INPUT

Options:
    -h, --help          Display this message
        --cfg <SPEC>    Configure the compilation environment.
                        SPEC supports the syntax `<NAME>[="<VALUE>"]`.
        --check-cfg <SPEC>
                        Provide list of expected cfgs for checking
    -L [<KIND>=]<PATH>  Add a directory to the library search path. The
                        optional KIND can be one of
                        <dependency|crate|native|framework|all> (default:
                        all).
    -l [<KIND>[:<MODIFIERS>]=]<NAME>[:<RENAME>]
                        Link the generated crate(s) to the specified native
                        library NAME. The optional KIND can be one of
                        <static|framework|dylib> (default: dylib).
                        Optional comma separated MODIFIERS
                        <bundle|verbatim|whole-archive|as-needed>
                        may be specified each with a prefix of either '+' to
                        enable or '-' to disable.
        --crate-type <bin|lib|rlib|dylib|cdylib|staticlib|proc-macro>
                        Comma separated list of types of crates
                        for the compiler to emit
        --crate-name <NAME>
                        Specify the name of the crate being built
        --edition <2015|2018|2021|2024|future>
                        Specify which edition of the compiler to use when
                        compiling code. The default is 2015 and the latest
                        stable edition is 2024.
        --emit <TYPE>[=<FILE>]
                        Comma separated list of types of output for the
                        compiler to emit.
                        Each TYPE has the default FILE name:
                        * asm - CRATE_NAME.s
                        * llvm-bc - CRATE_NAME.bc
                        * dep-info - CRATE_NAME.d
                        * link - (platform and crate-type dependent)
                        * llvm-ir - CRATE_NAME.ll
                        * metadata - libCRATE_NAME.rmeta
                        * mir - CRATE_NAME.mir
                        * obj - CRATE_NAME.o
                        * thin-link-bitcode - CRATE_NAME.indexing.o
        --print <INFO>[=<FILE>]
                        Compiler information to print on stdout (or to a file)
                        INFO may be one of
                        <all-target-specs-json|backend-has-mnemonic|backend-has-zstd|calling-conventions|cfg|check-cfg|code-models|crate-name|crate-root-lint-levels|deployment-target|file-names|host-tuple|link-args|native-static-libs|relocation-models|split-debuginfo|stack-protector-strategies|supported-crate-types|sysroot|target-cpus|target-features|target-libdir|target-list|target-spec-json|target-spec-json-schema|tls-models>.
    -g                  Equivalent to -C debuginfo=2
    -O                  Equivalent to -C opt-level=3
    -o <FILENAME>       Write output to FILENAME
        --out-dir <DIR> Write output to compiler-chosen filename in DIR
        --explain <OPT> Provide a detailed explanation of an error message
        --test          Build a test harness
        --target <TARGET>
                        Target tuple for which the code is compiled
    -A, --allow <LINT>  Set lint allowed
    -W, --warn <LINT>   Set lint warnings
        --force-warn <LINT>
                        Set lint force-warn
    -D, --deny <LINT>   Set lint denied
    -F, --forbid <LINT> Set lint forbidden
        --cap-lints <LEVEL>
                        Set the most restrictive lint level. More restrictive
                        lints are capped at this level
    -C, --codegen <OPT>[=<VALUE>]
                        Set a codegen option
    -V, --version       Print version info and exit
    -v, --verbose       Use verbose output

Additional help:
    -C help             Print codegen options
    -W help             Print 'lint' options and default settings
    --help -v           Print the full set of options rustc accepts from here). Applicable
to Tier C+. Permits **FLAT structs containing heap fields** () to construct, move
across function boundaries, and drop **without leaks or double-frees** — lifting the B8 barrier for the FLAT case.

**Issue:** The entire aggregate chain (struct/enum/nullable) up to ADR-0065 was **Copy-only** — the B8 barrier
(§4 ADR-0065) locked all heap fields/payloads (//) inside aggregates. Phase 1 Recon
(Axis B, 2026-06-21) proved: the **i64 value-model is VIABLE** (pointers can reside at field offsets in
StackSlot — ), and the only limitation was 10 construction-gate sites . But
lifting the barrier exposed **3 fatal hazards**: (1) whole-struct copy = duplicate pointers → double-free; (2) NO
struct drop-glue ( containing heap → ); (3) NO partial-move. This is a VISION campaign
(object-model/ownership/lifetime) — requiring a formal ADR before writing code.

**Related ADRs:** Settles deferral debt from ADR-0065 §4 (B8) + ADR-0062 §6 (heap-in-aggregate). Generalizes the
tombstone mechanism of ADR-0042 (Deinit) + Outcome heap drop-glue in ADR-0057 (). Underlying
value-model: ADR-0040 (heap layout) + ADR-0049 (fat-pointer String StackSlot). Ternary box  ≈ owner:
ADR-0022 §2 (drop-glue bound to owner-scope, NOT to object-header).

---

## Decision

Lift the B8 barrier for **FLAT heap-in-struct** (Slice 1) using **3 mechanisms**, following 3 guiding principles signed by G (2026-06-21):

### GP-1 — Inline per-struct static drop-glue (FORBID header/v-table)
The JIT retains full  +  at compile time. Upon , the JIT **walks the layout itself**:
for each heap field → statically emits a  instruction. **Zero runtime memory overhead** — NO
object-header, NO v-table, NO dynamic drop-flags. Cleanup code is "statically inserted at scope exit".

### GP-2 — Copy-then-Tombstone move semantics (emulating Move on copy value-model)
Moving a struct-containing-heap = byte-copy the entire StackSlot (copying pointer addresses) **+ immediately thereafter**
emit a **TOMBSTONE** (zero out heap pointers in the SOURCE slot). When the source scope reaches End-of-Scope, inline drop-glue
observes ptr==0 → skips deallocation → NO double-free. Generalizes the Outcome mechanism from  for
EACH heap field of the struct.

### GP-3 — FLAT only (recursive types deferred to Slice 2)
Slice 1 only covers **structs containing direct heap LEAVES** (// fields). Structs containing
structs-containing-heap (transitive/recursive ) **REMAIN refused** → deferred to Slice 2 (avoiding the
infinite-size recursion problem).

### Foundational Prerequisites (must be patched BEFORE running the 3 mechanisms)
- **M-1 Layout-sizing:**  () hardcoded all fields = 8B; fixup in ADR-0060
  (508-555) DID NOT handle heap types. **VERIFIED width table (measured from shims by O on 2026-06-21, audited per G):**

  | Heap type | Field width | Slot representation | Free shim | Per-field drop-glue |
  |---|---|---|---|---|
  |  | **24B** (fat) |  — slot caches len/cap |  **2-arg** | load ptr@off+0 + cap@off+16 → free |
  |  | **8B** (thin handle) |  — len/cap/data reside IN heap (header) |  **1-arg** | load ptr@off → free |
  |  | **8B** (thin handle) |  — len/cap/slots reside IN heap |  **1-arg** | load ptr@off → free |

  ⚠ **String ≠ Vector/HashMap regarding drop-arity** — drop-glue MUST dispatch per-field-type (String 2-arg with
  cap@+16; Vector/HashMap 1-arg). Declaring the wrong width or incorrect arity = byte-copy clobbers adjacent memory / frees invalid
  pointer = immediate SIGSEGV. Extends ADR-0060 fixup: , , .
- **M-2  audit:**  defaults  ("SOUND only while B8 blocks heap
  fields"). Construction-gate  passes  → for **nested-Struct** fields it assumes Copy →
  LEAKING transitive heap. Slice 1 gate must thread  + distinguish **direct-heap-leaf** (allowed) vs
  **transitive-heap** (refused → Slice 2).

### Concrete Execution — Life and Death Cycle of 



Memory & lifetime (StackSlot 24B per struct local; String free shim is already null-safe: no-op if ptr==0):

Statement::Deinit(p)

**Soundness Invariant (SI):** every heap allocation has **exactly 1** owner-slot with ptr≠0 at any point in the CFG.
Two forms of move:
- **Arg-move (by-pointer, ):** callee Drop-glue (INSIDE call) + caller -tombstone IMMEDIATELY
  after return. IRON LAW: NO panic/CFG-branch interposed between call return and .
- **Assign-move ():** byte-copy slot→slot (mir_lower.rs:1510) + tombstone-source ATOMIC within the same
  basic block (GP-2 literal). IRON LAW: NO gap between copy and tombstone.

Drop-glue frees ⟺ ptr≠0. Tombstone (ptr=0) + null-safe free = idempotent no-op. (Diagram amended 2026-06-21
per Recon-2: parameter ABI is by-pointer + ADR-0042 -reuse, NOT byte-copy at call site as in original draft.)

### Slice 1 Scope (STRICTLY bounded)
| In scope | Out of scope (deferred) |
|---|---|
| Construct  (direct heap leaf) | Nested/recursive structs containing heap (Slice 2) |
| Whole-struct MOVE across function boundary (by-move param + sret return) | **Partial move**  (move field out — Slice 1.x) |
| Inline drop-glue layout walk, freeing each heap leaf | Field reassignment  (drop-old + move-new — Slice 1.x) |
| Tombstone source after move | Enum payload heap (same mechanism, next slice) |
| String first (Vector/HashMap follow same pattern) |  owner-drop (design ADR-0022, backend not yet implemented) |

---

## Alternatives Considered

### Drop-glue code generation strategy
| # | Alternative | Pros | Cons | Conclusion |
|---|-------------|------|------|------------|
| 1 | **Inline per-struct static JIT emit** (GP-1) | Zero runtime overhead; leverages compile-time MirType; no memory bloat | More complex JIT (recursive layout walk in Slice 2) | ✅ **CHOSEN by G** — aligns with "no managed runtime" philosophy (VISION §7) |
| 2 | Object-header + drop-glue-table (each heap object carries cleanup fn-ptr) | Uniform drop, easier recursion | +8B/object runtime overhead; "OOP clutter" (G); breaks pure value-model | ❌ Rejected by G — "forbidden OOP clutter" |
| 3 | Dynamic drop-flag (runtime bitset tracking moved state) | Flexible move handling | Runtime overhead + state management; redundant since compile-time info is complete | ❌ Redundant — static value-model |

### Move semantics across boundaries
| # | Alternative | Pros | Cons | Conclusion |
|---|-------------|------|------|------------|
| 1 | **Copy-then-tombstone** (GP-2) | Reuses existing StackSlot byte-copy; tombstone = 1 store; precedent in Outcome | "Redundant copy" of address before invalidation (harmless — just 1 store) | ✅ **CHOSEN by G** — generalizes Outcome 1454-1457 |
| 2 | True move (transfer ownership without copying bytes) | No redundant copy | Breaks existing sret/pointer ABI; destroys value-model | ❌ Destroys foundations — value-model MUST survive |

### Scope
| # | Alternative | Conclusion |
|---|-------------|------------|
| 1 | **FLAT first, recursive deferred** (GP-3) | ✅ **CHOSEN by G** — avoids infinite-size recursion, manageable scope |
| 2 | Tackle recursive  immediately | ❌ Rejected by G — "biting off more than we can chew" |

---

## Consequences

### Positive
- Unlocks core use case:  — any record/DTO carrying text. Foundation for all practical data structures.
- i64 ABI value-model preserved intact — foundations unbroken (proven by Phase 1 recon).
- Zero runtime overhead (no headers/v-tables) — upholds the freestanding/no-managed-runtime promise (VISION §7).
- Inline drop-glue paves the way for  owner-drop (ADR-0022) with the rule: "own → compiler inserts free at scope exit".

### Negative
- JIT drop-glue + tombstone increases lowering complexity (layout walk, per-field free emission).
- Redundant byte-copy of pointer addresses on each move (offset by 1 tombstone store — harmless).
- Layout fixup (M-1) must know fat-pointer width of each heap type (hard-coded 24/8/8 — acceptable for Tier C).

### Risks to Mitigate
- **R1 Double-free** if tombstone is omitted in any CFG branch (e.g. move inside if-branch). → Safeguard: poison
  tombstone →  (pattern HP.2 ).
- **R2 Leak** if drop-glue misses a heap field (multi-field structs). → Safeguard: struct with 2 heap fields →
  poison layout walk skipping 2nd field → .
- **R3 Transitive LEAK** if Slice 1 gate (M-2) mistakes nested-struct-containing-heap for allowed-FLAT → JIT
   or leak. → Safeguard:  MUST be refused at construction.
- **R4 Move-then-use** (using  after moving into ) — borrowck must reject (E2420 move). Slice 1 relies on
  existing borrowck whole-local move tracking; partial-move deferred so no safety holes exist.
- **R5 Use-after-tombstone** within same scope (reading  after move) → ptr=0 → reading garbage. borrowck
  E2420 must block this BEFORE reaching JIT.

---

## Effective Date

- **Tier C Slice 1+** — FLAT heap-in-struct: construct + whole-move + inline drop-glue + tombstone. Only
   fields initially; / follow same pattern in Slice 1.x.
- **Deferred to Slice 2** — recursive/nested heap-in-aggregate, enum-payload heap, partial-move, field reassignment.
- Not retroactive: Copy-only aggregates (ADR-0065 and earlier) remain unchanged; B8 barrier still locks all cases
  OUTSIDE FLAT-struct-heap-leaf.

---

## Implementation Progress (Slice 1 — 4 Steps)

- **Step 1a (M-1+M-2+GP-1+STEP 4):** heap-leaf field sizing (String=24/Vector=8/HashMap=8) · B8-relax
  gate ( recursive on : direct heap leaf ALLOW, transitive + 
  REFUSE) · inline per-struct static drop-glue ( layout walk) · fat-store (String field
  projected dest copies len/cap — eliminating pass-by-luck UB). Fixtures 256/257 + 3 unit safeguards (R-cap/R-leak/R2/R3).
- **Step 1b (arg-move):**  whole-move across boundary — callee by-pointer drop-glue
  ( unifies slot-local + param;  address-based) + caller 
  tombstone ( 6 sites → ; Deinit struct-walk). ATOMIC IRON LAW:  at start of ret_bb.
  Fixture 258 + counting test (R-callee/R1-deinit/R1-arg, double-free FREE_COUNT==2).
- **Step 1c (assign-move):**  true-move (eliminating pseudo-copy alias) —  →
  ;  IMMEDIATELY AFTER move-Assign (ATOMIC). LOWER-ONLY (0 lines in JIT). Fixture 259 +
  counting test (R1-assign).
- **Step 1d (LOCK & SEAL):** Vector/HashMap fields + struct use-after-move E2420 — generic mechanisms from
  1a/1b/1c fully covered, sealed with fixtures 260/261/262 + counting safeguards (R-leak-vec/R-leak-hmap +
  **isolation scalpel:** poisoning  alone → Vector leaks 0, String in SAME struct survives 1 — proving
  drop-glue dispatches per-field-type) + R-e2420. 0 new compiler lines.

**Partial-move minefield ( extracting heap field) — FIRMLY DEFERRED to Slice 1.x (decided by G, 2026-06-22).**
Borrowck tracks whole-local move-state, NOT field-level state; additionally blocked by read-side gap (String field
read →  type, not yet lowered — same gap as §12.8 ADR-0065). Not cleanly reachable → touching it now =
interfering with Type Inference and disrupting Axis B focus. Slice 1 halts at the boundary of **whole-local move**.

**✅ Slice 1 (1a+1b+1c+1d) COMPLETED** — heap-leaf fields (//) construct + whole-move
(arg + assign) + inline drop-glue + tombstone + use-after-move E2420: **sound + locked**. B8 barrier lifted for
FLAT heap-in-struct; remains locked for nested/recursive/enum-payload (Slice 2).

---

## §AMEND-1 — Callee Stack-Slot Copy-In for Struct Parameters (2026-07-28)

**Status:** ✅ Decision — G signed off on WO-Param-Aggregate-CopyIn 2026-07-28. Implemented by D.

**Defect resolved by this amendment:** Struct parameters were excluded from  during the prologue
derivation loop (, Slice 2 loop under §"Concrete Execution" above, guard
), causing all ownership control gates, Deinit, and Assign executing through
 to be blind or fall into incorrect fallback branches, causing UB double-free (134), SIGSEGV
(139), and SIGILL (132) — depending on which specific MIR structure hit the gate.

Step 1b's diagram above ("Concrete Execution", step B.1) accurately described the ABI at that time: callee receives 
via pointer and "reads p via pointer (same memory region as main)" — this was **ONLY PROVEN SOUND**
for the structure  (fixture 258): callee DOES NOT touch any field of  before
dropping it through that exact pointer. Recon on 2026-07-28 measured that: any form that READS or MOVES a
field of  (S1 reading field, S2 moving field out, P4 internal whole-move then reading again, S9 forwarding 
to another call) crashed into ONE OF THE FOLLOWING THREE  gates, all blind to
parameters because Slice 2's  guard excluded parameters from slot allocation:
- field move-out tombstone () — silently skipped, failing to zero the moved-out field
  in the parameter's "slot" (non-existent) → double-free when both the new field-owner AND Drop(param)
  free the same pointer.
- 's struct branch () — fell back to ,
  only zeroing the Cranelift Variable (holding caller pointer), WITHOUT touching physical memory → subsequent whole-move reads
  through corrupt pointer.
- call-site arg forwarding () — fell back to , forwarding the caller's pointer
  DIRECTLY (not an independent copy) to the next callee → two functions alias the same buffer.

1. **Caller contract (reaffirming Step 1b — UNCHANGED).** Aggregates are still passed **by-pointer**:
   call-site writes  of caller's own slot into argument cell (). **WITHDRAW**
   all prior interpretations that callee directly aliasing caller memory was a "bug" — it is an intentional
   ABI contract (saving a 24B byte-copy at every call, per Step 1b's original rationale).
2. **Callee obligation (NEW).** Callee prologue **MUST** explicitly allocate a  for each
   plain struct parameter and **copy-in**  bytes from the caller's pointer — matching the pattern
   already used for String (), Enum (, WO-NullableEnumParamABI), and Outcome ().
   Struct is the FOURTH aggregate ABI lacking this step. Following copy-in, the callee operates on **its own private
   copy** — no longer aliasing caller memory directly, even though incoming ABI remains a pointer.
3. ** exception (sret).** When a function returns via sret,  is a pointer SUPPLIED BY CALLER
   to receive the return value — allocating a StackSlot for it would shadow that pointer and miscompile the return path
   (precedent in struct 172/14, precedent in enum P0 §"Step 1b").  remains strictly excluded from
   copy-in.
4. **Resulting invariant.** Copy-in automatically satisfies ALL  gates in the JIT
   (ownership/Deinit/Assign/Drop/forwarding) for parameters, WITHOUT requiring individual ad-hoc patches. Patching
   individual gates was a pattern that repeated and left holes **THREE times**:  (enum param, gate
   ),  (single  branch in
   , ), and this exact pattern before §AMEND-1 generalized it. Soundness of
   copy-in relies on an EXISTING, unchanging invariant:  emits 
   **unconditionally** immediately after  for all Move-typed arguments (ADR-0042 Q1) — this
   tombstones CALLER's copy after logical ownership transfer to callee, regardless of what callee does
   with its own copy. If this invariant disappeared, copy-in would turn double-frees into SILENT LEAKS
   instead of crashes — protected by a dedicated MIR-structural canary (,
   ), independent of JIT.
5. **Strictly bounded scope.** Only PLAIN  (not unwrapping , unlike Enum branch above).
    parameter REMAINS on the existing fail-closed refuse path
   ('s  branch; Drop/'s refuse
   ) — allocating a slot for it would break that refuse branch or double-deref.
   This is tracked as separate technical debt (), not opened in §AMEND-1.

**Escalated debt discovered during verification (NOT fixed — out of scope for §AMEND-1):** returning struct-by-
value (sret, non-nullable) containing a  field produces GARBAGE values (without crashing) when that field
is written into the sret buffer via field-projected  () — destination 
(sret) NEVER has a  entry, so the / synchronization step specific to String
(, gated on ) never runs when destination field is sret.
REPRODUCIBLE WITH 0 PARAMETERS ( fails) — completely
independent of parameter copy-in, a pre-existing hole not covered by any prior fixture (440 only locked /Nullable case).
Tracked in .

**Signatures §AMEND-1:** O: drafted WO 2026-07-28 (49-site audit table + 3 mandatory semantics checkpoints) ·
G: ✅ signed off · D: executed, verified across all 8 fixtures (543-550) + counting ()
+ manual snapshot poisoning (mir_lower.rs, triet-lower/src/lib.rs) — no M	docs/decisions/0001-nullable-memory-layout.md
M	docs/decisions/0002-fstring-format-spec.md
M	docs/decisions/0003-iterator-protocol.md
M	docs/decisions/0004-multiline-string-indent.md
M	docs/decisions/0005-module-system.md
M	docs/decisions/0006-ternary-packaging-vision.md
M	docs/decisions/0007-ir-design.md
M	docs/decisions/0008-triv-binary-format.md
M	docs/decisions/0009-version-gate-policy.md
M	docs/decisions/0010-ternary-native-ir.md
M	docs/decisions/0011-abi-metadata-format.md
M	docs/decisions/0012-witness-table-dispatch.md
M	docs/decisions/0013-semver-linking-policy.md
M	docs/decisions/0014-hash-scheme-refinement.md
M	docs/decisions/0015-package-store-layout.md
M	docs/decisions/0016-capability-type-system.md
M	docs/decisions/0017-trilean-policy-hook.md
M	docs/decisions/0018-capability-loader-semantics.md
M	docs/decisions/0022-trit-balanced-ownership.md
M	docs/decisions/0024-khi-dao-identity-naming.md
M	docs/decisions/0025-borrow-checker-rules.md
M	docs/decisions/0026-actor-boundary-send-rules.md
M	docs/decisions/0027-diagnostic-format-standard.md
M	docs/decisions/0034-jit-aggregate-coverage.md
M	docs/decisions/0037-enum-tagged-union-layout.md
M	docs/decisions/0038-comparable-trait-deferred.md
M	docs/decisions/0039-nullable-operator-family.md
M	docs/decisions/0040-heap-aggregate-layout.md
M	docs/decisions/0041-nullable-representation-bac-a.md
M	docs/decisions/0042-ownership-across-boundary.md
M	docs/decisions/0043-hashmap-representation.md
M	docs/decisions/0044-arithmetic-range-enforcement.md
M	docs/decisions/0046-return-borrow-elision.md
M	docs/decisions/0047-read-ops-extension.md
M	docs/decisions/0055-block-body-tail-expression.md
M	docs/decisions/0056-heap-value-merge.md
M	docs/decisions/0059-stack-borrow-heap-vector-hashmap.md
M	docs/decisions/0060-nested-aggregate-layout.md
M	docs/decisions/0061-trait-system-tier1-static-dispatch.md
M	docs/decisions/0062-heap-nullable-ptr-sentinel-repr.md
M	docs/decisions/0069-zst-capability-token-luk3.md
M	docs/decisions/0077-typed-vector-p1.md
M	docs/decisions/0078-typed-hashmap-p1-value.md
M	docs/decisions/0080-hashmap-string-key.md
M	docs/decisions/0081-get-borrow-mutable.md
M	docs/decisions/0082-aggregate-by-value-collection-element.md
M	docs/decisions/0085-shim-meta-totality-verify-gate.md
M	docs/decisions/0087-builtin-print-overloads-and-io-shim.md
Your branch is ahead of 'origin/main' by 2 commits.
  (use "git push" to publish your local commits).

---

**Signatures ADR-0066:** O: ✅ (Phase 1 recon + architectural blueprint + Vector/HashMap width verification via shims) ·
G: ✅ (signed design blueprint 2026-06-21 — 3 GPs + bounded whole-move scope + merged M-1/M-2 into Slice 1 + carved ATOMIC IRON LAW). D permitted to Usage: rustc [OPTIONS] INPUT

Options:
    -h, --help          Display this message
        --cfg <SPEC>    Configure the compilation environment.
                        SPEC supports the syntax `<NAME>[="<VALUE>"]`.
        --check-cfg <SPEC>
                        Provide list of expected cfgs for checking
    -L [<KIND>=]<PATH>  Add a directory to the library search path. The
                        optional KIND can be one of
                        <dependency|crate|native|framework|all> (default:
                        all).
    -l [<KIND>[:<MODIFIERS>]=]<NAME>[:<RENAME>]
                        Link the generated crate(s) to the specified native
                        library NAME. The optional KIND can be one of
                        <static|framework|dylib> (default: dylib).
                        Optional comma separated MODIFIERS
                        <bundle|verbatim|whole-archive|as-needed>
                        may be specified each with a prefix of either '+' to
                        enable or '-' to disable.
        --crate-type <bin|lib|rlib|dylib|cdylib|staticlib|proc-macro>
                        Comma separated list of types of crates
                        for the compiler to emit
        --crate-name <NAME>
                        Specify the name of the crate being built
        --edition <2015|2018|2021|2024|future>
                        Specify which edition of the compiler to use when
                        compiling code. The default is 2015 and the latest
                        stable edition is 2024.
        --emit <TYPE>[=<FILE>]
                        Comma separated list of types of output for the
                        compiler to emit.
                        Each TYPE has the default FILE name:
                        * asm - CRATE_NAME.s
                        * llvm-bc - CRATE_NAME.bc
                        * dep-info - CRATE_NAME.d
                        * link - (platform and crate-type dependent)
                        * llvm-ir - CRATE_NAME.ll
                        * metadata - libCRATE_NAME.rmeta
                        * mir - CRATE_NAME.mir
                        * obj - CRATE_NAME.o
                        * thin-link-bitcode - CRATE_NAME.indexing.o
        --print <INFO>[=<FILE>]
                        Compiler information to print on stdout (or to a file)
                        INFO may be one of
                        <all-target-specs-json|backend-has-mnemonic|backend-has-zstd|calling-conventions|cfg|check-cfg|code-models|crate-name|crate-root-lint-levels|deployment-target|file-names|host-tuple|link-args|native-static-libs|relocation-models|split-debuginfo|stack-protector-strategies|supported-crate-types|sysroot|target-cpus|target-features|target-libdir|target-list|target-spec-json|target-spec-json-schema|tls-models>.
    -g                  Equivalent to -C debuginfo=2
    -O                  Equivalent to -C opt-level=3
    -o <FILENAME>       Write output to FILENAME
        --out-dir <DIR> Write output to compiler-chosen filename in DIR
        --explain <OPT> Provide a detailed explanation of an error message
        --test          Build a test harness
        --target <TARGET>
                        Target tuple for which the code is compiled
    -A, --allow <LINT>  Set lint allowed
    -W, --warn <LINT>   Set lint warnings
        --force-warn <LINT>
                        Set lint force-warn
    -D, --deny <LINT>   Set lint denied
    -F, --forbid <LINT> Set lint forbidden
        --cap-lints <LEVEL>
                        Set the most restrictive lint level. More restrictive
                        lints are capped at this level
    -C, --codegen <OPT>[=<VALUE>]
                        Set a codegen option
    -V, --version       Print version info and exit
    -v, --verbose       Use verbose output

Additional help:
    -C help             Print codegen options
    -W help             Print 'lint' options and default settings
    --help -v           Print the full set of options rustc accepts from this point.
