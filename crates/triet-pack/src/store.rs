//! Content-addressable package store — `~/.triet/store/` filesystem
//! layer per [ADR-0015].
//!
//! Three top-level branches mirror the 3-cấp hash tree from [ADR-0014]:
//! `term/`, `mod/`, `pkg/`. Plus `names/` (symbolic alias → pkg hash),
//! `roots/` (GC roots — projects referencing packs), and `tmp/`
//! (atomic install staging).
//!
//! Atomic install protocol: write to `tmp/<uid>/` then `rename()` to
//! the target hash dir. POSIX `rename` is atomic; on EEXIST another
//! process already installed the same hash → no-op.
//!
//! v0.5.4 scope: pack-level install + module-level rollup + term-level
//! iface bytes. Body bytes (`term/<hash>/body.bin`) deferred until the
//! lowerer can split per-term IR bodies (v0.5.8 / v0.6). v0.5.6 demo
//! verifies iface-level dedup end-to-end; full RAM-sharing of bodies
//! lands when the lowerer hookup arrives.
//!
//! v0.5.6: term dir keyed by `impl_hash_term` per ADR-0015 §2 (not
//! `iface_hash_term`). With empty bodies (v0.5.3 placeholder) the two
//! collapse, so dedup behaviour is unchanged; the rename is purely a
//! correctness fix that pays off when real bodies arrive.
//!
//! [ADR-0014]: ../../../docs/decisions/0014-hash-scheme-refinement.md
//! [ADR-0015]: ../../../docs/decisions/0015-package-store-layout.md

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{StoreError, StoreResult};
use crate::hash::{IMPL_HASH_LEN, ImplHash, ModuleImplHash};
use crate::serde::{
    canonical_term_signature_function, canonical_term_signature_type, read_khi,
};
use crate::types::{AbiMetadata, SemVer};

/// Counter for tmp-dir uniqueness within one process. Combined with
/// the system clock + PID we get collision-free names without pulling
/// in a UUID crate.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Convenience constants for subdirectory names. Keeping the strings
/// in one place keeps ADR-0015 §2 + §4 traceable to code.
mod dirs {
    pub(super) const TERM: &str = "term";
    pub(super) const MOD: &str = "mod";
    pub(super) const PKG: &str = "pkg";
    pub(super) const NAMES: &str = "names";
    pub(super) const ROOTS: &str = "roots";
    pub(super) const TMP: &str = "tmp";
}

/// Content-addressable package store rooted at a single directory.
/// One `Store` instance per process — multi-process access is safe
/// thanks to the atomic-rename protocol (ADR-0015 §6 + §8).
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

/// Snapshot of which packs a project currently references — used as
/// GC mark roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootEntry {
    /// Stable project identifier (typically BLAKE3 of absolute project
    /// path; caller decides — store treats it as opaque).
    pub project_id: String,
    /// Pack hashes the project pins. Empty list = unreference (caller
    /// should call [`Store::remove_root`] instead).
    pub pkg_hashes: Vec<ImplHash>,
}

/// Summary of one `gc()` pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GcReport {
    /// Number of pkg dirs removed.
    pub swept_pkgs: usize,
    /// Number of module dirs removed.
    pub swept_modules: usize,
    /// Number of term dirs removed.
    pub swept_terms: usize,
    /// Number of dangling name links removed.
    pub swept_name_links: usize,
    /// Pkg hashes whose `manifest.bin` couldn't be parsed during the
    /// mark phase — typically filesystem corruption or tampering.
    /// When non-empty, GC enters a conservative mode: module and term
    /// sweeps are skipped entirely (we can't tell what those packs
    /// referenced, so we don't risk orphaning their dependencies).
    pub corrupt_pkgs: Vec<ImplHash>,
}

impl Store {
    /// Open (or create) a store rooted at `root`. Creates the five
    /// top-level subdirectories if missing. Idempotent — opening an
    /// existing store doesn't touch its content.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] if directory creation fails (e.g.
    /// `root` is not writable).
    pub fn open(root: impl AsRef<Path>) -> StoreResult<Self> {
        let root = root.as_ref().to_path_buf();
        for sub in [
            dirs::TERM,
            dirs::MOD,
            dirs::PKG,
            dirs::NAMES,
            dirs::ROOTS,
            dirs::TMP,
        ] {
            let p = root.join(sub);
            fs::create_dir_all(&p).map_err(|e| StoreError::io(p.display().to_string(), e))?;
        }
        Ok(Self { root })
    }

