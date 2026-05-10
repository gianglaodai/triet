//! Mode-aware Triết lexer.
//!
//! The wrapper [`Lexer`] runs a small mode stack on top of the logos-
//! generated tokenizer to handle f-strings safely (see SPEC.md §1.5.4).
//! In `Normal` and `Interpolation` modes the wrapper delegates to logos;
//! in `FString` mode it hand-scans for literal text, `{{`/`}}` escapes,
//! `{` (interpolation start), and `"` (f-string end).
//!
//! This approach mirrors how `rustc`, the Swift compiler, and Python
//! 3.12+ tokenize f-strings — every `Span` is absolute from the start,
//! and string literals or nested blocks inside interpolations don't
//! confuse the brace tracker.

use logos::Logos as _;

use crate::{
    error::{LexError, Span},
    token::Token,
};

/// A token paired with its byte span in the source.
pub type SpannedToken = (Token, Span);

/// One layer of the lexer's mode stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LexerMode {
    /// Default mode — emit tokens via logos.
    Normal,
    /// Inside the body of an f-string, between `f"` and the matching `"`.
    FString,
    /// Inside an interpolation expression `{...}` of an f-string. The
    /// `brace_depth` counter increases for every `{` we see (e.g. nested
    /// blocks) and decreases on `}`; the `}` that brings depth below
    /// zero closes the interpolation.
    Interpolation { brace_depth: u32 },
}

/// Mode-aware Triết tokenizer.
///
/// `Lexer` owns a cursor into the source string and a mode stack. Its
/// [`Iterator`] implementation produces one token at a time; the
/// convenience [`lex`] function collects all tokens eagerly.
#[derive(Debug)]
pub struct Lexer<'source> {
    source: &'source str,
    cursor: usize,
    modes: Vec<LexerMode>,
}

impl<'source> Lexer<'source> {
    /// Create a new lexer positioned at the start of `source`.
    #[must_use]
    pub fn new(source: &'source str) -> Self {
        Self {
            source,
            cursor: 0,
            modes: vec![LexerMode::Normal],
        }
    }

    fn current_mode(&self) -> LexerMode {
        self.modes.last().copied().unwrap_or(LexerMode::Normal)
    }

    /// Produce the next token, or `None` if the source is exhausted.
    ///
    /// Errors are returned as `Some(Err(_))`; iteration may continue
    /// after an error in callers that wish to attempt recovery, but the
    /// default driver [`lex`] stops at the first error.
    pub fn next_token(&mut self) -> Option<Result<SpannedToken, LexError>> {
        match self.current_mode() {
            LexerMode::FString => self.next_in_fstring_mode(),
            LexerMode::Normal | LexerMode::Interpolation { .. } => self.next_in_normal_mode(),
        }
    }

