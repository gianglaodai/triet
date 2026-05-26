//! Triết lexer: tokenize source code into a stream of tokens.
//!
//! Supports ternary literals (`0t+0-+`), type-suffixed numbers
//! (`5_tryte`, `1_long`), Trilean keywords, identifiers, and the full
//! operator set defined in SPEC.md §1. F-strings are tokenized via a
//! mode stack — see [`Lexer`] for details.
//!
//! # Example
//!
//! ```
//! use triet_lexer::{lex, Token};
//!
//! let tokens = lex("let x = 42").unwrap();
//! assert!(matches!(tokens[0].0, Token::Let));
//! assert!(matches!(tokens[1].0, Token::Identifier(_)));
//! ```

#![warn(missing_docs)]

mod error;
mod lexer;
mod token;

pub use error::{LexError, Span};
pub use lexer::{Lexer, SpannedToken, lex};
pub use token::{IntLiteral, NumericSuffix, Token};

#[cfg(test)]
mod tests {
    use super::*;
    use Token::{
        Ampersand, AmpersandMinus, AmpersandPlus, AmpersandZero, And, AndAnd, As, Assign,
        Bang, BangBang, Break, Caret, Colon, Comma, Constant, Continue, Dot, DotDot, DotDotEq, Else,
        EqEq, FStringEnd, FStringStart, FStringText, False, FatArrow, For, From, Function, GtEq,
        Identifier, If, IfQ, Iff, Implies, In, IntegerLiteral, InterpolationEnd,
        InterpolationStart, KleeneIff, KleeneImplies, KleeneXor, LBrace, LBracket, LParen, Let,
        Loop, Lt, LtEq, LtEqGt, LtTildeGt, Match, Minus, Mutable, Not, NotEq, Null, Or, OrOr,
        Owned, PercentPercent, Pipe, Plus, Public, Question, QuestionColon, QuestionDot, RBrace,
        RBracket, RParen, Return, Semi, Slash, Star, StarStar, StringLiteral,
        TernaryLiteral, ThinArrow, TildeArrow, TildeCaret, TildeMinusGt, TildePlusGt, TildeZeroGt,
        True, Type, Underscore, Unknown, While, WhileQ, Xor,
    };

    fn lex_only(source: &str) -> Vec<Token> {
        lex(source).unwrap().into_iter().map(|(t, _)| t).collect()
    }

    // === Keywords ===

    #[test]
    fn lexes_all_keywords() {
        let source = "function let mutable constant type if else match return for while loop break \
                      continue in true false unknown null not and or xor iff implies \
                      kleene_implies kleene_xor kleene_iff import from as module public owned \
                      struct enum khi self super";
        let tokens = lex_only(source);
        assert_eq!(
            tokens,
            vec![
                Function,
                Let,
                Mutable,
                Constant,
                Type,
                If,
                Else,
                Match,
                Return,
                For,
                While,
                Loop,
                Break,
                Continue,
                In,
                True,
                False,
                Unknown,
                Null,
                Not,
                And,
                Or,
                Xor,
                Iff,
                Implies,
                KleeneImplies,
                KleeneXor,
                KleeneIff,
                Token::Import,
                From,
                As,
                Token::Module,
                Public,
                Owned,
                Token::Struct,
                Token::Enum,
                Token::Khi,
                Token::SelfKw,
                Token::Super,
            ],
        );
    }

    #[test]
    fn path_keywords_are_distinct_from_identifiers() {
        // `khi`/`self`/`super` are reserved path keywords
        // (ADR-0005 + ADR-0024 — `khi` replaces `crate`).
        assert_eq!(lex_only("khi"), vec![Token::Khi]);
        assert_eq!(lex_only("self"), vec![Token::SelfKw]);
        assert_eq!(lex_only("super"), vec![Token::Super]);
        // But identifiers that *contain* these as substrings stay
        // identifiers (no greedy keyword matching).
        assert!(matches!(
            lex_only("crater").as_slice(),
            [Token::Identifier(name)] if name == "crater"
        ));
        assert!(matches!(
            lex_only("selfish").as_slice(),
            [Token::Identifier(name)] if name == "selfish"
        ));
    }