    /// Return the store's root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Install a `.khi` into the store. Returns the pack's
    /// `impl_hash` (its CAS address).
    ///
    /// Side effects:
    /// - `pkg/<hex(impl_hash_pkg)>/pack.khi` + `manifest.bin`
    /// - `mod/<hex(impl_hash_mod)>/index.bin` for every module
    /// - `term/<hex(impl_hash_term)>/iface.bin` for every type +
    ///   export (canonical signature bytes — body.bin written once the
    ///   lowerer can split per-term IR bodies, v0.5.8 / v0.6)
    /// - `names/<pkg_name>/<semver>.link` containing the pack's
    ///   `impl_hash` hex bytes (overwrites any existing alias)
    ///
    /// Already-present hashes are skipped (no-op). Safe under
    /// concurrent installs of the same hash thanks to atomic rename.
    ///
    /// # Errors
    /// Returns [`StoreError::Pack`] if `pack_bytes` doesn't parse as a
    /// valid `.khi`, or [`StoreError::Io`] if any filesystem
    /// operation (write, rename, mkdir) fails.
    pub fn install_pack(&self, pack_bytes: &[u8]) -> StoreResult<ImplHash> {
        let (meta, _code) = read_khi(pack_bytes)?;
        let impl_hash = meta.impl_hash;

        // ── pkg/<hash>/ ─────────────────────────────────────────────
        let pkg_target = self.pkg_dir(&impl_hash);
        if !pkg_target.exists() {
            let manifest_bytes = extract_manifest_section(pack_bytes)?;
            self.atomic_install_dir(&pkg_target, |tmp| {
                write_file(&tmp.join("pack.khi"), pack_bytes)?;
                write_file(&tmp.join("manifest.bin"), &manifest_bytes)?;
                Ok(())
            })?;
        }

        // ── mod/<hash>/ per module ──────────────────────────────────
        for m in &meta.modules {
            self.install_module(&m.impl_hash_mod, &meta, &m.path)?;
        }

        // ── term/<hash>/ per type + export ──────────────────────────
        for t in &meta.types {
            let iface_bytes = canonical_term_signature_type(t);
            self.install_term(&t.impl_hash_term, &iface_bytes)?;
        }
        for e in &meta.exports {
            let iface_bytes = canonical_term_signature_function(e);
            self.install_term(&e.impl_hash_term, &iface_bytes)?;
        }

        // ── names/<pkg_name>/<semver>.link ──────────────────────────
        self.link_name(&meta.pkg_name, meta.pkg_version, &impl_hash)?;

        Ok(impl_hash)
    }

    /// Read pack bytes by `impl_hash`. Returns `Ok(None)` if the pack
    /// isn't in the store.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] for filesystem read failures other
    /// than NotFound (NotFound maps to `Ok(None)`).
    pub fn resolve_pack(&self, impl_hash: &ImplHash) -> StoreResult<Option<Vec<u8>>> {
        let path = self.pkg_dir(impl_hash).join("pack.khi");
        read_optional_file(&path)
    }

    /// Read the extracted manifest (`manifest.bin`) for a pack. Cheap
    /// alternative to parsing the full `pack.khi` when callers
    /// only need the metadata.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] for filesystem read failures other
    /// than NotFound (NotFound maps to `Ok(None)`).
    pub fn resolve_manifest_bytes(&self, impl_hash: &ImplHash) -> StoreResult<Option<Vec<u8>>> {
        let path = self.pkg_dir(impl_hash).join("manifest.bin");
        read_optional_file(&path)
    }

