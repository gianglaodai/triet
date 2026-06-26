# ADR 0071 — Path Separator (`::`) & Module Import (`use`)

**Trạng thái:** **🔒 SEALED** (Mentor G ký 2026-06-26; O verify máu Lát 1 (4 teeth) + Lát 2 (5 teeth) đỏ độc lập + restore byte-identical). Áp dụng cho rewrite-era (Bậc C). **Supersede [ADR-0005](0005-module-system.md)** phần dot-path + Python-import: đảo `.`→`::` cho đường tĩnh, giết `import`/`from`→`use`. KHÔNG revisionism: ADR-0005 status→`Superseded by ADR-0071`, giữ nguyên thân.

**Issue:** ADR-0005 chọn `.` làm path-separator (Java/Python instinct) và CHỦ ĐỘNG bác `::` với lý lẽ "resolver hai pha nên không cần phân biệt cú pháp tại lex" (ADR-0005 §"Tại sao dot path"). Hệ quả thực tế: `.` (`Token::Dot`) **gánh chồng** ba việc — path-separator (`std.io`), enum-variant (`Color.Red`), field-access (`obj.field`) — và parser KHÔNG phân biệt được; nó đẩy hết cho **typecheck** đoán (`expr_resolutions` map ghi `Color.Red`=FieldAccess→variant, `Color.Red(x)`=MethodCall→variant). AST mờ, không nói lên ngữ nghĩa. Khi xây Module Resolution + Trait, sự mờ này thành nợ.

**Lý do đảo (G, 2026-06-25):** *"Phân tách rõ ràng Static Resolution (`::`) và Instance Access (`.`) tại tầng Cú pháp (Syntax/AST), dọn đường cho Trait và Module Resolution."*

---

## Quyết định

### 1. Hai toán tử truy cập rạch ròi tại CÚ PHÁP
- **`::` — Static Resolution** (giải tại compile-time, không có receiver instance): namespace/module path, type, **enum variant** (`Color::Red`).
- **`.` — Instance Access** (có receiver động): field (`obj.vga`), method (`hw.use_vga()`), tuple-index (`t.0`), safe-chain (`obj?.field`).

Parser phân loại AST node **chỉ bằng token**, không nhờ typecheck đoán. `Color.Red` (dot trên type-variant) → **lỗi parse/typecheck**, không còn "đoán đúng".

### 2. Giết `import`/`from`/glob → `use`
`from X import a` bẻ một path `X::a` làm hai mảnh nhét giữa 2 keyword. Path phải là **một khối thống nhất**. Cú pháp mới:
```triet
use std::io::println;      // import một item
use std::io;               // import whole module (dùng như std::io::println)
use std::io::{a, b, c};    // brace-group multi-import (LOCKED — thay from..import a,b,c)
use std::io::out as o;     // rename giữ `as` (sau path/trong brace)
```
Keyword `import`, `from` **bị xóa khỏi lexer**. Glob `*` vẫn cấm (ADR-0005 §exclusions giữ nguyên).

### 3. Enum variant qua `::` — TÁI DÙNG node sẵn có (KHÔNG node mới)
- `Color::Red` → `Expr::EnumLiteral { name: "Color", variant_name: "Red", payload: None }` (node `EnumLiteral` đã tồn tại, nay nhận qualified form).
- `Color::Red(x)` → `Expr::EnumLiteral { ..., payload: Some(x) }`.
- Pattern `Color::Red` / `Color::Red(x)` → `Pattern::EnumVariant { name: Some("Color"), variant_name, payload }` (trường `name` hiện luôn `None`, nay điền tại parse).
- **Gỡ hack typecheck**: `check/exprs.rs` không còn ghi `expr_resolutions` cho `FieldAccess(Type,field)`/`MethodCall(Type,m)`→variant.
- **BẮT BUỘC QUALIFY (G+Giang LOCK 2026-06-25):** mọi user-enum variant phải `Type::Variant` (expr + pattern). **Bare `Red`/`None` không qualify → lỗi.** Hệ quả SẠCH: một bare identifier trong pattern position nay **chắc chắn là variable-binding**, không còn nhập nhằng variant-vs-binding → `pattern.rs` + typecheck pattern mỏng đi (`Pattern::EnumVariant.name` luôn `Some`, không còn đường bare-name→variant). *(Lưu ý phạm vi: chỉ user-enum; `~0`/`~+`/`~-` (Outcome) và `true`/`false`/`unknown` (Trilean) là literal/toán tử, KHÔNG phải variant — không đụng.)*

