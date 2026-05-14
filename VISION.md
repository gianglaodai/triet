# Triết — Vision

> Một ngôn ngữ tam phân cân bằng, AI-first, đủ năng lực viết hệ điều hành.

Tài liệu này là **north star** dài hạn của dự án. Mọi quyết định kiến trúc lớn phải đối chiếu với tầm nhìn ở đây. Khi tầm nhìn cần thay đổi, sửa tài liệu này trước, không phải sau.

---

## 1. Tại sao Triết tồn tại

Thế giới phần mềm hôm nay là nhị phân. Mọi ngôn ngữ — từ assembly tới TypeScript — đều giả định `bit ∈ {0, 1}`. Đây không phải định luật vật lý mà là di sản kỹ thuật từ điện tử transistor những năm 1950.

Tam phân cân bằng `{-1, 0, +1}` có lý thuyết mạnh hơn, từng được hiện thực phần cứng (Setun, Liên Xô 1958). Nó tự nhiên biểu diễn:
- Số có dấu (không cần two's complement)
- Logic ba giá trị (true / false / unknown — Łukasiewicz Ł3)
- Trạng thái thiếu thông tin (`null` bẩm sinh, không phải bolt-on)

**Triết tồn tại để chứng minh tam phân không chỉ là điều thú vị mà là nền tảng kỹ thuật vượt trội** — đủ để viết application, library, và một ngày nào đó là OS.

> Nếu Triết không thể viết được OS, nó sẽ mãi là một thí nghiệm thú vị bên lề thế giới nhị phân.
> Tham vọng dự án: khi phần cứng tam phân xuất hiện, Triết sẵn sàng làm tầng phần mềm cho nó.

## 2. Mâu thuẫn ngôn ngữ Triết phải giải quyết

Ngôn ngữ system hiện đại có ba trục đối lập:

| Trục | Đại diện | Ưu | Nhược |
|---|---|---|---|
| **Linh hoạt** | C++ | Toàn quyền với phần cứng | Unsafe, ABI mong manh |
| **An toàn** | Rust | Memory safety, không GC | ABI bất ổn, generics phá ABI binary |
| **Nhất quán** | Java | Tooling đỉnh, ABI ổn định | GC, không xuống được hardware |

Triết nhắm tới **đỉnh tam giác**: linh hoạt như C++, an toàn như Rust, nhất quán như Java. Bằng cách:
- Tam phân cân bằng giải quyết null/sign tự nhiên ở tầng kiểu
- Łukasiewicz Ł3 cho logic 3 giá trị làm capability runtime
- CAS packaging giải quyết DLL Hell ở gốc, không patch
- Stable ABI thiết kế từ đầu, không bolt-on về sau

## 3. Năm trụ cột kiến trúc

### 3.1 — CAS Packaging (Content-Addressable Storage)

Mỗi module/package được định danh bằng **hash** của nội dung, không phải đường dẫn hay version string.

**Why:**
- Triệt tiêu DLL Hell: chạy song song N phiên bản cùng thư viện không xung đột.
- Build deterministic: cùng input → cùng hash → cùng artifact.
- Shared loading ở OS level: 10 ứng dụng dùng `String.format` chỉ load 1 bản vào RAM.
- AI-first: hash là địa chỉ ngữ nghĩa hoàn hảo cho LLM tham chiếu code chính xác.

**Prior art:** [Unison language](https://www.unison-lang.org/) (hash AST nội dung, model phân phối phi tập trung), Nix/Guix (CAS ở OS level), Bazel (action cache), Go module sums.

**Triết-specific:** Tách hai cấp hash:
- `iface_hash` — hash của ABI surface (export, types, signatures) → dùng cho linker
- `impl_hash` — hash toàn bộ implementation → dùng cho deduplication runtime

Sửa implementation không thay đổi `iface_hash` → không trigger rebuild downstream. Quan trọng cho compile-time scaling.

**Phase:** v0.5 (sau khi có bytecode IR ổn định ở v0.3).

### 3.2 — Module System: Hierarchical, Explicit Export

Cấu trúc module phân cấp, **explicit `pub` export**, KHÔNG bind cứng vào filesystem.

**Why:**
- Hierarchical = predictable navigation, fast compile (compiler không scan blindly).
- Explicit export = ABI surface rõ ràng, là prerequisite cho stable ABI (trụ cột 3).
- Không bind filesystem = refactor-friendly. Java đã chính thức từ bỏ ràng buộc filesystem từ JPMS (Java 9).

**Prior art:** Rust mod system (chính), OCaml signatures (deferred), Mojo modules.

**Anti-prior-art:** Java pre-Jigsaw filesystem mapping, Python implicit packages.

**Phase:** v0.2.x ✅ (shipped). Chi tiết: [`docs/decisions/0005-module-system.md`](docs/decisions/0005-module-system.md).

### 3.3 — Stable ABI: Interface-First Design

Mỗi crate-pack mang theo file metadata mô tả ABI surface ở dạng nhị phân gọn. Compiler là gatekeeper: cross-package mismatch = refuse-to-link với diagnostic rõ ràng.

**Why:**
- ABI bất ổn là điểm yếu chí mạng của Rust và C++. Khắc phục được = lợi thế cạnh tranh thực sự.
- Triết có lợi thế bẩm sinh: Trit/Tryte/Integer/Long có kích thước cố định, không struct padding ambiguity, không endianness — ABI primitives đã ổn định bẩm sinh.

**Generics ở biên ABI** (vấn đề khó nhất):
- **Cross-package**: dictionary passing / witness tables (Swift-style). Generics không bị monomorphize qua biên crate-pack → ABI ổn định.
- **Intra-package**: monomorphization (Rust-style). Tốc độ tối ưu trong cùng compilation unit.

**KHÔNG hứa auto-shim.** ABI breakage *detection* là decidable; *automatic adaptation* không phải general case (semantic change không thể tự động suy luận). Thay vào đó:
- Compiler refuse-to-link với diff rõ ràng (kiểu miette).
- Migration tools explicit khi major-version mismatch.
- Semver-aware: minor mismatch = warning, major = error.

**Prior art:** Swift stable ABI (chính), .NET assemblies, Java `.class`, Mojo `.mojopkg`. Anti-prior-art: C++ ABI (cautionary tale).

**Phase:** v0.4 (sau bytecode IR).

### 3.4 — Crate-Pack & Hybrid Linking

Đơn vị phân phối là **Crate-Pack**: file nhị phân kèm metadata (ABI signatures, dependency hashes, capability claims).

**Hybrid Linking**:
- **Static link tại build time** cho hot path → tốc độ runtime tối đa.
- **Dynamic link tại runtime** cho cold path / shared libraries → RAM efficiency.

**Why:** JVM/CLR đã chứng minh mô hình này 25 năm. Không có gì tranh cãi về kỹ thuật.

**Functional error handling:** `Result<T, E>` + `Option<T>` (đã có nền G.1 generics). Triết có lợi thế: `T ⊂ T?` subtyping bẩm sinh → null không cần unwrap dance khi widening.

**Phase:** v0.4 (cùng phase với ABI).

### 3.5 — OS-Native Capability Namespaces

Top-level namespace dành riêng đại diện **Resource Tree** của OS:
- `sys::` — syscall surface
- `dev::` — driver / hardware interface
- `usr::` — user application space
- `std::` — standard library (mặc định ambient)

Capability không phải convention mà **enforce ở compiler**: ứng dụng `usr::*` không thể `use dev::*` trừ khi có capability token.

**Why đây là trụ cột novel nhất của Triết:**
1. **Trit-level capability:** capability không phải boolean (cấp/cấm). Là `Trit`:
   - `-1` = deny (cấm cứng, compile-time error)
   - `0` = ambient / inherit từ caller
   - `+1` = grant (cấp explicit)
2. **Łukasiewicz capability checking:** capability có thể là `Trilean::Unknown` → giải quyết runtime bởi user/policy. Logic 2-giá trị không làm được.
3. **Tự nhiên ăn nhập** với balanced ternary và Ł3 — không phải bolt-on.

**Prior art:** [Pony](https://www.ponylang.io/) (object capabilities ở type system), [Genode OS](https://genode.org/) / [seL4](https://sel4.systems/) (capability microkernel), E language, Plan 9 namespaces.

**Phase:** v0.6 (sau khi module system, ABI, CAS đã chín).

---

## 4. Mô hình thực thi: IR-centric, multi-backend

Triết là **AOT native language với multi-backend strategy**. KHÔNG phải VM-based language như Java/JVM hay C#/.NET. Phần này định khung lý do tách "ngôn ngữ Triết" khỏi "implementation chạy Triết".

### 4.1 Hai khái niệm cốt lõi cần phân biệt

**IR (Intermediate Representation)** — đặc tả ngôn ngữ máy ảo, là **biên giới giữa "ngôn ngữ Triết" và "phần cứng đích"**. Stable, version-locked sau v1.0. Đây là di sản kỹ thuật thực sự của project: spec sống lâu hơn bất kỳ implementation nào.

**Backend** — implementation chạy IR trên một target cụ thể (VM, JIT, AOT native, ternary native). Có nhiều backend, có thể thay thế / bổ sung qua thời gian. Backend là implementation detail, không phải kiến trúc.

JVM bake VM + IR vào một, ép managed runtime + GC vào ngữ nghĩa ngôn ngữ — đó là lý do Java không viết được OS. Triết tách rõ: **IR là spec, backend là implementation**. Không có "Triết runtime" mãi mãi như JVM runtime.

### 4.2 Bốn backend tiers

```
Triết source (.tri)
        │
        ▼  Lower (compile-time)
   Triết IR (stable spec, ADR-0007)
        │
        ├─► Backend 1: Bytecode VM (v0.3) ─► development tier
        │       Mục đích: fast iteration, IR validation, test oracle.
        │       KHÔNG phải production runtime.
        │
        ├─► Backend 2: JIT (v0.9, Cranelift) ─► hot-path runtime
        │       Mục đích: tier-up cho code chạy thường xuyên.
        │       Compile bytecode → machine code at runtime.
        │
        ├─► Backend 3: AOT native (v2.0, LLVM) ─► PRODUCTION TARGET nhị phân
        │       Mục đích: binary .exe native cho x86-64 / ARM64 / RISC-V.
        │       Zero VM overhead, zero managed runtime.
        │
        └─► Backend 4: Trytecode (v∞, ternary hardware) ─► PRODUCTION TARGET tam phân
                Mục đích: native code cho CPU tam phân thật khi xuất hiện.
                Trit là unit hardware thực, không emulate qua bit.
```

### 4.3 Production target qua thời gian

| Phase | Production runtime | Status |
|---|---|---|
| v0.2 | Tree-walking interpreter | Development tier (still runs alongside VM) |
| v0.3–v1.x | Bytecode VM | Development tier — không phải production |
| v2.0+ | AOT native binary (LLVM) | **Production target nhị phân** |
| v∞ | Trytecode native | **Production target tam phân** (cần phần cứng) |

VM ở v0.3 là **scaffolding**, không phải đích cuối. Nó tồn tại để:

1. **Validate IR design** — test IR shape qua thực thi trước khi commit IR vào ADR vĩnh viễn. LLVM debug cùng lỗi đó tốn vài tuần.
2. **Self-hosting platform** — compiler Triết viết bằng Triết (v0.7) cần một runtime để chạy trước khi LLVM landing ở v2.0.
3. **Differential test oracle** — VM output phải byte-identical với tree-walker (v0.2). Bug regression bắt sớm.
4. **Ecosystem development** — phát triển stdlib + library + tooling trong khi backend production chưa có.

Khi v2.0 LLVM backend landing, VM trở thành tier debug/development không bắt buộc cho production. Khi v∞ trytecode backend landing trên ternary hardware, LLVM backend trở thành compatibility tier cho legacy binary CPU.

### 4.4 Hệ quả: IR phải OS-friendly, không VM-friendly

Đây là điểm phân biệt then chốt giữa Triết IR và JVM bytecode:

| Phải có | KHÔNG được có |
|---|---|
| Raw pointer + manual memory model (Mojo-style ARC) | GC bắt buộc kiểu JVM |
| Capability namespace ở IR level (`sys.*`/`dev.*`/`usr.*` preserved) | Sandbox runtime ép buộc |
| Trit/Tryte/Integer/Long là first-class IR type với type tag | Object reference singleton runtime |
| Syscall opcodes / FFI primitives (định hình ở v0.4 ABI) | Type erasure kiểu JVM generics |
| Stable encoding — additive-only sau v1.0 | "Implementation defined" mơ hồ |

JVM không thể làm OS vì nó **cố tình** bake managed runtime vào IR. Triết IR sẽ **cố tình không** bake. Đây là điều ADR-0007 phải bảo vệ qua mọi quyết định opcode + memory model.

### 4.5 Trytecode không phải trick — nó là target cuối cùng

Một ngày khi phần cứng tam phân xuất hiện (Setun-style modern hoặc memristor-based ternary), backend trytecode sẽ là **production target chính**. Backend nhị phân (v2.0 LLVM) trở thành compatibility tier cho legacy binary CPU.

Đây là lý do "ngôn ngữ Triết" và "Triết IR" được tách rạch ròi:

- **Ngôn ngữ Triết** (SPEC.md) — semantics tam phân, viết một lần, không đổi qua hardware era.
- **Triết IR** (ADR-0007) — substrate-neutral, có type tag tam phân nhưng không hardcode encoding bit/trit.
- **Backend nhị phân** — encode `Trit` thành 2 bit, emit x86/ARM. Có overhead encoding nhưng vẫn chạy được.
- **Backend trytecode** — encode `Trit` thành 1 trit thật, emit native ternary instructions. Zero encoding overhead.

**Cùng source `.tri`. Cùng IR. Khác backend. Khác hardware.** Người dùng không sửa code khi đổi target — đây là cam kết dài hạn của VISION.

**Phase:** v0.3 (IR + backend 1 VM), v0.9 (backend 2 JIT), v2.0 (backend 3 AOT), v∞ (backend 4 trytecode). Chi tiết: [ADR-0007](docs/decisions/0007-ir-design.md).

---

## 5. Bản sắc Triết

Ba điều khiến Triết không thể bị thay thế bằng "Rust + Mojo + Nix":

1. **Trit-level capability** — 3-state native, không phải emulate bằng `enum { Allow, Deny, Inherit }`.
2. **Łukasiewicz capability checking** — `Unknown` capability giải quyết bởi runtime policy, không cần bolt-on policy engine.
3. **Tam phân ABI ổn định bẩm sinh** — không có struct padding, không endianness, không integer overflow ambiguity. Trit/Tryte/Integer/Long là ABI primitives ổn định trước khi viết dòng compiler nào.

Nếu giữ được 3 điểm này, Triết có lý do tồn tại độc lập — kể cả khi phần cứng tam phân chưa xuất hiện, kể cả khi Rust/Mojo/Pony hoàn thiện stable ABI của họ.

## 6. Nguyên tắc thiết kế (commit hard)

| Nguyên tắc | Ý nghĩa |
|---|---|
| **Stability over speed** | Mọi quyết định kiến trúc có ADR. Không "ship đại rồi sửa". |
| **Prior art over invention** | Đứng trên vai Unison/Mojo/Pony/Genode/Swift. Phát minh chỉ ở chỗ tam phân thực sự khác biệt. |
| **AI-first stays** | Cú pháp, error message, package layout đều phải tối ưu cho LLM sinh code đúng lần đầu. |
| **Tam phân là mặc định, không phụ trợ** | Không có "binary mode". Trit/Tryte/Integer/Long luôn là kiểu nguyên thủy. |
| **Explicit > implicit** | Export, capability, dependency, ABI surface — tất cả tường minh. Glob imports, `pub` mặc định, ambient capabilities — bị cấm. |
| **Refuse over guess** | Khi compiler không chắc → error rõ ràng, không suy luận im lặng. |

## 7. Cái Triết KHÔNG là

Để rõ ràng (giúp tránh scope creep):

- **Không phải replacement cho Rust/C++.** Triết là ngôn ngữ thứ ba, có domain riêng (tam phân + AI-first + capability).
- **Không phải ngôn ngữ "binary với Ł3 thêm vào".** Tam phân không thể lột bỏ được.
- **Không phải fast-iteration scripting.** Trade-off ngược: stability cao hơn, pace chậm hơn.
- **Không phải general-purpose language ngay từ v1.0.** v1.0 ổn định cho domain ngôn ngữ-cấp-OS; general-purpose là hệ quả tự nhiên, không phải mục tiêu chính.

## 8. Roadmap dài hạn

Phasing chi tiết: [`ROADMAP.md`](ROADMAP.md).

Tóm tắt trục thời gian (5–10 năm):

```
v0.2  ──────►  Struct, enum, generics                     ✅
v0.2.x ─────►  Module system                              ✅
v0.3  ──────►  Bytecode VM + Stable IR                    ✅
v0.4  ──────►  Crate-Pack + ABI metadata                  ✅  ← hiện tại
v0.5  ──────►  CAS packaging                              [next]
v0.6  ──────►  Capability system (sys/dev/usr)
v0.7  ──────►  Self-hosting compiler
v0.8  ──────►  Concurrency model
v0.9  ──────►  JIT (Cranelift)
v1.0  ──────►  Production stability
v2.0  ──────►  AOT native compile (LLVM)
v3.0  ──────►  Microkernel POC
v∞    ──────►  Backend cho phần cứng tam phân
```

## 9. Tham chiếu

**Languages:**
- [Unison](https://www.unison-lang.org/) — CAS code, hash AST.
- [Mojo](https://docs.modular.com/mojo/) — `.mojopkg` ABI metadata.
- [Pony](https://www.ponylang.io/) — object capabilities.
- [Swift](https://www.swift.org/) — stable ABI cho generics.
- [Roc](https://www.roc-lang.org/) — platform model gần với capability namespace.

**Operating Systems:**
- [Genode](https://genode.org/) — capability-based OS framework.
- [seL4](https://sel4.systems/) — formally-verified capability microkernel.
- [Plan 9](https://9p.io/plan9/) — namespace as resource tree.

**Packaging:**
- [Nix](https://nixos.org/) / [Guix](https://guix.gnu.org/) — CAS packaging ở OS.
- [WebAssembly Components](https://component-model.bytecodealliance.org/) — interface types, ABI.

**Lý thuyết:**
- Łukasiewicz Ł3 (1920) — three-valued logic.
- Setun computer (Brusentsov, 1958) — first balanced-ternary computer.
- "The Power of Interoperability" (Bird, 2013) — module systems theory.

---

*Tầm nhìn này là cam kết dài hạn. Pace của dự án sẽ chậm — đó là tính năng, không phải bug.*
