//! TTY prompt UX for capability resolution (ADR-0018 §4 +
//! ADR-0017 Addendum §B).
//!
//! When [`CapabilityResolver`](crate::CapabilityResolver) hits a
//! `prompt` rule with a TTY available, it delegates to a
//! [`PromptCallback`]. v0.6.10 ships:
//!
//! - **Pure rendering** — [`render_prompt`] + [`read_choice`] are
//!   generic over `Write` / `BufRead`, so tests use `Cursor<Vec<u8>>`
//!   without touching `/dev/tty`.
//! - **Provenance display** — [`PromptContext`] carries the full ADR
//!   §4 anti-typosquatting payload: full 64-hex `iface_hash` /
//!   `impl_hash`, CAS store path, per-dep [`ResolutionOrigin`]
//!   labels, lockfile cross-check status. Optional fields render
//!   gracefully — v0.6.10 produces a sparse rendering from the
//!   bare [`PolicyRequest`]; v0.6.11 loader integration enriches.
//! - **`/dev/tty` integration** — [`DevTtyPrompt`] opens
//!   `/dev/tty` on POSIX (ADR-0017 Addendum §B authoritative check),
//!   bypassing stdin/stderr to defeat pipe-injection spoofing.
//!   Non-POSIX platforms fall through to an `Unsupported` `io::Error`
//!   stub — Windows ConPTY (`CONIN$`/`CONOUT$`) is deferred.
//! - **`G`/`D` permanent write** — DevTtyPrompt reuses
//!   [`PolicyRules::save`](crate::PolicyRules::save) atomic-temp-rename
//!   to append the user's choice to `triet.policy`. ADR-0018 §4
//!   write-before-cache semantics.
//!
//! ## Security boundary decisions
//!
//! - **`/dev/tty` not stdin** — pipe-injection (`echo G | triet …`)
//!   cannot reach the prompt; the kernel allocates a separate
//!   handle bound to the controlling terminal.
//! - **No `isatty(stderr)` pre-screen** — authoritative check is the
//!   open itself per user decision (skip the optimisation;
//!   marginally slower on headless runs but eliminates a class of
//!   spoofing where `isatty(stderr) == true` but `/dev/tty` opens
//!   into a different device).
//! - **ASCII-only markers** — `!!` instead of `⚠`; security messages
//!   must render universally.
//! - **No hash truncation** — full 64 hex always; short-SHA is a
//!   collision attack surface in a security context.
//! - **No ANSI colour at v0.6.10** — colour is a usability win, not
//!   security; ship after the security floor is solid. Future patch
//!   honours `$NO_COLOR`.
//!
//! ## What v0.6.10 does NOT do
//!
//! - Wire deferrals from the linker into the resolver — that's
//!   v0.6.11.
//! - Build a rich [`PromptContext`] from real package metadata —
//!   v0.6.11 wires the loader.
//! - Windows ConPTY — stub `io::Error::Unsupported` per platform
//!   focus.
//! - Box-drawing Unicode + ANSI colour — deferred.

use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

use crate::capability_resolver::PolicyRequest;
use crate::policy::{Decision, OriginMatcher, PolicyRule, PolicyRules};
use crate::resolver::ResolutionOrigin;

/// Full provenance payload rendered to the TTY when a `prompt` rule
/// fires. v0.6.10 [`context_from_request`] fills only the
/// [`PolicyRequest`]-side fields; loader integration in v0.6.11
/// enriches `requester` + per-dep entries with hashes, store path,
/// and lockfile cross-check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptContext {
    /// Module path being defer-resolved.
    pub cap_path: String,
    /// How the upstream resolver picked the package — drives the
    /// "Decision token" sub-line of the prompt and the rule-key
    /// dimension when the user picks `G`/`D`.
    pub origin: OriginMatcher,
    /// Human-readable reason the prompt fired (e.g. "per triet.policy
    /// rule" / "no rule matched"). Caller-provided.
    pub decision_reason: String,
    /// The package requesting the capability — the typosquatting
    /// target.
    pub requester: PackageInfo,
    /// Transitive dep chain from root → requester. Empty for
    /// root-self requests.
    pub dep_chain: Vec<DepChainEntry>,
}

/// Provenance fields for the requesting package. Optional fields
/// render only when populated — v0.6.10 leaves them `None`; v0.6.11
/// fills them from `AbiMetadata` + `Store` + `Lockfile`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageInfo {
    /// Package name without version (e.g. `myapp`).
    pub name: String,
    /// Package version triple as string (e.g. `0.1.0`). `None` until
    /// loader integration.
    pub version: Option<String>,
    /// Full 64-hex `iface_hash`. NEVER truncated — short-SHA is a
    /// collision attack surface.
    pub iface_hash: Option<String>,
    /// Full 64-hex `impl_hash`.
    pub impl_hash: Option<String>,
    /// CAS store path of the pack — `~/.triet/store/pkg/<hash>/…`.
    pub store_path: Option<String>,
    /// Lockfile cross-check result — `iface_hash matches`,
    /// `MISMATCH`, or `not in lockfile`.
    pub lockfile_match: Option<LockfileMatch>,
}

