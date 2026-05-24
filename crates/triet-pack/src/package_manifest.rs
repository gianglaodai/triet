//! `triet.package` — per-package source manifest (ADR-0018 §1).
//!
//! Sibling of `triet.lock` (per-project pinned resolution, [`Lockfile`])
//! and `triet.policy` (per-deploy capability rules, v0.6.6).
//!
//! ADR-0018 §1 locks the grammar. Hand-rolled — same precedent as
//! [`Lockfile`] (ADR-0015 §6): no serde dep, diff-friendly,
//! sort-canonical:
//!
//! ```text
//! # triet.package — generated, hand-editable.
//! format_version 1
//! name myapp
//! version 0.1.0
//!
//! requires dev.disk deny
//! requires sys.io grant
//! requires sys.net.dns defer
//! requires usr.somelib ambient
//!
//! dep libdns 1.2.3 1.3.0 5c92ab17d4e8c1f6a3b8d2e5c97014b6f3e8d2a4c5b1f9e6d8c3a2b4f7e1d503
//! dep libtls 0.4.0 0.5.0 d041e8b9c5a2f4e8d1c6b3a597e0d4c8b1a3f6e9d2c5a7b4f1e8d3c0a6f9b2e4
//! ```
//!
//! Parser is **strict** per [ADR-0017 Addendum §A] — security boundary,
//! whitelist-only. Any shape outside the locked grammar → refuse-to-load.
//! Unlike [`Lockfile::parse`], this parser:
//!
//! - Rejects BOM (U+FEFF) at byte 0
//! - Rejects CRLF (must use LF)
//! - Rejects inline `#` (only line-start `#` is a comment)
//! - Rejects Unicode whitespace anywhere (only ASCII 0x20 / 0x09)
//! - Rejects lines > 4096 bytes
//! - Rejects files > 1 MiB
//! - Rejects unknown directives, duplicate `format_version`, etc.
//!
//! **v0.6.5 identifier scope:** ASCII-only identifiers
//! (`[A-Za-z_][A-Za-z0-9_]*`). [SPEC §1.3] permits Unicode (UAX #31 XID)
//! and [ADR-0017 Addendum §A] expects it; the upgrade lands when there's
//! demand. Forward-compatible — ASCII manifests stay valid.
//!
//! [ADR-0017 Addendum §A]: ../../../docs/decisions/0017-trilean-policy-hook.md#addendum--parser-strictness--tty-source--abstain-errata
//! [SPEC §1.3]: ../../../SPEC.md

use std::fs;
use std::path::Path;

use miette::Diagnostic;
use thiserror::Error;

use crate::error::{StoreError, StoreResult};
use crate::hash::{IFACE_HASH_LEN, IfaceHash};
use crate::strict_parser::{LineViolation, for_each_directive_line};
use crate::types::{CapabilityClaim, CapabilityLevel, Dep, SemVer};

/// `triet.package` format version. Bump on incompatible wire change.
const FORMAT_VERSION: u32 = 1;

/// Capability roots that require explicit grant (ADR-0016 §5 rule 3).
/// `std`/`core` are ambient — not declared in `requires`. Intra-package
/// roots (`crate`/`self`/`super`) never appear at this layer.
const RESERVED_CAP_ROOTS: &[&str] = &["sys", "dev", "usr"];

/// Parsed source manifest for one package. Mirrors the wire-side
/// [`crate::AbiMetadata`] but at the *source* layer — what the author
/// writes, what `triet build` would read before emitting a `.khi`.
///
/// ADR-0018 §1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageManifest {
    /// Package name. ASCII identifier `[a-z][a-z0-9_]*` at v0.6
    /// (packages stay URL-safe for future remote registries).
    pub name: String,
    /// Package version triple.
    pub version: SemVer,
    /// Capability claims (ADR-0016 §1, ADR-0018 §1). Sort-by-`cap_path`
    /// canonical on serialize.
    pub requires: Vec<CapabilityClaim>,
    /// Dependencies. Sort-by-`pkg_name` canonical on serialize.
    pub deps: Vec<Dep>,
}

