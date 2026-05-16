//! `triet.lock` — per-project lockfile mapping `pkg_name@version` →
//! exact `(iface_hash, impl_hash)`. Build determinism + reproducibility.
//!
//! ADR-0014 §1 says lockfile is the v0.5 CAS resolver's authoritative
//! input. ADR-0015 §5 covers the resolution flow. We deliberately do
//! NOT use TOML — pulling serde+toml just for this file would balloon
//! the dep tree. The hand-rolled line format below is enough:
//!
//! ```text
//! # triet.lock — auto-generated, do not edit by hand.
//! # ADR-0014/0015 — content-addressed package store.
//! format_version 1
//!
//! pkg <name> <major>.<minor>.<patch> <iface_hash_hex> <impl_hash_hex>
//! pkg <name> <major>.<minor>.<patch> <iface_hash_hex> <impl_hash_hex>
//! ...
//! ```
//!
//! Properties:
//!
//! - Lines starting `#` and blank lines are ignored.
//! - One `format_version` line; only `1` accepted at v0.5.
//! - `pkg` lines: 5 whitespace-separated fields. Sort by `name` for
//!   canonical output so the file diff stays minimal across builds.
//! - 64-char lowercase hex for both hashes.

use std::fs;
use std::path::Path;

use crate::error::{StoreError, StoreResult};
use crate::hash::{IFACE_HASH_LEN, IMPL_HASH_LEN, IfaceHash, ImplHash};
use crate::types::SemVer;

/// Lockfile format version. Bump on incompatible wire change.
const FORMAT_VERSION: u32 = 1;

/// One entry in the lockfile — a pinned package resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockEntry {
    /// Package name as declared by its author.
    pub pkg_name: String,
    /// Resolved version triple.
    pub version: SemVer,
    /// Resolved ABI surface hash. Authoritative for re-link decisions
    /// per ADR-0013 §4.
    pub iface_hash: IfaceHash,
    /// Resolved content hash — the CAS store address.
    pub impl_hash: ImplHash,
}

/// In-memory representation of a `triet.lock` file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lockfile {
    entries: Vec<LockEntry>,
}

impl Lockfile {
    /// Empty lockfile — used for first-time resolution before anything
    /// is pinned.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// All entries in canonical (name-sorted) order.
    #[must_use]
    pub fn entries(&self) -> &[LockEntry] {
        &self.entries
    }

    /// Look up the pinned entry for a package by name.
    #[must_use]
    pub fn find(&self, pkg_name: &str) -> Option<&LockEntry> {
        self.entries.iter().find(|e| e.pkg_name == pkg_name)
    }

    /// Insert or replace an entry. Keeps entries sorted by name so
    /// `serialize()` output is canonical without an extra sort pass.
    pub fn upsert(&mut self, entry: LockEntry) {
        if let Some(slot) = self
            .entries
            .iter_mut()
            .find(|e| e.pkg_name == entry.pkg_name)
        {
            *slot = entry;
        } else {
            // Insert in sorted position to preserve canonical order.
            let pos = self
                .entries
                .binary_search_by(|e| e.pkg_name.cmp(&entry.pkg_name))
                .unwrap_or_else(|p| p);
            self.entries.insert(pos, entry);
        }
    }

    /// Remove the entry for `pkg_name`, returning whether one was
    /// present.
    pub fn remove(&mut self, pkg_name: &str) -> bool {
        if let Some(pos) = self.entries.iter().position(|e| e.pkg_name == pkg_name) {
            self.entries.remove(pos);
            true
        } else {
            false
        }
    }

