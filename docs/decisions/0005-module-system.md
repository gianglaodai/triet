# ADR 0005 — Module system: Java JPMS aesthetic, dot paths, Python imports

**Trạng thái:** Quyết định. Áp dụng cho v0.2.x. Đây là trụ cột #2 trong [VISION.md](../../VISION.md).

> ⚠️ **Import syntax superseded by [ADR-0071](0071-path-separator-and-module-import.md) (2026-06-25).**
> The `import std.io` / `from std.io import …` keywords + dot-separated import
> paths described below are replaced by `use std::io::{a, b as c}` with `::`
> paths. The module *semantics* (hierarchical tree, visibility ladder, cyclic
> refusal, `khi`/`self`/`super` roots, stdlib resolution) remain authoritative —
> only the surface import syntax changed. The body below is kept verbatim as the
> historical record; read it for the module model, not the import keywords.

**Issue:** Triết đã đến giới hạn của single-file program. Code base demo đã 11 file `.tri` ở cùng namespace phẳng. Library nội bộ và phân chia codebase đòi hỏi module system thực sự. Đây cũng là tiền đề kiến trúc cho stable ABI (v0.4), CAS packaging (v0.5), và capability namespaces (v0.6) — module system thiết kế sai sẽ kéo theo phá vỡ ba trụ cột về sau.

## Quyết định

Triết áp dụng **hierarchical module tree theo phong cách Java JPMS**, với cú pháp **verbose keywords** + **dot-separated paths** + **Python-style imports**, **explicit `public` export**, và **không bind cứng vào filesystem**.

### Cú pháp

```triet
// Trong file `pkg.tri` (root của crate `pkg`):
module foo                            // declare submodule, compiler tìm `foo.tri`
module bar                            // declare submodule `bar`

// Trong file `foo.tri`:
public function hello() -> String =   // exported
    "hello"

function helper() -> Integer =        // private (default)
    42

module inline {                       // inline submodule
    public function nested() -> Integer = 1
}
```

### Path syntax

Triết dùng dấu chấm `.` làm path separator (giống Java/Python, khác Rust/C++). Không dùng `::`.

| Path | Ý nghĩa |
|---|---|
| `crate.foo.bar` | Absolute path từ crate root |
| `self.foo` | Relative — current module |
| `super.foo` | Relative — parent module |
| `std.io.println` | Stdlib path |
| `sys.*`, `dev.*`, `usr.*` | **Reserved** ở v0.2.x (chưa enforce; v0.6 sẽ enforce capability) |

`crate`, `self`, `super` là reserved path keywords (ADR-0005 §"Reserved top-level namespaces"), không thể dùng làm identifier.

### Visibility levels

```triet
public function open() = ...          // visible everywhere
public(package) function detail() = ... // visible within same crate-pack only
function helper() = ...               // private to current module (default)
```

**Triết SIMPLIFIES Rust's visibility:** chỉ 3 cấp (`public`, `public(package)`, private). Bỏ `public(super)`, `public(in path)` để giữ ABI surface đơn giản. Có thể bổ sung ở v1.0+ nếu thực sự cần.

### Imports — Python style

Triết dùng cú pháp Python `from ... import ...` cho selective import, và `import ...` cho whole-module import.

```triet
from crate.foo import bar             // single name
from crate.foo import a, b, c         // multi
from crate.foo import bar as baz      // rename
from std.io import println, print
import std.io                         // import whole module (use as `std.io.println`)
```

**KHÔNG hỗ trợ:**
- Glob imports (`from foo import *`) — vi phạm explicit export principle, làm ABI surface mơ hồ. Có thể revisit ở v1.0+ nếu có ngữ cảnh thuyết phục.
- Re-exports (`public from X import Y` hoặc tương đương) — defer sang v0.3+ khi nhu cầu rõ.

### File resolution

Compiler tìm file theo thứ tự:
1. `module foo` ở `path/to/parent.tri` → tìm `path/to/foo.tri` **hoặc** `path/to/foo/foo.tri`.
2. Inline `module foo { ... }` → không tìm file.

Một module có submodule = directory chứa cả file `foo.tri` (chính module) và children `foo/bar.tri`. Đơn giản hơn Rust 2018 (không có `mod.rs` nữa, tránh nhiều file cùng tên).