/// Outcome of cross-checking the requester's `iface_hash` against
/// the project's `triet.lock`. Rendered as a parenthesised hint
/// after the hash line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockfileMatch {
    /// Lockfile has an entry for the package and the hash matches.
    Match,
    /// Lockfile has an entry but the hash differs (typosquat
    /// signal) — full hex of the lockfile value carried for the
    /// diagnostic.
    Mismatch {
        /// The hash recorded in `triet.lock` (full 64 hex).
        lockfile_hash: String,
    },
    /// Package isn't in the lockfile — common for fresh deps.
    NotInLockfile,
}

/// One node in the dep-chain visualisation block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepChainEntry {
    /// Package name + version, e.g. `libdns@1.2.3`.
    pub name_version: String,
    /// Full 64-hex `iface_hash` of this dep. Optional for v0.6.10.
    pub iface_hash: Option<String>,
    /// Why the resolver picked this dep (`Fresh` / `Lockfile` /
    /// `IfacePin`). v0.6.10 carries strings — v0.6.11 will pull
    /// from `Resolution.origin`.
    pub origin: Option<String>,
    /// Whether this dep is part of the root package (no parent).
    pub is_root: bool,
}

/// The user's resolution. v0.6.10 ships four terminal choices —
/// session vs. permanent times grant vs. deny. `Explain` and
/// `ShowHashHelp` raw inputs trigger a re-prompt and never reach
/// the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptChoice {
    /// `g` — grant for this session only.
    GrantOnce,
    /// `d` — deny for this session only.
    DenyOnce,
    /// `G` — grant + write `rule … +1` to `triet.policy`.
    GrantPermanent,
    /// `D` — deny + write `rule … -1` to `triet.policy`.
    DenyPermanent,
}

/// Strategy for resolving a defer-cap that reached the prompt path.
/// Held by [`CapabilityResolver`](crate::CapabilityResolver) as
/// `Box<dyn PromptCallback>` so the I/O layer can be swapped (tests
/// inject a fixed-response mock; production uses
/// [`DevTtyPrompt`]).
pub trait PromptCallback: Send {
    /// Render the prompt and read the user's choice. Returns an
    /// `io::Error` for TTY I/O failures — the resolver maps that to
    /// `ResolverError::PromptCrash`.
    ///
    /// # Errors
    /// Returns `io::Error` when the TTY can't be opened, when read
    /// or write fails, or when the callback is configured in
    /// non-interactive mode.
    fn prompt(&mut self, req: &PolicyRequest) -> io::Result<PromptChoice>;
}

/// Raw choice parsed from a single line of user input. Includes
/// non-terminal options (`Explain` / `ShowHashHelp`) and the catch-
/// all `Invalid` so the prompt loop can re-display on bad input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RawChoice {
    GrantOnce,
    DenyOnce,
    GrantPermanent,
    DenyPermanent,
    Explain,
    ShowHashHelp,
    Invalid,
}

impl RawChoice {
    fn parse(line: &str) -> Self {
        // First non-whitespace ASCII byte decides — extra trailing
        // junk is ignored so users can type `g\n` or `g foo\n`
        // identically.
        match line.trim().chars().next() {
            Some('g') => Self::GrantOnce,
            Some('d') => Self::DenyOnce,
            Some('G') => Self::GrantPermanent,
            Some('D') => Self::DenyPermanent,
            Some('?') => Self::Explain,
            Some('h') => Self::ShowHashHelp,
            _ => Self::Invalid,
        }
    }
}