    /// Parse from text. See module docs for the format.
    ///
    /// # Errors
    /// Returns [`LockfileError`] if the text isn't well-formed (wrong
    /// `format_version`, malformed `pkg` line, bad hex, wrong hash
    /// length).
    pub fn parse(text: &str) -> Result<Self, LockfileError> {
        let mut format_version: Option<u32> = None;
        let mut entries = Vec::new();

        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let head = parts.next().unwrap_or("");
            match head {
                "format_version" => {
                    let v: u32 = parts
                        .next()
                        .ok_or_else(|| LockfileError::Malformed {
                            line: line_no + 1,
                            reason: "missing version after `format_version`".into(),
                        })?
                        .parse()
                        .map_err(|_| LockfileError::Malformed {
                            line: line_no + 1,
                            reason: "version must be an integer".into(),
                        })?;
                    if v != FORMAT_VERSION {
                        return Err(LockfileError::UnsupportedFormatVersion {
                            found: v,
                            supported: FORMAT_VERSION,
                        });
                    }
                    format_version = Some(v);
                }
                "pkg" => {
                    let name = parts.next().ok_or_else(|| LockfileError::Malformed {
                        line: line_no + 1,
                        reason: "missing pkg name".into(),
                    })?;
                    let ver_str = parts.next().ok_or_else(|| LockfileError::Malformed {
                        line: line_no + 1,
                        reason: "missing version".into(),
                    })?;
                    let iface_str = parts.next().ok_or_else(|| LockfileError::Malformed {
                        line: line_no + 1,
                        reason: "missing iface_hash".into(),
                    })?;
                    let impl_str = parts.next().ok_or_else(|| LockfileError::Malformed {
                        line: line_no + 1,
                        reason: "missing impl_hash".into(),
                    })?;
                    if parts.next().is_some() {
                        return Err(LockfileError::Malformed {
                            line: line_no + 1,
                            reason: "extra fields after impl_hash".into(),
                        });
                    }
                    let version =
                        parse_semver(ver_str).ok_or_else(|| LockfileError::Malformed {
                            line: line_no + 1,
                            reason: format!("bad version `{ver_str}`"),
                        })?;
                    let iface_bytes = parse_hash::<IFACE_HASH_LEN>(iface_str).ok_or_else(|| {
                        LockfileError::Malformed {
                            line: line_no + 1,
                            reason: "bad iface_hash hex".into(),
                        }
                    })?;
                    let impl_bytes = parse_hash::<IMPL_HASH_LEN>(impl_str).ok_or_else(|| {
                        LockfileError::Malformed {
                            line: line_no + 1,
                            reason: "bad impl_hash hex".into(),
                        }
                    })?;
                    entries.push(LockEntry {
                        pkg_name: name.to_owned(),
                        version,
                        iface_hash: IfaceHash(iface_bytes),
                        impl_hash: ImplHash(impl_bytes),
                    });
                }
                other => {
                    return Err(LockfileError::Malformed {
                        line: line_no + 1,
                        reason: format!("unknown directive `{other}`"),
                    });
                }
            }
        }

        if format_version.is_none() && !entries.is_empty() {
            return Err(LockfileError::Malformed {
                line: 0,
                reason: "missing `format_version` directive".into(),
            });
        }

        // Sort by pkg_name for canonical ordering.
        entries.sort_by(|a, b| a.pkg_name.cmp(&b.pkg_name));
        Ok(Self { entries })
    }

    /// Render to text in canonical form. Always emits the header
    /// comment + `format_version` line, even for empty lockfiles, so
    /// `parse(serialize(L)) == L`.
    #[must_use]
    pub fn serialize(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        out.push_str("# triet.lock — auto-generated, do not edit by hand.\n");
        out.push_str("# ADR-0014/0015 — content-addressed package store.\n");
        writeln!(&mut out, "format_version {FORMAT_VERSION}").expect("String write");
        if !self.entries.is_empty() {
            out.push('\n');
        }
        for e in &self.entries {
            writeln!(
                &mut out,
                "pkg {} {}.{}.{} {} {}",
                e.pkg_name,
                e.version.major,
                e.version.minor,
                e.version.patch,
                hex_encode(&e.iface_hash.0),
                hex_encode(&e.impl_hash.0),
            )
            .expect("String write");
        }
        out
    }

    /// Read a lockfile from disk. Treats NotFound as `Ok(empty)` so
    /// the first build of a project doesn't need special-casing.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] for read failures other than NotFound,
    /// or [`StoreError::Lockfile`] if the file exists but is malformed.
    pub fn load(path: &Path) -> StoreResult<Self> {
        let text = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::empty()),
            Err(e) => return Err(StoreError::io(path.display().to_string(), e)),
        };
        Self::parse(&text).map_err(StoreError::from)
    }

    /// Write the lockfile to `path` atomically (write to a sibling
    /// `.tmp` then `rename()` — POSIX atomic).
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
        let tmp = path.with_extension("lock.tmp");
        fs::write(&tmp, text.as_bytes())
            .map_err(|e| StoreError::io(tmp.display().to_string(), e))?;
        fs::rename(&tmp, path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            StoreError::io(path.display().to_string(), e)
        })
    }
}

/// Errors specific to lockfile I/O. Wrapped by [`StoreError::Lockfile`]
/// when raised through `Lockfile::load`.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum LockfileError {
    /// The file's `format_version` directive is newer than this reader
    /// supports.
    #[error("unsupported lockfile format version {found} (max supported: {supported})")]
    #[diagnostic(
        code(triet::pack::E2370),
        help("update the Triết toolchain — this `triet.lock` was generated by a newer build")
    )]
    UnsupportedFormatVersion {
        /// Version found in the file.
        found: u32,
        /// Maximum version this reader handles.
        supported: u32,
    },

    /// A line failed to parse. `line` is 1-based for editor click-throughs.
    #[error("malformed lockfile at line {line}: {reason}")]
    #[diagnostic(
        code(triet::pack::E2371),
        help("if the file was hand-edited, delete it and re-run the build to regenerate")
    )]
    Malformed {
        /// 1-based line number where the parser gave up.
        line: usize,
        /// Human-readable cause.
        reason: String,
    },
}

