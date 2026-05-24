# ADR 0016 — Capability type system (Trit-level grant/deny/ambient + Ł3 Unknown)

**Trạng thái:** Quyết định. Áp dụng cho v0.6 Capability System và mọi tool đọc/viết `caps section` trong ABI metadata kể từ v0.6. Hoàn thiện slot placeholder của [ADR-0011 §5](0011-abi-metadata-format.md); **không bump** `abi_version` (giữ nguyên `v=2` từ [ADR-0014](0014-hash-scheme-refinement.md)). Không thay đổi IR shape (reuse namespace tag đã có ở [ADR-0007 §3.4 + §6.7](0007-ir-design.md)).

**Issue:** [VISION §3.5](../../VISION.md) đặt trụ cột số 5 — *"OS-Native Capability Namespaces"* — với 2 trong 3 điểm bản sắc Triết ([VISION §5](../../VISION.md)):

1. **Trit-level capability** — `Trit ∈ {-1, 0, +1}` là level, không phải boolean `enum { Allow, Deny }`.
2. **Łukasiewicz capability checking** — `Trilean::Unknown` defer-to-runtime, không cần bolt-on policy engine.

v0.6 phải chọn shape kỹ thuật. Ba câu hỏi ADR phải khoá:

1. **Capability sống ở đâu?** Giá trị runtime (token argument) / annotation function-level / thuộc tính namespace?
2. **Granularity** — package / module / function?
3. **Encoding** — slot `caps section` ([ADR-0011 §5](0011-abi-metadata-format.md)) phải finalize binary format.

Quyết định ở đây là **prerequisite** cho [ADR-0017](0017-trilean-policy-hook.md) (runtime policy protocol — TBD) và [ADR-0018](0018-capability-loader-semantics.md) (loader refuse-to-load — TBD). 3 ADR cùng phase v0.6.

## Quyết định

### 1. Capability sống ở **namespace + manifest**, không phải runtime value

Capability là **thuộc tính của module path** (`AbsolutePath` từ [triet-modules](../../crates/triet-modules)), declared ở `.khi` manifest của package, không phải giá trị runtime pass qua function argument và không phải annotation per-function.

```triet
// Source code SẠCH — không thread token, không annotate effect:
function main() {
    let content = sys.io.read_file("/etc/hosts")
    let buf = dev.disk.raw_read(0x1000, 512)
}
```

```text
// Manifest (.tripack caps section) là chỗ declaration:
package myapp 0.1.0
requires:
    sys.io       = +1   // Grant — explicit
    dev.disk     = -1   // Deny  — refuse dù dep transitively tries
    sys.net.dns  = unknown   // Defer to runtime policy hook (Ł3)
    usr.somelib  = +1   // Cross-application boundary
```

Ba phương án bị từ chối:

| Bị từ chối | Lý do (neo vào doc) |
|---|---|
| **A — Capability token (Pony/Roc)** | `caps section` ADR-0011 §5 thành dead slot. Mọi function chạm syscall phải nhận `Capability<X>` arg → verbose, vi phạm "AI-first stays" ([VISION §6](../../VISION.md)) vì sinh code phình code. |
| **B — Effect annotation (Koka/F\*/checked exception)** | Effect propagate per-function → `iface_hash` đổi mỗi lần touch một function trong call chain → break compile-time scaling ([ADR-0014 §1](0014-hash-scheme-refinement.md) đã hứa term-level hash stability). Java checked-exception đã chứng minh ergonomics fail. |
| **C — Namespace + manifest** | ✅ Chọn. Lý do từng điểm ở §10 (Hệ quả). |

### 2. Granularity — module level, không wildcard, không function level

Mỗi entry trong `caps section` khoá **một `AbsolutePath` module**. Không hỗ trợ:

