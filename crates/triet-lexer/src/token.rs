//! Token definitions for Triết source code.

// `logos` callbacks must accept `&mut Lexer` even when they only inspect
// the slice; `unnecessary_wraps` is also a false positive for callbacks
// that share a signature with fallible siblings.
#![allow(clippy::needless_pass_by_ref_mut, clippy::unnecessary_wraps)]

use logos::{Lexer, Logos};

use crate::error::LexError;

/// A numeric type suffix attached to an integer literal.
///
/// Suffixes are written as `_trit`, `_tryte`, `_integer`, `_long` after a
/// decimal literal: `1_trit`, `5_tryte`, `42_integer`, `1_000_long`.
/// When absent, the literal defaults to `Integer` (see SPEC §1.5.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumericSuffix {
    /// `_trit` — single ternary digit.
    Trit,
    /// `_tryte` — 9-trit integer.
    Tryte,
    /// `_integer` — 27-trit integer (default; suffix is redundant but allowed).
    Integer,
    /// `_long` — 81-trit integer.
    Long,
}

impl NumericSuffix {
    fn parse(suffix: &str) -> Option<Self> {
        match suffix {
            "trit" => Some(Self::Trit),
            "tryte" => Some(Self::Tryte),
            "integer" => Some(Self::Integer),
            "long" => Some(Self::Long),
            _ => None,
        }
    }
}

/// A parsed integer literal: numeric value plus an optional type suffix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IntLiteral {
    /// The numeric value (always parsed as `i128` for headroom).
    pub value: i128,
    /// Optional explicit type suffix (`None` defaults to `Integer`).
    pub suffix: Option<NumericSuffix>,
}

/// A token produced by the Triết lexer.
///
/// Tokens carry their semantic value where applicable (literals, identifiers).
/// Spans are tracked separately by the [`crate::lex`] driver, not stored on
/// the token itself.
#[derive(Clone, Debug, Logos, PartialEq, Eq)]
#[logos(skip r"[ \t\r\n]+", skip r"//[^\n]*", error = LexError)]
pub enum Token {
    // === Keywords ===
    /// `fn` — function definition.
    #[token("fn")]
    Fn,
    /// `let` — variable binding.
    #[token("let")]
    Let,
    /// `mut` — mutable binding modifier.
    #[token("mut")]
    Mut,
    /// `const` — compile-time constant.
    #[token("const")]
    Const,
    /// `type` — type alias.
    #[token("type")]
    Type,
    /// `if` — conditional (requires definite Trilean).
    #[token("if")]
    If,
    /// `if?` — conditional treating `Unknown` as `False`.
    #[token("if?")]
    IfQ,
    /// `else` — alternative branch.
    #[token("else")]
    Else,
    /// `match` — pattern matching.
    #[token("match")]
    Match,
    /// `return` — early return from function.
    #[token("return")]
    Return,
    /// `for` — iteration loop.
    #[token("for")]
    For,
    /// `while` — condition-driven loop (requires definite Trilean).
    #[token("while")]
    While,
    /// `while?` — condition-driven loop treating `Unknown` as `False`.
    #[token("while?")]
    WhileQ,
    /// `loop` — infinite loop with break-with-value.
    #[token("loop")]
    Loop,
    /// `break` — exit loop, optionally with value (in `loop`).
    #[token("break")]
    Break,
    /// `continue` — skip to next iteration.
    #[token("continue")]
    Continue,
    /// `in` — used in `for x in ...`.
    #[token("in")]
    In,
    /// `true` — Trilean true literal.
    #[token("true")]
    True,
    /// `false` — Trilean false literal.
    #[token("false")]
    False,
    /// `unknown` — Trilean unknown literal.
    #[token("unknown")]
    Unknown,
    /// `null` — null marker for nullable types `T?`.
    #[token("null")]
    Null,
    /// `not` — prefix logical NOT (keyword form of `!`).
    #[token("not")]
    Not,
    /// `and` — logical AND (keyword form of `&&`).
    #[token("and")]
    And,
    /// `or` — logical OR (keyword form of `||`).
    #[token("or")]
    Or,
    /// `xor` — Łukasiewicz XOR (keyword form of `^`).
    #[token("xor")]
    Xor,
    /// `iff` — Łukasiewicz biconditional (keyword form of `<=>`).
    #[token("iff")]
    Iff,
    /// `implies` — Łukasiewicz implication (keyword form of `=>`).
    #[token("implies")]
    Implies,
    /// `kleene_implies` — Kleene K3 implication.
    #[token("kleene_implies")]
    KleeneImplies,
    /// `kleene_xor` — Kleene K3 XOR.
    #[token("kleene_xor")]
    KleeneXor,
    /// `kleene_iff` — Kleene K3 biconditional.
    #[token("kleene_iff")]
    KleeneIff,
    /// `import` — module import. Dot-path form (`import std.io`) is
    /// the v0.2 syntax; v0.2.x adds `use` with `::` path syntax per
    /// ADR-0005.
    #[token("import")]
    Import,
    /// `mod` — module declaration: `mod foo;` (file-bound) or
    /// `mod foo { ... }` (inline). Per ADR-0005.
    #[token("mod")]
    Mod,
    /// `pub` — public visibility modifier on items (`pub`, `pub(pkg)`).
    #[token("pub")]
    Pub,
    /// `owned` — parameter takes ownership (Mojo-style).
    #[token("owned")]
    Owned,
    /// `struct` — struct definition (v0.2+).
    #[token("struct")]
    Struct,
    /// `enum` — enum definition (v0.2+).
    #[token("enum")]
    Enum,
    /// `crate` — path keyword: refers to the current crate root.
    /// Reserved per ADR-0005; path syntax usage lands in v0.2.x.5.
    #[token("crate")]
    Crate,
    /// `self` — path keyword: refers to the current module.
    /// Reserved per ADR-0005; path syntax usage lands in v0.2.x.5.
    #[token("self")]
    SelfKw,
    /// `super` — path keyword: refers to the parent module.
    /// Reserved per ADR-0005; path syntax usage lands in v0.2.x.5.
    #[token("super")]
    Super,

