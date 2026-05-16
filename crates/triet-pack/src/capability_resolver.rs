//! Capability resolver — ADR-0017 §4 runtime resolution (Step 6b of
//! [ADR-0018 §2] loader workflow).
//!
//! Where [`check_link_capabilities`](crate::check_link_capabilities)
//! (Step 6a) refuses unsafe link configurations up front, this module
//! decides what to do with cap paths the root manifest marked
//! `Defer` (`Trilean::Unknown`). The decision uses
//! [`PolicyRules`](crate::PolicyRules) — parsed `triet.policy` — plus
//! an in-memory cache keyed by `(cap_path, requester_pkg)`.
//!
//! # Algorithm (ADR-0017 §4)
//!
//! ```text
//! resolve(req) -> CachedDecision
//!   1. Cache lookup — hit → return with source=Cache (replay)
//!   2. Try PolicyRules.find(cap_path, origin):
//!      - Some(+1)     → Trit::Positive,  source=ConfigRule
//!      - Some(0)      → Trit::Zero,      source=AbstainFromRule
//!      - Some(-1)     → Trit::Negative,  source=ConfigRule
//!      - Some(prompt) → tty_available?
//!                       yes → goto Bước 3 (v0.6.10 stub — n/a here)
//!                       no  → Trit::Negative,
//!                             source=Error(NonTTYDefer)
//!      - None         → use effective_default()
//!                       (collapses absent to Trit::Negative —
//!                       fail-closed)
//!   3. Cache the decision (per ADR-0017 §5 monotonicity) and return.
//! ```
//!
//! # Monotonicity invariant (ADR-0017 §5)
//!
//! Once a `(cap_path, requester_pkg)` key is cached, the decision is
//! frozen for the resolver's lifetime — modifications to `PolicyRules`
//! after the first resolve do **not** change cached outcomes. Tests
//! pin this behaviour via the [`CapabilityResolver::resolve`] return
//! type carrying [`DecisionSource::Cache`] on replay.
//!
//! # What v0.6.9 ships vs. what v0.6.10 + v0.6.11 wire in
//!
//! - **Shipped here:** algorithm steps 1–2, cache, default fallback,
//!   `NonTTYDefer` fail-closed (rule said `prompt` but no TTY).
//! - **v0.6.10:** TTY detection (`isatty` + `/dev/tty` open per
//!   [ADR-0017 Addendum §B]) flips `tty_available`, branches into
//!   the prompt path, populates [`DecisionSource::InteractivePrompt`].
//! - **v0.6.11:** loader integration — iterates
//!   [`CapabilityLinkReport::deferrals`](crate::CapabilityLinkReport)
//!   and calls [`CapabilityResolver::resolve`] for each.
//!
//! [ADR-0018 §2]: ../../../docs/decisions/0018-capability-loader-semantics.md
//! [ADR-0017 Addendum §B]: ../../../docs/decisions/0017-trilean-policy-hook.md#addendum--parser-strictness--tty-source--abstain-errata

use std::collections::HashMap;

use miette::Diagnostic;
use thiserror::Error;
use triet_core::Trit;

use crate::policy::{Decision, OriginMatcher, PolicyRules};
use crate::resolver::ResolutionOrigin;

/// Input to the capability resolver — the full identity of "who's
/// asking for what" plus *how* the dep was selected by the upstream
/// resolver. ADR-0017 §2 freezes this shape so v0.6.10 callback (and
/// v0.7+ Triết-function hook) plug in additively.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyRequest {
    /// Dotted module path being defer-resolved.
    pub cap_path: String,
    /// Package name issuing the request. Cache key partition.
    pub requester_pkg: String,
    /// Transitive chain from root → requester. v0.6.9 carries it for
    /// shape parity with future hooks; resolution algorithm doesn't
    /// branch on it yet (TTY display in v0.6.10 + Triết callback in
    /// v0.7+ will read it).
    pub dep_chain: Vec<String>,
    /// Why the upstream resolver selected this dep. Combined with
    /// ADR-0015 Addendum and ADR-0017 §3, this is the rule-key
    /// dimension keyed by `lockfile` / `ifacepin` / `fresh` / wildcard.
    pub origin: ResolutionOrigin,
}