impl PackageManifest {
    /// Build an empty manifest with just name + version. Useful as a
    /// starting point for builders or test fixtures.
    #[must_use]
    pub fn new(name: impl Into<String>, version: SemVer) -> Self {
        Self {
            name: name.into(),
            version,
            requires: Vec::new(),
            deps: Vec::new(),
        }
    }

    /// Parse a `triet.package` source. See module docs for the format
    /// and the strict-whitelist enforcement.
    ///
    /// # Errors
    /// Returns [`PackageManifestError`] on any rule violation —
    /// structural (BOM/CRLF/oversize/inline-`#`), missing/duplicate
    /// directive, malformed line, or [ADR-0016 §5] capability-root
    /// violation.
    // The directive dispatch is a single state machine with five
    // arms (format_version / name / version / requires / dep);
    // splitting it across helper fns would force a `ParseState`
    // struct without gaining readability. Keep it inline.
    #[allow(clippy::too_many_lines)]
    pub fn parse(text: &str) -> Result<Self, PackageManifestError> {
        let mut format_version_seen = false;
        let mut name: Option<String> = None;
        let mut version: Option<SemVer> = None;
        let mut requires: Vec<CapabilityClaim> = Vec::new();
        let mut deps: Vec<Dep> = Vec::new();

        for_each_directive_line(text, |line_no, trimmed| {
            // ── Directive dispatch ─────────────────────────────────
            let mut parts = trimmed.split_ascii_whitespace();
            let head = parts.next().unwrap_or("");

            match head {
                "format_version" => {
                    if format_version_seen {
                        return Err(PackageManifestError::Malformed {
                            line: line_no,
                            reason: "duplicate `format_version` directive".into(),
                        });
                    }
                    let v: u32 = parts
                        .next()
                        .ok_or_else(|| PackageManifestError::Malformed {
                            line: line_no,
                            reason: "missing version after `format_version`".into(),
                        })?
                        .parse()
                        .map_err(|_| PackageManifestError::Malformed {
                            line: line_no,
                            reason: "version must be an integer".into(),
                        })?;
                    if parts.next().is_some() {
                        return Err(PackageManifestError::Malformed {
                            line: line_no,
                            reason: "extra fields after format_version".into(),
                        });
                    }
                    if v != FORMAT_VERSION {
                        return Err(PackageManifestError::UnsupportedFormatVersion {
                            found: v,
                            supported: FORMAT_VERSION,
                        });
                    }
                    format_version_seen = true;
                }

                "name" => {
                    require_format_version(format_version_seen, line_no)?;
                    if name.is_some() {
                        return Err(PackageManifestError::Malformed {
                            line: line_no,
                            reason: "duplicate `name` directive".into(),
                        });
                    }
                    let n = parts
                        .next()
                        .ok_or_else(|| PackageManifestError::Malformed {
                            line: line_no,
                            reason: "missing pkg name".into(),
                        })?;
                    if parts.next().is_some() {
                        return Err(PackageManifestError::Malformed {
                            line: line_no,
                            reason: "extra fields after name".into(),
                        });
                    }
                    if !is_pkg_name(n) {
                        return Err(PackageManifestError::Malformed {
                            line: line_no,
                            reason: format!(
                                "invalid pkg name `{n}` — expected lowercase ASCII ident \
                                 (`[a-z][a-z0-9_]*`)"
                            ),
                        });
                    }
                    name = Some(n.to_owned());
                }

                "version" => {
                    require_format_version(format_version_seen, line_no)?;
                    if version.is_some() {
                        return Err(PackageManifestError::Malformed {
                            line: line_no,
                            reason: "duplicate `version` directive".into(),
                        });
                    }
                    let v_str = parts
                        .next()
                        .ok_or_else(|| PackageManifestError::Malformed {
                            line: line_no,
                            reason: "missing version".into(),
                        })?;
                    if parts.next().is_some() {
                        return Err(PackageManifestError::Malformed {
                            line: line_no,
                            reason: "extra fields after version".into(),
                        });
                    }
                    let v = parse_semver(v_str).ok_or_else(|| PackageManifestError::Malformed {
                        line: line_no,
                        reason: format!("bad version `{v_str}`"),
                    })?;
                    version = Some(v);
                }

                "requires" => {
                    require_format_version(format_version_seen, line_no)?;
                    let cap_path = parts
                        .next()
                        .ok_or_else(|| PackageManifestError::Malformed {
                            line: line_no,
                            reason: "missing cap_path".into(),
                        })?;
                    let level_tok =
                        parts
                            .next()
                            .ok_or_else(|| PackageManifestError::Malformed {
                                line: line_no,
                                reason: "missing level after cap_path".into(),
                            })?;
                    if parts.next().is_some() {
                        return Err(PackageManifestError::Malformed {
                            line: line_no,
                            reason: "extra fields after level".into(),
                        });
                    }
                    validate_cap_path(cap_path, line_no)?;
                    let level = parse_level_token(level_tok).ok_or_else(|| {
                        PackageManifestError::Malformed {
                            line: line_no,
                            reason: format!(
                                "unknown level `{level_tok}` — expected one of: \
                                     grant, ambient, deny, defer"
                            ),
                        }
                    })?;
                    if requires.iter().any(|c| c.cap_path == cap_path) {
                        return Err(PackageManifestError::Malformed {
                            line: line_no,
                            reason: format!("duplicate `requires` for `{cap_path}`"),
                        });
                    }
                    requires.push(CapabilityClaim {
                        cap_path: cap_path.to_owned(),
                        level,
                    });
                }

                "dep" => {
                    require_format_version(format_version_seen, line_no)?;
                    let dep_name = parts
                        .next()
                        .ok_or_else(|| PackageManifestError::Malformed {
                            line: line_no,
                            reason: "missing dep name".into(),
                        })?;
                    let min_str = parts
                        .next()
                        .ok_or_else(|| PackageManifestError::Malformed {
                            line: line_no,
                            reason: "missing dep min version".into(),
                        })?;
                    let max_str = parts
                        .next()
                        .ok_or_else(|| PackageManifestError::Malformed {
                            line: line_no,
                            reason: "missing dep max_excl version".into(),
                        })?;
                    let hash_str = parts
                        .next()
                        .ok_or_else(|| PackageManifestError::Malformed {
                            line: line_no,
                            reason: "missing dep iface_hash".into(),
                        })?;
                    if parts.next().is_some() {
                        return Err(PackageManifestError::Malformed {
                            line: line_no,
                            reason: "extra fields after dep iface_hash".into(),
                        });
                    }
                    if !is_pkg_name(dep_name) {
                        return Err(PackageManifestError::Malformed {
                            line: line_no,
                            reason: format!(
                                "invalid dep name `{dep_name}` — expected lowercase ASCII \
                                 ident"
                            ),
                        });
                    }
                    let min =
                        parse_semver(min_str).ok_or_else(|| PackageManifestError::Malformed {
                            line: line_no,
                            reason: format!("bad dep min version `{min_str}`"),
                        })?;
                    let max =
                        parse_semver(max_str).ok_or_else(|| PackageManifestError::Malformed {
                            line: line_no,
                            reason: format!("bad dep max version `{max_str}`"),
                        })?;
                    let hash_bytes = parse_hash::<IFACE_HASH_LEN>(hash_str).ok_or_else(|| {
                        PackageManifestError::Malformed {
                            line: line_no,
                            reason: "bad iface_hash hex (expected 64 lowercase hex chars)".into(),
                        }
                    })?;
                    if deps.iter().any(|d| d.pkg_name == dep_name) {
                        return Err(PackageManifestError::Malformed {
                            line: line_no,
                            reason: format!("duplicate `dep` for `{dep_name}`"),
                        });
                    }
                    deps.push(Dep {
                        pkg_name: dep_name.to_owned(),
                        version_min: min,
                        version_max_exclusive: max,
                        iface_hash_pin: IfaceHash(hash_bytes),
                    });
                }

                other => {
                    return Err(PackageManifestError::Malformed {
                        line: line_no,
                        reason: format!("unknown directive `{other}`"),
                    });
                }
            }
            Ok(())
        })?;

        // ── Required-field gates ───────────────────────────────────
        if !format_version_seen {
            return Err(PackageManifestError::Malformed {
                line: 0,
                reason: "missing `format_version` directive".into(),
            });
        }
        let name = name.ok_or_else(|| PackageManifestError::Malformed {
            line: 0,
            reason: "missing `name` directive".into(),
        })?;
        let version = version.ok_or_else(|| PackageManifestError::Malformed {
            line: 0,
            reason: "missing `version` directive".into(),
        })?;

        // Canonical sort on parse output too, so callers don't have to
        // think about input ordering.
        requires.sort_by(|a, b| a.cap_path.cmp(&b.cap_path));
        deps.sort_by(|a, b| a.pkg_name.cmp(&b.pkg_name));

        Ok(Self {
            name,
            version,
            requires,
            deps,
        })
    }

