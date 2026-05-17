//! `triet.policy` — per-deploy capability resolution rules
//! ([ADR-0017 §3]).
//!
//! Sibling of [`Lockfile`](crate::Lockfile) (per-project pinned
//! resolution) and [`PackageManifest`](crate::PackageManifest)
//! (per-package source manifest). Where `triet.package` declares
//! *what a package needs*, `triet.policy` declares *what the deploy
//! grants*. The hook fires whenever a manifest entry resolves to
//! `Defer` ([`CapabilityLevel::Defer`](crate::CapabilityLevel)) —
//! that's when this file's rules decide.
//!
//! ADR-0017 §3 locks the grammar. Hand-rolled — same precedent as
//! `triet.lock` (ADR-0015 §6) and `triet.package` (ADR-0018 §1):
//! no serde dep, diff-friendly, sort-canonical:
//!
//! ```text
//! # triet.policy v1
//! format_version 1
//!
//! rule sys.io      lockfile +1
//! rule sys.io      ifacepin prompt
//! rule sys.io      fresh    prompt
//! rule sys.net.dns *        +1
//! rule dev.disk    *        -1
//!
//! default -1
//! ```
//!
//! Token style: **numeric** (`+1`/`0`/`-1`/`prompt`) for sysadmin
//! audit (compact, audit-grep-friendly). The audience-split with
//! `triet.package` textual tokens (`grant`/`ambient`/`deny`/`defer`)
//! is intentional per ADR-0018 §1.
//!
//! The parser is **strict** per [ADR-0017 Addendum §A] — security
//! boundary, whitelist-only. Same structural rules as
//! `triet.package`: rejects BOM, CRLF, inline `#`, Unicode
//! whitespace, oversize line / file. Reused via the shared
//! [`strict_parser`](crate::strict_parser) module.
//!
//! **Lookup precedence ([ADR-0017 §4]):** exact `origin` match beats
//! wildcard `*`. A `(cap_path, origin)` tuple may appear at most once;
//! duplicates fire `RuleConflict`. Wildcards live in their own
//! "origin slot" — `(cap_path, *)` is *not* a duplicate of
//! `(cap_path, lockfile)`.
//!
//! **Default decision:** the `default` line (zero or one) provides the
//! catch-all. Absent → implicit `-1` (Deny — fail-closed, ADR-0017 §3).
//! `default prompt` is intentionally rejected — defaults must be
//! static, otherwise every unconfigured cap reaches TTY.
//!
//! [ADR-0017 §3]: ../../../docs/decisions/0017-trilean-policy-hook.md
//! [ADR-0017 §4]: ../../../docs/decisions/0017-trilean-policy-hook.md
//! [ADR-0017 Addendum §A]: ../../../docs/decisions/0017-trilean-policy-hook.md#addendum--parser-strictness--tty-source--abstain-errata

use std::fs;
use std::path::Path;

use miette::Diagnostic;
use thiserror::Error;

use crate::error::{StoreError, StoreResult};
use crate::strict_parser::{LineViolation, for_each_directive_line};

/// `triet.policy` format version. Bump on incompatible wire change.
const FORMAT_VERSION: u32 = 1;

/// Per-rule origin matcher — exact match for one of the three
/// [`ResolutionOrigin`](crate::ResolutionOrigin) values, or
/// [`OriginMatcher::Any`] for the wildcard `*` token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OriginMatcher {
    /// `lockfile` — dependency came from `triet.lock` pinning.
    Lockfile,
    /// `ifacepin` — dependency resolved by `iface_hash_pin` in
    /// `triet.package`.
    IfacePin,
    /// `fresh` — dependency newly resolved this session.
    Fresh,
    /// `*` — match any origin. Has lower precedence than exact at
    /// lookup time (ADR-0017 §4).
    Any,
}

impl OriginMatcher {
    /// Source token as it appears in `triet.policy`.
    #[must_use]
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::Lockfile => "lockfile",
            Self::IfacePin => "ifacepin",
            Self::Fresh => "fresh",
            Self::Any => "*",
        }
    }

    /// Parse `lockfile` / `ifacepin` / `fresh` / `*`. None for any
    /// other token — caller maps to `UnknownOrigin`.
    #[must_use]
    pub fn from_token(s: &str) -> Option<Self> {
        match s {
            "lockfile" => Some(Self::Lockfile),
            "ifacepin" => Some(Self::IfacePin),
            "fresh" => Some(Self::Fresh),
            "*" => Some(Self::Any),
            _ => None,
        }
    }

    /// Stable sort key (`lockfile < ifacepin < fresh < *`). Used to
    /// emit canonical-order rules — sort by `(cap_path, origin_rank)`.
    pub(crate) const fn sort_rank(self) -> u8 {
        match self {
            Self::Lockfile => 0,
            Self::IfacePin => 1,
            Self::Fresh => 2,
            Self::Any => 3,
        }
    }
}

