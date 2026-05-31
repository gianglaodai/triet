//! v0.11.x.jit.3 Step 3 (ADR-0033 §3 + Addendum) — the Path-A
//! relocating loader: turn a `cranelift-object` `.o` back into
//! executable code Triết owns end-to-end (no system linker, no
//! `dlopen` — the OS-capable / from-scratch identity per VISION).
//!
//! Mechanism: copy `.text` into an anonymous RW `mmap`, build a small
//! GOT for external `__triet_*` symbols, apply the bounded relocation
//! set (verified empirically in `aot.rs` tests — `R_X86_64_PLT32`/`PC32`
//! for in-object calls, `R_X86_64_GOTPCREL{,X}` for shims), then flip
//! the mapping to RX with `make_exec`.
//!
//! **W^X (Addendum constraint 4c):** the mapping is `MmapMut` (RW, never
//! executable) while we copy + patch, then `make_exec` (RX, write
//! dropped) — never RW+X simultaneously. Enforced by construction via
//! memmap2's typed API.
//!
//! **Refuse-on-unknown (constraint 3):** any relocation type outside the
//! verified allowlist, any unresolved symbol, any out-of-range
//! displacement, or an unsupported format/arch → [`JitError::Cache`],
//! which the dispatcher (Step 4) turns into a silent cache miss + fresh
//! compile (§8). We never guess-patch — a wrong-but-valid patch is
//! silent memory corruption, the failure mode this whole regimen exists
//! to prevent.
//!
//! This module itself contains **no `unsafe`**: memmap2's safe API
//! (`map_anon` / `make_exec`) + plain slice writes do all the work.
//! Executing the loaded code (transmute of a code address to a `fn`
//! pointer + call) is the one unsafe step, and it lives at the call
//! site — the audited [`crate::dispatch_integer`] pattern — not here.
//!
//! [ADR-0033 §3]: ../../../docs/decisions/0033-aot-cache-cranelift-object.md

#![allow(clippy::redundant_pub_crate)]
// Step 3 lands the loader + its tests; the dispatcher that *calls* it on
// a cache hit lands in Step 4. Until then these items are exercised only
// by tests. Removed once Step 4 wires them.
#![allow(dead_code)]

use std::collections::HashMap;

use memmap2::MmapMut;
use object::{
    Architecture, BinaryFormat, Object, ObjectSection, ObjectSymbol, RelocationFlags,
    RelocationTarget, SectionKind, SymbolIndex,
};

use crate::JitError;

/// Resolves an undefined/external symbol (`__triet_*` shim or a
/// Cranelift libcall) to its host address per [ADR-0033 §3]. Built from
/// the `SHIM_TABLE` (see [`shim_symbol_resolver`]); mockable in tests.
///
/// [ADR-0033 §3]: ../../../docs/decisions/0033-aot-cache-cranelift-object.md
pub(crate) type SymbolResolver<'a> = dyn Fn(&str) -> Option<usize> + 'a;

/// Executable code produced by a [`CodeLoader`]: owns the RX mapping so
/// the code stays live for the process, plus a defined-function
/// symbol→host-address map.
#[derive(Debug)]
pub(crate) struct LoadedCode {
    /// Owns the mapping — dropping `LoadedCode` unmaps the code. Held
    /// only to keep the addresses in `functions` valid; never read.
    _mmap: memmap2::Mmap,
    functions: HashMap<String, usize>,
}

impl LoadedCode {
    /// Host address of a defined function by its (mangled) symbol name.
    pub(crate) fn function_addr(&self, symbol: &str) -> Option<usize> {
        self.functions.get(symbol).copied()
    }
}

/// Backend-agnostic code loader (ADR-0033 Addendum safety constraint 1).
/// v0.11 ships the ELF/`x86_64` impl; the eventual balanced-ternary
/// backend (or any other arch) slots a sibling impl behind this trait
/// without touching the dispatcher. Tests use a mock resolver.
pub(crate) trait CodeLoader {
    /// Parse `object_bytes`, map `.text`, resolve external symbols via
    /// `resolve`, apply relocations, and return executable
    /// [`LoadedCode`].
    ///
    /// # Errors
    /// [`JitError::Cache`] on any refuse condition (constraint 3):
    /// unsupported format/arch, zero or multiple text sections, a
    /// relocation type outside the verified allowlist, an unresolved
    /// symbol, an out-of-range displacement, or an mmap/mprotect
    /// failure. The caller treats it as a cache miss (§8).
    fn load(
        &self,
        object_bytes: &[u8],
        resolve: &SymbolResolver<'_>,
    ) -> Result<LoadedCode, JitError>;
}

