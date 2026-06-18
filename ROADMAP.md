# Triết — Roadmap

> Lộ trình của bản **rewrite** (2026-06 trở đi), chia theo **Bậc A/B/C**.
> **Nguyên tắc:** *stability over speed* + *soundness over test color*.
>
> ⚠️ Đây KHÔNG còn là cuộc hành quân version-number v0.2 → v3.0 → OS. Lộ trình đó
> thuộc compiler **đã bị xóa** (2026-06-04) và mang cái bệnh sương mù mà
> [VISION §0](VISION.md) đã mổ. Tiến độ giờ đo bằng **gate của từng Bậc**, không
> bằng việc tiến tới một đích OS. OS-capable là **ràng buộc thiết kế**, không phải
> milestone — xem [VISION §7](VISION.md).

---

## Triết lý phasing

1. **Mỗi Bậc có gate rõ ràng** (`scripts/gate.sh`: build + test + fixtures +
   clippy). Không mở Bậc kế khi Bậc hiện tại chưa pass gate.
2. **Soundness trước syntax.** Một crash/soundness hole có sẵn được đập trước mọi
   tính năng mới (quy ước G, đã áp dụng nhiều lát — xem `docs/TODO-ARCHIVE.md`).
3. **Quyết định kiến trúc có ADR**, hai chữ ký (O + G) cho mọi ADR rewrite-era
   (0037+).
4. **Không có milestone nào mà dự án một-người không thể giao.** Đích xa (OS thật,
   phần cứng tam phân) là *cảm hứng*, không phải cam kết — §"Cảm hứng cuối chân trời".

---

## Trạng thái hiện tại — rewrite, Bậc C đang chạy

**v0.2–v0.10 đã ship và đã bị xóa** (2026-06-04). Frontend (lexer, parser, modules,
typecheck) giữ lại; backend viết mới: MIR → NLL borrowck → Cranelift JIT.

Pipeline: `.tri → parse → typecheck → lower → MIR verify → borrowck → JIT → execute`

| Bậc | Nội dung | Trạng thái |
|---|---|---|
| **A** | Scalar + arithmetic + logic Ł3/K3 + control flow + đệ quy + flat struct (StackSlot/sret) + enum + NLL borrowck + MIR verifier + nullable `T?` (PA-3c sentinel) | ✅ Đóng 2026-06-06 |
| **B** | Heap types (String/Vector/HashMap qua shim Rust) + match `~+/~0` + heap qua biên user-fn (B7-lift, move-only, `Deinit` tombstone) — ADR-0041/0042/0043, hai chữ ký O+G | ✅ Đóng 2026-06-07 |
| **C** | (1) ✅ Arithmetic range enforcement, trap-on-overflow (ADR-0044) · (2) ✅ CFG tail-expression (ADR-0055, lát 1 SIGILL + lát 2 `= ~0`) · (3) 🔨 Heap-nullable (`T?` cho heap, saga ~5 lát — gate ở LOWER) · (4) ⏳ Borrow params heap `&+ T`/`&0 T`/`&- T` · (5) Outcome 2-reg ABI · (6) Native multi-field layout | 🔨 Đang chạy |

Gate hiện hành: `scripts/gate.sh`. Backlog chi tiết + debt registry: [`TODO.md`](TODO.md)
(nguồn sống duy nhất). Năng lực compiler: [`CLAUDE.md`](CLAUDE.md) §Maturity.

**⚠️ Schema type-system gap:** generated `Type` enum là spec-only; typechecker dùng
hand-written `Type`. Schema lái AST + ownership, CHƯA lái type system. Xem
`spec/plans/phase1-schema-s6-model.md`.

---

## Sau Bậc C — CHƯA XẾP LỊCH (không hứa version, không hứa ngày)

Danh sách trung thực những việc lớn còn lại. Thứ tự sẽ do soundness + nhu cầu thật
quyết định, KHÔNG do một lộ trình tuyến tính áp đặt:

### 🚨 ƯU TIÊN HÀNG ĐẦU: AI-First Validation (Workstream kiểm chứng bắt buộc)

Để chứng minh dự án là một thí nghiệm khoa học thực thụ chứ không phải là sự trốn tránh thực tế, hai công cụ sau **phải được xây dựng đầu tiên** trước khi tiếp tục mở rộng các tính năng compiler backend khác:

1. **🔬 AI-first instrument (turns-to-green)** (từ [VISION §5.3](VISION.md)): Xây dựng bộ đo turns-to-green tự động (cho LLM spec + ví dụ -> sinh Triết -> chạy driver -> nạp lại diagnostic khi lỗi -> đo số turns và tỷ lệ tự sửa đúng). Đây là gate để quyết định thiết kế ngôn ngữ có thực sự tối ưu cho AI hay không.
2. **🔨 Auto-fixer (triet fix)**: Xây dựng hạ tầng tự động sửa các lỗi Machine-Applicable (deterministic) trực tiếp trên AST. LLM tuyệt đối không được dùng cho nhóm lỗi này để tránh lãng phí năng lượng và gây nhiễu kết quả đo đạc.

> [!IMPORTANT]
> **Quy tắc và tính hợp lệ của phép đo (Sanity check cho Benchmark):**
> - **Chỉ đo trên các tính năng đã hỗ trợ đầy đủ**: Tập task test chỉ được sử dụng các cú pháp và kiểu dữ liệu mà compiler hiện tại đã hạ (lower) và JIT chạy thành công. Không bắt LLM viết code sử dụng các tính năng chưa hoàn thiện (như heap-nullable, borrow-param heap...) rồi quy kết turns-to-green cao là do thiết kế ngôn ngữ tệ cho AI.
> - **Mục tiêu là lặp (Iteration), không phải tự hủy (Guillotine)**: Kết quả đo đạc ban đầu xấu không có nghĩa là khai tử dự án ngay lập tức. Nó là tín hiệu chỉ ra phân khúc lỗi nào đang bị nghẽn (mượn, kiểu, hay diagnostic mơ hồ) để điều chỉnh thiết kế ngôn ngữ/diagnostic rồi đo lại. Nếu sau nhiều vòng lặp cải tiến thiết kế mà turns-to-green vẫn không nhúc nhích → **rút lại trụ cột "AI-first" một cách trung thực** (VISION §1: dự án vẫn giữ giá trị craft (a), nhưng claim (b) bị khai tử). Đây là rút claim, KHÔNG tự động là xóa dự án.

---

### Các việc lớn khác (Sẽ triển khai sau khi có kết quả đo đạc AI-first)

- **Self-host trở lại** — `compiler/` (~23K LOC `.tri`) là ORPHAN: target IR/VM đã
  xóa, không bootstrap được. Phải viết lại trên MIR. Multi-month milestone.
- **AOT cache / native binary** — hiện chỉ có JIT. AOT là tiền đề cho ràng buộc
  freestanding ([VISION §7](VISION.md)).
- **Wire `triet-pack`** — `.khi` + cross-package linker còn code (giữ từ compiler
  cũ), chưa wire vào pipeline mới.
- **Rebuild capability runtime** — Trit-level + Ł3 `Unknown` (ADR-0016/0017/0018,
  thiết kế còn sống, hiện thực đã xóa). Phục vụ tính nhất quán thẩm mỹ ([VISION §8](VISION.md)).
- **CAS packaging + stable ABI rebuild** — ADR-0014/0015 + ABI design còn sống,
  hiện thực đã xóa.
- **BYOS concurrency** — ADR-0026 v2, sau khi nền ổn định.

---

## Cảm hứng cuối chân trời — KHÔNG phải milestone cam kết

Những thứ dưới đây là **cảm hứng định hình bản sắc**, không phải đích trên lộ trình.
Một dự án một-người sẽ không "giao" chúng; chúng tồn tại để **kỷ luật ràng buộc hôm
nay**, không để hứa hẹn ngày mai. (Đây chính là chỗ VISION cũ tự lừa mình — xem
[VISION §0](VISION.md).)

- **Production stability ("v1.0")** — đóng băng spec, backwards-compat policy. *Cảm
  hứng*, mở khi có nền + ecosystem thật, không có ngày.