/// Render the full prompt block to `writer`. Pure function — no TTY,
/// no I/O beyond the writer. Lines for optional fields are omitted
/// when the field is `None`.
///
/// Format mirrors ADR-0018 §4 mock. ASCII-only (`!!` markers); full
/// hex hashes; per-dep origin labels.
///
/// # Errors
/// Forwards any `io::Error` from the underlying writer.
pub fn render_prompt<W: Write>(writer: &mut W, ctx: &PromptContext) -> io::Result<()> {
    writeln!(writer, "[triet] Capability decision required")?;
    writeln!(writer)?;
    writeln!(writer, "  Capability:     {}", ctx.cap_path)?;
    writeln!(
        writer,
        "  Decision token: prompt  ({}, origin={})",
        ctx.decision_reason,
        ctx.origin.as_token(),
    )?;
    writeln!(writer)?;

    writeln!(writer, "  Requester (package asking):")?;
    let pkg_label = ctx.requester.version.as_ref().map_or_else(
        || ctx.requester.name.clone(),
        |v| format!("{}@{}", ctx.requester.name, v),
    );
    writeln!(writer, "    Name:        {pkg_label}")?;
    if let Some(h) = &ctx.requester.iface_hash {
        let suffix = match &ctx.requester.lockfile_match {
            Some(LockfileMatch::Match) => "   (matches triet.lock OK)",
            Some(LockfileMatch::Mismatch { .. }) => "   !! MISMATCH vs triet.lock",
            Some(LockfileMatch::NotInLockfile) => "   (not in lockfile)",
            None => "",
        };
        writeln!(writer, "    iface_hash:  {h}{suffix}")?;
        if let Some(LockfileMatch::Mismatch { lockfile_hash }) = &ctx.requester.lockfile_match {
            // Show the lockfile hash on its own line so the user can
            // compare full 64 hex against full 64 hex side by side.
            writeln!(writer, "                 lockfile was: {lockfile_hash}")?;
        }
    }
    if let Some(h) = &ctx.requester.impl_hash {
        writeln!(writer, "    impl_hash:   {h}")?;
    }
    if let Some(p) = &ctx.requester.store_path {
        writeln!(writer, "    Store path:  {p}")?;
    }
    writeln!(writer)?;

    if !ctx.dep_chain.is_empty() {
        writeln!(writer, "  Dep chain:")?;
        for (idx, entry) in ctx.dep_chain.iter().enumerate() {
            let prefix = if entry.is_root || idx == 0 {
                "    "
            } else {
                "    └─ "
            };
            writeln!(writer, "{prefix}{}", entry.name_version)?;
            if let Some(h) = &entry.iface_hash {
                writeln!(writer, "         iface_hash:  {h}")?;
            }
            match (&entry.origin, entry.is_root) {
                (Some(o), false) => {
                    let marker = if o == "Fresh" { "    !! NOT in lockfile" } else { "" };
                    writeln!(writer, "         origin={o}{marker}")?;
                }
                (_, true) => writeln!(writer, "         (root)")?,
                (None, false) => {}
            }
        }
        writeln!(writer)?;
    }

    if ctx
        .dep_chain
        .iter()
        .any(|e| e.origin.as_deref() == Some("Fresh"))
    {
        writeln!(
            writer,
            "  !! Fresh deps were added since the last lockfile commit.",
        )?;
        writeln!(writer, "  !! Verify hash against your records before granting.")?;
        writeln!(writer)?;
    }

    writeln!(writer, "  [g] grant once   [d] deny once")?;
    writeln!(writer, "  [G] grant permanent (write rule to triet.policy)")?;
    writeln!(writer, "  [D] deny permanent  (write rule to triet.policy)")?;
    writeln!(writer, "  [?] explain   [h] show hash help")?;
    writeln!(writer)?;
    write!(writer, "  choice > ")?;
    writer.flush()
}

/// Read one line from `reader`, parse as [`RawChoice`]. EOF or empty
/// line returns [`RawChoice::Invalid`] so the prompt loop re-displays
/// without forward-progressing.
///
/// # Errors
/// Forwards any non-EOF `io::Error` from the reader.
pub(crate) fn read_choice<R: BufRead>(reader: &mut R) -> io::Result<RawChoice> {
    let mut line = String::new();
    let n = reader.read_line(&mut line)?;
    if n == 0 {
        // EOF — invalid input, loop will re-prompt or fail.
        return Ok(RawChoice::Invalid);
    }
    Ok(RawChoice::parse(&line))
}

/// Print the `[?]` explanation. The user typically reads this once
/// to understand why the prompt fired, then re-enters their choice.
fn print_explanation<W: Write>(writer: &mut W, ctx: &PromptContext) -> io::Result<()> {
    writeln!(writer)?;
    writeln!(writer, "  --- Explanation ---")?;
    writeln!(
        writer,
        "  Capability `{}` reached the runtime prompt because no static",
        ctx.cap_path,
    )?;
    writeln!(writer, "  rule resolved it. Possible reasons:")?;
    writeln!(
        writer,
        "    1. A triet.policy rule matched `(cap_path, origin)` and said `prompt`.",
    )?;
    writeln!(writer, "    2. No rule matched and the resolver fell back to default.")?;
    writeln!(
        writer,
        "  Grant once (g/d) for this session only, or permanent (G/D) to",
    )?;
    writeln!(writer, "  write the rule to triet.policy for future sessions.")?;
    writeln!(writer)?;
    writer.flush()
}