/// The ELF / `x86_64` (POSIX) loader — the only target v0.11 supports
/// per ADR-0018 POSIX-first precedent + Addendum constraint 2.
pub(crate) struct ElfX86_64Loader;

/// Size of a GOT slot (one 64-bit absolute address).
const GOT_ENTRY: usize = 8;

impl CodeLoader for ElfX86_64Loader {
    // The load path is one linear sequence (parse → collect → map →
    // patch → exec → index); splitting it would scatter the safety
    // invariants that must hold in order. Kept whole + documented.
    #[allow(clippy::too_many_lines)]
    fn load(
        &self,
        object_bytes: &[u8],
        resolve: &SymbolResolver<'_>,
    ) -> Result<LoadedCode, JitError> {
        let file =
            object::File::parse(object_bytes).map_err(|e| cache(format!("ELF parse: {e}")))?;
        // Constraint 2: ELF + x86_64 only. Anything else is a clean miss
        // (tier-down), never a corruption.
        if file.format() != BinaryFormat::Elf {
            return Err(cache("unsupported object format (ELF only)"));
        }
        if file.architecture() != Architecture::X86_64 {
            return Err(cache("unsupported architecture (x86_64 only)"));
        }

        // Exactly one executable section — cranelift-object emits a
        // single `.text`. Zero or many → refuse (bounded surface).
        let mut text_sections = file.sections().filter(|s| s.kind() == SectionKind::Text);
        let text = text_sections
            .next()
            .ok_or_else(|| cache("no executable (.text) section"))?;
        if text_sections.next().is_some() {
            return Err(cache("multiple executable sections — unsupported"));
        }
        let text_index = text.index();
        let text_bytes = text.data().map_err(|e| cache(format!(".text data: {e}")))?;
        let text_len = text_bytes.len();

        // ── Pass 1: collect relocations + resolve the external GOT set ──
        let mut relocs: Vec<PendingReloc> = Vec::new();
        // Insertion-ordered: name → GOT slot index; `got_addrs[idx]` is
        // the resolved host address written into that slot.
        let mut got_index: HashMap<String, usize> = HashMap::new();
        let mut got_addrs: Vec<usize> = Vec::new();

        for (offset, reloc) in text.relocations() {
            let RelocationFlags::Elf { r_type } = reloc.flags() else {
                return Err(cache(format!(
                    "non-ELF relocation flags: {:?}",
                    reloc.flags()
                )));
            };
            let RelocationTarget::Symbol(sym) = reloc.target() else {
                return Err(cache("section-relative relocation — unsupported"));
            };
            match r_type {
                R_X86_64_PC32 | R_X86_64_PLT32 => {}
                R_X86_64_GOTPCREL | R_X86_64_GOTPCRELX | R_X86_64_REX_GOTPCRELX => {
                    // External symbol → ensure a resolved GOT slot exists
                    // (refuse-on-unresolved, constraint 3).
                    let name = symbol_name(&file, sym)?;
                    if let std::collections::hash_map::Entry::Vacant(slot) = got_index.entry(name) {
                        let addr = resolve(slot.key())
                            .ok_or_else(|| cache(format!("unresolved symbol: {}", slot.key())))?;
                        slot.insert(got_addrs.len());
                        got_addrs.push(addr);
                    }
                }
                other => return Err(cache(format!("unsupported relocation type {other}"))),
            }
            relocs.push(PendingReloc {
                offset: usize::try_from(offset).map_err(|_| cache("relocation offset overflow"))?,
                r_type,
                sym,
                addend: reloc.addend(),
            });
        }

        // ── Layout: [.text][pad→8][GOT slots] in one anonymous mapping ──
        let got_base_off = align_up(text_len, GOT_ENTRY);
        // map_anon requires len > 0 even for a code-free object.
        let total = (got_base_off + got_addrs.len() * GOT_ENTRY).max(1);
        let mut map = MmapMut::map_anon(total).map_err(|e| cache(format!("mmap RW: {e}")))?;
        map[..text_len].copy_from_slice(text_bytes);
        // Base address is stable across `make_exec` (mprotect doesn't
        // move the mapping), so addresses computed here stay valid.
        let base = map.as_ptr() as usize;
        let got_base = base + got_base_off;

        // Write each GOT slot's absolute resolved host address.
        for (i, addr) in got_addrs.iter().enumerate() {
            let slot = got_base_off + i * GOT_ENTRY;
            map[slot..slot + GOT_ENTRY].copy_from_slice(&(*addr as u64).to_le_bytes());
        }

        // ── Pass 2: patch each relocation (32-bit PC-relative) ──
        for r in &relocs {
            let site = base + r.offset;
            let target = match r.r_type {
                R_X86_64_PC32 | R_X86_64_PLT32 => {
                    // In-object call: the target must be a symbol defined
                    // in *this* `.text` (so its address lands in our
                    // mapping). Anything else → refuse.
                    let symbol = file
                        .symbol_by_index(r.sym)
                        .map_err(|e| cache(format!("symbol: {e}")))?;
                    if symbol.section_index() != Some(text_index) {
                        return Err(cache("PC32/PLT32 target not defined in .text"));
                    }
                    base + usize::try_from(symbol.address())
                        .map_err(|_| cache("symbol address overflow"))?
                }
                _ => {
                    // GOTPCREL family: target is the symbol's GOT slot.
                    let name = symbol_name(&file, r.sym)?;
                    let idx = *got_index
                        .get(&name)
                        .ok_or_else(|| cache("GOT slot missing (internal)"))?;
                    got_base + idx * GOT_ENTRY
                }
            };
            let disp = pc_relative_i32(target, site, r.addend)
                .ok_or_else(|| cache("relocation displacement out of i32 range"))?;
            map[r.offset..r.offset + 4].copy_from_slice(&disp.to_le_bytes());
        }

        // ── W^X flip: RW → RX (never RW+X simultaneously). ──
        let exec = map
            .make_exec()
            .map_err(|e| cache(format!("mprotect RX: {e}")))?;

        // ── Defined-function symbol → host address map ──
        let mut functions = HashMap::new();
        for symbol in file.symbols() {
            if symbol.is_definition()
                && symbol.section_index() == Some(text_index)
                && let Ok(name) = symbol.name()
            {
                let addr = base
                    + usize::try_from(symbol.address())
                        .map_err(|_| cache("symbol address overflow"))?;
                functions.insert(name.to_string(), addr);
            }
        }

        Ok(LoadedCode {
            _mmap: exec,
            functions,
        })
    }
}

