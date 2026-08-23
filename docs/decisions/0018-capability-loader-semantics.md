# ADR 0018 — Capability loader semantics (`dao.package` + eager link-time check + provenance prompt)

**Status:** Decided. Applies to v0.6 Capability System loader stage. Fills `E2208` reserved in [ADR-0016 §6](0016-capability-type-system.md). Finalizes manifest source syntax deferred from [ADR-0016 §1](0016-capability-type-system.md). Finalizes TTY prompt UX + parser implementation strategy deferred from [ADR-0017 §4](0017-trilean-policy-hook.md) + [Addendum §A/§B](0017-trilean-policy-hook.md#addendum--parser-strictness--tty-source--abstain-errata). Locks anti-typosquatting display per author constraint 2026-05-17 (commit `dd6b2f4`). No `abi_version` bump (keep `v=2`), no change to `.triv` wire format, no change to IR shape.

**Issue:** ADR-0016 + ADR-0017 locked semantics + protocol, but left 5 areas open for ADR-0018 to finalize:

1. **Source manifest file** — concrete `dao.package` grammar (ADR-0016 §1 only provided pseudo-syntax)
2. **Loader pipeline** — eager vs lazy + where to insert the step in the [ADR-0011 §8](0011-abi-metadata-format.md) workflow
3. **`dao.policy` reader** — implementation strategy (parser, miette span, per-E2205 error format)
4. **TTY prompt UX** — provenance display + anti-typosquatting (author constraint 2026-05-17)
5. **E2208 sub-variants** — loader refuse-to-load codes

Plus: replace `Capability { name: String }` placeholder in [`crates/triet-pack/src/types.rs`](../../crates/triet-pack/src/types.rs) with concrete `CapabilityClaim` struct shape.

## §1 — `dao.package` source manifest

**File location:** project root, hand-rolled line format, mirrors precedent [ADR-0015 §6](0015-package-store-layout.md) (`dao.lock`) + [ADR-0017 §3](0017-trilean-policy-hook.md) (`dao.policy`). Filename: `dao.package` — parallel naming convention. No `serde` dependency.

**Grammar:**

```text
format_version 1
name <pkg-name>
version <major>.<minor>.<patch>
requires <cap_path> <level>     # zero-or-more, sort by cap_path
requires <cap_path> <level>
...
dep <name> <min> <max_excl> <iface_hash_hex>   # zero-or-more, sort by name
dep <name> <min> <max_excl> <iface_hash_hex>
...
```

**Field rules:**

- `format_version`: required first non-comment line, value `1`. Duplicate or missing $\rightarrow$ E2208. ManifestParse refuse-to-load.
- `name`: ASCII identifier matching `[a-z][a-z0-9_]*` in v0.6. Required, exactly once. Unicode package names are deferred (packages need to be URL-safe for future remote registries).
- `version`: 3-tuple semver. Required, exactly once.
- `requires`: zero+ entries. `cap_path` = `AbsolutePath` ([ADR-0005](0005-module-system.md)); root MUST $\in$ {sys, dev, usr} per [ADR-0016 §5 rule 3](0016-capability-type-system.md); violation $\rightarrow$ E2206 InvalidCapabilityRoot.
- `dep`: zero+ entries. `min`/`max_excl` = semver triple. `iface_hash_hex` = 64 hex chars (BLAKE3, [ADR-0011 §4](0011-abi-metadata-format.md)). All-zero hex = no pin.

**Level tokens** (textual, intent-revealing — to distinguish from `dao.policy` numeric tokens):

| Token | CapabilityLevel | Wire encoding (ABI caps section) |
|---|---|---|
| `grant` | Grant (Trit::Positive) | u8 `0x02` |
| `ambient` | Ambient (Trit::Zero) | u8 `0x01` |
| `deny` | Deny (Trit::Negative) | u8 `0x00` |
| `defer` | Defer (Trilean::Unknown) | u8 `0x03` |

**Token convention mixed with `dao.policy`** (accepts trade-off):

| File | Tokens | Audience |
|---|---|---|
| `dao.package` | `grant` / `ambient` / `deny` / `defer` | Library author — textual, intent-revealing |
| `dao.policy` | `+1` / `0` / `-1` / `prompt` | Sysadmin / security audit — numeric, audit-compact |

Rationale for separation: 2 files, 2 audiences. Manifest is written rarely (publish-time); policy is edited often (deploy-time). Tokens are NOT aliased — `0` in the manifest = Ambient, but `0` in a policy decision = Abstain. Semantically distinct.

**Parser strictness:** same whitelist rules as [ADR-0017 Addendum §A](0017-trilean-policy-hook.md#addendum--parser-strictness--tty-source--abstain-errata). Any non-matching shape $\rightarrow$ E2208.ManifestParse refuse-to-load entire binary.

**Example:**

```text
# dao.package — myapp v0.1.0
format_version 1
name myapp
version 0.1.0
requires sys.io        grant
requires sys.net.dns   defer
requires dev.disk      deny
requires usr.somelib   grant
dep libdns 1.2.3 1.3.0 5c92ab17d4e8c1f6a3b8d2e5c97014b6f3e8d2a4c5b1f9e6d8c3a2b4f7e1d503
dep libtls 0.4.0 0.5.0 d041e8b9c5a2f4e8d1c6b3a597e0d4c8b1a3f6e9d2c5a7b4f1e8d3c0a6f9b2e4
```

**Canonical encoding:** writer sorts `requires` by `cap_path` ASC, `dep` by `name` ASC. Whitespace between fields: 1+ space/tab. Comments: line starts with `#` (no inline `#`). LF only. UTF-8 (BOM rejected). Mirrors Addendum §A.

## §2 — Loader pipeline (eager link-time cap check)

Insert 2 new steps into the [ADR-0011 §8](0011-abi-metadata-format.md) workflow (original 5 steps $\rightarrow$ 7 steps):

```
1. Open .tripack $\rightarrow$ parse ABI metadata.
2. Hash verify (CAS integrity, ADR-0014).
3. Version check (E2301, ADR-0013).
4. Dep resolve (ADR-0015 store lookups).
5. Semver check per dep (E2300-series).
6a. Capability section refusal — E2208 (NEW v0.6).         $\leftarrow$ ADR-0018
6b. Capability resolution — ADR-0017 machinery for Defer. $\leftarrow$ ADR-0018
7. Witness link.
8. Load code into VM.
```

**Eager mode** — ALL Defer caps in the dependency tree are resolved **before main()**:
- User sees batch TTY prompts (if any) at startup, without being interrupted mid-run.
- Predictable failure: if a cap is rejected $\rightarrow$ process aborts BEFORE user code runs, preventing partial state.
- Matches security tool UX (`sudo` asks once upfront; `apt` asks before install).

**Lazy mode** (defer resolution to first cross-namespace call site): deferred to post-v0.7 if hot-path profiling demands it. Not required for v0.6 — cache hit is O(1) after eager warmup.

**Step 6a — Capability section refusal** (BEFORE policy hook fires):

Linker checks structural validity BEFORE calling ADR-0017 resolution machinery. Order matters — the policy hook must never fire on malformed input:

```
for each pack in load_order:
    if reader_abi_version < pack.abi_version_with_caps_semantics:
        if pack.caps.count > 0:
            return E2208.PreV06Reader
    if !manifest_matches_abi(pack):
        return E2208.CapabilityDivergence
    for each cap in pack.caps:
        resolve_path_in_dep_exports(cap.path)  // may raise E2202 (ADR-0016 §6)
```

**Step 6b — Capability resolution** (eager iterate):

```
let cache = PolicyCache::new()
for each cap in union(root.caps, transitive_deps.caps):
    match cap.level:
        Grant   -> cache.insert((cap.path, root.pkg), Grant)
        Ambient -> if root_pkg: cache.insert((cap.path, root.pkg), Deny)  // ambient at root = deny
                   else: defer to parent caller decision
        Deny    -> cache.insert((cap.path, root.pkg), Deny)
        Defer   -> let decision = ADR_0017.resolve(PolicyRequest { ... })
                   cache.insert((cap.path, root.pkg), decision)
                   if decision is Err: emit diagnostic, continue (not abort)
```

Single Defer failure $\rightarrow$ per-key Deny + diagnostic (ADR-0017 §6 NonTTYDefer / PromptCrash semantics). Process continues with other caps. Step 7 only fires when the cache is fully warm.

## §3 — `dao.policy` reader implementation strategy

[ADR-0017 Addendum §A](0017-trilean-policy-hook.md#addendum--parser-strictness--tty-source--abstain-errata) specified **WHAT** (whitelist rules); ADR-0018 specifies **HOW** (parser strategy + miette span + error format).

**Strategy:** Line tokenizer with explicit state machine. Pseudo-code:

```text
fn parse_policy(path: &Path) -> Result<PolicyRules, E2205> {
    let bytes = read_file_with_size_limit(path, 1 MiB)?      // E2205.ConfigParse if > 1 MiB
    reject_bom(bytes)?                                        // E2205.ConfigParse
    let mut state = Header                                    // Header | Body
    let mut rules = Vec::new()
    for (line_no, line_bytes) in bytes.split(b'\n').enumerate():
        reject_crlf_remnant(line_bytes, line_no)?             // E2205.ConfigParse
        reject_unicode_whitespace_outside_ident(line_bytes)?  // E2205.ConfigParse
        reject_line_too_long(line_bytes, 4096, line_no)?      // E2205.ConfigParse
        let trimmed = trim_ascii_ws(line_bytes)
        if trimmed.is_empty() || trimmed[0] == b'#': continue
        reject_inline_comment(trimmed, line_no)?              // E2205.ConfigParse
        match state:
            Header -> parse_format_version(trimmed, line_no, &mut state)?
            Body   -> parse_rule_or_default(trimmed, line_no, &mut rules)?
    reject_missing_format_version(&state)?                    // E2205.ConfigParse
    reject_duplicate_rules(&rules)?                            // E2205.RuleConflict
    Ok(PolicyRules::from(rules))
}
```

**Miette span format:** every E2205 sub-variant carries `(line: usize, col_start: <<usize, col_end: usize)` + file `source` string. Diagnostic rendering: file path + `line:col` + offending bytes highlighted with ANSI escape.

**Per-E2205 error message format** (locked for v0.7 self-host bit-identical parity):

| Sub-variant | Format |
|---|---|
| `E2205.ConfigParse` | `dao.policy:{line}:{col}: invalid {what} — {reason}` (+ hint when applicable) |
| `E2205.RuleConflict` | `dao.policy:{line}:{col}: duplicate rule for ({path}, {origin}) — first declared at line {first_line}` |
| `E2205.UnknownOrigin` | `dao.policy:{line}:{col}: unknown origin '{token}' — expected: lockfile, ifacepin, fresh, *` |
| `E2205.UnknownDecision` | `dao.policy:{line}:{col}: unknown decision '{token}' — expected: +1, 0, -1, prompt` |
| `E2205.NonTTYDefer` | `cap '{cap_path}' (requester {pkg}@{ver}): policy returned 'prompt' but no TTY available — set explicit rule in dao.policy or run with TTY` |
| `E2205.PromptCrash` | `cap '{cap_path}': TKY prompt I/O error: {os_error} — treating as Deny` |

**Memoization:** parse once per process, cache `PolicyRules` as immutable. Re-parse on next process start (capability monotonicity invariant per [ADR-0017 §5](0017-trilean-policy-hook.md)).

**Apply same strategy to `dao.package` parser** — share tokenizer code path; differ in semantic validation. E2208.ManifestParse uses identical span + format conventions.

## §4 — TTY prompt UX (provenance display + anti-typosquatting)

Lock format per author constraint 2026-05-17. **Full hash, no truncation anywhere** — security context, short-SHA collision attack surface.

```text
[triet] Capability decision required

  Capability:     sys.net.dns
  Decision token: defer  (per dao.policy rule, origin=Fresh)

  Requester (package asking):
    Name:        myapp@0.1.0
    iface_hash:  e7a1c4f0b2d8a629f4e8d0c7b3a51928f6e2d9c8a4b3f7e9d8c6a2b1f5e3d829
                 (matches dao.lock OK)
    impl_hash:   91b3d8e2a4c7d935a8e6f0b2d4c97186a3e5f8d2c0b4a791e2f5c8d9a04af5b6
    Store path:  ~/.triet/store/pkg/91b3d8e2a4c7d935a8e6f0b2d4c97186a3e5f8d2c0b4a7ng_path/pack.tripack

  Dep chain:
    myapp@0.1.0
      iface_hash:  e7a1c4f0b2d8a629f4e8d0c7b3a51928f6e2d9c8a4b3f7e9d8c6a2b1f5e3d829
      (root)

    └─ libdns@1.2.3
         iface_hash:  5c92ab17d4e8c1f6a3b8d2e5c97014b6f3e8d2a4c5b1f9e6d8c3a2b4f7e1d503
         origin=Fresh    !! NOT in lockfile

         └─ libtls@0.4.1
              iface_hash:  d041e8b9c5a2f4e8d1c6b3a597e0d4c8b1a3f6e9d2c5a7b4f1e8d3c0a6t6f9b2e4
              origin=Lockfile

  !! Origin Fresh: libdns@1.2.3 was added since last lockfile commit.
  !! Verify hash against your records before granting.

  [g] grant once   [d] deny once
  [G] grant permanent (write rule to dao.policy)
  [D] deny permanent  (write rule to dao.policy)
  [?] explain   [h] show hash help

  choice >
```

**Lock decisions:**

| Aspect | Decision | Rationale |
|---|---|---|
| Hash display | **Full 64 hex chars, never truncate** | Security: short-SHA collision attack surface |
| Hash line wrap | Single line if terminal width $\ge$ 100 cols; wrap to 2 lines of 32 chars if < 100 cols | Audit comparison friendly |
| Lockfile cross-check | `(matches dao.lock OK)` / `(MISMATCH — was <full_hash>)` / `(not in lockfile)` | Strongest typosquatting signal — show full mismatch hash, not partial |
| Origin per dep | Always shown: `origin=Fresh` / `origin=IfacePin` / `origin=Lockfile` — color-coded ANSI (Fresh=yellow, IfacePin=cyan, Lockfile=default) | Reinforces "new dep" warning |
| Box-drawing | None in mock (avoid overflow with full hash); indentation only. Implementation may use `┌─┐│└─┘` Unicode if `$TERM` supports (terminfo check), ASCII fallback otherwise | Compatibility |
| Color | ANSI 16-color default; disable per `$NO_COLOR` env spec | Standard convention |
| Warning markers | `!!` ASCII (not Unicode `⚠`) — guaranteed render across terminals | Compatibility — security message must always render |
| Language | English only in v0.6; i18n hook reserved | CLI consistency with existing diagnostics; security context disallows ambiguity |
| Input source | `/dev/tty` (POSIX) / ConPTY (Windows) per [ADR-0017 Addendum §B](0017-trilean-policy-hook.md#addendum--parser-strictness--tty-source--abstain-errata) | Anti-spoofing |
| Output destination | `/dev/tty` (paired with input) | Consistency — does not go through stderr redirection |

**`G`/`D` write semantics:** append rule to `dao.policy` BEFORE caching:

```text
1. Open dao.policy for append. Missing $\rightarrow$ create with "format_version 1\n".
2. Append: rule <cap_path> <origin> <decision>
   - decision = "+1" for G / "-1" for D
   - origin = origin from PolicyRequest
3. fsync() for durability. Fail $\rightarrow$ fallback to session-only cache + warning diagnostic.
4. Re-sort atomically: write canonical sorted form to dao.policy.tmp, rename() to dao.policy.
   (Mirrors atomic install pattern ADR-0015 §5.)
5. Cache decision in session.
```

**`[?] explain` action:** print expanded rationale:
- Which dep declared this cap
- Why Defer arose (no rule matched / explicit `prompt` decision in policy)
- Suggested `dao.policy` entries to pre-grant for next session
Then re-prompt.

**`[h] show hash help` action:** print BLAKE3 verification guide:
- "Compare iface_hash against trusted registry / Git tag / colleague's lockfile."
- "Hash mismatch = different package, even if name same. Refuse if unsure."
Then re-prompt.

## §5 — E2208 sub-variants (loader refuse-to-load)

[ADR-0016 §6](0016-capability-type-system.md) reserved `E2208` for ADR-0018. Locking 3 sub-variants:

| Code | Variant | Stage | When |
|---|---|---|---|
| `E2208.PreV06Reader` | Reader pre-v0.6 sees `cap_count > 0` in `.khi` ABI metadata | Step 6a load-time | Forward-compat refusal — pre-v0.6 binary cannot validate caps |
| `E2208.ManifestParse` | `dao.package` source file syntax error | Pre-build (compiler reads source) | Whitelist parser refuse-to-load |
| `E2208.CapabilityDivergence` | `dao.package` declares `requires` lines but `.khi` `caps_count = 0` (writer bug) | Step 6a load-time | Writer/reader divergence detection |

**Stage table:**

- Sub-variant 1 fires at loader **Step 6a** (after dep resolve, before policy hook). Refuse entire link.
- Sub-variant 2 fires **pre-build** (compiler reading source before emitting `.khi`). Refuse compilation.
- Sub-variant 3 fires at loader **Step 6a**. Refuse entire link.

**Diagnostic format:** miette with primary span on `.khi` byte offset (sub-variant 1, 3) or `dao.package:line:col` (sub-variant 2). Format mirrors §3 E2205 conventions.

**Not E2208** (already covered by other codes):
- E2202 `UnresolvedCapabilityPath` (ADR-0016 §6) — cap path does not match dep export. Fires at Step 6a but uses E2202.
- E2203 `CapabilityRefused` — root manifest refuses. Fires at Step 6a after structural validation passes.
- E2205.<sub> — policy hook errors. Fires at Step 6b.

## §6 — `CapabilityClaim` Rust struct shape (replace placeholder)

Current placeholder at [`crates/triet-pack/src/types.rs:272-277`](../../crates/triet-pack//src/types.rs):

```rust
pub struct Capability {
    pub name: String,  // placeholder, ADR-0016 picks shape
}
```

Replace with (locked by ADR-0018):

```rust
pub struct CapabilityClaim {
    pub cap_path: AbsolutePath,    // ADR-0005 path type
    pub level: CapabilityLevel,
}

pub enum CapabilityLevel {
    Grant,    // Trit::Positive  (+1)  $\rightarrow$ u8 0x02
    Ambient,  // Trit::Zero      ( 0)  $\rightarrow$ u8 0x01
    Deny,     // Trit::Negative  (-1)  $\rightarrow$ u8 0x00
    Defer,    // Trilean::Unknown      $\rightarrow$ u8 0x03
}
```

**Rename** `Capability` $\rightarrow$ `CapabilityClaim` for clarity (avoid confusion with generic "capability" concept). `AbiMetadata.caps: Vec<Capability>` $\rightarrow$ `caps: Vec<cap_path>`. Breaking change in Rust API, but caps slot is always empty in v0.5 $\rightarrow$ zero impact on existing test fixtures.

Wire encoding (ABI caps section binary format) unchanged from [ADR-0016 §4](0016-capability-type-system.md): `cap_count` varint + per-entry `(namespace_path: length-prefixed UTF-8, level: u8, reserved: u8)`. Sort canonical by `namespace_path`.

## Consequences

### For ADR-0016 — closes §6 E22XX namespace

After ADR-0018, E22XX namespace is fully populated: E2200–E2204, E2205 (+ 6 sub-variants ADR-0017), E2206–E2207, E2208 (+ 3 sub-variants ADR-0018). No reserved slots remain in the v0.6 namespace.

### For ADR-0017 — closes deferred sections

ADR-0017 §4 pseudo-code `prompt_user(req)` $\rightarrow$ §4 mock is fully locked. Addendum §A whitelist rules $\rightarrow$ §3 implementation strategy is locked. Addendum §B `/dev/tty` direction $\rightarrow$ §4 lock decisions table is applied.

### For [`triet-pack`](../../crates/triet-pack) crate

Implementation targets (v0.6.4+ sub-tasks):
- `crates/triet-pack/src/types.rs`: rename `Capability` $\rightarrow$ `CapabilityClaim`, add `CapabilityLevel` enum.
- `crates/triet-pack/src/serde.rs`: extend writer/reader for non-empty caps section.
- New `crates/triet-pack/src/package_manifest.rs`: `dao.package` parser + writer (mirrors `lockfile.rs` pattern).
- New `crates/triet-pack/src/policy.rs`: `dao.．policy` parser + writer (mirrors `lockfile.rs` pattern).
- New `crates/triet-pack/src/capability_resolver.rs`: PolicyCache + ADR-0017 §4 algorithm + ADR-0018 §2 loader steps 6a/6b.

### For `triet-cli`

New subcommands (v0.6.4+):
- `triet pack init` — emit boilerplate `dao.package`
- `triet policy show` — render `dao.policy` rules table
- `triet policy add <cap> <origin> <decision>` — append rule atomically
- TTY prompt machinery wired into runtime link path

### For ABI metadata ([ADR-0011](0011-abi-metadata-format.md))

Binary format unchanged. `abi_version` remains `v=2`. `caps section` populated per ADR-0016 §4 encoding (already locked).

### For IR ([ADR-0007](0007-ir-design.md)) / `.triv` wire format

Unchanged. Cap check fires at loader stage, no new IR opcode.

### For v0.7 self-hosting

Self-hosted parser for `dao.package` + `dao.policy` must emit byte-identical errors with Rust implementation per §3 format table. Critical for bit-identical bootstrap (ROADMAP §v0.7 gate).

### For v0.8 concurrency

Eager mode cache fully warm before `main()` $\rightarrow$ v0.8 actor threads share immutable `PolicyCache` snapshot. Thread-safety finalized in v0.8 concurrency ADR; ADR-0018 does not pre-commit lock shape.

### For v0.9 JIT / v2.0 AOT

Cached decision is authoritative; JIT lift across cap boundary reads cache, does not re-evaluate. AOT v2.0: cache state baked into binary header is REJECTED — cache is initialized empty per process (deployment-specific, not AOT-baked).

## Alternatives Considered

- **Lazy cap resolution** — deferred to post-v0.7 if hot-path profiling demands it. Eager is sufficient for v0.6.
- **Source manifest implementation** — ADR-0018 locks grammar; writer/reader/CLI implementation = v0.6.4+ sub-tasks in TODO.md. Separates design vs implementation cadence.
- **Multi-language manifest** — English only in v0.6; i18n deferred indefinitely (security context disallows ambiguity).
- **Capability claim composition** (claim references another claim) — NOT in v0.6; each entry is self-contained.
- **Versioning `dao.package` format** — `format_version 1` is sufficient; future ADR bump if additive fields are needed.
- **Persistent session cache across processes** — cache discarded on process exit per ADR-0017 §5 monotonicity.
- **TTY prompt timeout** — synchronous, no timeout per ADR-0017 §8 known limit.
- **Hash truncation anywhere in UI** — full 64 hex chars always. Short-SHA = collision attack surface.
- **Box-drawing chars in core security display** — ASCII fallback markers (`!!` not `⚠`). Security message must render guaranteed.
- **Auto-generate `dao.policy` rules from dep tree heuristics** — refuse over guess. User must explicitly choose `G`/`D` at prompt OR write rule manually.

## Prior art

- **[`Cargo.toml`](https://doc.rust-lang.org/cargo/reference/manifest.html)** — Rust source manifest. Inspires `dao.package` field shape (name, version, deps); rejected TOML format because hand-rolled precedent is stronger.
- **[`go.mod` + `go.sum`](https://go.dev/ref/mod)** — hand-rolled module file with hash pins. Closer precedent — line format, no nested syntax, hash-as-trust-anchor. Direct inspiration for `dao.package`.
- **[npm `package.json` + `package-lock.json`](https://docs.npmjs.com/cli/v9/configuring-npm/package-json)** — JSON manifest. Rejected because JSON syntax invites silent typing errors (string-vs-number, missing-trailing-comma rendering ambiguous).
- **[Android `<uses-permission>` + runtime grant dialog](https://developer.android.com/guide/topics/manifest/uses-permission-element)** — Manifest declares + OS prompts at runtime. Direct inspiration for ADR-0018 §4 mock UI structure.
- **`sudo(8)` AUTHENTICATION** — `/dev/tty` direct read, terminal-bound prompt. Direct precedent for ADR-0018 §4 lock decisions (input/output source).
- **`apt install` Y/N prompt** — eager confirmation before action. Direct precedent for §2 eager mode UX.
- **[Nix `trusted-public-keys` + signature verify](https://nixos.org/manual/nix/stable/installation/multi-user.html)** — CAS hash anti-typosquatting. Inspires §4 anti-typosquatting display (full hash + lockfile cross-check).

**Anti-prior-art:**

- **`npm install` legacy auto-resolve** — silent transitive grants $\rightarrow$ supply chain CVEs. ADR-0018 is explicitly the opposite: eager prompt + refuse-over-guess.
- **Java `policy` files with grant blocks** — verbose nested syntax + JVM-internal semantics $\rightarrow$ barely used in practice. ADR-0018 uses flat line format, security-front-and-center.
- **Short SHA in package UIs** (Git, GitHub PR refs) — collision attack surface. ADR-0018 §4 lock: never truncate hash in security context.

## References

- [VISION §3.5 + §5 + §6](../../VISION.md)
- [SPEC §1.3 (identifiers), §10 (reserved roots)](../../SPEC.md)
- [ADR-0005 — Module system (AbsolutePath)](0005-module-system.md)
- [ADR-0011 §4 (dep table), §5 (caps section), §8 (linker workflow)](0011-abi-metadata-format.md)
- [ADR-0013 — Semver linking policy (E23XX series)](0013-semver-linking-policy.md)
- [ADR-0014 §4 (impl_hash unforgeable trust anchor)](0014-hash-scheme-refinement.md)
- [ADR-0015 §6 (hand-rolled file format precedent — `dao.lock`)](0015-package-store-layout.md)
- [ADR-0016 §1 (manifest pseudo-syntax), §4 (caps section encoding), §6 (E22XX namespace)](0016-capability-type-system.md)
- [ADR-0017 §3 (dao.policy grammar), §4 (resolution algorithm), §5 (monotonicity), Addendum §A (parser whitelist), Addlam §B (/dev/tty)](0017-trilean-policy-hook.md)
- TODO.md v0.6.3 anti-typosquatting constraint (commit `dd6b2f4`)
- [`crates/triet-pack/src/types.rs:272-277`](../../crates/triet-pack/src/types.rs) — placeholder being replaced
- [`crates/triet-pack/src/lockfile.rs`](../../crates/triet-pack/src/lockfile.rs) — hand-rolled parser precedent to mirror
- [ROADMAP §v0.6](../../ROADMAP.md)

---

## Addendum — v0.6.x.review (pre-v0.7 audit)

Audit window post-decision, mirrors precedent [ADR-0015 Addendum](0015-package-store-layout.md#addendum--v05xreview-pre-v06-audit). All 3 ADRs (0016, 0017, 0018) were verified; findings are anchored here because 0018 is the capstone integrative of the v0.6 phase.

### Test coverage scorecard

| Original gap | Layer | Status | Anchor |
|---|---|---|---|
| Monotonicity replay assertion | resolver | Partial $\rightarrow$ strengthened | `second_resolve_same_key_replays_from_cache` (replay only) + new `monotonicity_holds_under_policy_mutation` (mutation invariant) |
| `upsert_rule` + `save` round-trip | policy | Real gap $\rightarrow$ filled | new `upsert_then_save_round_trip` |
| Multi-dep aggregation determinism | linker | Partial $\rightarrow$ strengthened | `multiple_dep_requesters_aggregated` (alphabetical insertion) + new `requesters_sorted_when_inserted_out_of_order` |
| E2204 duplicate cap claim | manifest | Already covered | `rejects_duplicate_requires` |
| Unused `grant` claim semantic | typecheck | Already covered | `orphan_claim_without_import_passes` |
| `prompt_loop` retry-on-invalid | tty | Already covered | `prompt_loop_reprompts_on_invalid_input` |
| `?` ShowHashHelp branch | tty | Already covered | `prompt_loop_reprompts_on_hash_help_then_terminal` |
| `default prompt` rejection message | policy | Already covered | `rejects_default_prompt` (reason contains "static") |
| Cross-stage propagation | pipeline | Not a v0.6 gap | CLI orchestration deferred to v0.7 per SPEC §0.7 |
| CRLF/BOM positional contract | strict_parser | Partial $\rightarrow$ strengthened | basic `rejects_bom`/`rejects_crlf` + new `empty_file_succeeds_with_zero_callbacks` + `bom_mid_file_classifies_as_non_ascii_not_bom` + `cr_mid_line_classifies_as_non_ascii_not_crlf` |

Audit listed 10 gaps; 5 already covered, 1 deferred (CLI wiring $\rightarrow$ v0.7), 4 partial/real $\rightarrow$ 6 net-new tests across review.1 (`d56c518`) + review.2 (`b6bde0c`). Workspace: 1079 $\rightarrow$ 1085 tests, clippy `-D warnings` clean.

### Monotonicity invariant — pinned under PolicyRules mutation

ADR-0017 §5 mandates "knowledge growth doesn't flip". v0.6.9 implementation honors this (cache lookup precedes rule lookup), but existing tests only proved replay, not the mutation step. v0.6.x.review.1 added an assertion: flipping a rule `+1 $\rightarrow$ -1` mid-session $\rightarrow$ cached `Positive` survives + source=Cache. Commit `d56c51 $\rightarrow$ d56c518`.

### `upsert_rule` + `save` insight — in-memory $\neq$ disk byte-equal

Test surfaced a subtle contract: `upsert_rule` appends to a `Vec` (insertion order); `save` canonicalizes via sort-by-cap-path $\rightarrow$ in-memory state is NOT byte-equal to disk state. User-facing guarantee: rule survives round-trip. Test also asserts that the canonical form is a fixed point across re-saves. Important context for DevTtyPrompt G/D path. Commit `d56c518`.

### Strict parser positional contracts

`strict_parser.rs` distinguishes positional violations (Bom = file-start; CRLF = line-trailing) vs generic Non-ASCII. Existing tests covered positive cases only; v0.6.x.review.2 pins *negative* cases to prevent future refactors from conflating distinct violation kinds. Commit `b6bde0c`.
