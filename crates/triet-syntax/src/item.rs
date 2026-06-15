//! Auxiliary item-AST types not generated from the schema.
//!
//! `Item`, `Program`, the `*Def` structs, `ModuleItem`/`ModuleContent`, etc. are
//! schema-generated (`crate::generated`). What remains here are the small helper
//! types the generated definitions reference through `crate::item::…` paths.

use crate::arena::TypeId;

/// A bound on a generic parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GenericBound {
    /// `: Send` bound, requiring the type to be safe to send across threads.
    Send,
}

/// A generic parameter, e.g. `T` or `F: Send`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeParameter {
    /// The name of the parameter, e.g. `T`.
    pub name: String,
    /// An optional bound, e.g. `Send`.
    pub bound: Option<GenericBound>,
}

/// A single field in a struct definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructField {
    /// Field name.
    pub name: String,
    /// Field type annotation.
    pub type_annotation: TypeId,
}

/// A single variant in an enum definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumVariant {
    /// Variant name.
    pub name: String,
    /// Optional payload type. `None` = unit variant (`None`),
    /// `Some(TypeId)` = tuple variant (`Some(Integer)`).
    pub payload: Option<TypeId>,
}

/// A dotted import path: `import std.io.println` → `["std", "io", "println"]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportPath {
    /// Dot-separated segments, in order.
    pub segments: Vec<String>,
}

/// A single name in a `from … import …` name list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportName {
    /// Imported symbol name (as it appears in the source module).
    pub name: String,
    /// Optional alias introduced by `as` — when present, the binding
    /// in the importing module uses this name instead of `name`.
    pub alias: Option<String>,
}