    /// Render to canonical text form. `parse(serialize(M)) == M` for
    /// every well-formed manifest. Always emits the `format_version`
    /// line; blank lines separate header / requires / deps for human
    /// readability.
    #[must_use]
    pub fn serialize(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        out.push_str("# triet.package — generated, hand-editable.\n");
        out.push_str("# ADR-0018 §1 — capability claims + deps.\n");
        writeln!(&mut out, "format_version {FORMAT_VERSION}").expect("String write");
        writeln!(&mut out, "name {}", self.name).expect("String write");
        writeln!(
            &mut out,
            "version {}.{}.{}",
            self.version.major, self.version.minor, self.version.patch,
        )
        .expect("String write");

        let mut requires = self.requires.clone();
        requires.sort_by(|a, b| a.cap_path.cmp(&b.cap_path));
        if !requires.is_empty() {
            out.push('\n');
            for c in &requires {
                writeln!(&mut out, "requires {} {}", c.cap_path, level_token(c.level))
                    .expect("String write");
            }
        }

        let mut deps = self.deps.clone();
        deps.sort_by(|a, b| a.pkg_name.cmp(&b.pkg_name));
        if !deps.is_empty() {
            out.push('\n');
            for d in &deps {
                writeln!(
                    &mut out,
                    "dep {} {}.{}.{} {}.{}.{} {}",
                    d.pkg_name,
                    d.version_min.major,
                    d.version_min.minor,
                    d.version_min.patch,
                    d.version_max_exclusive.major,
                    d.version_max_exclusive.minor,
                    d.version_max_exclusive.patch,
                    hex_encode(&d.iface_hash_pin.0),
                )
                .expect("String write");
            }
        }
        out
    }