```
mypkg/
├── mypkg.tri              # crate root: declares `module foo; module bar`
├── foo.tri                # module `foo` content
├── foo/                   # foo's children
│   ├── inner.tri          # foo.inner
│   └── helper.tri         # foo.helper
└── bar.tri                # module `bar`, no children
```

**Chú ý:** filesystem layout là **convention**, không phải ngữ nghĩa. Compiler chỉ resolve theo `module` declarations. Mapping được thiết kế sao cho:
- Dev mới đọc filesystem hiểu ngay structure (helpful).
- Refactor (đổi tên, di chuyển) chỉ cần sửa `module` declarations + đổi tên file (flexible).

### Cyclic imports

**Cấm.** Compiler error tại name resolution. Diagnostic chỉ rõ chu trình:

```
error[E2100]: cyclic module dependency
   ┌─ crate/foo.tri:3:1
   │
3  │ from crate.bar import B
   │ ^^^^^^^^^^^^^^^^^^^^^^^ creates cycle: foo → bar → baz → foo
```

### Reserved top-level namespaces

Ở v0.2.x các root namespace sau **được giữ chỗ** (compiler từ chối user khai báo):

| Root | Mục đích | Phase enforce |
|---|---|---|
| `std` | Standard library | v0.2.x (đã có) |
| `sys` | Syscall surface | v0.6 (capability) |
| `dev` | Driver / hardware | v0.6 (capability) |
| `usr` | User application | v0.6 (capability) |
| `core` | Minimal stdlib (no_std style) | v1.0+ |

Reserve sớm = không phải break user code khi v0.6 enforce.

## Lý do

### Tại sao verbose keywords?

Triết là **AI-first language**. Mục tiêu: LLM sinh code đúng ngay lần đầu, dev đọc code không phải tra từ điển từ tắt.

- `function` / `public` / `mutable` / `constant` / `module` — dài hơn vài ký tự, nhưng zero ambiguity. `fn` có thể là Function-key, `pub` có thể là pub(lication), `mut` có thể là mutex, `mod` có thể là modulo.
- LLM context tokens dày: verbose keywords chiếm 1–2 BPE tokens, không đắt hơn ký hiệu nhiều.
- Java đã chứng minh hệ sinh thái lớn không bị tắc bởi keyword dài.
- Theo nguyên tắc thiết kế #1 trong VISION.md: "explicit > implicit, regular > exception, keyword > ký hiệu khi mơ hồ, low ambiguity > terseness."

### Tại sao dot path, không `::`?

- `.` đã là field access trong Triết — trải nghiệm nhất quán cho người mới (đặc biệt từ Java/Python/JS).
- `::` là di sản C++ phân biệt namespace với member; Triết dùng resolver hai pha (load → resolve) nên không cần phân biệt cú pháp tại lex.
- Field access và path resolution không ambiguous trong Triết: parser quyết định bởi context (sau `import`/`from`/type annotation = path; sau expression = field). Module path luôn xuất hiện ở vị trí xác định.
- Java/Python/Kotlin/Swift đều dùng `.` ở cả module path và field access, không gặp vấn đề thực tiễn.

### Tại sao Python-style `from X import Y`?

- Tách rõ "module nào" với "tên nào" — dễ đọc dễ refactor.
- Multi-import gói gọn: `from std.io import println, print` súc tích hơn `import std.io.println; import std.io.print`.
- Aliasing cùng cú pháp với selective: `from std.io import println as out` — không cần keyword riêng.
- Khác `import std.io.println` (Java) — yêu cầu viết tên cuối ở lần đầu, ép dev đặt tên rõ.
- Mọi LLM đã thấy hàng triệu dòng Python — sinh code đúng ngay.

### Tại sao Java-style `module foo`?

- Java JPMS (Java 9+) đã chứng minh module declaration là first-class concept, tách hẳn khỏi filesystem.
- Triết áp dụng tinh thần đó: `module foo` là declaration thật trong source, không phải implicit từ thư mục.
- Chấp nhận filesystem layout là convention (xem §"File resolution"), nhưng ngữ nghĩa do `module` keyword quyết định — refactor chỉ cần sửa keyword + đổi tên file.

