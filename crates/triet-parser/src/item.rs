//! Top-level item parser — `function`, `constant`, `type`, `use`.

use triet_lexer::{Span, Token};
use triet_syntax::{
    CapabilityLevel, EnumDefinition, EnumVariant, FunctionBody, FunctionDefinition,
    FunctionParameter, GenericBound, ImplementationDefinition, ImportName, ImportPath, Item,
    MethodSignature, ModuleContent, ModuleItem, ParameterPassing, Spanned, StructDefinition,
    StructField, TraitDefinition, TypeExpr, TypeParameter, Visibility,
};

use crate::{
    error::ParseError,
    expr::parse_expression,
    parser::Parser,
    stmt::{parse_assignment_body, parse_top_block},
    type_expr::parse_type,
};

/// Parse a top-level item.
///
/// Items may be prefixed with a visibility modifier (`public`, `public(package)`).
/// The captured [`Visibility`] is stored on the resulting AST node;
/// downstream passes (name resolver, ABI extractor) read it from there.
pub(crate) fn parse_item(parser: &mut Parser<'_>) -> Result<Spanned<Item>, ParseError> {
    // Capture span start before optional visibility prefix so the item's
    // overall span includes the visibility keyword.
    let head_span_start = parser
        .peek()
        .map_or_else(|| parser.eof_span().start, |(_, span)| span.start);

    let visibility = parse_visibility(parser)?;

    let Some((token, kw_span)) = parser.peek().cloned() else {
        let expected = if visibility == Visibility::Private {
            "item".to_owned()
        } else {
            format!("item after `{visibility}`")
        };
        return Err(ParseError::UnexpectedEof {
            expected,
            span: parser.eof_span(),
        });
    };

    let head_span = head_span_start..kw_span.end;

    match token {
        Token::Function => parse_function(parser, head_span, visibility),
        Token::Constant => parse_const_item(parser, head_span, visibility),
        Token::Type => parse_type_alias(parser, head_span, visibility),
        Token::Struct => parse_struct(parser, head_span, visibility),
        Token::Enum => parse_enum(parser, head_span, visibility),
        Token::Trait => parse_trait(parser, head_span, visibility),
        Token::Implement => parse_implementation(parser, head_span, visibility),
        Token::Capability => {
            if visibility != Visibility::Private {
                reject_visibility_on_capability(head_span.clone())?;
            }
            parse_capability(parser, head_span)
        }
        Token::Module => parse_module(parser, head_span, visibility),
        Token::Use => {
            if visibility != Visibility::Private {
                reject_visibility_on_import(head_span, "use")?;
            }
            parse_use(parser, kw_span)
        }
        other => Err(ParseError::UnexpectedToken {
            expected:
                "`function`, `constant`, `type`, `struct`, `enum`, `trait`, `implement`, `module`, `use`, or `public`"
                    .to_owned(),
            found: format!("{other:?}"),
            span: kw_span,
        }),
    }
}

/// Imports never carry visibility. Re-exporting an imported name is a
/// post-v0.2.x feature (ADR-0005), so `public use` is rejected with a
/// clear error.
fn reject_visibility_on_import(head_span: Span, keyword: &str) -> Result<(), ParseError> {
    Err(ParseError::UnexpectedToken {
        expected: "`function`, `constant`, `type`, `struct`, `enum`, or `module` after `public`"
            .to_owned(),
        found: format!("`{keyword}` (re-exports of imported names are not yet implemented)"),
        span: head_span,
    })
}

/// ADR-0069 Lát 0: a `capability` declaration carries no visibility yet. Rather
/// than drop a `public` prefix silently (a refuse-over-guess violation — the
/// user would think the capability is exported), reject it explicitly.
fn reject_visibility_on_capability(head_span: Span) -> Result<(), ParseError> {
    Err(ParseError::UnexpectedToken {
        expected: "`function`, `constant`, `type`, `struct`, `enum`, or `module` after `public`"
            .to_owned(),
        found: "`capability` (visibility is not yet supported on capabilities — ADR-0069 Lát 1+)"
            .to_owned(),
        span: head_span,
    })
}

/// Parse an optional visibility prefix.
///
/// Recognized forms (per ADR-0005):
/// - (nothing) → `Visibility::Private`
/// - `public` → `Visibility::Public`
/// - `public(package)` → `Visibility::PublicPackage`
///
/// Anything else after `public(` is rejected. Triết deliberately omits
/// `public(super)` / `public(in path)` to keep the ABI surface model
/// simple.
fn parse_visibility(parser: &mut Parser<'_>) -> Result<Visibility, ParseError> {
    if !matches!(parser.peek_token(), Some(Token::Public)) {
        return Ok(Visibility::Private);
    }
    parser.advance(); // consume `public`

    if !matches!(parser.peek_token(), Some(Token::LParen)) {
        return Ok(Visibility::Public);
    }
    parser.advance(); // consume `(`

    let (token, span) = parser
        .peek()
        .cloned()
        .ok_or_else(|| ParseError::UnexpectedEof {
            expected: "`package` after `public(`".to_owned(),
            span: parser.eof_span(),
        })?;
    match token {
        Token::Identifier(ref name) if name == "package" => {
            parser.advance();
            parser.expect(&Token::RParen, "`)`")?;
            Ok(Visibility::PublicPackage)
        }
        other => Err(ParseError::UnexpectedToken {
            expected: "`package` (the only restriction allowed in `public(...)`)".to_owned(),
            found: format!("{other:?}"),
            span,
        }),
    }
}

fn parse_function(
    parser: &mut Parser<'_>,
    head_span: Span,
    visibility: Visibility,
) -> Result<Spanned<Item>, ParseError> {
    let def = parse_function_def(parser, visibility)?;
    let end = parser.previous_token_end(head_span.end);
    let span = head_span.start..end;
    Ok(Spanned::new(Item::Function { def }, span))
}

/// Parse a `function ...` definition (after optional visibility) into a
/// [`FunctionDefinition`]. Shared by top-level functions and `implement`
/// method bodies (ADR-0061 T2.3) so the two parse paths never drift.
fn parse_function_def(
    parser: &mut Parser<'_>,
    visibility: Visibility,
) -> Result<FunctionDefinition, ParseError> {
    parser.expect(&Token::Function, "`function`")?;

    let (name, _) = parse_item_name(parser, "function name")?;
    // Optional generic type parameters `<T, U>` per ADR-0019
    // Addendum §A7 (v0.7.4.1). Returns empty Vec when absent.
    let type_parameters = parse_generic_params(parser)?;

    parser.expect(&Token::LParen, "`(`")?;
    let parameters = parse_parameter_list(parser)?;
    parser.expect(&Token::RParen, "`)`")?;

    let return_type = if parser.eat(&Token::ThinArrow) {
        Some(parse_type(parser)?)
    } else {
        None
    };

    // Body — either `{ ... }` or `= expr`.
    let body = match parser.peek_token() {
        Some(Token::LBrace) => FunctionBody::Block {
            block: parse_top_block(parser)?,
        },
        Some(Token::Assign) => FunctionBody::Expression {
            expr: parse_assignment_body(parser)?,
        },
        _ => {
            return Err(ParseError::UnexpectedToken {
                expected: "function body (`{...}` or `= expr`)".to_owned(),
                found: format!("{:?}", parser.peek_token()),
                span: parser.current_span(),
            });
        }
    };

    Ok(FunctionDefinition {
        visibility,
        name,
        type_parameters,
        parameters,
        return_type,
        body,
    })
}