- **Wildcard** (`sys.* = +1`): vi phạm "Explicit > implicit" ([VISION §6](../../VISION.md)). Muốn 3 modules trong `sys.*` → list 3 entries.
- **Path inheritance** (`sys.io = +1` không tự động grant `sys.io.async`): mỗi module path là một declaration độc lập. Linker không suy luận parent-child relations.
- **Function-level** (`sys.io.read_file = +1` nhưng `sys.io.write_file = -1`): defer post-v1.0. Muốn tách thì stdlib author tách module (`sys.io.read` vs `sys.io.write`).

**Lý do chọn module-level:**
- Khớp [ADR-0007 §6.7](0007-ir-design.md): IR cross-module call mang `AbsolutePath` ở scope module, không phải function. Capability check đọc cùng metadata — zero IR change.
- Match VISION §3.5 wording: trụ cột là *"capability NAMESPACES"*, không phải capability functions.
- Khớp Android `<uses-permission>` mental model (anh đã quen): app declare permission per resource, không per API call.

### 3. Capability level — `Trit` + `Trilean::Unknown`

Mỗi entry trong `caps section` mang một **CapabilityLevel** với 4 trạng thái — đúng cú pháp [VISION §3.5.1](../../VISION.md) (3 Trit values) **kết hợp** [VISION §3.5.2](../../VISION.md) (Trilean defer):

| Giá trị nguồn | Tên | Ý nghĩa ở compile time | Ý nghĩa ở link time |
|---|---|---|---|
| `Trit::Positive` (`+1`) | **Grant** | Explicit cấp | Importer của namespace pass check |
| `Trit::Zero` (`0`) | **Ambient** | "Tôi không quyết — inherit từ caller" | Root package: ambient ≡ deny (không có caller). Non-root: được override bởi root package's declaration. |
| `Trit::Negative` (`-1`) | **Deny** | Explicit cấm | Refuse-to-link, **deny luôn thắng** ở cùng path (refuse over guess) |
| `Trilean::Unknown` | **Defer** | "Hỏi runtime policy" | Loader hook gọi ở runtime, return `Trit` → cache trong session. Protocol cụ thể: ADR-0017. |

**Source syntax tham khảo** (manifest chưa parse — ADR-0018 chốt syntax cụ thể):

```text
sys.io        = +1            // hoặc: grant
dev.disk      = -1            // hoặc: deny
core.fs       =  0            // hoặc: ambient
sys.net.dns   = unknown       // hoặc: defer
```

### 4. ABI encoding — finalize `caps section` (ADR-0011 §5)

`caps section` v0.6 mở rộng từ placeholder của [ADR-0011 §5](0011-abi-metadata-format.md). Format binary, canonical, sort-by-`namespace_path`:

```
cap_count: varint
for each cap entry:
    namespace_path: length-prefixed UTF-8    // AbsolutePath of module, e.g. "sys.io"
                                              //   - root MUST be one of: sys, dev, usr
                                              //   - std, core, crate, self, super → refused (E2206 — không phải cap-checked namespace)
    level: u8                                 // 0x00 = Deny    (Trit::Negative)
                                              //   0x01 = Ambient (Trit::Zero)
                                              //   0x02 = Grant   (Trit::Positive)
                                              //   0x03 = Defer   (Trilean::Unknown)
                                              //   0x04..0xFF: refused (E2207 — invalid level encoding)
    reserved: u8                              // 0x00 ở v0.6. Future use: per-cap policy ID, witness ref...
```

**Canonical rules** (strengthen [ADR-0011 §6](0011-abi-metadata-format.md) + [ADR-0014 §7](0014-hash-scheme-refinement.md) cho cấp package):

- Entries sort lexicographically theo `namespace_path` bytes.
- Path same → parse error (E2204 — duplicate cap declaration). KHÔNG có merge/last-wins rule.
- Empty section (`cap_count = 0`) = package không request capability nào → vẫn hợp lệ (leaf lib với chỉ pure logic). Pack-level `iface_hash` vẫn bao gồm trailing `cap_count = 0` bytes — hash-stable.

