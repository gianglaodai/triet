# Triết — Roadmap

> Lộ trình từ interpreter v0.2 tới ngôn ngữ-OS v3.0+.
> **Nguyên tắc:** *stability over speed.* Mỗi version mở khóa version sau.

Xem tầm nhìn dài hạn ở [`VISION.md`](VISION.md).

---

## Triết lý phasing

1. **Mỗi phase có gate rõ ràng.** Không bắt đầu phase N+1 nếu phase N chưa pass gate.
2. **Quyết định kiến trúc có ADR.** Mỗi trụ cột lớn = một ADR ở `docs/decisions/`.
3. **Test bao phủ trước khi tiến.** Workspace tests phải xanh và sample programs chạy được trước khi bump version.
4. **Không skip phase.** v0.5 (CAS) không thể làm trước v0.3 (bytecode) vì hash AST chưa ổn định.

---

## Trạng thái hiện tại — v0.2

✅ Tree-walking interpreter end-to-end
✅ Type checker với inference + monomorphization
✅ Struct, enum + generics (G.1)
✅ Łukasiewicz Ł3 + Kleene K3
✅ Nullable subtyping `T ⊂ T?` (bẩm sinh tam phân, không bolt-on)
✅ Diagnostic format (miette, error codes E0000–E2007)
✅ 11 demo `.tri` programs pass, 522 tests workspace-wide
🔄 Đang chuẩn bị: module system (v0.2.x)

**Gate đã đạt:** Pipeline hoàn chỉnh, semantics ổn, có thể viết library nội bộ.

---

## v0.2.x — Module system cơ bản

**Mục tiêu:** Cho phép tách codebase thành nhiều file/module với hierarchical namespace, explicit export.

**Quyết định kiến trúc:** [`docs/decisions/0005-module-system.md`](docs/decisions/0005-module-system.md)

**Deliverables:**
- `mod foo;` declaration + file resolution (`foo.tri` hoặc `foo/`).
- `pub`, `pub(pkg)`, default private.
- Path: `crate::`, `self::`, `super::`.
- `use` import: explicit list, `as` rename, **không glob**.
- Reserve top-level namespaces: `std`, `sys`, `dev`, `usr` (chưa enforce capability).
- Cyclic import = compile error.
- Cập nhật stdlib hiện tại (`std.io.println`, `std.text.from_integer`) thành proper module.

**Không làm:**
- Capability enforcement (đợi v0.6).
- Cross-package linking (đợi v0.4).
- Signature files riêng (compiler tự suy ra từ source).

**Gate:** Demo lớn (>500 dòng) chia thành 5+ module, type-check qua biên module đúng, error message chỉ rõ visibility/path mismatch.

---

## v0.3 — Bytecode VM + Stable IR

**Mục tiêu:** Tách "ngôn ngữ Triết" khỏi "implementation Rust hiện tại". Mở khóa CAS, ABI, JIT về sau.

**Deliverables:**
- IR thiết kế lần đầu: stack-based hoặc register-based (sẽ quyết ở ADR-0006).
- Bytecode format đặc tả binary stable.
- VM thực thi bytecode (không còn AST tree-walking ở runtime).
- Compiler `triet build → .triv` (Triết bytecode).
- Bench cho 11 demo programs: bytecode VM nhanh hơn tree-walking ≥3×.
- Snapshot tests cho IR output (regression detection).

**Không làm:**
- JIT (đợi v0.9).
- Native compile (đợi v2.0).
- ABI metadata (đợi v0.4 — cần IR ổn trước).

**Gate:**
- IR spec written + frozen format số phiên bản.
- Tất cả demo `.tri` chạy qua bytecode VM với output identical interpreter.
- Bench ≥3× speedup.

**ADR cần viết:** ADR-0006 (IR design — stack vs register), ADR-0007 (bytecode format).

---

## v0.4 — Crate-Pack + Stable ABI

**Mục tiêu:** Cho phép phân phối binary library, type-safe cross-package linking.

**Deliverables:**
- Crate-Pack format: file nhị phân kèm metadata (`.tripack`).
- Metadata gồm: ABI signatures, dependency declarations, capability claims (placeholder).
- Compiler refuse-to-link với diagnostic khi major-version mismatch.
- Witness table dispatch cho generics qua biên crate-pack.
- Monomorphization vẫn dùng intra-package (tối ưu).
- Linker: hybrid (static cho hot path, dynamic cho shared libs).
- Result/Option đầy đủ trong stdlib (sử dụng generic của G.1).