### Phạm vi đã đo (recon file:line)
| Tầng | Điểm | Lát |
|---|---|---|
| Lexer | `token.rs:151/155/159` keyword import/from/as; thêm `ColonColon` cạnh `Colon:369`/`Dot:377` | 1+2 |
| Parser import | `item.rs:48/72` dispatch · `parse_import:451` · `parse_from_import:473` · `parse_dot_path:571` · `parse_import_name:509` (`as`) | 1 |
| AST Item | `Item::Import`/`Item::ImportFrom` (schema-gen `ast_item.rs:159/175`) → hợp nhất `Item::Use` (schema-first) | 1 |
| Resolver | `resolver.rs:145 collect_imports` · `208 resolve_whole_import` · `293 resolve_from_import` | 1 |
| Parser expr | `expr.rs:725 parse_postfix` · `879 parse_dot_postfix` (FieldAccess/MethodCall) | 2 |
| Typecheck variant-hack | `check/exprs.rs:182-201` (MethodCall→variant) · `1567-1601` (FieldAccess→variant) | 2 |
| Parser pattern | `pattern.rs:63-84` (điền `EnumVariant.name`) | 2 |
| `.tri` sweep | import: 1 example + 22 fixtures · `Type.Variant`: quét toàn corpus | 1+2 |
| Docs | SPEC.md, CLAUDE.md bảng §Language convention, ADR-0005 status | 1+2 |

### Phân lát (mỗi lát O verify máu + G ký)
- **Lát 1 — `use` + `::` import path.** ✅ ĐÓNG (`4a7da96`). Lexer `::`+`use`/giết import/from; parser use-path; AST `Item::Use` (schema→codegen→consumers); resolver; sweep import `.tri`+docs.
- **Lát 2 — Expr/Pattern static `::` (READING A — G phán 2026-06-26: giết KHÔNG THA).** `Color::Red`(+payload) via EnumLiteral; pattern `name:Some`; **giết CẢ BA cơ chế bare/dot-variant ngầm**; bare un-qualified → lỗi MỌI NƠI; sweep toàn corpus.

#### §2.A — Lát 2 Reading A: ba cơ chế "variant ngầm" bị khai tử
Triết hôm nay nhận user-enum variant qua BA đường ngầm — Reading A giết cả ba, chỉ chừa `::` qualified + `use`-import:
1. **Pattern guess-hack** (`check.rs:892-918`): bare `Red` trong match arm → đoán theo *scrutinee*. → GIẾT. Sau đó bare-trong-pattern = **variable binding 100%, luôn luôn**.
2. **Expr in-scope-enum-scan** (`check/exprs.rs:1216 resolve_enum_variant` + 2 call-site dòng 101 Identifier, 145-150 Call-bare): bare `Green` → *quét MỌI enum in-scope* tìm variant trùng tên. → GIẾT. Đây là implicit-magic (G: "tư duy bãi rác"). Import-bound (`use X::{Ok}`) KHÔNG đụng đường này — nó qua `env.lookup` (exprs.rs:91) TRƯỚC, nên giết scan KHÔNG đập import-bound.
3. **Dot-variant hacks** (3 site): `check/exprs.rs:182-201` MethodCall→variant (`Color.Red(x)`) · `check_field_access` FieldAccess→variant (`Color.Red`) · `exprs.rs:152-172` Call-FieldAccess (`CD.SomeInt(5)`). → GIẾT cả ba. `.`-variant = lỗi (FieldAccess trên enum-TYPE, không có field).

**Hệ quả — enum-match nhận Variable-arm làm catch-all:** giết ① phơi ra một lỗ guess-hack vẫn che: path "Variable-binding trên enum-match" chưa từng chạy (hack nuốt mọi bare-ident-arm thành variant trước). Để "bare = binding 100%" thành thật, enum-match phải nhận `Pattern::Variable` làm catch-all (bind cả scrutinee + default-case) — **đối xứng `has_scalar_catch_all` mà scalar-match ĐÃ có (ADR-0064 §8)**. Đây là *thực thi* Reading A, KHÔNG phải feature mới. Sửa 2 nơi: `check_enum_exhaustiveness` short-circuit (thêm Variable cạnh Wildcard) + lower enum-match else-branch (default-case + bind). Bare-ident TRÙNG tên variant (`match c { Green => }`) nay = binding cả `c`, KHÔNG match variant Green.

**Hệ quả — E1018 AmbiguousEnumVariant KHAI TỬ:** E1018 chỉ phát từ `resolve_enum_variant` (đường ②). Giết ② → E1018 vô-nhân (bare illegal → không thể mơ hồ). Gỡ variant `TypeError::AmbiguousEnumVariant` + emitter + help-text (no-dead-code). E1018 退役 — không tái dụng mã. Sau Reading A: **mọi user-variant reference NÊU TÊN enum** (`Color::Red`) HOẶC là import-bound symbol (`use`); không có đường thứ ba.

