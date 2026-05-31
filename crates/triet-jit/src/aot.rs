//! v0.11.x.jit.3 (ADR-0033) — AOT cache: object emission + manifest.
//!
//! **Step 1 scope (this commit):** emit a relocatable ELF `.o` from a
//! Triết IR program via `cranelift-object`, reusing the **shared**
//! translator [`crate::codegen::declare_and_define_program`] (one
//! codegen pipeline, two emission targets per [ADR-0033 §1]), plus the
//! [`AotCacheManifest`] that records the version pins (§2) + symbol
//! table the Path-A loader will need. **No loading yet** — the
//! relocating loader + `Store` integration land in Steps 2–4.
//!
//! The manifest framing is a small hand-rolled binary format (LE
//! integers + length-prefixed UTF-8) in the spirit of the project's
//! other binary formats, but deliberately simpler than the `.khi`
//! canonical encoding: the AOT cache is internal + best-effort
//! (invalidated on any version-pin mismatch per §2), so it needs only
//! deterministic round-trip, not canonical-hash stability.
//!
//! [ADR-0033 §1]: ../../../docs/decisions/0033-aot-cache-cranelift-object.md
//! [ADR-0033 §2]: ../../../docs/decisions/0033-aot-cache-cranelift-object.md

#![allow(clippy::redundant_pub_crate)]
// Step 1 lands object emission + the manifest type; the dispatcher
// wiring that *consumes* them (Path-A load + cache write) lands in
// Steps 2–4. Until then these items are exercised only by tests, so
// the lib build sees them as unused. Removed once Step 4 wires them.
#![allow(dead_code)]

use cranelift_module::default_libcall_names;
use cranelift_object::{ObjectBuilder, ObjectModule};
use triet_ir::IrProgram;

use crate::codegen::{build_host_isa, declare_and_define_module};
use crate::{JitError, SHIM_ABI_VERSION};

/// One entry in the manifest function table: a compiled Triết function
/// paired with the symbol name under which it lives in the emitted
/// `.o`. The Path-A loader (Step 3) resolves the symbol to a section
/// offset to recover the function's load address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FunctionEntry {
    /// Triết `FuncId` raw value (`triet_ir::FuncId.0`).
    pub func_id: u32,
    /// Mangled symbol name in the object's symbol table.
    pub symbol: String,
}

/// AOT cache manifest per [ADR-0033 §2]. Stored beside `functions.o`.
/// The Path-A loader checks the three version pins (exact-match;
/// mismatch → silent fallback per §8) before attempting any
/// relocation, then uses `function_table` to map each Triết `FuncId`
/// to its symbol in the loaded object.
///
/// [ADR-0033 §2]: ../../../docs/decisions/0033-aot-cache-cranelift-object.md
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AotCacheManifest {
    /// `cranelift_codegen::VERSION` at write time. Cache invalidates on
    /// any Cranelift bump (codegen / reloc shape may change).
    pub cranelift_version: String,
    /// [`crate::SHIM_ABI_VERSION`] at write time (§2). Bump on any
    /// ADR-0032 shim ABI break.
    pub shim_abi_version: u32,
    /// Host target triple at write time. Redundant with the per-triple
    /// path (§5) but cross-checked defensively (§2).
    pub target_triple: String,
    /// Compiled functions, sorted by `func_id` for deterministic bytes.
    pub function_table: Vec<FunctionEntry>,
    /// Cranelift internal libcalls (`__truncdfsf2` etc.) the object
    /// references (§3). Empty at v0.10/v0.11 — Triết IR uses no
    /// floating point, so no libcalls are emitted yet.
    pub libcall_set: Vec<String>,
}

/// Magic bytes for the manifest framing — "TAOT" (Triết AOT).
const MANIFEST_MAGIC: [u8; 4] = *b"TAOT";

