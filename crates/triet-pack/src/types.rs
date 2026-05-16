//! ABI metadata data structures — the in-memory shape of what
//! `.tripack` files carry.
//!
//! Each struct mirrors one table from [ADR-0011 §2–5], extended at v0.5
//! by [ADR-0014] with a 3-cấp hash tree: term → module → package.
//! Field ordering is significant for hash stability (see
//! [`hash::compute_iface_hash`]).
//!
//! [ADR-0011 §2–5]: ../../../docs/decisions/0011-abi-metadata-format.md
//! [ADR-0014]: ../../../docs/decisions/0014-hash-scheme-refinement.md
//! [`hash::compute_iface_hash`]: super::hash::compute_iface_hash

use crate::hash::{
    IfaceHash, ImplHash, ModuleIfaceHash, ModuleImplHash, TermIfaceHash, TermImplHash,
};

/// Semantic version triple (major, minor, patch). ADR-0011 §1 +
/// ADR-0013 §1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SemVer {
    /// Breaking-change version. Bump => `iface_hash` should differ.
    pub major: u32,
    /// Additive-change version. Bump => downstream stays compatible.
    pub minor: u32,
    /// Bug-fix / internal-impl version. `iface_hash` stays identical.
    pub patch: u32,
}

impl SemVer {
    /// Construct from a `(major, minor, patch)` tuple.
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

/// Item visibility inside a package. ADR-0005 (module system) defines
/// three levels; the linker only ever exposes `Public` items, but the
/// other variants are encoded so future tools (docs generator,
/// auto-rename, package diff) can reason about them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Visibility {
    /// `public` — exported across packages.
    Public,
    /// `public(package)` — visible inside the same package.
    Package,
    /// Default — visible only inside the declaring module.
    Private,
}

/// Kind tag for an entry in the type table. Matches ADR-0011 §2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeKind {
    /// Product type — encoded as a [`StructDef`].
    Struct,
    /// Sum type — encoded as an [`EnumDef`].
    Enum,
    /// Generic shell with type parameters but no fields/variants yet.
    /// Used for opaque generic interfaces (rare; reserved for future).
    GenericShell,
}

/// A reference to a type used inside a function signature or a type
/// definition's body. Encoded per ADR-0011 §2 TypeRef discriminants.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TypeRef {
    /// A built-in primitive. The inner byte mirrors `TypeTag` from
    /// the IR crate so the linker doesn't need a separate enum.
    Primitive(u8),
    /// A type defined in *this* package — index into the type table.
    Local(u32),
    /// A type parameter slot — index inside the current scope's
    /// `type_params` list.
    TypeParam(u32),
    /// A type defined in a dependency package. First index is into
    /// the dep table, second is into that package's type table.
    External {
        /// Index into this package's [`AbiMetadata::deps`] table.
        dep_idx: u32,
        /// Index into the dependency's type table.
        type_idx: u32,
    },
    /// `T?` — a nullable wrapper around an inner type.
    Nullable(Box<Self>),
    /// Generic instantiation: `base<args...>` (e.g. `Option<Integer>`).
    Instantiation {
        /// Index into the package's type table for the base generic.
        base: u32,
        /// Concrete type arguments substituted into the generic's
        /// type parameters.
        args: Vec<Self>,
    },
}

/// A field inside a struct definition.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FieldDef {
    /// Field name as it appears in source.
    pub name: String,
    /// Field type.
    pub type_ref: TypeRef,
    /// Field visibility.
    pub visibility: Visibility,
}

/// Struct body — list of named fields. Field order matches source.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StructDef {
    /// Fields in declaration order.
    pub fields: Vec<FieldDef>,
}

/// Enum body — list of variants, each optionally carrying a payload.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EnumDef {
    /// Variants in declaration order. Per ADR-0010, the variant
    /// discriminator encodes naturally onto a balanced trit for the
    /// 2- and 3-variant cases.
    pub variants: Vec<EnumVariant>,
}

/// A single enum variant.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EnumVariant {
    /// Variant name.
    pub name: String,
    /// `Some(t)` for tuple-style variants (`Some(Integer)`); `None`
    /// for unit variants (`None`).
    pub payload: Option<TypeRef>,
}

