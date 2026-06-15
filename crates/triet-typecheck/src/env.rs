//! Type environment — scoped variable bindings + a built-in prelude.

use std::collections::HashMap;

use crate::types::Type;
use triet_syntax::ReferenceForm;

/// A stack of name → binding frames. Entering a block / function pushes
/// a frame; leaving pops it. Lookup walks from innermost to outermost.
#[derive(Clone, Debug)]
pub struct TypeEnvironment {
    /// The frame stack. Accessible to the checker for enum-variant
    /// scanning; prefer `lookup()` / `declare()` for normal use.
    pub(crate) frames: Vec<Frame>,
    /// Overloaded function signatures per frame. Used when a single name
    /// (e.g. `len`) has multiple type signatures depending on argument
    /// types. Each frame holds a map from name → candidate types.
    /// Lookups here are separate from `names` — a name can be in one,
    /// both, or neither.
    pub(crate) overloads: Vec<HashMap<String, Vec<Type>>>,
}

impl Default for TypeEnvironment {
    fn default() -> Self {
        Self {
            frames: vec![Frame::default()],
            overloads: vec![HashMap::new()],
        }
    }
}

/// A type-level binding: the type plus whether reassignment is allowed.
#[derive(Clone, Debug)]
pub struct Binding {
    /// Bound type.
    pub ty: Type,
    /// `true` for `let mut`, `false` for `let`/`const`/function/pattern.
    pub mutable: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Frame {
    /// Bindings in this frame.
    pub names: HashMap<String, Binding>,
}

impl TypeEnvironment {
    /// Construct a fresh, empty environment with one root frame and the
    /// Triết prelude (`print`, `println`, `to_string`, ...) pre-bound.
    #[must_use]
    pub fn with_prelude() -> Self {
        let mut env = Self::default();
        bind_prelude(&mut env);
        env
    }

    /// Push a new (empty) frame onto the stack.
    pub fn push_frame(&mut self) {
        self.frames.push(Frame::default());
        self.overloads.push(HashMap::new());
    }

    /// Pop the top frame. Panics if only the root frame remains.
    pub fn pop_frame(&mut self) {
        assert!(
            self.frames.len() > 1,
            "cannot pop the root environment frame",
        );
        self.frames.pop();
        self.overloads.pop();
    }

    /// Bind `name` to `ty` in the current top frame as immutable. Returns
    /// `true` if the name was newly inserted, `false` if it shadowed an
    /// existing binding in the same frame.
    pub fn declare(&mut self, name: &str, ty: Type) -> bool {
        self.declare_with_mut(name, ty, false)
    }

    /// Bind `name` to `ty` with explicit mutability.
    pub fn declare_with_mut(&mut self, name: &str, ty: Type, mutable: bool) -> bool {
        let top = self.frames.last_mut().expect("at least one frame");
        let was_absent = !top.names.contains_key(name);
        top.names.insert(name.to_owned(), Binding { ty, mutable });
        was_absent
    }

    /// Look up `name`, walking frames from innermost out. Returns the
    /// bound type, or `None` if not found.
    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<&Type> {
        self.lookup_binding(name).map(|b| &b.ty)
    }

    /// Look up the full binding (type + mutability) for `name`.
    #[must_use]
    pub fn lookup_binding(&self, name: &str) -> Option<&Binding> {
        for frame in self.frames.iter().rev() {
            if let Some(binding) = frame.names.get(name) {
                return Some(binding);
            }
        }
        None
    }

    /// Register an overloaded function signature for `name` in the
    /// current top frame. If the name already has overloads in this
    /// frame, the new type is appended.
    pub fn declare_overload(&mut self, name: &str, ty: Type) {
        let top = self
            .overloads
            .last_mut()
            .expect("at least one overload frame");
        top.entry(name.to_owned()).or_default().push(ty);
    }