/// Manifest framing version (the *container*, independent of the
/// Cranelift / shim version pins inside). Bump when the field layout
/// below changes.
const MANIFEST_FORMAT_VERSION: u32 = 1;

impl AotCacheManifest {
    /// Serialize to the hand-rolled binary framing. Infallible: all
    /// length fields fit in `u32` (symbol names, version strings, and
    /// table sizes are all far below `u32::MAX` in practice).
    pub(crate) fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MANIFEST_MAGIC);
        put_u32(&mut buf, MANIFEST_FORMAT_VERSION);
        put_str(&mut buf, &self.cranelift_version);
        put_u32(&mut buf, self.shim_abi_version);
        put_str(&mut buf, &self.target_triple);
        put_len(&mut buf, self.function_table.len());
        for entry in &self.function_table {
            put_u32(&mut buf, entry.func_id);
            put_str(&mut buf, &entry.symbol);
        }
        put_len(&mut buf, self.libcall_set.len());
        for name in &self.libcall_set {
            put_str(&mut buf, name);
        }
        buf
    }

    /// Parse the framing. Any malformed input — bad magic, unknown
    /// format version, truncation, invalid UTF-8 — yields
    /// [`JitError::Cache`] so the loader treats it as a miss (§8).
    pub(crate) fn deserialize(bytes: &[u8]) -> Result<Self, JitError> {
        let mut reader = Reader::new(bytes);
        if reader.take(4)? != MANIFEST_MAGIC {
            return Err(cache_err("manifest: bad magic"));
        }
        let format = reader.u32()?;
        if format != MANIFEST_FORMAT_VERSION {
            return Err(cache_err(format!(
                "manifest: unknown format version {format}"
            )));
        }
        let cranelift_version = reader.string()?;
        let shim_abi_version = reader.u32()?;
        let target_triple = reader.string()?;
        // NOTE: counts come from untrusted bytes → never pre-allocate
        // with `with_capacity(count)` (a corrupt huge count would OOM).
        // Push-as-we-go; `reader` errors out fast on truncation.
        let func_count = reader.u32()?;
        let mut function_table = Vec::new();
        for _ in 0..func_count {
            let func_id = reader.u32()?;
            let symbol = reader.string()?;
            function_table.push(FunctionEntry { func_id, symbol });
        }
        let libcall_count = reader.u32()?;
        let mut libcall_set = Vec::new();
        for _ in 0..libcall_count {
            libcall_set.push(reader.string()?);
        }
        if !reader.is_at_end() {
            return Err(cache_err("manifest: trailing bytes"));
        }
        Ok(Self {
            cranelift_version,
            shim_abi_version,
            target_triple,
            function_table,
            libcall_set,
        })
    }
}