    /// Resolve a pack hash by symbolic `(pkg_name, version)`. Returns
    /// `Ok(None)` if no name link exists.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] if the link file exists but is
    /// unreadable or malformed (non-UTF8, bad hex, wrong length).
    pub fn resolve_by_name(
        &self,
        pkg_name: &str,
        version: SemVer,
    ) -> StoreResult<Option<ImplHash>> {
        let path = self.name_link_path(pkg_name, version);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(StoreError::io(path.display().to_string(), e)),
        };
        let hex = std::str::from_utf8(bytes.trim_ascii())
            .map_err(|_| StoreError::io(path.display().to_string(), invalid_data("non-UTF8 link")))?
            .trim();
        let raw = hex_decode(hex).ok_or_else(|| {
            StoreError::io(path.display().to_string(), invalid_data("bad hex in link"))
        })?;
        if raw.len() != IMPL_HASH_LEN {
            return Err(StoreError::io(
                path.display().to_string(),
                invalid_data("link hash wrong length"),
            ));
        }
        let mut arr = [0u8; IMPL_HASH_LEN];
        arr.copy_from_slice(&raw);
        Ok(Some(ImplHash(arr)))
    }

    /// List all installed versions of `pkg_name`, sorted ascending by
    /// version. Returns an empty vec if the package isn't in the store.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] if the names directory exists but
    /// can't be enumerated, or if a link file is malformed.
    pub fn list_versions(&self, pkg_name: &str) -> StoreResult<Vec<(SemVer, ImplHash)>> {
        let dir = self.root.join(dirs::NAMES).join(pkg_name);
        let mut out = Vec::new();
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(StoreError::io(dir.display().to_string(), e)),
        };
        for entry in entries {
            let entry = entry.map_err(|e| StoreError::io(dir.display().to_string(), e))?;
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("link") {
                continue;
            }
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            // Parse `<major>.<minor>.<patch>` from the file stem.
            let mut parts = stem.split('.');
            let Some(major_str) = parts.next() else {
                continue;
            };
            let Some(minor_str) = parts.next() else {
                continue;
            };
            let Some(patch_str) = parts.next() else {
                continue;
            };
            if parts.next().is_some() {
                continue;
            }
            let (Ok(major), Ok(minor), Ok(patch)) = (
                major_str.parse::<u32>(),
                minor_str.parse::<u32>(),
                patch_str.parse::<u32>(),
            ) else {
                continue;
            };
            let version = SemVer::new(major, minor, patch);
            // Resolve the link to its impl_hash.
            if let Some(hash) = self.resolve_by_name(pkg_name, version)? {
                out.push((version, hash));
            }
        }
        out.sort_by(|a, b| {
            a.0.major
                .cmp(&b.0.major)
                .then(a.0.minor.cmp(&b.0.minor))
                .then(a.0.patch.cmp(&b.0.patch))
        });
        Ok(out)
    }

    /// Create or overwrite a name link `names/<pkg>/<semver>.link`
    /// pointing at `impl_hash`.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] if mkdir or atomic write fails.
    pub fn link_name(
        &self,
        pkg_name: &str,
        version: SemVer,
        impl_hash: &ImplHash,
    ) -> StoreResult<()> {
        let path = self.name_link_path(pkg_name, version);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| StoreError::io(parent.display().to_string(), e))?;
        }
        let hex = hex_encode(&impl_hash.0);
        atomic_write_file(&path, hex.as_bytes())
    }

    /// Register `project_id` as a GC root referencing `pkg_hashes`.
    /// Overwrites any previous registration for the same project.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] if the root file can't be written.
    pub fn add_root(&self, project_id: &str, pkg_hashes: &[ImplHash]) -> StoreResult<()> {
        let path = self.roots_dir().join(format!("{project_id}.root"));
        let mut body = String::with_capacity(pkg_hashes.len() * (IMPL_HASH_LEN * 2 + 1));
        for h in pkg_hashes {
            body.push_str(&hex_encode(&h.0));
            body.push('\n');
        }
        atomic_write_file(&path, body.as_bytes())
    }

    /// Remove `project_id`'s root entry. No-op if the project wasn't
    /// registered.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] if the file exists but can't be
    /// removed.
    pub fn remove_root(&self, project_id: &str) -> StoreResult<()> {
        let path = self.roots_dir().join(format!("{project_id}.root"));
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StoreError::io(path.display().to_string(), e)),
        }
    }

    /// List all current GC roots.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] if the roots directory can't be
    /// read, or if a root file contains malformed hex.
    pub fn list_roots(&self) -> StoreResult<Vec<RootEntry>> {
        let dir = self.roots_dir();
        let mut out = Vec::new();
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(StoreError::io(dir.display().to_string(), e)),
        };
        for entry in entries {
            let entry = entry.map_err(|e| StoreError::io(dir.display().to_string(), e))?;
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("root") {
                continue;
            }
            let project_id = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_owned();
            let body =
                fs::read_to_string(&p).map_err(|e| StoreError::io(p.display().to_string(), e))?;
            let mut pkg_hashes = Vec::new();
            for line in body.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let raw = hex_decode(line).ok_or_else(|| {
                    StoreError::io(p.display().to_string(), invalid_data("bad hex in root"))
                })?;
                if raw.len() != IMPL_HASH_LEN {
                    return Err(StoreError::io(
                        p.display().to_string(),
                        invalid_data("root hash wrong length"),
                    ));
                }
                let mut arr = [0u8; IMPL_HASH_LEN];
                arr.copy_from_slice(&raw);
                pkg_hashes.push(ImplHash(arr));
            }
            out.push(RootEntry {
                project_id,
                pkg_hashes,
            });
        }
        out.sort_by(|a, b| a.project_id.cmp(&b.project_id));
        Ok(out)
    }

    /// Garbage-collect unreachable content. Mark-and-sweep:
    /// 1. Walk `roots/*.root` → mark pkg hashes.
    /// 2. For each marked pkg: read manifest, mark referenced module
    ///    + term hashes.
    /// 3. Sweep: remove unmarked dirs under `term/`, `mod/`, `pkg/`.
    /// 4. Drop name links pointing at swept pkg hashes.
    /// 5. Wipe `tmp/` unconditionally (no install survives a GC pass).
    ///
    /// Not safe under concurrent installs (ADR-0015 §8) — caller
    /// should ensure exclusivity.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] if directory walking or removal
    /// fails. Malformed roots/links are best-effort skipped without
    /// aborting the sweep.
    pub fn gc(&self) -> StoreResult<GcReport> {
        use std::collections::HashSet;

        let mut live_pkgs: HashSet<[u8; IMPL_HASH_LEN]> = HashSet::new();
        let mut live_mods: HashSet<[u8; IMPL_HASH_LEN]> = HashSet::new();
        let mut live_terms: HashSet<[u8; IMPL_HASH_LEN]> = HashSet::new();
        let mut corrupt_pkgs: Vec<ImplHash> = Vec::new();

        // ── Mark phase ──────────────────────────────────────────────
        for root in self.list_roots()? {
            for h in root.pkg_hashes {
                live_pkgs.insert(h.0);
                if let Some(manifest_bytes) = self.resolve_manifest_bytes(&h)? {
                    match parse_manifest_only(&manifest_bytes) {
                        Ok(meta) => {
                            for m in meta.modules {
                                live_mods.insert(m.impl_hash_mod.0);
                            }
                            for t in meta.types {
                                live_terms.insert(t.impl_hash_term.0);
                            }
                            for e in meta.exports {
                                live_terms.insert(e.impl_hash_term.0);
                            }
                        }
                        Err(_) => {
                            // Can't enumerate references → don't risk
                            // orphaning this pkg's mod/term deps in the
                            // sweep. Record for the report; conservative
                            // mode kicks in below.
                            corrupt_pkgs.push(h);
                        }
                    }
                }
            }
        }

        // ── Sweep phase ─────────────────────────────────────────────
        let mut report = GcReport::default();
        report.swept_pkgs += sweep_hash_dir(&self.root.join(dirs::PKG), |bytes| {
            !live_pkgs.contains(&bytes)
        })?;
        // Conservative mode: if any live pkg had a corrupt manifest, we
        // can't tell which mods/terms it referenced — skip those sweeps
        // entirely. User fixes the corruption + re-runs GC.
        if corrupt_pkgs.is_empty() {
            report.swept_modules += sweep_hash_dir(&self.root.join(dirs::MOD), |bytes| {
                !live_mods.contains(&bytes)
            })?;
            report.swept_terms += sweep_hash_dir(&self.root.join(dirs::TERM), |bytes| {
                !live_terms.contains(&bytes)
            })?;
        }
        report.corrupt_pkgs = corrupt_pkgs;

        // Drop dangling name links — alias pointing at a swept pkg.
        report.swept_name_links += self.sweep_name_links(&live_pkgs)?;

        // Wipe tmp/ — anything there was an in-progress install that
        // didn't reach the atomic rename. User can re-run.
        let tmp = self.root.join(dirs::TMP);
        if tmp.exists() {
            for entry in
                fs::read_dir(&tmp).map_err(|e| StoreError::io(tmp.display().to_string(), e))?
            {
                let entry = entry.map_err(|e| StoreError::io(tmp.display().to_string(), e))?;
                remove_path(&entry.path())?;
            }
        }

        Ok(report)
    }

    // ── Internal helpers ────────────────────────────────────────────

    fn pkg_dir(&self, h: &ImplHash) -> PathBuf {
        self.root.join(dirs::PKG).join(hex_encode(&h.0))
    }

    fn mod_dir(&self, h: &ModuleImplHash) -> PathBuf {
        self.root.join(dirs::MOD).join(hex_encode(&h.0))
    }

    fn term_dir(&self, impl_hash_bytes: &[u8; IMPL_HASH_LEN]) -> PathBuf {
        self.root.join(dirs::TERM).join(hex_encode(impl_hash_bytes))
    }

    fn roots_dir(&self) -> PathBuf {
        self.root.join(dirs::ROOTS)
    }

    fn name_link_path(&self, pkg_name: &str, version: SemVer) -> PathBuf {
        self.root.join(dirs::NAMES).join(pkg_name).join(format!(
            "{}.{}.{}.link",
            version.major, version.minor, version.patch
        ))
    }

    fn install_module(
        &self,
        mod_hash: &ModuleImplHash,
        meta: &AbiMetadata,
        module_path: &str,
    ) -> StoreResult<()> {
        let target = self.mod_dir(mod_hash);
        if target.exists() {
            return Ok(());
        }
        // index.bin = sorted (term_name, impl_hash_term) entries for
        // terms belonging to this module path. Cheap re-read so
        // resolvers don't need the parent pack. Entries point at the
        // term dirs which are keyed by impl_hash per ADR-0015 §2.
        let mut entries: Vec<(String, [u8; IMPL_HASH_LEN])> = Vec::new();
        for t in &meta.types {
            if t.module_path == module_path {
                entries.push((t.name.clone(), t.impl_hash_term.0));
            }
        }
        for e in &meta.exports {
            if e.module_path == module_path {
                entries.push((e.name.clone(), e.impl_hash_term.0));
            }
        }
        entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

        let mut body = Vec::with_capacity(entries.len() * (IMPL_HASH_LEN + 32));
        for (name, hash_bytes) in entries {
            let len = u32::try_from(name.len()).unwrap_or(u32::MAX);
            body.extend_from_slice(&len.to_le_bytes());
            body.extend_from_slice(name.as_bytes());
            body.extend_from_slice(&hash_bytes);
        }

        self.atomic_install_dir(&target, |tmp| {
            write_file(&tmp.join("index.bin"), &body)?;
            Ok(())
        })
    }

    fn install_term(
        &self,
        impl_hash: &crate::hash::TermImplHash,
        signature_bytes: &[u8],
    ) -> StoreResult<()> {
        let target = self.term_dir(&impl_hash.0);
        if target.exists() {
            return Ok(());
        }
        self.atomic_install_dir(&target, |tmp| {
            write_file(&tmp.join("iface.bin"), signature_bytes)?;
            // body.bin intentionally absent — wires up when the
            // lowerer can split per-term IR bodies (v0.5.8 or v0.6).
            // v0.5.6 demo proves the dedup mechanism at iface level.
            Ok(())
        })
    }

    /// Generic atomic-install: build content under `tmp/<uid>/`, then
    /// rename to `target`. On EEXIST another process already installed
    /// the same hash → treat as success.
    fn atomic_install_dir(
        &self,
        target: &Path,
        populate: impl FnOnce(&Path) -> io::Result<()>,
    ) -> StoreResult<()> {
        let staging = self.fresh_tmp_dir()?;
        // Ensure cleanup if populate or rename fails — best-effort.
        let rollback = StagingGuard {
            path: staging.clone(),
        };

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| StoreError::io(parent.display().to_string(), e))?;
        }
        populate(&staging).map_err(|e| StoreError::io(staging.display().to_string(), e))?;

        match fs::rename(&staging, target) {
            Ok(()) => {
                rollback.disarm();
                Ok(())
            }
            Err(e) if target.exists() => {
                // Race lost — another install of the same hash got
                // there first. Cleanup staging and treat as success.
                drop(rollback);
                _ = e;
                Ok(())
            }
            Err(e) => {
                drop(rollback);
                Err(StoreError::io(target.display().to_string(), e))
            }
        }
    }

    fn fresh_tmp_dir(&self) -> StoreResult<PathBuf> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let counter = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let path = self
            .root
            .join(dirs::TMP)
            .join(format!("{pid}-{nanos}-{counter}"));
        fs::create_dir_all(&path).map_err(|e| StoreError::io(path.display().to_string(), e))?;
        Ok(path)
    }

    fn sweep_name_links(
        &self,
        live_pkgs: &std::collections::HashSet<[u8; IMPL_HASH_LEN]>,
    ) -> StoreResult<usize> {
        let names = self.root.join(dirs::NAMES);
        if !names.exists() {
            return Ok(0);
        }
        let mut removed = 0;
        for pkg_entry in
            fs::read_dir(&names).map_err(|e| StoreError::io(names.display().to_string(), e))?
        {
            let pkg_entry =
                pkg_entry.map_err(|e| StoreError::io(names.display().to_string(), e))?;
            let pkg_dir = pkg_entry.path();
            if !pkg_dir.is_dir() {
                continue;
            }
            for link in fs::read_dir(&pkg_dir)
                .map_err(|e| StoreError::io(pkg_dir.display().to_string(), e))?
            {
                let link = link.map_err(|e| StoreError::io(pkg_dir.display().to_string(), e))?;
                let p = link.path();
                if p.extension().and_then(|s| s.to_str()) != Some("link") {
                    continue;
                }
                let bytes = fs::read(&p).unwrap_or_default();
                let hex = std::str::from_utf8(&bytes).unwrap_or("").trim();
                if let Some(raw) = hex_decode(hex)
                    && raw.len() == IMPL_HASH_LEN
                {
                    let mut arr = [0u8; IMPL_HASH_LEN];
                    arr.copy_from_slice(&raw);
                    if !live_pkgs.contains(&arr) {
                        fs::remove_file(&p)
                            .map_err(|e| StoreError::io(p.display().to_string(), e))?;
                        removed += 1;
                    }
                }
            }
        }
        Ok(removed)
    }
}