`abi_version` **giữ nguyên `v=2`** ([ADR-0014 §5](0014-hash-scheme-refinement.md)) — đây là việc populate slot reserved, không phải additive field mới. Reader v0.5 (không hiểu cap semantics) vẫn parse được bytes (`cap_count + entries`) và phải refuse-to-link nếu `cap_count > 0` với error E2208 (capability section present nhưng reader pre-v0.6) — implementation cụ thể ở ADR-0018.

### 5. Compile-time enforcement rules

Type checker thêm 1 pass mới sau name resolution ([triet-modules](../../crates/triet-modules)): **capability check**. Pass đọc:

- Imports của package hiện tại (`from sys.io import read_file` → required path `sys.io`).
- `caps section` của package hiện tại (claim của chính mình về what it needs).
- Root package's effective grants (chỉ áp dụng khi linking — compile chỉ verify self-claim consistency).

Rules:

1. **Tự khai báo bắt buộc:** Mọi import từ `sys.*`/`dev.*`/`usr.<other>` phải có entry tương ứng trong `caps section` của package hiện tại. Thiếu → **E2200 `MissingCapabilityClaim`**.
2. **Tự deny mâu thuẫn:** Nếu package import `sys.io.read_file` nhưng `caps section` khai `sys.io = -1` → **E2201 `SelfContradictoryCapability`** (anh đang refuse cái anh đang dùng).
3. **Namespace root scope:**
   - Import từ **same root** (intra-`usr` lib-to-lib, intra-`sys` stdlib-to-stdlib) → không cần claim (cap-check chỉ áp dụng cross-root boundaries).
   - Import từ **`std.*` hoặc `core.*`** → **ambient, không cần claim** (per [VISION §3.5](../../VISION.md): *"std mặc định ambient"*; `core.*` là foundational types — Trit/Tryte/Integer/Long/Trilean).
   - Import từ `crate.*` / `self.*` / `super.*` → intra-package, không cap concept.
4. **Path validity:**
   - Cap entry với root ∉ {sys, dev, usr} → **E2206 `InvalidCapabilityRoot`**.
   - Path không tồn tại trong any dep's exports → **E2202 `UnresolvedCapabilityPath`** (defer-detection: ở link time vì compile time chưa thấy deps).
