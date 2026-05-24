//! v0.7.9.3 — `.khi` round-trip smoke test.
//!
//! Loads `compiler/pack_writer.tri` (which now hosts both the .triv
//! writer from v0.7.9.2 and the .khi wrapper from v0.7.9.3),
//! invokes `khi_smoke_main()` inside the VM, and then asserts
//! that the Triết-emitted .khi bytes round-trip cleanly through
//! Rust `triet_pack::read_khi`.

use std::path::PathBuf;
use std::sync::OnceLock;

use miette::Diagnostic as _;
use triet_ir::{FuncId, IrProgram, RuntimeValue, Vm, lower_program};
use triet_modules::load_program;
use triet_pack::read_khi;
use triet_typecheck::check_resolved;

fn compiler_pack_writer_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("compiler")
        .join("pack_writer.tri")
}

fn pack_writer_ir() -> &'static IrProgram {
    static IR: OnceLock<IrProgram> = OnceLock::new();
    IR.get_or_init(|| {
        let path = compiler_pack_writer_path();
        assert!(
            path.is_file(),
            "missing pack_writer.tri at {}",
            path.display()
        );
        let resolved = load_program(&path).expect("load_program");
        let diagnostics = check_resolved(&resolved);
        let blocking: Vec<_> = diagnostics
            .iter()
            .filter(|err| err.severity() != Some(miette::Severity::Warning))
            .collect();
        assert!(blocking.is_empty(), "type errors: {blocking:#?}");
        lower_program(&resolved)
    })
}

fn lookup_func(ir: &IrProgram, name: &str) -> FuncId {
    ir.modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing function `{name}`"))
        .id
}

#[test]
fn khi_smoke_main_returns_positive_true() {
    let ir = pack_writer_ir().clone();
    let func_id = lookup_func(&ir, "khi_smoke_main");
    let mut vm = Vm::new(ir);
    let result = vm
        .execute(func_id, vec![])
        .expect("khi_smoke_main must execute without VM error");

    match result {
        RuntimeValue::Outcome {
            discriminator,
            payload,
        } => {
            assert_eq!(
                discriminator,
                triet_core::Trit::Positive,
                "expected Positive outcome, got {discriminator:?} payload {payload:?}",
            );
            match payload {
                Some(boxed) => match *boxed {
                    RuntimeValue::Trilean(triet_logic::Trilean::True) => {}
                    other => panic!("expected Trilean::True payload, got {other:?}"),
                },
                None => panic!("Positive outcome with no payload"),
            }
        }
        other => panic!("expected Outcome runtime value, got {other:?}"),
    }
}

/// Invoke `serialize_source_to_khi` for a simple Triết source and
/// confirm the result decodes via Rust `triet_pack::read_khi`. The
/// Rust decoder validates the magic bytes + ABI version + section
/// framing, so a clean round-trip proves the wire format matches.
#[test]
fn rust_read_khi_decodes_triet_emitted_bytes() {
    let ir = pack_writer_ir().clone();
    let func_id = lookup_func(&ir, "serialize_source_to_khi");
    let mut vm = Vm::new(ir);
    let source = "function f() -> Integer = 42";
    let pkg_name = "compiler";
    let result = vm
        .execute(
            func_id,
            vec![
                RuntimeValue::String(source.to_owned()),
                RuntimeValue::String(pkg_name.to_owned()),
            ],
        )
        .expect("serialize_source_to_khi execution");

    let bytes: Vec<u8> = match result {
        RuntimeValue::Vector(vec) => vec
            .iter()
            .map(|v| match v {
                RuntimeValue::Integer(i) => u8::try_from(i.to_i64()).expect("byte out of u8 range"),
                other => panic!("expected Integer in byte vector, got {other:?}"),
            })
            .collect(),
        other => panic!("expected Vector<Integer>, got {other:?}"),
    };

    // First 4 bytes must be "trip" magic.
    assert_eq!(
        &bytes[..4],
        b"trip",
        "Triết khi output must start with `trip` magic"
    );

    // Rust decodes the Triết bytes.
    let (metadata, code_section) =
        read_khi(&bytes).expect("Rust read_khi must accept Triết-emitted .khi");

    assert_eq!(
        metadata.abi_version, 2,
        "abi_version stays at 2 per ADR-0014"
    );
    assert_eq!(metadata.pkg_name, pkg_name);
    assert_eq!(metadata.pkg_version.major, 0);
    assert_eq!(metadata.pkg_version.minor, 0);
    assert_eq!(metadata.pkg_version.patch, 0);
    assert!(
        !metadata.iface_hash.is_zero(),
        "iface_hash should be computed BLAKE3, not zero sentinel"
    );
    assert!(
        !metadata.impl_hash.is_zero(),
        "impl_hash should be computed BLAKE3, not zero sentinel"
    );
    // Code section is the embedded .triv payload — must start with `triv` magic.
    assert_eq!(
        &code_section[..4],
        b"triv",
        "embedded code section should be a complete .triv file"
    );
    // Empty types/exports/deps/caps (v0.7.9.3 minimal) means
    // metadata.modules is empty (no terms → no module groupings).
    assert!(metadata.modules.is_empty());
    assert!(metadata.types.is_empty());
    assert!(metadata.exports.is_empty());
    assert!(metadata.deps.is_empty());
    assert!(metadata.caps.is_empty());
}