**Không làm:**
- CAS hash identity (đợi v0.5).
- Auto-shim ABI migration (rejected — không khả thi general case).
- Capability enforcement runtime (đợi v0.6).

**Gate:**
- Phân phối được 1 stdlib package + 1 user package qua crate-pack format.
- Sửa minor version của lib không cần rebuild app.
- Sửa major version mà không update app → linker error rõ ràng.

**ADR cần viết:** ADR-0008 (ABI metadata format), ADR-0009 (witness table dispatch), ADR-0010 (semver linking policy).

---

## v0.5 — CAS Packaging

**Mục tiêu:** Định danh package bằng hash, eliminate DLL Hell, parallel versions.

**Deliverables:**
- Hash scheme: hai cấp `iface_hash` (ABI surface) + `impl_hash` (toàn nội dung).
- Package store: filesystem layout `~/.triet/store/<hash>/`.
- Resolver: `use foo::bar` → resolve theo hash trong manifest.
- Manifest format: lockfile có hash đầy đủ.
- Shared loading: 2+ apps cùng dùng `std@hash_X` → load 1 lần.
- Migration tool: import package từ filesystem path → CAS store.

**Không làm:**
- Distributed registry (defer — local store đủ cho v0.5).
- Garbage collection (defer — manual `triet store gc` đủ).

**Gate:**
- Chạy song song 2 phiên bản incompatible của cùng package trong cùng app, không xung đột.
- Sửa internal của lib không invalidate `iface_hash` → app không rebuild.

**ADR cần viết:** ADR-0011 (hash scheme), ADR-0012 (package store layout).

---

## v0.6 — Capability System (`sys::` / `dev::` / `usr::`)

**Mục tiêu:** Enforce isolation ở compiler level. Đây là trụ cột bản sắc của Triết.

**Deliverables:**
- Top-level namespaces `sys`, `dev`, `usr` enforced.
- `Capability<T>` type ở stdlib: `Capability<Sys::Net>`, `Capability<Dev::Disk>`...
- Trit-level capability: `Trit` hoặc `Trilean` làm capability level.
  - `+1` (Grant) — explicit grant.
  - `0` (Ambient) — inherit từ caller.
  - `-1` (Deny) — compile error nếu cố use.
  - `Trilean::Unknown` — runtime policy resolves.
- Capability propagation rules ở type checker.
- Crate-pack metadata khai báo capability requirements.
- Loader runtime: refuse-to-load nếu không cấp đủ capability.
- Demo: minimal "kernel-style" program với `usr::app` không thể chạm `dev::*`.

**Không làm:**
- Hardware enforcement (cần phần cứng tam phân hoặc bytecode VM sandbox — defer).
- Distributed capability (defer).

**Gate:**
- Compile-time error rõ ràng khi `usr::*` import `dev::*` không có capability.
- Runtime policy hook hoạt động cho `Trilean::Unknown`.
- Demo capability-restricted program chạy được + bị reject khi capability sai.

**ADR cần viết:** ADR-0013 (capability type system), ADR-0014 (Trilean policy hook), ADR-0015 (loader semantics).

---

## v0.7 — Self-hosting Compiler

**Mục tiêu:** Compiler Triết viết bằng Triết. Bootstrap đầy đủ.

**Deliverables:**
- Lexer, parser, typecheck, IR generator viết lại bằng Triết.
- Bootstrap chain: Rust-compiler-v0.6 → Triết-compiler-v0.7 → Triết-compiler-v0.7 (self-build).
- Performance parity với Rust impl trong vòng 2×.

**Không làm:**
- Compile thẳng sang native (vẫn xuất bytecode v0.3).

**Gate:** Bit-identical bootstrap qua 2 vòng tự build.

---

## v0.8 — Concurrency Model

**Mục tiêu:** Triết có model concurrency riêng, có lý thuyết.

**Lựa chọn cần đánh giá ở phase này:**
- Actor model (Pony, Erlang) — tự nhiên ăn nhập với capability namespace.
- Async/await (Rust) — quen thuộc.
- CSP / channels (Go) — đơn giản.
- Structured concurrency (Trio, Project Loom) — modern.

**Quyết định:** ADR-0016 ở giai đoạn vào v0.8. Hiện tại favor **Actor + structured concurrency** vì alignment với capability model.

**Gate:** Demo concurrent program chạy đúng dưới load + race detector pass.

---

## v0.9 — JIT (Cranelift)

**Mục tiêu:** Bytecode VM có JIT tier cho hot code paths.