/// Parse `trait Name { function sig... }` → `Item::Trait` (ADR-0061 T2.2).
/// A trait body holds method *signatures* (no body); the `implement` block
/// supplies the bodies.
fn parse_trait(
    parser: &mut Parser<'_>,
    head_span: Span,
    visibility: Visibility,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Trait, "`trait`")?;
    let (name, _) = parse_item_name(parser, "trait name")?;
    let type_parameters = parse_generic_params(parser)?;

    parser.expect(&Token::LBrace, "`{`")?;
    let mut methods = Vec::new();
    while !matches!(parser.peek_token(), Some(Token::RBrace)) {
        methods.push(parse_method_signature(parser)?);
    }
    parser.expect(&Token::RBrace, "`}`")?;

    let end = parser.previous_token_end(head_span.end);
    let span = head_span.start..end;
    Ok(Spanned::new(
        Item::Trait {
            def: TraitDefinition {
                name,
                type_parameters,
                methods,
                visibility,
            },
        },
        span,
    ))
}

/// Parse one trait method signature: `function name(params) -> Type` with
/// no body (ADR-0061 Tier 1). Per-method generics are unsupported
/// (`MethodSignature` carries no type parameters). A trailing `;` is
/// optional.
fn parse_method_signature(parser: &mut Parser<'_>) -> Result<MethodSignature, ParseError> {
    parser.expect(&Token::Function, "`function`")?;
    let (name, _) = parse_item_name(parser, "method name")?;

    parser.expect(&Token::LParen, "`(`")?;
    let parameters = parse_parameter_list(parser)?;
    parser.expect(&Token::RParen, "`)`")?;

    let return_type = if parser.eat(&Token::ThinArrow) {
        Some(parse_type(parser)?)
    } else {
        None
    };
    let _ = parser.eat(&Token::Semi);

    Ok(MethodSignature {
        name,
        parameters,
        return_type,
    })
}

/// Parse `implement Trait for Type { function... }` → `Item::Implementation`
/// (ADR-0061 T2.3). Method bodies are full `FunctionDefinition`s parsed by
/// the shared `parse_function_def`, so they never drift from top-level
/// functions. `for_type` is stored as an arena `TypeId`.
fn parse_implementation(
    parser: &mut Parser<'_>,
    head_span: Span,
    _visibility: Visibility,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Implement, "`implement`")?;
    let (trait_name, _) = parse_item_name(parser, "trait name")?;
    parser.expect(&Token::For, "`for`")?;
    let for_type = parse_type(parser)?;

    parser.expect(&Token::LBrace, "`{`")?;
    let mut methods = Vec::new();
    while !matches!(parser.peek_token(), Some(Token::RBrace)) {
        let method_visibility = parse_visibility(parser)?;
        methods.push(parse_function_def(parser, method_visibility)?);
    }
    parser.expect(&Token::RBrace, "`}`")?;

    let end = parser.previous_token_end(head_span.end);
    let span = head_span.start..end;
    Ok(Spanned::new(
        Item::Implementation {
            def: ImplementationDefinition {
                trait_name,
                for_type,
                methods,
            },
        },
        span,
    ))
}

fn parse_parameter_list(parser: &mut Parser<'_>) -> Result<Vec<FunctionParameter>, ParseError> {
    let mut parameters = Vec::new();
    if matches!(parser.peek_token(), Some(Token::RParen)) {
        return Ok(parameters);
    }
    loop {
        let is_first = parameters.is_empty();
        parameters.push(parse_parameter(parser, is_first)?);
        if !parser.eat(&Token::Comma) {
            break;
        }
        if matches!(parser.peek_token(), Some(Token::RParen)) {
            break;
        }
    }
    Ok(parameters)
}

fn parse_parameter(
    parser: &mut Parser<'_>,
    is_first: bool,
) -> Result<FunctionParameter, ParseError> {
    // ADR-0061 T2.4: bare `self` receiver. Only valid as the first
    // parameter (`function compare(self, other: T) -> Trit`). It carries
    // no `: Type` annotation; its type is the marker `TypeExpr::SelfType`,
    // resolved to the receiver type in typecheck (T3). `self` anywhere
    // else is a parse error.
    if matches!(parser.peek_token(), Some(Token::SelfKw)) {
        let span = parser.current_span();
        if !is_first {
            return Err(ParseError::UnexpectedToken {
                expected: "parameter name (`self` is only valid as the first parameter)".to_owned(),
                found: "`self`".to_owned(),
                span,
            });
        }
        parser.advance();
        let type_annotation = parser
            .arena
            .alloc_type(Spanned::new(TypeExpr::SelfType, span));
        return Ok(FunctionParameter {
            name: "self".to_owned(),
            type_annotation,
            passing_mode: ParameterPassing::Borrow,
        });
    }

    // Optional passing mode prefix: `mutable` or `owned`.
    let passing = if parser.eat(&Token::Mutable) {
        ParameterPassing::MutableBorrow
    } else if parser.eat(&Token::Owned) {
        ParameterPassing::Move
    } else {
        ParameterPassing::Borrow
    };

    let (name_token, _) = parser
        .peek()
        .cloned()
        .ok_or_else(|| ParseError::UnexpectedEof {
            expected: "parameter name".to_owned(),
            span: parser.eof_span(),
        })?;
    let name = match name_token {
        Token::Identifier(name) => {
            parser.advance();
            name
        }
        other => {
            return Err(ParseError::UnexpectedToken {
                expected: "parameter name".to_owned(),
                found: format!("{other:?}"),
                span: parser.current_span(),
            });
        }
    };

    parser.expect(&Token::Colon, "`:`")?;
    let type_annotation = parse_type(parser)?;

    Ok(FunctionParameter {
        name,
        type_annotation,
        passing_mode: passing,
    })
}