/// Auto-cleanup guard for `atomic_install_dir`'s staging directory.
/// Removed in the success path via `disarm()`; on error or panic the
/// `Drop` impl best-effort cleans up to avoid orphan tmp/ entries.
struct StagingGuard {
    path: PathBuf,
}

impl StagingGuard {
    const fn disarm(self) {
        std::mem::forget(self);
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        let _ = remove_path(&self.path);
    }
}

// ── File-level helpers ─────────────────────────────────────────────

fn write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

fn atomic_write_file(path: &Path, bytes: &[u8]) -> StoreResult<()> {
    // Write to a sibling tmp file then rename — atomic on POSIX so
    // readers never see a partial file. Safer than direct fs::write
    // when crash-during-write is a concern.
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let counter = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".{}-{}-{}.tmp", std::process::id(), nanos, counter));
    write_file(&tmp, bytes).map_err(|e| StoreError::io(tmp.display().to_string(), e))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        StoreError::io(path.display().to_string(), e)
    })
}

fn read_optional_file(path: &Path) -> StoreResult<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(StoreError::io(path.display().to_string(), e)),
    }
}

fn remove_path(path: &Path) -> StoreResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(StoreError::io(path.display().to_string(), e)),
    };
    if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(|e| StoreError::io(path.display().to_string(), e))
    } else {
        fs::remove_file(path).map_err(|e| StoreError::io(path.display().to_string(), e))
    }
}