**Deliverables:**
- Tier 1: bytecode interpreter (v0.3).
- Tier 2: Cranelift JIT cho function chạy thường xuyên (profile-guided).
- AOT cache: lần chạy thứ 2 dùng JIT-output cached.

**Gate:** Bench ≥10× so với v0.3 bytecode trên numeric-heavy programs.

---

## v1.0 — Production Stability

**Mục tiêu:** Đóng băng spec ngôn ngữ ở tầng v1.0. Backwards-compat policy có hiệu lực.

**Deliverables:**
- SPEC.md đóng băng (chỉ thêm, không phá ngữ nghĩa cũ).
- Stable ABI vĩnh viễn cho v1.x.
- LTS branch.
- Toolchain installer (giống `rustup`).
- Documentation đầy đủ + tutorial sách-style.

**Gate:**
- 100+ public crate-pack ngoài stdlib.
- 3+ ứng dụng production thực.

---

## v2.0 — Native AOT Compile (LLVM)

**Mục tiêu:** Sản xuất binary native cho x86-64, ARM64, RISC-V.

**Deliverables:**
- Backend LLVM (Cranelift đã quen từ v0.9 — LLVM cho production codegen).
- Cross-compile toolchain.
- Debug symbol format (DWARF compat).
- FFI ổn định với C ABI (cho legacy interop).

**Gate:** Triết-compiled binary perf ≥80% so với equivalent Rust binary.

---

## v3.0 — Microkernel POC

**Mục tiêu:** Chứng minh Triết viết được OS. Đây là **mục tiêu lý tưởng** đặt cọc tầm nhìn.

**Deliverables:**
- Microkernel boot trên x86-64 (QEMU đầu tiên, sau đó hardware thực).
- `sys::` namespace là syscall thực.
- `dev::` driver tối thiểu (UART, disk, ethernet).
- `usr::` chạy được 1–2 ứng dụng (shell + 1 demo).
- Capability enforcement runtime ở loader.

**Gate:**
- Boot tới shell prompt.
- App `usr::*` không thể chạm hardware không có capability — kernel test xác nhận.

**Đây không phải production OS.** Đây là chứng minh: ngôn ngữ Triết có đủ năng lực ngữ nghĩa và ABI để implement OS. Production OS là dự án riêng.

---

## v∞ — Phần cứng tam phân

**Khi phần cứng tam phân xuất hiện** (Setun-style modern, hoặc memristor-based ternary):
- Backend native cho ternary CPU (không phải emulate trên binary).
- Trit là unit hardware thực.
- Discriminator của `T?` là một trit thật, không phải bit-packed.

Triết đã sẵn sàng — semantics tam phân đã có từ v0.1, không cần thay đổi ngôn ngữ. Chỉ cần backend codegen mới.

---

## Decision log: Cái KHÔNG làm và lý do

| Đề xuất | Quyết định | Lý do |
|---|---|---|
| Java-style strict filesystem mapping | **Reject** | Java đã từ bỏ với JPMS. Refactor unfriendly. |
| Auto-shim ABI migration | **Reject** | Decidable detection nhưng undecidable adaptation. Misleading promise. |
| Glob imports `use foo::*` | **Reject ở v0.2.x** | Phá ABI clarity. Có thể revisit v1.0+. |
| GC | **Reject** | Triết là system language. Memory model tham khảo Rust borrow checker (defer thiết kế đến v0.4+). |
| Macro hệ thống lớn (Rust-style) | **Defer to v0.7+** | Tăng surface area của ngôn ngữ. Compile-time const eval ưu tiên hơn. |
| Backwards-compat shim cho v0.x | **No** | Trước v1.0, breaking changes free. Sau v1.0, bound chặt. |
| Phát minh CAS scheme riêng | **No** | Dùng prior art (Unison/Nix-inspired), bất biến + Triết-specific 2 cấp hash. |

---

## Pace expectations

| Phase | Realistic timeline (small team) |
|---|---|
| v0.2.x | 1–3 tháng |
| v0.3 (bytecode) | 6–12 tháng |
| v0.4 (ABI) | 6–12 tháng |
| v0.5 (CAS) | 4–8 tháng |
| v0.6 (capability) | 8–12 tháng |
| v0.7 (self-host) | 12+ tháng |
| v0.8–v0.9 | 6–12 tháng mỗi phase |
| v1.0 | tích lũy, release window |
| v2.0 (LLVM) | 12+ tháng |
| v3.0 (kernel) | 24+ tháng |

**Tổng:** 5–10 năm cho v3.0 với một team nhỏ hoặc 1 người. Dự án được scale theo realistic, không hứa hẹn.

> Stability over speed. Đây là tính năng.
