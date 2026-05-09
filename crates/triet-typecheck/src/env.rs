//! Type environment — scoped variable bindings + a built-in prelude.

use std::collections::HashMap;

use crate::types::Type;

/// A stack of name → binding frames. Entering a block / function pushes
/// a frame; leaving pops it. Lookup walks from innermost to outermost.
#[derive(Clone, Debug, Default)]
pub struct TypeEnvironment {
    /// The frame stack. Accessible to the checker for enum-variant
    /// scanning; prefer `lookup()` / `declare()` for normal use.
    pub(crate) frames: Vec<Frame>,
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
        let mut env = Self {
            frames: vec![Frame::default()],
        };
        bind_prelude(&mut env);
        env
    }

    /// Push a new (empty) frame onto the stack.
    pub fn push_frame(&mut self) {
        self.frames.push(Frame::default());
    }

    /// Pop the top frame. Panics if only the root frame remains.
    pub fn pop_frame(&mut self) {
        assert!(
            self.frames.len() > 1,
            "cannot pop the root environment frame",
        );
        self.frames.pop();
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
}

/// Populate the root frame with built-in functions used by the v0.1
/// demo programs (`print`, `println`, `to_string`, etc.). The prelude
/// is intentionally minimal — extending it lives alongside library
/// growth, not the type-checker core.
fn bind_prelude(env: &mut TypeEnvironment) {
    use Type::{Integer, Long, String, Trilean, Tryte, Unit};

    env.declare(
        "print",
        Type::Function {
            parameters: vec![String.clone()],
            return_type: Box::new(Unit.clone()),
        },
    );
    env.declare(
        "println",
        Type::Function {
            parameters: vec![String.clone()],
            return_type: Box::new(Unit.clone()),
        },
    );
    env.declare(
        "read_line",
        Type::Function {
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
            parameters: vec![Integer.clone()],
            return_type: Box::new(String.clone()),
        },
    );
    env.declare(
        "tryte_to_string",
        Type::Function {
            parameters: vec![Tryte.clone()],
            return_type: Box::new(String.clone()),
        },
    );
    env.declare(
        "long_to_string",
        Type::Function {
            parameters: vec![Long.clone()],
            return_type: Box::new(String.clone()),
        },
    );
    env.declare(
        "trilean_to_string",
        Type::Function {
            parameters: vec![Trilean.clone()],
            return_type: Box::new(String.clone()),
        },
    );

    // `length` on String is exposed as a free function for v0.1; in
    // v0.2 it should become a method.
    env.declare(
        "length",
        Type::Function {
            parameters: vec![String.clone()],
            return_type: Box::new(Integer),
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
