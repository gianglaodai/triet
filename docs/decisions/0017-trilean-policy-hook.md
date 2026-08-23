# ADR 0017 — Trilean policy hook protocol (`dao.policy` + TTY fallback)

**Status:** Decision. Applies to v0.6 Capability System runtime resolution. Depends on [ADR-0016 §3](0016-capability-type-system.md) (Defer slot in CapabilityLevel) and [ADR-0016 §8](0016-capability-type-system.md) (ResolutionOrigin dispatch). Populates the `E2205` reserved slot in [ADR-0016 §6](0016-capability-type-system.md). No `abi_version` bump, no changes to `.triv` wire format, no changes to IR shape.

**Issue:** ADR-0016 locks 4 CapabilityLevel states: `Grant (+1)`, `Ambient (0)`, `Deny (-1)`, and **`Defer (Trilean::Unknown)`**. The first three states resolve completely at compile + link time. The fourth state — `Defer` — defers the decision to runtime, following the promise in [VISION §3.5.2](../../VISION.md): *"capabilities can be `Trilean::Unknown` → resolved at runtime by user/policy"*.

ADR-0016 "For ADR-0017" lists 4 critical questions:

1. **Where does the hook live?** Declarative config / Trilean function callback / interactive TYY?
2. **Hook invocation protocol** — input record + output type?
3. **Cache scope** + capability monotonicity invariant?
4. **Failure mode** when policy crashes / is unreachable / config is missing?

