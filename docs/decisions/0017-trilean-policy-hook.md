# ADR 0017 — Trilean policy hook protocol (`triet.policy` + TTY fallback)

**Trạng thái:** Quyết định. Áp dụng cho v0.6 Capability System runtime resolution. Phụ thuộc [ADR-0016 §3](0016-capability-type-system.md) (Defer slot trong CapabilityLevel) và [ADR-0016 §8](0016-capability-type-system.md) (ResolutionOrigin dispatch). Lấp đầy `E2205` reserved trong [ADR-0016 §6](0016-capability-type-system.md). Không bump `abi_version`, không đổi `.triv` wire format, không đổi IR shape.

**Issue:** ADR-0016 lock 4 trạng thái CapabilityLevel: `Grant (+1)`, `Ambient (0)`, `Deny (-1)`, **`Defer (Trilean::Unknown)`**. Ba trạng thái đầu resolve hoàn toàn ở compile + link time. Trạng thái thứ tư — `Defer` — đẩy quyết định sang runtime, theo lời hứa [VISION §3.5.2](../../VISION.md): *"capability có thể là `Trilean::Unknown` → giải quyết runtime bởi user/policy"*.

ADR-0016 "Cho ADR-0017" list 4 câu phải chốt:

1. **Hook sống ở đâu?** Declarative config / Triết function callback / interactive TTY?
2. **Protocol gọi hook** — input record + output type?
3. **Cache scope** + capability monotonicity invariant?
4. **Failure mode** khi policy crash / unreachable / config missing?