// ── Helpers ─────────────────────────────────────────────────────────

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
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_nibble(bytes[i])?;
        let lo = hex_nibble(bytes[i + 1])?;
        out[i / 2] = (hi << 4) | lo;
        i += 2;
    }
    Some(out)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0F) as usize] as char);
    }
    s
}

const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn entry(name: &str, ver: SemVer, iface_byte: u8, impl_byte: u8) -> LockEntry {
        LockEntry {
            pkg_name: name.to_owned(),
            version: ver,
            iface_hash: IfaceHash([iface_byte; IFACE_HASH_LEN]),
            impl_hash: ImplHash([impl_byte; IMPL_HASH_LEN]),
        }
    }

    #[test]
    fn empty_round_trips() {
        let lf = Lockfile::empty();
        let text = lf.serialize();
        let parsed = Lockfile::parse(&text).unwrap();
        assert_eq!(lf, parsed);
        assert!(parsed.entries().is_empty());
    }

    #[test]
    fn single_entry_round_trips() {
        let mut lf = Lockfile::empty();
        lf.upsert(entry("math", SemVer::new(1, 2, 3), 0xAB, 0xCD));
        let text = lf.serialize();
        let parsed = Lockfile::parse(&text).unwrap();
        assert_eq!(lf, parsed);
    }

    #[test]
    fn multiple_entries_sorted_canonically() {
        let mut lf = Lockfile::empty();
        // Insert out of order on purpose.
        lf.upsert(entry("zeta", SemVer::new(0, 1, 0), 1, 2));
        lf.upsert(entry("alpha", SemVer::new(2, 0, 0), 3, 4));
        lf.upsert(entry("middle", SemVer::new(1, 0, 0), 5, 6));
        let text = lf.serialize();
        // Order in output must be alpha → middle → zeta.
        let alpha_idx = text.find("pkg alpha").unwrap();
        let middle_idx = text.find("pkg middle").unwrap();
        let zeta_idx = text.find("pkg zeta").unwrap();
        assert!(alpha_idx < middle_idx);
        assert!(middle_idx < zeta_idx);
    }

    #[test]
    fn upsert_replaces_existing() {
        let mut lf = Lockfile::empty();
        lf.upsert(entry("foo", SemVer::new(1, 0, 0), 0x11, 0x11));
        lf.upsert(entry("foo", SemVer::new(2, 0, 0), 0x22, 0x22));
        assert_eq!(lf.entries().len(), 1);
        assert_eq!(lf.entries()[0].version, SemVer::new(2, 0, 0));
    }

    #[test]
    fn remove_takes_existing_entry() {
        let mut lf = Lockfile::empty();
        lf.upsert(entry("foo", SemVer::new(1, 0, 0), 0, 0));
        assert!(lf.remove("foo"));
        assert!(lf.entries().is_empty());
        assert!(!lf.remove("foo"));
    }

    #[test]
    fn comments_and_blank_lines_skipped() {
        let text = "
            # a comment
            format_version 1

            # another comment

            pkg foo 1.0.0 aa00aa00aa00aa00aa00aa00aa00aa00aa00aa00aa00aa00aa00aa00aa00aa00 bb00bb00bb00bb00bb00bb00bb00bb00bb00bb00bb00bb00bb00bb00bb00bb00
            ";
        let lf = Lockfile::parse(text).unwrap();
        assert_eq!(lf.entries().len(), 1);
        assert_eq!(lf.entries()[0].pkg_name, "foo");
    }

    #[test]
    fn unsupported_format_version_rejected() {
        let text = "format_version 99\n";
        let err = Lockfile::parse(text).unwrap_err();
        assert!(matches!(
            err,
            LockfileError::UnsupportedFormatVersion {
                found: 99,
                supported: 1
            }
        ));
    }

    #[test]
    fn bad_hex_rejected() {
        let text = "format_version 1\npkg foo 1.0.0 not-hex zz\n";
        let err = Lockfile::parse(text).unwrap_err();
        assert!(matches!(err, LockfileError::Malformed { .. }));
    }

    #[test]
    fn missing_format_version_with_entries_rejected() {
        let text = "pkg foo 1.0.0 aa00aa00aa00aa00aa00aa00aa00aa00aa00aa00aa00aa00aa00aa00aa00aa00 bb00bb00bb00bb00bb00bb00bb00bb00bb00bb00bb00bb00bb00bb00bb00bb00\n";
        let err = Lockfile::parse(text).unwrap_err();
        assert!(matches!(err, LockfileError::Malformed { .. }));
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("ghost.lock");
        let lf = Lockfile::load(&path).unwrap();
        assert!(lf.entries().is_empty());
    }

    #[test]
    fn save_then_load_round_trip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("triet.lock");
        let mut lf = Lockfile::empty();
        lf.upsert(entry("a", SemVer::new(1, 0, 0), 1, 2));
        lf.upsert(entry("b", SemVer::new(0, 5, 3), 3, 4));
        lf.save(&path).unwrap();
        let loaded = Lockfile::load(&path).unwrap();
        assert_eq!(lf, loaded);
    }
}