[ADR-0015 Addendum](0015-package-store-layout.md#addendum--v05xreview-pre-v06-audit) hinted at specific mapping: *"only `Lockfile` is auto-trusted, `IfacePin` requires admin grant, `Fresh` deps must prompt the user"*. ADR-0017 completes this mechanism.

## Decision

### 1. Approach — Hybrid: `dao.policy` rules first, TTY prompt fallback, headless = fail-closed

Runtime hook resolution proceeds through 3 steps, in order:

```
┌─────────────────────────────────────────────────────────────┐
│  Step 1: Lookup cache                                       │
│    Hit  → return cached Trit (O(1))                         │
│    Miss → step 2                                            │
├─────────────────────────────────────────────────────────────┤
│  Step 2: Match rule in dao.policy                          │
│    Match → resolve via decision token, cache, and return    │
│    No match → step 3                                        │
├─────────────────────────────────────────────────────────────┤
│  Step 3: Fallback                                           │
│    TTY available     → interactive prompt → cache → return  │
│    Headless (no TTY) → E2205.NonTTYDefer → Deny + cache     │
│    Config absent     → default rule (`default -1` implicit) │
└─────────────────────────────────────────────────────────────┘
```

Three alternatives were rejected:

| Rejected | Reason |
|---|---|
| **Static config only** (rules without prompt) | Loses the ADR-0015 Addendum hint *"Fresh deps must prompt user"* — without TTY fallback, Fresh deps default to Deny, breaking all new dependency builds. |
| **Trilean function callback** (manifest `policy_hook: usr.myapp.policy.decide`) | Bootstrap cycle: a hook function in `usr.*` requires cap context → triggers hook → ... Must be locked back to ADR-0016 §5 enforcement rules. Hostile policy code = a new attack surface at the loader stage. Defer this additively post-v0.7. |
| **Interactive prompt only** (zero config) | CI/headless environments would default to Deny all Defer requests → breaking automation. Does not match the webapp developer mental model (Spring/nginx use config first, prompt second). |

Approach 4 (hybrid) **maps 1-1** with the ADR-0015 Addendum hint via rule keys with an `origin` dimension:

```
rule * lockfile +1        # auto-trust lockfile-pinned deps
rule * ifacepin prompt    # iface hash pin = admin confirm
rule * fresh    prompt    # newly added dep = prompt user
```

### 2. PolicyRequest + PolicyDecision — frozen shape at v0.6

**Hook input** (the record passed into the resolution machinery; although v0.6 does not expose this to user code, the shape is frozen so that v0.7+ Approach 2 callbacks can be built upon):

```text
PolicyRequest {
    cap_path:       String,            // AbsolutePath module, e.g. "sys.io"
    requester_pkg:  String,            // "myapp@0.1.0" — pkg requesting access
    dep_chain:      List<String>,      // ["myapp", "libdns", "libtls"] — transitive
    origin:         ResolutionOrigin,  // Lockfile | IfacePin | Fresh
}
```

**Hook output** — `PolicyDecision = Result<Trit, PolicyError>`:

| Outcome | Runtime Consequence | Diagnostic |
|---|---|---|
| `Ok(Trit::Positive)` | Grant — cache, allow current + all future calls (cap_path, requester_pkg) | None |
| `Ok(Trit::Zero)` | Abstain — cache as Deny, allow re-evaluation next session | Info: "policy abstained" |
| `Ok(Trit::Negative)` | Deny — cache, refuse current + all future calls | None |
| `Err(PolicyError)` | Fail-closed Deny — cache with reason | E2205.<sub> |

3 Trit + 1 Err = 4 outcomes, preserving the ternary identity at runtime. `Trit::Zero` (Abtain) vs `Trit::Negative` (Deny) differs **only in diagnostics**: Abstain = "policy cannot decide"; Deny = "policy actively refuses". Audit logs must distinguish these — critical for post-incident review.

### 3. `dao.policy` file format (hand-rolled, canonical sort)

Filename: `dao.policy` at the project root (parallel to `dao.lock`). Hand-rolled line format, mirroring the precedent in [ADR-0015 §6](0015-package-store-layout.md) — no serde dependency, diff-friendly.

```text
format_version 1
rule <cap_path>      <origin>      <decision>
rule <cap_path>      <origin>      <decision>
...
default <decision>
```

**Field rules:**

- `cap_path`: exact `AbsolutePath` (e.g. `sys.io`, `dev.disk`, `sys.net.dns`). **NO globbing** in v0.6 — exact match only. Blanket policies use the `default` line.
- `origin`: `lockfile` | `ifacepin` | `fresh` | `*` (wildcard match any origin). This is the **sole location** where a wildcard is permitted — since `ResolutionOrigin` is a closed enum, `*` does not violate "Explicit > implicit".
- `decision`: `+1` | `0` | `-1` | `prompt`. Four tokens, no aliases (e.g. `grant` is not allowed — keep the parser dead simple).
- `default <decision>`: zero or one line. If absent → implicit `default -1`. The `decision` in `default` cannot be `prompt` (default must be static).

**Canonical encoding** (for hash stability if future versions pin the policy file):

- Sort by `(cap_path ASC, origin ASC)`. `origin` ordering: `lockfile < ifacepin < fresh < *`.
- Whitespace between fields: 1+ spaces or tabs; ignored by the parser, normalized by the writer.
- Comments: lines starting with `#` are ignored. NO inline comments (the parser drops a line if any `#` is found outside a string).
- Encoding: UTF-8, LF line endings (CRLF rejected — matches `dao.lock`).
- Duplicate `(cap_path, origin)` tuple → **E2205.RuleConflict** (refuse-to-load). NO last-wins / merge semantics.

**Example:**

```text
# dao.policy v1
format_version 1

# Trusted: lockfile-pinned deps get auto-grant for std-adjacent paths
rule sys.io       lockfile +1
rule sys.io       ifacepin prompt
rule sys.io       fresh    prompt

# Network DNS: blanket grant (low-risk read-only)
rule sys.net.dns  *        +1

# Disk raw access: blanket deny — must explicitly override per-path
rule dev.disk     *        -1

# Catch-all
default -1
```

### 4. Resolution algorithm

Pseudo-code (sequential, single-threaded — v0.6 has no concurrency):

```
function resolve(req: PolicyRequest) -> Trit {
    // Step 1: cache lookup
    let key = (req.cap_path, req.requester_pkg)
    if cache.contains(key) {
        return cache[key].outcome
    }

    // Step 2: try dao.policy rules
    if policy_file_exists {
        let rules = load_and_parse(dao.policy)  // memoized after first call
        let matched = rules.find_exact(req.cap_path, req.origin)
                       or rules.find_exact(req.cap_path, Wildcard)
        if matched != null {
            let outcome = match matched.decision {
                +1     -> +1
                0      -> 0
                -1     -> -1
                prompt -> goto step 3 (force prompt)
            }
            cache[key] = CachedDecision { outcome, source: ConfigRule }
            return outcome
        }
        // No rule matched: use `default`
        let default_decision = rules.default or -1
        cache[key] = CachedDecision { outcome: default_decision, source: Default }
        return default_decision
    }

    // Step 3: fallback (no policy file OR rule said `prompt`)
    if tty_available {
        let user_choice = prompt_user(req)
        cache[key] = CachedDecision { outcome: user_choice, source: InteractivePrompt }
        return user_choice
    }
    // Headless + Defer reached → fail-closed
    emit_diagnostic(E2205.NonTTYDefer, req)
    cache[key] = CachedDecision { outcome: -1, source: Error(NonTTYDefer) }
    return -1
}
```

**Rule match precedence** (simplified for v0.6):

1. A rule with an **exact match** origin (lockfile/ifacepin/fresh) wins over `*`.
2. Same precedence level → impossible due to duplicate (path, origin) $\rightarrow$ E2E205.RuleConflict.

**TTY prompt UX** (specified in v0.6, detailed implementation in ADR-0018):

```text
[triet] Capability decision required
  Package:        myapp@0.1.0
  Requesting:     sys.net.dns
  Dep chain:      myapp → libdns@1.2.3 → libtls@0.4.1
  Origin:         Fresh (newly resolved, not in dao.lock)

  [g] grant once (this session)
  [d] deny once  (this/session)
  [G] grant permanent (write rule to dao.policy)
  [D] deny permanent  (write rule to dao.policy)
  [?] explain

  choice >
```

`G`/`D` appends the rule to `dao.policy` before caching and returning. `g`/`d` only caches for the current session. Specific implementation is in ADR-0018.

### 5. Cache scope + capability monotonicity invariant

**Cache key:** `(cap_path: String, requester_pkg: String)`.

`origin` is not included in the key because:
- `origin` is decided by the resolver before reaching the hook — fixed per-session.
- The same (path, pkg) within the same session implies the same origin $\rightarrow$ including origin is redundant.

**Lifetime:** Process lifetime. Cache is discarded upon process exit.

**Monotonicity invariant** (locked from [ADR-0016 "Rejected Alternatives"](0016-capability-type-system.md)):

> *Once cached, the decision is frozen for the session. Capabilities do not support hot-reloading.*

Consequences:
- Re-evaluation is triggered ONLY when a (path, pkg) entry does not exist.
- Modifying `dao.policy` mid-session $\rightarrow$ does NOT affect already-cached decisions. The next process start will re-read the file.
- User selects `G`/`D` at the prompt $\rightarrow$ file update + cache update are atomic. The next process start will see the new rule from Step 2.

**Hot path optimization:** Capability checks fire on every cross-namespace call. Cache lookup is O(1) (HashMap by `(String, String)` key). Hook execution runs only once per unique key per session.

### 6. E2205 sub-variants — finalize

| Code | Variant | Stage | Runtime Outcome |
|---|---|---|---|
| `E2205.ConfigParse` | `dao.policy` syntax invalid | Load-time | Refuse to load entire binary, abort |
| `E2205.RuleConflict` | Duplicate `(path, origin)` in rules | Load-time | Refuse to load entire binary, abort |
| `E2205.UnknownOrigin` | `origin` field $\notin$ {lockfile, ifacepin, fresh, \*} | Load-time | Refuse to load entire binary, abort |
| `E2205.UnknownDecision` | `decision` field $\notin$ {+1, 0, -1, prompt} | Load-time | Refuse to load entire binary, abort |
| `E2205.NonTTYDefer` | Defer reached + no rule match + headless | First-call | Fail-closed Deny + cached + diagnostic |
| `E22/05.PromptCrash` | TTY closed mid-prompt / I/O error | First-call | Fail-closed Deny + cached + diagnostic |

Load-time errors (ConfigParse/RuleConflict/UnknownOrigin/UnknownDecision): refuse to load the **entire binary**. Reason: `dao.policy` corruption means no Defer resolution can be trusted $\rightarrow$ it is safer to abort than to perform a partial run.

First-call errors (NonTTYDefer/PromptCrash): per-key Deny + diagnostic. The process continues — as this only affects the unresolved capability; other capabilities may have been granted in Step 2.

### 7. Headless vs TTY detection

The loader checks `isatty(stderr)` upon the first Defer reached. TTY availability is cached for the remainder of the session — NO re-checking (avoids races with external `stty` changes).

CI environment variables (`CI=true`, `GITHUB_ACTIONS=true`, ...) are **not** parsed — only `isatty` is trusted. Reason: env vars are unreliable cross-platform; `isatty` is the POSIX standard.

The `--non-interactive` CLI flag (future, deferred to ADR-0018) will force headless mode regardless of TTY availability — useful for attended scripts.

### 8. v0.6 known limits

ADR-0017 intentionally does NOT finalize the following points, leaving them for later phases:

- **Timeout enforcement:** Synchronous prompt, no timeout. A hostile prompt (e.g., a malicious `stty` consuming input) could hang the loader. Defer to v0.8 actor model.
- **Trilean function callback** (Approach 2 was rejected in §1): defer additively post-v0.7. Adding a new rule type (`rule X * call usr.myapp.policy.decide`) will extend `dao.policy` v=1 $\rightarrow$ v=2 with an additive field.
- **Cross-process policy daemon:** Policy file is local-only. Distributed policy (system-wide cap server) deferred to v1.0+.
- **Persistent cache across sessions:** No persistence — process exit = cache gone. Users persist decisions by choosing `G`/`D` (writing a rule to the file).
- **Per-thread cache:** v0.6 uses a single-threaded VM. When the v0.8 actor model arrives $\rightarrow$ cache thread-safety will be finalized in the concurrency ADR.
- **Globbing in `cap_path`:** Not allowed. The `default` line covers blanket cases. Globbing is deferred (probably never — violates "Explicit > implicit").

## Consequences

### For ADR-0016 — populating the Defer slot

ADR-0016 §3 sets one of the four CapabilityLevel states as `Defer (Trilean::Unknown)`. ADR-0017 provides the resolution machinery $\rightarrow$ `Defer` is no longer a leaf-pending state; runtime is guaranteed to terminate with a final `Trit` (or an explicit fail-closed Deny + diagnostic).

### For ADR-0016 §6 — E2205 fully populated

`E2205` was reserved in ADR-0016 §6 with the note *"reserved for ADR-0017"*. ADR-0017 finalizes 6 sub-variants. ADR-0016 does not need re-issuing — sub-variants are extensions under the existing slot.

### For ADR-0018 (loader semantics) — TBD

ADR-0018 must finalize:
- The specific loader stage where `resolve()` fires (link-time pre-cache vs. lazy first-call).
- Detailed TTY prompt UX implementation (terminal escape sequences, color, multi-line rendering).
- The `--non-interactive` CLI flag specification.
- The `dao.policy` reader implementation (line tokenizer, error span reporting for `miette`).
- Manifest source syntax for the `requires:` block (ADR-0016 §1 uses pseudo-syntax).

ADR-0017 only commits: resolution **occurs** with the contract defined in §4 algorithm; lifecycle/UX details are deferred to ADR-0018.

### For v0.5 hash scheme

`dao.policy` **does not participate** in the `iface_hash` or `impl_hash` of the package. Policy is a deployment-environment concern, not a package-content concern. Two users running the same `.khi` with different `dao.policy` files $\rightarrow$ same hash, different runtime behavior — as per spec.

### For ABI metadata ([ADR-0011](0011-abi-metadata-format.md))

Unchanged. The `caps section` only encodes level `Defer (0x03)`; the runtime resolution machinery lives in the loader, not in the pack metadata.

### For IR ([ADR-0007](0007-ir-design.md)) / `.triv` wire format

Unchanged. Capability checks fire at the cross-module call dispatch site — the IR preserves the `AbsolutePath` ([ADR-0007 §6.7](0007-ir-design.md)). Cache lookup is a Rust-side data structure within the runtime, not a new IR opcode.

### For v0.7 self-hosting

The Trilean-rewritten compiler must honor `dao.policy` parsing semantics + the resolution algorithm. Test contract: the bootstrap chain output must be byte-identical to the Rust implementation for a `dao.policy` round-trip.

### For v0.8 concurrency

Cache thread-safety is an open question until v0.8. A hint is provided: `Arc<RwLock<HashMap<(String, String), CachedDecision>>>` on the Rust side so that actor messages can share an immutable view. ADR-0017 does NOT pre-commit the shape — waiting for the v0.8 actor ADR.

### For v0.9 JIT / v2.0 AOT

JIT lifts functions across capability boundaries $\rightarrow$ check at lift-time (deferred to ADR-0018). Cached decisions remain authoritative — lifting does not re-evaluate.

AOT v2.0 baked-binary: cache is initialized empty at every process start. `dao.policy` is loaded the same way — not AOT-baked (deployment-specific).

## Rejected Alternatives

- **Trilean function callback** (Approach 2 from proposal) — deferred post-v0.7 additive. Bootstrap risk + sandbox concerns + v0.6 VM hot-path performance.
- **Globbing in `cap_path`** — violates "Explicit > implicit". The `default` line is sufficient for blanket cases.
- **Last-wins / merge for duplicate rules** — refuse rather than guess. Duplicates = E2205.RuleConflict.
- **Inline comments** in `dao.policy` — `#` is only allowed at the start of a line. Mirrors `dao.lock`.
- **CRLF line endings** — LF only.
- **TOML / YAML / JSON syntax** — hand-rolled to mirror the `dao.lock` precedent ([ADR-0015 §6](0015-package-store-layout.md)). No serde dependency.
- **Timeout enforcement** in v0.6 — hostile prompts could hang the process. Deferred to v0.8.
- **Cross-process policy daemon** — local file only. Distributed policy deferred to v1.0+.
- **Persistent session cache** — process exit = cache gone. Decisions persist via the user choosing `G`/`D` (writing to the file).
- **Env-var-based headless detection** (`CI=true` etc.) — only `isatty(stderr)`. Env vars are unreliable.
- **Auto-write `dao.policy` on Deny** — only on explicit user `G`/`D`. Avoid silent grant accumulation.
- **Re-evaluation when config changes mid-session** — monotonicity invariant. Restarting the process = a new chance.

## Prior art

- **[nginx `location` rules](https://nginx.org/en/docs/http/ngx_http_core_module.html#location)** — declarative rule matching, ordered fallthrough. Trilean differs: canonical sort instead of source order (diff-friendly).
- **[Android runtime permissions](https://developer.android.com/training/permissions/requesting)** — manifest pre-declaration + OS prompts at runtime if not granted. This is the closest mental model to Approach 4. Difference: Android prompts are OS-level; Trilean prompts are loader-level (per-process).
- **[`sudo` / `polkit`](https://www.freedesktop.org/wiki/Software/polkit/)** — rule-based + interactive escalation. Polkit's `.rules` file is a JavaScript callback — Trilean rejects this due to code execution risks.
- **[OAuth consent screen](https://datatracker.ietf.org/doc/html/rfc6749#section-4.1.1)** — interactive grant flow with scoped tokens. Inspires the per-(path, pkg) cache shape.
- **[Spring Security `WebSecurityConfigurerAdapter`](https://docs.spring.io/spring-security/site/docs/current/api/org/springframework/security/config/annotation/web/WebSecurityConfigurerAdapter.html)** — code-driven policy. Deferred (Approach 2) post-v0.7.
- **[E language vat](http://www.erights.org/elib/distrib/vat.html)** — defer-to-vat for cross-vat capability resolution. Inspires the `Trilean::Unknown` defer pattern (acknowledged in ADR-0016 §3).

**Anti-prior-art:**

- **Java SecurityManager** (deprecated JDK 17) — code-based, brittle stack inspection. Trilean avoids this via declarative + interactive mechanisms.
- **Polkit JS rules** — code execution in a privileged context; history of CVEs. Trilean avoids this via data-only `dao.php`.
- **POSIX setuid + `cap_set_file`** — runtime capabilities with a history of confused-deputy CVEs. Trilean avoids this via compile-time + link-time + load-time enforcement; runtime hooks are only for explicit `Defer`.

## References

- [VISION §3.5 — OS-Native Capability Namespaces](../../VISION.md)
- [VISION §5 — The Essence of Trilean (Trit-level + Łukasiewicz capability)](../../VISION.md)
- [VISION §6 — Refuse over guess, Explicit > implicit](../../VISION.md)
- [SPEC §1.5.2 — Trilean type (`Unknown`)](../../SPEC.md)
- [ADR-0011 §5 — `caps section` ABI metadata](0011-abi-metadata-format.md)
- [ADR-0015 §6 — `dao.lock` hand-rolled format precedent](0015-package-store-layout.md)
- [ADR-0015 Addendum — ResolutionOrigin 3-state, dispatch hint](0015-package-store-layout.md#addendum--v05xreview-pre-v06-audit)
- [ADR-0016 §3 — Defer slot in CapabilityLevel](0016-capability-type-system.md)
- [ADR-0016 §6 — E22XX namespace, E2205 reserved](0016-capability-type-system.md)
- [ADR-0016 §8 — ResolutionOrigin dispatch slot](0016-capability-type-system.md)
- ADR-0018 — Capability loader semantics (TBD, v0.6.3)
- [ROADMAP §v0.6 — Capability System](../../ROADMAP.md)
- [ROADMAP §v0.8 — Concurrency Model](../../ROADMAP.md) (future: timeout + thread-safe cache)

---

## Addendum — Parser strictness + TTY source + Abstain errata

Audit window post-decision, mirroring the precedent in [ADR-0015 Addendum](0015-package-store-layout.md#addendum--v05xreview-pre-v06-audit). This does not reopen the original decision; it addresses 3 blind spots flagged by the author before v0.6.3 (ADR-0018) begins implementing the loader. All 3 are *clarifications + errata*, not changes to the locked semantics.

### §A — Parser whitelist rules (strengthen §3)

§3 listed CRLF rejection + duplicate path rejection, but **lacked** a clear specification for unusual input shapes. The principle for addressing blind spots: **the parser is extremely simple, whitelist-only**. Any ambiguity = `E2205.ConfigParse` $\rightarrow$ refuse to load. Refuse-over-guess strict mode ([VISION §6](../../VISION.md)).

| Input shape | Behavior |
|---|---|
| Empty file (0 bytes) | `E2205.ConfigParse` — "missing format_version" |
| BOM (`U/FEFF` at byte 0) | `E2205.ConfigParse` — "BOM not allowed" |
| Missing `format_version 1` as the first non-comment line | `E2205.ConfigParse` |
| Duplicate `format_version` line | `E2205.ConfigParse` |
| Unicode whitespace outside identifiers (U+00A0 NBSP, U+2028 LS, U+2029 PS, U+200B ZWSP, ...) | `E2205.ConfigParse` |
| Mixed tabs/spaces between fields | Accept (normalized by the writer) |
| Trailing whitespace at EOL | Accept (ignored) |
| Blank line (zero or whitespace-only) | Accept (no-op) |
| Comment line (`#` prefix, optionally leading whitespace) | Accept (ignored) |
| Inline comment (`#` mid-line) | `E2205.ConfigParse` — as listed in §3 |
| CRLF | `E2205.ConfigParse` — as listed in §3 |
| Line > 4096 bytes | `E2205.ConfigParse` — DoS prevention |
| File > 1 MiB | `E2205.ConfigParse` — DoS prevention |
| Identifier (`cap_path` component) contains Unicode | Accept if it passes XID Start/Continue ([SPEC §1.3](../../SPEC.md)); reject otherwise $\rightarrow$ `E2205.ConfigParse` |
| Any shape not matching the grammar | `E2205.ConfigParse` |

**Separating rule whitespace vs identifier:** The byte structure (separators, line endings) must be ASCII (`0x09` tab, `0x20` space, `0x0A` LF only); identifier content (`cap_path` components) may contain Unicode per XID rules — because `sys.tính_giá_trị` is a legal `AbsolutePath` per [SPEC §1.3](../../SPEC.md).

**Reason for "stupid parser":** The policy file controls the security boundary. A "smart" parser with recovery/fuzzy matching leads to silent semantic drift = false grants. Hand-rolled parsers lack the fuzzer coverage of `serde`/`TOML` — they are only safe if the grammar is small + rejects anything weird.

### §B — TTY input source: `/dev/tty`, not stdin (strengthen §7)

§7 only checks `isatty(stderr)` and §4 pseudo-code `prompt_user(req)` does not specify the input source. Blind spot: default reading from `stdin` allows an attack like `echo G | dao run` to auto-grant Defer capabilities. Classic pipe spoofing.

**Fix:**

1. **Authoritative TTY check = opening the terminal device directly.**
   - POSIX: `open("/dev/tty", O_RDWR)`. Success $\rightarrow$ prompt. Failure (daemon has no controlling tty) $\rightarrow$ `E2205.NonTTYDefer`.
   - Windows: `CreateFile("CONIN$" / "CONOUT$")` (ConPTY console handles). Same fail-closed semantics.
2. **Both input + output are bound to the newly opened terminal handle**, NOT via `stdin`/`stdout`/`stderr`. Displaying `PolicyRequest` details and reading the user choice (g/d/G/D/?) both occur via the `/dev/tty` fd.
3. **`isatty(stderr)` is a fast pre-screen optimization**, not the authoritative check. Skipping the optimization $\rightarrow$ identical correctness. The loader may choose to pre-screen or always attempt to open `/dev/tty`.
4. **`--non-interactive` CLI flag** (ADR-0018 TBD) forces skipping the terminal open $\rightarrow$ `E2205.NonTTYDefer` even if a TTY is available. For attended scripts.
5. **Authorized wrapper inheritance:** If a user runs `dao run` inside a wrapper script that wraps `/dev/tty` via FIFO/expect $\rightarrow$ this is **authorized spoofing** (the user owns the wrapper). ADR-0017 does NOT attempt to prevent this; it only prevents **unauthorized pipe injection** from an untrusted caller redirecting `stdin`.

Prior art reference: `sudo(8)` AUTHENTICATION section — `/dev/tty` bypasses all password prompts for the same reason.

### §C — Abstain row errata (§2 table)

The §2 row text creates a false impression of a structural difference between `Ok(Trit::Zero)` and `Ok(Trit::Negative)`. In reality: the cache lifetime = process lifetime for all 4 outcomes (per §5 monotonicity invariant); "allow re-evaluation next session" applies universally — it is NOT a difference between Abstain and Deny.

**Errata** — replace §2 table rows with the following:

| Outcome | Runtime Consequence | Diagnostic |
|---|---|---|
| `Ok(Trit::Positive)` | Grant — cache as Grant **for process lifetime**; allow current + all future calls (cap_path, requester_pkg) | None |
| `Ok(Trit::Zero)` | Abstain — cache as Deny **for process lifetime**; behavior cache is **identical** to `Negative` | Info: "policy abstained" — distinction is diagnostic only |
| `Ok(Trit::Negative)` | Deny — cache as Deny **for process lifetime**; behavior cache is identical to `Zero` | None (decision recorded as authoritative) |
| `Err(PolicyError)` | Fail-closed Deny — cache as Deny with reason | E2205.<sub> |

**Implementation hint:** The code path for `Zero` and `Negative` shares the cache-write logic; the branch occurs only at the diagnostic emission step (the line right before the cache insertion).

**Restate monotonicity invariant** (already in §5, repeated for clarity): the cache is discarded upon process exit. Re-evaluation next session applies to **all** outcomes (Grant also re-evaluates at the next process start — it is not a permanent grant outside the session). A persistent grant occurs when the user selects `G` at the prompt $\rightarrow$ the rule is written to the `dao.policy` file $\rightarrow$ the next process reads the file and grants it from Step 2.

### Addendum References

- Trigger: author audit before opening v0.6.3 (ADR-0018 loader semantics).
- Pattern: mirrors [ADR-0015 Addendum](0015-package-store-layout.md#addendum--v05xreview-pre-v06-audit) — clarification + errata, does not reopen the decision.
- Commit: `docs(v0.6.2.addendum): ADR-0017 Addendum — parser strictness + TTY source + Abstain errata`.