/// Print the `[h]` hash-verification help text.
fn print_hash_help<W: Write>(writer: &mut W) -> io::Result<()> {
    writeln!(writer)?;
    writeln!(writer, "  --- Hash help ---")?;
    writeln!(
        writer,
        "  iface_hash and impl_hash are BLAKE3 (full 64 hex chars, never truncated).",
    )?;
    writeln!(
        writer,
        "  Compare them against your records (lockfile, registry, colleague's build).",
    )?;
    writeln!(
        writer,
        "  Hash mismatch = different package even if the name is the same. Refuse",
    )?;
    writeln!(writer, "  if unsure — typosquatters reuse names but cannot fake hashes.")?;
    writeln!(writer)?;
    writer.flush()
}

/// Drive the prompt loop on caller-supplied I/O streams. Returns the
/// user's terminal choice after re-prompting through Explain /
/// ShowHashHelp / Invalid as needed.
///
/// Decoupled from `/dev/tty` opening so tests can supply
/// `Cursor<Vec<u8>>` pairs.
///
/// # Errors
/// Forwards any `io::Error` from `reader` or `writer`.
pub fn prompt_loop<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    ctx: &PromptContext,
) -> io::Result<PromptChoice> {
    loop {
        render_prompt(writer, ctx)?;
        match read_choice(reader)? {
            RawChoice::GrantOnce => return Ok(PromptChoice::GrantOnce),
            RawChoice::DenyOnce => return Ok(PromptChoice::DenyOnce),
            RawChoice::GrantPermanent => return Ok(PromptChoice::GrantPermanent),
            RawChoice::DenyPermanent => return Ok(PromptChoice::DenyPermanent),
            RawChoice::Explain => print_explanation(writer, ctx)?,
            RawChoice::ShowHashHelp => print_hash_help(writer)?,
            RawChoice::Invalid => {
                writeln!(writer, "  !! invalid choice — please pick one of g/d/G/D/?/h")?;
                writeln!(writer)?;
            }
        }
    }
}

/// Build a sparse [`PromptContext`] from a [`PolicyRequest`] alone.
/// v0.6.10 default — populates `cap_path`, `origin`, requester name,
/// and dep-chain names. v0.6.11 loader will replace this with an
/// enriching builder that fills hashes + store path + lockfile match.
#[must_use]
pub fn context_from_request(req: &PolicyRequest) -> PromptContext {
    let origin = match req.origin {
        ResolutionOrigin::Lockfile => OriginMatcher::Lockfile,
        ResolutionOrigin::IfacePin => OriginMatcher::IfacePin,
        ResolutionOrigin::Fresh => OriginMatcher::Fresh,
    };
    let origin_label = match req.origin {
        ResolutionOrigin::Lockfile => "Lockfile",
        ResolutionOrigin::IfacePin => "IfacePin",
        ResolutionOrigin::Fresh => "Fresh",
    };
    let dep_chain = req
        .dep_chain
        .iter()
        .enumerate()
        .map(|(idx, name)| DepChainEntry {
            name_version: name.clone(),
            iface_hash: None,
            origin: if idx == 0 { None } else { Some(origin_label.into()) },
            is_root: idx == 0,
        })
        .collect();
    PromptContext {
        cap_path: req.cap_path.clone(),
        origin,
        decision_reason: "per triet.policy rule".into(),
        requester: PackageInfo {
            name: req.requester_pkg.clone(),
            version: None,
            iface_hash: None,
            impl_hash: None,
            store_path: None,
            lockfile_match: None,
        },
        dep_chain,
    }
}

/// Production [`PromptCallback`] — opens `/dev/tty` per ADR-0017
/// Addendum §B, renders the prompt, reads the user's choice, and
/// writes back to `triet.policy` on `G`/`D`. Non-POSIX platforms
/// return [`io::ErrorKind::Unsupported`].
pub struct DevTtyPrompt {
    policy_path: PathBuf,
    non_interactive: bool,
}

impl DevTtyPrompt {
    /// New prompt that writes `G`/`D` choices to `policy_path`. The
    /// file is loaded with [`PolicyRules::load`] (treats `NotFound` as
    /// empty) so the first prompt can run before any policy file
    /// exists; the save uses [`PolicyRules::save`] atomic temp+rename
    /// (ADR-0015 §5).
    #[must_use]
    pub fn new(policy_path: impl Into<PathBuf>) -> Self {
        Self {
            policy_path: policy_path.into(),
            non_interactive: false,
        }
    }

    /// Builder — when `true`, all prompt attempts immediately return
    /// `io::ErrorKind::Unsupported`. Equivalent to running the binary
    /// with `--non-interactive` (ADR-0018 §4 lock decision).
    #[must_use]
    pub const fn non_interactive(mut self, flag: bool) -> Self {
        self.non_interactive = flag;
        self
    }