    /// Look up all overloaded function signatures for `name`, walking
    /// frames from innermost out. Returns owned `Type` values to avoid
    /// borrowing `self` across a mutable call in `check_call`.
    /// Returns `None` if no overloads exist for this name.
    #[must_use]
    pub fn lookup_all(&self, name: &str) -> Option<Vec<Type>> {
        for frame in self.overloads.iter().rev() {
            if let Some(types) = frame.get(name) {
                return Some(types.clone());
            }
        }
        None
    }
}

/// Populate the root frame with built-in functions used by the v0.1
/// demo programs (`print`, `println`, `to_string`, etc.). The prelude
/// is intentionally minimal — extending it lives alongside library
/// growth, not the type-checker core.
#[allow(clippy::too_many_lines)]
fn bind_prelude(env: &mut TypeEnvironment) {
    use Type::{Integer, Long, String, Tryte, Unit, Vector};
    // Trilean is now a struct variant — use the const helpers (ADR-0021).
    let trilean = Type::TRILEAN;
    let vector_integer = Vector(Box::new(Integer.clone()));

    env.declare(
        "print",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![String.clone()],
            return_type: Box::new(Unit.clone()),
        },
    );
    env.declare(
        "println",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![String.clone()],
            return_type: Box::new(Unit.clone()),
        },
    );
    env.declare(
        "read_line",
        Type::Function {
            type_params: Vec::new(),
            parameters: Vec::new(),
            return_type: Box::new(String.clone()),
        },
    );

    // `to_string` accepts any of the four numeric types and Trilean.
    // V0.1 has no overload resolution, so we expose one variant per
    // input type with a name suffix; the AI-friendly path. Plus a
    // generic `to_string` that accepts Integer (default).
    env.declare(
        "to_string",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![Integer.clone()],
            return_type: Box::new(String.clone()),
        },
    );
    env.declare(
        "tryte_to_string",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![Tryte.clone()],
            return_type: Box::new(String.clone()),
        },
    );
    env.declare(
        "long_to_string",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![Long.clone()],
            return_type: Box::new(String.clone()),
        },
    );
    env.declare(
        "trilean_to_string",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![trilean],
            return_type: Box::new(String.clone()),
        },
    );

    // `length` is overloaded: works on String (owned) and
    // &0 String (shared borrow, ADR-0045 §8). Registered via
    // declare_overload so check_call tries each candidate.
    env.declare_overload(
        "length",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![String.clone()],
            return_type: Box::new(Integer),
        },
    );
    env.declare_overload(
        "length",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![Type::Reference(
                ReferenceForm::BorrowReadOnly,
                Box::new(String.clone()),
            )],
            return_type: Box::new(Integer),
        },
    );

    // ── Phase 4.3c: String builtins (deferred from 4.3a) ──

    // `concat(String, String) -> String` — borrow semantics (arg_consumes = [false, false]).
    env.declare(
        "concat",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![String.clone(), String.clone()],
            return_type: Box::new(String.clone()),
        },
    );

    // `eq(String, String) -> Integer` — returns 1 (true) or 0 (false).
    env.declare(
        "eq",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![String.clone(), String.clone()],
            return_type: Box::new(Integer.clone()),
        },
    );

    // ── Phase 4.3b: Vector builtins (ADR-0040 §3.1) ──

    // `vector_new() -> Vector<Integer>` — heap-allocate an empty vector.
    env.declare(
        "vector_new",
        Type::Function {
            type_params: Vec::new(),
            parameters: Vec::new(),
            return_type: Box::new(vector_integer.clone()),
        },
    );

    // `push(Vector<Integer>, Integer) -> Vector<Integer>` — consume vec, append elem.
    env.declare(
        "push",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![vector_integer.clone(), Integer.clone()],
            return_type: Box::new(vector_integer.clone()),
        },
    );

    // `len` is overloaded: works on String and Vector<Integer>.
    // Registered via overloads so check_call can try each candidate.
    env.declare_overload(
        "len",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![String.clone()],
            return_type: Box::new(Integer.clone()),
        },
    );
    env.declare_overload(
        "len",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![vector_integer.clone()],
            return_type: Box::new(Integer),
        },
    );

    // ── ADR-0041 Bước 4: get(Vector<Integer>, Integer) -> Integer? ──
    // Total function: bounds-check returns null (NULL_SENTINEL), never panics.
    env.declare_overload(
        "get",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![vector_integer.clone(), Integer],
            return_type: Box::new(Type::Nullable(Box::new(Integer))),
        },
    );

    // ── ADR-0043: HashMap builtins ──
    let hashmap_ii = Type::UserStruct {
        name: "HashMap".into(),
        type_params: Vec::new(),
        fields: vec![
            ("__key".into(), Integer.clone()),
            ("__value".into(), Integer.clone()),
        ],
    };

    // `hashmap_new() -> HashMap<Integer,Integer>`
    env.declare(
        "hashmap_new",
        Type::Function {
            type_params: Vec::new(),
            parameters: Vec::new(),
            return_type: Box::new(hashmap_ii.clone()),
        },
    );

    // `insert(HashMap<Integer,Integer>, Integer, Integer) -> HashMap<Integer,Integer>`
    env.declare(
        "insert",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![hashmap_ii.clone(), Integer.clone(), Integer.clone()],
            return_type: Box::new(hashmap_ii.clone()),
        },
    );

    // `get(HashMap<Integer,Integer>, Integer) -> Integer?`
    env.declare_overload(
        "get",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![hashmap_ii.clone(), Integer.clone()],
            return_type: Box::new(Type::Nullable(Box::new(Integer))),
        },
    );

    // `len` overload for HashMap
    env.declare_overload(
        "len",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![hashmap_ii.clone()],
            return_type: Box::new(Integer),
        },
    );

    // ── ADR-0047: contains + is_empty read-ops ──

    let trilean_refined = Type::Trilean { refined: true }; // Trilean!

    // `contains` overloads — owned variants
    env.declare_overload(
        "contains",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![String.clone(), String.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "contains",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![vector_integer.clone(), Integer.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "contains",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![hashmap_ii.clone(), Integer.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );

    let ref_string = Type::Reference(ReferenceForm::BorrowReadOnly, Box::new(String.clone()));
    let ref_vector = Type::Reference(
        ReferenceForm::BorrowReadOnly,
        Box::new(vector_integer.clone()),
    );
    let ref_hashmap = Type::Reference(ReferenceForm::BorrowReadOnly, Box::new(hashmap_ii.clone()));

    // ADR-0059 C.2: `len` and `get` overloads for &0 Vector/HashMap
    env.declare_overload(
        "len",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![ref_vector.clone()],
            return_type: Box::new(Integer.clone()),
        },
    );
    env.declare_overload(
        "len",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![ref_hashmap.clone()],
            return_type: Box::new(Integer.clone()),
        },
    );
    env.declare_overload(
        "get",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![ref_vector.clone(), Integer.clone()],
            return_type: Box::new(Type::Nullable(Box::new(Integer.clone()))),
        },
    );
    env.declare_overload(
        "get",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![ref_hashmap.clone(), Integer.clone()],
            return_type: Box::new(Type::Nullable(Box::new(Integer.clone()))),
        },
    );

    // `contains` overloads — &0 borrow variants
    env.declare_overload(
        "contains",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![ref_string.clone(), String.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "contains",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![ref_vector.clone(), Integer.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "contains",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![ref_hashmap.clone(), Integer.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );

    // `is_empty` overloads — owned variants
    env.declare_overload(
        "is_empty",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![String.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "is_empty",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![vector_integer],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "is_empty",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![hashmap_ii],
            return_type: Box::new(trilean_refined.clone()),
        },
    );

    // `is_empty` overloads — &0 borrow variants
    env.declare_overload(
        "is_empty",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![ref_string],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "is_empty",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![ref_vector],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "is_empty",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![ref_hashmap],
            return_type: Box::new(trilean_refined),
        },
    );

    // ── ADR-0048: mutable borrow — clear op ──

    // `clear(&0 mutable String)` — set len=0 in-place.
    // Only accepts &0 mutable (not shared &0, not owned String).
    env.declare_overload(
        "clear",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![Type::Reference(
                ReferenceForm::BorrowExclusiveMutable,
                Box::new(String.clone()),
            )],
            return_type: Box::new(Integer), // Unit-equivalent
        },
    );

    // ── ADR-0049 Lát 5: append op ──

    // `append(&0 mutable String, Integer)` — append one byte, realloc if needed.
    env.declare_overload(
        "append",
        Type::Function {
            type_params: Vec::new(),
            parameters: vec![
                Type::Reference(
                    ReferenceForm::BorrowExclusiveMutable,
                    Box::new(String.clone()),
                ),
                Integer.clone(),
            ],
            return_type: Box::new(Integer), // Unit-equivalent
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_walks_frames_innermost_first() {
        let mut env = TypeEnvironment::default();
        env.frames.push(Frame::default());
        env.declare("x", Type::Integer);
        env.push_frame();
        env.declare("x", Type::Tryte); // shadow
        assert_eq!(env.lookup("x"), Some(&Type::Tryte));
        env.pop_frame();
        assert_eq!(env.lookup("x"), Some(&Type::Integer));
    }

    #[test]
    fn declare_with_mut_records_mutability_flag() {
        let mut env = TypeEnvironment::default();
        env.frames.push(Frame::default());
        env.declare_with_mut("a", Type::Integer, true);
        env.declare("b", Type::Integer);
        assert!(env.lookup_binding("a").unwrap().mutable);
        assert!(!env.lookup_binding("b").unwrap().mutable);
    }

    #[test]
    fn declare_returns_false_when_shadowing_in_same_frame() {
        let mut env = TypeEnvironment::default();
        env.frames.push(Frame::default());
        assert!(env.declare("x", Type::Integer));
        assert!(!env.declare("x", Type::Tryte));
    }

    #[test]
    fn prelude_includes_print_and_println() {
        let env = TypeEnvironment::with_prelude();
        assert!(env.lookup("print").is_some());
        assert!(env.lookup("println").is_some());
        assert!(env.lookup("to_string").is_some());
    }

    #[test]
    fn lookup_missing_returns_none() {
        let env = TypeEnvironment::with_prelude();
        assert!(env.lookup("not_a_name").is_none());
    }
}