    #[test]
    fn lexes_question_modified_keywords() {
        assert_eq!(lex_only("if?"), vec![IfQ]);
        assert_eq!(lex_only("while?"), vec![WhileQ]);
    }

    #[test]
    fn keyword_is_distinct_from_identifier_starting_with_same_chars() {
        let tokens = lex_only("iffy");
        match tokens.as_slice() {
            [Identifier(name)] if name == "iffy" => {}
            other => panic!("expected single identifier 'iffy', got {other:?}"),
        }
    }

    // === Operators ===

    #[test]
    fn lexes_arithmetic_operators() {
        assert_eq!(
            lex_only("+ - * / %% **"),
            vec![Plus, Minus, Star, Slash, PercentPercent, StarStar],
        );
    }

    #[test]
    fn star_star_beats_star_via_longest_match() {
        assert_eq!(lex_only("**"), vec![StarStar]);
        assert_eq!(lex_only("* *"), vec![Star, Star]);
        assert_eq!(
            lex_only("a ** b"),
            vec![
                Identifier("a".to_owned()),
                StarStar,
                Identifier("b".to_owned()),
            ]
        );
    }

    #[test]
    fn lexes_comparison_operators() {
        assert_eq!(
            lex_only("< > <= >= == !="),
            vec![Lt, Token::Gt, LtEq, GtEq, EqEq, NotEq],
        );
    }

    #[test]
    fn lexes_logic_operators() {
        assert_eq!(
            lex_only("&& || ! ^ ~^"),
            vec![AndAnd, OrOr, Bang, Caret, TildeCaret],
        );
    }

    #[test]
    fn lexes_implication_operators() {
        assert_eq!(
            lex_only("=> ~> <=> <~>"),
            vec![FatArrow, TildeArrow, LtEqGt, LtTildeGt],
        );
    }

    #[test]
    fn lexes_nullable_operators() {
        assert_eq!(
            lex_only("? ?. ?: !!"),
            vec![Question, QuestionDot, QuestionColon, BangBang],
        );
    }

    #[test]
    fn lexes_function_arrow_distinct_from_minus_gt() {
        assert_eq!(lex_only("->"), vec![ThinArrow]);
        assert_eq!(lex_only("- >"), vec![Minus, Token::Gt]);
    }

    #[test]
    fn lexes_assignment_distinct_from_eq_eq_and_fat_arrow() {
        assert_eq!(lex_only("="), vec![Assign]);
        assert_eq!(lex_only("=="), vec![EqEq]);
        assert_eq!(lex_only("=>"), vec![FatArrow]);
    }

    #[test]
    fn lexes_range_operators_at_correct_length() {
        assert_eq!(lex_only(".."), vec![DotDot]);
        assert_eq!(lex_only("..="), vec![DotDotEq]);
        assert_eq!(lex_only("."), vec![Dot]);
        assert_eq!(
            lex_only("0..100"),
            vec![
                IntegerLiteral(IntLiteral {
                    value: 0,
                    suffix: None
                }),
                DotDot,
                IntegerLiteral(IntLiteral {
                    value: 100,
                    suffix: None
                }),
            ]
        );
    }

    // === Punctuation ===

    #[test]
    fn lexes_punctuation() {
        assert_eq!(
            lex_only("{ } [ ] ( ) : ; , . | _"),
            vec![
                LBrace, RBrace, LBracket, RBracket, LParen, RParen, Colon, Semi, Comma, Dot, Pipe,
                Underscore,
            ],
        );
    }

    // === Integer literals ===

    #[test]
    fn lexes_plain_integer() {
        assert_eq!(
            lex_only("42"),
            vec![IntegerLiteral(IntLiteral {
                value: 42,
                suffix: None
            })],
        );
    }

    #[test]
    fn lexes_integer_with_underscores() {
        assert_eq!(
            lex_only("1_000_000"),
            vec![IntegerLiteral(IntLiteral {
                value: 1_000_000,
                suffix: None
            })],
        );
    }

