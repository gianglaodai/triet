//! v0.7.4.4 — `lexer_differential` test (closes the v0.7.4 lexer
//! umbrella per [ADR-0019 §A7.4]).
//!
//! For each corpus source, runs the Rust impl [`triet_lexer::lex`] and
//! the Triết-in-Triết port at `compiler/lexer.tri::dump_ndjson` over
//! the same input. Both sides emit the same line-delimited JSON shape
//! (one token per line) and the test asserts byte-equality.
//!
//! Format (matches the in-source spec at the head of `compiler/lexer.tri`'s
//! NDJSON dump section):
//!
//! ```text
//! {"t":"<Kind>","s":[<start>,<end>]}                       // unit token
//! {"t":"<Kind>","s":[<start>,<end>],"v":"<text>"}          // text-bearing
//! {"t":"<Kind>","s":[<start>,<end>],"v":<value>}           // integer / ternary
//! {"t":"<Kind>","s":[<start>,<end>],"v":<value>,"u":"<s>"} // suffixed integer
//! ```
//!
//! On lex error a single line `{"e":"<Kind>","s":[...]}` replaces the
//! token stream.
//!
//! ## Char-indexed spans
//!
//! The Triết-side scanner uses `std.string.substring` / `std.string.index_of`,
//! which are char-indexed per Q3-A. The Rust impl produces byte spans.
//! [`byte_to_char_index`] builds a `byte → char` lookup table on the
//! source so the Rust mirror can translate every emitted span before
//! comparing, letting the corpus include real `.tri` files with UTF-8
//! comments (box-drawing characters, Vietnamese) — every shipped
//! example file qualifies.
//!
//! ## Transient bridge
//!
//! NDJSON is a transient bridge format per [ADR-0019 §A2] — dropped at
//! v0.7.9 when Triết-side data flows in-memory. It exists solely to
//! make a byte-diff a tractable gate while the bootstrap is incomplete.
//!
//! [ADR-0019 §A2]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md
//! [ADR-0019 §A7.4]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::OnceLock;

use miette::Diagnostic;
use triet_ir::{FuncId, IrProgram, RuntimeValue, Vm, lower_program, read_program, write_program};
use triet_lexer::{IntLiteral, LexError, NumericSuffix, Token, lex as rust_lex};
use triet_modules::load_program;
use triet_typecheck::check_resolved;

// ─────────────────────────────────────────────────────────────────
// Triết-side: compile `compiler/lexer.tri` once + run `dump_ndjson`
// ─────────────────────────────────────────────────────────────────

fn compiler_lexer_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("compiler")
        .join("lexer.tri")
}

/// Build `compiler/lexer.tri` once, including the `.triv` round-trip.
/// Subsequent calls clone from the cached `IrProgram` so each VM run
/// gets a fresh frame stack while the (slow) typecheck + lower work
/// runs only once per test binary.
fn lexer_ir() -> &'static IrProgram {
    static IR: OnceLock<IrProgram> = OnceLock::new();
    IR.get_or_init(|| {
        let path = compiler_lexer_path();
        assert!(
            path.is_file(),
            "missing compiler/lexer.tri at {}",
            path.display()
        );
        let resolved = load_program(&path).expect("load_program");
        let diagnostics = check_resolved(&resolved);
        let blocking: Vec<_> = diagnostics
            .iter()
            .filter(|err| err.severity() != Some(miette::Severity::Warning))
            .collect();
        assert!(
            blocking.is_empty(),
            "type errors in compiler/lexer.tri: {blocking:#?}",
        );
        let ir = lower_program(&resolved);
        // Round-trip through `.triv` so the bytecode the differential
        // exercises matches what `lexer_self_smoke.rs` covers.
        let bytes = write_program(&ir);
        read_program(&bytes).expect("read .triv round-trip")
    })
}

fn lookup_func(ir: &IrProgram, name: &str) -> FuncId {
    ir.modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing function `{name}` in compiler/lexer.tri"))
        .id
}