/// One outcome a `triet.policy` rule (or the `default` line) can
/// resolve to. Three Trit values plus the `Prompt` token for runtime
/// TTY fallback (ADR-0017 §4 Bước 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Decision {
    /// `+1` — grant. Cache + allow.
    Plus1,
    /// `0` — abstain. Cache as Deny; diagnostic distinguishes from `-1`.
    Zero,
    /// `-1` — deny. Cache + refuse.
    Minus1,
    /// `prompt` — force TTY prompt (runtime resolution). Disallowed
    /// in the `default` line — defaults must be static.
    Prompt,
}

impl Decision {
    /// Source token as it appears in `triet.policy`.
    #[must_use]
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::Plus1 => "+1",
            Self::Zero => "0",
            Self::Minus1 => "-1",
            Self::Prompt => "prompt",
        }
    }

    /// Parse `+1` / `0` / `-1` / `prompt`. None for any other token.
    #[must_use]
    pub fn from_token(s: &str) -> Option<Self> {
        match s {
            "+1" => Some(Self::Plus1),
            "0" => Some(Self::Zero),
            "-1" => Some(Self::Minus1),
            "prompt" => Some(Self::Prompt),
            _ => None,
        }
    }
}

/// One rule entry. Indexed by `(cap_path, origin)` tuple — at most
/// one rule per tuple, enforced at parse time via
/// [`PolicyError::RuleConflict`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyRule {
    /// Exact dotted module path. No globs at v0.6 (ADR-0017 §3) —
    /// use the `default` line for catch-all behaviour.
    pub cap_path: String,
    /// Origin selector — exact or wildcard.
    pub origin: OriginMatcher,
    /// Static or runtime-deferred decision.
    pub decision: Decision,
}

/// In-memory representation of a `triet.policy` file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PolicyRules {
    rules: Vec<PolicyRule>,
    /// `default` decision. `None` = implicit `-1` (ADR-0017 §3).
    /// Restricted to static decisions: `Plus1` / `Zero` / `Minus1`.
    default: Option<Decision>,
}

