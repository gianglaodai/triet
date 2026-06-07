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

## Trạng thái hiện tại — Rewrite v0.1.0-dev (2026-06-04)

**v0.2–v0.10 đã ship và đã bị xóa.** Ngày 2026-06-04, toàn bộ backend cũ
(triet-ir, triet-interpreter, triet-bootstrap, triet-cli) bị xóa vĩnh viễn
trong một cuộc rewrite từ đầu. Frontend (lexer, parser, modules, typecheck)
được giữ lại. Backend mới: MIR → NLL borrowck → Cranelift JIT.

Pipeline hiện tại: `.tri → parse → typecheck → lower → MIR verify → borrowck → JIT → execute`

**Lộ trình rewrite chia theo Bậc** (thay cho version number của dòng cũ):

| Bậc | Nội dung | Trạng thái |
|---|---|---|
| **A** | Scalar + arithmetic + logic + control flow + flat struct (StackSlot/sret) + enum + NLL borrowck + MIR verifier + nullable `T?` (PA-3c) | ✅ Đóng 2026-06-06 |
| **B** | Heap types (String/Vector/HashMap qua shims) + match `~+/~0` + heap qua user-fn boundary (B7-lift, move-only, `Deinit`) — ADR-0041/0042/0043, đều hai chữ ký O+G | ✅ Đóng 2026-06-07 |
| **C** | (1) ✅ Arithmetic range enforcement — trap-on-overflow, ADR-0044 (D1/D1-literal/D3 đóng) · (2) ⏳ Borrow params heap `&+ T`/`&0 T`/`&- T` (ADR-0045, next) · (3) Outcome 2-reg ABI · (4) Native layout + String keys HashMap · codegen opt backlog | 🔨 Đang chạy |
| **Sau C** | Self-host trở lại (compiler/ đã xóa — viết lại trên MIR), AOT cache, wire triet-pack, capability runtime, BYOS concurrency | Chưa xếp lịch |

Gate hiện hành: `scripts/gate.sh` (build + test + fixtures + clippy location-set).
Backlog chi tiết + debt registry (D2, F6, …): `TODO.md`. Trạng thái năng lực
compiler: `CLAUDE.md` §Maturity.

**⚠️ Schema type system gap:** generated `Type` enum is spec-only — typechecker
uses hand-written Type. Schema drives AST + ownership, NOT the type system.
See `spec/plans/phase1-schema-s6-model.md`.

Version `0.1.0-dev` thừa nhận đây là một dòng mới — không phải bản nâng cấp từ v0.10.

### Feature inventory from deleted v0.2–v0.10 compiler (HISTORICAL — kept for reference)

> ⚠️ EVERY item below describes the DELETED compiler's peak state. None of
> these backend features exist in the current rewrite. The frontend items
> marked "giữ lại" survive; everything else was purged 2026-06-04.

✅ Tree-walking interpreter + Bytecode VM (register SSA IR, 53 opcodes) — **đã xóa**
✅ Type checker với inference + monomorphization + Trilean! refinement (ADR-0021) — **giữ lại**
✅ Outcome error handling — `T~E` / `T?~E` syntax (ADR-0020) — **giữ lại, JIT chưa hỗ trợ**
✅ Łukasiewicz Ł3 + Kleene K3 — **hoạt động end-to-end**
✅ Module system — **giữ lại**
✅ Crate-Pack `.khi` + cross-package linker với semver decision matrix (ADR-0011/0012/0013) — **giữ lại (triet-pack), chưa wired vào pipeline mới**
✅ CAS Packaging — 3-cấp hash tree, package store `~/.triet/store/`, atomic install, mark-and-sweep GC, `dao.lock` hash-pinned (ADR-0014/0015)
✅ Capability System — `sys.*`/`dev.*`/`usr.*` 4-state level, `dao.package` + `dao.policy` + `/dev/tty` prompt, E22XX namespace (ADR-0016/0017/0018)
✅ Self-hosting Compiler — `compiler/` 7 `.tri` files (~23K LOC), 3-stage bootstrap chain; main.tri convergence gate `#[ignore]`'d (ADR-0019). **⚠️ ORPHAN: compiler/ sources exist but target IR/VM was deleted — cannot bootstrap.**
✅ S6 Ownership Model — 5-form reference `&+`/`&0`/`&-`/`&` + `owned`, `ObjectHeader` 8-byte binary header với refcount atomic ops (ADR-0022)
✅ Concurrency Primitives (BYOS) — Send derivation cho 13 type categories, E2500 fires, capability gates extended (ADR-0026 v2)
✅ Borrow Checker NLL enforcement — E2440, E2400 lifetime elision (3 rules), E2411, E2403 (ADR-0025/0027/0031 §10.1) — **đã xóa cùng VM; borrowck mới có ít hơn**
✅ JIT builtin-shim layer — 36/43 builtins JIT-shimmed, all DELEGATE to `triet_ir::dispatch_builtin` (ADR-0032 §4) — **đã xóa cùng VM**
✅ Multi-thread Atomic — real `raw_thread.spawn` OS threads, `Atomic<T>` via `Arc<Mutex>` (ADR-0026 v2 §3 + ADR-0028 §5)
✅ Cargo workspace `version = 0.10.0`, SPEC header v0.10
✅ Differential tests: 14 single-file + 1 multi-file examples
✅ **1637 tests workspace-wide** (3 `#[ignore]` documented)
🔜 (deleted backlog) v0.11 — JIT AOT cache + bootstrap gate lift + ≥10× perf bench

---

## Lịch sử v0.2–v0.10 — compiler đã xóa (digest)

> Toàn văn từng phase (deliverables, gates, pivot history) nằm trong git history
> của file này và trong [`docs/ARCHIVE.md`](docs/ARCHIVE.md) (digest + catalog
> 36 ADR LIVE/HISTORICAL). Bảng dưới chỉ là mục lục một dòng mỗi phase.

| Phase | Nội dung chính | ADR |
|---|---|---|
| v0.2.x | Module system (dot paths, verbose keywords) | 0005 |
| v0.3 + .x | Bytecode VM, register-SSA IR 53 opcodes, ternary-native IR | 0007/0008/0010 |
| v0.4 | Crate-Pack `.khi` + stable ABI + semver linker | 0011-0013 |
| v0.5 | CAS packaging, store, GC, `dao.lock` | 0014/0015 |
| v0.6 | Capability system `sys./dev./usr.` | 0016-0018 |
| v0.7 | Self-host compiler ~23K LOC + Outcome + Trilean! | 0019-0021/0024 |
| v0.8 + .x | S6 ownership 5-form + BYOS concurrency + audit/cadence/docs-reorg | 0022/0025-0027 |
| v0.9 + .x | JIT Cranelift + borrow enforcement + Atomic | 0028-0031 |
| v0.10 | Builtin-shim layer 36/43 + NLL + multi-thread Atomic (1637 tests) | 0032/0033 |
| v0.11 (dở dang) | JIT aggregate 96% + AOT cache — bị xóa cùng backend 2026-06-04 | 0034-0036 |

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
- **Syscall opcode design** trong IR (VISION §4.4 "OS-friendly properties — syscall opcodes / FFI primitives"). Lock encoding khi LLVM AOT landing — trước đó VM dev tier không cần.

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
