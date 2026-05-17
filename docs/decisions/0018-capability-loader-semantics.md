# ADR 0018 — Capability loader semantics (`triet.package` + eager link-time check + provenance prompt)

**Trạng thái:** Quyết định. Áp dụng cho v0.6 Capability System loader stage. Lấp đầy `E2208` reserved trong [ADR-0016 §6](0016-capability-type-system.md). Hoàn thiện manifest source syntax đã defer từ [ADR-0016 §1](0016-capability-type-system.md). Hoàn thiện TTY prompt UX + parser implementation strategy đã defer từ [ADR-0017 §4](0017-trilean-policy-hook.md) + [Addendum §A/§B](0017-trilean-policy-hook.md#addendum--parser-strictness--tty-source--abstain-errata). Lock anti-typosquatting display per author constraint 2026-05-17 (commit `dd6b2f4`). Không bump `abi_version` (giữ `v=2`), không đổi `.triv` wire format, không đổi IR shape.

**Issue:** ADR-0016 + ADR-0017 lock semantics + protocol nhưng để hở 5 vùng để ADR-0018 chốt:

1. **Source manifest file** — concrete `triet.package` grammar (ADR-0016 §1 chỉ có pseudo-syntax)
2. **Loader pipeline** — eager vs lazy + chèn step nào trong [ADR-0011 §8](0011-abi-metadata-format.md) workflow
3. **`triet.policy` reader** — implementation strategy (parser, miette span, per-E2205 error format)
4. **TTY prompt UX** — provenance display + anti-typosquatting (author constraint 2026-05-17)
5. **E2208 sub-variants** — loader refuse-to-load codes

Plus: replace `Capability { name: String }` placeholder ở [`crates/triet-pack/src/types.rs`](../../crates/triet-pack/src/types.rs) với concrete `CapabilityClaim` struct shape.

## §1 — `triet.package` source manifest

**File location:** project root, hand-rolled line format, mirror precedent [ADR-0015 §6](0015-package-store-layout.md) (`triet.lock`) + [ADR-0017 §3](0017-trilean-policy-hook.md) (`triet.policy`). Tên file: `triet.package` — parallel naming convention. Không serde dep.

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

- `format_version`: required first non-comment line, value `1`. Duplicate hoặc missing → E2208.ManifestParse refuse-to-load.
- `name`: ASCII identifier matching `[a-z][a-z0-9_]*` ở v0.6. Required, exactly once. Unicode package names defer (packages cần URL-safe cho future remote registry).
- `version`: 3-tuple semver. Required, exactly once.
- `requires`: zero+ entries. `cap_path` = `AbsolutePath` ([ADR-0005](0005-module-system.md)); root MUST ∈ {sys, dev, usr} per [ADR-0016 §5 rule 3](0016-capability-type-system.md), violate → E2206 InvalidCapabilityRoot.
- `dep`: zero+ entries. `min`/`max_excl` = semver triple. `iface_hash_hex` = 64 hex chars (BLAKE3, [ADR-0011 §4](0011-abi-metadata-format.md)). All-zero hex = no pin.

**Level tokens** (textual, intent-revealing — distinguish khỏi `triet.policy` numeric tokens):

| Token | CapabilityLevel | Wire encoding (ABI caps section) |
|---|---|---|
| `grant` | Grant (Trit::Positive) | u8 `0x02` |
| `ambient` | Ambient (Trit::Zero) | u8 `0x01` |
| `deny` | Deny (Trit::Negative) | u8 `0x00` |
| `defer` | Defer (Trilean::Unknown) | u8 `0x03` |

**Token convention mixed với `triet.policy`** (chấp nhận trade-off):

| File | Tokens | Audience |
|---|---|---|
| `triet.package` | `grant` / `ambient` / `deny` / `defer` | Library author — textual, intent-revealing |
| `triet.policy` | `+1` / `0` / `-1` / `prompt` | Sysadmin / security audit — numeric, audit-compact |

Lý do tách: 2 file 2 audience. Manifest written rare (publish-time); policy edited often (deploy-time). Tokens KHÔNG alias được — `0` ở manifest = Ambient nhưng `0` ở policy decision = Abstain. Semantic-distinct.

**Parser strictness:** same whitelist rules [ADR-0017 Addendum §A](0017-trilean-policy-hook.md#addendum--parser-strictness--tty-source--abstain-errata). Mọi shape không match → E2208.ManifestParse refuse-to-load entire binary.

**Example:**

```text
# triet.package — myapp v0.1.0
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

**Canonical encoding:** writer sort `requires` by `cap_path` ASC, `dep` by `name` ASC. Whitespace giữa fields: 1+ space/tab. Comments: line starts với `#` (no inline `#`). LF only. UTF-8 (BOM rejected). Mirror Addendum §A.

## §2 — Loader pipeline (eager link-time cap check)

Chèn 2 step mới vào [ADR-0011 §8](0011-abi-metadata-format.md) workflow (gốc 5 step → 7 step):

```
1. Open .tripack → parse ABI metadata.
2. Hash verify (CAS integrity, ADR-0014).
3. Version check (E2301, ADR-0013).
4. Dep resolve (ADR-0015 store lookups).
5. Semver check per dep (E2300-series).
6a. Capability section refusal — E2208 (NEW v0.6).         ← ADR-0018
6b. Capability resolution — ADR-0017 machinery cho Defer. ← ADR-0018
7. Witness link.
8. Load code into VM.
```

**Eager mode** — TẤT CẢ Defer caps trong dep tree resolve **before main()**:
- User thấy batch TTY prompts (nếu có) ở startup, không bị interrupt mid-run.
- Predictable failure: nếu cap reject → process abort BEFORE user code runs, không partial state.
- Match security tool UX (`sudo` asks once upfront; `apt` asks before install).

**Lazy mode** (defer resolve to first cross-namespace call site): defer post-v0.7 nếu hot-path profiling demand. v0.6 không cần — cache hit O(1) sau eager warmup.

**Step 6a — Capability section refusal** (BEFORE policy hook fires):

Linker check structural validity TRƯỚC khi gọi ADR-0017 resolution machinery. Order matters — policy hook không bao giờ fire trên malformed input:

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
        Ambient -> if root_pkg: cache.insert((cap.path, root.pkg), Deny)  // ambient ở root = deny
                   else: defer to parent caller decision
        Deny    -> cache.insert((cap.path, root.pkg), Deny)
        Defer   -> let decision = ADR_0017.resolve(PolicyRequest { ... })
                   cache.insert((cap.path, root.pkg), decision)
                   if decision is Err: emit diagnostic, continue (not abort)
```

Single Defer failure → per-key Deny + diagnostic (ADR-0017 §6 NonTTYDefer / PromptCrash semantics). Process tiếp tục với other caps. Step 7 chỉ fire khi cache fully warm.

## §3 — `triet.policy` reader implementation strategy

[ADR-0017 Addendum §A](0017-trilean-policy-hook.md#addendum--parser-strictness--tty-source--abstain-errata) đã spec **WHAT** (whitelist rules); ADR-0018 spec **HOW** (parser strategy + miette span + error format).

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

**Miette span format:** mọi E2205 sub-variant carry `(line: usize, col_start: usize, col_end: usize)` + file `source` string. Diagnostic rendering: file path + `line:col` + offending bytes highlighted với ANSI escape.

**Per-E2205 error message format** (locked for v0.7 self-host bit-identical parity):

| Sub-variant | Format |
|---|---|
| `E2205.ConfigParse` | `triet.policy:{line}:{col}: invalid {what} — {reason}` (+ hint khi applicable) |
| `E2205.RuleConflict` | `triet.policy:{line}:{col}: duplicate rule for ({path}, {origin}) — first declared at line {first_line}` |
| `E2205.UnknownOrigin` | `triet.policy:{line}:{col}: unknown origin '{token}' — expected: lockfile, ifacepin, fresh, *` |
| `E2205.UnknownDecision` | `triet.policy:{line}:{col}: unknown decision '{token}' — expected: +1, 0, -1, prompt` |
| `E2205.NonTTYDefer` | `cap '{cap_path}' (requester {pkg}@{ver}): policy returned 'prompt' but no TTY available — set explicit rule in triet.policy or run with TTY` |
| `E2205.PromptCrash` | `cap '{cap_path}': TTY prompt I/O error: {os_error} — treating as Deny` |

**Memoization:** parse once per process, cache `PolicyRules` immutable. Re-parse on next process start (capability monotonicity invariant per [ADR-0017 §5](0017-trilean-policy-hook.md)).

**Apply same strategy to `triet.package` parser** — share tokenizer code path; differ ở semantic validation. E2208.ManifestParse uses identical span + format conventions.

## §4 — TTY prompt UX (provenance display + anti-typosquatting)

Lock format per author constraint 2026-05-17. **Full hash, no truncation anywhere** — security context, short-SHA collision attack surface.

```text
[triet] Capability decision required

  Capability:     sys.net.dns
  Decision token: defer  (per triet.policy rule, origin=Fresh)

  Requester (package asking):
    Name:        myapp@0.1.0
    iface_hash:  e7a1c4f0b2d8a629f4e8d0c7b3a51928f6e2d9c8a4b3f7e9d8c6a2b1f5e3d829
                 (matches triet.lock OK)
    impl_hash:   91b3d8e2a4c7d935a8e6f0b2d4c97186a3e5f8d2c0b4a791e2f5c8d9a04af5b6
    Store path:  ~/.triet/store/pkg/91b3d8e2a4c7d935a8e6f0b2d4c97186a3e5f8d2c0b4a791e2f5c8d9a04af5b6/pack.tripack

  Dep chain:
    myapp@0.1.0
      iface_hash:  e7a1c4f0b2d8a629f4e8d0c7b3a51928f6e2d9c8a4b3f7e9d8c6a2b1f5e3d829
      (root)

    └─ libdns@1.2.3
         iface_hash:  5c92ab17d4e8c1f6a3b8d2e5c97014b6f3e8d2a4c5b1f9e6d8c3a2b4f7e1d503
         origin=Fresh    !! NOT in lockfile

         └─ libtls@0.4.1
              iface_hash:  d041e8b9c5a2f4e8d1c6b3a597e0d4c8b1a3f6e9d2c5a7b4f1e8d3c0a6f9b2e4
              origin=Lockfile

  !! Origin Fresh: libdns@1.2.3 was added since last lockfile commit.
  !! Verify hash against your records before granting.

  [g] grant once   [d] deny once
  [G] grant permanent (write rule to triet.policy)
  [D] deny permanent  (write rule to triet.policy)
  [?] explain   [h] show hash help

  choice >
```

**Lock decisions:**

| Aspect | Decision | Lý do |
|---|---|---|
| Hash display | **Full 64 hex chars, never truncate** | Security: short-SHA collision attack surface |
| Hash line wrap | Single line nếu terminal width ≥ 100 cols; wrap to 2 lines of 32 chars nếu < 100 cols | Audit comparison friendly |
| Lockfile cross-check | `(matches triet.lock OK)` / `(MISMATCH — was <full_hash>)` / `(not in lockfile)` | Strongest typosquatting signal — show full mismatch hash, not partial |
| Origin per dep | Always shown: `origin=Fresh` / `origin=IfacePin` / `origin=Lockfile` — color-coded ANSI (Fresh=yellow, IfacePin=cyan, Lockfile=default) | Reinforces "new dep" warning |
| Box-drawing | None ở mock (avoid overflow with full hash); indentation only. Implementation có thể dùng `┌─┐│└─┘` Unicode nếu `$TERM` supports (terminfo check), ASCII fallback ngược lại | Compatibility |
| Color | ANSI 16-color default; disable per `$NO_COLOR` env spec | Standard convention |
| Warning markers | `!!` ASCII (not Unicode `⚠`) — guaranteed render across terminals | Compatibility — security message must always render |
| Language | English only ở v0.6; i18n hook reserved | CLI consistency với existing diagnostics; security context disallows ambiguity |
| Input source | `/dev/tty` (POSIX) / ConPTY (Windows) per [ADR-0017 Addendum §B](0017-trilean-policy-hook.md#addendum--parser-strictness--tty-source--abstain-errata) | Anti-spoofing |
| Output destination | `/dev/tty` (paired với input) | Consistency — không qua stderr redirect |

**`G`/`D` write semantics:** append rule vào `triet.policy` BEFORE caching:

```text
1. Open triet.policy for append. Missing → create với "format_version 1\n".
2. Append: rule <cap_path> <origin> <decision>
   - decision = "+1" cho G / "-1" cho D
   - origin = origin từ PolicyRequest
3. fsync() để durable. Fail → fallback session-only cache + warning diagnostic.
4. Re-sort atomically: write canonical sorted form to triet.policy.tmp, rename() to triet.policy.
   (Mirrors atomic install pattern ADR-0015 §5.)
5. Cache decision in session.
```

**`[?] explain` action:** print expanded rationale:
- Which dep declared this cap
- Why Defer arose (no rule matched / explicit `prompt` decision in policy)
- Suggested `triet.policy` entries to pre-grant for next session
Then re-prompt.

**`[h] show hash help` action:** print BLAKE3 verification guide:
- "Compare iface_hash against trusted registry / Git tag / colleague's lockfile."
- "Hash mismatch = different package, even if name same. Refuse if unsure."
Then re-prompt.

## §5 — E2208 sub-variants (loader refuse-to-load)

[ADR-0016 §6](0016-capability-type-system.md) reserved `E2208` cho ADR-0018. Lock 3 sub-variants:

| Code | Variant | Stage | Khi nào |
|---|---|---|---|
| `E2208.PreV06Reader` | Reader pre-v0.6 sees `cap_count > 0` in `.tripack` ABI metadata | Step 6a load-time | Forward-compat refusal — pre-v0.6 binary can't validate caps |
| `E2208.ManifestParse` | `triet.package` source file syntax error | Pre-build (compiler reads source) | Whitelist parser refuse-to-load |
| `E2208.CapabilityDivergence` | `triet.package` declares `requires` lines nhưng `.tripack` `caps_count = 0` (writer bug) | Step 6a load-time | Writer/reader divergence detection |

**Stage table:**

- Sub-variant 1 fires ở loader **Step 6a** (after dep resolve, before policy hook). Refuse entire link.
- Sub-variant 2 fires **pre-build** (compiler reading source before emitting `.tripack`). Refuse compilation.
- Sub-variant 3 fires ở loader **Step 6a**. Refuse entire link.

**Diagnostic format:** miette with primary span on `.tripack` byte offset (sub-variant 1, 3) hoặc `triet.package:line:col` (sub-variant 2). Format mirrors §3 E2205 conventions.

**Not E2208** (already covered by other codes):
- E2202 `UnresolvedCapabilityPath` (ADR-0016 §6) — cap path không match dep export. Fires ở Step 6a but uses E2202.
- E2203 `CapabilityRefused` — root manifest refuses. Fires ở Step 6a after structural validation passes.
- E2205.<sub> — policy hook errors. Fires ở Step 6b.

## §6 — `CapabilityClaim` Rust struct shape (replace placeholder)

Current placeholder at [`crates/triet-pack/src/types.rs:272-277`](../../crates/triet-pack/src/types.rs):

```rust
pub struct Capability {
    pub name: String,  // placeholder, ADR-0016 picks shape
}
```

Replace với (locked by ADR-0018):

```rust
pub struct CapabilityClaim {
    pub cap_path: AbsolutePath,    // ADR-0005 path type
    pub level: CapabilityLevel,
}

pub enum CapabilityLevel {
    Grant,    // Trit::Positive  (+1)  → u8 0x02
    Ambient,  // Trit::Zero      ( 0)  → u8 0x01
    Deny,     // Trit::Negative  (-1)  → u8 0x00
    Defer,    // Trilean::Unknown      → u8 0x03
}
```

**Rename** `Capability` → `CapabilityClaim` cho clarity (avoid confusion với generic "capability" concept). `AbiMetadata.caps: Vec<Capability>` → `caps: Vec<CapabilityClaim>`. Breaking change ở Rust API, nhưng caps slot luôn empty ở v0.5 → zero impact on existing test fixtures.

Wire encoding (ABI caps section binary format) unchanged from [ADR-0016 §4](0016-capability-type-system.md): `cap_count` varint + per-entry `(namespace_path: length-prefixed UTF-8, level: u8, reserved: u8)`. Sort canonical by `namespace_path`.

## Hệ quả

### Cho ADR-0016 — closes §6 E22XX namespace

Sau ADR-0018, E22XX namespace fully populated: E2200–E2204, E2205 (+ 6 sub-variants ADR-0017), E2206–E2207, E2208 (+ 3 sub-variants ADR-0018). Không còn reserved slot trong v0.6 namespace.

### Cho ADR-0017 — closes deferred sections

ADR-0017 §4 pseudo-code `prompt_user(req)` → §4 mock locked đầy đủ. Addendum §A whitelist rules → §3 implementation strategy locked. Addendum §B `/dev/tty` direction → §4 lock decisions table áp dụng.

### Cho [`triet-pack`](../../crates/triet-pack) crate

Implementation targets (v0.6.4+ sub-tasks):
- `crates/triet-pack/src/types.rs`: rename `Capability` → `CapabilityClaim`, add `CapabilityLevel` enum.
- `crates/triet-pack/src/serde.rs`: extend writer/reader cho non-empty caps section.
- New `crates/triet-pack/src/package_manifest.rs`: `triet.package` parser + writer (mirror `lockfile.rs` pattern).
- New `crates/triet-pack/src/policy.rs`: `triet.policy` parser + writer (mirror `lockfile.rs` pattern).
- New `crates/triet-pack/src/capability_resolver.rs`: PolicyCache + ADR-0017 §4 algorithm + ADR-0018 §2 loader steps 6a/6b.

### Cho `triet-cli`

New subcommands (v0.6.4+):
- `triet pack init` — emit boilerplate `triet.package`
- `triet policy show` — render `triet.policy` rules table
- `triet policy add <cap> <origin> <decision>` — append rule atomically
- TTY prompt machinery wired into runtime link path

### Cho ABI metadata ([ADR-0011](0011-abi-metadata-format.md))

Không đổi binary format. `abi_version` giữ `v=2`. `caps section` populate per ADR-0016 §4 encoding (already locked).

### Cho IR ([ADR-0007](0007-ir-design.md)) / `.triv` wire format

Không đổi. Cap check fires ở loader stage, không IR opcode mới.

### Cho v0.7 self-hosting

Self-hosted parser cho `triet.package` + `triet.policy` phải emit byte-identical errors với Rust impl per §3 format table. Critical for bit-identical bootstrap (ROADMAP §v0.7 gate).

### Cho v0.8 concurrency

Eager mode cache fully warm before main() → v0.8 actor threads share immutable `PolicyCache` snapshot. Thread-safety chốt ở v0.8 concurrency ADR; ADR-0018 không pre-commit lock shape.

### Cho v0.9 JIT / v2.0 AOT

Cached decision authoritative; JIT lift across cap boundary đọc cache, không re-evaluate. AOT v2.0: cache state baked vào binary header is REJECTED — cache initialized empty per process (deployment-specific, không AOT-bake).

## Không làm

- **Lazy cap resolution** — defer post-v0.7 nếu hot-path profiling demand. Eager đủ cho v0.6.
- **Source manifest implementation** — ADR-0018 lock grammar; writer/reader/CLI implementation = v0.6.4+ sub-tasks trong TODO.md. Split design vs implementation cadence.
- **Multi-language manifest** — English only v0.6; i18n defer indefinitely (security context disallows ambiguity).
- **Capability claim composition** (claim references another claim) — KHÔNG ở v0.6; mỗi entry self-contained.
- **Versioning `triet.package` format** — `format_version 1` đủ; future ADR bump nếu cần additive field.
- **Persistent session cache across processes** — cache discarded process exit per ADR-0017 §5 monotonicity.
- **TTY prompt timeout** — sync, no timeout per ADR-0017 §8 known limit.
- **Hash truncation anywhere in UI** — full 64 hex chars always. Short-SHA = collision attack surface.
- **Box-drawing chars ở core security display** — ASCII fallback markers (`!!` not `⚠`). Security message must render guaranteed.
- **Auto-generate `triet.policy` rules** từ dep tree heuristics — refuse over guess. User must explicitly choose `G`/`D` ở prompt OR write rule manually.

## Prior art

- **[`Cargo.toml`](https://doc.rust-lang.org/cargo/reference/manifest.html)** — Rust source manifest. Inspires `triet.package` field shape (name, version, deps); reject TOML format vì hand-rolled precedent stronger.
- **[`go.mod` + `go.sum`](https://go.dev/ref/mod)** — hand-rolled module file with hash pins. Closer precedent — line format, no nested syntax, hash-as-trust-anchor. Direct inspiration cho `triet.package`.
- **[npm `package.json` + `package-lock.json`](https://docs.npmjs.com/cli/v9/configuring-npm/package-json)** — JSON manifest. Reject vì JSON syntax invites silent typing errors (string-vs-number, missing-trailing-comma rendering ambiguous).
- **[Android `<uses-permission>` + runtime grant dialog](https://developer.android.com/guide/topics/manifest/uses-permission-element)** — Manifest declares + OS prompts at runtime. Direct inspiration cho ADR-0018 §4 mock UI structure.
- **`sudo(8)` AUTHENTICATION** — `/dev/tty` direct read, terminal-bound prompt. Direct precedent for ADR-0018 §4 lock decisions (input/output source).
- **`apt install` Y/N prompt** — eager confirmation before action. Direct precedent for §2 eager mode UX.
- **[Nix `trusted-public-keys` + signature verify](https://nixos.org/manual/nix/stable/installation/multi-user.html)** — CAS hash anti-typosquatting. Inspires §4 anti-typosquatting display (full hash + lockfile cross-check).

**Anti-prior-art:**

- **`npm install` legacy auto-resolve** — silent transitive grants → supply chain CVEs. ADR-0018 explicitly opposite: eager prompt + refuse-over-guess.
- **Java `policy` files với grant blocks** — verbose nested syntax + JVM-internal semantics → barely used in practice. ADR-0018 flat line format, security-front-and-center.
- **Short SHA in package UIs** (Git, GitHub PR refs) — collision attack surface. ADR-0018 §4 lock: never truncate hash in security context.

## Tham chiếu

- [VISION §3.5 + §5 + §6](../../VISION.md)
- [SPEC §1.3 (identifiers), §10 (reserved roots)](../../SPEC.md)
- [ADR-0005 — Module system (AbsolutePath)](0005-module-system.md)
- [ADR-0011 §4 (dep table), §5 (caps section), §8 (linker workflow)](0011-abi-metadata-format.md)
- [ADR-0013 — Semver linking policy (E23XX series)](0013-semver-linking-policy.md)
- [ADR-0014 §4 (impl_hash unforgeable trust anchor)](0014-hash-scheme-refinement.md)
- [ADR-0015 §6 (hand-rolled file format precedent — `triet.lock`)](0015-package-store-layout.md)
- [ADR-0016 §1 (manifest pseudo-syntax), §4 (caps section encoding), §6 (E22XX namespace)](0016-capability-type-system.md)
- [ADR-0017 §3 (triet.policy grammar), §4 (resolution algorithm), §5 (monotonicity), Addendum §A (parser whitelist), Addendum §B (/dev/tty)](0017-trilean-policy-hook.md)
- TODO.md v0.6.3 anti-typosquatting constraint (commit `dd6b2f4`)
- [`crates/triet-pack/src/types.rs:272-277`](../../crates/triet-pack/src/types.rs) — placeholder being replaced
- [`crates/triet-pack/src/lockfile.rs`](../../crates/triet-pack/src/lockfile.rs) — hand-rolled parser precedent to mirror
- [ROADMAP §v0.6](../../ROADMAP.md)

---

## Addendum — v0.6.x.review (pre-v0.7 audit)

Audit window post-decision, mirror precedent [ADR-0015 Addendum](0015-package-store-layout.md#addendum--v05xreview-pre-v06-audit). Cả 3 ADRs (0016, 0017, 0018) được verify; findings anchor ở đây vì 0018 là capstone integrative của phase v0.6.

### Test coverage scorecard

| Original gap | Layer | Status | Anchor |
|---|---|---|---|
| Monotonicity replay assertion | resolver | Partial → strengthened | `second_resolve_same_key_replays_from_cache` (replay only) + new `monotonicity_holds_under_policy_mutation` (mutation invariant) |
| `upsert_rule` + `save` round-trip | policy | Real gap → filled | new `upsert_then_save_round_trip` |
| Multi-dep aggregation determinism | linker | Partial → strengthened | `multiple_dep_requesters_aggregated` (alphabetical insertion) + new `requesters_sorted_when_inserted_out_of_order` |
| E2204 duplicate cap claim | manifest | Already covered | `rejects_duplicate_requires` |
| Unused `grant` claim semantic | typecheck | Already covered | `orphan_claim_without_import_passes` |
| `prompt_loop` retry-on-invalid | tty | Already covered | `prompt_loop_reprompts_on_invalid_input` |
| `?` ShowHashHelp branch | tty | Already covered | `prompt_loop_reprompts_on_hash_help_then_terminal` |
| `default prompt` rejection message | policy | Already covered | `rejects_default_prompt` (reason contains "static") |
| Cross-stage propagation | pipeline | Not a v0.6 gap | CLI orchestration deferred to v0.7 per SPEC §0.7 |
| CRLF/BOM positional contract | strict_parser | Partial → strengthened | basic `rejects_bom`/`rejects_crlf` + new `empty_file_succeeds_with_zero_callbacks` + `bom_mid_file_classifies_as_non_ascii_not_bom` + `cr_mid_line_classifies_as_non_ascii_not_crlf` |

Audit listed 10 gaps; 5 already covered, 1 deferred (CLI wiring → v0.7), 4 partial/real → 6 net-new tests across review.1 (`d56c518`) + review.2 (`b6bde0c`). Workspace: 1079 → 1085 tests, clippy `-D warnings` clean.

### Monotonicity invariant — pinned under PolicyRules mutation

ADR-0017 §5 quy định "knowledge growth doesn't flip". v0.6.9 implementation honors this (cache lookup precedes rule lookup), nhưng existing test chỉ prove replay, không exercise mutation step. v0.6.x.review.1 thêm assertion: flip rule `+1 → -1` mid-session → cached `Positive` survives + source=Cache. Commit `d56c518`.

### `upsert_rule` + `save` insight — in-memory ≠ disk byte-equal

Test surfaced contract subtle: `upsert_rule` appends to `Vec` (insertion order); `save` canonicalize sort-by-cap-path → in-memory state NOT byte-equal disk state. User-facing guarantee: rule survives round-trip. Test cũng assert canonical form is fixed point across re-save. Important context cho DevTtyPrompt G/D path. Commit `d56c518`.

### Strict parser positional contracts

`strict_parser.rs` phân biệt positional violations (Bom = file-start; Crlf = line-trailing) vs generic NonAscii. Existing tests cover positive cases only; v0.6.x.review.2 pin *negative* cases để prevent future refactor conflate distinct violation kinds. Commit `b6bde0c`.