impl PolicyRules {
    /// Empty ruleset with implicit `default -1`. Equivalent to an
    /// absent `triet.policy` file (ADR-0017 §3).
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            rules: Vec::new(),
            default: None,
        }
    }

    /// All rules in canonical order: `(cap_path ASC, origin_rank ASC)`.
    #[must_use]
    pub fn rules(&self) -> &[PolicyRule] {
        &self.rules
    }

    /// The `default` decision if one was declared, otherwise `None`
    /// (callers MUST treat `None` as implicit `Decision::Minus1` per
    /// ADR-0017 §3).
    #[must_use]
    pub const fn default_decision(&self) -> Option<Decision> {
        self.default
    }

    /// The effective default — never returns `None`. `None` collapses
    /// to `Decision::Minus1` per ADR-0017 §3 (implicit Deny).
    #[must_use]
    pub const fn effective_default(&self) -> Decision {
        match self.default {
            Some(d) => d,
            None => Decision::Minus1,
        }
    }

    /// Insert a rule, or overwrite an existing rule that has the same
    /// `(cap_path, origin)` tuple. Returns `true` if a fresh slot was
    /// added; `false` if an existing rule was replaced.
    ///
    /// Used by the `G`/`D` permanent-write path in
    /// [`crate::DevTtyPrompt`] (v0.6.10) to record a user's prompt
    /// choice. The parser refuses duplicate tuples (ADR-0017 §3 — no
    /// merge/last-wins semantics), but programmatic callers explicitly
    /// asking to update an existing rule are different: the user
    /// re-prompted on the same path and chose a different decision.
    pub fn upsert_rule(&mut self, rule: PolicyRule) -> bool {
        if let Some(slot) = self
            .rules
            .iter_mut()
            .find(|r| r.cap_path == rule.cap_path && r.origin == rule.origin)
        {
            *slot = rule;
            false
        } else {
            self.rules.push(rule);
            true
        }
    }

    /// Override the `default` decision. Used by tests and by the
    /// `G`/`D` permanent-write path. Pass `None` to clear the explicit
    /// default (reverts to implicit `Minus1`).
    pub const fn set_default(&mut self, decision: Option<Decision>) {
        self.default = decision;
    }

    /// Look up the decision for `(cap_path, origin)` per ADR-0017 §4
    /// match precedence:
    ///
    /// 1. Exact origin match (`lockfile` / `ifacepin` / `fresh`) wins.
    /// 2. Wildcard rule with `origin = Any` matches if no exact rule.
    /// 3. No rule matched → `None` (caller falls back to
    ///    [`effective_default`](Self::effective_default)).
    ///
    /// Returns the matched [`Decision`] — `Decision::Prompt` is a
    /// valid result and indicates runtime fallback (ADR-0017 §4
    /// Bước 3, machinery lands in v0.6.9 / v0.6.10).
    #[must_use]
    pub fn find(&self, cap_path: &str, origin: OriginMatcher) -> Option<Decision> {
        // The exact-match branch deliberately ignores wildcard `Any`
        // — only exact matches qualify here.
        let exact = self
            .rules
            .iter()
            .find(|r| r.cap_path == cap_path && r.origin == origin);
        if let Some(rule) = exact {
            return Some(rule.decision);
        }
        // Fallback: wildcard match for the same `cap_path`.
        self.rules
            .iter()
            .find(|r| r.cap_path == cap_path && r.origin == OriginMatcher::Any)
            .map(|r| r.decision)
    }

    /// Parse a `triet.policy` source. See module docs for the format
    /// and strict-whitelist enforcement.
    ///
    /// # Errors
    /// Returns [`PolicyError`] on any rule violation — structural
    /// (BOM/CRLF/oversize/inline-`#`), [`UnknownOrigin`](PolicyError::UnknownOrigin),
    /// [`UnknownDecision`](PolicyError::UnknownDecision), or
    /// [`RuleConflict`](PolicyError::RuleConflict) for duplicate
    /// `(cap_path, origin)` tuples.
    // Single state machine dispatching `format_version` / `rule` /
    // `default`. Splitting into helpers would force a `ParseState`
    // struct without gaining readability — same trade-off as
    // `PackageManifest::parse`.
    #[allow(clippy::too_many_lines)]
    pub fn parse(text: &str) -> Result<Self, PolicyError> {
        let mut format_version_seen = false;
        let mut rules: Vec<PolicyRule> = Vec::new();
        let mut default: Option<Decision> = None;
        // Track each (cap_path, origin) first-seen line for the
        // RuleConflict diagnostic — ADR-0018 §3 requires the
        // "first declared at line N" hint.
        let mut first_seen: Vec<(String, OriginMatcher, usize)> = Vec::new();

        for_each_directive_line(text, |line_no, trimmed| {
            let mut parts = trimmed.split_ascii_whitespace();
            let head = parts.next().unwrap_or("");
            match head {
                "format_version" => {
                    if format_version_seen {
                        return Err(PolicyError::ConfigParse {
                            line: line_no,
                            reason: "duplicate `format_version` directive".into(),
                        });
                    }
                    let v: u32 = parts
                        .next()
                        .ok_or_else(|| PolicyError::ConfigParse {
                            line: line_no,
                            reason: "missing version after `format_version`".into(),
                        })?
                        .parse()
                        .map_err(|_| PolicyError::ConfigParse {
                            line: line_no,
                            reason: "version must be an integer".into(),
                        })?;
                    if parts.next().is_some() {
                        return Err(PolicyError::ConfigParse {
                            line: line_no,
                            reason: "extra fields after format_version".into(),
                        });
                    }
                    if v != FORMAT_VERSION {
                        return Err(PolicyError::UnsupportedFormatVersion {
                            found: v,
                            supported: FORMAT_VERSION,
                        });
                    }
                    format_version_seen = true;
                }
                "rule" => {
                    require_format_version(format_version_seen, line_no)?;
                    let cap_path = parts.next().ok_or_else(|| PolicyError::ConfigParse {
                        line: line_no,
                        reason: "missing cap_path".into(),
                    })?;
                    let origin_tok = parts.next().ok_or_else(|| PolicyError::ConfigParse {
                        line: line_no,
                        reason: "missing origin after cap_path".into(),
                    })?;
                    let decision_tok = parts.next().ok_or_else(|| PolicyError::ConfigParse {
                        line: line_no,
                        reason: "missing decision after origin".into(),
                    })?;
                    if parts.next().is_some() {
                        return Err(PolicyError::ConfigParse {
                            line: line_no,
                            reason: "extra fields after decision".into(),
                        });
                    }
                    validate_cap_path(cap_path, line_no)?;
                    let origin = OriginMatcher::from_token(origin_tok).ok_or_else(|| {
                        PolicyError::UnknownOrigin {
                            line: line_no,
                            token: origin_tok.to_owned(),
                        }
                    })?;
                    let decision = Decision::from_token(decision_tok).ok_or_else(|| {
                        PolicyError::UnknownDecision {
                            line: line_no,
                            token: decision_tok.to_owned(),
                        }
                    })?;

                    if let Some((_, _, prev_line)) = first_seen
                        .iter()
                        .find(|(p, o, _)| p == cap_path && *o == origin)
                    {
                        return Err(PolicyError::RuleConflict {
                            line: line_no,
                            cap_path: cap_path.to_owned(),
                            origin: origin.as_token().to_owned(),
                            first_line: *prev_line,
                        });
                    }
                    first_seen.push((cap_path.to_owned(), origin, line_no));
                    rules.push(PolicyRule {
                        cap_path: cap_path.to_owned(),
                        origin,
                        decision,
                    });
                }
                "default" => {
                    require_format_version(format_version_seen, line_no)?;
                    if default.is_some() {
                        return Err(PolicyError::ConfigParse {
                            line: line_no,
                            reason: "duplicate `default` directive".into(),
                        });
                    }
                    let decision_tok = parts.next().ok_or_else(|| PolicyError::ConfigParse {
                        line: line_no,
                        reason: "missing decision after `default`".into(),
                    })?;
                    if parts.next().is_some() {
                        return Err(PolicyError::ConfigParse {
                            line: line_no,
                            reason: "extra fields after default decision".into(),
                        });
                    }
                    let d = Decision::from_token(decision_tok).ok_or_else(|| {
                        PolicyError::UnknownDecision {
                            line: line_no,
                            token: decision_tok.to_owned(),
                        }
                    })?;
                    if d == Decision::Prompt {
                        return Err(PolicyError::ConfigParse {
                            line: line_no,
                            reason: "`default prompt` is not allowed — defaults must be \
                                     static; use an explicit `rule` instead"
                                .into(),
                        });
                    }
                    default = Some(d);
                }
                other => {
                    return Err(PolicyError::ConfigParse {
                        line: line_no,
                        reason: format!("unknown directive `{other}`"),
                    });
                }
            }
            Ok(())
        })?;

        if !format_version_seen {
            // Empty file (no directives) is valid — equivalent to
            // PolicyRules::empty(). Only flag missing format_version
            // when there's content that would otherwise be ambiguous.
            if !rules.is_empty() || default.is_some() {
                return Err(PolicyError::ConfigParse {
                    line: 0,
                    reason: "missing `format_version` directive".into(),
                });
            }
        }

        // Canonical sort on parse output so callers don't have to
        // think about input ordering.
        rules.sort_by(|a, b| {
            a.cap_path
                .cmp(&b.cap_path)
                .then_with(|| a.origin.sort_rank().cmp(&b.origin.sort_rank()))
        });

        Ok(Self { rules, default })
    }

    /// Render to canonical text. `parse(serialize(P)) == P` for every
    /// well-formed ruleset. Always emits the `format_version` line
    /// (even for empty rulesets) so a round-trip is unambiguous.
    #[must_use]
    pub fn serialize(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        out.push_str("# triet.policy — capability resolution rules.\n");
        out.push_str("# ADR-0017 §3 — sysadmin audit-friendly numeric tokens.\n");
        writeln!(&mut out, "format_version {FORMAT_VERSION}").expect("String write");

        let mut sorted = self.rules.clone();
        sorted.sort_by(|a, b| {
            a.cap_path
                .cmp(&b.cap_path)
                .then_with(|| a.origin.sort_rank().cmp(&b.origin.sort_rank()))
        });
        if !sorted.is_empty() {
            out.push('\n');
            for r in &sorted {
                writeln!(
                    &mut out,
                    "rule {} {} {}",
                    r.cap_path,
                    r.origin.as_token(),
                    r.decision.as_token(),
                )
                .expect("String write");
            }
        }
        if let Some(d) = self.default {
            out.push('\n');
            writeln!(&mut out, "default {}", d.as_token()).expect("String write");
        }
        out
    }

    /// Read a `triet.policy` from disk. NotFound is treated as
    /// `Ok(empty)` so the implicit-Deny default applies to
    /// projects that haven't opted in.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] for read failures other than
    /// NotFound, or [`StoreError::Policy`] if the file exists but
    /// parses with a rule violation.
    pub fn load(path: &Path) -> StoreResult<Self> {
        let text = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::empty()),
            Err(e) => return Err(StoreError::io(path.display().to_string(), e)),
        };
        Self::parse(&text).map_err(StoreError::from)
    }

    /// Write the policy to `path` atomically (sibling `.tmp` →
    /// `rename()`, ADR-0015 §5).
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] on write/rename failure.
    pub fn save(&self, path: &Path) -> StoreResult<()> {
        let text = self.serialize();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)
                .map_err(|e| StoreError::io(parent.display().to_string(), e))?;
        }
        let tmp = path.with_extension("policy.tmp");
        fs::write(&tmp, text.as_bytes())
            .map_err(|e| StoreError::io(tmp.display().to_string(), e))?;
        fs::rename(&tmp, path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            StoreError::io(path.display().to_string(), e)
        })
    }
}