/// Emit a relocatable ELF `.o` for a **single module**
/// (`program.modules[local_idx]`) plus its [`AotCacheManifest`], using
/// the shared translator against a `cranelift-object` backend (Path A
/// persist per [ADR-0033 §1] + v0.11.0.2 per-module granularity).
///
/// One object per module so each is cacheable + GC'd by its own
/// `impl_hash_mod`. Cross-module calls become external symbols the
/// load-time linker resolves (see [`declare_and_define_module`]). The
/// ISA is built PIC (`build_host_isa(true)`) so the object uses
/// PC-relative relocations — loadable at an arbitrary `mmap` address.
/// Functions that tier down during translation (per ADR-0030 §2) are
/// absent from both the object and the manifest's function table.
///
/// # Errors
///
/// [`JitError::Cranelift`] if ISA construction, the `ObjectBuilder`,
/// or final object emission fails; [`JitError::UnsupportedOpcode`] if
/// `local_idx` is out of range. (Object *emission* faults are
/// `Cranelift`, not [`JitError::Cache`] — the latter is the Path-A
/// *load* side per §8.)
///
/// [ADR-0033 §1]: ../../../docs/decisions/0033-aot-cache-cranelift-object.md
pub(crate) fn emit_module_object(
    program: &IrProgram,
    local_idx: usize,
) -> Result<(Vec<u8>, AotCacheManifest), JitError> {
    let isa = build_host_isa(true)?;
    let target_triple = isa.triple().to_string();
    let builder = ObjectBuilder::new(isa, "triet_aot", default_libcall_names()).map_err(|err| {
        JitError::Cranelift {
            message: format!("ObjectBuilder init: {err}"),
        }
    })?;
    let mut module = ObjectModule::new(builder);
    let translated = declare_and_define_module(&mut module, program, local_idx, &[])?;

    let mut function_table: Vec<FunctionEntry> = translated
        .compiled
        .iter()
        .map(|id| FunctionEntry {
            func_id: id.0,
            symbol: translated.symbol_names[id].clone(),
        })
        .collect();
    function_table.sort_by_key(|entry| entry.func_id);

    let product = module.finish();
    let object_bytes = product.emit().map_err(|err| JitError::Cranelift {
        message: format!("object emit: {err}"),
    })?;

    let manifest = AotCacheManifest {
        cranelift_version: cranelift_codegen::VERSION.to_string(),
        shim_abi_version: SHIM_ABI_VERSION,
        target_triple,
        function_table,
        libcall_set: Vec::new(),
    };
    Ok((object_bytes, manifest))
}

// ── Hand-rolled framing helpers ─────────────────────────────────────

fn put_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

/// Write a collection length as `u32`. Lengths in this format (table
/// sizes) are bounded well below `u32::MAX`; the saturating cast keeps
/// `serialize` infallible without a panic path.
#[allow(clippy::cast_possible_truncation)]
fn put_len(buf: &mut Vec<u8>, len: usize) {
    put_u32(buf, u32::try_from(len).unwrap_or(u32::MAX));
}

/// Write a length-prefixed UTF-8 string (`u32` byte length + bytes).
fn put_str(buf: &mut Vec<u8>, value: &str) {
    put_len(buf, value.len());
    buf.extend_from_slice(value.as_bytes());
}

/// Cursor over the manifest bytes with bounds-checked reads.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], JitError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| cache_err("manifest: length overflow"))?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| cache_err("manifest: unexpected end"))?;
        self.pos = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32, JitError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn string(&mut self) -> Result<String, JitError> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|err| cache_err(format!("manifest: invalid utf-8: {err}")))
    }

    const fn is_at_end(&self) -> bool {
        self.pos == self.bytes.len()
    }
}

