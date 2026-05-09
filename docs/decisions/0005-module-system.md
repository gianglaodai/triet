# ADR 0005 — Module system: Hierarchical, Explicit Export, Filesystem-Independent

**Trạng thái:** Quyết định. Áp dụng cho v0.2.x. Đây là trụ cột #2 trong [VISION.md](../../VISION.md).

**Issue:** Triết đã đến giới hạn của single-file program. Code base demo đã 11 file `.tri` ở cùng namespace phẳng. Library nội bộ và phân chia codebase đòi hỏi module system thực sự. Đây cũng là tiền đề kiến trúc cho stable ABI (v0.4), CAS packaging (v0.5), và capability namespaces (v0.6) — module system thiết kế sai sẽ kéo theo phá vỡ ba trụ cột về sau.

## Quyết định

Triết áp dụng **hierarchical module tree theo phong cách Rust**, với **explicit `pub` export** và **không bind cứng vào filesystem**.

### Cú pháp

```triet
// Trong file `pkg.tri` (root của crate `pkg`):
mod foo                            // declare submodule, compiler tìm `foo.tri`
mod bar                            // declare submodule `bar`

pub use crate::foo::Foo            // re-export

// Trong file `foo.tri`:
pub fn hello() -> String =         // exported
    "hello"

fn helper() -> Integer =           // private (default)
    42

mod inline {                       // inline submodule
    pub fn nested() -> Integer = 1
}
```

### Path syntax

| Path | Ý nghĩa |
|---|---|
| `crate::foo::bar` | Absolute path từ crate root |
| `self::foo` | Relative — current module |
| `super::foo` | Relative — parent module |
| `std::io::println` | Stdlib path |
| `sys::*`, `dev::*`, `usr::*` | **Reserved** ở v0.2.x (chưa enforce; v0.6 sẽ enforce capability) |

### Visibility levels

```triet
pub fn open() = ...        // visible everywhere
pub(pkg) fn detail() = ... // visible within same crate-pack only
fn helper() = ...          // private to current module (default)
```

**Triết SIMPLIFIES Rust's visibility:** chỉ 3 cấp (`pub`, `pub(pkg)`, private). Bỏ `pub(super)`, `pub(in path)` để giữ ABI surface đơn giản. Có thể bổ sung ở v1.0+ nếu thực sự cần.

### Imports

```triet
use crate::foo::bar                      // single
use crate::foo::{a, b, c}                // multi
use crate::foo::bar as baz               // rename
use std::io::println
```

**KHÔNG hỗ trợ:**
- Glob imports (`use foo::*`) — vi phạm explicit export principle, làm ABI surface mơ hồ. Có thể revisit ở v1.0+ nếu có ngữ cảnh thuyết phục.
- Nested groups (`use crate::{foo::a, bar::b}`) — defer, không cấp thiết.

### File resolution

Compiler tìm file theo thứ tự:
1. `mod foo` ở `path/to/parent.tri` → tìm `path/to/foo.tri` **hoặc** `path/to/foo/foo.tri`.
2. Inline `mod foo { ... }` → không tìm file.

Một module có submodule = directory chứa cả file `foo.tri` (chính module) và children `foo/bar.tri`. Đơn giản hơn Rust 2018 (không có `mod.rs` nữa, tránh nhiều file cùng tên).

```
mypkg/
├── mypkg.tri              # crate root: declares `mod foo; mod bar`
├── foo.tri                # module `foo` content
├── foo/                   # foo's children
│   ├── inner.tri          # foo::inner
│   └── helper.tri         # foo::helper
└── bar.tri                # module `bar`, no children
```

**Chú ý:** filesystem layout là **convention**, không phải ngữ nghĩa. Compiler chỉ resolve theo `mod` declarations. Mapping được thiết kế sao cho:
- Dev mới đọc filesystem hiểu ngay structure (helpful).
- Refactor (đổi tên, di chuyển) chỉ cần sửa `mod` declarations + đổi tên file (flexible).

### Cyclic imports

**Cấm.** Compiler error tại name resolution. Diagnostic chỉ rõ chu trình:

```
error[E2100]: cyclic module dependency
   ┌─ crate/foo.tri:3:5
   │
3  │ use crate::bar::B
   │     ^^^^^^^^^^^^ creates cycle: foo → bar → baz → foo
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

### Tại sao Rust-style, không Java-style?

**Java pre-Jigsaw filesystem mapping** ràng buộc `package com.example.foo` ↔ `com/example/foo/`. Java 9 (Project Jigsaw) đã CHÍNH THỨC giới thiệu module system mới (`module-info.java`) thừa nhận filesystem-binding làm refactor đau. JPMS tách module identity khỏi package path.

Lessons:
- **Strict filesystem binding làm refactor đau.** Đổi tên một namespace = di chuyển toàn bộ cây thư mục.
- **Explicit export là cần thiết.** Java pre-9 dùng `public` ngầm hiểu là exported, dẫn đến accidental API surface lớn. Module declaration `exports com.example.foo;` của JPMS là đáp án đúng.

Rust mod system chứng minh được:
- Module identity tách khỏi filesystem (mod tree do `mod` keyword đặt, không phải directory).
- Explicit `pub` (mặc định private).
- Path syntax compose tốt với generics, traits, types.
- Refactor-friendly.

→ **Adopt Rust mod model làm baseline.**

### Tại sao đơn giản hóa visibility?

Rust có 5 cấp: `pub`, `pub(crate)`, `pub(super)`, `pub(in path)`, private. Đa số code base dùng 80% `pub` + `pub(crate)`. `pub(super)` và `pub(in path)` ít gặp và phức tạp hóa ABI surface.

Triết v0.2.x chỉ cần `pub` (export) + `pub(pkg)` (internal) + private (default). Đơn giản dễ học cho LLM-generated code. Có thể mở rộng ở v1.0+ nếu thực sự cần (theo nguyên tắc "stability over speed", thêm dễ hơn xóa).

### Tại sao không glob imports?

Glob imports (`use foo::*`) vi phạm explicit export principle:
- Người đọc không biết item nào được import.
- Refactor ở `foo` (thêm symbol) có thể shadow accidental ở scope local.
- ABI metadata phải scan toàn bộ `foo` để biết surface.

Cấm ở v0.2.x. **Có thể revisit ở v1.0+** với constraint chặt (ví dụ: chỉ glob trong `mod tests`, hoặc explicit `use foo::*` only on prelude modules).

### Tại sao path dùng `crate::` chứ không `pkg::`?

"Crate" đã là terminology trong Triết (workspace có `crates/`). Đổi sang `pkg::` chỉ vì khác Rust là không lý do đủ. Reserve `pkg::` cho concept "Crate-Pack distributable" ở v0.4 (sẽ là thứ khác).

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

## Hậu quả

**Tích cực:**
- Codebase Triết có thể scale tới hàng chục/trăm module mà không name collision.
- Library nội bộ có thể tách `crate::core`, `crate::utils`, `crate::api`.
- Stdlib được tổ chức lại từ flat (`std.io.println`) thành proper hierarchy (`std::io::println`).
- ABI surface (v0.4) chỉ cần scan items có `pub` — fast.
- Capability enforcement (v0.6) có anchor là top-level namespace.

**Tiêu cực:**
- Breaking change từ v0.1 cú pháp `std.io.println` (dot-path) → `std::io::println` (path-with-colons). Cần migration script + một version cycle với cả hai cú pháp.
- Người mới đến từ Python/JS sẽ thấy lạ (`::` thay vì `.`). Tradeoff chấp nhận được vì alignment với system languages (Rust, C++, Swift đều dùng `::`).

**Migration strategy:**
- v0.2.x: cả hai cú pháp `std.io.println` và `std::io::println` đều parse, dot-path emit deprecation warning.
- v0.3: dot-path → error với fix-it gợi ý.
- v0.4+: chỉ `::`.

## Implementation roadmap (v0.2.x)

1. **Lexer:** đã có `::` token. Cần `mod`, `use`, `pub`, `as`, `super`, `self`, `crate` keywords.
2. **AST:**
   - `Item::Mod { name, content: Either<Inline(Vec<Item>), External> }`.
   - `Item::Use { path: Path, alias: Option<String> }`.
   - `Item::*` thêm field `visibility: Visibility { Public, PublicPkg, Private }`.
   - `Path` AST node phân biệt absolute (`crate::`), relative (`self::`/`super::`), reserved (`std::`/`sys::`/...).
3. **Parser:** `parse_mod`, `parse_use`, `parse_visibility`. Đã có recursive descent + Pratt — extend tự nhiên.
4. **Module loader:** new pass trước typecheck. Build module tree từ root file. Resolve `mod foo;` → tìm file. Detect cycles.
5. **Name resolver:** new pass trước typecheck. Resolve `use` paths to absolute. Handle re-exports. Validate visibility.
6. **Typecheck:** chạy per-module với resolved names. Type definitions + functions cross-module qua name resolver.
7. **Interpreter:** runtime đã có symbol table phẳng — extend thành module-aware (path-based lookup).
8. **CLI:** `triet check` + `triet run` accept root file, tự load module tree.
9. **Stdlib migration:** chuyển `std.io`, `std.text` thành `std::io`, `std::text` proper modules.
10. **Demo lớn:** viết 1 demo (~500 dòng) chia 5+ module để validate end-to-end.

**Test gate:**
- Tất cả demo `.tri` cũ tiếp tục chạy (qua dot-path compat).
- Demo lớn module-split chạy đúng.
- Snapshot tests cho diagnostics: cyclic import, visibility violation, unresolved path, reserved namespace abuse.
- 50+ unit test mới cho module loader + name resolver.

## Tham chiếu

- [Rust Reference — Items: Modules](https://doc.rust-lang.org/reference/items/modules.html) — model chính.
- [Java Project Jigsaw / JEP 261](https://openjdk.org/projects/jigsaw/) — bài học từ Java.
- [OCaml Module System](https://v2.ocaml.org/manual/moduleexamples.html) — first-class modules (defer).
- [TypeScript Modules](https://www.typescriptlang.org/docs/handbook/modules.html) — ES modules (rejected pattern).
- [Mojo Modules](https://docs.modular.com/mojo/manual/packages) — reference.

## Liên quan

- ADR-0008 (sắp viết, v0.4): ABI metadata format — module visibility là input.
- ADR-0011 (sắp viết, v0.5): Hash scheme — module structure ảnh hưởng `iface_hash`.
- ADR-0013 (sắp viết, v0.6): Capability type system — top-level namespace là anchor.

---

*Quyết định này đóng băng module model cho v0.2.x. Breaking change từ phase này về sau cần ADR riêng.*