    /// Read a `triet.package` from disk.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] for read failures or
    /// [`StoreError::PackageManifest`] when the file parses with a
    /// rule violation.
    pub fn load(path: &Path) -> StoreResult<Self> {
        let text =
            fs::read_to_string(path).map_err(|e| StoreError::io(path.display().to_string(), e))?;
        Self::parse(&text).map_err(StoreError::from)
    }

    /// Write the manifest to `path` atomically (sibling `.tmp` →
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
        let tmp = path.with_extension("package.tmp");
        fs::write(&tmp, text.as_bytes())
            .map_err(|e| StoreError::io(tmp.display().to_string(), e))?;
        fs::rename(&tmp, path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            StoreError::io(path.display().to_string(), e)
        })
    }
}

/// Errors raised when parsing `triet.package`. Wrapped by
/// [`StoreError::PackageManifest`] when raised through
/// [`PackageManifest::load`].
///
/// All diagnostics live in the [`triet::capability::E22XX`] namespace
/// (ADR-0016 §6). Two dedicated codes at v0.6.5:
///
/// - [`E2208`] — general loader refuse-to-load (ADR-0018 §5)
/// - [`E2206`] — capability root violation (ADR-0016 §6)
///
/// [`triet::capability::E22XX`]: ../../../docs/decisions/0016-capability-type-system.md
/// [`E2208`]: ../../../docs/decisions/0018-capability-loader-semantics.md
/// [`E2206`]: ../../../docs/decisions/0016-capability-type-system.md
#[derive(Clone, Debug, Diagnostic, Error, PartialEq, Eq)]
pub enum PackageManifestError {
    /// `format_version` is newer than this reader supports.
    #[error("unsupported triet.package format_version {found} (max supported: {supported})")]
    #[diagnostic(
        code(triet::capability::E2208),
        help("update the Triết toolchain — this `triet.package` was written by a newer release")
    )]
    UnsupportedFormatVersion {
        /// Version found in the file.
        found: u32,
        /// Maximum version this reader understands.
        supported: u32,
    },

    /// A line failed to parse. Whitelist-only enforcement per
    /// ADR-0017 Addendum §A — any shape outside the locked grammar
    /// refuses-to-load.
    #[error("malformed triet.package at line {line}: {reason}")]
    #[diagnostic(
        code(triet::capability::E2208),
        help(
            "the parser is strict by design — every shape outside the locked grammar is \
             rejected. See ADR-0017 Addendum §A."
        )
    )]
    Malformed {
        /// 1-based line number, or `0` for whole-file errors (missing
        /// required directive, file-size cap exceeded).
        line: usize,
        /// Human-readable cause.
        reason: String,
    },

    /// `cap_path` root is not one of `sys`, `dev`, `usr` (ADR-0016 §5
    /// rule 3). Distinct from generic [`Malformed`](Self::Malformed)
    /// so the diagnostic can call out the architectural rule.
    #[error("invalid capability root `{root}` at line {line}: must be one of sys, dev, usr")]
    #[diagnostic(
        code(triet::capability::E2206),
        help(
            "only sys, dev, usr roots require explicit grants. std/core are ambient; \
             crate/self/super are intra-package and never appear in requires."
        )
    )]
    InvalidCapabilityRoot {
        /// 1-based line number.
        line: usize,
        /// The offending root segment.
        root: String,
    },
}