    /// Advance through whitespace and line comments at the current cursor.
    /// Returns the number of bytes skipped (relative to the start of the
    /// remaining slice).
    fn skip_whitespace_and_line_comments(remaining: &str) -> usize {
        let bytes = remaining.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b' ' | b'\t' | b'\r' | b'\n' => i += 1,
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    // Skip until end of line.
                    i += 2;
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                _ => break,
            }
        }
        i
    }

    fn next_in_normal_mode(&mut self) -> Option<Result<SpannedToken, LexError>> {
        let remaining = self.source.get(self.cursor..)?;
        let skipped = Self::skip_whitespace_and_line_comments(remaining);
        let after_ws_offset = self.cursor + skipped;
        let after_ws = &self.source[after_ws_offset..];

        if after_ws.is_empty() {
            self.cursor = self.source.len();
            return None;
        }

        // Detect `f"` ourselves and switch into FString mode. We must do
        // this *before* delegating to logos, since logos would otherwise
        // tokenize `f` as an identifier and the `"` as a separate string.
        if after_ws.starts_with("f\"") {
            let start = after_ws_offset;
            let end = start + 2;
            self.cursor = end;
            self.modes.push(LexerMode::FString);
            return Some(Ok((Token::FStringStart, start..end)));
        }

        // Run logos against the remaining slice (post-whitespace).
        let mut sub_lexer = Token::lexer(after_ws);
        match sub_lexer.next() {
            None => {
                self.cursor = self.source.len();
                None
            }
            Some(Ok(token)) => {
                let local = sub_lexer.span();
                let absolute = (after_ws_offset + local.start)..(after_ws_offset + local.end);
                self.cursor = absolute.end;

                // Track interpolation brace depth.
                if let Some(LexerMode::Interpolation { brace_depth }) = self.modes.last_mut() {
                    match token {
                        Token::LBrace => {
                            *brace_depth += 1;
                            return Some(Ok((Token::LBrace, absolute)));
                        }
                        Token::RBrace => {
                            if *brace_depth > 0 {
                                *brace_depth -= 1;
                                return Some(Ok((Token::RBrace, absolute)));
                            }
                            self.modes.pop();
                            return Some(Ok((Token::InterpolationEnd, absolute)));
                        }
                        // All other tokens inside an interpolation are literal f-string text.
                        _ => {}
                    }
                }

                Some(Ok((token, absolute)))
            }
            Some(Err(LexError::Unrecognized)) => {
                let local = sub_lexer.span();
                let absolute = (after_ws_offset + local.start)..(after_ws_offset + local.end);
                let snippet = self.source[absolute.clone()].to_owned();
                self.cursor = absolute.end;
                Some(Err(LexError::UnexpectedCharacter { span: absolute, snippet }))
            }
            Some(Err(other)) => {
                // Callback-emitted errors carry spans relative to the
                // slice we passed to logos; shift them to absolute.
                let local = sub_lexer.span();
                self.cursor = after_ws_offset + local.end;
                Some(Err(other.shift_span(after_ws_offset)))
            }
        }
    }

    // `next_in_fstring_mode` always returns `Some(...)` (either a token
    // or a deterministic error like `UnterminatedString`), but we keep
    // the `Option` return type to match `next_in_normal_mode` for a
    // uniform caller in `next_token`.
    #[allow(clippy::unnecessary_wraps)]
    fn next_in_fstring_mode(&mut self) -> Option<Result<SpannedToken, LexError>> {
        let start = self.cursor;
        let mut text = String::new();

        while let Some(remaining) = self.source.get(self.cursor..) {
            let mut chars = remaining.chars();
            let Some(character) = chars.next() else {
                return Some(Err(LexError::UnterminatedString { span: start..self.cursor }));
            };
            let char_len = character.len_utf8();

            match character {
                '{' => {
                    if remaining.as_bytes().get(1) == Some(&b'{') {
                        text.push('{');
                        self.cursor += 2;
                        continue;
                    }
                    if !text.is_empty() {
                        // Emit accumulated text first; the `{` becomes
                        // InterpolationStart on the next call.
                        return Some(Ok((Token::FStringText(text), start..self.cursor)));
                    }
                    let interp_start = self.cursor;
                    self.cursor += 1;
                    self.modes.push(LexerMode::Interpolation { brace_depth: 0 });
                    return Some(Ok((Token::InterpolationStart, interp_start..self.cursor)));
                }
                '}' => {
                    if remaining.as_bytes().get(1) == Some(&b'}') {
                        text.push('}');
                        self.cursor += 2;
                        continue;
                    }
                    return Some(Err(LexError::UnmatchedFStringBrace {
                        span: self.cursor..self.cursor + char_len,
                    }));
                }
                '"' => {
                    if !text.is_empty() {
                        return Some(Ok((Token::FStringText(text), start..self.cursor)));
                    }
                    let end_start = self.cursor;
                    self.cursor += 1;
                    self.modes.pop();
                    return Some(Ok((Token::FStringEnd, end_start..self.cursor)));
                }
                '\\' => {
                    let Some(next_character) = chars.next() else {
                        return Some(Err(LexError::UnterminatedString { span: start..self.cursor }));
                    };
                    let escaped = match next_character {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '\\' => '\\',
                        '"' => '"',
                        '0' => '\0',
                        other => {
                            return Some(Err(LexError::InvalidEscape {
                                span: self.cursor..self.cursor + 1 + other.len_utf8(),
                                sequence: format!("\\{other}"),
                            }));
                        }
                    };
                    text.push(escaped);
                    self.cursor += 1 + next_character.len_utf8();
                }
                _ => {
                    text.push(character);
                    self.cursor += char_len;
                }
            }
        }

        // Source exhausted without a closing `"`.
        Some(Err(LexError::UnterminatedString { span: start..self.cursor }))
    }
}

impl Iterator for Lexer<'_> {
    type Item = Result<SpannedToken, LexError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_token()
    }
}

/// Tokenize the given source string into a vector of `(Token, Span)` pairs.
///
/// Whitespace and line comments are skipped. The first lexical error
/// encountered is returned; the lexer does not attempt recovery in v0.1.
///
/// # Errors
///
/// Returns the first [`LexError`] encountered while tokenizing.
pub fn lex(source: &str) -> Result<Vec<SpannedToken>, LexError> {
    Lexer::new(source).collect()
}

