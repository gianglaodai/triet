# ADR-0049: Fat-Pointer ABI for String (Slice provenance deferred)

## 1. Status
**Approved (O + G, 2026-06-08)** — Phase-0 CLOSED. Entering Phase-1 Implementation.

**G Constraints (invariants — must not be violated during implementation):**
1. **`free` shim ABI = discrete 2-arg `free(ptr, cap)`.** Forbidden to pass `*const FatStr` to consuming shims. Unpack fields via `stack_load` then call — explicit, fast, no redundant dereferences.
2. **`return-fat` (e.g. `concat`) = `sret` StackSlot + `ReturnShape::Struct`.** Reuses Gate A pattern. Caller allocates StackSlot, callee populates via implicit first argument.
3. **ObjectHeader 8B REMAINS UNCHANGED.** Heap layout: `[Header 8B][data...]`. `ptr` points to data; dealloc = `free(ptr - 8, layout(cap))`. Prepared for RefCount (ADR-0022) — avoid modifying offsets a second time.

## 2. Context & Motivation
Triet's current memory model (Tier C) manages strings (`String`) via a handle (I64) pointing to a contiguous heap block `[ObjectHeader 8B] [len i64] [cap i64] [data...]`.
All string modification operations (such as `append`, `concat`) follow a functional model: allocate a new block, copy data, free the old block, and return a new handle. This model is **Sound** (no dangling pointers) but incurs severe performance limitations (O(n) per push operation) and does not permit memory sharing (sub-slicing).

**Primary Motivation:**
We are NOT patching a UB bug. We are **unlocking features**:
1. Allowing `append` to achieve amortized O(1) through capacity-doubling and reallocation (which may relocate the buffer → ptr changes). The caller receives the new ptr via writeback. The fact that `ptr` can change is precisely (a) the raison d'etre of fat-pointer writeback, (b) the reason all shared borrows must be forbidden during an append.
2. Allowing mutable references to observe String mutations through pointers.
3. Supporting String Views / Slices (`&[T]`) sharing the underlying buffer with the original String without copying.

## 3. Architectural Decisions

### 3.1. Fat-Pointer Layout on StackSlot
- **Owned String**: Uses a **3-field** model `[ptr, len, cap]`. This allows `append` operations to read `cap` directly from the stack without dereferencing the heap, optimizing CPU cycle count.
- **Borrowed Slice (`&str` / `&[T]`)**: Uses a **2-field** model `[ptr, len]`. Slices do not manage capacity and are not responsible for deallocation.

### 3.2. Layout on Heap
- Heap block retains only `[ObjectHeader 8B] [data...]`.
- The `len` and `cap` fields are moved entirely to the StackSlot.
- The fat-pointer's `ptr` points directly to the `data` region.
- Deallocation/free operations compute the base address via `ptr - 8` and the deallocation size from `layout(cap)` (where `cap` must be passed from the StackSlot since it no longer resides on the heap).

### 3.3. Implementation Scope
- **String-only**: Tier D focuses exclusively on applying this model to `String`. `Vector` and `HashMap` remain unchanged (or temporarily disabled) to constrain the blast radius until the String ABI is proven stable.

## 4. Spike & Probes (Validation)
Prior to actual implementation, 4 ABI questions were dissected and concluded as follows:

