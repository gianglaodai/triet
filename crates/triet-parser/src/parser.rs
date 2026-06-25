//! `Parser` struct and low-level helpers shared across modules.

use triet_lexer::{Span, SpannedToken, Token};
use triet_syntax::Arena;

use crate::error::ParseError;

/// State held by the parser as it walks a token stream.
///
/// Modules in this crate consume `&mut Parser` and add nodes to its
/// `arena`, advancing the cursor through `tokens`. Errors are
/// accumulated rather than aborting; on parse end the driver returns
/// either a `Program` or the full error list.
#[derive(Debug)]
pub(crate) struct Parser<'tokens> {
    tokens: &'tokens [SpannedToken],
    cursor: usize,
    pub(crate) arena: Arena,
    errors: Vec<ParseError>,
}

impl<'tokens> Parser<'tokens> {
    pub(crate) const fn new(tokens: &'tokens [SpannedToken]) -> Self {
        Self {
            tokens,
            cursor: 0,
            arena: Arena::new(),
            errors: Vec::new(),
        }
    }

    /// Token + span at the cursor, or `None` if the stream is exhausted.
    pub(crate) fn peek(&self) -> Option<&SpannedToken> {
        self.tokens.get(self.cursor)
    }

    /// Token at the cursor (no span), or `None` if exhausted.
    pub(crate) fn peek_token(&self) -> Option<&Token> {
        self.peek().map(|(t, _)| t)
    }

    /// Save the current cursor position for backtracking.
    pub(crate) const fn save_position(&self) -> usize {
        self.cursor
    }

    /// Restore cursor to a previously saved position. Used to
    /// backtrack after a speculative parse attempt (e.g., trying
    /// struct-literal `{ ... }` syntax).
    pub(crate) const fn restore_position(&mut self, position: usize) {
        self.cursor = position;
    }

    /// Look ahead up to `n` tokens without consuming them. Returns a
    /// slice (possibly shorter if EOF is reached first). Used for
    /// bounded lookahead (e.g. disambiguating struct literals).
    #[allow(dead_code)]
    pub(crate) fn peek_tokens(&self, n: usize) -> Vec<Token> {
        self.tokens[self.cursor..]
            .iter()
            .take(n)
            .map(|(t, _)| t.clone())
            .collect()
    }

    /// Span of the current token, or an empty span at end-of-input.
    pub(crate) fn current_span(&self) -> Span {
        self.peek()
            .map_or_else(|| self.eof_span(), |(_, span)| span.clone())
    }

    /// Empty span anchored at end-of-input — used for "expected more"
    /// diagnostics.
    pub(crate) fn eof_span(&self) -> Span {
        self.tokens
            .last()
            .map_or(0..0, |(_, span)| span.end..span.end)
    }

    /// Advance past the current token. Returns it (with span) or `None`.
    pub(crate) fn advance(&mut self) -> Option<&SpannedToken> {
        let token = self.tokens.get(self.cursor)?;
        self.cursor += 1;
        Some(token)
    }