    #[test]
    fn lexes_integer_with_each_suffix() {
        assert_eq!(
            lex_only("1_trit 5_tryte 42_integer 1000_long"),
            vec![
                IntegerLiteral(IntLiteral {
                    value: 1,
                    suffix: Some(NumericSuffix::Trit)
                }),
                IntegerLiteral(IntLiteral {
                    value: 5,
                    suffix: Some(NumericSuffix::Tryte)
                }),
                IntegerLiteral(IntLiteral {
                    value: 42,
                    suffix: Some(NumericSuffix::Integer)
                }),
                IntegerLiteral(IntLiteral {
                    value: 1000,
                    suffix: Some(NumericSuffix::Long)
                }),
            ],
        );
    }

    #[test]
    fn lexes_integer_with_underscores_and_suffix() {
        assert_eq!(
            lex_only("1_000_long"),
            vec![IntegerLiteral(IntLiteral {
                value: 1_000,
                suffix: Some(NumericSuffix::Long),
            })],
        );
    }

    #[test]
    fn integer_with_unknown_suffix_does_not_consume_trailing_chars() {
        let tokens = lex_only("1_xyz");
        assert_eq!(
            tokens,
            vec![
                IntegerLiteral(IntLiteral {
                    value: 1,
                    suffix: None
                }),
                Underscore,
                Identifier("xyz".to_owned()),
            ],
        );
    }

    // === Ternary literals ===

    #[test]
    fn lexes_ternary_literal_basic() {
        assert_eq!(
            lex_only("0t+0-+"),
            vec![TernaryLiteral(IntLiteral {
                value: 25,
                suffix: None
            })],
        );
    }

    #[test]
    fn lexes_ternary_zero() {
        assert_eq!(
            lex_only("0t0"),
            vec![TernaryLiteral(IntLiteral {
                value: 0,
                suffix: None
            })],
        );
    }

    #[test]
    fn lexes_ternary_positive_only() {
        assert_eq!(
            lex_only("0t+++"),
            vec![TernaryLiteral(IntLiteral {
                value: 13,
                suffix: None
            })],
        );
    }

    #[test]
    fn lexes_ternary_negative_only() {
        assert_eq!(
            lex_only("0t---"),
            vec![TernaryLiteral(IntLiteral {
                value: -13,
                suffix: None
            })],
        );
    }

    #[test]
    fn lexes_ternary_with_underscores() {
        assert_eq!(
            lex_only("0t+_0_-_+"),
            vec![TernaryLiteral(IntLiteral {
                value: 25,
                suffix: None
            })],
        );
    }

    // === String literals ===