### Tại sao đơn giản hóa visibility?

Rust có 5 cấp: `pub`, `pub(crate)`, `pub(super)`, `pub(in path)`, private. Đa số code base dùng 80% `pub` + `pub(crate)`. `pub(super)` và `pub(in path)` ít gặp và phức tạp hóa ABI surface.

Triết v0.2.x chỉ cần `public` (export) + `public(package)` (internal) + private (default). Đơn giản dễ học cho LLM-generated code. Có thể mở rộng ở v1.0+ nếu thực sự cần (theo nguyên tắc "stability over speed", thêm dễ hơn xóa).

### Tại sao không glob imports?

Glob imports (`from foo import *`) vi phạm explicit export principle:
- Người đọc không biết tên nào được import.
- Refactor ở `foo` (thêm symbol) có thể shadow accidental ở scope local.
- ABI metadata phải scan toàn bộ `foo` để biết surface.

Cấm ở v0.2.x. **Có thể revisit ở v1.0+** với constraint chặt (ví dụ: chỉ trong test module).

### Tại sao path dùng `crate.` chứ không `pkg.`?

"Crate" đã là terminology trong Triết (workspace có `crates/`). Đổi sang `pkg.` chỉ vì khác Rust là không lý do đủ. Reserve `pkg.` cho concept "Crate-Pack distributable" ở v0.4 (sẽ là thứ khác).

### Tại sao reserve `sys`/`dev`/`usr` từ v0.2.x?

Ba namespace này là core của trụ cột #5 (capability system, v0.6). Reserve sớm:
- Không break user code khi v0.6 ship.
- Định hướng người viết library: stdlib system thì đặt ở `sys`, app thì `usr`.
- Cho phép typecheck cảnh báo sớm (v0.5+) nếu user import sai namespace.

### Cyclic imports — vì sao cấm cứng?

- Cycle phá compile-time: linker không biết khởi tạo theo thứ tự nào.
- Cycle là dấu hiệu thiết kế sai (high coupling).
- Mọi ngôn ngữ system production (Rust, Go, OCaml) đều cấm hoặc warn nặng.

Diagnostic chỉ rõ chu trình giúp dev fix nhanh.

## Alternatives considered

### A1. Filesystem-strict (Java pre-Jigsaw / Python 2)
**Reject.** Refactor unfriendly. Java đã từ bỏ.

### A2. First-class modules (OCaml functor)
**Defer.** Đẹp về lý thuyết (parametric modules) nhưng phức tạp impl + LLM khó học. Có thể bổ sung ở v2.0+ nếu cần thực sự.

### A3. ES modules (file = module, default exports)
**Reject.** Implicit namespace từ filesystem. Default export tốt cho ergonomics nhưng làm ABI surface mơ hồ. Triết ưu tiên explicit.

### A4. Mojo modules
**Reference, không adopt full.** Mojo theo Python module model có một số điểm tham khảo (file = module), nhưng Mojo cũng đang định hình. Chờ Mojo settle trước khi học chi tiết.

### A5. Single-file packages (Go)
**Reject.** Go gộp toàn bộ file trong cùng directory thành 1 namespace. Đơn giản nhưng không hỗ trợ nested namespace tự nhiên.

### A6. Rust-style `::` paths + `mod`/`use`
**Reject.** Tham chiếu C++; trong Triết không có ambiguity giữa namespace và member access nên không cần ký hiệu riêng. Verbose keyword + dot path đẹp mắt và thuận với background Java/Python phổ biến hơn.

## Hậu quả

**Tích cực:**
- Codebase Triết có thể scale tới hàng chục/trăm module mà không name collision.
- Library nội bộ có thể tách `crate.core`, `crate.utils`, `crate.api`.
- Stdlib được tổ chức lại từ flat (`std.io.println` ở v0.2 monolith) thành proper hierarchy (`std.io.println` qua module system).
- ABI surface (v0.4) chỉ cần scan items có `public` — fast.
- Capability enforcement (v0.6) có anchor là top-level namespace.