fn cache_err(reason: impl Into<String>) -> JitError {
    JitError::Cache {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use object::{Object, ObjectSection, ObjectSymbol, RelocationFlags, RelocationTarget};
    use triet_ir::{
        BasicBlock, BlockId, BuiltinName, ConstantPool, FuncId, Function, Instruction, IrModule,
        Operand, TypeTag, ValueId,
    };
    use triet_modules::{AbsolutePath, ModulePath};

    use super::*;

    fn make_function(
        id: FuncId,
        name: &str,
        params: Vec<(String, TypeTag)>,
        return_type: TypeTag,
        instructions: Vec<Instruction>,
    ) -> Function {
        let mut block = BasicBlock::new(BlockId(0), Some("entry".to_string()));
        block.instructions = instructions;
        let mut func = Function::new(id, Some(name.to_string()), params, return_type);
        func.blocks = vec![block];
        func
    }

    /// `helper() -> Integer = 7`, `main() -> Integer = helper()`.
    /// The `CallLocal` exercises an intra-object function reference —
    /// the relocation kind the Path-A loader must handle for self-host
    /// (most calls are local).
    fn call_local_program() -> IrProgram {
        let mut pool = ConstantPool::new();
        let seven = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(7).unwrap(),
        ));
        let helper = make_function(
            FuncId(0),
            "helper",
            vec![],
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
        let main = make_function(
            FuncId(1),
            "main",
            vec![],
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
        let path = AbsolutePath::new(ModulePath::new(vec!["khi".to_string()]), String::new());
        IrProgram {
            modules: vec![IrModule {
                path,
                functions: vec![helper, main],
            }],
            constants: pool,
            witness_tables: Vec::new(),
        }
    }

    /// `to_text(n) -> String = TextFromInteger(n)` — a single-block
    /// function that calls the `__triet_text_from_integer` shim
    /// (signature `(I64) -> Ptr`). Exercises the **external** symbol
    /// case: the shim is undefined in the `.o`, so the Path-A loader
    /// (Step 3) resolves it via `SHIM_TABLE` rather than an in-object
    /// offset. Also pulls in `__triet_shim_failed` via the per-call
    /// sentinel (ADR-0032 §4 option-2).
    fn shim_call_program() -> IrProgram {
        let mut pool = ConstantPool::new();
        let n = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(123).unwrap(),
        ));
        let to_text = make_function(
            FuncId(0),
            "to_text",
            vec![],
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
        let path = AbsolutePath::new(ModulePath::new(vec!["khi".to_string()]), String::new());
        IrProgram {
            modules: vec![IrModule {
                path,
                functions: vec![to_text],
            }],
            constants: pool,
            witness_tables: Vec::new(),
        }
    }

    /// ADR-0033 Addendum safety constraint 3: the relocation set is
    /// verified *empirically against the actual emitter*, not assumed.
    /// Step 3's patcher handles exactly this allowlist and refuses
    /// anything outside it; this trips if a future Cranelift bump
    /// introduces a new relocation type. Allowlist verified for
    /// cranelift-object 0.132 + PIC `x86_64` (`elf::R_X86_64_*` codes).
    ///
    /// Returns `(r_type, target_symbol_name, target_is_undefined)` for
    /// every relocation, after asserting each `r_type` is allowed.
    fn collect_bounded_relocations(object_bytes: &[u8]) -> Vec<(u32, String, bool)> {
        const ALLOWED_R_X86_64: &[u32] = &[
            object::elf::R_X86_64_PC32,          // 2  PC-relative 32 (calls/data)
            object::elf::R_X86_64_PLT32,         // 4  PLT-relative 32 (calls)
            object::elf::R_X86_64_64,            // 1  absolute 64 (data refs)
            object::elf::R_X86_64_GOTPCREL,      // 9  GOT-relative (extern data)
            object::elf::R_X86_64_GOTPCRELX,     // 41
            object::elf::R_X86_64_REX_GOTPCRELX, // 42
        ];
        let file = object::File::parse(object_bytes).expect("parse ELF .o");
        let mut seen = Vec::new();
        for section in file.sections() {
            for (_off, reloc) in section.relocations() {
                let r_type = match reloc.flags() {
                    RelocationFlags::Elf { r_type } => r_type,
                    other => panic!("non-ELF relocation flags: {other:?}"),
                };
                let (name, undefined) = match reloc.target() {
                    RelocationTarget::Symbol(idx) => file.symbol_by_index(idx).map_or_else(
                        |_| (format!("<sym#{}>", idx.0), false),
                        |s| (s.name().unwrap_or("<noname>").to_string(), s.is_undefined()),
                    ),
                    other => (format!("{other:?}"), false),
                };
                assert!(
                    ALLOWED_R_X86_64.contains(&r_type),
                    "unexpected relocation type {r_type} (target {name}) — Step 3 patcher \
                     must be extended + verified before this is allowed"
                );
                seen.push((r_type, name, undefined));
            }
        }
        seen
    }

    #[test]
    fn manifest_round_trips() {
        let manifest = AotCacheManifest {
            cranelift_version: "0.132.0".to_string(),
            shim_abi_version: SHIM_ABI_VERSION,
            target_triple: "x86_64-unknown-linux-gnu".to_string(),
            function_table: vec![
                FunctionEntry {
                    func_id: 0,
                    symbol: "helper__f0".to_string(),
                },
                FunctionEntry {
                    func_id: 1,
                    symbol: "main__f1".to_string(),
                },
            ],
            libcall_set: vec![],
        };
        let bytes = manifest.serialize();
        let parsed = AotCacheManifest::deserialize(&bytes).expect("round-trip");
        assert_eq!(manifest, parsed);
    }

    #[test]
    fn manifest_rejects_corruption() {
        let manifest = AotCacheManifest {
            cranelift_version: "0.132.0".to_string(),
            shim_abi_version: 1,
            target_triple: "x86_64-unknown-linux-gnu".to_string(),
            function_table: vec![],
            libcall_set: vec![],
        };
        let mut bytes = manifest.serialize();
        // Truncated → unexpected end.
        assert!(AotCacheManifest::deserialize(&bytes[..bytes.len() - 3]).is_err());
        // Bad magic.
        let mut bad_magic = bytes.clone();
        bad_magic[0] ^= 0xFF;
        assert!(AotCacheManifest::deserialize(&bad_magic).is_err());
        // Trailing byte → rejected.
        bytes.push(0);
        assert!(AotCacheManifest::deserialize(&bytes).is_err());
    }

    #[test]
    fn emit_object_produces_parseable_elf_with_bounded_relocations() {
        let program = call_local_program();
        let (object_bytes, manifest) = emit_module_object(&program, 0).expect("emit");

        // Manifest reflects both compiled functions.
        assert_eq!(manifest.function_table.len(), 2);
        assert_eq!(manifest.shim_abi_version, SHIM_ABI_VERSION);
        assert_eq!(manifest.cranelift_version, cranelift_codegen::VERSION);
        // Manifest itself round-trips after a real emission.
        let reparsed = AotCacheManifest::deserialize(&manifest.serialize()).expect("round-trip");
        assert_eq!(manifest, reparsed);

        // The emitted bytes parse as an object file.
        let file = object::File::parse(&object_bytes[..]).expect("parse ELF .o");

        // Every manifest symbol is present + defined in the object.
        let defined: std::collections::HashSet<String> = file
            .symbols()
            .filter(object::ObjectSymbol::is_definition)
            .filter_map(|s| s.name().ok().map(str::to_string))
            .collect();
        for entry in &manifest.function_table {
            assert!(
                defined.contains(&entry.symbol),
                "manifest symbol {} missing from object (defined: {defined:?})",
                entry.symbol
            );
        }

        // Enumerate every relocation's raw ELF type + target symbol,
        // asserting each is within the empirically-verified allowlist.
        let seen = collect_bounded_relocations(&object_bytes);
        eprintln!("CallLocal relocations (r_type, target, undefined): {seen:?}");
        // The CallLocal must have produced ≥1 relocation referencing
        // the (defined, in-object) helper symbol.
        assert!(
            seen.iter()
                .any(|(_, name, undef)| name == "helper__f0" && !*undef),
            "expected a defined-symbol relocation to helper__f0, got {seen:?}"
        );
    }

    #[test]
    fn emit_object_external_shim_relocations_are_undefined_symbols() {
        // The external-symbol case: shim references resolve via
        // SHIM_TABLE at load (Step 3), not an in-object offset. Verify
        // they appear as *undefined* symbols + bounded relocation type.
        let (object_bytes, manifest) = emit_module_object(&shim_call_program(), 0).expect("emit");
        assert_eq!(manifest.function_table.len(), 1);

        let seen = collect_bounded_relocations(&object_bytes);
        eprintln!("shim-call relocations (r_type, target, undefined): {seen:?}");
        for shim in ["__triet_text_from_integer", "__triet_shim_failed"] {
            assert!(
                seen.iter().any(|(_, name, undef)| name == shim && *undef),
                "expected an undefined-symbol relocation to {shim}, got {seen:?}"
            );
        }
    }
}
