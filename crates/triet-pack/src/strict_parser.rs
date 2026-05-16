//! Shared whitelist-only line tokenizer for v0.6 capability files.
//!
//! [ADR-0017 Addendum §A] locks the structural rules for both
//! [`PackageManifest`](crate::PackageManifest) (`triet.package`, v0.6.5)
//! and the upcoming [`PolicyRules`](crate::PolicyRules) (`triet.policy`,
//! v0.6.6). With two proven consumers, the line-level checks live here
//! so the two parsers can't drift.
//!
//! **What this module owns:**
//!
//! - File-size cap (1 MiB)
//! - Per-line byte cap (4096)
//! - BOM rejection (`U+FEFF` at byte 0)
//! - CRLF rejection (line ends with `\r`)
//! - Inline-`#` rejection (only line-start `#` is a comment)
//! - Non-ASCII byte rejection outside comments
//! - ASCII-whitespace-only trimming (`0x20` / `0x09`)
//!
//! **What it doesn't own:**
//!
//! - Directive dispatch (consumer-specific: `format_version`, `name`,
//!   `requires`, `rule`, `default`, …)
//! - Required-field gating (consumer-specific: name + version vs.
//!   format_version-only, etc.)
//! - Field-value validation (semver, hash hex, cap path, level token, …)
//!
//! [ADR-0017 Addendum §A]: ../../../docs/decisions/0017-trilean-policy-hook.md#addendum--parser-strictness--tty-source--abstain-errata

/// Per-line byte cap — ADR-0017 Addendum §A DoS prevention.
pub(crate) const MAX_LINE_LEN: usize = 4096;

/// Per-file byte cap — ADR-0017 Addendum §A DoS prevention.
pub(crate) const MAX_FILE_SIZE: usize = 1024 * 1024;

/// One structural violation flagged by [`for_each_directive_line`].
/// Carries enough info for the consumer to build its own diagnostic
/// without re-implementing the rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LineViolation {
    /// 1-based line number. `0` for file-level violations
    /// (`FileTooBig`).
    pub line: usize,
    /// What rule was broken.
    pub kind: StrictParseViolation,
}

/// The closed set of whitelist violations. Each consumer maps these
/// into its own `Malformed`/`ConfigParse` variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StrictParseViolation {
    /// File exceeded [`MAX_FILE_SIZE`].
    FileTooBig,
    /// BOM (`U+FEFF`) at byte 0.
    Bom,
    /// CRLF — line ended with `\r`.
    Crlf,
    /// Line exceeded [`MAX_LINE_LEN`].
    LineTooLong,
    /// Non-ASCII byte appeared outside a comment.
    NonAscii {
        /// The offending byte.
        byte: u8,
    },
    /// `#` appeared mid-line. Only a line-start `#` is a comment.
    InlineComment,
}

impl StrictParseViolation {
    /// Human-readable reason — identical wording across consumers so
    /// `triet.package` and `triet.policy` errors look uniform.
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::FileTooBig => format!("file exceeds {MAX_FILE_SIZE} byte cap"),
            Self::Bom => "BOM (U+FEFF) is not allowed".into(),
            Self::Crlf => "CRLF line endings are not allowed (use LF)".into(),
            Self::LineTooLong => format!("line exceeds {MAX_LINE_LEN} byte cap"),
            Self::NonAscii { byte } => format!(
                "non-ASCII byte 0x{byte:02X} outside comment — Unicode identifiers \
                 require XID support, deferred"
            ),
            Self::InlineComment => "inline `#` comments are not allowed".into(),
        }
    }
}

/// Trim ASCII space (`0x20`) and tab (`0x09`). Anything else — including
/// Unicode whitespace and control chars — is intentionally preserved so
/// downstream non-ASCII byte detection can flag it.
pub(crate) fn trim_ascii_ws(s: &str) -> &str {
    let bytes = s.as_bytes();
    let start = bytes
        .iter()
        .position(|b| !is_ascii_ws(*b))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !is_ascii_ws(*b))
        .map_or(start, |p| p + 1);
    &s[start..end]
}

