//! Source span and spanned-node wrapper.
//!
//! Every AST node carries a [`Span`] indicating its byte range in the
//! original source, enabling precise error messages without re-running
//! the lexer.

use std::ops::Range;

/// Half-open byte range `[start, end)` in source text.
pub type Span = Range<usize>;

/// An AST node paired with its source span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spanned<T> {
    /// The underlying node.
    pub node: T,
    /// The source range that produced this node.
    pub span: Span,
}

impl<T> Spanned<T> {
    /// Wrap a node with its span.
    pub const fn new(node: T, span: Span) -> Self {
        Self { node, span }
    }

    /// Apply a function to the inner node, preserving the span.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            node: f(self.node),
            span: self.span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_holds_node_and_span() {
        let spanned = Spanned::new(42, 0..10);
        assert_eq!(spanned.node, 42);
        assert_eq!(spanned.span, 0..10);
    }

    #[test]
    fn map_transforms_node_and_keeps_span() {
        let spanned = Spanned::new(5, 7..12);
        let mapped = spanned.map(|n| n * 2);
        assert_eq!(mapped.node, 10);
        assert_eq!(mapped.span, 7..12);
    }
}