- **Native AOT (LLVM)** — production codegen sau Cranelift. Tiền đề cho freestanding.
- **OS trên phần cứng nhị phân** — Rust chứng minh khả thi (Redox/Hubris). Với Triết
  đây là **ràng buộc giữ-cửa-mở** (no-GC, freestanding-được), **KHÔNG phải lời hứa
  build microkernel**. Xem [VISION §7](VISION.md). Không có "v3.0 kernel POC" nữa.
- **Phần cứng tam phân** — backend trytecode nếu/khi phần cứng tam phân xuất hiện.
  Lợi thế phần cứng là lý thuyết, dự án **không cược vào nó** ([VISION §6](VISION.md)).

---

## Lịch sử v0.2–v0.10 — compiler đã xóa (digest)

> ⚠️ Mọi mục dưới đây mô tả trạng thái đỉnh của compiler **ĐÃ BỊ XÓA** 2026-06-04.
> Trừ frontend "giữ lại", không feature backend nào dưới đây tồn tại trong rewrite.
> Toàn văn từng phase: git history + [`docs/ARCHIVE.md`](docs/ARCHIVE.md).

| Phase | Nội dung chính | ADR | Trạng thái |
|---|---|---|---|
| v0.2.x | Module system (dot paths, verbose keywords) | 0005 | **giữ lại** (frontend) |
| v0.3 | Bytecode VM, register-SSA IR 53 opcodes | 0007/0008/0010 | **đã xóa** |
| v0.4 | Crate-Pack `.khi` + stable ABI + semver linker | 0011-0013 | triet-pack giữ, **chưa wire** |
| v0.5 | CAS packaging, store, GC, `dao.lock` | 0014/0015 | **đã xóa** |
| v0.6 | Capability system `sys./dev./usr.` | 0016-0018 | **đã xóa** (ADR sống) |
| v0.7 | Self-host ~23K LOC + Outcome + Trilean! | 0019-0021/0024 | ORPHAN (không bootstrap) |
| v0.8 | S6 ownership 5-form + BYOS concurrency | 0022/0025-0027 | **đã xóa** (semantics giữ) |
| v0.9 | JIT Cranelift + borrow enforcement + Atomic | 0028-0031 | **đã xóa** |
| v0.10 | Builtin-shim 36/43 + NLL + Atomic (1637 test) | 0032/0033 | **đã xóa** |
| v0.11 (dở) | JIT aggregate 96% + AOT cache | 0034-0036 | **đã xóa cùng backend** |

Frontend giữ lại đang chạy: typecheck (inference + Trilean! refinement ADR-0021),
Łukasiewicz Ł3 + Kleene K3, module system, Outcome syntax (`T~E`/`T?~E`, JIT chưa
hỗ trợ). 1637-test safety net **đã mất**; lưới mới: ~1086 workspace test + 72 fixture.

---

## Decision log: Cái KHÔNG làm và lý do

| Đề xuất | Quyết định | Lý do |
|---|---|---|
| "v3.0 microkernel POC" làm milestone | **Reject (2026-06-18)** | OS-trên-nhị-phân khả thi nhưng là multi-hundred-person-year; làm milestone = sương mù unfalsifiable. Giữ làm *ràng buộc*, không *đích*. [VISION §7]. |
| Cược vào phần cứng tam phân | **Reject** | Lợi thế lý thuyết, thua nhị phân 70 năm. Tam phân là bản sắc, không phải cá cược. [VISION §6]. |
| Tuyên "AI-first đã chứng minh" | **Reject** | Chưa có phép đo. Giả thuyết-chưa-đo, instrument sẽ xây. [VISION §5]. |
| GC | **Reject** | System language. Memory model kiểu Rust borrow checker (đã có MIR + NLL borrowck). |
| Java-style strict filesystem mapping | **Reject** | Java đã bỏ với JPMS. Refactor-unfriendly. |
| Auto-shim ABI migration | **Reject** | Detection decidable, adaptation undecidable. Misleading promise. |
| Backwards-compat shim trước v1.0 | **No** | Trước stable, breaking changes free. |

---

## Pace

Không cam kết ngày. Chân trời thật duy nhất hiện nay là **đóng Bậc C**. Mọi thứ sau
đó (§"Sau Bậc C") xếp theo soundness + nhu cầu, không theo lịch áp đặt. Đích xa là
cảm hứng, không phải deadline.

> Stability over speed. Pace chậm là tính năng, không phải bug.