fn parse_const_item(
    parser: &mut Parser<'_>,
    head_span: Span,
    visibility: Visibility,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Constant, "`constant`")?;
    let (name, _) = parse_item_name(parser, "constant name")?;

    let type_annotation = if parser.eat(&Token::Colon) {
        Some(parse_type(parser)?)
    } else {
        None
    };

    parser.expect(&Token::Assign, "`=`")?;
    let value = parse_expression(parser)?;
    let _ = parser.eat(&Token::Semi);

    let end = parser.arena.expression(value).span.end;
    let span = head_span.start..end;
    Ok(Spanned::new(
        Item::Constant {
            visibility,
            name,
            type_annotation,
            value,
        },
        span,
    ))
}

fn parse_type_alias(
    parser: &mut Parser<'_>,
    head_span: Span,
    visibility: Visibility,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Type, "`type`")?;
    let (name, _) = parse_item_name(parser, "type alias name")?;

    parser.expect(&Token::Assign, "`=`")?;
    let target = parse_type(parser)?;
    let _ = parser.eat(&Token::Semi);

    let end = parser.arena.type_expression(target).span.end;
    let span = head_span.start..end;
    Ok(Spanned::new(
        Item::TypeAlias {
            name,
            type_annotation: target,
            visibility,
        },
        span,
    ))
}

/// Parse a `use` declaration (ADR-0071, supersedes `import`/`from`):
/// `use std::io;`, `use std::io::println;`, `use a::b::{x, y as z};`.
///
/// A `::`-separated path with an optional trailing brace group. An EMPTY
/// group is a plain path (whole module or single item — bound by the leaf
/// segment in the resolver); a NON-empty group binds each name out of the
/// module named by the path. Glob (`*`) is rejected (carried over from
/// ADR-0005).
fn parse_use(parser: &mut Parser<'_>, head_span: Span) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Use, "`use`")?;
    let (segments, group) = parse_use_path(parser, "use path")?;
    let _ = parser.eat(&Token::Semi);

    let end = parser.previous_token_end(head_span.end);
    let span = head_span.start..end;
    Ok(Spanned::new(
        Item::Use {
            path: ImportPath { segments },
            group,
        },
        span,
    ))
}

/// Parse a brace group `{a, b as c}` at the tail of a `use` path (ADR-0071).
/// At least one item is required; a trailing comma is allowed.
fn parse_use_group(parser: &mut Parser<'_>) -> Result<Vec<ImportName>, ParseError> {
    parser.expect(&Token::LBrace, "`{`")?;
    let mut names = vec![parse_use_item(parser)?];
    while parser.eat(&Token::Comma) {
        // Trailing comma allowed: `use x::{a, b,}`.
        if matches!(parser.peek_token(), Some(Token::RBrace) | None) {
            break;
        }
        names.push(parse_use_item(parser)?);
    }
    parser.expect(&Token::RBrace, "`}`")?;
    Ok(names)
}

fn parse_use_item(parser: &mut Parser<'_>) -> Result<ImportName, ParseError> {
    let (token, span) = parser
        .peek()
        .cloned()
        .ok_or_else(|| ParseError::UnexpectedEof {
            expected: "imported name".to_owned(),
            span: parser.eof_span(),
        })?;
    let name = match token {
        Token::Identifier(name) => {
            parser.advance();
            name
        }
        Token::Star => {
            return Err(ParseError::UnexpectedToken {
                expected: "imported name (glob `*` is not supported per ADR-0071)".to_owned(),
                found: "`*`".to_owned(),
                span,
            });
        }
        other => {
            return Err(ParseError::UnexpectedToken {
                expected: "imported name".to_owned(),
                found: format!("{other:?}"),
                span,
            });
        }
    };

    let alias = if parser.eat(&Token::As) {
        let (alias_token, alias_span) =
            parser
                .peek()
                .cloned()
                .ok_or_else(|| ParseError::UnexpectedEof {
                    expected: "alias identifier after `as`".to_owned(),
                    span: parser.eof_span(),
                })?;
        match alias_token {
            Token::Identifier(alias_name) => {
                parser.advance();
                Some(alias_name)
            }
            other => {
                return Err(ParseError::UnexpectedToken {
                    expected: "alias identifier after `as`".to_owned(),
                    found: format!("{other:?}"),
                    span: alias_span,
                });
            }
        }
    } else {
        None
    };

    Ok(ImportName { name, alias })
}

/// Parse a `::`-separated `use` path with an optional trailing brace group
/// (ADR-0071). The first segment may be a regular identifier or a reserved
/// path keyword (`khi`/`self`/`super`); later segments must be regular
/// identifiers — path keywords are only meaningful at the root. Returns
/// `(segments, group)`; an empty group means a plain path. A `.` separator
/// is NOT accepted here — `use std.io` fails at the segment boundary.
fn parse_use_path(
    parser: &mut Parser<'_>,
    what: &str,
) -> Result<(Vec<String>, Vec<ImportName>), ParseError> {
    let mut segments = vec![parse_use_path_root(parser, what)?];
    let mut group = Vec::new();
    while parser.eat(&Token::ColonColon) {
        // A brace group ends the path: `use a::b::{x, y}`.
        if matches!(parser.peek_token(), Some(Token::LBrace)) {
            group = parse_use_group(parser)?;
            break;
        }
        segments.push(parse_use_path_segment(parser)?);
    }
    Ok((segments, group))
}

fn parse_use_path_root(parser: &mut Parser<'_>, what: &str) -> Result<String, ParseError> {
    let (token, span) = parser
        .peek()
        .cloned()
        .ok_or_else(|| ParseError::UnexpectedEof {
            expected: what.to_owned(),
            span: parser.eof_span(),
        })?;
    let name = match token {
        Token::Identifier(name) => name,
        Token::Khi => "khi".to_owned(),
        Token::SelfKw => "self".to_owned(),
        Token::Super => "super".to_owned(),
        other => {
            return Err(ParseError::UnexpectedToken {
                expected: what.to_owned(),
                found: format!("{other:?}"),
                span,
            });
        }
    };
    parser.advance();
    Ok(name)
}

fn parse_use_path_segment(parser: &mut Parser<'_>) -> Result<String, ParseError> {
    let (token, span) = parser
        .peek()
        .cloned()
        .ok_or_else(|| ParseError::UnexpectedEof {
            expected: "identifier after `::`".to_owned(),
            span: parser.eof_span(),
        })?;
    match token {
        Token::Identifier(name) => {
            parser.advance();
            Ok(name)
        }
        other => Err(ParseError::UnexpectedToken {
            expected: "identifier after `::`".to_owned(),
            found: format!("{other:?}"),
            span,
        }),
    }
}