/// Outcome of a [`CapabilityResolver::resolve`] call. Carries the
/// Trit-valued decision plus the *why* so callers can:
///
/// - Surface a diagnostic on first resolution but stay silent on
///   replay ([`DecisionSource::Cache`]).
/// - Distinguish `Trit::Zero` from `Trit::Negative` for the
///   `AbstainFromRule` diagnostic (ADR-0017 §2 row 2).
/// - Emit `E2205.NonTTYDefer` when a `prompt` rule fell through to
///   fail-closed.
///
/// `outcome` is what the caller should *act on*: `Positive` grants,
/// `Zero` and `Negative` both deny (treat behaviourally the same —
/// only the diagnostic differs per ADR-0017 §2 + Addendum §C).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedDecision {
    /// The Trit-valued decision. `Positive` = grant; `Zero` /
    /// `Negative` = deny.
    pub outcome: Trit,
    /// How the resolver arrived at `outcome`. Drives diagnostic
    /// emission at the call site.
    pub source: DecisionSource,
}

/// How the resolver arrived at a [`CachedDecision`].
///
/// Replay hits (cache hit on a previously-resolved key) carry
/// [`DecisionSource::Cache`] so the caller knows the decision was
/// computed earlier — no fresh diagnostic needed. First-time
/// resolutions carry the source that originally produced the
/// outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecisionSource {
    /// Cache hit — replay of an earlier resolve. Caller emits no
    /// diagnostic; the original source already triggered one (if
    /// applicable) on first computation.
    Cache,
    /// Rule matched with `+1` or `-1`. Trit::Positive or
    /// Trit::Negative.
    ConfigRule,
    /// Rule matched with `0` (abstain). Behaviourally deny, but the
    /// caller surfaces a "policy abstained" info diagnostic to
    /// distinguish from explicit `-1` (ADR-0017 §2 + Addendum §C).
    AbstainFromRule,
    /// No rule matched; resolver used
    /// [`PolicyRules::effective_default`]. Absent default collapses
    /// to deny (fail-closed) per ADR-0017 §3.
    Default,
    /// Rule said `prompt`, TTY would be the next step (v0.6.10). At
    /// v0.6.9 the resolver always treats `tty_available = false`, so
    /// this variant is created only for completeness — actual
    /// resolutions hit [`DecisionSource::Error`] with
    /// [`ResolverError::NonTTYDefer`] instead.
    InteractivePrompt,
    /// Fail-closed deny due to a [`ResolverError`]. Caller surfaces
    /// the embedded diagnostic.
    Error(ResolverError),
}

/// Errors raised during runtime cap resolution. Both variants live
/// in `E2205` per [ADR-0017 §6]. The third + fourth runtime
/// sub-variants reserved at ADR-0017 (`NonTTYDefer`, `PromptCrash`)
/// land here; load-time `ConfigParse` / `RuleConflict` /
/// `UnknownOrigin` / `UnknownDecision` already shipped with
/// [`PolicyError`](crate::PolicyError) in v0.6.6.
///
/// [ADR-0017 §6]: ../../../docs/decisions/0017-trilean-policy-hook.md
#[derive(Clone, Debug, Diagnostic, Error, PartialEq, Eq)]
pub enum ResolverError {
    /// E2205.NonTTYDefer — rule said `prompt` but no TTY available.
    /// Fail-closed deny + diagnostic per ADR-0017 §6.
    #[error(
        "cap `{cap_path}` (requester `{requester_pkg}`): policy returned `prompt` but no \
         TTY available"
    )]
    #[diagnostic(
        code(triet::capability::E2205),
        help(
            "set an explicit rule (`+1`/`0`/`-1`) in triet.policy or run the binary with \
             an interactive terminal. ADR-0017 §6."
        )
    )]
    NonTTYDefer {
        /// Cap path that triggered the prompt rule.
        cap_path: String,
        /// Requester package whose claim hit the rule.
        requester_pkg: String,
    },

    /// E2205.PromptCrash — TTY I/O error during prompt. Placeholder
    /// at v0.6.9; raised by the prompt machinery in v0.6.10.
    ///
    /// Kept exhaustive so downstream matches don't grow stale
    /// silently when v0.6.10 wires the actual prompt path.
    #[error(
        "cap `{cap_path}`: TTY prompt I/O error: {os_error} — treating as Deny"
    )]
    #[diagnostic(
        code(triet::capability::E2205),
        help("retry from a working terminal; check terminal capabilities. ADR-0017 §6.")
    )]
    PromptCrash {
        /// Cap path whose prompt crashed.
        cap_path: String,
        /// Best-effort OS error message.
        os_error: String,
    },
}