    /// Write the user's permanent decision (`G`/`D`) to the policy
    /// file via load-modify-save. Tries to be best-effort — on
    /// failure the caller can downgrade `GrantPermanent` to
    /// `GrantOnce` (ADR-0018 §4 "fallback session-only" hint).
    fn write_permanent(&self, req: &PolicyRequest, decision: Decision) -> io::Result<()> {
        let mut rules = PolicyRules::load(&self.policy_path)
            .map_err(|e| io::Error::other(format!("triet.policy load: {e}")))?;
        let origin = match req.origin {
            ResolutionOrigin::Lockfile => OriginMatcher::Lockfile,
            ResolutionOrigin::IfacePin => OriginMatcher::IfacePin,
            ResolutionOrigin::Fresh => OriginMatcher::Fresh,
        };
        rules.upsert_rule(PolicyRule {
            cap_path: req.cap_path.clone(),
            origin,
            decision,
        });
        rules
            .save(&self.policy_path)
            .map_err(|e| io::Error::other(format!("triet.policy save: {e}")))
    }
}

impl fmt::Debug for DevTtyPrompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DevTtyPrompt")
            .field("policy_path", &self.policy_path)
            .field("non_interactive", &self.non_interactive)
            .finish()
    }
}

impl PromptCallback for DevTtyPrompt {
    fn prompt(&mut self, req: &PolicyRequest) -> io::Result<PromptChoice> {
        if self.non_interactive {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "--non-interactive: TTY prompt suppressed",
            ));
        }
        let (mut reader, mut writer) = open_dev_tty()?;
        let ctx = context_from_request(req);
        let choice = prompt_loop(&mut reader, &mut writer, &ctx)?;
        match choice {
            PromptChoice::GrantPermanent => {
                if let Err(e) = self.write_permanent(req, Decision::Plus1) {
                    // ADR-0018 §4: on write failure, warn + fall back
                    // to session-only.
                    let _ = writeln!(
                        writer,
                        "  !! policy write failed ({e}) — granting for this session only",
                    );
                    return Ok(PromptChoice::GrantOnce);
                }
            }
            PromptChoice::DenyPermanent => {
                if let Err(e) = self.write_permanent(req, Decision::Minus1) {
                    let _ = writeln!(
                        writer,
                        "  !! policy write failed ({e}) — denying for this session only",
                    );
                    return Ok(PromptChoice::DenyOnce);
                }
            }
            PromptChoice::GrantOnce | PromptChoice::DenyOnce => {}
        }
        Ok(choice)
    }
}

/// Open `/dev/tty` as a (reader, writer) pair on POSIX, bypassing
/// stdin/stderr for anti-spoofing per ADR-0017 Addendum §B.
///
/// Non-POSIX → `Unsupported`. Windows ConPTY (`CONIN$`/`CONOUT$`)
/// deferred — user-base focus is Linux.
#[cfg(unix)]
fn open_dev_tty() -> io::Result<(BufReader<std::fs::File>, std::fs::File)> {
    let read_handle = OpenOptions::new().read(true).open("/dev/tty")?;
    let write_handle = OpenOptions::new().write(true).open("/dev/tty")?;
    Ok((BufReader::new(read_handle), write_handle))
}

#[cfg(not(unix))]
fn open_dev_tty() -> io::Result<(BufReader<std::fs::File>, std::fs::File)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "TTY prompt is POSIX-only; Windows ConPTY is not implemented yet",
    ))
}