### Phạm vi đã đo (recon file:line)
| Tầng | Điểm | Lát |
|---|---|---|
| Lexer | `token.rs` keyword import/from/as; `ColonColon`+`use` | 1 ✅ |
| Parser import | `item.rs` dispatch · `parse_use*` thay `parse_import/from/dot_path` | 1 ✅ |
| AST Item | `Item::Import`/`ImportFrom` → `Item::Use` (schema-first) | 1 ✅ |
| Resolver | `resolver.rs collect_imports` route Item::Use | 1 ✅ |
| Parser expr/pattern | `::` primary-level → EnumLiteral / Pattern::EnumVariant{name:Some} | 2 |
| Typecheck giết ① | `check.rs:892-918` pattern guess-hack | 2 |
| Typecheck giết ② | `check/exprs.rs:1216 resolve_enum_variant` + call-site 101/145 + E1018 retire | 2 |
| Typecheck giết ③ | `check/exprs.rs:182-201` + `check_field_access` + `152-172` | 2 |
| `.tri` sweep | bare/dot variant ~25 fixtures + ~13 expr-construction + examples | 2 |
| Docs | SPEC enum, CLAUDE.md bảng (`Color.Red`→`::`), E1018 retire | 2 |

---

## Các phương án đã cân nhắc

| # | Phương án | Ưu | Nhược | Kết luận |
|---|-----------|---|-------|----------|
| 1 | **PA-B Rust-model: `::` static / `.` instance + `use`** (chọn) | Parser phân loại AST bằng token; gỡ hack typecheck; dọn đường Trait/Module | Sweep diện rộng; đảo ADR khóa | **CHỌN** (G lock) |
| 2 | PA-A: `::` chỉ ở `import`, call-site giữ `.` | Sweep nhỏ | `std.io.x` vẫn lẫn path/field → parser vẫn phải đoán = KHÔNG trong sáng | Bác (G: "rác rưởi thỏa hiệp") |
| 3 | Thêm `Expr::Path` node tổng quát đa-segment | Tổng quát call-path tương lai | Qualified call CHƯA implement → YAGNI; node mới = schema+lower+typecheck nặng | Defer tới khi có qualified call |
| 4 | Giữ `from..import` thêm `use` song song | Không phá cũ | Hai cú pháp = mờ AST y như cũ | Bác (G: giết for trót) |

---

## Hậu quả

### Tích cực
- AST nói đúng ngữ nghĩa: static node (EnumLiteral/Use) vs instance node (FieldAccess/MethodCall) phân định tại parse.
- Gỡ được hack typecheck `expr_resolutions` cho dot-variant → typecheck mỏng hơn.
- Path là một khối `::` → nền sạch cho Module Resolution + Trait path tương lai.

### Tiêu cực
- Sweep diện rộng corpus `.tri` + docs (đảo cú pháp nền).
- Đảo một ADR LOCKED — phải supersede tường minh, cập nhật mọi tài liệu nói "dot path".

### Rủi ro cần mitigate
- **`::` vs `:` lexer**: longest-match, `:` (type annotation) không được nuốt nhầm. Teeth: `let x: Integer` còn parse đúng.
- **Regression instance-access**: `obj.field`/`obj.method()`/`t.0`/`obj?.field` PHẢI giữ `.`. Teeth regression.
- **Schema-first**: `Item::Use` đổi qua schema+codegen, KHÔNG hand-edit generated.

---

## Poison-teeth matrix (G mandate)

| Tooth | Input | Phải |
|---|---|---|
| T-dot-path | `use std.io::x` (dot trong path) | **parse error** |
| T-old-import | `import std::io` / `from std::io import x` | **parse error** (keyword đã xóa) |
| T-dot-variant | `Color.Red` (expr) | **lỗi** (không còn resolve thành variant) |
| T-colon-variant | `Color::Red` / `Color::Red(x)` | parse → EnumLiteral → chạy |
| T-bare-variant | bare `Red` (user-enum, không qualify) | **lỗi** (bắt buộc qualify) |
| T-use-ok | `use std::io::println;` | parse OK + resolve |
| T-brace-use | `use std::io::{a, b};` | parse 2 binding |
| R-field (regression) | `obj.field`, `hw.use_vga()`, `t.0`, `obj?.field` | giữ `.`, chạy như cũ |
| R-bind (regression) | bare ident trong pattern (`match x { y => }`) | variable-binding, KHÔNG variant |
| R-colon-annot (regression) | `let x: Integer = 1` | `:` annotation parse đúng |

---

## Ngày hiệu lực
- Rewrite-era Bậc C — kích hoạt từng lát khi WO đóng (O verify + G ký).
- **Supersede ADR-0005** §dot-path + §Python-import: ADR-0005 status→`Superseded by ADR-0071`, thân giữ nguyên (lịch sử).
- Áp dụng cho mọi `.tri` mới; corpus cũ sweep trong campaign. KHÔNG có chế độ tương thích `.`-path (giết for trót).

## Quyết định bổ sung (G+Giang chốt 2026-06-25)
1. **Brace-group LOCKED:** `use a::b::{x, y};` giữ multi-import (thay `from..import a,b,c`). Rename trong brace: `{x, y as z}`.
2. **Bắt buộc qualify LOCKED:** mọi user-enum variant `Type::Variant`; bare `Red`→lỗi (xem §3). Sweep rộng hơn (qualify cả match-arm) nhưng pattern parsing sạch hơn (bare-ident = binding chắc chắn).