/// Parse a `module foo` or `module foo { items… }` declaration.
///
/// Per ADR-0005, module declarations are first-class (Java JPMS-aligned).
/// File-bound form leaves resolution to the module loader (v0.2.x.6);
/// inline form recurses into nested items here.
fn parse_module(
    parser: &mut Parser<'_>,
    head_span: Span,
    visibility: Visibility,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Module, "`module`")?;
    let (name, _) = parse_item_name(parser, "module name")?;

    // Submodule paths nest via inline `module` blocks; `module foo.bar`
    // is *not* valid syntax (ADR-0005 §"File resolution").
    if matches!(parser.peek_token(), Some(Token::Dot)) {
        let span = parser.current_span();
        return Err(ParseError::UnexpectedToken {
            expected: "`{` (inline body), `;`, or end of declaration — \
                 nested paths must use nested `module` blocks"
                .to_owned(),
            found: "`.`".to_owned(),
            span,
        });
    }

    let content = if matches!(parser.peek_token(), Some(Token::LBrace)) {
        parser.advance();
        let mut items = Vec::new();
        while !matches!(parser.peek_token(), Some(Token::RBrace)) {
            if parser.at_end() {
                return Err(ParseError::UnexpectedEof {
                    expected: "`}` to close module body".to_owned(),
                    span: parser.eof_span(),
                });
            }
            items.push(parse_item(parser)?);
        }
        parser.expect(&Token::RBrace, "`}`")?;
        ModuleContent::Inline { items }
    } else {
        let _ = parser.eat(&Token::Semi);
        ModuleContent::External
    };

    let end = parser.previous_token_end(head_span.end);
    let span = head_span.start..end;
    Ok(Spanned::new(
        Item::Module {
            module: ModuleItem {
                name,
                content,
                visibility,
            },
        },
        span,
    ))
}

fn parse_struct(
    parser: &mut Parser<'_>,
    head_span: Span,
    visibility: Visibility,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Struct, "`struct`")?;

    let (name, _) = parse_item_name(parser, "struct name")?;
    let type_parameters = parse_generic_params(parser)?;

    parser.expect(&Token::LBrace, "`{`")?;
    let fields = parse_struct_fields(parser)?;
    parser.expect(&Token::RBrace, "`}`")?;

    let end = parser.previous_token_end(head_span.end);
    let span = head_span.start..end;
    Ok(Spanned::new(
        Item::Struct {
            def: StructDefinition {
                visibility,
                name,
                type_parameters,
                fields,
            },
        },
        span,
    ))
}

fn parse_struct_fields(parser: &mut Parser<'_>) -> Result<Vec<StructField>, ParseError> {
    let mut fields = Vec::new();
    if matches!(parser.peek_token(), Some(Token::RBrace)) {
        return Ok(fields);
    }
    loop {
        let name = parse_ident(parser, "field name")?;
        parser.expect(&Token::Colon, "`:`")?;
        let type_annotation = parse_type(parser)?;
        fields.push(StructField {
            name,
            type_annotation,
        });

        if !parser.eat(&Token::Comma) {
            break;
        }
        if matches!(parser.peek_token(), Some(Token::RBrace)) {
            break;
        }
    }
    Ok(fields)
}

/// Parse `capability Name <level>` → `Item::Capability` (ADR-0069 Lát 0).
///
/// The level (`grant`/`ambient`/`deny`/`defer`) is a CONTEXTUAL keyword:
/// lexed as a plain identifier, only meaningful here. Omitting it defaults
/// to `Ambient` (ADR-0069 §2). Any other identifier in level position is a
/// parse error.
fn parse_capability(parser: &mut Parser<'_>, head_span: Span) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Capability, "`capability`")?;
    let (name, _) = parse_item_name(parser, "capability name")?;

    // Optional contextual level keyword (identifier). None → Ambient.
    let level = if let Some(Token::Identifier(word)) = parser.peek_token().cloned() {
        let lvl = match word.as_str() {
            "grant" => CapabilityLevel::Grant,
            "ambient" => CapabilityLevel::Ambient,
            "deny" => CapabilityLevel::Deny,
            "defer" => CapabilityLevel::Defer,
            other => {
                return Err(ParseError::UnexpectedToken {
                    expected: "`grant`, `ambient`, `deny`, or `defer`".to_owned(),
                    found: format!("`{other}`"),
                    span: parser.current_span(),
                });
            }
        };
        parser.advance();
        lvl
    } else {
        CapabilityLevel::Ambient
    };
    let _ = parser.eat(&Token::Semi);

    let end = parser.previous_token_end(head_span.end);
    let span = head_span.start..end;
    Ok(Spanned::new(Item::Capability { name, level }, span))
}

fn parse_enum(
    parser: &mut Parser<'_>,
    head_span: Span,
    visibility: Visibility,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Enum, "`enum`")?;

    let (name, _) = parse_item_name(parser, "enum name")?;
    let type_parameters = parse_generic_params(parser)?;

    parser.expect(&Token::LBrace, "`{`")?;
    let variants = parse_enum_variants(parser)?;
    parser.expect(&Token::RBrace, "`}`")?;

    let end = parser.previous_token_end(head_span.end);
    let span = head_span.start..end;
    Ok(Spanned::new(
        Item::Enum {
            def: EnumDefinition {
                visibility,
                name,
                type_parameters,
                variants,
            },
        },
        span,
    ))
}

fn parse_enum_variants(parser: &mut Parser<'_>) -> Result<Vec<EnumVariant>, ParseError> {
    let mut variants = Vec::new();
    if matches!(parser.peek_token(), Some(Token::RBrace)) {
        return Ok(variants);
    }
    loop {
        let name = parse_ident(parser, "variant name")?;
        let payload = if parser.eat(&Token::LParen) {
            let ty = parse_type(parser)?;
            parser.expect(&Token::RParen, "`)`")?;
            Some(ty)
        } else {
            None
        };
        variants.push(EnumVariant { name, payload });

        if !parser.eat(&Token::Comma) {
            break;
        }
        if matches!(parser.peek_token(), Some(Token::RBrace)) {
            break;
        }
    }
    Ok(variants)
}

/// Parse optional generic type parameters: `<T, U>` or `<F: Send>`. Returns an empty
/// vec if the next token is not `<`.
fn parse_generic_params(parser: &mut Parser<'_>) -> Result<Vec<TypeParameter>, ParseError> {
    if !matches!(parser.peek_token(), Some(Token::Lt)) {
        return Ok(Vec::new());
    }
    parser.advance(); // consume `<`
    let mut parameters = Vec::new();
    loop {
        let name = parse_ident(parser, "type parameter")?;
        let mut bound = None;
        if parser.eat(&Token::Colon) {
            let (bound_tok, span) =
                parser
                    .peek()
                    .cloned()
                    .ok_or_else(|| ParseError::UnexpectedEof {
                        expected: "generic bound".to_owned(),
                        span: parser.eof_span(),
                    })?;
            match bound_tok {
                Token::Identifier(ref id) if id == "Send" => {
                    parser.advance();
                    bound = Some(GenericBound::Send);
                }
                other => {
                    return Err(ParseError::UnexpectedToken {
                        expected: "`Send` bound".to_owned(),
                        found: format!("{other:?}"),
                        span,
                    });
                }
            }
        }
        parameters.push(TypeParameter { name, bound });
        if !parser.eat(&Token::Comma) {
            break;
        }
    }
    parser.expect(&Token::Gt, "`>`")?;
    Ok(parameters)
}