/// Top-level type definition entry — struct, enum, or generic shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeDef {
    /// Kind tag.
    pub kind: TypeKind,
    /// Type name as it appears in source.
    pub name: String,
    /// Dotted module path this type belongs to (e.g. `"foo.core"`).
    /// Empty string means "the package's root module" — given path
    /// `pkg_name` at hash time. ADR-0014 §3 uses this to group terms
    /// into modules for rollup.
    pub module_path: String,
    /// Generic type parameters, in declaration order. Empty for
    /// non-generic types.
    pub type_params: Vec<String>,
    /// Struct body, if `kind == Struct`.
    pub struct_body: Option<StructDef>,
    /// Enum body, if `kind == Enum`.
    pub enum_body: Option<EnumDef>,
    /// ADR-0014 §2 term iface hash. Populated by `write_tripack`
    /// before serialization; left zero in user-built metadata.
    pub iface_hash_term: TermIfaceHash,
    /// ADR-0014 §2 term impl hash. v0.5.3 computes this with empty
    /// body bytes; v0.5.4 wires real per-term IR bodies.
    pub impl_hash_term: TermImplHash,
}

/// A function parameter.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Param {
    /// Parameter name (kept for human-readable diagnostics; not used
    /// in dispatch).
    pub name: String,
    /// Parameter type.
    pub type_ref: TypeRef,
}

/// A function exported by this package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionExport {
    /// Function name as it appears in source.
    pub name: String,
    /// Dotted module path (see [`TypeDef::module_path`]).
    pub module_path: String,
    /// Always `Public` today (only public items are exported), but
    /// the slot lets future tools encode package-visible items too.
    pub visibility: Visibility,
    /// Generic type parameters, in declaration order. Empty for
    /// non-generic functions.
    pub type_params: Vec<String>,
    /// Positional parameters.
    pub params: Vec<Param>,
    /// Return type. Use a primitive `Unit` ref when there's no return.
    pub return_type: TypeRef,
    /// Offset (bytes) into the package's IR code section where this
    /// function's body lives. `0` means "no body" (abstract; reserved
    /// for future).
    pub body_offset: u32,
    /// ADR-0014 §2 term iface hash. See [`TypeDef::iface_hash_term`].
    pub iface_hash_term: TermIfaceHash,
    /// ADR-0014 §2 term impl hash. See [`TypeDef::impl_hash_term`].
    pub impl_hash_term: TermImplHash,
}

/// A declared dependency on another package.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Dep {
    /// Dependency package name.
    pub pkg_name: String,
    /// Lower bound (inclusive) on accepted versions.
    pub version_min: SemVer,
    /// Upper bound (exclusive). `SemVer::default()` means open-ended.
    pub version_max_exclusive: SemVer,
    /// Optional `iface_hash` pin. All-zeros means "no pin" (match
    /// any version in the range). Non-zero means strict — linker
    /// refuses if the actual `iface_hash` differs.
    pub iface_hash_pin: IfaceHash,
}

/// A module entry — one logical namespace inside the package. Hash
/// fields are populated by `write_tripack`'s canonical pass from the
/// terms (types + exports) sharing the same `module_path`.
///
/// ADR-0014 §3.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    /// Dotted module path (e.g. `"foo.core"`, or `pkg_name` for the
    /// root module).
    pub path: String,
    /// Rollup of term iface hashes belonging to this module.
    pub iface_hash_mod: ModuleIfaceHash,
    /// Rollup of term impl hashes. v0.5.3 placeholder; v0.5.4 carries
    /// real per-term body bytes.
    pub impl_hash_mod: ModuleImplHash,
}