const fn is_ascii_ws(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// Iterate every directive line — non-blank, non-comment lines that
/// pass every structural whitelist rule. File-level checks fire
/// before any callback. Per-line checks fire before the callback for
/// that line.
///
/// `line_no` passed to the callback is 1-based. The callback's `E`
/// must impl `From<LineViolation>` so this module's violations and
/// the consumer's own field/dispatch errors share a single return type.
///
/// # Errors
/// Returns `E` if any structural rule fires, or if the callback
/// returns an error for any directive line.
pub(crate) fn for_each_directive_line<E, F>(text: &str, mut callback: F) -> Result<(), E>
where
    E: From<LineViolation>,
    F: FnMut(usize, &str) -> Result<(), E>,
{
    if text.len() > MAX_FILE_SIZE {
        return Err(LineViolation {
            line: 0,
            kind: StrictParseViolation::FileTooBig,
        }
        .into());
    }
    if text.starts_with('\u{FEFF}') {
        return Err(LineViolation {
            line: 1,
            kind: StrictParseViolation::Bom,
        }
        .into());
    }

    // `split('\n')` preserves trailing `\r`, so CRLF is detectable.
    for (idx, raw) in text.split('\n').enumerate() {
        let line_no = idx + 1;

        if raw.len() > MAX_LINE_LEN {
            return Err(LineViolation {
                line: line_no,
                kind: StrictParseViolation::LineTooLong,
            }
            .into());
        }
        if raw.ends_with('\r') {
            return Err(LineViolation {
                line: line_no,
                kind: StrictParseViolation::Crlf,
            }
            .into());
        }

        let trimmed = trim_ascii_ws(raw);
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(byte) = trimmed.bytes().find(|b| !is_strict_ascii(*b)) {
            return Err(LineViolation {
                line: line_no,
                kind: StrictParseViolation::NonAscii { byte },
            }
            .into());
        }

        if trimmed.contains('#') {
            return Err(LineViolation {
                line: line_no,
                kind: StrictParseViolation::InlineComment,
            }
            .into());
        }

        callback(line_no, trimmed)?;
    }

    Ok(())
}

/// Allow ASCII printable + space + tab. Reject anything else (control
/// chars, non-ASCII bytes). Comments may carry UTF-8 — this check is
/// only applied to non-comment lines.
const fn is_strict_ascii(b: u8) -> bool {
    matches!(b, 0x20..=0x7E | b'\t')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only error wrapper to exercise the `From<LineViolation>`
    /// contract without dragging a real consumer's error type in.
    #[derive(Debug, PartialEq, Eq)]
    enum TestErr {
        Structural(LineViolation),
        Callback(usize, String),
    }

    impl From<LineViolation> for TestErr {
        fn from(v: LineViolation) -> Self {
            Self::Structural(v)
        }
    }

    fn collect_lines(text: &str) -> Result<Vec<(usize, String)>, TestErr> {
        let mut out = Vec::new();
        for_each_directive_line(text, |line_no, trimmed| {
            out.push((line_no, trimmed.to_owned()));
            Ok::<(), TestErr>(())
        })?;
        Ok(out)
    }

    #[test]
    fn skips_blanks_and_comments() {
        let text = "\n# header\n  # indented comment\nfoo bar\n\n  \nbaz\n";
        let lines = collect_lines(text).expect("ok");
        assert_eq!(
            lines,
            vec![(4, "foo bar".to_owned()), (7, "baz".to_owned())],
        );
    }

    #[test]
    fn rejects_bom() {
        let err = collect_lines("\u{FEFF}foo\n").expect_err("must reject");
        assert!(matches!(
            err,
            TestErr::Structural(LineViolation {
                line: 1,
                kind: StrictParseViolation::Bom,
            }),
        ));
    }

    #[test]
    fn rejects_crlf() {
        let err = collect_lines("foo\r\n").expect_err("must reject");
        assert!(matches!(
            err,
            TestErr::Structural(LineViolation {
                line: 1,
                kind: StrictParseViolation::Crlf,
            }),
        ));
    }

    #[test]
    fn rejects_inline_comment() {
        let err = collect_lines("foo bar # nope\n").expect_err("must reject");
        assert!(matches!(
            err,
            TestErr::Structural(LineViolation {
                line: 1,
                kind: StrictParseViolation::InlineComment,
            }),
        ));
    }

    #[test]
    fn rejects_unicode_whitespace() {
        let err = collect_lines("foo\u{00A0}bar\n").expect_err("must reject");
        assert!(matches!(
            err,
            TestErr::Structural(LineViolation {
                line: 1,
                kind: StrictParseViolation::NonAscii { .. },
            }),
        ));
    }

    #[test]
    fn rejects_oversize_line() {
        let mut text = String::from("a");
        text.extend(std::iter::repeat_n('b', MAX_LINE_LEN));
        text.push('\n');
        let err = collect_lines(&text).expect_err("must reject");
        assert!(matches!(
            err,
            TestErr::Structural(LineViolation {
                line: 1,
                kind: StrictParseViolation::LineTooLong,
            }),
        ));
    }

    #[test]
    fn rejects_oversize_file() {
        let text = "# pad\n".repeat(MAX_FILE_SIZE / 6 + 1);
        let err = collect_lines(&text).expect_err("must reject");
        assert!(matches!(
            err,
            TestErr::Structural(LineViolation {
                line: 0,
                kind: StrictParseViolation::FileTooBig,
            }),
        ));
    }

    #[test]
    fn callback_errors_propagate() {
        let result: Result<(), TestErr> = for_each_directive_line("foo\nbar\n", |line, _| {
            if line == 2 {
                Err(TestErr::Callback(line, "stop".into()))
            } else {
                Ok(())
            }
        });
        assert_eq!(result, Err(TestErr::Callback(2, "stop".into())));
    }

    #[test]
    fn reason_strings_uniform_across_violations() {
        // Smoke test that every variant produces a non-empty reason —
        // important because consumers route these straight into miette.
        for v in [
            StrictParseViolation::FileTooBig,
            StrictParseViolation::Bom,
            StrictParseViolation::Crlf,
            StrictParseViolation::LineTooLong,
            StrictParseViolation::NonAscii { byte: 0xA0 },
            StrictParseViolation::InlineComment,
        ] {
            assert!(!v.reason().is_empty());
        }
    }

    #[test]
    fn trim_ascii_ws_handles_tabs_and_spaces() {
        assert_eq!(trim_ascii_ws("  foo\t"), "foo");
        assert_eq!(trim_ascii_ws("\t\tfoo bar\t \t"), "foo bar");
        assert_eq!(trim_ascii_ws("   "), "");
        assert_eq!(trim_ascii_ws(""), "");
        // Unicode WS preserved so caller can flag it.
        assert_eq!(trim_ascii_ws("\u{00A0}foo"), "\u{00A0}foo");
    }
}