1. **Q2 - Shim ABI:** *PROVEN (in part).*
   - `mutate-writeback` shim class (such as `append`, `clear`): Spike constructed and successfully proved SystemV C-ABI passing `*mut FatStr` in-place; caller reloads (`stack_load`) and observes the new ptr+len correctly.
   - *TBD:* 4 shim classes remaining to be measured: (a) `read-scalar` (len/is_empty) — shim can be completely eliminated since variables already reside on stack; (b) `read-buffer` (eq/contains) — **decided: unpack discrete arguments** (ptr, len for each string; eq(a_ptr, a_len, b_ptr, b_len) = 4 args, well within SysV's 6 registers), forbid `*const FatStr`; (c) `return-fat` (such as `concat`) — **decided: sret StackSlot + `ReturnShape::Struct`** (confirmed by G); (d) `free/deinit-shim` — since heap omits cap, free must accept cap from StackSlot, **decided: discrete 2-arg `free(ptr, cap)`**, forbid `*const FatStr`. Will be measured during implementation.
   - **G ABI Principle:** `*mut FatStr` is ONLY used for mutate-writeback shims (`append`/`clear`). All other shims — read-scalar, read-buffer, free/consuming — unpack discrete fields via `stack_load`, avoiding struct-ptr passing (eliminating redundant dereferences + avoiding L1 cache waste).
   - (c) `return-fat` (concat/sret): **PROVEN by spike** `spike_sret_string_roundtrip` + `spike_sret_ptr_writeback` (Slice 6 Phase-0). SystemV: struct 24B > 16B → automatic by-pointer implicit first arg. Callee writes {ptr,len,cap} to caller's slot via sret pointer; caller reloads. No new Cranelift mechanism needed — reuses Gate A `ReturnShape::Struct`.
2. **Q1/Q3 - Deinit & Move (Tombstone):** *Decided at Design-level.*
   - Tombstone against double-free: When moving a fat-pointer, only zeroing `ptr` (field 0) is load-bearing. Zeroing `len/cap` is purely hygiene. The free function leverages the existing guard `if ptr == 0`.
   - *DEFER:* Current spike is only a mock at the Rust layer. Converting `def_var` to `stack_store(0, slot, 0)` in Cranelift lowering (for real Move and Deinit) is reserved for the implementation phase.
3. **Q4 - Exclusivity (E2440):** *CLOSED for String-only scope.*
   - Moving from a single i64-handle to a 3-field StackSlot is a detail at the codegen layer (below MIR). At the borrowck layer, 1 String remains 1 Place. Exclusivity rules (E2440) are completely independent of StackSlot and remain correct.
   - *CAVEAT (Slice):* This fat-pointer design paves the way for "cheap sub-slice sharing". However, current borrowck lacks a provenance model linking aliases between a sub-slice (generated as a new local) and the buffer of the original String. The "slice provenance" problem is out of scope and belongs to a future ADR.

## 5. Consequences
- **Positive:** Paves the way for standard heap memory management, optimizes append performance, unlocks slice capabilities.
- **Negative:** Increases StackSlot complexity, requires rewriting all FFI shim logic and deinitialization for String.
- **Tier D Technical Debt:** Must resolve `is_propagated` bypass (nested scope) and unify the two borrowck tiers (typecheck + MIR) as recorded from handoff.

## 6. Slice 6 — Heap Trimming + External ABI Redefinition (Blueprint)

**Status: Approved (O+G, 2026-06-08)**

### 6.1. Phase-0 findings

- **Q2 return-fat sret:** PROVEN. Spike `spike_sret_string_roundtrip` + `spike_sret_ptr_writeback` confirmed: SystemV ABI automatically uses by-pointer implicit first arg for struct 24B > 16B. Callee writes {ptr,len,cap} into caller's slot; caller reloads accurately.
- **Q1 param fat-String:** PROVEN by spike `spike_byptr_param_roundtrip`. Manual by-pointer like Slice 5 append shim: caller `stack_addr(slot)` → passes 1 i64 arg (pointer-to-slot); callee receives i64=pointer, loads 3 fields {ptr,len,cap} from pointer. DOES NOT use Cranelift struct-param (does not exist in Gate A).
- **Q3 heap removal:** After L6-1/L6-2, all String boundaries (param, return) use slots. Heap no longer needs len/cap → layout `[Header 8B][data…]`. Data offset +16→+8. dealloc = `free(ptr−8, layout(cap))` with cap from slot/arg.

### 6.2. Implementation steps

1. **L6-1: param fat-String by-pointer.** Caller: before call, if param type = "String" → `stack_addr(slot, 0)`. Callee: receives via sret-style block param → `def_var(Local(0), sret_val)`. Regression: 77/84 green.
2. **L6-2: return fat-String sret (Approach d: JIT-only, MIR retains M4-escape).** String-sret = move-escape semantics on MIR (retains M4). sret population executed implicitly under JIT, DOES NOT use `Stmt::Return` copy-fields like struct(Copy). Lowerer: sets `ReturnShape::Struct` to signal JIT to use sret 24B, but DOES NOT allocate `sret_ptr` + DOES NOT take copy-fields path. Retains pure `Return[s]`. JIT callee: upon `Return[s]` + ReturnShape::Struct + type=String → `stack_load` {ptr,len,cap} from slot → store into sret buffer. JIT caller: allocates sret-slot before call, passes addr as implicit-first-arg; after call reads slot populated by callee. Concat shim: (sret_ptr, a_ptr, a_len, b_ptr, b_len) → writes {ptr,len,cap} into sret. Regression: 35/60/78 green.
3. **L6-3: trim heap len/cap.** `alloc`: remove `+8+8` len/cap → heap is only `[Header][data]`. Data offset +16→+8 in all shims (eq/contains/concat/append/clear/from_bytes). `free(ptr, cap)`: dealloc `ptr−8` with `layout(cap)` (cap from parameter).
4. **L6-4: retire Approach B.** Delete caller-populate heap-read (param entry + return-value populate). Delete append heap-sync. The slot is the single source of truth.

### 6.3. Endgame fixture

`endgame_string_roundtrip.tri`: String across multiple boundaries — f(s) receives fat param, appends (realloc, ptr changes), returns sret, caller appends further, compares content with eq. + double-free (move across boundary) + E2440.