[ADR-0015 Addendum](0015-package-store-layout.md#addendum--v05xreview-pre-v06-audit) đã hint mapping cụ thể: *"chỉ `Lockfile` được auto-trust, `IfacePin` cần admin grant, `Fresh` deps phải hỏi user"*. ADR-0017 hoàn thiện cơ chế.

## Quyết định

### 1. Phương án — Hybrid: `triet.policy` rules first, TTY prompt fallback, headless = fail-closed

Hook resolution ở runtime đi qua 3 bước, theo thứ tự:

```
┌─────────────────────────────────────────────────────────────┐
│  Bước 1: Lookup cache                                       │
│    Hit  → return cached Trit (O(1))                         │
│    Miss → bước 2                                            │
├─────────────────────────────────────────────────────────────┤
│  Bước 2: Match rule trong triet.policy                      │
│    Match → resolve theo decision token, cache, return        │
│    No match → bước 3                                        │
├─────────────────────────────────────────────────────────────┤
│  Bước 3: Fallback                                           │
│    TTY available     → interactive prompt → cache → return  │
│    Headless (no TTY) → E2205.NonTTYDefer → Deny + cache     │
│    Config absent     → default rule (`default -1` implicit) │
└─────────────────────────────────────────────────────────────┘
```

Ba phương án bị từ chối:

| Bị từ chối | Lý do |
|---|---|
| **Static config only** (rules without prompt) | Mất hint ADR-0015 Addendum *"Fresh deps phải hỏi user"* — không có TTY fallback thì Fresh dep mặc định Deny, mọi dep mới broke build. |
| **Triết function callback** (manifest `policy_hook: usr.myapp.policy.decide`) | Bootstrap cycle: hook function ở `usr.*` cần cap context → trigger hook → ... Phải khoá ngược vào ADR-0016 §5 enforcement rules. Hostile policy code = attack surface mới ở loader stage. Defer additively post-v0.7. |
| **Interactive prompt only** (zero config) | CI/headless mặc định Deny mọi Defer → break automation. Không match webapp dev mental model (Spring/nginx có config trước, prompt sau). |

Phương án 4 (hybrid) **map 1-1** với ADR-0015 Addendum hint qua rule keys có dimension `origin`:

```
rule * lockfile +1        # auto-trust lockfile-pinned deps
rule * ifacepin prompt    # iface hash pin = admin confirm
rule * fresh    prompt    # newly added dep = hỏi user
```

### 2. PolicyRequest + PolicyDecision — frozen shape ở v0.6

**Hook input** (record passed vào resolution machinery; mặc dù v0.6 không expose ra user code, shape vẫn frozen để v0.7+ Phương án 2 callback build trên):

```text
PolicyRequest {
    cap_path:       String,            // AbsolutePath module, e.g. "sys.io"
    requester_pkg:  String,            // "myapp@0.1.0" — pkg requesting access
    dep_chain:      List<String>,      // ["myapp", "libdns", "libtls"] — transitive
    origin:         ResolutionOrigin,  // Lockfile | IfacePin | Fresh
}
```

**Hook output** — `PolicyDecision = Result<Trit, PolicyError>`:

| Outcome | Hậu quả runtime | Diagnostic |
|---|---|---|
| `Ok(Trit::Positive)` | Grant — cache, allow current + all future calls (cap_path, requester_pkg) | None |
| `Ok(Trit::Zero)` | Abstain — cache as Deny, allow re-eval next session | Info: "policy abstained" |
| `Ok(Trit::Negative)` | Deny — cache, refuse current + all future calls | None |
| `Err(PolicyError)` | Fail-closed Deny — cache với reason | E2205.<sub> |

3 Trit + 1 Err = 4 outcomes preserved bản sắc tam phân ở runtime. `Trit::Zero` (Abstain) vs `Trit::Negative` (Deny) khác **diagnostic only**: Abstain = "policy không quyết được"; Deny = "policy chủ động refuse". Audit log distinguish — quan trọng cho post-incident review.

### 3. `triet.policy` file format (hand-rolled, sort canonical)

Tên file: `triet.policy` ở project root (song song `triet.lock`). Hand-rolled line format, mirror precedent [ADR-0015 §6](0015-package-store-layout.md) — không serde dep, diff-friendly.

```text
format_version 1
rule <cap_path>      <origin>      <decision>
rule <cap_path>      <origin>      <decision>
...
default <decision>
```

**Field rules:**

- `cap_path`: exact `AbsolutePath` (e.g. `sys.io`, `dev.disk`, `sys.net.dns`). **KHÔNG glob** ở v0.6 — match exact. Blanket policies dùng `default` line.
- `origin`: `lockfile` | `ifacepin` | `fresh` | `*` (wildcard match any origin). Đây là **chỗ duy nhất** wildcard được phép — vì `ResolutionOrigin` đã là enum đóng, `*` không vi phạm "Explicit > implicit".
- `decision`: `+1` | `0` | `-1` | `prompt`. Bốn token, không có alias (e.g. `grant` không được — keep parser dead simple).
- `default <decision>`: zero hoặc một line. Vắng mặt → implicit `default -1`. `decision` ở default không được là `prompt` (default phải static).

**Canonical encoding** (cho hash stability nếu future version pin policy file):

- Sort by `(cap_path ASC, origin ASC)`. `origin` ordering: `lockfile < ifacepin < fresh < *`.
- Whitespace giữa fields: 1+ space hoặc tab; ignored ở parser, normalized ở writer.
- Comments: line starts với `#` ignored. KHÔNG inline comments (parser drops a line if any `#` outside string).
- Encoding: UTF-8, LF line endings (CRLF rejected — match `triet.lock`).
- Duplicate `(cap_path, origin)` tuple → **E2205.RuleConflict** (refuse-to-load). KHÔNG có last-wins / merge semantics.

**Example:**

```text
# triet.policy v1
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

Pseudo-code (sequential, single-threaded — v0.6 không có concurrency):

```
function resolve(req: PolicyRequest) -> Trit {
    // Bước 1: cache lookup
    let key = (req.cap_path, req.requester_pkg)
    if cache.contains(key) {
        return cache[key].outcome
    }

    // Bước 2: try triet.policy rules
    if policy_file_exists {
        let rules = load_and_parse(triet.policy)  // memoized after first call
        let matched = rules.find_exact(req.cap_path, req.origin)
                       or rules.find_exact(req.cap_path, Wildcard)
        if matched != null {
            let outcome = match matched.decision {
                +1     -> +1
                0      -> 0
                -1     -> -1
                prompt -> goto bước 3 (force prompt)
            }
            cache[key] = CachedDecision { outcome, source: ConfigRule }
            return outcome
        }
        // No rule matched: use `default`
        let default_decision = rules.default or -1
        cache[key] = CachedDecision { outcome: default_decision, source: Default }
        return default_decision
    }

    // Bước 3: fallback (no policy file OR rule said `prompt`)
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

**Match precedence trong rules** (đơn giản ở v0.6):

1. Rule với origin **exact match** (lockfile/ifacepin/fresh) wins over `*`.
2. Same precedence cấp đó → impossible vì duplicate (path, origin) → E2205.RuleConflict.

**TTY prompt UX** (chỉ định ở v0.6, implementation chi tiết ở ADR-0018):

```text
[triet] Capability decision required
  Package:        myapp@0.1.0
  Requesting:     sys.net.dns
  Dep chain:      myapp → libdns@1.2.3 → libtls@0.4.1
  Origin:         Fresh (newly resolved, not in triet.lock)

  [g] grant once (this session)
  [d] deny once  (this session)
  [G] grant permanent (write rule to triet.policy)
  [D] deny permanent  (write rule to triet.policy)
  [?] explain

  choice >
```

`G`/`D` append rule vào `triet.policy` trước khi cache + return. `g`/`d` chỉ cache session. Implementation cụ thể ở ADR-0018.

### 5. Cache scope + capability monotonicity invariant

**Cache key:** `(cap_path: String, requester_pkg: String)`.

Không bao gồm `origin` trong key vì:
- `origin` đã được resolver quyết định trước khi reaches hook — fixed per-session.
- Same (path, pkg) cùng session = same origin → bao gồm origin redundant.

**Lifetime:** Process lifetime. Cache discarded khi process exit.

**Monotonicity invariant** (khoá từ [ADR-0016 "Không làm"](0016-capability-type-system.md)):

> *Once cached, decision frozen for session. Capability không có hot-reload.*

Hệ quả:
- Re-evaluation triggers ONLY khi (path, pkg) chưa có entry.
- Modify `triet.policy` mid-session → KHÔNG affect already-cached decisions. Next process start = re-read.
- User chọn `G`/`D` ở prompt → file update + cache update atomic. Next process start sẽ thấy rule mới ngay từ Bước 2.

**Hot path optimization:** Cap check fire mỗi cross-namespace call. Cache lookup O(1) (HashMap by `(String, String)` key). Hook execution chỉ chạy 1 lần per unique key per session.

### 6. E2205 sub-variants — finalize

| Code | Variant | Stage | Outcome runtime |
|---|---|---|---|
| `E2205.ConfigParse` | `triet.policy` syntax invalid | Load-time | Refuse-to-load entire binary, abort |
| `E2205.RuleConflict` | Duplicate `(path, origin)` trong rules | Load-time | Refuse-to-load entire binary, abort |
| `E2205.UnknownOrigin` | `origin` field ∉ {lockfile, ifacepin, fresh, \*} | Load-time | Refuse-to-load entire binary, abort |
| `E2205.UnknownDecision` | `decision` field ∉ {+1, 0, -1, prompt} | Load-time | Refuse-to-load entire binary, abort |
| `E2205.NonTTYDefer` | Defer reached + no rule match + headless | First-call | Fail-closed Deny + cached + diagnostic |
| `E2205.PromptCrash` | TTY closed mid-prompt / I/O error | First-call | Fail-closed Deny + cached + diagnostic |

Load-time errors (ConfigParse/RuleConflict/UnknownOrigin/UnknownDecision): refuse-to-load **toàn bộ binary**. Reason: `triet.policy` corruption = không thể trust bất kỳ Defer resolution → safer to abort hơn là partial-run.

First-call errors (NonTTYDefer/PromptCrash): per-key Deny + diagnostic. Process tiếp tục — vì chỉ ảnh hưởng cap chưa resolve được; cap khác có thể đã grant ở Bước 2.

### 7. Headless vs TTY detection

Loader check `isatty(stderr)` ở first Defer reached. Cache TTY-availability cho remaining session — KHÔNG re-check (avoid race với external `stty` change).

CI environment vars (`CI=true`, `GITHUB_ACTIONS=true`, ...) **không** được parse — chỉ trust `isatty`. Reason: env vars unreliable cross-platform; `isatty` là POSIX standard.

`--non-interactive` CLI flag (future, defer ADR-0018) sẽ force headless mode bất chấp TTY — useful cho script chạy attended.

### 8. v0.6 known limits

ADR-0017 cố ý KHÔNG chốt các điểm sau, để phase sau lấp:

- **Timeout enforcement:** Sync prompt, không timeout. Hostile prompt (e.g. malicious `stty` consumes input) có thể hang loader. Defer v0.8 actor model.
- **Triết function callback** (Phương án 2 đã reject ở §1): defer additively post-v0.7. Thêm rule type mới (`rule X * call usr.myapp.policy.decide`) sẽ extend `triet.policy` v=1 → v=2 với additive field.
- **Cross-process policy daemon:** policy file local-only. Distributed policy (system-wide cap server) defer v1.0+.
- **Persistent cache across sessions:** không persist — process exit = cache gone. User pin decisions bằng cách chọn `G`/`D` (write rule vào file).
- **Per-thread cache:** v0.6 single-threaded VM. v0.8 actor model lands → cache thread-safety chốt ở ADR concurrency.
- **Glob trong `cap_path`:** không cho phép. `default` line cover blanket case. Glob defer (probably never — vi phạm "Explicit > implicit").

## Hệ quả

### Cho ADR-0016 — populate Defer slot

ADR-0016 §3 đặt 1 trong 4 trạng thái CapabilityLevel là `Defer (Trilean::Unknown)`. ADR-0017 cung cấp resolution machinery → `Defer` không còn là leaf-pending; runtime guaranteed terminates với `Trit` final (hoặc explicit fail-closed Deny + diagnostic).

### Cho ADR-0016 §6 — E2205 fully populated

`E2205` đã reserved trong ADR-0016 §6 với note *"reserved cho ADR-0017"*. ADR-0017 chốt 6 sub-variants. ADR-0016 không cần re-issue — sub-variants là extension dưới existing slot.

### Cho ADR-0018 (loader semantics) — TBD

ADR-0018 phải chốt:
- Loader stage cụ thể nơi `resolve()` fires (link-time pre-cache vs lazy first-call).
- TTY prompt UX implementation chi tiết (terminal escape sequences, color, multi-line render).
- `--non-interactive` CLI flag spec.
- `triet.policy` reader implementation (line tokenizer, error span reporting cho miette).
- Manifest source syntax cho `requires:` block (ADR-0016 §1 dùng pseudo-syntax).

ADR-0017 chỉ commit: resolution **xảy ra** với contract ở §4 algorithm; lifecycle/UX detail = ADR-0018.

### Cho v0.5 hash scheme

`triet.policy` **không tham gia** vào `iface_hash` hay `impl_hash` của package. Policy là deployment-environment concern, không phải package-content. Hai user chạy cùng `.tripack` với different `triet.policy` → cùng hash, khác behavior runtime — đúng spec.

### Cho ABI metadata ([ADR-0011](0011-abi-metadata-format.md))

Không đổi. `caps section` chỉ encode level `Defer (0x03)`; runtime resolution machinery sống ở loader, không trong pack metadata.

### Cho IR ([ADR-0007](0007-ir-design.md)) / `.triv` wire format

Không đổi. Cap check site fire ở cross-module call dispatch — IR đã preserve `AbsolutePath` ([ADR-0007 §6.7](0007-ir-design.md)). Cache lookup là Rust-side data structure trong runtime, không IR opcode mới.

### Cho v0.7 self-hosting

Triết-rewritten compiler phải honor `triet.policy` parsing semantics + resolution algorithm. Test contract: bootstrap chain output must match Rust impl byte-identical cho `triet.policy` round-trip.

### Cho v0.8 concurrency

Cache thread-safety = open question đến v0.8. Hint sẵn: `Arc<RwLock<HashMap<(String, String), CachedDecision>>>` Rust-side để actor messages có thể share immutable view. ADR-0017 KHÔNG pre-commit shape — wait v0.8 actor ADR.

### Cho v0.9 JIT / v2.0 AOT

JIT lift function across cap boundary → check at lift-time (defer ADR-0018). Cached decision vẫn authoritative — lift không re-evaluate.

AOT v2.0 baked-binary: cache initialized empty mỗi process start. `triet.policy` loaded same way — không AOT-bake (deployment-specific).

## Không làm

- **Triết function callback** (Phương án 2 from proposal) — defer post-v0.7 additive. Bootstrap risk + sandbox concern + v0.6 VM hot-path performance.
- **Glob trong `cap_path`** — vi phạm "Explicit > implicit". `default` line đủ cho blanket.
- **Last-wins / merge cho duplicate rules** — refuse over guess. Duplicate = E2205.RuleConflict.
- **Inline comments** trong `triet.policy` — `#` chỉ ở đầu line. Mirror `triet.lock`.
- **CRLF line endings** — LF only.
- **TOML / YAML / JSON syntax** — hand-rolled mirror `triet.lock` precedent ([ADR-0015 §6](0015-package-store-layout.md)). Không serde dep.
- **Timeout enforcement** ở v0.6 — hostile prompt có thể hang. Defer v0.8.
- **Cross-process policy daemon** — local file only. Distributed defer v1.0+.
- **Persistent session cache** — process exit = cache gone. Decisions persist qua user choosing `G`/`D` (write to file).
- **Env-var-based headless detection** (`CI=true` etc.) — chỉ `isatty(stderr)`. Env unreliable.
- **Auto-write `triet.policy`** trên Deny — only on user explicit `G`/`D`. Avoid silent grant accumulation.
- **Re-eval khi config changes mid-session** — monotonicity invariant. Restart = new chance.

## Prior art

- **[nginx `location` rules](https://nginx.org/en/docs/http/ngx_http_core_module.html#location)** — declarative rule matching, ordered fallthrough. Triết khác: sort canonical thay vì source order (diff-friendly).
- **[Android runtime permissions](https://developer.android.com/training/permissions/requesting)** — manifest pre-declare + OS prompts ở runtime nếu chưa grant. Đây là mental model gần nhất với Phương án 4. Khác: Android prompt là OS-level; Triết prompt là loader-level (per-process).
- **[`sudo` / `polkit`](https://www.freedesktop.org/wiki/Software/polkit/)** — rule-based + interactive escalation. Polkit's `.rules` file là JavaScript callback — Triết reject vì code execution.
- **[OAuth consent screen](https://datatracker.ietf.org/doc/html/rfc6749#section-4.1.1)** — interactive grant flow với scoped tokens. Inspires per-(path, pkg) cache shape.
- **[Spring Security `WebSecurityConfigurerAdapter`](https://docs.spring.io/spring-security/site/docs/current/api/org/springframework/security/config/annotation/web/configuration/WebSecurityConfigurerAdapter.html)** — code-driven policy. Defer (Phương án 2) post-v0.7.
- **[E language vat](http://www.erights.org/elib/distrib/vat.html)** — defer-to-vat for cross-vat capability resolution. Inspires Trilean::Unknown defer pattern (ADR-0016 §3 đã ack).

**Anti-prior-art:**

- **Java SecurityManager** (deprecated JDK 17) — code-based, brittle stack inspection. Triết tránh bằng declarative + interactive.
- **Polkit JS rules** — code execution ở privileged context; CVE history. Triết tránh bằng data-only `triet.policy`.
- **POSIX setuid + `cap_set_file`** — runtime cap với confused-deputy CVE history. Triết tránh bằng compile-time + link-time + load-time enforcement, runtime hook chỉ explicit Defer.

## Tham chiếu

- [VISION §3.5 — OS-Native Capability Namespaces](../../VISION.md)
- [VISION §5 — Bản sắc Triết (Trit-level + Łukasiewicz capability)](../../VISION.md)
- [VISION §6 — Refuse over guess, Explicit > implicit](../../VISION.md)
- [SPEC §1.5.2 — Trilean type (`Unknown`)](../../SPEC.md)
- [ADR-0011 §5 — `caps section` ABI metadata](0011-abi-metadata-format.md)
- [ADR-0015 §6 — `triet.lock` hand-rolled format precedent](0015-package-store-layout.md)
- [ADR-0015 Addendum — ResolutionOrigin 3-state, dispatch hint](0015-package-store-layout.md#addendum--v05xreview-pre-v06-audit)
- [ADR-0016 §3 — Defer slot trong CapabilityLevel](0016-capability-type-system.md)
- [ADR-0016 §6 — E22XX namespace, E2205 reserved](0016-capability-type-system.md)
- [ADR-0016 §8 — ResolutionOrigin dispatch slot](0016-capability-type-system.md)
- ADR-0018 — Capability loader semantics (TBD, v0.6.3)
- [ROADMAP §v0.6 — Capability System](../../ROADMAP.md)
- [ROADMAP §v0.8 — Concurrency Model](../../ROADMAP.md) (future: timeout + thread-safe cache)
