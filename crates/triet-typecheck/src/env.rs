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
            type_parameters: Vec::new(),
            parameters: vec![String.clone()],
            return_type: Box::new(Unit.clone()),
        },
    );
    env.declare(
        "println",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![String.clone()],
            return_type: Box::new(Unit.clone()),
        },
    );
    env.declare(
        "read_line",
        Type::Function {
            type_parameters: Vec::new(),
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
            type_parameters: Vec::new(),
            parameters: vec![Integer.clone()],
            return_type: Box::new(String.clone()),
        },
    );
    env.declare(
        "tryte_to_string",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![Tryte.clone()],
            return_type: Box::new(String.clone()),
        },
    );
    env.declare(
        "long_to_string",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![Long.clone()],
            return_type: Box::new(String.clone()),
        },
    );
    env.declare(
        "trilean_to_string",
        Type::Function {
            type_parameters: Vec::new(),
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
            type_parameters: Vec::new(),
            parameters: vec![String.clone()],
            return_type: Box::new(Integer),
        },
    );
    env.declare_overload(
        "length",
        Type::Function {
            type_parameters: Vec::new(),
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
            type_parameters: Vec::new(),
            parameters: vec![String.clone(), String.clone()],
            return_type: Box::new(String.clone()),
        },
    );

    // `eq(String, String) -> Integer` — returns 1 (true) or 0 (false).
    env.declare(
        "eq",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![String.clone(), String.clone()],
            return_type: Box::new(Integer.clone()),
        },
    );

    // ── Phase 4.3b: Vector builtins (ADR-0040 §3.1; ADR-0077 Slice B generic) ──

    // ADR-0077 Slice B: `vector_new`/`push` are element-polymorphic over a
    // built-in `T`. The generic-fn machinery (v0.7.4.1) binds `T` from the
    // args; for the 0-arg `vector_new()` the element is seeded from the
    // expected type (`let v: Vector<String> = vector_new()`). No HM-unify —
    // structural + expected-type only (G ruling). Bare `let v = vector_new()`
    // with no context falls back to `Vector<Integer>` (byte-compat).
    let elem_t = Type::TypeParameter("T".into());
    let vector_t = Vector(Box::new(elem_t.clone()));
    let vector_type_params = vec![triet_syntax::TypeParameter {
        name: "T".into(),
        bound: None,
    }];

    // `vector_new<T>() -> Vector<T>` — heap-allocate an empty vector.
    env.declare(
        "vector_new",
        Type::Function {
            type_parameters: vector_type_params.clone(),
            parameters: Vec::new(),
            return_type: Box::new(vector_t.clone()),
        },
    );

    // `push<T>(Vector<T>, T) -> Vector<T>` — consume vec, append elem.
    env.declare(
        "push",
        Type::Function {
            type_parameters: vector_type_params,
            parameters: vec![vector_t.clone(), elem_t],
            return_type: Box::new(vector_t),
        },
    );

    // ADR-0077 P1.5: `pop<T>(Vector<T>) -> T?` — move the last element out
    // of the vector (len--). Returns ~0 for an empty vector. Same generic
    // params as push (T ∈ built-in). get-heap-refuse does NOT apply — pop is
    // a valid move-out (ownership cuts cleanly), unlike get (copy-out).
    let pop_params = vec![triet_syntax::TypeParameter {
        name: "T".into(),
        bound: None,
    }];
    let pop_elem_t = Type::TypeParameter("T".into());
    let pop_vector_t = Vector(Box::new(pop_elem_t.clone()));
    let pop_ret = Type::Nullable(Box::new(pop_elem_t));
    env.declare(
        "pop",
        Type::Function {
            type_parameters: pop_params,
            parameters: vec![pop_vector_t],
            return_type: Box::new(pop_ret),
        },
    );

    // `len` is overloaded: works on String and Vector<Integer>.
    // Registered via overloads so check_call can try each candidate.
    env.declare_overload(
        "len",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![String.clone()],
            return_type: Box::new(Integer.clone()),
        },
    );
    env.declare_overload(
        "len",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![vector_integer.clone()],
            return_type: Box::new(Integer),
        },
    );

    // ── ADR-0041 Bước 4: get(Vector<Integer>, Integer) -> Integer? ──
    // Total function: bounds-check returns null (NULL_SENTINEL), never panics.
    env.declare_overload(
        "get",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![vector_integer.clone(), Integer],
            return_type: Box::new(Type::Nullable(Box::new(Integer))),
        },
    );

    // ── ADR-0043 + ADR-0078 + ADR-0080: HashMap builtins ──
    // ADR-0080 KM-P1b: key K generic-ized ∈ {Integer, String} (was Integer
    // cứng); value V stays polymorphic exactly as before ("value-side giữ
    // nguyên máy HM-P1b" — every V constraint below mirrors the pre-existing
    // Integer-key declaration one-for-one, just with K swapped to String).
    // hashmap_new/insert/remove are GENERIC functions (declare, not overload)
    // in BOTH K and V now — K bound from the map/key arg (insert/remove) or
    // seeded from expected_type_stack (hashmap_new, 0-arg), same mechanism
    // `extract_type_params`'s `HashMap(pk,pv)` arm already walks for V.
    //
    // Monomorphic fallback for overloads (get/len/contains/is_empty, &0 ref):
    // one candidate per concrete K (Integer, String); V held fixed per-site.
    let hashmap_ii = Type::HashMap(Box::new(Integer.clone()), Box::new(Integer.clone()));
    let hashmap_str_int = Type::HashMap(Box::new(String.clone()), Box::new(Integer.clone()));

    // Generic type parameters K, V for K/V-polymorphic builtins (hashmap_new/
    // insert/remove). K ∈ {Integer, String} enforced at the REFUSE boundary
    // (ADR-0080 Mũi C2, E1048 UnsupportedHashMapKey) — the type param itself
    // doesn't constrain membership, the REFUSE check does.
    let hm_key_param = Type::TypeParameter("K".into());
    let hm_val_param = Type::TypeParameter("V".into());
    let hm_kv = Type::HashMap(
        Box::new(hm_key_param.clone()),
        Box::new(hm_val_param.clone()),
    );
    let hm_type_params = vec![
        triet_syntax::TypeParameter {
            name: "K".into(),
            bound: None,
        },
        triet_syntax::TypeParameter {
            name: "V".into(),
            bound: None,
        },
    ];

    // `hashmap_new<K,V>() -> HashMap<K, V>` — 0-arg generic.
    // K and V are both seeded from expected_type_stack (same mechanism as
    // vector_new); ADR-0078 P1b's Integer-only default now only fires when
    // NEITHER has context (see exprs.rs check_call).
    env.declare(
        "hashmap_new",
        Type::Function {
            type_parameters: hm_type_params.clone(),
            parameters: Vec::new(),
            return_type: Box::new(hm_kv.clone()),
        },
    );

    // `insert<K,V>(HashMap<K,V>, K, V) -> HashMap<K,V>`
    // K bound from arg[0] (map) or arg[1] (key); V bound from arg[0] (map) or
    // arg[2] (value); consumes map handle + key + value (heap → move;
    // Copy→no-op, per ADR-0078/0080 MŨI D — key-consume mirrors value-consume
    // via the SAME per-call `is_copy` check in borrowck, checker.rs).
    env.declare(
        "insert",
        Type::Function {
            type_parameters: hm_type_params.clone(),
            parameters: vec![hm_kv.clone(), hm_key_param.clone(), hm_val_param.clone()],
            return_type: Box::new(hm_kv.clone()),
        },
    );

    // `remove<K,V>(HashMap<K,V>, K) -> V?` — move value out
    // (ownership cut, like pop). Returns ~0 when key not present. The
    // lookup key is a BORROW (ADR-0080 Mũi D4), not consumed — asymmetric
    // with insert's Move (Mũi D point 4).
    env.declare(
        "remove",
        Type::Function {
            type_parameters: hm_type_params,
            parameters: vec![hm_kv, hm_key_param],
            return_type: Box::new(Type::Nullable(Box::new(hm_val_param))),
        },
    );

    // ── HashMap overloads (monomorphic, for get/len/contains/is_empty) ──
    // These use monomorphic `hashmap_ii`/`hashmap_str_int` (HashMap<Integer,
    // Integer> / HashMap<String,Integer>) for backward compat + the new
    // String-key parity track. Heap value gets are refused by E1047 in
    // resolve_overload before candidate matching.

    // `get(HashMap<Integer,Integer>, Integer) -> Integer?`
    env.declare_overload(
        "get",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![hashmap_ii.clone(), Integer.clone()],
            return_type: Box::new(Type::Nullable(Box::new(Integer))),
        },
    );
    // `get(HashMap<String,Integer>, String) -> Integer?` (ADR-0080 parity)
    env.declare_overload(
        "get",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![hashmap_str_int.clone(), String.clone()],
            return_type: Box::new(Type::Nullable(Box::new(Integer))),
        },
    );

    // `len` overload for HashMap
    env.declare_overload(
        "len",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![hashmap_ii.clone()],
            return_type: Box::new(Integer),
        },
    );
    env.declare_overload(
        "len",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![hashmap_str_int.clone()],
            return_type: Box::new(Integer),
        },
    );

    // ── ADR-0047: contains + is_empty read-ops ──

    let trilean_refined = Type::Trilean { refined: true }; // Trilean!

    // `contains` overloads — owned variants
    env.declare_overload(
        "contains",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![String.clone(), String.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "contains",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![vector_integer.clone(), Integer.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "contains",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![hashmap_ii.clone(), Integer.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "contains",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![hashmap_str_int.clone(), String.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );

    let ref_string = Type::Reference(ReferenceForm::BorrowReadOnly, Box::new(String.clone()));
    let ref_vector = Type::Reference(
        ReferenceForm::BorrowReadOnly,
        Box::new(vector_integer.clone()),
    );
    let ref_hashmap = Type::Reference(ReferenceForm::BorrowReadOnly, Box::new(hashmap_ii.clone()));
    let ref_hashmap_str_int = Type::Reference(
        ReferenceForm::BorrowReadOnly,
        Box::new(hashmap_str_int.clone()),
    );

    // ADR-0079 Slice B: borrow-get overloads for heap element/value types.
    // Returns `(&0 V)?` — a nullable reference to the element slot. Zero-copy.
    // Concrete overloads for each heap type (P1: String). Other heap types
    // (Vector<Vector<String>> etc.) need follow-up overloads.
    let ref_vector_string = Type::Reference(
        ReferenceForm::BorrowReadOnly,
        Box::new(Vector(Box::new(String.clone()))),
    );
    let ref_hashmap_string = Type::Reference(
        ReferenceForm::BorrowReadOnly,
        Box::new(Type::HashMap(
            Box::new(Integer.clone()),
            Box::new(String.clone()),
        )),
    );
    // ADR-0080: String-key parity for the get_ref value=String overload —
    // `HashMap<String,String>` (the ★SS tooth) reads its heap value the same
    // zero-copy-borrow way `HashMap<Integer,String>` already does.
    let ref_hashmap_str_str = Type::Reference(
        ReferenceForm::BorrowReadOnly,
        Box::new(Type::HashMap(
            Box::new(String.clone()),
            Box::new(String.clone()),
        )),
    );
    let ref_string_ret = Type::Reference(ReferenceForm::BorrowReadOnly, Box::new(String.clone()));
    let nullable_ref_string = Type::Nullable(Box::new(ref_string_ret));

    env.declare_overload(
        "get",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_vector_string, Integer.clone()],
            return_type: Box::new(nullable_ref_string.clone()),
        },
    );
    env.declare_overload(
        "get",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_hashmap_string, Integer.clone()],
            return_type: Box::new(nullable_ref_string.clone()),
        },
    );
    env.declare_overload(
        "get",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_hashmap_str_str, String.clone()],
            return_type: Box::new(nullable_ref_string),
        },
    );

    // ADR-0059 C.2: `len` and `get` overloads for &0 Vector/HashMap
    env.declare_overload(
        "len",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_vector.clone()],
            return_type: Box::new(Integer.clone()),
        },
    );
    env.declare_overload(
        "len",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_hashmap.clone()],
            return_type: Box::new(Integer.clone()),
        },
    );
    env.declare_overload(
        "len",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_hashmap_str_int.clone()],
            return_type: Box::new(Integer.clone()),
        },
    );
    env.declare_overload(
        "get",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_vector.clone(), Integer.clone()],
            return_type: Box::new(Type::Nullable(Box::new(Integer.clone()))),
        },
    );
    env.declare_overload(
        "get",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_hashmap.clone(), Integer.clone()],
            return_type: Box::new(Type::Nullable(Box::new(Integer.clone()))),
        },
    );
    env.declare_overload(
        "get",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_hashmap_str_int.clone(), String.clone()],
            return_type: Box::new(Type::Nullable(Box::new(Integer.clone()))),
        },
    );

    // `contains` overloads — &0 borrow variants
    env.declare_overload(
        "contains",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_string.clone(), String.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "contains",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_vector.clone(), Integer.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "contains",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_hashmap.clone(), Integer.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "contains",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_hashmap_str_int.clone(), String.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );

    // `is_empty` overloads — owned variants
    env.declare_overload(
        "is_empty",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![String.clone()],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "is_empty",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![vector_integer],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "is_empty",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![hashmap_ii],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "is_empty",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![hashmap_str_int],
            return_type: Box::new(trilean_refined.clone()),
        },
    );

    // `is_empty` overloads — &0 borrow variants
    env.declare_overload(
        "is_empty",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_string],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "is_empty",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_vector],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "is_empty",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_hashmap],
            return_type: Box::new(trilean_refined.clone()),
        },
    );
    env.declare_overload(
        "is_empty",
        Type::Function {
            type_parameters: Vec::new(),
            parameters: vec![ref_hashmap_str_int],
            return_type: Box::new(trilean_refined),
        },
    );

    // ── ADR-0048: mutable borrow — clear op ──

    // `clear(&0 mutable String)` — set len=0 in-place.
    // Only accepts &0 mutable (not shared &0, not owned String).
    env.declare_overload(
        "clear",
        Type::Function {
            type_parameters: Vec::new(),
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
            type_parameters: Vec::new(),
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