// ── Helpers ─────────────────────────────────────────────────────────

impl From<LineViolation> for PackageManifestError {
    fn from(v: LineViolation) -> Self {
        Self::Malformed {
            line: v.line,
            reason: v.kind.reason(),
        }
    }
}

fn require_format_version(seen: bool, line: usize) -> Result<(), PackageManifestError> {
    if seen {
        Ok(())
    } else {
        Err(PackageManifestError::Malformed {
            line,
            reason: "directive precedes `format_version`".into(),
        })
    }
}

fn is_pkg_name(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
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

fn validate_cap_path(s: &str, line: usize) -> Result<(), PackageManifestError> {
    let segments: Vec<&str> = s.split('.').collect();
    if segments.iter().any(|seg| seg.is_empty()) {
        return Err(PackageManifestError::Malformed {
            line,
            reason: format!(
                "malformed cap_path `{s}` — empty segment (double `.` or trailing `.`?)"
            ),
        });
    }
    for seg in &segments {
        if !is_path_segment(seg) {
            return Err(PackageManifestError::Malformed {
                line,
                reason: format!("malformed cap_path segment `{seg}` in `{s}`"),
            });
        }
    }
    let root = segments[0];
    if !RESERVED_CAP_ROOTS.contains(&root) {
        return Err(PackageManifestError::InvalidCapabilityRoot {
            line,
            root: root.to_owned(),
        });
    }
    Ok(())
}

fn parse_level_token(s: &str) -> Option<CapabilityLevel> {
    match s {
        "grant" => Some(CapabilityLevel::Grant),
        "ambient" => Some(CapabilityLevel::Ambient),
        "deny" => Some(CapabilityLevel::Deny),
        "defer" => Some(CapabilityLevel::Defer),
        _ => None,
    }
}

const fn level_token(level: CapabilityLevel) -> &'static str {
    match level {
        CapabilityLevel::Grant => "grant",
        CapabilityLevel::Ambient => "ambient",
        CapabilityLevel::Deny => "deny",
        CapabilityLevel::Defer => "defer",
    }
}