    /// Consume the current token if it equals `expected`. Returns whether
    /// the token was consumed.
    pub(crate) fn eat(&mut self, expected: &Token) -> bool {
        if self.peek_token() == Some(expected) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    /// Consume `expected` or record an error. The `description` argument
    /// is used in the error message ("expected {description}").
    ///
    /// Returns the consumed token's span on success.
    pub(crate) fn expect(
        &mut self,
        expected: &Token,
        description: &str,
    ) -> Result<Span, ParseError> {
        match self.peek() {
            Some((token, span)) if token == expected => {
                let span = span.clone();
                self.cursor += 1;
                Ok(span)
            }
            Some((found, span)) => Err(ParseError::UnexpectedToken {
                expected: description.to_owned(),
                found: format!("{found:?}"),
                span: span.clone(),
            }),
            None => Err(ParseError::UnexpectedEof {
                expected: description.to_owned(),
                span: self.eof_span(),
            }),
        }
    }

    /// True if the cursor is past the last token.
    pub(crate) const fn at_end(&self) -> bool {
        self.cursor >= self.tokens.len()
    }

    /// Skip tokens until reaching a synchronization boundary or EOF.
    ///
    /// After an error, the parser jumps to the next plausible "fresh start"
    /// (an item or statement boundary) and continues, so the user gets
    /// multiple errors per run instead of one-at-a-time.
    pub(crate) fn synchronize(&mut self) {
        while let Some(token) = self.peek_token() {
            match token {
                // Stop *before* these — they begin a new construct.
                Token::Function
                | Token::Let
                | Token::Constant
                | Token::Type
                | Token::If
                | Token::IfQ
                | Token::Match
                | Token::While
                | Token::WhileQ
                | Token::For
                | Token::Loop
                | Token::Return
                | Token::Break
                | Token::Continue
                | Token::Use
                | Token::Module
                | Token::Public
                | Token::RBrace => return,

                // Consume `;` and stop *after* — it ends the bad statement.
                Token::Semi => {
                    self.cursor += 1;
                    return;
                }

                _ => {
                    self.cursor += 1;
                }
            }
        }
    }

    /// Push an error into the accumulator. Used after a recoverable
    /// failure, before calling [`Self::synchronize`].
    pub(crate) fn record_error(&mut self, error: ParseError) {
        self.errors.push(error);
    }

    /// Current cursor index. Exposed for the top-level driver to detect
    /// "no progress" after a failed `parse_item` + `synchronize`, so it
    /// can force-advance and avoid an infinite loop at sync boundaries.
    pub(crate) const fn cursor_index(&self) -> usize {
        self.cursor
    }

    /// End-byte of the most recently consumed token, falling back to
    /// `default` when nothing has been consumed yet.
    pub(crate) fn previous_token_end(&self, default: usize) -> usize {
        if self.cursor == 0 {
            return default;
        }
        self.tokens
            .get(self.cursor - 1)
            .map_or(default, |(_, span)| span.end)
    }

    /// Consume the parser, returning the populated arena and error list.
    pub(crate) fn finish(self) -> (Arena, Vec<ParseError>) {
        (self.arena, self.errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use triet_lexer::lex;

    fn parser_for(source: &str) -> (Vec<SpannedToken>, ()) {
        let tokens = lex(source).unwrap();
        (tokens, ())
    }

    #[test]
    fn peek_advance_cursor() {
        let (tokens, ()) = parser_for("function x");
        let mut parser = Parser::new(&tokens);
        assert!(matches!(parser.peek_token(), Some(Token::Function)));
        parser.advance();
        assert!(matches!(
            parser.peek_token(),
            Some(Token::Identifier(name)) if name == "x"
        ));
    }

    #[test]
    fn at_end_returns_true_when_exhausted() {
        let (tokens, ()) = parser_for("function");
        let mut parser = Parser::new(&tokens);
        assert!(!parser.at_end());
        parser.advance();
        assert!(parser.at_end());
    }

    #[test]
    fn eat_returns_false_on_mismatch() {
        let (tokens, ()) = parser_for("function");
        let mut parser = Parser::new(&tokens);
        assert!(!parser.eat(&Token::Let));
        assert!(parser.eat(&Token::Function));
        assert!(parser.at_end());
    }

    #[test]
    fn expect_returns_span_on_match() {
        let (tokens, ()) = parser_for("function");
        let mut parser = Parser::new(&tokens);
        let span = parser.expect(&Token::Function, "`function`").unwrap();
        assert_eq!(span, 0..8);
    }

    #[test]
    fn expect_returns_error_on_mismatch() {
        let (tokens, ()) = parser_for("function");
        let mut parser = Parser::new(&tokens);
        let error = parser.expect(&Token::Let, "`let`").unwrap_err();
        assert!(matches!(error, ParseError::UnexpectedToken { .. }));
    }

    #[test]
    fn expect_returns_eof_at_end() {
        let (tokens, ()) = parser_for("function");
        let mut parser = Parser::new(&tokens);
        parser.advance();
        let error = parser.expect(&Token::LParen, "`(`").unwrap_err();
        assert!(matches!(error, ParseError::UnexpectedEof { .. }));
    }

    #[test]
    fn synchronize_stops_at_keyword() {
        let (tokens, ()) = parser_for("garbage tokens here let x = 5");
        let mut parser = Parser::new(&tokens);
        parser.synchronize();
        assert!(matches!(parser.peek_token(), Some(Token::Let)));
    }

    #[test]
    fn synchronize_consumes_semicolon_and_stops() {
        let (tokens, ()) = parser_for("garbage ; rest");
        let mut parser = Parser::new(&tokens);
        parser.synchronize();
        // Should be after the `;`, looking at `rest`.
        assert!(matches!(
            parser.peek_token(),
            Some(Token::Identifier(name)) if name == "rest"
        ));
    }

    #[test]
    fn synchronize_stops_at_close_brace() {
        let (tokens, ()) = parser_for("garbage }");
        let mut parser = Parser::new(&tokens);
        parser.synchronize();
        assert!(matches!(parser.peek_token(), Some(Token::RBrace)));
    }

    #[test]
    fn record_error_accumulates() {
        let (tokens, ()) = parser_for("function");
        let mut parser = Parser::new(&tokens);
        parser.record_error(ParseError::UnexpectedEof {
            expected: "x".to_owned(),
            span: 0..0,
        });
        let (_, errors) = parser.finish();
        assert_eq!(errors.len(), 1);
    }
}