    // === Multi-character operators (longest-match ordering matters) ===
    /// `<=>` — Łukasiewicz biconditional.
    #[token("<=>")]
    LtEqGt,
    /// `<~>` — Kleene biconditional.
    #[token("<~>")]
    LtTildeGt,
    /// `..=` — inclusive range.
    #[token("..=")]
    DotDotEq,
    /// `..` — exclusive range.
    #[token("..")]
    DotDot,
    /// `==` — equality.
    #[token("==")]
    EqEq,
    /// `!=` — inequality.
    #[token("!=")]
    NotEq,
    /// `<=` — less than or equal.
    #[token("<=")]
    LtEq,
    /// `>=` — greater than or equal.
    #[token(">=")]
    GtEq,
    /// `&&` — logical AND.
    #[token("&&")]
    AndAnd,
    /// `||` — logical OR.
    #[token("||")]
    OrOr,
    /// `=>` — Łukasiewicz implication / match arm.
    #[token("=>")]
    FatArrow,
    /// `~>` — Kleene implication.
    #[token("~>")]
    TildeArrow,
    /// `~^` — Kleene XOR.
    #[token("~^")]
    TildeCaret,
    /// `->` — function return type.
    #[token("->")]
    ThinArrow,
    /// `?.` — safe call (nullable chain).
    #[token("?.")]
    QuestionDot,
    /// `?:` — Elvis operator (default for null).
    #[token("?:")]
    QuestionColon,
    /// `!!` — force unwrap (panic on null).
    #[token("!!")]
    BangBang,
    /// `::` — path separator (v0.2+ modules).
    #[token("::")]
    ColonColon,

    // === Single- and double-character operators ===
    /// `**` — exponentiation (must precede `*` for longest-match).
    #[token("**")]
    StarStar,
    /// `+` — addition / positive trit.
    #[token("+")]
    Plus,
    /// `-` — subtraction / negation / negative trit.
    #[token("-")]
    Minus,
    /// `*` — multiplication.
    #[token("*")]
    Star,
    /// `/` — division.
    #[token("/")]
    Slash,
    /// `%%` — balanced ternary modulo.
    #[token("%%")]
    PercentPercent,
    /// `=` — assignment.
    #[token("=")]
    Assign,
    /// `<` — less than.
    #[token("<")]
    Lt,
    /// `>` — greater than.
    #[token(">")]
    Gt,
    /// `!` — logical NOT.
    #[token("!")]
    Bang,
    /// `^` — Łukasiewicz XOR.
    #[token("^")]
    Caret,
    /// `?` — nullable type marker / null assertion.
    #[token("?")]
    Question,

    // === Punctuation ===
    /// `:` — type annotation separator.
    #[token(":")]
    Colon,
    /// `;` — statement terminator.
    #[token(";")]
    Semi,
    /// `,` — element separator.
    #[token(",")]
    Comma,
    /// `.` — field access.
    #[token(".")]
    Dot,
    /// `{` — open brace.
    #[token("{")]
    LBrace,
    /// `}` — close brace.
    #[token("}")]
    RBrace,
    /// `[` — open bracket.
    #[token("[")]
    LBracket,
    /// `]` — close bracket.
    #[token("]")]
    RBracket,
    /// `(` — open paren.
    #[token("(")]
    LParen,
    /// `)` — close paren.
    #[token(")")]
    RParen,
    /// `|` — pipe (or-pattern, closure delimiter).
    #[token("|")]
    Pipe,
    /// `_` — wildcard.
    #[token("_")]
    Underscore,