fn parse_semver(s: &str) -> Option<SemVer> {
    let mut parts = s.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    let patch: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(SemVer::new(major, minor, patch))
}

fn parse_hash<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        let hi = hex_nibble(s.as_bytes()[i * 2])?;
        let lo = hex_nibble(s.as_bytes()[i * 2 + 1])?;
        *byte = (hi << 4) | lo;
    }
    Some(out)
}

const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        // Reject uppercase — canonical form is lowercase.
        _ => None,
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0F) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strict_parser::{MAX_FILE_SIZE, MAX_LINE_LEN};

    fn sample_hash() -> [u8; IFACE_HASH_LEN] {
        let mut h = [0u8; IFACE_HASH_LEN];
        for (i, b) in h.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(7);
        }
        h
    }

    fn happy_path_text() -> String {
        format!(
            "# triet.package — generated, hand-editable.\n\
             # ADR-0018 §1 — capability claims + deps.\n\
             format_version 1\n\
             name myapp\n\
             version 0.1.0\n\
             \n\
             requires dev.disk deny\n\
             requires sys.io grant\n\
             requires sys.net.dns defer\n\
             requires usr.somelib ambient\n\
             \n\
             dep libdns 1.2.3 1.3.0 {h}\n\
             dep libtls 0.4.0 0.5.0 {h}\n",
            h = hex_encode(&sample_hash()),
        )
    }

    // ── Happy path / round-trip ─────────────────────────────────────

    #[test]
    fn parses_full_happy_path() {
        let m = PackageManifest::parse(&happy_path_text()).expect("parse ok");
        assert_eq!(m.name, "myapp");
        assert_eq!(m.version, SemVer::new(0, 1, 0));
        assert_eq!(m.requires.len(), 4);
        assert_eq!(m.deps.len(), 2);
        // Canonical sort applied even on parsed output.
        assert_eq!(m.requires[0].cap_path, "dev.disk");
        assert_eq!(m.requires[0].level, CapabilityLevel::Deny);
        assert_eq!(m.requires[3].cap_path, "usr.somelib");
        assert_eq!(m.requires[3].level, CapabilityLevel::Ambient);
        assert_eq!(m.deps[0].pkg_name, "libdns");
    }

    #[test]
    fn roundtrip_preserves_content() {
        let m = PackageManifest::parse(&happy_path_text()).expect("parse ok");
        let serialized = m.serialize();
        let m2 = PackageManifest::parse(&serialized).expect("re-parse ok");
        assert_eq!(m, m2);
    }

    #[test]
    fn roundtrip_minimal_manifest() {
        let m = PackageManifest::new("leaflib", SemVer::new(0, 1, 0));
        let s = m.serialize();
        let m2 = PackageManifest::parse(&s).expect("re-parse ok");
        assert_eq!(m, m2);
        assert!(m2.requires.is_empty());
        assert!(m2.deps.is_empty());
    }

    #[test]
    fn serialize_sorts_requires_and_deps() {
        let h = hex_encode(&sample_hash());
        // Author writes in arbitrary order — serialize must emit canonical.
        let m = PackageManifest {
            name: "u".into(),
            version: SemVer::new(0, 1, 0),
            requires: vec![
                CapabilityClaim {
                    cap_path: "sys.io".into(),
                    level: CapabilityLevel::Grant,
                },
                CapabilityClaim {
                    cap_path: "dev.disk".into(),
                    level: CapabilityLevel::Deny,
                },
            ],
            deps: vec![Dep {
                pkg_name: "zlib".into(),
                version_min: SemVer::new(1, 0, 0),
                version_max_exclusive: SemVer::new(2, 0, 0),
                iface_hash_pin: IfaceHash(sample_hash()),
            }],
        };
        let out = m.serialize();
        let dev_pos = out.find("dev.disk").expect("dev.disk emitted");
        let sys_pos = out.find("sys.io").expect("sys.io emitted");
        assert!(dev_pos < sys_pos, "requires must be sorted by cap_path");
        assert!(out.contains(&format!("dep zlib 1.0.0 2.0.0 {h}")));
    }

    // ── Whitelist edges (ADR-0017 Addendum §A) ──────────────────────

    #[test]
    fn rejects_bom() {
        let text = "\u{FEFF}format_version 1\nname x\nversion 0.1.0\n";
        let err = PackageManifest::parse(text).expect_err("must reject BOM");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 1, ref reason } if reason.contains("BOM"))
        );
    }

    #[test]
    fn rejects_crlf() {
        let text = "format_version 1\r\nname x\nversion 0.1.0\n";
        let err = PackageManifest::parse(text).expect_err("must reject CRLF");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 1, ref reason } if reason.contains("CRLF"))
        );
    }

    #[test]
    fn rejects_inline_comment() {
        let text = "format_version 1\nname x # inline\nversion 0.1.0\n";
        let err = PackageManifest::parse(text).expect_err("must reject inline #");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 2, ref reason } if reason.contains("inline"))
        );
    }

    #[test]
    fn rejects_unicode_whitespace() {
        // U+00A0 NBSP between tokens.
        let text = "format_version 1\nname\u{00A0}x\nversion 0.1.0\n";
        let err = PackageManifest::parse(text).expect_err("must reject NBSP");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 2, ref reason } if reason.contains("non-ASCII"))
        );
    }

    #[test]
    fn rejects_oversize_line() {
        let mut text = String::from("format_version 1\n");
        text.push_str("name ");
        text.push_str(&"a".repeat(MAX_LINE_LEN));
        text.push('\n');
        let err = PackageManifest::parse(&text).expect_err("must reject long line");
        assert!(
            matches!(err, PackageManifestError::Malformed { ref reason, .. } if reason.contains("line exceeds"))
        );
    }

    #[test]
    fn rejects_oversize_file() {
        let mut text = String::from("format_version 1\n");
        text.push_str(&"# pad\n".repeat(MAX_FILE_SIZE / 6 + 1));
        let err = PackageManifest::parse(&text).expect_err("must reject big file");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 0, ref reason } if reason.contains("file exceeds"))
        );
    }

    #[test]
    fn rejects_unknown_directive() {
        let text = "format_version 1\nfrobnicate yes\nname x\nversion 0.1.0\n";
        let err = PackageManifest::parse(text).expect_err("must reject unknown");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 2, ref reason } if reason.contains("frobnicate"))
        );
    }

    #[test]
    fn rejects_missing_format_version() {
        let text = "name x\nversion 0.1.0\n";
        let err = PackageManifest::parse(text).expect_err("must reject");
        // First "name" line fires `directive precedes format_version`.
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 1, ref reason } if reason.contains("precedes"))
        );
    }

    #[test]
    fn rejects_duplicate_format_version() {
        let text = "format_version 1\nformat_version 1\nname x\nversion 0.1.0\n";
        let err = PackageManifest::parse(text).expect_err("must reject dup");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 2, ref reason } if reason.contains("duplicate"))
        );
    }

    #[test]
    fn rejects_unsupported_format_version() {
        let text = "format_version 2\nname x\nversion 0.1.0\n";
        let err = PackageManifest::parse(text).expect_err("must reject");
        assert!(matches!(
            err,
            PackageManifestError::UnsupportedFormatVersion {
                found: 2,
                supported: 1
            }
        ));
    }

    #[test]
    fn rejects_missing_name() {
        let text = "format_version 1\nversion 0.1.0\n";
        let err = PackageManifest::parse(text).expect_err("must reject");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 0, ref reason } if reason.contains("missing `name`"))
        );
    }

    #[test]
    fn rejects_missing_version() {
        let text = "format_version 1\nname x\n";
        let err = PackageManifest::parse(text).expect_err("must reject");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 0, ref reason } if reason.contains("missing `version`"))
        );
    }

    // ── Capability path validation ──────────────────────────────────

    #[test]
    fn rejects_invalid_cap_root() {
        let text = "format_version 1\nname x\nversion 0.1.0\nrequires foo.bar grant\n";
        let err = PackageManifest::parse(text).expect_err("must reject");
        assert!(matches!(
            err,
            PackageManifestError::InvalidCapabilityRoot {
                line: 4,
                ref root,
            } if root == "foo"
        ));
    }

    #[test]
    fn rejects_empty_cap_segment() {
        let text = "format_version 1\nname x\nversion 0.1.0\nrequires sys..io grant\n";
        let err = PackageManifest::parse(text).expect_err("must reject");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 4, ref reason } if reason.contains("empty segment"))
        );
    }

    #[test]
    fn rejects_duplicate_requires() {
        let text = "format_version 1\nname x\nversion 0.1.0\nrequires sys.io grant\nrequires sys.io deny\n";
        let err = PackageManifest::parse(text).expect_err("must reject");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 5, ref reason } if reason.contains("duplicate"))
        );
    }

    #[test]
    fn rejects_unknown_level_token() {
        let text = "format_version 1\nname x\nversion 0.1.0\nrequires sys.io frobnicate\n";
        let err = PackageManifest::parse(text).expect_err("must reject");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 4, ref reason } if reason.contains("unknown level"))
        );
    }

    #[test]
    fn accepts_all_four_level_tokens() {
        let text = "format_version 1\nname x\nversion 0.1.0\n\
             requires sys.a grant\nrequires sys.b ambient\n\
             requires sys.c deny\nrequires sys.d defer\n";
        let m = PackageManifest::parse(text).expect("parse ok");
        let levels: Vec<_> = m.requires.iter().map(|c| c.level).collect();
        assert_eq!(
            levels,
            vec![
                CapabilityLevel::Grant,
                CapabilityLevel::Ambient,
                CapabilityLevel::Deny,
                CapabilityLevel::Defer,
            ]
        );
    }

    // ── Dep parsing ─────────────────────────────────────────────────

    #[test]
    fn rejects_bad_hash_hex() {
        let text = "format_version 1\nname x\nversion 0.1.0\ndep libx 1.0.0 2.0.0 not_hex\n";
        let err = PackageManifest::parse(text).expect_err("must reject");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 4, ref reason } if reason.contains("iface_hash"))
        );
    }

    #[test]
    fn rejects_duplicate_dep() {
        let h = hex_encode(&sample_hash());
        let text = format!(
            "format_version 1\nname x\nversion 0.1.0\n\
             dep libx 1.0.0 2.0.0 {h}\ndep libx 1.1.0 2.0.0 {h}\n",
        );
        let err = PackageManifest::parse(&text).expect_err("must reject");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 5, ref reason } if reason.contains("duplicate"))
        );
    }

    #[test]
    fn rejects_uppercase_hash() {
        // Canonical form is lowercase hex; uppercase is a refuse-to-load.
        let mut upper = hex_encode(&sample_hash());
        upper.make_ascii_uppercase();
        let text =
            format!("format_version 1\nname x\nversion 0.1.0\ndep libx 1.0.0 2.0.0 {upper}\n");
        let err = PackageManifest::parse(&text).expect_err("must reject");
        assert!(
            matches!(err, PackageManifestError::Malformed { line: 4, ref reason } if reason.contains("iface_hash"))
        );
    }

    // ── File I/O round-trip ─────────────────────────────────────────

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("triet.package");
        let m = PackageManifest {
            name: "iodemo".into(),
            version: SemVer::new(2, 0, 1),
            requires: vec![CapabilityClaim {
                cap_path: "sys.io".into(),
                level: CapabilityLevel::Grant,
            }],
            deps: Vec::new(),
        };
        m.save(&path).expect("save ok");
        let m2 = PackageManifest::load(&path).expect("load ok");
        assert_eq!(m, m2);
    }
}