/// Drive the Triết-side `dump_ndjson(source) -> String` and return the
/// emitted bytes.
fn triet_dump(source: &str) -> String {
    let ir = lexer_ir().clone();
    let func_id = lookup_func(&ir, "dump_ndjson");
    let mut vm = Vm::new(ir);
    let result = vm
        .execute(func_id, vec![RuntimeValue::String(source.to_owned())])
        .expect("compiler/lexer.tri::dump_ndjson must execute without VM error");
    match result {
        RuntimeValue::String(s) => s,
        other => panic!("expected String from dump_ndjson, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────
// Rust-side mirror: same NDJSON output as the Triết-side, with byte
// spans translated to char spans so a corpus with UTF-8 comments works
// ─────────────────────────────────────────────────────────────────

/// `result[b]` = char index of the char whose UTF-8 encoding contains
/// byte `b`. `result[source.len()]` = total char count. Inside a
/// multi-byte sequence every byte maps to the same char index as the
/// sequence's leading byte.
fn byte_to_char_index(source: &str) -> Vec<usize> {
    let mut idx = vec![0_usize; source.len() + 1];
    let mut char_count = 0_usize;
    let mut last = 0_usize;
    for (byte_pos, slot) in idx.iter_mut().enumerate().take(source.len()) {
        if source.is_char_boundary(byte_pos) {
            last = char_count;
            char_count += 1;
        }
        *slot = last;
    }
    idx[source.len()] = char_count;
    idx
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

const fn suffix_name(s: NumericSuffix) -> &'static str {
    match s {
        NumericSuffix::Trit => "trit",
        NumericSuffix::Tryte => "tryte",
        NumericSuffix::Integer => "integer",
        NumericSuffix::Long => "long",
    }
}

const fn kind_name(token: &Token) -> &'static str {
    match token {
        Token::Function => "Function",
        Token::Let => "Let",
        Token::Mutable => "Mutable",
        Token::Constant => "Constant",
        Token::Type => "Type",
        Token::If => "If",
        Token::IfQ => "IfQ",
        Token::Else => "Else",
        Token::Match => "Match",
        Token::Return => "Return",
        Token::For => "For",
        Token::While => "While",
        Token::WhileQ => "WhileQ",
        Token::Loop => "Loop",
        Token::Break => "Break",
        Token::Continue => "Continue",
        Token::In => "In",
        Token::True => "True",
        Token::False => "False",
        Token::Unknown => "Unknown",
        Token::Null => "Null",
        Token::Not => "Not",
        Token::And => "And",
        Token::Or => "Or",
        Token::Xor => "Xor",
        Token::Iff => "Iff",
        Token::Implies => "Implies",
        Token::KleeneImplies => "KleeneImplies",
        Token::KleeneXor => "KleeneXor",
        Token::KleeneIff => "KleeneIff",
        Token::Import => "Import",
        Token::From => "From",
        Token::As => "As",
        Token::Module => "Module",
        Token::Public => "Public",
        Token::Owned => "Owned",
        Token::Struct => "Struct",
        Token::Enum => "Enum",
        Token::Crate => "Crate",
        Token::SelfKw => "SelfKw",
        Token::Super => "Super",
        Token::LtEqGt => "LtEqGt",
        Token::LtTildeGt => "LtTildeGt",
        Token::DotDotEq => "DotDotEq",
        Token::DotDot => "DotDot",
        Token::EqEq => "EqEq",
        Token::NotEq => "NotEq",
        Token::LtEq => "LtEq",
        Token::GtEq => "GtEq",
        Token::AndAnd => "AndAnd",
        Token::OrOr => "OrOr",
        Token::FatArrow => "FatArrow",
        Token::TildeArrow => "TildeArrow",
        Token::TildeCaret => "TildeCaret",
        Token::QuestionTilde => "QuestionTilde",
        Token::TildePlus => "TildePlus",
        Token::TildeMinus => "TildeMinus",
        Token::TildeZero => "TildeZero",
        Token::TildeQuestion => "TildeQuestion",
        Token::TildeColon => "TildeColon",
        Token::Tilde => "Tilde",
        Token::ThinArrow => "ThinArrow",
        Token::QuestionDot => "QuestionDot",
        Token::QuestionColon => "QuestionColon",
        Token::BangBang => "BangBang",
        Token::StarStar => "StarStar",
        Token::Plus => "Plus",
        Token::Minus => "Minus",
        Token::Star => "Star",
        Token::Slash => "Slash",
        Token::PercentPercent => "PercentPercent",
        Token::Assign => "Assign",
        Token::Lt => "Lt",
        Token::Gt => "Gt",
        Token::Bang => "Bang",
        Token::Caret => "Caret",
        Token::Question => "Question",
        Token::Colon => "Colon",
        Token::Semi => "Semi",
        Token::Comma => "Comma",
        Token::Dot => "Dot",
        Token::LBrace => "LBrace",
        Token::RBrace => "RBrace",
        Token::LBracket => "LBracket",
        Token::RBracket => "RBracket",
        Token::LParen => "LParen",
        Token::RParen => "RParen",
        Token::Pipe => "Pipe",
        Token::Underscore => "Underscore",
        Token::IntegerLiteral(_) => "IntegerLiteral",
        Token::TernaryLiteral(_) => "TernaryLiteral",
        Token::StringLiteral(_) => "StringLiteral",
        Token::Identifier(_) => "Identifier",
        Token::FStringStart => "FStringStart",
        Token::FStringText(_) => "FStringText",
        Token::InterpolationStart => "InterpolationStart",
        Token::InterpolationEnd => "InterpolationEnd",
        Token::FStringEnd => "FStringEnd",
    }
}

fn write_token_line(out: &mut String, token: &Token, start: usize, end: usize) {
    let kind = kind_name(token);
    let _ = write!(out, "{{\"t\":\"{kind}\",\"s\":[{start},{end}]");
    match token {
        Token::IntegerLiteral(IntLiteral { value, suffix })
        | Token::TernaryLiteral(IntLiteral { value, suffix }) => {
            let _ = write!(out, ",\"v\":{value}");
            if let Some(suf) = suffix {
                let _ = write!(out, ",\"u\":\"{}\"", suffix_name(*suf));
            }
        }
        Token::StringLiteral(text) | Token::Identifier(text) | Token::FStringText(text) => {
            let _ = write!(out, ",\"v\":\"{}\"", json_escape(text));
        }
        _ => {}
    }
    out.push_str("}\n");
}

fn write_error_line(out: &mut String, err: &LexError, byte_to_char: &[usize]) {
    fn span_pair(byte_to_char: &[usize], s: std::ops::Range<usize>) -> (usize, usize) {
        // Span endpoints can legitimately land on byte indices past the
        // last character (e.g. at EOF); the lookup table is sized
        // `source.len() + 1` so end-of-source resolves cleanly.
        (byte_to_char[s.start], byte_to_char[s.end])
    }
    match err {
        LexError::Unrecognized => {
            // Driver never surfaces this — `lex()` translates it to
            // `UnexpectedCharacter`. Kept for completeness.
            out.push_str("{\"e\":\"Unrecognized\",\"s\":[0,0]}\n");
        }
        LexError::UnexpectedCharacter { span, snippet } => {
            let (s, e) = span_pair(byte_to_char, span.clone());
            let _ = writeln!(
                out,
                "{{\"e\":\"UnexpectedCharacter\",\"s\":[{s},{e}],\"v\":\"{}\"}}",
                json_escape(snippet)
            );
        }
        LexError::NumericOverflow { span } => {
            let (s, e) = span_pair(byte_to_char, span.clone());
            let _ = writeln!(out, "{{\"e\":\"NumericOverflow\",\"s\":[{s},{e}]}}");
        }
        LexError::InvalidTernaryDigit { span, character } => {
            let (s, e) = span_pair(byte_to_char, span.clone());
            let mut ch = String::new();
            ch.push(*character);
            let _ = writeln!(
                out,
                "{{\"e\":\"InvalidTernaryDigit\",\"s\":[{s},{e}],\"v\":\"{}\"}}",
                json_escape(&ch)
            );
        }
        LexError::UnterminatedBlockComment { span } => {
            let (s, e) = span_pair(byte_to_char, span.clone());
            let _ = writeln!(
                out,
                "{{\"e\":\"UnterminatedBlockComment\",\"s\":[{s},{e}]}}"
            );
        }
        LexError::UnterminatedString { span } => {
            let (s, e) = span_pair(byte_to_char, span.clone());
            let _ = writeln!(out, "{{\"e\":\"UnterminatedString\",\"s\":[{s},{e}]}}");
        }
        LexError::InvalidEscape { span, sequence } => {
            let (s, e) = span_pair(byte_to_char, span.clone());
            let _ = writeln!(
                out,
                "{{\"e\":\"InvalidEscape\",\"s\":[{s},{e}],\"v\":\"{}\"}}",
                json_escape(sequence)
            );
        }
        LexError::UnmatchedFStringBrace { span } => {
            let (s, e) = span_pair(byte_to_char, span.clone());
            let _ = writeln!(out, "{{\"e\":\"UnmatchedFStringBrace\",\"s\":[{s},{e}]}}");
        }
    }
}

fn rust_dump(source: &str) -> String {
    let byte_to_char = byte_to_char_index(source);
    let mut out = String::new();
    match rust_lex(source) {
        Ok(tokens) => {
            for (token, span) in tokens {
                let start = byte_to_char[span.start];
                let end = byte_to_char[span.end];
                write_token_line(&mut out, &token, start, end);
            }
        }
        Err(err) => write_error_line(&mut out, &err, &byte_to_char),
    }
    out
}

// ─────────────────────────────────────────────────────────────────
// Differential driver
// ─────────────────────────────────────────────────────────────────

/// Run both impls on `source` and assert byte-equality. On divergence
/// the panic message prints the first ~400 chars of each side along
/// with the diverging byte offset to make root-cause diagnosis fast
/// (per ADR-0019 §4 "binary search for diverging offset" debug path).
fn assert_equal(label: &str, source: &str) {
    let rust = rust_dump(source);
    let triet = triet_dump(source);
    if rust == triet {
        return;
    }
    let first_diff = rust
        .bytes()
        .zip(triet.bytes())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| rust.len().min(triet.len()));
    panic!(
        "lexer differential mismatch for `{label}`\n\
         Rust length:  {}\n\
         Triết length: {}\n\
         first diverging byte: {first_diff}\n\
         ── Rust output (truncated) ──\n{}\n\
         ── Triết output (truncated) ──\n{}",
        rust.len(),
        triet.len(),
        rust.chars().take(400).collect::<String>(),
        triet.chars().take(400).collect::<String>(),
    );
}

// ─────────────────────────────────────────────────────────────────
// Corpus
// ─────────────────────────────────────────────────────────────────
//
// Each fixture exercises a specific class of token shape. Tests
// covering example files at the end exercise real-world UTF-8 sources
// to gate the byte-→-char translation. lexer.tri itself is skipped
// from the corpus on purpose — running the Triết-side scanner over
// its own ~1100 LOC body via the VM costs minutes per test and the
// smaller fixtures already cover every token shape it contains.

#[test]
fn keywords_and_simple_bindings() {
    assert_equal("let_x_42", "let x = 42");
}

#[test]
fn single_char_operators_and_punctuation() {
    assert_equal("ops_simple", "+ - * / ( ) [ ] { } : ; , . | _");
}

#[test]
fn compound_operators_longest_match() {
    assert_equal(
        "ops_compound",
        "== != <= >= && || => -> <=> <~> ..= .. ** %%",
    );
}

#[test]
fn outcome_compound_tokens() {
    assert_equal("ops_outcome", "~+ ~- ~0 ~? ~: ?~ ~> ~^ ~");
}

#[test]
fn nullable_and_force_unwrap_operators() {
    assert_equal("ops_nullable", "? ?. ?: !!");
}

#[test]
fn ternary_literals_balanced() {
    assert_equal("ternary_lit", "0t+0-+ 0t--- 0t+++ 0t0 0t+_0_-_+");
}

#[test]
fn decimal_integer_literals_with_suffixes() {
    assert_equal(
        "typed_int",
        "5_tryte 1_000_long 42_integer 1_trit 1_000_000 1_xyz",
    );
}

#[test]
fn string_literals_and_escapes() {
    assert_equal(
        "string_lit",
        r#""hello" "she said \"hi\"" "line1\nline2\t\\done" """#,
    );
}

#[test]
fn fstring_basic_interpolation() {
    assert_equal("fstring_simple", r#"f"hello {name}""#);
}

#[test]
fn fstring_with_multiple_interpolations_and_text() {
    assert_equal("fstring_multi", r#"f"sum {a} + {b} = {c}""#);
}

#[test]
fn fstring_double_brace_escape() {
    assert_equal("fstring_dbrace", r#"f"{{ literal }}""#);
}

#[test]
fn line_comments_are_skipped() {
    assert_equal(
        "line_comments",
        "// preface\nlet x = 1 // trailing\nlet y = 2\n",
    );
}

#[test]
fn keywords_question_modified() {
    assert_equal("question_kw", "if? while? if while");
}

#[test]
fn all_logic_keywords_and_symbolic_forms() {
    assert_equal(
        "logic_mix",
        "a and b or not c xor d iff e implies f kleene_implies g kleene_xor h kleene_iff i && j || !k ^ l => m ~> n ~^ o <=> p <~> q",
    );
}

#[test]
fn path_keywords_stay_distinct_from_identifiers() {
    assert_equal("path_kw", "crate self super crater selfish supermassive");
}

#[test]
fn realistic_function_signature() {
    assert_equal(
        "fizz_sig",
        "function fizzbuzz(n: Integer) -> String = match (n %% 3, n %% 5) {\n    (0, 0) => \"FizzBuzz\",\n    _      => to_string(n),\n}",
    );
}

#[test]
fn outcome_type_annotations() {
    assert_equal(
        "outcome_types",
        "let x: Integer~Error = ~+ 5\nlet y: Integer?~Error = ~0\nlet z: Integer? = ~0",
    );
}

#[test]
fn example_factorial_matches_byte_for_byte() {
    let path = example_path("factorial");
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert_equal("example/factorial", &source);
}

#[test]
fn example_maybe_matches_byte_for_byte() {
    let path = example_path("maybe");
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert_equal("example/maybe", &source);
}

#[test]
fn example_nullable_matches_byte_for_byte() {
    let path = example_path("nullable");
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert_equal("example/nullable", &source);
}

fn example_path(name: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("examples")
        .join(format!("{name}.tri"))
}