/// Convenience: build a non-interactive [`DevTtyPrompt`] that always
/// returns `Unsupported`. Useful as a callback for loaders that want
/// the resolver to fail-closed via [`crate::ResolverError::PromptCrash`]
/// rather than `NonTTYDefer`.
///
/// `policy_path` is accepted but unused — non-interactive mode never
/// writes.
#[must_use]
pub fn non_interactive_callback(policy_path: impl Into<PathBuf>) -> Box<dyn PromptCallback> {
    Box::new(DevTtyPrompt::new(policy_path).non_interactive(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn req(cap_path: &str, requester_pkg: &str, origin: ResolutionOrigin) -> PolicyRequest {
        PolicyRequest {
            cap_path: cap_path.into(),
            requester_pkg: requester_pkg.into(),
            dep_chain: vec![],
            origin,
        }
    }

    fn rich_ctx() -> PromptContext {
        PromptContext {
            cap_path: "sys.net.dns".into(),
            origin: OriginMatcher::Fresh,
            decision_reason: "per triet.policy rule".into(),
            requester: PackageInfo {
                name: "myapp".into(),
                version: Some("0.1.0".into()),
                iface_hash: Some(
                    "e7a1c4f0b2d8a629f4e8d0c7b3a51928f6e2d9c8a4b3f7e9d8c6a2b1f5e3d829".into(),
                ),
                impl_hash: Some(
                    "91b3d8e2a4c7d935a8e6f0b2d4c97186a3e5f8d2c0b4a791e2f5c8d9a04af5b6".into(),
                ),
                store_path: Some(
                    "~/.triet/store/pkg/91b3d8e2a4c7d935a8e6f0b2d4c97186a3e5f8d2c0b4a791e2f5c8d9a04af5b6/pack.tripack"
                        .into(),
                ),
                lockfile_match: Some(LockfileMatch::Match),
            },
            dep_chain: vec![
                DepChainEntry {
                    name_version: "myapp@0.1.0".into(),
                    iface_hash: Some(
                        "e7a1c4f0b2d8a629f4e8d0c7b3a51928f6e2d9c8a4b3f7e9d8c6a2b1f5e3d829".into(),
                    ),
                    origin: None,
                    is_root: true,
                },
                DepChainEntry {
                    name_version: "libdns@1.2.3".into(),
                    iface_hash: Some(
                        "5c92ab17d4e8c1f6a3b8d2e5c97014b6f3e8d2a4c5b1f9e6d8c3a2b4f7e1d503".into(),
                    ),
                    origin: Some("Fresh".into()),
                    is_root: false,
                },
            ],
        }
    }

    // ── RawChoice parsing ──────────────────────────────────────────

    #[test]
    fn raw_choice_recognises_terminal_keys() {
        assert_eq!(RawChoice::parse("g"), RawChoice::GrantOnce);
        assert_eq!(RawChoice::parse("d"), RawChoice::DenyOnce);
        assert_eq!(RawChoice::parse("G"), RawChoice::GrantPermanent);
        assert_eq!(RawChoice::parse("D"), RawChoice::DenyPermanent);
    }

    #[test]
    fn raw_choice_recognises_help_keys() {
        assert_eq!(RawChoice::parse("?"), RawChoice::Explain);
        assert_eq!(RawChoice::parse("h"), RawChoice::ShowHashHelp);
    }

    #[test]
    fn raw_choice_invalid_for_unknown_input() {
        assert_eq!(RawChoice::parse(""), RawChoice::Invalid);
        assert_eq!(RawChoice::parse("\n"), RawChoice::Invalid);
        assert_eq!(RawChoice::parse("yes"), RawChoice::Invalid);
        assert_eq!(RawChoice::parse("x"), RawChoice::Invalid);
    }

    #[test]
    fn raw_choice_trims_and_takes_first_char() {
        assert_eq!(RawChoice::parse("  g  "), RawChoice::GrantOnce);
        assert_eq!(RawChoice::parse("g whatever junk"), RawChoice::GrantOnce);
    }

    // ── render_prompt output ──────────────────────────────────────

    #[test]
    fn render_full_context_includes_all_provenance_lines() {
        let ctx = rich_ctx();
        let mut buf = Vec::new();
        render_prompt(&mut buf, &ctx).unwrap();
        let out = String::from_utf8(buf).unwrap();

        assert!(out.contains("Capability:     sys.net.dns"));
        assert!(out.contains("origin=fresh"));
        assert!(out.contains("Name:        myapp@0.1.0"));
        assert!(out.contains(
            "iface_hash:  e7a1c4f0b2d8a629f4e8d0c7b3a51928f6e2d9c8a4b3f7e9d8c6a2b1f5e3d829"
        ));
        assert!(out.contains("(matches triet.lock OK)"));
        assert!(out.contains(
            "impl_hash:   91b3d8e2a4c7d935a8e6f0b2d4c97186a3e5f8d2c0b4a791e2f5c8d9a04af5b6"
        ));
        assert!(out.contains("Store path:  ~/.triet/store/pkg/"));
        assert!(out.contains("libdns@1.2.3"));
        assert!(out.contains("origin=Fresh"));
        assert!(out.contains("!! NOT in lockfile"));
        assert!(out.contains("[g] grant once"));
        assert!(out.contains("[G] grant permanent"));
        assert!(out.contains("[?] explain"));
        assert!(out.contains("choice > "));
    }

    #[test]
    fn render_never_truncates_hashes() {
        let ctx = rich_ctx();
        let mut buf = Vec::new();
        render_prompt(&mut buf, &ctx).unwrap();
        let out = String::from_utf8(buf).unwrap();
        // ASCII ellipsis `...` is the canonical truncation indicator;
        // ADR-0018 §4 forbids it in this security context.
        assert!(
            !out.contains("..."),
            "render must never truncate hashes — full hex always",
        );
    }

    #[test]
    fn render_uses_ascii_warning_markers() {
        let ctx = rich_ctx();
        let mut buf = Vec::new();
        render_prompt(&mut buf, &ctx).unwrap();
        let out = String::from_utf8(buf).unwrap();
        // ⚠ is U+26A0 — universally render-able terminals can't be
        // assumed. Stick with `!!` per ADR-0018 §4 lock decision.
        assert!(!out.contains('\u{26A0}'), "no Unicode warning sigil");
        assert!(out.contains("!! "));
    }

    #[test]
    fn render_sparse_context_omits_missing_fields() {
        let req = req("sys.io", "myapp", ResolutionOrigin::Fresh);
        let ctx = context_from_request(&req);
        let mut buf = Vec::new();
        render_prompt(&mut buf, &ctx).unwrap();
        let out = String::from_utf8(buf).unwrap();

        assert!(out.contains("Capability:     sys.io"));
        assert!(out.contains("Name:        myapp"));
        // iface_hash / impl_hash / store path / lockfile match are
        // None → corresponding lines absent.
        assert!(!out.contains("iface_hash:"));
        assert!(!out.contains("impl_hash:"));
        assert!(!out.contains("Store path:"));
        assert!(!out.contains("matches triet.lock"));
    }

    #[test]
    fn render_mismatch_shows_both_hashes() {
        let mut ctx = rich_ctx();
        ctx.requester.lockfile_match = Some(LockfileMatch::Mismatch {
            lockfile_hash:
                "aaaaaaaa11111111bbbbbbbb22222222cccccccc33333333dddddddd44444444".into(),
        });
        let mut buf = Vec::new();
        render_prompt(&mut buf, &ctx).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("MISMATCH vs triet.lock"));
        assert!(out.contains(
            "lockfile was: aaaaaaaa11111111bbbbbbbb22222222cccccccc33333333dddddddd44444444"
        ));
    }

    // ── prompt_loop happy paths ────────────────────────────────────

    #[test]
    fn prompt_loop_returns_grant_once_on_g() {
        let mut reader = Cursor::new(b"g\n".to_vec());
        let mut writer = Vec::new();
        let ctx = context_from_request(&req("sys.io", "myapp", ResolutionOrigin::Fresh));
        let choice = prompt_loop(&mut reader, &mut writer, &ctx).unwrap();
        assert_eq!(choice, PromptChoice::GrantOnce);
    }

    #[test]
    fn prompt_loop_returns_deny_once_on_d() {
        let mut reader = Cursor::new(b"d\n".to_vec());
        let mut writer = Vec::new();
        let ctx = context_from_request(&req("sys.io", "myapp", ResolutionOrigin::Fresh));
        assert_eq!(
            prompt_loop(&mut reader, &mut writer, &ctx).unwrap(),
            PromptChoice::DenyOnce,
        );
    }

    #[test]
    fn prompt_loop_returns_permanent_variants() {
        let ctx = context_from_request(&req("sys.io", "myapp", ResolutionOrigin::Fresh));

        let mut reader_g = Cursor::new(b"G\n".to_vec());
        let mut writer_g = Vec::new();
        assert_eq!(
            prompt_loop(&mut reader_g, &mut writer_g, &ctx).unwrap(),
            PromptChoice::GrantPermanent,
        );

        let mut reader_d = Cursor::new(b"D\n".to_vec());
        let mut writer_d = Vec::new();
        assert_eq!(
            prompt_loop(&mut reader_d, &mut writer_d, &ctx).unwrap(),
            PromptChoice::DenyPermanent,
        );
    }

    // ── prompt_loop re-prompting ──────────────────────────────────

    #[test]
    fn prompt_loop_reprompts_on_explain_then_terminal() {
        // First `?` triggers Explain (re-prompt), then `g` resolves.
        let mut reader = Cursor::new(b"?\ng\n".to_vec());
        let mut writer = Vec::new();
        let ctx = context_from_request(&req("sys.io", "myapp", ResolutionOrigin::Fresh));
        let choice = prompt_loop(&mut reader, &mut writer, &ctx).unwrap();
        assert_eq!(choice, PromptChoice::GrantOnce);

        let out = String::from_utf8(writer).unwrap();
        assert!(out.contains("--- Explanation ---"));
        // Prompt rendered twice (once before ?, once after).
        assert!(out.matches("Capability:     sys.io").count() == 2);
    }

    #[test]
    fn prompt_loop_reprompts_on_hash_help_then_terminal() {
        let mut reader = Cursor::new(b"h\nD\n".to_vec());
        let mut writer = Vec::new();
        let ctx = context_from_request(&req("sys.io", "myapp", ResolutionOrigin::Fresh));
        let choice = prompt_loop(&mut reader, &mut writer, &ctx).unwrap();
        assert_eq!(choice, PromptChoice::DenyPermanent);

        let out = String::from_utf8(writer).unwrap();
        assert!(out.contains("--- Hash help ---"));
        assert!(out.contains("BLAKE3"));
    }

    #[test]
    fn prompt_loop_reprompts_on_invalid_input() {
        let mut reader = Cursor::new(b"x\nyes\ng\n".to_vec());
        let mut writer = Vec::new();
        let ctx = context_from_request(&req("sys.io", "myapp", ResolutionOrigin::Fresh));
        let choice = prompt_loop(&mut reader, &mut writer, &ctx).unwrap();
        assert_eq!(choice, PromptChoice::GrantOnce);

        let out = String::from_utf8(writer).unwrap();
        assert!(out.matches("!! invalid choice").count() == 2);
    }

    // ── context_from_request ──────────────────────────────────────

    #[test]
    fn context_from_request_carries_basic_fields() {
        let r = PolicyRequest {
            cap_path: "sys.net.dns".into(),
            requester_pkg: "myapp".into(),
            dep_chain: vec!["myapp".into(), "libdns".into(), "libtls".into()],
            origin: ResolutionOrigin::Fresh,
        };
        let ctx = context_from_request(&r);
        assert_eq!(ctx.cap_path, "sys.net.dns");
        assert_eq!(ctx.origin, OriginMatcher::Fresh);
        assert_eq!(ctx.requester.name, "myapp");
        assert_eq!(ctx.requester.version, None);
        assert_eq!(ctx.dep_chain.len(), 3);
        assert!(ctx.dep_chain[0].is_root);
        assert_eq!(ctx.dep_chain[1].origin.as_deref(), Some("Fresh"));
    }

    // ── DevTtyPrompt — non-interactive returns Unsupported ────────

    #[test]
    fn dev_tty_prompt_non_interactive_returns_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let mut prompt =
            DevTtyPrompt::new(dir.path().join("triet.policy")).non_interactive(true);
        let err = prompt
            .prompt(&req("sys.io", "myapp", ResolutionOrigin::Fresh))
            .expect_err("non-interactive must fail-closed");
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn non_interactive_callback_factory_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let mut cb = non_interactive_callback(dir.path().join("triet.policy"));
        let err = cb
            .prompt(&req("sys.io", "myapp", ResolutionOrigin::Fresh))
            .expect_err("factory must build a non-interactive prompt");
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    // ── DevTtyPrompt::write_permanent ─────────────────────────────

    #[test]
    fn write_permanent_appends_to_empty_policy_file() {
        let dir = tempfile::tempdir().unwrap();
        let policy_path = dir.path().join("triet.policy");
        let prompt = DevTtyPrompt::new(&policy_path);
        let r = req("sys.io", "myapp", ResolutionOrigin::Fresh);
        prompt.write_permanent(&r, Decision::Plus1).unwrap();

        let loaded = PolicyRules::load(&policy_path).unwrap();
        assert_eq!(loaded.rules().len(), 1);
        assert_eq!(loaded.rules()[0].cap_path, "sys.io");
        assert_eq!(loaded.rules()[0].origin, OriginMatcher::Fresh);
        assert_eq!(loaded.rules()[0].decision, Decision::Plus1);
    }

    #[test]
    fn write_permanent_upserts_existing_rule() {
        let dir = tempfile::tempdir().unwrap();
        let policy_path = dir.path().join("triet.policy");
        let prompt = DevTtyPrompt::new(&policy_path);
        let r = req("sys.io", "myapp", ResolutionOrigin::Fresh);

        prompt.write_permanent(&r, Decision::Plus1).unwrap();
        prompt.write_permanent(&r, Decision::Minus1).unwrap();

        let loaded = PolicyRules::load(&policy_path).unwrap();
        assert_eq!(loaded.rules().len(), 1, "upsert must not duplicate");
        assert_eq!(loaded.rules()[0].decision, Decision::Minus1);
    }

    #[test]
    fn write_permanent_preserves_unrelated_rules() {
        let dir = tempfile::tempdir().unwrap();
        let policy_path = dir.path().join("triet.policy");

        // Seed with a rule for a different path.
        let initial = "format_version 1\nrule dev.disk fresh -1\n";
        std::fs::write(&policy_path, initial).unwrap();

        let prompt = DevTtyPrompt::new(&policy_path);
        let r = req("sys.io", "myapp", ResolutionOrigin::Fresh);
        prompt.write_permanent(&r, Decision::Plus1).unwrap();

        let loaded = PolicyRules::load(&policy_path).unwrap();
        assert_eq!(loaded.rules().len(), 2);
        // Sorted by cap_path → dev.disk first.
        assert_eq!(loaded.rules()[0].cap_path, "dev.disk");
        assert_eq!(loaded.rules()[1].cap_path, "sys.io");
    }
}