/// One relocation to apply, decoded from the object's `.text`.
struct PendingReloc {
    /// Byte offset of the 4-byte field within `.text`.
    offset: usize,
    /// ELF `R_X86_64_*` type code (validated against the allowlist).
    r_type: u32,
    /// Target symbol.
    sym: SymbolIndex,
    /// Explicit RELA addend.
    addend: i64,
}

/// The production [ADR-0033 §3] symbol resolver: `__triet_*` shims via
/// the `SHIM_TABLE` (`production_shim_entries`), plus the (currently
/// empty) Cranelift libcall table — Triết IR uses no floating point at
/// v0.11, so no libcalls are emitted. No `dlsym`. Reused by Step 4's
/// dispatcher.
///
/// [ADR-0033 §3]: ../../../docs/decisions/0033-aot-cache-cranelift-object.md
pub(crate) fn shim_symbol_resolver(name: &str) -> Option<usize> {
    crate::shims::production_shim_entries()
        .iter()
        .find(|e| e.symbol == name)
        .map(|e| e.addr)
}

/// 32-bit PC-relative displacement `target + addend - site`, or `None`
/// if it doesn't fit in `i32`. The single arithmetic core of the
/// patcher — `proptest`-fuzzed against an independent `i128` inverse
/// (Addendum constraint 4b). Done in `i128` so the intermediate never
/// wraps regardless of where the kernel placed the mapping.
fn pc_relative_i32(target: usize, site: usize, addend: i64) -> Option<i32> {
    let disp = i128::from(target as u64) + i128::from(addend) - i128::from(site as u64);
    i32::try_from(disp).ok()
}

/// Round `n` up to the next multiple of `align` (a power of two).
const fn align_up(n: usize, align: usize) -> usize {
    (n + align - 1) & !(align - 1)
}