/// Errors raised when parsing `triet.policy`. All four variants live
/// in the [`triet::capability::E2205`] namespace (ADR-0017 §6 —
/// six sub-variants total; the runtime-only `NonTTYDefer` /
/// `PromptCrash` land with the policy resolver in v0.6.9 / v0.6.10).
///
/// [`triet::capability::E2205`]: ../../../docs/decisions/0017-trilean-policy-hook.md
#[derive(Clone, Debug, Diagnostic, Error, PartialEq, Eq)]
pub enum PolicyError {
    /// `format_version` is newer than this reader supports.
    #[error("unsupported triet.policy format_version {found} (max supported: {supported})")]
    #[diagnostic(
        code(triet::capability::E2205),
        help("update the Triết toolchain — this `triet.policy` was written by a newer release")
    )]
    UnsupportedFormatVersion {
        /// Version found in the file.
        found: u32,
        /// Maximum version this reader understands.
        supported: u32,
    },

    /// Generic whitelist refusal — structural error or missing field.
    #[error("malformed triet.policy at line {line}: {reason}")]
    #[diagnostic(
        code(triet::capability::E2205),
        help(
            "the parser is strict by design — every shape outside the locked grammar is \
             rejected. See ADR-0017 Addendum §A."
        )
    )]
    ConfigParse {
        /// 1-based line number, or `0` for whole-file errors.
        line: usize,
        /// Human-readable cause.
        reason: String,
    },

    /// Duplicate `(cap_path, origin)` tuple. ADR-0017 §3 — no
    /// merge/last-wins semantics; duplicates must be resolved by hand.
    #[error(
        "duplicate rule for ({cap_path}, {origin}) at line {line} — first declared at \
         line {first_line}"
    )]
    #[diagnostic(
        code(triet::capability::E2205),
        help("remove one of the entries — duplicates are refused over guessed merge")
    )]
    RuleConflict {
        /// 1-based line number of the duplicate.
        line: usize,
        /// Cap path tuple element.
        cap_path: String,
        /// Origin tuple element rendered as source token.
        origin: String,
        /// 1-based line number where the same tuple was first declared.
        first_line: usize,
    },

    /// Origin field carries a token outside the locked set.
    #[error("unknown origin '{token}' at line {line} — expected: lockfile, ifacepin, fresh, *")]
    #[diagnostic(
        code(triet::capability::E2205),
        help("origin is one of the three ResolutionOrigin values or the wildcard `*`")
    )]
    UnknownOrigin {
        /// 1-based line number.
        line: usize,
        /// The offending origin token.
        token: String,
    },

    /// Decision field carries a token outside the locked set.
    #[error("unknown decision '{token}' at line {line} — expected: +1, 0, -1, prompt")]
    #[diagnostic(
        code(triet::capability::E2205),
        help("decision token is numeric Trit (+1/0/-1) or the literal `prompt`")
    )]
    UnknownDecision {
        /// 1-based line number.
        line: usize,
        /// The offending decision token.
        token: String,
    },
}