fn parse_ident(parser: &mut Parser<'_>, expected: &str) -> Result<String, ParseError> {
    let (token, _) = parser
        .peek()
        .cloned()
        .ok_or_else(|| ParseError::UnexpectedEof {
            expected: expected.to_owned(),
            span: parser.eof_span(),
        })?;
    match token {
        Token::Identifier(name) => {
            parser.advance();
            Ok(name)
        }
        other => Err(ParseError::UnexpectedToken {
            expected: expected.to_owned(),
            found: format!("{other:?}"),
            span: parser.current_span(),
        }),
    }
}

/// Reserved namespace roots — top-level items cannot be defined with
/// these names. They're kept for the standard library and OS-native
/// namespaces (ADR-0005, trụ cột #5).
///
/// Path keywords (`crate`, `self`, `super`) are reserved at the lexer
/// level and surface as `UnexpectedToken` if used in identifier position.
const RESERVED_ITEM_NAMES: &[&str] = &["std", "sys", "dev", "usr", "core"];

/// Parse an identifier intended as a top-level item name (function,
/// const, type alias, struct, enum). Rejects reserved namespace roots
/// per ADR-0005.
///
/// Returns `(name, span_of_identifier)` so callers can re-use the span
/// for error reporting downstream.
fn parse_item_name(parser: &mut Parser<'_>, expected: &str) -> Result<(String, Span), ParseError> {
    let (token, span) = parser
        .peek()
        .cloned()
        .ok_or_else(|| ParseError::UnexpectedEof {
            expected: expected.to_owned(),
            span: parser.eof_span(),
        })?;
    let name = match token {
        Token::Identifier(name) => {
            parser.advance();
            name
        }
        other => {
            return Err(ParseError::UnexpectedToken {
                expected: expected.to_owned(),
                found: format!("{other:?}"),
                span,
            });
        }
    };
    if RESERVED_ITEM_NAMES.contains(&name.as_str()) {
        return Err(ParseError::ReservedItemName { name, span });
    }
    Ok((name, span))
}

#[cfg(test)]
mod tests {
    use super::*;
    use triet_lexer::lex;

    fn parse(source: &str) -> (Parser<'static>, Spanned<Item>) {
        let tokens: Vec<_> = lex(source).unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let item = parse_item(&mut parser).expect("parse failed");
        (parser, item)
    }

    fn try_parse(source: &str) -> Result<(Parser<'static>, Spanned<Item>), ParseError> {
        let tokens: Vec<_> = lex(source).unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let item = parse_item(&mut parser)?;
        Ok((parser, item))
    }