/// Look up a symbol's name as an owned `String`, mapping any failure to
/// a refuse-load [`JitError::Cache`].
fn symbol_name(file: &object::File<'_>, sym: SymbolIndex) -> Result<String, JitError> {
    let symbol = file
        .symbol_by_index(sym)
        .map_err(|e| cache(format!("symbol: {e}")))?;
    symbol
        .name()
        .map(str::to_string)
        .map_err(|e| cache(format!("symbol name: {e}")))
}

fn cache(reason: impl Into<String>) -> JitError {
    JitError::Cache {
        reason: reason.into(),
    }
}

// ── ELF x86_64 relocation type codes (the verified allowlist) ──────
// Named locally to keep the match arms readable; values match
// `object::elf::R_X86_64_*`. The set is exactly what cranelift-object
// 0.132 + PIC emits (verified empirically in `aot.rs`): PC-relative
// calls + the GOTPCREL family for external shims. `R_X86_64_64`
// (absolute data) is intentionally absent — Triết IR emits no data
// relocations yet, and refusing it (rather than guess-handling) is the
// safe default per constraint 3.
const R_X86_64_PC32: u32 = object::elf::R_X86_64_PC32;
const R_X86_64_PLT32: u32 = object::elf::R_X86_64_PLT32;
const R_X86_64_GOTPCREL: u32 = object::elf::R_X86_64_GOTPCREL;
const R_X86_64_GOTPCRELX: u32 = object::elf::R_X86_64_GOTPCRELX;
const R_X86_64_REX_GOTPCRELX: u32 = object::elf::R_X86_64_REX_GOTPCRELX;

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use triet_ir::{
        BasicBlock, BlockId, BuiltinName, ConstantPool, FuncId, Function, Instruction, IrModule,
        IrProgram, Operand, TypeTag, ValueId,
    };
    use triet_modules::{AbsolutePath, ModulePath};

    use super::*;

    fn func(id: FuncId, name: &str, ret: TypeTag, instructions: Vec<Instruction>) -> Function {
        let mut block = BasicBlock::new(BlockId(0), Some("entry".to_string()));
        block.instructions = instructions;
        let mut f = Function::new(id, Some(name.to_string()), vec![], ret);
        f.blocks = vec![block];
        f
    }

    fn program(functions: Vec<Function>, constants: ConstantPool) -> IrProgram {
        let path = AbsolutePath::new(ModulePath::new(vec!["khi".to_string()]), String::new());
        IrProgram {
            modules: vec![IrModule { path, functions }],
            constants,
            witness_tables: Vec::new(),
        }
    }

    /// `helper() = 7`, `main() = helper()` — exercises an in-object
    /// `PLT32` call through the loader, executed end-to-end.
    fn call_local_program() -> IrProgram {
        let mut pool = ConstantPool::new();
        let seven = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(7).unwrap(),
        ));
        let helper = func(
            FuncId(0),
            "helper",
            TypeTag::Integer,
            vec![
                Instruction::Const {
                    dest: ValueId(0),
                    constant: seven,
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let main = func(
            FuncId(1),
            "main",
            TypeTag::Integer,
            vec![
                Instruction::CallLocal {
                    dest: Some(ValueId(0)),
                    callee: FuncId(0),
                    args: vec![],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        program(vec![helper, main], pool)
    }

    /// `to_text() = TextFromInteger(123)` — a single-block function that
    /// calls the `__triet_text_from_integer` shim (external GOTPCREL).
    fn shim_call_program() -> IrProgram {
        let mut pool = ConstantPool::new();
        let n = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(123).unwrap(),
        ));
        let to_text = func(
            FuncId(0),
            "to_text",
            TypeTag::String,
            vec![
                Instruction::Const {
                    dest: ValueId(0),
                    constant: n,
                },
                Instruction::CallBuiltin {
                    dest: Some(ValueId(1)),
                    name: BuiltinName::TextFromInteger,
                    args: vec![Operand::Value(ValueId(0))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        program(vec![to_text], pool)
    }

    /// A resolver that never resolves anything — drives the
    /// refuse-on-unresolved path.
    fn deny_resolver(_: &str) -> Option<usize> {
        None
    }

    #[test]
    fn loads_and_executes_in_object_call() {
        // Path-A value parity: a function loaded via the relocating
        // loader must return the same value the IR (and VM) would —
        // here `main() = helper() = 7`. Exercises PLT32 patch + W^X +
        // execution end-to-end.
        let (object_bytes, manifest) =
            crate::aot::emit_object(&call_local_program()).expect("emit");
        let loaded = ElfX86_64Loader
            .load(&object_bytes, &deny_resolver)
            .expect("load (no external symbols)");

        let main_symbol = &manifest
            .function_table
            .iter()
            .find(|e| e.func_id == 1)
            .expect("main in table")
            .symbol;
        let addr = loaded.function_addr(main_symbol).expect("main address");

        // SAFETY: `addr` points at code the loader just mapped RX from
        // `main`'s `cranelift-object` output. `main` is a 0-arg
        // `i64`-returning function (TypeTag::Integer → I64,
        // CallConv::SystemV), so the transmute to `extern "C" fn() -> i64`
        // matches its real ABI. Mirrors `crate::dispatch_integer`.
        #[allow(unsafe_code)]
        let result = unsafe {
            let f: extern "C" fn() -> i64 = std::mem::transmute(addr as *const ());
            f()
        };
        assert_eq!(result, 7, "loaded main() must return helper() == 7");
    }

    #[test]
    fn loads_external_shim_via_got_with_resolver() {
        // The shim is undefined in the .o → resolved via SHIM_TABLE into
        // a GOT slot. Loading must succeed (GOT built, GOTPCREL patched).
        let (object_bytes, _manifest) =
            crate::aot::emit_object(&shim_call_program()).expect("emit");
        let loaded = ElfX86_64Loader
            .load(&object_bytes, &shim_symbol_resolver)
            .expect("load with shim resolver");
        // The defining function is present + addressable.
        assert!(
            loaded.functions.keys().any(|k| k.starts_with("to_text")),
            "to_text function symbol present: {:?}",
            loaded.functions.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn refuses_unresolved_external_symbol() {
        // Same shim program, but the resolver denies everything → the
        // GOTPCREL symbol is unresolved → refuse-load (constraint 3).
        let (object_bytes, _manifest) =
            crate::aot::emit_object(&shim_call_program()).expect("emit");
        let err = ElfX86_64Loader
            .load(&object_bytes, &deny_resolver)
            .expect_err("must refuse on unresolved symbol");
        match err {
            JitError::Cache { reason } => {
                assert!(reason.contains("unresolved symbol"), "reason: {reason}");
            }
            other => panic!("expected JitError::Cache, got {other:?}"),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn loaded_code_is_rx_not_writable() {
        // Addendum constraint 4c (W^X): the loaded code page must be
        // executable AND non-writable — never RWX. Checked against the
        // live mapping via /proc/self/maps.
        let (object_bytes, manifest) =
            crate::aot::emit_object(&call_local_program()).expect("emit");
        let loaded = ElfX86_64Loader
            .load(&object_bytes, &deny_resolver)
            .expect("load");
        let main_symbol = &manifest.function_table[0].symbol;
        let addr = loaded.function_addr(main_symbol).expect("addr") as u64;

        let maps = std::fs::read_to_string("/proc/self/maps").expect("read maps");
        let perms = maps
            .lines()
            .find_map(|line| {
                let (range, rest) = line.split_once(' ')?;
                let (start, end) = range.split_once('-')?;
                let start = u64::from_str_radix(start, 16).ok()?;
                let end = u64::from_str_radix(end, 16).ok()?;
                (start..end)
                    .contains(&addr)
                    .then(|| rest.split(' ').next().unwrap_or("").to_string())
            })
            .expect("mapping line for loaded code");
        assert_eq!(
            &perms[..4],
            "r-xp",
            "loaded code must be r-x (W^X), got {perms}"
        );
    }

    proptest! {
        /// Fuzz the patcher's arithmetic core against an independent
        /// inverse: when a displacement is produced, re-adding it at the
        /// site must reconstruct the target; when `None`, the true
        /// displacement must genuinely overflow `i32` (Addendum
        /// constraint 4b).
        #[test]
        fn pc_relative_disp_is_correct_or_refused(
            target in any::<u32>(),
            site in any::<u32>(),
            addend in -16i64..16,
        ) {
            let (t, s) = (target as usize, site as usize);
            if let Some(disp) = pc_relative_i32(t, s, addend) {
                let recon = s as i128 + i128::from(disp) - i128::from(addend);
                prop_assert_eq!(recon, t as i128);
            } else {
                let true_disp = t as i128 + i128::from(addend) - s as i128;
                prop_assert!(i32::try_from(true_disp).is_err());
            }
        }
    }
}