/// Runtime capability resolver — owns a snapshot of
/// [`PolicyRules`] and a per-session decision cache.
///
/// Owning the rules avoids lifetime gymnastics when the resolver
/// outlives the parsed-file struct (`triet.policy` is read once at
/// loader start, then the rules object is conceptually owned by the
/// resolver for the rest of the process). Callers wanting to swap
/// rules mid-session must build a new resolver; ADR-0017 §5
/// monotonicity means in-flight cached decisions stay frozen anyway.
///
/// The resolver is **not** thread-safe at v0.6.9. v0.8 concurrency
/// will revisit (probably `Arc<RwLock<HashMap<…>>>` for the cache;
/// rules are immutable so they stay `Arc<PolicyRules>`).
#[derive(Clone, Debug)]
pub struct CapabilityResolver {
    rules: PolicyRules,
    cache: HashMap<(String, String), CachedDecision>,
    /// At v0.6.9 always `false` — the `prompt` decision always
    /// falls through to fail-closed [`ResolverError::NonTTYDefer`].
    /// v0.6.10 sets this from `isatty(stderr)` + `/dev/tty` open per
    /// [ADR-0017 Addendum §B] and branches into the prompt path.
    tty_available: bool,
}

impl CapabilityResolver {
    /// New resolver from a parsed policy. `tty_available` defaults
    /// `false` at v0.6.9 — any `prompt` rule fires fail-closed
    /// [`ResolverError::NonTTYDefer`]. Use
    /// [`Self::with_tty_available`] from v0.6.10 onward.
    #[must_use]
    pub fn new(rules: PolicyRules) -> Self {
        Self {
            rules,
            cache: HashMap::new(),
            tty_available: false,
        }
    }

    /// Override the `tty_available` flag. Intended for v0.6.10's TTY
    /// detection layer; tests at v0.6.9 leave it `false`. Builder-
    /// style return so it composes with `new(...)`.
    #[must_use]
    pub const fn with_tty_available(mut self, tty: bool) -> Self {
        self.tty_available = tty;
        self
    }

    /// Number of distinct `(cap_path, requester_pkg)` decisions
    /// cached this session. Exposed for testing the monotonicity
    /// invariant — call resolve(), inspect, resolve same key again,
    /// inspect: count must stay stable.
    #[must_use]
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }

    /// Resolve one [`PolicyRequest`].
    ///
    /// Returns a [`CachedDecision`] with a Trit-valued `outcome` and
    /// a [`DecisionSource`] describing the path the algorithm took.
    /// Idempotent on the same `(cap_path, requester_pkg)` key —
    /// subsequent calls return `source = Cache` with the same
    /// `outcome` (ADR-0017 §5 monotonicity).
    pub fn resolve(&mut self, req: &PolicyRequest) -> CachedDecision {
        let key = (req.cap_path.clone(), req.requester_pkg.clone());

        // ── Step 1 — Cache lookup ─────────────────────────────────
        if let Some(cached) = self.cache.get(&key) {
            return CachedDecision {
                outcome: cached.outcome,
                source: DecisionSource::Cache,
            };
        }

        // ── Step 2 — Rule lookup ──────────────────────────────────
        let origin = origin_to_matcher(req.origin);
        let decision = self.rules.find(&req.cap_path, origin);

        let fresh = match decision {
            Some(Decision::Plus1) => CachedDecision {
                outcome: Trit::Positive,
                source: DecisionSource::ConfigRule,
            },
            Some(Decision::Minus1) => CachedDecision {
                outcome: Trit::Negative,
                source: DecisionSource::ConfigRule,
            },
            Some(Decision::Zero) => CachedDecision {
                outcome: Trit::Zero,
                source: DecisionSource::AbstainFromRule,
            },
            Some(Decision::Prompt) => self.handle_prompt(req),
            None => {
                // ADR-0017 §3 — absent rule + absent default = fail-closed.
                let default = self.rules.effective_default();
                CachedDecision {
                    outcome: decision_to_trit_static(default),
                    source: DecisionSource::Default,
                }
            }
        };

        self.cache.insert(key, fresh.clone());
        fresh
    }

    /// Handle a `prompt` rule. v0.6.9: always fall through to
    /// fail-closed `NonTTYDefer` because `tty_available = false`.
    /// v0.6.10: branches into the TTY prompt path when
    /// `tty_available = true`.
    fn handle_prompt(&self, req: &PolicyRequest) -> CachedDecision {
        if self.tty_available {
            // Placeholder for v0.6.10 — current code path never
            // executes because the constructor pins `false`. Kept
            // explicit so the v0.6.10 patch is purely additive.
            CachedDecision {
                outcome: Trit::Zero,
                source: DecisionSource::InteractivePrompt,
            }
        } else {
            CachedDecision {
                outcome: Trit::Negative,
                source: DecisionSource::Error(ResolverError::NonTTYDefer {
                    cap_path: req.cap_path.clone(),
                    requester_pkg: req.requester_pkg.clone(),
                }),
            }
        }
    }
}