    // === Literals ===
    /// Decimal integer literal: `42`, `1_000`, `5_tryte`, `1_long`.
    ///
    /// Digits portion always ends on a digit (each `_?[0-9]` group ends in a
    /// digit), so the optional `_<suffix>` branch can claim its leading
    /// underscore unambiguously.
    #[regex(
        r"[0-9](_?[0-9])*(_trit|_tryte|_integer|_long)?",
        lex_decimal_integer,
    )]
    IntegerLiteral(IntLiteral),

    /// Balanced ternary literal: `0t+0-+`, `0t+_0_-`.
    #[regex(r"0t[+\-0_]+", lex_ternary_integer)]
    TernaryLiteral(IntLiteral),

    /// String literal `"..."` (single-line, with escape sequences).
    #[regex(r#""([^"\\]|\\.)*""#, lex_string_literal)]
    StringLiteral(String),

    /// Identifier — letters, digits, underscores; must start with letter or `_`.
    ///
    /// V0.1 supports ASCII identifiers only. Unicode (Vietnamese diacritics)
    /// support is deferred to v0.2 — see SPEC §1.3.
    #[regex(r"[a-zA-Z][a-zA-Z0-9_]*", |lex| lex.slice().to_owned())]
    Identifier(String),

    // === F-string mode tokens (emitted by the [`crate::Lexer`] driver, not by logos) ===
    //
    // The wrapper Lexer drives a stack-based mode machine: after seeing
    // `f"`, it switches to FString mode and emits these tokens directly,
    // bypassing logos. These variants intentionally have no `#[token]` /
    // `#[regex]` attribute — logos will never produce them, only the
    // wrapper does. See `crate::lexer` for the state machine.
    /// `f"` — start of an f-string literal.
    FStringStart,
    /// A run of literal text inside an f-string (escapes already processed).
    FStringText(String),
    /// `{` opening an interpolation expression inside an f-string.
    InterpolationStart,
    /// `}` closing the matching interpolation expression.
    InterpolationEnd,
    /// `"` ending an f-string literal.
    FStringEnd,
}

// ============================================================================
// Callbacks
// ============================================================================

fn lex_decimal_integer(lex: &mut Lexer<'_, Token>) -> Result<IntLiteral, LexError> {
    let slice = lex.slice();
    let span = lex.span();

    // Split off optional suffix (last `_<word>` if it matches a known suffix).
    let (digits, suffix) = match split_suffix(slice) {
        Some((digits, suffix)) => (digits, Some(suffix)),
        None => (slice, None),
    };

    let cleaned: String = digits.chars().filter(|c| *c != '_').collect();
    let value: i128 = cleaned
        .parse()
        .map_err(|_| LexError::NumericOverflow { span })?;

    Ok(IntLiteral { value, suffix })
}

fn split_suffix(slice: &str) -> Option<(&str, NumericSuffix)> {
    // Find last `_` and check whether the tail is a known suffix word.
    let last_underscore = slice.rfind('_')?;
    let (head, tail) = slice.split_at(last_underscore);
    let suffix_word = &tail[1..]; // strip the underscore
    NumericSuffix::parse(suffix_word).map(|suffix| (head, suffix))
}

fn lex_ternary_integer(lex: &mut Lexer<'_, Token>) -> Result<IntLiteral, LexError> {
    let slice = lex.slice();
    let span = lex.span();
    let body = &slice[2..]; // strip `0t`

    let mut value: i128 = 0;
    for (offset, character) in body.char_indices() {
        let trit_value = match character {
            '+' => 1_i128,
            '0' => 0,
            '-' => -1,
            '_' => continue,
            other => {
                return Err(LexError::InvalidTernaryDigit {
                    span: (span.start + 2 + offset)..(span.start + 2 + offset + other.len_utf8()),
                    character: other,
                });
            }
        };
        value = value
            .checked_mul(3)
            .and_then(|v| v.checked_add(trit_value))
            .ok_or_else(|| LexError::NumericOverflow { span: span.clone() })?;
    }

    Ok(IntLiteral {
        value,
        suffix: None,
    })
}

fn lex_string_literal(lex: &mut Lexer<'_, Token>) -> Result<String, LexError> {
    let slice = lex.slice();
    let span = lex.span();
    // Strip surrounding quotes
    let body = &slice[1..slice.len() - 1];
    process_escapes(body, span.start + 1)
}

/// Process escape sequences inside a plain string body.
fn process_escapes(body: &str, body_start: usize) -> Result<String, LexError> {
    let mut output = String::with_capacity(body.len());
    let mut chars = body.char_indices();
    while let Some((index, character)) = chars.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        let (_, escape_char) = chars.next().ok_or_else(|| LexError::InvalidEscape {
            span: (body_start + index)..(body_start + index + 1),
            sequence: "\\".to_owned(),
        })?;
        match escape_char {
            'n' => output.push('\n'),
            't' => output.push('\t'),
            'r' => output.push('\r'),
            '\\' => output.push('\\'),
            '"' => output.push('"'),
            '0' => output.push('\0'),
            other => {
                return Err(LexError::InvalidEscape {
                    span: (body_start + index)..(body_start + index + 1 + other.len_utf8()),
                    sequence: format!("\\{other}"),
                });
            }
        }
    }
    Ok(output)
}