    #[test]
    fn parses_no_arg_function_with_block_body() {
        let (_, item) = parse("function main() { }");
        match &item.node {
            Item::Function { def } => {
                assert_eq!(def.name, "main");
                assert!(def.parameters.is_empty());
                assert!(matches!(def.body, FunctionBody::Block { block: _ }));
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn parses_function_with_expression_body() {
        let (_, item) = parse("function double(n: Integer) -> Integer = n * 2");
        match &item.node {
            Item::Function { def } => {
                assert_eq!(def.parameters.len(), 1);
                assert!(def.return_type.is_some());
                assert!(matches!(def.body, FunctionBody::Expression { expr: _ }));
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn parses_function_with_multiple_params() {
        let (_, item) = parse("function add(a: Integer, b: Integer) -> Integer = a + b");
        match &item.node {
            Item::Function { def } => assert_eq!(def.parameters.len(), 2),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_function_with_mut_parameter() {
        let (_, item) = parse("function append(mutable buffer: String, suffix: String) { }");
        match &item.node {
            Item::Function { def } => {
                assert_eq!(
                    def.parameters[0].passing_mode,
                    ParameterPassing::MutableBorrow
                );
                assert_eq!(def.parameters[1].passing_mode, ParameterPassing::Borrow);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_function_with_owned_parameter() {
        let (_, item) = parse("function consume(owned data: String) -> String = data");
        match &item.node {
            Item::Function { def } => {
                assert_eq!(def.parameters[0].passing_mode, ParameterPassing::Move);
            }
            other => panic!("got {other:?}"),
        }
    }

    /// v0.7.4.1: generic function with single type parameter.
    /// ADR-0019 Addendum §A7 — unblocks self-host compiler stdlib
    /// stubs. Mirror existing `enum Option<T>` parsing pattern.
    #[test]
    fn parses_function_with_single_type_param() {
        let (_, item) = parse("function identity<T>(x: T) -> T = x");
        match &item.node {
            Item::Function { def } => {
                assert_eq!(
                    def.type_parameters,
                    vec![TypeParameter {
                        name: "T".to_owned(),
                        bound: None
                    }]
                );
                assert_eq!(def.parameters.len(), 1);
                assert!(def.return_type.is_some());
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    /// v0.7.4.1: generic function with multiple type parameters.
    /// Targets `function hashmap_keys<K, V>(m: HashMap<K, V>) -> Vector<K>`
    /// shape needed for stdlib stub work (v0.7.4.2).
    #[test]
    fn parses_function_with_multiple_type_params() {
        let (_, item) = parse("function pair<K, V>(k: K, v: V) -> K = k");
        match &item.node {
            Item::Function { def } => {
                assert_eq!(
                    def.type_parameters,
                    vec![
                        TypeParameter {
                            name: "K".to_owned(),
                            bound: None
                        },
                        TypeParameter {
                            name: "V".to_owned(),
                            bound: None
                        }
                    ]
                );
                assert_eq!(def.parameters.len(), 2);
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    /// v0.7.4.1: non-generic function still parses (no regression).
    #[test]
    fn parses_function_without_type_params_has_empty_type_params() {
        let (_, item) = parse("function add(a: Integer, b: Integer) -> Integer = a + b");
        match &item.node {
            Item::Function { def } => {
                assert!(
                    def.type_parameters.is_empty(),
                    "non-generic function must have empty type_parameters, got {:?}",
                    def.type_parameters
                );
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_const_with_annotation() {
        let (_, item) = parse("constant PI: Integer = 3");
        match &item.node {
            Item::Constant {
                name,
                type_annotation,
                ..
            } => {
                assert_eq!(name, "PI");
                assert!(type_annotation.is_some());
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_const_without_annotation() {
        let (_, item) = parse("constant ANSWER = 42");
        match &item.node {
            Item::Constant { name, .. } => assert_eq!(name, "ANSWER"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_type_alias() {
        let (_, item) = parse("type Username = String");
        match &item.node {
            Item::TypeAlias { name, .. } => assert_eq!(name, "Username"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_type_alias_with_generic_target() {
        let (_, item) = parse("type Lookup = Map<String, Integer>");
        match &item.node {
            Item::TypeAlias { name, .. } => assert_eq!(name, "Lookup"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_simple_import() {
        let (_, item) = parse("use std");
        match &item.node {
            Item::Use { path, group } => {
                assert_eq!(path.segments, vec!["std"]);
                assert!(group.is_empty());
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_dotted_import() {
        let (_, item) = parse("use std::io::println");
        match &item.node {
            Item::Use { path, group } => {
                assert_eq!(path.segments.len(), 3);
                assert!(group.is_empty());
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn default_function_visibility_is_private() {
        let (_, item) = parse("function greet() { }");
        let Item::Function { def } = &item.node else {
            panic!("expected function")
        };
        assert_eq!(def.visibility, Visibility::Private);
    }

    #[test]
    fn pub_function_captures_public_visibility() {
        let (_, item) = parse("public function greet() { }");
        let Item::Function { def } = &item.node else {
            panic!("expected function")
        };
        assert_eq!(def.visibility, Visibility::Public);
        assert_eq!(def.name, "greet");
    }

    #[test]
    fn pub_pkg_function_captures_publicpkg_visibility() {
        let (_, item) = parse("public(package) function helper() { }");
        let Item::Function { def } = &item.node else {
            panic!("expected function")
        };
        assert_eq!(def.visibility, Visibility::PublicPackage);
    }

    #[test]
    fn pub_struct_captures_visibility() {
        let (_, item) = parse("public struct Point { x: Integer, y: Integer }");
        let Item::Struct { def } = &item.node else {
            panic!("expected struct")
        };
        assert_eq!(def.visibility, Visibility::Public);
        assert_eq!(def.name, "Point");
    }

    #[test]
    fn pub_enum_captures_visibility() {
        let (_, item) = parse("public enum Option<T> { Some(T), None }");
        let Item::Enum { def } = &item.node else {
            panic!("expected enum")
        };
        assert_eq!(def.visibility, Visibility::Public);
    }

    #[test]
    fn pub_const_captures_visibility() {
        let (_, item) = parse("public constant PI: Integer = 3");
        let Item::Constant {
            visibility, name, ..
        } = &item.node
        else {
            panic!("expected constant")
        };
        assert_eq!(*visibility, Visibility::Public);
        assert_eq!(name, "PI");
    }

    #[test]
    fn pub_pkg_type_alias_captures_visibility() {
        let (_, item) = parse("public(package) type Username = String");
        let Item::TypeAlias {
            visibility, name, ..
        } = &item.node
        else {
            panic!("expected type alias")
        };
        assert_eq!(*visibility, Visibility::PublicPackage);
        assert_eq!(name, "Username");
    }

    #[test]
    fn public_on_import_is_rejected() {
        // Re-exports of imported names are post-v0.2.x.
        let result = try_parse("public use std::io");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn public_with_invalid_restriction_is_rejected() {
        // Only `public(package)` is accepted; `public(crate)` / `public(super)` are not.
        let result = try_parse("public(crate) function foo() { }");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn public_at_eof_errors() {
        let result = try_parse("public");
        assert!(matches!(result, Err(ParseError::UnexpectedEof { .. })));
    }

    #[test]
    fn item_span_includes_public_keyword() {
        // Span should start at `public`, not at the inner keyword.
        let (_, item) = parse("public function greet() { }");
        // `public` starts at byte 0, so the item span must too.
        assert_eq!(item.span.start, 0);
    }

    // === Reserved-name validation (ADR-0005, task v0.2.x.4) ===

    #[test]
    fn struct_named_std_is_rejected() {
        let result = try_parse("struct std { x: Integer }");
        assert!(matches!(
            result,
            Err(ParseError::ReservedItemName { name, .. }) if name == "std"
        ));
    }

    #[test]
    fn fn_named_sys_is_rejected() {
        let result = try_parse("function sys() { }");
        assert!(matches!(
            result,
            Err(ParseError::ReservedItemName { name, .. }) if name == "sys"
        ));
    }

    #[test]
    fn enum_named_dev_is_rejected() {
        let result = try_parse("enum dev { A, B }");
        assert!(matches!(
            result,
            Err(ParseError::ReservedItemName { name, .. }) if name == "dev"
        ));
    }

    #[test]
    fn const_named_usr_is_rejected() {
        let result = try_parse("constant usr: Integer = 5");
        assert!(matches!(
            result,
            Err(ParseError::ReservedItemName { name, .. }) if name == "usr"
        ));
    }

    #[test]
    fn type_alias_named_core_is_rejected() {
        let result = try_parse("type core = Integer");
        assert!(matches!(
            result,
            Err(ParseError::ReservedItemName { name, .. }) if name == "core"
        ));
    }

    #[test]
    fn fn_named_khi_is_rejected_via_unexpected_token() {
        // `khi` lexes as Token::Khi, not Token::Identifier (ADR-0024).
        let result = try_parse("function khi() { }");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn struct_named_self_is_rejected_via_unexpected_token() {
        let result = try_parse("struct self { }");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn enum_named_super_is_rejected_via_unexpected_token() {
        let result = try_parse("enum super { A }");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn substring_of_reserved_name_is_allowed() {
        // `crater`, `stderr`, `system` start with a reserved prefix but
        // are themselves valid identifiers — keyword matching is not
        // greedy, and reserved-name comparison is exact.
        let (_, item) = parse("function crater() { }");
        let Item::Function { def } = &item.node else {
            panic!("expected function")
        };
        assert_eq!(def.name, "crater");

        let (_, item) = parse("struct Stderr { }");
        let Item::Struct { def } = &item.node else {
            panic!("expected struct")
        };
        assert_eq!(def.name, "Stderr");
    }

    #[test]
    fn pub_reserved_name_still_rejected() {
        // Visibility prefix doesn't bypass reservation.
        let result = try_parse("public struct std { }");
        assert!(matches!(
            result,
            Err(ParseError::ReservedItemName { name, .. }) if name == "std"
        ));
    }

    #[test]
    fn errors_on_unrecognized_item_keyword() {
        let result = try_parse("nonsense foo");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn errors_on_eof_at_item_start() {
        let result = try_parse("");
        assert!(matches!(result, Err(ParseError::UnexpectedEof { .. })));
    }

    #[test]
    fn errors_on_function_missing_body() {
        let result = try_parse("function foo()");
        assert!(matches!(
            result,
            Err(ParseError::UnexpectedToken { .. } | ParseError::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn errors_on_function_missing_param_name() {
        let result = try_parse("function foo(: Integer) { }");
        assert!(result.is_err());
    }

    #[test]
    fn parses_empty_struct() {
        let (_, item) = parse("struct Empty { }");
        assert!(matches!(item.node, Item::Struct { def: _ }));
        if let Item::Struct { def } = &item.node {
            assert_eq!(def.name, "Empty");
            assert!(def.fields.is_empty());
        }
    }

    #[test]
    fn parses_struct_with_fields() {
        let (_, item) = parse("struct Point { x: Integer, y: Integer }");
        assert!(matches!(item.node, Item::Struct { def: _ }));
        if let Item::Struct { def } = &item.node {
            assert_eq!(def.name, "Point");
            assert_eq!(def.fields.len(), 2);
            assert_eq!(def.fields[0].name, "x");
            assert_eq!(def.fields[1].name, "y");
        }
    }

    #[test]
    fn parses_enum_with_unit_variants() {
        let (_, item) = parse("enum Color { Red, Green, Blue }");
        assert!(matches!(item.node, Item::Enum { def: _ }));
        if let Item::Enum { def } = &item.node {
            assert_eq!(def.name, "Color");
            assert_eq!(def.variants.len(), 3);
            assert_eq!(def.variants[0].name, "Red");
            assert!(def.variants[0].payload.is_none());
        }
    }

    #[test]
    fn parses_enum_with_payload_variants() {
        let (_, item) = parse("enum Maybe { Some(Integer), None }");
        assert!(matches!(item.node, Item::Enum { def: _ }));
        if let Item::Enum { def } = &item.node {
            assert_eq!(def.name, "Maybe");
            assert_eq!(def.variants.len(), 2);
            assert_eq!(def.variants[0].name, "Some");
            assert!(def.variants[0].payload.is_some());
            assert_eq!(def.variants[1].name, "None");
            assert!(def.variants[1].payload.is_none());
        }
    }

    // === Module declarations (ADR-0005, task v0.2.x.5) ===

    #[test]
    fn parses_external_module_declaration() {
        let (_, item) = parse("module foo");
        let Item::Module { module: decl } = &item.node else {
            panic!("expected module")
        };
        assert_eq!(decl.name, "foo");
        assert_eq!(decl.visibility, Visibility::Private);
        assert!(matches!(decl.content, ModuleContent::External));
    }

    #[test]
    fn parses_external_module_with_trailing_semi() {
        let (_, item) = parse("module foo;");
        let Item::Module { module: decl } = &item.node else {
            panic!("expected module")
        };
        assert!(matches!(decl.content, ModuleContent::External));
    }

    #[test]
    fn parses_inline_empty_module() {
        let (_, item) = parse("module foo { }");
        let Item::Module { module: decl } = &item.node else {
            panic!("expected module")
        };
        match &decl.content {
            ModuleContent::Inline { items } => assert!(items.is_empty()),
            other @ ModuleContent::External => panic!("expected inline, got {other:?}"),
        }
    }

    #[test]
    fn parses_inline_module_with_items() {
        let (_, item) = parse(
            "module greet {\n    public function hello() { }\n    constant N: Integer = 1\n}",
        );
        let Item::Module { module: decl } = &item.node else {
            panic!("expected module")
        };
        match &decl.content {
            ModuleContent::Inline { items } => {
                assert_eq!(items.len(), 2);
                assert!(matches!(items[0].node, Item::Function { def: _ }));
                assert!(matches!(items[1].node, Item::Constant { .. }));
            }
            other @ ModuleContent::External => panic!("expected inline, got {other:?}"),
        }
    }

    #[test]
    fn parses_public_module() {
        let (_, item) = parse("public module exposed");
        let Item::Module { module: decl } = &item.node else {
            panic!("expected module")
        };
        assert_eq!(decl.visibility, Visibility::Public);
        assert_eq!(decl.name, "exposed");
    }

    #[test]
    fn parses_public_package_module() {
        let (_, item) = parse("public(package) module internal { }");
        let Item::Module { module: decl } = &item.node else {
            panic!("expected module")
        };
        assert_eq!(decl.visibility, Visibility::PublicPackage);
    }

    #[test]
    fn parses_nested_inline_modules() {
        let (_, item) = parse("module outer { module inner { } }");
        let Item::Module { module: outer } = &item.node else {
            panic!("expected outer module")
        };
        match &outer.content {
            ModuleContent::Inline { items } => {
                assert_eq!(items.len(), 1);
                let Item::Module { module: inner } = &items[0].node else {
                    panic!("expected inner module")
                };
                assert_eq!(inner.name, "inner");
            }
            other @ ModuleContent::External => panic!("expected inline, got {other:?}"),
        }
    }

    #[test]
    fn module_named_std_is_rejected() {
        // Reserved namespace roots cannot be redeclared by user code.
        let result = try_parse("module std");
        assert!(matches!(
            result,
            Err(ParseError::ReservedItemName { name, .. }) if name == "std"
        ));
    }

    #[test]
    fn module_with_dotted_name_is_rejected() {
        // `module foo.bar` is not valid — submodules nest via inline blocks.
        let result = try_parse("module foo.bar");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn unclosed_inline_module_errors() {
        let result = try_parse("module foo { function x() { }");
        assert!(matches!(result, Err(ParseError::UnexpectedEof { .. })));
    }

    // === `use path::{group}` brace form (ADR-0071, supersedes the ADR-0005 import keywords) ===

    #[test]
    fn parses_use_group_single_name() {
        let (_, item) = parse("use std::io::{println}");
        let Item::Use { path, group } = &item.node else {
            panic!("expected Use")
        };
        assert_eq!(path.segments, vec!["std", "io"]);
        assert_eq!(group.len(), 1);
        assert_eq!(group[0].name, "println");
        assert!(group[0].alias.is_none());
    }

    #[test]
    fn parses_use_group_multiple_names() {
        let (_, item) = parse("use std::io::{println, print, read_line}");
        let Item::Use { group, .. } = &item.node else {
            panic!("expected Use")
        };
        assert_eq!(group.len(), 3);
        assert_eq!(group[0].name, "println");
        assert_eq!(group[1].name, "print");
        assert_eq!(group[2].name, "read_line");
    }

    #[test]
    fn parses_use_group_with_alias() {
        let (_, item) = parse("use std::io::{println as out}");
        let Item::Use { group, .. } = &item.node else {
            panic!("expected Use")
        };
        assert_eq!(group[0].name, "println");
        assert_eq!(group[0].alias.as_deref(), Some("out"));
    }

    #[test]
    fn parses_use_group_mixed_aliases() {
        let (_, item) = parse("use std::io::{println, print as p, read_line}");
        let Item::Use { group, .. } = &item.node else {
            panic!("expected Use")
        };
        assert_eq!(group.len(), 3);
        assert!(group[0].alias.is_none());
        assert_eq!(group[1].alias.as_deref(), Some("p"));
        assert!(group[2].alias.is_none());
    }

    #[test]
    fn parses_use_with_path_keyword_root() {
        // `khi` as path root is valid per ADR-0005 + ADR-0024 (carried to 0071).
        let (_, item) = parse("use khi::utils::{helper}");
        let Item::Use { path, .. } = &item.node else {
            panic!("expected Use")
        };
        assert_eq!(path.segments, vec!["khi", "utils"]);
    }

    #[test]
    fn parses_use_with_self_root() {
        let (_, item) = parse("use self::helpers::{twice}");
        let Item::Use { path, .. } = &item.node else {
            panic!("expected Use")
        };
        assert_eq!(path.segments[0], "self");
    }

    #[test]
    fn parses_use_with_super_root() {
        let (_, item) = parse("use super::api::{handle}");
        let Item::Use { path, .. } = &item.node else {
            panic!("expected Use")
        };
        assert_eq!(path.segments[0], "super");
    }

    #[test]
    fn parses_use_group_trailing_comma() {
        let (_, item) = parse("use std::io::{println, print,}");
        let Item::Use { group, .. } = &item.node else {
            panic!("expected Use")
        };
        assert_eq!(group.len(), 2);
    }

    #[test]
    fn use_group_glob_is_rejected() {
        // ADR-0071 forbids `use X::{*}` (carried over from ADR-0005).
        let result = try_parse("use std::io::{*}");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn use_dotted_path_is_rejected() {
        // T-dot-path: `.` is no longer a path separator in `use` (ADR-0071).
        // Parsed as a whole program: `use std` consumes only the first segment,
        // and the leftover `.io::println` is not a valid top-level item → error.
        let (_, errors) = crate::parse("use std.io::println");
        assert!(
            !errors.is_empty(),
            "dotted use path must produce a parse error"
        );
    }

    #[test]
    fn use_at_eof_errors() {
        let result = try_parse("use");
        assert!(matches!(result, Err(ParseError::UnexpectedEof { .. })));
    }

    #[test]
    fn public_use_group_is_rejected() {
        // Re-exports of imported names are post-v0.2.x.
        let result = try_parse("public use std::io::{println}");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn use_with_path_keyword_root_is_accepted() {
        // ADR-0005 + ADR-0024 allow `khi` / `self` / `super` as path roots.
        let (_, item) = parse("use khi::utils::helper");
        let Item::Use { path, group } = &item.node else {
            panic!("expected Use")
        };
        assert_eq!(path.segments, vec!["khi", "utils", "helper"]);
        assert!(group.is_empty());
    }

    // ── ADR-0061 Tier 1: trait / implement / self (T2) ──────────────

    #[test]
    fn parses_trait_declaration() {
        let (parser, item) =
            parse("trait Comparable { function compare(self, other: Integer) -> Trit }");
        let Item::Trait { def } = &item.node else {
            panic!("expected Item::Trait, got {:?}", item.node)
        };
        assert_eq!(def.name, "Comparable");
        assert_eq!(def.methods.len(), 1);
        let method = &def.methods[0];
        assert_eq!(method.name, "compare");
        // params[0] is the bare `self` receiver; params[1] is `other`.
        assert_eq!(method.parameters.len(), 2);
        assert_eq!(method.parameters[0].name, "self");
        assert_eq!(method.parameters[1].name, "other");
        // T2.0 teeth: the receiver carries TypeExpr::SelfType, not a name.
        let self_ty = &parser
            .arena
            .type_expression(method.parameters[0].type_annotation)
            .node;
        assert!(
            matches!(self_ty, TypeExpr::SelfType),
            "self receiver must be TypeExpr::SelfType, got {self_ty:?}"
        );
        assert!(method.return_type.is_some());
    }

    #[test]
    fn parses_implementation_block() {
        let (parser, item) = parse(
            "implement Comparable for Integer { function compare(self, other: Integer) -> Trit = other }",
        );
        let Item::Implementation { def } = &item.node else {
            panic!("expected Item::Implementation, got {:?}", item.node)
        };
        assert_eq!(def.trait_name, "Comparable");
        // for_type is stored as a TypeId → resolve to the concrete type.
        let for_ty = &parser.arena.type_expression(def.for_type).node;
        assert!(
            matches!(for_ty, TypeExpr::Named(n) if n == "Integer"),
            "for_type must be Named(\"Integer\"), got {for_ty:?}"
        );
        assert_eq!(def.methods.len(), 1);
        assert_eq!(def.methods[0].name, "compare");
        // Impl methods carry a full body (unlike trait method signatures).
        assert!(matches!(
            def.methods[0].body,
            FunctionBody::Expression { .. }
        ));
    }

    #[test]
    fn self_param_resolves_to_self_type() {
        // T2.0/T2.4 teeth: a bare `self` first parameter resolves to the
        // SelfType marker. Poison parse_parameter (alloc Named("Self")
        // instead) → this assertion goes red.
        let (parser, item) = parse("function compare(self, other: Integer) -> Trit = other");
        let Item::Function { def } = &item.node else {
            panic!("expected Item::Function")
        };
        assert_eq!(def.parameters[0].name, "self");
        let self_ty = &parser
            .arena
            .type_expression(def.parameters[0].type_annotation)
            .node;
        assert!(matches!(self_ty, TypeExpr::SelfType), "got {self_ty:?}");
    }

    #[test]
    fn self_param_rejected_outside_first_position() {
        // T2.4 negative: `self` is only valid as the first parameter.
        let result = try_parse("function bad(x: Integer, self) -> Integer = x");
        assert!(
            matches!(result, Err(ParseError::UnexpectedToken { .. })),
            "self in 2nd position must be a parse error, got {result:?}"
        );
    }

    #[test]
    fn self_parses_as_expression() {
        // T2.5: `self` at expression position is a plain identifier.
        let (parser, item) = parse("function get_self(self) -> Integer = self");
        let Item::Function { def } = &item.node else {
            panic!("expected Item::Function")
        };
        let FunctionBody::Expression { expr } = def.body else {
            panic!("expected expression body")
        };
        let body = &parser.arena.expression(expr).node;
        assert!(
            matches!(body, triet_syntax::Expr::Identifier { name } if name == "self"),
            "self expression must be Identifier(\"self\"), got {body:?}"
        );
    }
}