5. **Conflict resolution at link time** (root package's manifest = authority):
   - Cho mỗi cap path P trong union(deps' claims): root manifest's level cho P quyết định.
   - Root: `+1` → pass. `-1` hoặc `0` (ambient ở root = no caller = effectively deny) → **E2203 `CapabilityRefused`**.
   - Root: `Unknown` → defer (load-time policy hook resolves; ADR-0017 chốt protocol).
   - Path không trong root manifest mà có dep claim nó → **E2200 `MissingCapabilityClaim`** ở root.

### 6. Error code namespace — `triet::capability::E22XX`

Phase v0.6 chiếm slot E2200–E2299, **không nhập** với `triet::modules::E21XX` (loader/resolver) hay `triet::semver::E23XX` (linker version/iface drift). Phân tách rõ vì 3 loại lỗi xuất hiện ở 3 stage khác nhau (resolve → compile → link).

Initial assignments (v0.6.1 — ADR-0016 land):

| Code | Tên | Stage | Khi nào |
|---|---|---|---|
| `E2200` | `MissingCapabilityClaim` | compile + link | Import dùng cap chưa declare trong manifest |
| `E2201` | `SelfContradictoryCapability` | compile | Package import path mà chính nó deny |
| `E2202` | `UnresolvedCapabilityPath` | link | Cap path không match bất kỳ dep export |
| `E2203` | `CapabilityRefused` | link | Root manifest từ chối cap mà dep request |
| `E2204` | `DuplicateCapabilityDecl` | parse | Cùng path xuất hiện ≥2 lần trong `caps section` |
| `E2205` | reserved cho ADR-0017 (policy hook protocol error) | — | — |
| `E2206` | `InvalidCapabilityRoot` | parse | Root ∉ {sys, dev, usr} |
| `E2207` | `InvalidCapabilityLevel` | parse | `level` byte ∉ {0x00..0x03} |
| `E2208` | reserved cho ADR-0018 (loader refuse-to-load) | — | — |

E2205, E2208 giữ trống cho ADR sau lấp đầy, **không pre-claim semantics**.

### 7. Root package = authority. Dep claims là requests, không phải decisions.

Khoá rule "Refuse over guess" ([VISION §6](../../VISION.md)) ở cấp link:

- Mỗi dep's `caps section` là **claim**: "tôi cần các path này để chạy". Linker đọc, không enforce trên dep tự thân.
- Root package's `caps section` là **decision**: cho cái nào, không cho cái nào. Linker enforce root decisions over toàn bộ closure.
- KHÔNG có "auto-promote": dep claim `sys.io = +1` không tự động cấp root `sys.io = +1`. Root manifest must explicitly grant.
- KHÔNG có "implicit union" (Cargo features kind): root manifest grant set không tự đóng dưới deps' transitive needs.

Hệ quả ergonomics: thêm dep mới mà dep đó request cap mới → user phải explicitly add grant vào root manifest. Đây là FEATURE, không phải bug — đảm bảo capability surface luôn audit được ở 1 chỗ.

### 8. ResolutionOrigin dispatch (placeholder hook cho ADR-0017)

[ADR-0015 Addendum](0015-package-store-layout.md#addendum--v05xreview-pre-v06-audit) thêm `ResolutionOrigin { Lockfile, IfacePin, Fresh }` cho mỗi resolved package. ADR-0017 sẽ chốt policy protocol có thể dispatch theo origin (ví dụ: chỉ `Lockfile` auto-trust grant cho `dev.*`; `Fresh` deps phải hỏi user). ADR-0016 chỉ lock **rằng** dispatch slot tồn tại — `Trilean::Unknown` resolution có quyền inspect `ResolutionOrigin` của dep request — không lock protocol chi tiết.

## Hệ quả

### Cho v0.5 hash scheme ([ADR-0014](0014-hash-scheme-refinement.md))

- `caps section` đã trong scope hash của `iface_hash_pkg` từ v0.4 (per [ADR-0014 §4](0014-hash-scheme-refinement.md): *"Rollup: BLAKE3(sorted module hashes + deps + caps)"*). v0.6 chỉ populate, **không** đổi rollup. Hai package với cap claims khác nhau → khác `iface_hash` → khác CAS address → tự nhiên không xung đột trong store.
- Empty `caps section` (`cap_count = 0`) of pre-v0.6 packs hash-stable: cùng bytes (`varint 0` = 1 byte `0x00`) → cùng hash. Backward compat tự nhiên.

### Cho ADR-0007 IR — zero change

[ADR-0007 §6.7](0007-ir-design.md) đã preserve `AbsolutePath` ở cross-module call. Capability check đọc namespace tag từ existing IR. **Không bump `.triv` wire format** (v3 từ [ADR-0010](0010-ternary-native-ir.md) + [ADR-0012](0012-witness-table-dispatch.md) giữ nguyên).

### Cho ADR-0011 ABI — populate slot, không bump

[ADR-0011 §7](0011-abi-metadata-format.md) đã hứa: *"v0.6 chỉ cần populate, không bump abi_version"*. ADR-0016 giữ lời. `abi_version = 2` ([ADR-0014 §5](0014-hash-scheme-refinement.md)) bao trùm cả v0.5 CAS Packaging và v0.6 Capability System. Reader pre-v0.6 vẫn parse được `cap_count` field; chỉ refuse khi `cap_count > 0` (E2208, ADR-0018 chốt detail).

### Cho ADR-0013 linker policy

E22XX namespace tách khỏi E23XX. Linker workflow ([ADR-0011 §8](0011-abi-metadata-format.md)) thêm bước:

```
1. Mở .tripack → đọc ABI metadata.
2. Resolve deps.
3. Version check (E23XX) → refuse/accept.        ← ADR-0013
4. Capability check (E22XX) → refuse/accept.     ← ADR-0016 (new)
5. Accept → load .triv, build symbol table.
6. Runtime/JIT: witness dispatch + cap defer hook.
```

Bước 4 chèn giữa bước 3 và 5. Diagnostic show diff của manifest cap entries (miette-style).

### Cho ADR-0017 (Trilean policy hook) — TBD

ADR-0017 phải chốt:
- Protocol gọi policy hook khi cap level = `Defer`.
- Caching scope (per-session vs per-call).
- Return type (`Trit` final answer, hay `Trilean` chained Unknown?).
- Failure mode khi user policy crash.

ADR-0016 chỉ commit: hook **tồn tại**, hook input bao gồm `(namespace_path, requester_pkg, dep_chain, ResolutionOrigin)`.

### Cho ADR-0018 (loader semantics) — TBD

ADR-0018 phải chốt:
- Refuse-to-load wire-level behavior khi `cap_count > 0` ở reader pre-v0.6 (E2208).
- Manifest source syntax (parse rules cho user-facing `requires:` block).
- Capability check at JIT-load-time (v0.9 Cranelift) khi function lifted across cap boundary.

ADR-0016 chỉ commit: load-time check **xảy ra**, occurs sau resolver and before code section load.

### Cho v0.7 self-hosting compiler

Triết-written compiler phải honor `caps section` semantics — đây là contract bootstrap chain phải bit-identical preserve. Self-hosting test ([ROADMAP §v0.7](../../ROADMAP.md)) phải verify cap enforcement không phụ thuộc Rust impl.

### Cho v0.8 concurrency

[ROADMAP §v0.8](../../ROADMAP.md) hint *"Actor + structured concurrency"* alignment với capability. Actor's mailbox capability có thể reuse namespace mechanism (e.g., `usr.actor.mailbox = +1`). ADR-0016 không pre-commit, nhưng namespace shape không khoá v0.8 ra.

## Không làm

- **Per-function capability** (`sys.io.read_file = +1` còn `sys.io.write_file = -1`). Defer post-v1.0. Workaround: stdlib author tách thành module riêng nếu cần granularity đó.
- **Wildcard grants** (`sys.* = +1`). Vi phạm "Explicit > implicit". Mỗi module phải explicit. Pain ergonomics có chủ ý — capability audit phải đọc được linear ở manifest.
- **Path inheritance** (parent grant covers children). Module path là leaf identifier ở cap-check level. `sys.io = +1` không cover `sys.io.async` — phải declare riêng.
- **Implicit union qua deps** (Cargo features pattern). Root must explicitly grant. Refuse over guess.
- **Auto-shim cap mismatch** ([ADR-0013](0013-semver-linking-policy.md) đã từ chối auto-shim ABI; ADR-0016 kế thừa cùng nguyên tắc cho cap). Refuse-to-link, output miette-friendly diff, user fix manifest.
- **Bump `abi_version`** — slot đã reserved, populate là edit data, không phải schema change.
- **Hardware enforcement** ở v0.6. Cần phần cứng tam phân hoặc bytecode VM sandbox để fence địa chỉ. VM v0.3 chạy in-process Rust, không sandbox — defer v0.8+ khi concurrency landing.
- **Distributed capability** (cross-machine grant tokens). Local-only ở v0.6. Distributed defer v1.0+ cùng remote registry.
- **Cross-arch cap** — cap declarations arch-independent (definitions là namespace strings, không hardware). Inherits từ [ADR-0007 §4](0007-ir-design.md) arch-independent IR.
- **`std.*` / `core.*` cap enforcement** — ambient, không check. `std.io.println` không cần grant (giống printf trong C — không hardware fence). Future tighten: defer.
- **Capability runtime hot-reload** — grant set frozen at link time + Defer resolution at load time. KHÔNG dynamic re-grant mid-session (vi phạm capability monotonicity).

## Prior art

- **[Java JPMS](https://openjdk.org/jeps/261)** — module-info.java with `requires`/`exports`/`opens`. Lấy: declarative module-level capability ở manifest, không runtime token. Khác: Triết thêm Trit level (deny + ambient ngoài grant) và Trilean defer.
- **[Android `<uses-permission>`](https://developer.android.com/guide/topics/manifest/uses-permission-element)** — root app manifest declares all permissions; OS enforces at runtime. Lấy: root manifest = authority; deps' claims là requests. Khác: Triết enforce ở compile + link, không chỉ runtime.
- **[Pony object capabilities](https://www.ponylang.io/discover/#object-capabilities)** — capability as type modifier on object refs (`iso`, `tag`, ...). Bị từ chối: per-object token-passing verbose, không khớp namespace mental model.
- **[Genode OS](https://genode.org/documentation/general-overview/index)** + **[seL4](https://sel4.systems/About/seL4-whitepaper.pdf)** — cap-based microkernel, parent component grants caps to child explicitly. Lấy: parent (root pkg) authoritative; refuse-by-default. Khác: Triết là language-level static check, không kernel.
- **[E language](http://www.erights.org/)** — object cap với defer-to-vat mechanism. Lấy: Trilean::Unknown defer pattern inspired (vat ≈ runtime policy hook).
- **[Mojo capabilities](https://docs.modular.com/mojo/manual/structs/) (status: tentative)** — declared in roadmap nhưng chưa land. Watch list, không adopt.
- **[Roc platform](https://www.roc-lang.org/platforms)** — platform-injected capabilities. Bị từ chối tương tự Pony: verbose token passing.

**Anti-prior-art:**

- **Java checked exception** — function-level `throws` propagate khắp call chain → community quay lưng. Triết tránh bằng cách giữ cap ở module level, không function.
- **POSIX setuid + capabilities(7)** — Linux runtime cap system; vô số CVE từ confused-deputy. Triết tránh bằng compile + link enforcement, runtime hook chỉ cho explicit Defer.
- **C++ `friend` keyword** — fine-grained leak. Triết refuse fine-grained; explicit module-level only.

## Tham chiếu

- [VISION §3.5 — OS-Native Capability Namespaces](../../VISION.md) — trụ cột 5
- [VISION §5 — Bản sắc Triết](../../VISION.md) — Trit-level cap + Łukasiewicz cap check
- [VISION §6 — Nguyên tắc thiết kế](../../VISION.md) — Refuse over guess, Explicit > implicit
- [SPEC §1.4 — Keywords](../../SPEC.md), [SPEC §10 — Reserved namespace roots](../../SPEC.md)
- [ADR-0005 — Module system](0005-module-system.md) — `AbsolutePath` shape; reserved roots
- [ADR-0006 §3 — Ternary Tree namespace](0006-ternary-packaging-vision.md) — north star: `module sys.io (layer: -1)` syntax direction
- [ADR-0007 §3.4, §6.7 — IR namespace tag preserved](0007-ir-design.md) — zero IR change for cap check
- [ADR-0011 §5, §7 — `caps section` reserved + abi_version policy](0011-abi-metadata-format.md) — slot to populate, no bump
- [ADR-0013 — Semver linking policy](0013-semver-linking-policy.md) — E23XX namespace, refuse-to-link pattern
- [ADR-0014 §4, §5 — Package hash includes caps](0014-hash-scheme-refinement.md) — hash-stable across pre/post v0.6
- [ADR-0015 Addendum — ResolutionOrigin 3-state](0015-package-store-layout.md) — dispatch slot for ADR-0017
- ADR-0017 — Trilean policy hook protocol (TBD, v0.6 phase)
- ADR-0018 — Capability loader semantics (TBD, v0.6 phase)
- [ROADMAP §v0.6 — Capability System](../../ROADMAP.md)