/// Walk `dir/*` (one hex-named subdir per hash). Remove subdirs whose
/// 32-byte hex name fails the `is_unreachable` predicate. Returns the
/// number removed.
fn sweep_hash_dir(
    dir: &Path,
    is_unreachable: impl Fn([u8; IMPL_HASH_LEN]) -> bool,
) -> StoreResult<usize> {
    if !dir.exists() {
        return Ok(0);
    }
    let mut removed = 0;
    for entry in fs::read_dir(dir).map_err(|e| StoreError::io(dir.display().to_string(), e))? {
        let entry = entry.map_err(|e| StoreError::io(dir.display().to_string(), e))?;
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let raw = match hex_decode(name) {
            Some(r) if r.len() == IMPL_HASH_LEN => r,
            // Skip non-hash entries — could be a future kind of file.
            _ => continue,
        };
        let mut arr = [0u8; IMPL_HASH_LEN];
        arr.copy_from_slice(&raw);
        if is_unreachable(arr) {
            remove_path(&p)?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Pull the ABI metadata section bytes back out of a `.khi`.
/// Used to populate `pkg/<hash>/manifest.bin` so the resolver can
/// re-read the metadata without parsing the whole pack.
fn extract_manifest_section(pack_bytes: &[u8]) -> StoreResult<Vec<u8>> {
    // The `.khi` header layout: MAGIC(4) + pack_version(4) +
    // section_count(4) + (section_id(1) + size(4) + body) repeated.
    // ABI_METADATA section_id is 1. We do a minimal walk here rather
    // than expose serde internals.
    if pack_bytes.len() < 12 || pack_bytes[..4] != [0x74, 0x72, 0x69, 0x70] {
        return Err(StoreError::Pack(crate::error::PackError::BadMagic));
    }
    let mut pos = 12;
    let section_count = u32::from_le_bytes(pack_bytes[8..12].try_into().unwrap_or([0; 4]));
    for _ in 0..section_count {
        if pos + 5 > pack_bytes.len() {
            return Err(StoreError::Pack(crate::error::PackError::Corrupted(
                "section header truncated".into(),
            )));
        }
        let id = pack_bytes[pos];
        let size =
            u32::from_le_bytes(pack_bytes[pos + 1..pos + 5].try_into().unwrap_or([0; 4])) as usize;
        pos += 5;
        let end = pos.checked_add(size).ok_or_else(|| {
            StoreError::Pack(crate::error::PackError::Corrupted(
                "section size overflows".into(),
            ))
        })?;
        if end > pack_bytes.len() {
            return Err(StoreError::Pack(crate::error::PackError::Corrupted(
                "section runs past EOF".into(),
            )));
        }
        if id == 1 {
            return Ok(pack_bytes[pos..end].to_vec());
        }
        pos = end;
    }
    Err(StoreError::Pack(crate::error::PackError::Corrupted(
        "no ABI metadata section in pack".into(),
    )))
}

/// Parse a manifest.bin (ABI metadata section bytes) without needing
/// the surrounding `.khi` framing. Reused by gc().
fn parse_manifest_only(manifest_bytes: &[u8]) -> Result<AbiMetadata, crate::error::PackError> {
    // Wrap the manifest in a minimal pack envelope so we can reuse
    // read_khi. Cheap because we control both producer + consumer.
    let mut wrap = Vec::with_capacity(manifest_bytes.len() + 20);
    wrap.extend_from_slice(&[0x74, 0x72, 0x69, 0x70]); // MAGIC
    wrap.extend_from_slice(&1u32.to_le_bytes()); // pack_version
    wrap.extend_from_slice(&1u32.to_le_bytes()); // section_count
    wrap.push(1); // ABI_METADATA section id
    wrap.extend_from_slice(&(manifest_bytes.len() as u32).to_le_bytes());
    wrap.extend_from_slice(manifest_bytes);
    let (meta, _code) = read_khi(&wrap)?;
    Ok(meta)
}

// ── Hex codec (no external crate to keep the dep tree small) ───────

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(hex_char(b >> 4));
        s.push(hex_char(b & 0x0F));
    }
    s
}

const fn hex_char(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => '0', // unreachable for masked nibble
    }
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_nibble(bytes[i])?;
        let lo = hex_nibble(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn invalid_data(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::{TermIfaceHash, TermImplHash};
    use crate::serde::write_khi;
    use crate::types::{FunctionExport, Param, SemVer, TypeRef, Visibility};
    use tempfile::TempDir;

    fn mk_pack(pkg: &str, version: SemVer, exports: &[&str]) -> Vec<u8> {
        let mut meta = AbiMetadata::empty(pkg, version);
        for name in exports {
            meta.exports.push(FunctionExport {
                name: (*name).into(),
                module_path: String::new(),
                visibility: Visibility::Public,
                type_params: Vec::new(),
                params: vec![Param {
                    name: "x".into(),
                    type_ref: TypeRef::Primitive(0x02),
                }],
                return_type: TypeRef::Primitive(0x02),
                body_offset: 0,
                iface_hash_term: TermIfaceHash::default(),
                impl_hash_term: TermImplHash::default(),
            });
        }
        write_khi(&meta, &[0xDE, 0xAD, 0xBE, 0xEF])
    }

    #[test]
    fn open_creates_layout() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        for sub in ["term", "mod", "pkg", "names", "roots", "tmp"] {
            assert!(store.root().join(sub).is_dir(), "missing {sub}");
        }
    }

    #[test]
    fn install_and_resolve_pack_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let bytes = mk_pack("math", SemVer::new(1, 0, 0), &["add", "sub"]);

        let hash = store.install_pack(&bytes).unwrap();
        let got = store.resolve_pack(&hash).unwrap().expect("present");
        assert_eq!(got, bytes);

        // manifest.bin readable too
        let manifest = store
            .resolve_manifest_bytes(&hash)
            .unwrap()
            .expect("manifest present");
        assert!(!manifest.is_empty());

        // term iface bytes were installed (dirs keyed by impl_hash per
        // ADR-0015 §2; iface.bin lives inside).
        let (meta, _) = read_khi(&bytes).unwrap();
        for e in &meta.exports {
            let iface = store
                .root()
                .join("term")
                .join(hex_encode(&e.impl_hash_term.0))
                .join("iface.bin");
            assert!(iface.exists(), "missing iface for {}", e.name);
        }
    }

    #[test]
    fn install_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let bytes = mk_pack("foo", SemVer::new(1, 0, 0), &["f"]);
        let h1 = store.install_pack(&bytes).unwrap();
        let h2 = store.install_pack(&bytes).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn name_resolution_returns_hash() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let bytes = mk_pack("foo", SemVer::new(1, 2, 3), &["f"]);
        let hash = store.install_pack(&bytes).unwrap();
        let resolved = store
            .resolve_by_name("foo", SemVer::new(1, 2, 3))
            .unwrap()
            .expect("found");
        assert_eq!(resolved, hash);
    }

    #[test]
    fn name_resolution_returns_none_for_missing() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let missing = store
            .resolve_by_name("ghost", SemVer::new(1, 0, 0))
            .unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn roots_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let bytes = mk_pack("foo", SemVer::new(1, 0, 0), &["f"]);
        let hash = store.install_pack(&bytes).unwrap();

        store.add_root("proj-abc", &[hash]).unwrap();
        let roots = store.list_roots().unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].project_id, "proj-abc");
        assert_eq!(roots[0].pkg_hashes, vec![hash]);

        store.remove_root("proj-abc").unwrap();
        assert!(store.list_roots().unwrap().is_empty());
    }

    #[test]
    fn gc_sweeps_unreferenced_packs() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let bytes_a = mk_pack("a", SemVer::new(1, 0, 0), &["fa"]);
        let bytes_b = mk_pack("b", SemVer::new(1, 0, 0), &["fb"]);
        let ha = store.install_pack(&bytes_a).unwrap();
        let hb = store.install_pack(&bytes_b).unwrap();

        // Reference only pkg b.
        store.add_root("proj", &[hb]).unwrap();

        let report = store.gc().unwrap();
        assert!(report.swept_pkgs >= 1, "expected to sweep pkg a");

        // a is gone, b stays.
        assert!(store.resolve_pack(&ha).unwrap().is_none());
        assert!(store.resolve_pack(&hb).unwrap().is_some());
    }

    #[test]
    fn gc_keeps_referenced_modules_and_terms() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let bytes = mk_pack("keep", SemVer::new(1, 0, 0), &["alpha"]);
        let h = store.install_pack(&bytes).unwrap();
        store.add_root("proj", &[h]).unwrap();

        // No sweep expected.
        let report = store.gc().unwrap();
        assert_eq!(report.swept_pkgs, 0);
        assert_eq!(report.swept_modules, 0);
        assert_eq!(report.swept_terms, 0);

        // Pack still readable.
        assert!(store.resolve_pack(&h).unwrap().is_some());
    }

    #[test]
    fn gc_wipes_tmp() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        // Plant a stray tmp/ entry (simulating a crashed install).
        let stray = store.root().join("tmp").join("stray-stuff");
        fs::create_dir_all(&stray).unwrap();
        fs::write(stray.join("orphan.bin"), b"x").unwrap();
        assert!(stray.exists());

        store.gc().unwrap();
        assert!(!stray.exists());
    }

    #[test]
    fn rejects_non_khi_bytes() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let err = store.install_pack(&[0, 0, 0, 0]).unwrap_err();
        assert!(matches!(
            err,
            StoreError::Pack(crate::error::PackError::BadMagic)
        ));
    }

    #[test]
    fn concurrent_install_same_hash_is_race_safe() {
        // Two `cargo build` jobs producing identical pack bytes will
        // race on the atomic rename. ADR-0015 §6 says EEXIST = race-loss
        // = success. Exercise that code path with real threads.
        use std::sync::Arc;
        use std::thread;

        let tmp = TempDir::new().unwrap();
        let store = Arc::new(Store::open(tmp.path()).unwrap());
        let pack = mk_pack("racy", SemVer::new(1, 0, 0), &["f"]);

        let n_threads = 8;
        let handles: Vec<_> = (0..n_threads)
            .map(|_| {
                let s = Arc::clone(&store);
                let p = pack.clone();
                thread::spawn(move || s.install_pack(&p))
            })
            .collect();

        let mut hashes = Vec::with_capacity(n_threads);
        for h in handles {
            hashes.push(h.join().expect("thread panicked").expect("install failed"));
        }

        // All threads agree on the same hash.
        let first = hashes[0];
        for h in &hashes {
            assert_eq!(*h, first);
        }

        // Exactly one pkg dir landed — race losers cleaned up after
        // themselves and did not produce duplicate entries.
        let pkg_dirs: Vec<_> = fs::read_dir(store.root().join("pkg"))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(pkg_dirs.len(), 1, "expected exactly one pkg dir after race");

        // tmp/ is empty — all stagers cleaned up.
        let tmp_dir = store.root().join("tmp");
        let leftovers: Vec<_> = fs::read_dir(&tmp_dir)
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(
            leftovers.is_empty(),
            "tmp/ should be empty after install race, got {} entries",
            leftovers.len()
        );
    }

    #[test]
    fn gc_keeps_pkg_referenced_by_multiple_roots() {
        // CAS invariant: a pkg referenced by ≥1 root must survive GC.
        // Two projects depending on the same lib then removing only
        // one of the roots must NOT sweep the lib.
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let bytes = mk_pack("shared", SemVer::new(1, 0, 0), &["f"]);
        let h = store.install_pack(&bytes).unwrap();

        store.add_root("proj-a", &[h]).unwrap();
        store.add_root("proj-b", &[h]).unwrap();

        // Drop one root — the other still pins the pkg.
        store.remove_root("proj-a").unwrap();

        let report = store.gc().unwrap();
        assert_eq!(report.swept_pkgs, 0, "shared pkg must survive");
        assert_eq!(report.swept_modules, 0);
        assert_eq!(report.swept_terms, 0);
        assert!(store.resolve_pack(&h).unwrap().is_some());

        // Drop the second root — now nothing pins it. GC should sweep.
        store.remove_root("proj-b").unwrap();
        let report2 = store.gc().unwrap();
        assert!(report2.swept_pkgs >= 1);
        assert!(store.resolve_pack(&h).unwrap().is_none());
    }

    #[test]
    fn gc_preserves_mods_terms_when_manifest_corrupt() {
        // If a live pkg's manifest.bin is unreadable, gc() can't
        // enumerate its mod/term references — sweeping under that
        // ambiguity could orphan still-needed dirs. Conservative
        // behaviour: report corruption, skip mod + term sweeps.
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let bytes = mk_pack("victim", SemVer::new(1, 0, 0), &["alpha", "beta"]);
        let h = store.install_pack(&bytes).unwrap();
        store.add_root("proj", &[h]).unwrap();

        // Count term dirs before corruption (should match exports).
        let term_root = store.root().join("term");
        let term_count_before = fs::read_dir(&term_root).unwrap().count();
        assert!(term_count_before > 0, "expected installed term dirs");

        // Smash the manifest.
        let manifest_path = store
            .root()
            .join("pkg")
            .join(hex_encode(&h.0))
            .join("manifest.bin");
        fs::write(&manifest_path, b"not a valid manifest").unwrap();

        let report = store.gc().unwrap();

        // Corruption reported.
        assert_eq!(report.corrupt_pkgs, vec![h]);
        // Mod + term sweeps suppressed (conservative).
        assert_eq!(report.swept_modules, 0);
        assert_eq!(report.swept_terms, 0);

        // Term dirs survive — the gc didn't silently orphan them.
        let term_count_after = fs::read_dir(&term_root).unwrap().count();
        assert_eq!(term_count_after, term_count_before);
    }
}