/// Translate `ResolutionOrigin` (which is "where did this dep come
/// from") into `OriginMatcher` (which is "which `triet.policy` rule
/// keys match"). The two enums are deliberately distinct types —
/// `OriginMatcher` carries the wildcard `Any` while `ResolutionOrigin`
/// is closed-set — but the three exact variants line up 1:1.
const fn origin_to_matcher(origin: ResolutionOrigin) -> OriginMatcher {
    match origin {
        ResolutionOrigin::Lockfile => OriginMatcher::Lockfile,
        ResolutionOrigin::IfacePin => OriginMatcher::IfacePin,
        ResolutionOrigin::Fresh => OriginMatcher::Fresh,
    }
}

/// Map a `Decision` (which is the user-source token-style enum) to a
/// `Trit` for use as a resolution outcome. `Decision::Prompt` is
/// **invalid** here — `effective_default` filters it out at parse
/// time per ADR-0017 §3 (default decisions must be static), and rule
/// dispatch handles `Prompt` separately.
const fn decision_to_trit_static(d: Decision) -> Trit {
    match d {
        Decision::Plus1 => Trit::Positive,
        Decision::Zero => Trit::Zero,
        Decision::Minus1 => Trit::Negative,
        // ADR-0017 §3: `default prompt` is rejected at parse time.
        // PolicyRules::effective_default() only ever returns
        // Plus1/Zero/Minus1 (None → Minus1). Reaching this arm means
        // somebody constructed PolicyRules in-memory with a Prompt
        // default — invalid state per the parser's guarantees.
        // Conservative fallback: treat as deny.
        Decision::Prompt => Trit::Negative,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(cap_path: &str, requester_pkg: &str, origin: ResolutionOrigin) -> PolicyRequest {
        PolicyRequest {
            cap_path: cap_path.into(),
            requester_pkg: requester_pkg.into(),
            dep_chain: vec![],
            origin,
        }
    }

    fn rules(text: &str) -> PolicyRules {
        PolicyRules::parse(text).expect("test fixture should parse")
    }

    // ── Default fallback (no rules) ────────────────────────────────

    #[test]
    fn empty_rules_fresh_origin_defaults_to_deny() {
        let mut r = CapabilityResolver::new(PolicyRules::empty());
        let d = r.resolve(&req("sys.io", "myapp", ResolutionOrigin::Fresh));
        assert_eq!(d.outcome, Trit::Negative);
        assert!(matches!(d.source, DecisionSource::Default));
    }

    #[test]
    fn explicit_default_grant_overrides_implicit_deny() {
        let rules = rules("format_version 1\ndefault +1\n");
        let mut r = CapabilityResolver::new(rules);
        let d = r.resolve(&req("sys.io", "myapp", ResolutionOrigin::Fresh));
        assert_eq!(d.outcome, Trit::Positive);
        assert!(matches!(d.source, DecisionSource::Default));
    }

    // ── Direct rule matches ────────────────────────────────────────

    #[test]
    fn exact_rule_plus_one_grants() {
        let rules = rules(
            "format_version 1\n\
             rule sys.io fresh +1\n",
        );
        let mut r = CapabilityResolver::new(rules);
        let d = r.resolve(&req("sys.io", "myapp", ResolutionOrigin::Fresh));
        assert_eq!(d.outcome, Trit::Positive);
        assert!(matches!(d.source, DecisionSource::ConfigRule));
    }

    #[test]
    fn exact_rule_minus_one_denies() {
        let rules = rules(
            "format_version 1\n\
             rule dev.disk fresh -1\n",
        );
        let mut r = CapabilityResolver::new(rules);
        let d = r.resolve(&req("dev.disk", "myapp", ResolutionOrigin::Fresh));
        assert_eq!(d.outcome, Trit::Negative);
        assert!(matches!(d.source, DecisionSource::ConfigRule));
    }

    #[test]
    fn exact_rule_zero_abstains_with_distinct_source() {
        // Rule says `0` — outcome is Trit::Zero (behaviourally deny)
        // but the source is AbstainFromRule so the caller surfaces a
        // distinct "policy abstained" diagnostic per ADR-0017 §2.
        let rules = rules(
            "format_version 1\n\
             rule sys.io fresh 0\n",
        );
        let mut r = CapabilityResolver::new(rules);
        let d = r.resolve(&req("sys.io", "myapp", ResolutionOrigin::Fresh));
        assert_eq!(d.outcome, Trit::Zero);
        assert!(matches!(d.source, DecisionSource::AbstainFromRule));
    }

    // ── Prompt → NonTTYDefer (v0.6.9: no TTY) ─────────────────────

    #[test]
    fn prompt_rule_without_tty_fails_closed() {
        let rules = rules(
            "format_version 1\n\
             rule sys.net.dns fresh prompt\n",
        );
        let mut r = CapabilityResolver::new(rules);
        let d = r.resolve(&req("sys.net.dns", "myapp", ResolutionOrigin::Fresh));
        assert_eq!(d.outcome, Trit::Negative);
        match &d.source {
            DecisionSource::Error(ResolverError::NonTTYDefer {
                cap_path,
                requester_pkg,
            }) => {
                assert_eq!(cap_path, "sys.net.dns");
                assert_eq!(requester_pkg, "myapp");
            }
            other => panic!("expected NonTTYDefer, got {other:?}"),
        }
    }

    #[test]
    fn tty_available_flag_branches_into_prompt_path() {
        // v0.6.10 will fill out the prompt path; here we just verify
        // the branch is reachable so the v0.6.10 patch can be purely
        // additive.
        let rules = rules(
            "format_version 1\n\
             rule sys.io fresh prompt\n",
        );
        let mut r = CapabilityResolver::new(rules).with_tty_available(true);
        let d = r.resolve(&req("sys.io", "myapp", ResolutionOrigin::Fresh));
        // Placeholder branch returns Trit::Zero + InteractivePrompt.
        assert_eq!(d.outcome, Trit::Zero);
        assert!(matches!(d.source, DecisionSource::InteractivePrompt));
    }

    // ── Origin dispatch ────────────────────────────────────────────

    #[test]
    fn lockfile_origin_uses_lockfile_rule() {
        let rules = rules(
            "format_version 1\n\
             rule sys.io lockfile +1\n\
             rule sys.io fresh -1\n",
        );
        let mut r = CapabilityResolver::new(rules);
        let d = r.resolve(&req("sys.io", "myapp", ResolutionOrigin::Lockfile));
        assert_eq!(d.outcome, Trit::Positive);
    }

    #[test]
    fn ifacepin_origin_uses_ifacepin_rule() {
        let rules = rules(
            "format_version 1\n\
             rule sys.io lockfile +1\n\
             rule sys.io ifacepin -1\n\
             rule sys.io fresh 0\n",
        );
        let mut r = CapabilityResolver::new(rules);
        let d = r.resolve(&req("sys.io", "myapp", ResolutionOrigin::IfacePin));
        assert_eq!(d.outcome, Trit::Negative);
    }

    #[test]
    fn wildcard_rule_catches_fresh_when_no_exact() {
        let rules = rules(
            "format_version 1\n\
             rule sys.io * +1\n",
        );
        let mut r = CapabilityResolver::new(rules);
        let d = r.resolve(&req("sys.io", "myapp", ResolutionOrigin::Fresh));
        assert_eq!(d.outcome, Trit::Positive);
        assert!(matches!(d.source, DecisionSource::ConfigRule));
    }

    // ── Cache + monotonicity ───────────────────────────────────────

    #[test]
    fn second_resolve_same_key_replays_from_cache() {
        let rules = rules(
            "format_version 1\n\
             rule sys.io fresh +1\n",
        );
        let mut r = CapabilityResolver::new(rules);
        let first = r.resolve(&req("sys.io", "myapp", ResolutionOrigin::Fresh));
        assert!(matches!(first.source, DecisionSource::ConfigRule));

        let second = r.resolve(&req("sys.io", "myapp", ResolutionOrigin::Fresh));
        assert_eq!(second.outcome, first.outcome);
        assert!(matches!(second.source, DecisionSource::Cache));
    }

    #[test]
    fn cache_count_stable_under_replay() {
        let rules = rules(
            "format_version 1\n\
             rule sys.io fresh +1\n",
        );
        let mut r = CapabilityResolver::new(rules);
        let req_a = req("sys.io", "myapp", ResolutionOrigin::Fresh);
        let _ = r.resolve(&req_a);
        let _ = r.resolve(&req_a);
        let _ = r.resolve(&req_a);
        assert_eq!(r.cache_size(), 1);
    }

    #[test]
    fn distinct_requester_pkgs_get_separate_entries() {
        let rules = rules(
            "format_version 1\n\
             rule sys.io fresh +1\n",
        );
        let mut r = CapabilityResolver::new(rules);
        let _ = r.resolve(&req("sys.io", "app_a", ResolutionOrigin::Fresh));
        let _ = r.resolve(&req("sys.io", "app_b", ResolutionOrigin::Fresh));
        assert_eq!(r.cache_size(), 2);
    }

    #[test]
    fn distinct_cap_paths_get_separate_entries() {
        let rules = rules("format_version 1\ndefault -1\n");
        let mut r = CapabilityResolver::new(rules);
        let _ = r.resolve(&req("sys.io", "myapp", ResolutionOrigin::Fresh));
        let _ = r.resolve(&req("dev.disk", "myapp", ResolutionOrigin::Fresh));
        assert_eq!(r.cache_size(), 2);
    }

    // ── Error variants ─────────────────────────────────────────────

    #[test]
    fn non_tty_defer_carries_diagnostic_context() {
        let rules = rules(
            "format_version 1\n\
             rule sys.net.dns fresh prompt\n",
        );
        let mut r = CapabilityResolver::new(rules);
        let d = r.resolve(&req("sys.net.dns", "myapp", ResolutionOrigin::Fresh));
        // miette `Display` impl carries the cap_path + requester_pkg.
        match &d.source {
            DecisionSource::Error(err) => {
                let msg = err.to_string();
                assert!(msg.contains("sys.net.dns"), "msg: {msg}");
                assert!(msg.contains("myapp"), "msg: {msg}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn prompt_crash_variant_is_constructible() {
        // Placeholder check — v0.6.10 wires the actual prompt path.
        // We construct PromptCrash directly to ensure exhaustive
        // pattern matching downstream stays honest.
        let err = ResolverError::PromptCrash {
            cap_path: "sys.io".into(),
            os_error: "test".into(),
        };
        assert!(err.to_string().contains("sys.io"));
    }

    // ── Dep chain (shape only at v0.6.9) ──────────────────────────

    #[test]
    fn dep_chain_is_carried_but_does_not_affect_resolution() {
        let rules = rules(
            "format_version 1\n\
             rule sys.io fresh +1\n",
        );
        let mut r = CapabilityResolver::new(rules);
        let req = PolicyRequest {
            cap_path: "sys.io".into(),
            requester_pkg: "myapp".into(),
            dep_chain: vec!["myapp".into(), "libdns".into(), "libtls".into()],
            origin: ResolutionOrigin::Fresh,
        };
        let d = r.resolve(&req);
        // v0.6.9 algorithm doesn't branch on dep_chain — outcome
        // equals what an empty chain would give.
        assert_eq!(d.outcome, Trit::Positive);
    }
}