**Tiêu cực:**
- Verbose keywords dài hơn ký hiệu — accept tradeoff (xem "Tại sao verbose keywords?").
- Người mới đến từ Rust/C++ sẽ thấy thiếu `::` — accept (xem "Tại sao dot path").
- v0.2 đã ship cú pháp `import std.io.println` (đặt tên cuối ngay sau `import`) khác Python `import std.io`. Sẽ chuẩn hóa khi v0.2.x ship module system: cú pháp `from std.io import println` chính thức thay thế cho selective import; `import std.io` (whole module) vẫn giữ.

**Migration strategy:**
- v0.2 baseline: chỉ có `import std.io.println` form (dot-path with terminal name).
- v0.2.x: thêm `from X import Y` form. `import std.io.println` form được giữ làm "import whole sub-path with named tail" tương đương `from std.io import println`.
- v0.3: cú pháp ổn định, không thay đổi thêm.

## Implementation roadmap (v0.2.x)

1. **Lexer:** đã có `module`, `public`, `import`, `crate`, `self`, `super` keywords (ADR-0005 commits). Cần thêm `from`, `as` keywords cho Python-style import.
2. **AST:**
   - `Item::Module { name, content: Either<Inline(Vec<Item>), External> }`.
   - `Item::Import { source: Path, names: Vec<(String, Option<String>)> }` — `from X import a, b as c`.
   - `Item::Import { whole: Path }` — `import X` (whole module).
   - `Item::*` đã có field `visibility: Visibility { Public, PublicPackage, Private }` (commit `7cb63e7`).
   - `Path` AST node phân biệt absolute (`crate.`), relative (`self.`/`super.`), reserved (`std.`/`sys.`/...).
3. **Parser:** `parse_module`, `parse_import`, `parse_visibility` (đã có). Đã có recursive descent + Pratt — extend tự nhiên.
4. **Module loader:** new pass trước typecheck. Build module tree từ root file. Resolve `module foo` → tìm file. Detect cycles.
5. **Name resolver:** new pass trước typecheck. Resolve `from X import Y` paths to absolute. Validate visibility.
6. **Typecheck:** chạy per-module với resolved names. Type definitions + functions cross-module qua name resolver.
7. **Interpreter:** runtime đã có symbol table phẳng — extend thành module-aware (path-based lookup).
8. **CLI:** `dao check` + `dao run` accept root file, tự load module tree.
9. **Stdlib migration:** chuyển `std.io`, `std.text` thành proper module với `module` declarations.
10. **Demo lớn:** viết 1 demo (~500 dòng) chia 5+ module để validate end-to-end.

**Test gate:**
- Tất cả demo `.tri` cũ tiếp tục chạy.
- Demo lớn module-split chạy đúng.
- Snapshot tests cho diagnostics: cyclic import, visibility violation, unresolved path, reserved namespace abuse.
- 50+ unit test mới cho module loader + name resolver.

## Tham chiếu

- [Java Project Jigsaw / JEP 261](https://openjdk.org/projects/jigsaw/) — JPMS module model, baseline cho `module` declaration.
- [Python Language Reference — Imports](https://docs.python.org/3/reference/import.html) — `from X import Y` syntax.
- [Rust Reference — Items: Modules](https://doc.rust-lang.org/reference/items/modules.html) — reference cho hierarchical module tree (visibility, no filesystem binding).
- [OCaml Module System](https://v2.ocaml.org/manual/moduleexamples.html) — first-class modules (defer).
- [TypeScript Modules](https://www.typescriptlang.org/docs/handbook/modules.html) — ES modules (rejected pattern).
- [Mojo Modules](https://docs.modular.com/mojo/manual/packages) — reference.

## Liên quan

- [ADR-0007](0007-ir-design.md) (đã viết, v0.3): IR design — `AbsolutePath` từ module loader là input cho cross-module call ở IR.
- ADR-0009 (sắp viết, v0.4): ABI metadata format — module visibility là input.
- ADR-0012 (sắp viết, v0.5): Hash scheme — module structure ảnh hưởng `iface_hash`.
- ADR-0014 (sắp viết, v0.6): Capability type system — top-level namespace là anchor.

---

*Quyết định này đóng băng module model cho v0.2.x. Breaking change từ phase này về sau cần ADR riêng.*