impl From<LineViolation> for PolicyError {
    fn from(v: LineViolation) -> Self {
        Self::ConfigParse {
            line: v.line,
            reason: v.kind.reason(),
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn require_format_version(seen: bool, line: usize) -> Result<(), PolicyError> {
    if seen {
        Ok(())
    } else {
        Err(PolicyError::ConfigParse {
            line,
            reason: "directive precedes `format_version`".into(),
        })
    }
}

/// Policy-side cap path validation — well-formed dotted ident only.
/// Unlike `triet.package`, root is NOT restricted to {sys, dev, usr}:
/// sysadmins may write rules for any namespace they audit.
fn validate_cap_path(s: &str, line: usize) -> Result<(), PolicyError> {
    if s.is_empty() {
        return Err(PolicyError::ConfigParse {
            line,
            reason: "cap_path must not be empty".into(),
        });
    }
    let segments: Vec<&str> = s.split('.').collect();
    if segments.iter().any(|seg| seg.is_empty()) {
        return Err(PolicyError::ConfigParse {
            line,
            reason: format!(
                "malformed cap_path `{s}` — empty segment (double `.` or trailing `.`?)"
            ),
        });
    }
    for seg in &segments {
        if !is_path_segment(seg) {
            return Err(PolicyError::ConfigParse {
                line,
                reason: format!("malformed cap_path segment `{seg}` in `{s}`"),
            });
        }
    }
    Ok(())
}

fn is_path_segment(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Happy path / round-trip ────────────────────────────────────

    fn happy_path() -> &'static str {
        "# triet.policy — capability resolution rules.\n\
         # ADR-0017 §3 — sysadmin audit-friendly numeric tokens.\n\
         format_version 1\n\
         \n\
         rule sys.io lockfile +1\n\
         rule sys.io ifacepin prompt\n\
         rule sys.io fresh prompt\n\
         rule sys.net.dns * +1\n\
         rule dev.disk * -1\n\
         \n\
         default -1\n"
    }

    #[test]
    fn parses_full_happy_path() {
        let p = PolicyRules::parse(happy_path()).expect("parse ok");
        assert_eq!(p.rules().len(), 5);
        assert_eq!(p.default_decision(), Some(Decision::Minus1));
    }

    #[test]
    fn roundtrip_preserves_content() {
        let p = PolicyRules::parse(happy_path()).expect("parse ok");
        let s = p.serialize();
        let p2 = PolicyRules::parse(&s).expect("re-parse ok");
        assert_eq!(p, p2);
    }

    #[test]
    fn roundtrip_empty_policy() {
        let p = PolicyRules::empty();
        let s = p.serialize();
        let p2 = PolicyRules::parse(&s).expect("re-parse ok");
        assert_eq!(p, p2);
    }

    #[test]
    fn serialize_sorts_rules() {
        let mut p = PolicyRules::empty();
        p.rules.push(PolicyRule {
            cap_path: "sys.io".into(),
            origin: OriginMatcher::Any,
            decision: Decision::Plus1,
        });
        p.rules.push(PolicyRule {
            cap_path: "dev.disk".into(),
            origin: OriginMatcher::Lockfile,
            decision: Decision::Minus1,
        });
        let s = p.serialize();
        let dev_pos = s.find("dev.disk").expect("dev.disk emitted");
        let sys_pos = s.find("sys.io").expect("sys.io emitted");
        assert!(dev_pos < sys_pos, "rules must sort by cap_path");
    }

    #[test]
    fn serialize_sorts_by_origin_rank_within_path() {
        // Same cap_path, mixed origins — origin_rank must order them
        // lockfile (0) < ifacepin (1) < fresh (2) < * (3).
        let p = PolicyRules {
            rules: vec![
                PolicyRule {
                    cap_path: "sys.io".into(),
                    origin: OriginMatcher::Any,
                    decision: Decision::Plus1,
                },
                PolicyRule {
                    cap_path: "sys.io".into(),
                    origin: OriginMatcher::Lockfile,
                    decision: Decision::Plus1,
                },
            ],
            default: None,
        };
        let s = p.serialize();
        let lock_pos = s.find("sys.io lockfile").expect("lockfile line");
        let star_pos = s.find("sys.io *").expect("any line");
        assert!(lock_pos < star_pos);
    }

    // ── Lookup precedence ─────────────────────────────────────────

    #[test]
    fn lookup_exact_wins_over_wildcard() {
        let p = PolicyRules::parse(
            "format_version 1\n\
             rule sys.io lockfile +1\n\
             rule sys.io * -1\n",
        )
        .expect("parse ok");
        assert_eq!(
            p.find("sys.io", OriginMatcher::Lockfile),
            Some(Decision::Plus1),
        );
        assert_eq!(
            p.find("sys.io", OriginMatcher::Fresh),
            Some(Decision::Minus1),
        );
    }

    #[test]
    fn lookup_wildcard_only() {
        let p = PolicyRules::parse(
            "format_version 1\n\
             rule sys.net.dns * +1\n",
        )
        .expect("parse ok");
        assert_eq!(
            p.find("sys.net.dns", OriginMatcher::Fresh),
            Some(Decision::Plus1),
        );
        assert_eq!(
            p.find("sys.net.dns", OriginMatcher::Lockfile),
            Some(Decision::Plus1),
        );
    }

    #[test]
    fn lookup_miss_returns_none() {
        let p = PolicyRules::parse(
            "format_version 1\n\
             rule sys.io lockfile +1\n",
        )
        .expect("parse ok");
        assert_eq!(p.find("dev.disk", OriginMatcher::Fresh), None);
    }

    #[test]
    fn effective_default_collapses_none_to_deny() {
        let p = PolicyRules::empty();
        assert_eq!(p.effective_default(), Decision::Minus1);
    }

    #[test]
    fn effective_default_uses_explicit_default() {
        let p = PolicyRules::parse(
            "format_version 1\n\
             default +1\n",
        )
        .expect("parse ok");
        assert_eq!(p.effective_default(), Decision::Plus1);
    }

    // ── Whitelist edges (ADR-0017 Addendum §A, shared via strict_parser) ──

    #[test]
    fn rejects_bom() {
        let err = PolicyRules::parse("\u{FEFF}format_version 1\n").expect_err("must reject");
        assert!(matches!(err, PolicyError::ConfigParse { line: 1, ref reason } if reason.contains("BOM")));
    }

    #[test]
    fn rejects_crlf() {
        let err = PolicyRules::parse("format_version 1\r\n").expect_err("must reject");
        assert!(matches!(err, PolicyError::ConfigParse { line: 1, ref reason } if reason.contains("CRLF")));
    }

    #[test]
    fn rejects_inline_comment() {
        let err = PolicyRules::parse("format_version 1 # nope\n").expect_err("must reject");
        assert!(matches!(err, PolicyError::ConfigParse { line: 1, ref reason } if reason.contains("inline")));
    }

    #[test]
    fn rejects_unicode_whitespace() {
        let err = PolicyRules::parse("format_version 1\nrule\u{00A0}sys.io lockfile +1\n")
            .expect_err("must reject");
        assert!(matches!(err, PolicyError::ConfigParse { line: 2, ref reason } if reason.contains("non-ASCII")));
    }

    // ── Policy-specific errors ────────────────────────────────────

    #[test]
    fn rejects_unknown_origin() {
        let err = PolicyRules::parse("format_version 1\nrule sys.io vibes +1\n")
            .expect_err("must reject");
        assert!(matches!(
            err,
            PolicyError::UnknownOrigin {
                line: 2,
                ref token,
            } if token == "vibes",
        ));
    }

    #[test]
    fn rejects_unknown_decision() {
        let err = PolicyRules::parse("format_version 1\nrule sys.io * maybe\n")
            .expect_err("must reject");
        assert!(matches!(
            err,
            PolicyError::UnknownDecision {
                line: 2,
                ref token,
            } if token == "maybe",
        ));
    }

    #[test]
    fn rejects_rule_conflict() {
        let err = PolicyRules::parse(
            "format_version 1\n\
             rule sys.io lockfile +1\n\
             rule sys.io lockfile -1\n",
        )
        .expect_err("must reject");
        assert!(matches!(
            err,
            PolicyError::RuleConflict {
                line: 3,
                first_line: 2,
                ref cap_path,
                ref origin,
            } if cap_path == "sys.io" && origin == "lockfile",
        ));
    }

    #[test]
    fn rule_conflict_distinguishes_wildcard_from_exact() {
        // (path, lockfile) and (path, *) are NOT duplicates — they
        // live in separate slots and follow ADR-0017 §4 precedence.
        let p = PolicyRules::parse(
            "format_version 1\n\
             rule sys.io lockfile +1\n\
             rule sys.io * -1\n",
        )
        .expect("parse ok");
        assert_eq!(p.rules().len(), 2);
    }

    #[test]
    fn rejects_default_prompt() {
        let err = PolicyRules::parse("format_version 1\ndefault prompt\n").expect_err("must reject");
        assert!(matches!(err, PolicyError::ConfigParse { line: 2, ref reason } if reason.contains("static")));
    }

    #[test]
    fn rejects_duplicate_default() {
        let err =
            PolicyRules::parse("format_version 1\ndefault +1\ndefault -1\n").expect_err("must reject");
        assert!(matches!(err, PolicyError::ConfigParse { line: 3, ref reason } if reason.contains("duplicate")));
    }

    #[test]
    fn rejects_unknown_directive() {
        let err = PolicyRules::parse("format_version 1\nfrobnicate yes\n").expect_err("must reject");
        assert!(matches!(err, PolicyError::ConfigParse { line: 2, ref reason } if reason.contains("frobnicate")));
    }

    #[test]
    fn rejects_directive_before_format_version() {
        let err = PolicyRules::parse("rule sys.io * +1\n").expect_err("must reject");
        assert!(matches!(err, PolicyError::ConfigParse { line: 1, ref reason } if reason.contains("precedes")));
    }

    #[test]
    fn rejects_duplicate_format_version() {
        let err =
            PolicyRules::parse("format_version 1\nformat_version 1\n").expect_err("must reject");
        assert!(matches!(err, PolicyError::ConfigParse { line: 2, ref reason } if reason.contains("duplicate")));
    }

    #[test]
    fn rejects_unsupported_format_version() {
        let err = PolicyRules::parse("format_version 2\n").expect_err("must reject");
        assert!(matches!(
            err,
            PolicyError::UnsupportedFormatVersion {
                found: 2,
                supported: 1,
            }
        ));
    }

    #[test]
    fn empty_file_parses_as_empty() {
        // Header comments only — equivalent to absent file.
        let p = PolicyRules::parse("# just a header\n").expect("empty ok");
        assert!(p.rules().is_empty());
        assert_eq!(p.default_decision(), None);
    }

    #[test]
    fn rejects_cap_path_with_invalid_segments() {
        let err =
            PolicyRules::parse("format_version 1\nrule sys..io * +1\n").expect_err("must reject");
        assert!(matches!(err, PolicyError::ConfigParse { line: 2, ref reason } if reason.contains("empty segment")));
    }

    #[test]
    fn accepts_non_sys_dev_usr_root() {
        // Unlike triet.package, policy doesn't restrict roots —
        // sysadmins can write rules for std/core too.
        let p = PolicyRules::parse("format_version 1\nrule std.io * +1\n").expect("parse ok");
        assert_eq!(p.rules().len(), 1);
    }

    #[test]
    fn accepts_all_origin_tokens() {
        let p = PolicyRules::parse(
            "format_version 1\n\
             rule a.b lockfile +1\n\
             rule a.b ifacepin +1\n\
             rule a.b fresh +1\n\
             rule a.b * +1\n",
        )
        .expect("parse ok");
        let origins: Vec<_> = p.rules().iter().map(|r| r.origin).collect();
        assert_eq!(
            origins,
            vec![
                OriginMatcher::Lockfile,
                OriginMatcher::IfacePin,
                OriginMatcher::Fresh,
                OriginMatcher::Any,
            ],
        );
    }

    #[test]
    fn accepts_all_decision_tokens() {
        let p = PolicyRules::parse(
            "format_version 1\n\
             rule sys.a * +1\n\
             rule sys.b * 0\n\
             rule sys.c * -1\n\
             rule sys.d * prompt\n",
        )
        .expect("parse ok");
        let decisions: Vec<_> = p.rules().iter().map(|r| r.decision).collect();
        assert_eq!(
            decisions,
            vec![
                Decision::Plus1,
                Decision::Zero,
                Decision::Minus1,
                Decision::Prompt,
            ],
        );
    }

    // ── File I/O round-trip ───────────────────────────────────────

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("triet.policy");
        let p = PolicyRules::parse(happy_path()).expect("parse ok");
        p.save(&path).expect("save ok");
        let p2 = PolicyRules::load(&path).expect("load ok");
        assert_eq!(p, p2);
    }

    #[test]
    fn upsert_then_save_round_trip() {
        // v0.6.x.review.1: pins the exact mutation→persist path used
        // by [`crate::DevTtyPrompt`]'s `G`/`D` permanent-write branch
        // (upsert_rule → save). Existing `save_load_roundtrip`
        // proves save+load equality on a freshly-parsed instance,
        // but never exercises a programmatic mutation in between.
        //
        // Insight surfaced by writing this test: `upsert_rule` appends
        // to the `Vec` (insertion order); `save` canonicalises via
        // sort-by-cap-path. So in-memory upserted state is NOT byte-
        // equal to the loaded state — but the rule survives, which is
        // the user-facing guarantee callers depend on.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("triet.policy");

        let mut p = PolicyRules::parse(happy_path()).expect("parse ok");
        let inserted = p.upsert_rule(PolicyRule {
            cap_path: "dev.gpu".into(),
            origin: OriginMatcher::Lockfile,
            decision: Decision::Plus1,
        });
        assert!(inserted, "fresh slot expected — dev.gpu absent from happy_path");

        p.save(&path).expect("save ok");
        let p2 = PolicyRules::load(&path).expect("load ok");

        // Survives the round-trip (regardless of in-mem vs canonical
        // disk ordering).
        assert!(
            p2.rules().iter().any(|r| r.cap_path == "dev.gpu"
                && r.origin == OriginMatcher::Lockfile
                && matches!(r.decision, Decision::Plus1)),
            "upserted (dev.gpu, lockfile, +1) missing after reload",
        );

        // Canonical form is a fixed point — save→load→save→load is
        // identity once we're past the first save. Proves the
        // permanent-write path is deterministic across sessions.
        p2.save(&path).expect("re-save ok");
        let p3 = PolicyRules::load(&path).expect("re-load ok");
        assert_eq!(p2, p3, "canonical form is not a fixed point");
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("missing.policy");
        let p = PolicyRules::load(&path).expect("load ok");
        assert_eq!(p, PolicyRules::empty());
    }
}
