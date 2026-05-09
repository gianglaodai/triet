//! Runtime value environment — frame stack with lexical scoping.

use std::collections::HashMap;

use crate::value::Value;

/// A stack of name → Value frames.
#[derive(Clone, Debug, Default)]
pub struct ValueEnvironment {
    frames: Vec<Frame>,
}

#[derive(Clone, Debug, Default)]
struct Frame {
    names: HashMap<String, Value>,
}

impl ValueEnvironment {
    /// Construct an empty environment with one root frame.
    #[must_use]
    pub fn new() -> Self {
        Self {
            frames: vec![Frame::default()],
        }
    }

    /// Push a new frame.
    pub fn push_frame(&mut self) {
        self.frames.push(Frame::default());
    }

    /// Pop the top frame (panics if only the root remains).
    pub fn pop_frame(&mut self) {
        assert!(self.frames.len() > 1, "cannot pop the root environment frame");
        self.frames.pop();
    }

    /// Bind `name` to `value` in the top frame.
    pub fn declare(&mut self, name: &str, value: Value) {
        let top = self.frames.last_mut().expect("at least one frame");
        top.names.insert(name.to_owned(), value);
    }

    /// Reassign an existing binding, walking frames from innermost out.
    /// Returns `true` if a binding was found and updated, `false`
    /// otherwise. The type checker is the source of truth for whether
    /// an assignment is legal; this method only handles the lookup +
    /// in-place update.
    pub fn assign(&mut self, name: &str, value: Value) -> bool {
        for frame in self.frames.iter_mut().rev() {
            if frame.names.contains_key(name) {
                frame.names.insert(name.to_owned(), value);
                return true;
            }
        }
        false
    }

    /// Look up `name`, walking from innermost frame outward.
    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<&Value> {
        for frame in self.frames.iter().rev() {
            if let Some(value) = frame.names.get(name) {
                return Some(value);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use triet_core::Integer;

    #[test]
    fn lookup_walks_frames_innermost_first() {
        let mut env = ValueEnvironment::new();
        env.declare("x", Value::Integer(Integer::new(1).unwrap()));
        env.push_frame();
        env.declare("x", Value::Integer(Integer::new(2).unwrap()));
        match env.lookup("x") {
            Some(Value::Integer(i)) => assert_eq!(i.to_i64(), 2),
            other => panic!("expected Integer(2), got {other:?}"),
        }
        env.pop_frame();
        match env.lookup("x") {
            Some(Value::Integer(i)) => assert_eq!(i.to_i64(), 1),
            other => panic!("expected Integer(1), got {other:?}"),
        }
    }

    #[test]
    fn lookup_returns_none_when_unbound() {
        let env = ValueEnvironment::new();
        assert!(env.lookup("missing").is_none());
    }

    #[test]
    fn assign_updates_existing_binding_in_outer_frame() {
        let mut env = ValueEnvironment::new();
        env.declare("x", Value::Integer(Integer::new(1).unwrap()));
        env.push_frame();
        // Assignment in inner frame should find binding in outer frame
        // and update it in place — not create a new shadow.
        assert!(env.assign("x", Value::Integer(Integer::new(2).unwrap())));
        env.pop_frame();
        match env.lookup("x") {
            Some(Value::Integer(i)) => assert_eq!(i.to_i64(), 2),
            other => panic!("expected Integer(2), got {other:?}"),
        }
    }

    #[test]
    fn assign_returns_false_when_name_missing() {
        let mut env = ValueEnvironment::new();
        assert!(!env.assign("nope", Value::Integer(Integer::new(0).unwrap())));
    }
}