/// The full ABI metadata for one `.tripack`. This is what the linker
/// loads to decide refuse-to-link, before touching code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbiMetadata {
    /// Format-version field. v0.5 ships with `abi_version = 2` —
    /// extends v0.4's `abi_version = 1` additively with the term +
    /// module hash fields and the modules table. Per ADR-0014 §5 the
    /// reader refuses `1` (no shim).
    pub abi_version: u32,
    /// Package name as declared by the author (e.g. `"std"`,
    /// `"user.app"`).
    pub pkg_name: String,
    /// Package version triple.
    pub pkg_version: SemVer,
    /// BLAKE3 hash over the canonical ABI surface — at v0.5 rolled up
    /// from module iface hashes (ADR-0014 §4). Stable across re-builds
    /// when the surface didn't change. Linker compares this with dep
    /// pin (ADR-0013 §4).
    pub iface_hash: IfaceHash,
    /// BLAKE3 hash over `iface_hash` + IR code bytes. v0.5.3 keeps the
    /// v0.4 formula until `.triv` v4 lands per-term bodies.
    pub impl_hash: ImplHash,
    /// Modules in this package. Populated automatically by
    /// `write_tripack`'s canonical pass from the unique
    /// `module_path` values across `types` + `exports`.
    pub modules: Vec<Module>,
    /// User-defined types referenced by exports.
    pub types: Vec<TypeDef>,
    /// Functions this package exposes to others.
    pub exports: Vec<FunctionExport>,
    /// Declared dependencies — packages this one will look up at
    /// link time.
    pub deps: Vec<Dep>,
    /// Capability claims (ADR-0011 §5 slot, populated v0.6 per
    /// ADR-0016 §4 + ADR-0018 §6). Empty for leaf libs that need
    /// no cross-root caps; non-empty entries serialize sorted by
    /// `cap_path` (ADR-0016 §4 canonical rule).
    pub caps: Vec<CapabilityClaim>,
}

/// One capability claim — *"this package needs to access `cap_path`
/// at `level`"*. Locked in ADR-0018 §6 (rename from v0.5 `Capability`
/// placeholder). Path stored as dotted-`String` matching pack-level
/// convention (`module_path`, `pkg_name` — see crate docs); structural
/// validation (root ∈ {sys, dev, usr}, well-formed dot path) lives at
/// the manifest parser boundary (v0.6.5+).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CapabilityClaim {
    /// Module-level `AbsolutePath` (ADR-0005) as dotted string, e.g.
    /// `"sys.io"`, `"dev.disk"`. Encoded as length-prefixed UTF-8 in
    /// the caps section.
    pub cap_path: String,
    /// Static level — Trit-valued grant/ambient/deny plus the
    /// Trilean::Unknown `Defer` slot (ADR-0016 §3).
    pub level: CapabilityLevel,
}

/// Four-state capability level (ADR-0016 §3). Three Trit values
/// (Grant/Ambient/Deny) plus the `Defer` slot encoding
/// `Trilean::Unknown` — the case where the static manifest defers
/// the decision to a runtime policy hook (ADR-0017).
///
/// Wire encoding (ADR-0016 §4): single byte, values `0x00..=0x03`.
/// Anything outside that range is rejected at deserialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CapabilityLevel {
    /// Explicit refuse — caller cannot reach `cap_path` (`Trit::Negative`).
    Deny,
    /// "Inherit from caller" — at the root package this collapses to
    /// `Deny` (no caller above). Non-root: linker overrides per root
    /// manifest authority (ADR-0016 §7).
    Ambient,
    /// Explicit allow — caller may reach `cap_path` (`Trit::Positive`).
    Grant,
    /// `Trilean::Unknown` — defer to runtime policy hook (ADR-0017 §4).
    Defer,
}

impl CapabilityLevel {
    /// Wire byte (ADR-0016 §4).
    #[must_use]
    pub const fn as_byte(self) -> u8 {
        match self {
            Self::Deny => 0x00,
            Self::Ambient => 0x01,
            Self::Grant => 0x02,
            Self::Defer => 0x03,
        }
    }

    /// Parse the wire byte, returning `None` for any value outside
    /// the four locked encodings. Caller maps `None` to a structural
    /// error (currently [`PackError::Corrupted`](crate::PackError);
    /// dedicated `E2207 InvalidCapabilityLevel` lands with the
    /// manifest parser in v0.6.5+).
    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(Self::Deny),
            0x01 => Some(Self::Ambient),
            0x02 => Some(Self::Grant),
            0x03 => Some(Self::Defer),
            _ => None,
        }
    }
}

impl AbiMetadata {
    /// Build an empty metadata block — useful as a starting point
    /// when writing a new package builder.
    #[must_use]
    pub fn empty(pkg_name: impl Into<String>, pkg_version: SemVer) -> Self {
        Self {
            abi_version: 2,
            pkg_name: pkg_name.into(),
            pkg_version,
            iface_hash: IfaceHash::default(),
            impl_hash: ImplHash::default(),
            modules: Vec::new(),
            types: Vec::new(),
            exports: Vec::new(),
            deps: Vec::new(),
            caps: Vec::new(),
        }
    }
}