    #[test]
    fn lexes_simple_string() {
        assert_eq!(
            lex_only(r#""hello""#),
            vec![StringLiteral("hello".to_owned())]
        );
    }

    #[test]
    fn lexes_string_with_escape_sequences() {
        assert_eq!(
            lex_only(r#""line1\nline2\t\\done""#),
            vec![StringLiteral("line1\nline2\t\\done".to_owned())],
        );
    }

    #[test]
    fn lexes_string_with_escaped_quote() {
        assert_eq!(
            lex_only(r#""she said \"hi\"""#),
            vec![StringLiteral(r#"she said "hi""#.to_owned())],
        );
    }

    #[test]
    fn lexes_empty_string() {
        assert_eq!(lex_only(r#""""#), vec![StringLiteral(String::new())]);
    }

    #[test]
    fn rejects_invalid_escape() {
        let result = lex(r#""\q""#);
        assert!(matches!(result, Err(LexError::InvalidEscape { .. })));
    }

    #[test]
    fn rejects_unterminated_string() {
        let result = lex(r#""hello"#);
        assert!(result.is_err());
    }

    // === F-strings (mode stack) ===

    #[test]
    fn lexes_f_string_with_single_interpolation() {
        let tokens = lex_only(r#"f"hello {name}""#);
        assert_eq!(
            tokens,
            vec![
                FStringStart,
                FStringText("hello ".to_owned()),
                InterpolationStart,
                Identifier("name".to_owned()),
                InterpolationEnd,
                FStringEnd,
            ],
        );
    }

    #[test]
    fn lexes_f_string_with_no_interpolation_and_no_text() {
        let tokens = lex_only(r#"f"""#);
        assert_eq!(tokens, vec![FStringStart, FStringEnd]);
    }

    #[test]
    fn lexes_f_string_with_only_text() {
        let tokens = lex_only(r#"f"hello""#);
        assert_eq!(
            tokens,
            vec![FStringStart, FStringText("hello".to_owned()), FStringEnd],
        );
    }

    #[test]
    fn lexes_f_string_with_only_interpolation_no_surrounding_text() {
        let tokens = lex_only(r#"f"{x}""#);
        assert_eq!(
            tokens,
            vec![
                FStringStart,
                InterpolationStart,
                Identifier("x".to_owned()),
                InterpolationEnd,
                FStringEnd,
            ],
        );
    }

    #[test]
    fn lexes_f_string_with_multiple_interpolations() {
        let tokens = lex_only(r#"f"{a} + {b} = {c}""#);
        assert_eq!(
            tokens,
            vec![
                FStringStart,
                InterpolationStart,
                Identifier("a".to_owned()),
                InterpolationEnd,
                FStringText(" + ".to_owned()),
                InterpolationStart,
                Identifier("b".to_owned()),
                InterpolationEnd,
                FStringText(" = ".to_owned()),
                InterpolationStart,
                Identifier("c".to_owned()),
                InterpolationEnd,
                FStringEnd,
            ],
        );
    }

    #[test]
    fn f_string_text_handles_escape_sequences() {
        let tokens = lex_only(r#"f"line1\nline2""#);
        assert_eq!(
            tokens,
            vec![
                FStringStart,
                FStringText("line1\nline2".to_owned()),
                FStringEnd,
            ],
        );
    }

    #[test]
    fn f_string_double_brace_is_literal_brace() {
        let tokens = lex_only(r#"f"{{ literal }}""#);
        assert_eq!(
            tokens,
            vec![
                FStringStart,
                FStringText("{ literal }".to_owned()),
                FStringEnd,
            ],
        );
    }

    #[test]
    fn f_string_can_contain_arbitrary_expression() {
        // Critical: nested block `if true { 1 } else { 0 }` inside
        // interpolation. Brace counter must let inner `{` and `}` pass
        // through without closing the interpolation prematurely.
        let tokens = lex_only(r#"f"r = { if x { 1 } else { 0 } }""#);
        // Expect: FStringStart, FStringText("r = "), InterpolationStart,
        // If, Ident(x), LBrace, Int(1), RBrace, Else, LBrace, Int(0),
        // RBrace, InterpolationEnd, FStringEnd
        assert!(matches!(tokens[0], FStringStart));
        assert!(matches!(&tokens[1], FStringText(_)));
        assert!(matches!(tokens[2], InterpolationStart));
        assert!(tokens.contains(&LBrace));
        assert!(tokens.contains(&RBrace));
        assert!(matches!(tokens.last(), Some(FStringEnd)));
        // Crucially: we must reach FStringEnd, proving the brace tracker
        // did not consume an inner `}` as the interpolation terminator.
    }

    #[test]
    fn f_string_interpolation_can_contain_string_with_braces() {
        // String literal "}" inside interpolation must NOT close the
        // outer interpolation: logos atomically lexes the string.
        let tokens = lex_only(r#"f"x = { "}" }""#);
        assert!(matches!(tokens[0], FStringStart));
        assert!(matches!(tokens.last(), Some(FStringEnd)));
        // Verify the inner string literal made it through.
        let has_string = tokens
            .iter()
            .any(|t| matches!(t, StringLiteral(s) if s == "}"));
        assert!(has_string, "expected inner string \"}}\", got {tokens:?}");
    }

    #[test]
    fn rejects_unmatched_closing_brace_in_f_string_text() {
        // Single `}` in text without matching `{` is an error.
        let result = lex(r#"f"oops}""#);
        assert!(matches!(
            result,
            Err(LexError::UnmatchedFStringBrace { .. })
        ));
    }

    #[test]
    fn rejects_unterminated_f_string() {
        let result = lex(r#"f"hello"#);
        assert!(matches!(result, Err(LexError::UnterminatedString { .. })));
    }

    // === Identifiers ===

    #[test]
    fn lexes_simple_identifier() {
        assert_eq!(lex_only("foo"), vec![Identifier("foo".to_owned())]);
    }

    #[test]
    fn lexes_identifier_with_underscore_and_digits() {
        assert_eq!(
            lex_only("foo_bar_42"),
            vec![Identifier("foo_bar_42".to_owned())],
        );
    }

    #[test]
    fn lexes_pascal_case_type_names() {
        assert_eq!(lex_only("Trit"), vec![Identifier("Trit".to_owned())]);
        assert_eq!(lex_only("Integer"), vec![Identifier("Integer".to_owned())]);
        assert_eq!(lex_only("Trilean"), vec![Identifier("Trilean".to_owned())]);
    }

    #[test]
    fn underscore_alone_is_wildcard_not_identifier() {
        assert_eq!(lex_only("_"), vec![Underscore]);
    }

    // === Comments ===

    #[test]
    fn skips_line_comments() {
        let source = "let x = 5  // assign\nlet y = 7";
        let tokens = lex_only(source);
        assert_eq!(tokens.len(), 8);
    }

    #[test]
    fn skips_whitespace() {
        let source = "  let \t x  \n=  5  ";
        let tokens = lex_only(source);
        assert_eq!(tokens.len(), 4);
    }

    // === Spans ===

    #[test]
    fn tracks_byte_spans() {
        let source = "function add";
        let tokens = lex(source).unwrap();
        assert_eq!(tokens[0].1, 0..8);
        assert_eq!(tokens[1].1, 9..12);
    }

    #[test]
    fn f_string_spans_are_absolute() {
        let source = r#"  f"hi {x}""#;
        //              ^ ^  ^^   ^^
        //              0 2  3 7  9 10
        let tokens = lex(source).unwrap();
        assert_eq!(tokens[0].0, FStringStart);
        assert_eq!(tokens[0].1, 2..4);
        assert!(matches!(&tokens[1].0, FStringText(s) if s == "hi "));
        assert_eq!(tokens[1].1, 4..7);
        assert_eq!(tokens[2].0, InterpolationStart);
        assert_eq!(tokens[2].1, 7..8);
        assert!(matches!(&tokens[3].0, Identifier(s) if s == "x"));
        assert_eq!(tokens[3].1, 8..9);
        assert_eq!(tokens[4].0, InterpolationEnd);
        assert_eq!(tokens[4].1, 9..10);
        assert_eq!(tokens[5].0, FStringEnd);
        assert_eq!(tokens[5].1, 10..11);
    }

    // === Realistic samples ===

    #[test]
    fn lexes_fizzbuzz_signature() {
        let source = "function fizzbuzz(n: Integer) -> String =";
        let tokens = lex_only(source);
        assert_eq!(
            tokens,
            vec![
                Function,
                Identifier("fizzbuzz".to_owned()),
                LParen,
                Identifier("n".to_owned()),
                Colon,
                Identifier("Integer".to_owned()),
                RParen,
                ThinArrow,
                Identifier("String".to_owned()),
                Assign,
            ],
        );
    }

    #[test]
    fn lexes_match_arm_with_tuple_pattern() {
        let source = "(0, 0) => \"FizzBuzz\",";
        let tokens = lex_only(source);
        assert_eq!(
            tokens,
            vec![
                LParen,
                IntegerLiteral(IntLiteral {
                    value: 0,
                    suffix: None
                }),
                Comma,
                IntegerLiteral(IntLiteral {
                    value: 0,
                    suffix: None
                }),
                RParen,
                FatArrow,
                StringLiteral("FizzBuzz".to_owned()),
                Comma,
            ],
        );
    }

    #[test]
    fn lexes_logic_expression_with_keywords_and_symbols() {
        let source = "fever and rash and not vaccinated";
        let tokens = lex_only(source);
        assert_eq!(
            tokens,
            vec![
                Identifier("fever".to_owned()),
                And,
                Identifier("rash".to_owned()),
                And,
                Not,
                Identifier("vaccinated".to_owned()),
            ],
        );
    }

    #[test]
    fn lexes_kleene_implication() {
        assert_eq!(
            lex_only("a kleene_implies b"),
            vec![
                Identifier("a".to_owned()),
                KleeneImplies,
                Identifier("b".to_owned()),
            ],
        );
    }

    // ── v0.8 ownership tokens ────────────────────────────────────────

    #[test]
    fn lexes_ownership_compound_tokens() {
        assert_eq!(
            lex_only("&+ &0 &-"),
            vec![AmpersandPlus, AmpersandZero, AmpersandMinus],
        );
    }

    #[test]
    fn ownership_compound_longest_match_over_bare_ampersand() {
        // `&+` must lex as compound AmpersandPlus, not Ampersand + Plus.
        assert_eq!(
            lex_only("&+ &-"),
            vec![AmpersandPlus, AmpersandMinus],
        );
    }

    #[test]
    fn lexes_bare_ampersand_with_space() {
        // `& x` (with whitespace) → Ampersand, Identifier — not compound.
        let tokens = lex_only("& x");
        assert_eq!(tokens.len(), 2, "expected Ampersand + Identifier, got {tokens:?}");
        assert!(matches!(tokens[0], Ampersand));
    }

    #[test]
    fn lexes_ownership_in_type_expr() {
        // `&+ mutable T` — realistic ownership type expression.
        assert_eq!(
            lex_only("&+ mutable T"),
            vec![
                AmpersandPlus,
                Mutable,
                Identifier("T".to_owned()),
            ],
        );
    }

    #[test]
    fn ampersand_minus_preserved_over_and_and() {
        // `&-` (2 chars) vs `&&` (also 2 chars). Logos resolves ties by
        // declaration order — `&-` is defined before `&&` and must win.
        assert_eq!(lex_only("&-"), vec![AmpersandMinus]);
        assert_eq!(lex_only("&&"), vec![AndAnd]);
    }

    #[test]
    fn lexes_strong_owner_fully_specified() {
        // Realistic struct field declaration: `name: &+ mutable String`
        let tokens = lex_only("&+ mutable String");
        assert_eq!(tokens.len(), 3);
        assert!(matches!(tokens[0], AmpersandPlus));
        assert!(matches!(tokens[1], Mutable));
        assert!(matches!(tokens[2], Identifier(_)));
    }

    #[test]
    fn lexes_weak_observer_in_struct_field() {
        // `parent: &- Process`
        assert_eq!(
            lex_only("&- Process"),
            vec![AmpersandMinus, Identifier("Process".to_owned())],
        );
    }

    #[test]
    fn lexes_neutral_borrow_in_function_param() {
        // `param: &0 T` — neutral borrow.
        assert_eq!(
            lex_only("&0 T"),
            vec![AmpersandZero, Identifier("T".to_owned())],
        );
    }

    #[test]
    fn all_five_reference_forms_parse_separately() {
        // Each of the 5 forms must lex correctly without ambiguity.
        let cases: &[(&str, Token)] = &[
            ("&+", AmpersandPlus),
            ("&0", AmpersandZero),
            ("&-", AmpersandMinus),
        ];
        for (input, expected) in cases {
            let tokens = lex_only(input);
            assert_eq!(tokens.len(), 1, "input {input:?}");
            assert_eq!(tokens[0], *expected, "input {input:?}");
        }
    }

    #[test]
    fn mutable_keyword_lexes_as_token_not_identifier() {
        let tokens = lex_only("mutable");
        assert_eq!(tokens, vec![Mutable]);
    }

    // v0.8 actor keywords removed per BYOS philosophy (ADR-0026 v2).
    // `actor`/`receive`/`send`/`spawn` are now stdlib functions, not
    // language keywords. Lexer should treat them as plain identifiers.
    #[test]
    fn actor_words_lex_as_identifiers_under_byos() {
        for kw in &["actor", "receive", "send", "spawn"] {
            let tokens = lex_only(kw);
            assert_eq!(tokens.len(), 1, "{kw:?}");
            assert!(matches!(&tokens[0], Identifier(_)), "{kw:?} should be Identifier");
        }
    }
}
