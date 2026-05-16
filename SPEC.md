# Triết — Đặc tả ngôn ngữ v0.6

> Triết (哲) là một ngôn ngữ lập trình **balanced ternary, AI-first**, với tham vọng **đủ năng lực viết hệ điều hành** khi phần cứng tam phân xuất hiện. Lấy cảm hứng từ Setun (Liên Xô, 1958) và logic Łukasiewicz Ł3 (1920).
>
> Tài liệu này đặc tả semantics ngôn ngữ. Tầm nhìn dài hạn ở [VISION.md](VISION.md), lộ trình triển khai phasing ở [ROADMAP.md](ROADMAP.md), quyết định kiến trúc ở [docs/decisions/](docs/decisions/).

---

## 0. Triết lý thiết kế

### 0.1 Năm trụ cột kiến trúc

Triết được thiết kế quanh **năm trụ cột** (chi tiết: [VISION.md](VISION.md)):

1. **CAS Packaging** — hash-based module identity (Unison-inspired). Phase v0.5 ([ADR-0014](docs/decisions/0014-hash-scheme-refinement.md), [ADR-0015](docs/decisions/0015-package-store-layout.md)).
2. **Module System** — hierarchical, explicit `public` export, không bind filesystem. Phase v0.2.x ([ADR-0005](docs/decisions/0005-module-system.md)).
3. **Stable ABI** — witness tables cho cross-package generics, refuse-to-link với diff rõ ràng. Phase v0.4.
4. **Crate-Pack & Hybrid Linking** — binary distribution với metadata, static + dynamic linking. Phase v0.4.
5. **OS-Native Capability Namespaces** — `sys.*`/`dev.*`/`usr.*` enforce ở compiler. Trit-level capability + Łukasiewicz `Unknown` runtime policy. Phase v0.6 ([ADR-0016](docs/decisions/0016-capability-type-system.md), [ADR-0017](docs/decisions/0017-trilean-policy-hook.md), [ADR-0018](docs/decisions/0018-capability-loader-semantics.md)).

### 0.2 Bản sắc Triết

Ba điều khiến Triết không thể bị thay thế bằng tổ hợp ngôn ngữ khác:

- **Trit-level capability** — 3-state native (`-1` deny / `0` ambient / `+1` grant), không emulate qua boolean.
- **Łukasiewicz capability checking** — `Trilean.Unknown` resolved bởi runtime policy, không cần bolt-on policy engine.
- **Tam phân ABI ổn định bẩm sinh** — Trit/Tryte/Integer/Long fixed-size, không struct padding, không endianness ambiguity.

### 0.3 Nguyên tắc thiết kế (commit hard)

1. **AI-first.** Cú pháp và semantics tối ưu cho việc LLM sinh code đúng ngay lần đầu. Ưu tiên: explicit > implicit, regular > exception, keyword > ký hiệu khi mơ hồ, low ambiguity > terseness.
2. **Tam phân là first-class.** Trit, balanced ternary arithmetic, và logic 3 giá trị Łukasiewicz là kiểu/phép toán nguyên thủy — không phải library bên trên hệ nhị phân.
3. **Production-grade ở Ł3, mở rộng được tới Ł∞.** v0.2 dùng giá trị rời rạc 3 mức {-1, 0, +1}. Đường tiến hóa tới logic vô hạn giá trị (fuzzy/probabilistic) phải không đập bỏ semantics hiện tại.
4. **Stability over speed.** Quyết định kiến trúc có ADR. Không "ship đại rồi sửa". Pace dài hạn 5–10 năm cho v3.0 (microkernel POC).
5. **Refuse over guess.** Khi compiler không chắc → error rõ ràng, không suy luận im lặng.
6. **Explicit > implicit.** Export, capability, dependency, ABI surface — tất cả tường minh. Glob imports, default-public, ambient capabilities — bị cấm.

### 0.4 Phạm vi v0.2 (đã ship)

- Pipeline lexer → parser → typecheck → tree-walking interpreter end-to-end.
- Kiểu nguyên thủy: `Trit`, `Tryte`, `Integer` (27 trit), `Long` (81 trit), `Trilean`, `String`, `Unit`.
- Logic Łukasiewicz Ł3 (default) + Kleene K3 (alternative).
- Struct, enum + generics (type parameters trên type definitions).
- Nullable subtyping `T ⊂ T?` bẩm sinh tam phân (1-trit discriminator, [ADR-0001](docs/decisions/0001-nullable-memory-layout.md)).
- F-string interpolation ([ADR-0002](docs/decisions/0002-fstring-format-spec.md)).
- Iterator protocol ([ADR-0003](docs/decisions/0003-iterator-protocol.md)).
- Multi-line string indent ([ADR-0004](docs/decisions/0004-multiline-string-indent.md)).
- Diagnostic format (miette, error codes E0000–E2106).

### 0.5 Đã ship ở v0.5

CAS packaging trên nền tảng v0.4 ABI metadata. v0.5 land:
- 3-cấp hash tree (term + module + package) per [ADR-0014](docs/decisions/0014-hash-scheme-refinement.md). `abi_version` 1 → 2.
- Package store layout `~/.triet/store/{term,mod,pkg,names,roots,tmp}/` per [ADR-0015](docs/decisions/0015-package-store-layout.md). Atomic install protocol.
- Hash-based resolver + `triet.lock` format (hand-rolled line format, no serde dep).
- CLI: `triet store {import,list,gc}`.
- Shared loading demo — VISION §3.1 gate hit at iface level (body-level RAM dedup chờ lowerer per-term body extraction).
- Cross-module enum variant import (`from std.result import Ok, Err`) — pre-existing gap từ v0.2.x closed; aliased variant imports rejected với E2107.

### 0.6 Đã ship ở v0.6

Capability system — trụ cột bản sắc #5 (VISION §3.5 + §5). 3 ADRs ([0016](docs/decisions/0016-capability-type-system.md), [0017](docs/decisions/0017-trilean-policy-hook.md), [0018](docs/decisions/0018-capability-loader-semantics.md)) lock the contract; 11 sub-tasks land the machinery:

- **Capability declaration:** namespace-level claims in `triet.package` source manifest (ADR-0018 §1). 4-state `CapabilityLevel`: `Grant` / `Ambient` / `Deny` (Trit) + `Defer` (`Trilean::Unknown`).
- **Wire format:** `caps section` of `.tripack` ABI metadata populated (ADR-0016 §4). `abi_version` stays `2` (no bump — slot was reserved since v0.4 per ADR-0011 §5).
- **Compile-stage enforcement** (`triet-typecheck::check_capabilities`): cross-root imports (`sys.*`/`dev.*`/`usr.*`) require manifest claim. E2200 `MissingCapabilityClaim` + E2201 `SelfContradictoryCapability`. `std.*`/`core.*` ambient (skip); `crate.*`/`self.*`/`super.*` intra-pkg (skip).
- **Link-stage enforcement** (`triet-pack::check_link_capabilities`, ADR-0018 §2 Step 6a): root manifest is sole authority over the dep closure (ADR-0016 §7). E2202 `UnresolvedCapabilityPath`, E2203 `CapabilityRefused`. `Defer` collected into `CapabilityLinkReport::deferrals`.
- **Runtime resolution** (`triet-pack::CapabilityResolver`, ADR-0017 §4 + ADR-0018 §2 Step 6b): `triet.policy` rules indexed by `(cap_path, origin)` with exact > wildcard precedence. Per-session cache, monotonicity invariant (ADR-0017 §5). E2205 sub-variants for runtime errors.
- **TTY prompt** (`triet-pack::DevTtyPrompt`, ADR-0018 §4 + ADR-0017 Addendum §B): `/dev/tty` paired I/O bypassing stdin/stderr (anti-spoofing). Full 64-hex hashes (no truncation — security context). ASCII `!!` markers. `G`/`D` permanent-write via atomic `PolicyRules::save`.
- **Error namespace:** `triet::capability::E22XX` (E2200–E2208 + sub-variants) — distinct from `triet::pack::E23XX` (semver linker) and `triet::modules::E21XX` (resolver).
- **Demo + capstone test:** [`demos/04-capability-system/`](demos/04-capability-system/) illustrative files + [`capability_pipeline.rs`](crates/triet-typecheck/tests/capability_pipeline.rs) executable proof for the three ROADMAP §v0.6 gates.

### 0.7 Non-goals của v0.6

Các thứ sau được phasing rõ ràng vào version cụ thể, **KHÔNG** thuộc v0.6:

- **CLI wiring** (`triet check` reading `triet.package` from project root, `triet build` populating `.tripack` caps section from manifest, loader integration with `DevTtyPrompt`) — needs project-layout discovery convention; lands cleaner với v0.7 self-hosting.
- **Hiệu năng tối ưu** — production runtime AOT đến v2.0 (LLVM). VM + tree-walker đều là development tiers per [VISION §4.3](VISION.md).
- **Concurrency/async** — phase v0.8.
- **Lowerer emit `WitnessCall` cho cross-package generics** — defer khỏi v0.6. Cần package-aware `ResolvedProgram` + generic-instantiation tracking; multi-week architectural milestone. Lands cùng multi-package compile path hoặc v0.7 self-hosting.
- **v=1 `.tripack` lossy migration** (ADR-0015 §9) — defer. Hiện chưa có v=1 packs trong wild.
- **Self-hosting compiler** — phase v0.7.
- **Per-function capability granularity** — defer post-v1.0 (ADR-0016 "Không làm"). Workaround: stdlib author splits modules.
- **Wildcard cap claims** (`sys.* grant`) — refuse-over-guess; explicit > implicit (ADR-0016).
- **Windows ConPTY** for TTY prompt — POSIX-first; Windows defer.
- **ANSI colour + Unicode box-drawing** in TTY prompt — usability win, defer post-security-floor.
- **JIT** — phase v0.9 (Cranelift backend đọc cùng Triết IR). **Native AOT compile** — phase v2.0 (LLVM backend đọc cùng Triết IR). **Trytecode native** — phase v∞ khi phần cứng tam phân xuất hiện.
- **FFI với C/Rust runtime** — `.tripack` format đã ready host FFI signatures, wire encoding cho FFI thunks defer.
- **Distributed registry** — local store đủ; network fetch + signature/provenance là v1.0+ work.

---

## 1. Cấu trúc từ vựng (Lexical structure)

### 1.1 Mã hóa nguồn

Mã nguồn UTF-8. Ký tự không phải ASCII được phép trong identifier (theo Unicode UAX #31), comment, và string literal.

### 1.2 Comment

```triet
// Comment một dòng
/// Doc comment cho item ngay sau (function, type, constant)
/* Comment khối,
   có thể nhiều dòng, /* lồng được */ */
```

### 1.3 Identifier

```
identifier = (letter | "_") (letter | digit | "_")*
```

`letter` theo Unicode `XID_Start`, `digit` theo Unicode `XID_Continue`. Dấu Việt được phép: `số_trit`, `tính_giá_trị` đều hợp lệ.

### 1.4 Keyword (đã reserve)

```
function  let  mutable  constant  type  if  else  match  return
true  false  unknown  not  and  or  xor  iff  implies
kleene_implies  kleene_xor  kleene_iff
Trit  Tryte  Integer  Long  Trilean  String
import  module  public  owned
struct  enum
```

> **v0.2.x reserves** (per [ADR-0005](docs/decisions/0005-module-system.md), enforcement landing incrementally):
> - Path keywords: `crate`, `self`, `super`
> - Import keywords: `from`, `as` (Python-style imports, ADR-0005)
> - Reserved namespace roots: `std`, `sys`, `dev`, `usr`, `core`

### 1.5 Literal

#### 1.5.1 Số nguyên

Mặc định mọi literal số nguyên thuộc kiểu `Integer` (27 trit, ~±3.8 ngàn tỷ).

```triet
42                  // Integer 42
-17                 // Integer -17
1_000_000           // dùng _ để nhóm
0t+0-+              // balanced ternary literal — đọc trái sang phải, MSB trước
                    // = (+1)(0)(-1)(+1) base 3 = 27 + 0 - 3 + 1 = 25
```

Type suffix bắt buộc khi muốn kiểu khác:

```triet
5_tryte             // Tryte (9 trit)
1_000_000_000_long // Long (81 trit)
1_trit              // Trit
```

Literal balanced ternary `0t...` chỉ chấp nhận ký tự `+`, `0`, `-`, `_`. Không có biểu diễn `0b`, `0x`, `0o` (nhị phân, hex, bát phân) — Triết là ngôn ngữ tam phân first, các hệ cơ số khác không phải nguyên thủy.

#### 1.5.2 Trilean

```triet
true        // +1
false       // -1
unknown     // 0
```

#### 1.5.3 String (string)

```triet
"Hello, thế giới!"
"Ngắt dòng:\n và tab:\t"
"""
String nhiều dòng,
không cần escape "ngoặc kép" bên trong.
"""
```

Escape sequences: `\n`, `\t`, `\r`, `\\`, `\"`, `\u{XXXX}` (Unicode codepoint hex).

#### 1.5.4 String interpolation (f-string)

Prefix `f` bật interpolation. String thường (không có `f`) là literal nguyên bản.

```triet
let n: Integer = 42
let msg: String = f"Câu trả lời là {n}"
let calc: String = f"Tổng: {a + b}, gấp đôi: {(a + b) * 2}"

// Định dạng explicit (qua trait Display):
let pretty: String = f"Giá: {price:#.2}"     // số thập phân, 2 chữ số
```

Quy tắc: chỉ string có prefix `f` mới interpret `{expr}`. Đảm bảo string không có `f` luôn nguyên bản — không hallucinate khi LLM gen code có chứa `{` `}` ngẫu nhiên.

Escape `{` và `}` trong f-string: dùng `{{` và `}}`.

**Nested f-strings không được phép.** Một f-string không thể chứa f-string khác trong phần interpolation. Cần thiết: tách thành biến trung gian. (Quyết định cứng — không có plan cho phép trong tương lai gần; xem [ADR-0002](docs/decisions/0002-fstring-format-spec.md).)

**Implementation note (informative):** Lexer dùng *mode stack* (như rustc/Swift/Python 3.12+) để xử lý f-string. Khi gặp `f"`, lexer push mode `FString`; trong mode này, text được emit thành `FStringText` cho đến khi gặp `{` (push mode `Interpolation`) hoặc `"` (pop, end f-string). Trong mode `Interpolation`, lexer hoạt động bình thường, đếm độ sâu ngoặc nhọn; `}` ở depth 0 đóng interpolation. Cách này tránh được mọi vấn đề scan ngây ngô (string `"}"` bên trong, block `{ ... }` bên trong, span tracking sai lệch).

---

## 2. Hệ thống kiểu (Type system)

### 2.1 Kiểu nguyên thủy

| Kiểu | Số trit | Phạm vi | Mô tả |
|---|---|---|---|
| `Trit` | 1 | `{-1, 0, +1}` | Đơn vị thông tin tam phân cơ bản |
| `Tryte` | 9 (= 3²) | `±9_841` | Số nguyên nhỏ |
| `Integer` | 27 (= 3³) | `±3_812_798_742_493` | **Số nguyên mặc định** |
| `Long` | 81 (= 3⁴) | `±2.21 × 10³⁸` | Số nguyên lớn — **deferred v0.2** |
| `Trilean` | 1 | `{false, unknown, true}` | Logic 3 giá trị |
| `String` | — | UTF-8 string | Chuỗi văn bản |
| `Unit` | — | `()` | Không có giá trị (giống void) |

`Trit` và `Trilean` đều là 1-trit về biểu diễn nhưng **khác kiểu** ở mức ngôn ngữ — `Trit` là số (`-1`, `0`, `+1`), `Trilean` là chân lý (`false`, `unknown`, `true`). Conversion phải explicit (xem §2.4).

> **Note:** `Long` dùng big-integer backing (`bnum::I256`) trong interpreter v0.2 vì phạm vi vượt quá `i128::MAX` (~1.7×10³⁸). Backend tam phân native (v2.0+) sẽ map trực tiếp sang 81 trit hardware.

#### Quy ước đặt tên: tam phân first

Các kiểu trên đều **ngầm là tam phân**. Triết là ngôn ngữ tam phân first, nên `Integer` mặc nhiên có nghĩa "số nguyên 27 trit" — không cần prefix `Ternary`.

Khi (v0.2+) cần interop với kiểu nhị phân, kiểu nhị phân phải mang prefix `Binary` rõ ràng:
- `BinaryInteger` (32 bit, ánh xạ với i32)
- `BinaryLong` (64 bit, i64)
- `BinaryByte` (8 bit, u8)

Đây là một statement triết học: trong Triết, **nhị phân là ngoại lệ phải đánh dấu**, ngược với phần còn lại của ngành lập trình hiện nay.

### 2.2 Đóng gói trong bộ nhớ

5 trit nhồi trong 1 byte (3⁵ = 243 < 256, lãng phí 1.5%). Word size đặt ngay trong số nguyên byte:

| Kiểu | Trit | Bytes | Bit lãng phí |
|---|---|---|---|
| `Trit` | 1 | 1 | 6 bit |
| `Tryte` | 9 | 2 | 16 - 9·log₂3 ≈ 1.7 bit |
| `Integer` | 27 | 6 | 6 bit |
| `Long` | 81 | 17 | ~6 bit |

Chi tiết encoding xem §3.4.

### 2.3 Type alias

```triet
type Confidence = Integer
type Username = String
```

Alias không tạo kiểu mới, chỉ là tên thay thế (giống `type` của Rust, không như `newtype`).

### 2.4 Conversion

Tất cả conversion giữa kiểu **explicit**. Không có implicit coercion.

```triet
let x: Tryte = 5_tryte
let y: Integer = x.to_integer()                       // Tryte → Integer (không mất mát)
let z: Tryte = (1000).to_tryte()                      // Integer → Tryte, panic nếu tràn
let w: Option<Tryte> = (1000).try_to_tryte()           // → Unknown nếu tràn
let v: Trilean = Trilean.from_trit(t)                 // Trit → Trilean
let t: Trit = b.to_trit()                             // Trilean → Trit (false→-1, unknown→0, true→+1)
```

Khi cần chuyển đổi với hành vi overflow chuyên biệt (saturating, truncating), dùng *narrowing conversion methods* tương tự arithmetic:
- `to_tryte()` — panic on overflow (default)
- `to_tryte_and_saturate()` — kẹp tại biên Tryte
- `to_tryte_and_truncate()` — cắt cụt trit cao
- `try_to_tryte()` — trả về `Option<Tryte>`

### 2.5 Nullable types `T?`

Bất kỳ kiểu nào cũng có thể "có thể null" qua hậu tố `?`:

```triet
let name: String? = get_name()       // có thể là null
let count: Integer? = parse_number(s) // null nếu parse fail
```

`T?` là **kiểu khác** với `T`. Compiler ép xử lý null trước khi dùng giá trị:

```triet
let n: Integer? = ...
n + 1                       // ❌ lỗi compile: phải kiểm tra null trước
```

#### Toán tử và pattern cho nullable

```triet
// Smart cast qua kiểm tra
if name != null {
    std.io.println(name)    // name lúc này được narrow về String
}

// Safe call: trả về null nếu chuỗi bị null ở đâu đó
let len: Integer? = name?.length

// Elvis: thay null bằng giá trị mặc định
let len: Integer = name?.length ?: 0

// Force unwrap: panic nếu null
let must: String = name!!

// match trên null
match name {
    null => "khuyết danh",
    n    => f"xin chào {n}",
}
```

#### `T?` là **PRIMARY** cho "value may be absent"

Quyết định kể từ v0.4: **`T?` là cách chính tắc** để diễn đạt "giá trị có thể vắng mặt". `Option<T>` (như một stdlib enum riêng) **không còn được khuyến khích** vì redundant — `T?` đã có discriminator 1-trit bẩm sinh (Trit::Zero = null per ADR-0010), không cần wrapper.

Khi cần model "operation has failed with detail", dùng `Result<T, E>` từ `std.result` (ADR-0011 ABI metadata + v0.4 stdlib). Hai mục đích, hai công cụ:

| Trường hợp | Dùng | Ví dụ |
|---|---|---|
| Value có/không có (lookup thất bại, optional field) | `T?` | `function find_user(id: Integer) -> User?` |
| Operation thất bại có lý do (parse, IO, capability) | `Result<T, E>` | `function parse(s: String) -> Result<Integer, ParseError>` |
| Value có Unknown state (sensor, async, capability) | `Trilean` | `function vaccinated() -> Trilean` |

Tự dùng `enum MyOption<T> { Some(T), None }` trong user code vẫn hợp lệ (Triết không cấm), nhưng compiler emit `W0030` advisory: *"prefer `T?` for nullable values"*.

Lý do thiết kế:
- **Tam phân bẩm sinh.** Discriminator của `T?` là 1 trit, không phải 1 byte. `Option<T>` wrap thêm tag layer redundant.
- **Operators `?.`, `?:`, `!!` giữ cú pháp ngắn.** OOP-style: check-and-use ngay tại call site.
- **AI-first:** một cách làm cho một thứ. Đọc `T?` → biết ngay phải null-check. Không có "tôi nên dùng `Option` hay `T?`?" mơ hồ.
- **Self-consistency:** chương trình mà có cả `String?` và `Option<String>` ở khác chỗ là smell. Chọn một.

V0.1–v0.3 thử nghiệm cả hai. v0.4 chốt **`T?` primary**. `Result<T, E>` trong `std.result` cung cấp cho error-handling explicit. Local user-defined enum vẫn được phép cho ad-hoc sum types (Color, Direction, ...) nhưng không nên dùng làm wrapper cho nullability.

### 2.6 Type inference

Inference thực hiện local, theo Hindley-Milner đơn giản hóa. Annotation bắt buộc tại:
- Tham số function
- Return type của function (trừ khi function có thân là expression đơn và type suy được dễ)
- Tham số type của generic struct/enum khi context không suy được (v0.2 hỗ trợ inference từ argument: `Some(42)` → `Option<Integer>`)

```triet
let x = 5                       // suy ra Integer
let y: Tryte = 5                // explicit, literal coerced tới Tryte (đặc biệt cho integer literal)
function double(n: Integer) = n * 2     // return type suy được = Integer
function id(n: Integer) -> Integer { n }  // explicit, bắt buộc khi block form
```

---

## 3. Số học balanced ternary

### 3.1 Biểu diễn

Một số trong balanced ternary dùng các trit `{-1, 0, +1}` (viết là `-`, `0`, `+`). Đọc MSB trước, LSB cuối:

```
giá_trị(t_{n-1} t_{n-2} ... t_1 t_0) = Σ t_i · 3^i
```

Ví dụ `0t+0-+` = `(+1)·3³ + 0·3² + (-1)·3¹ + (+1)·3⁰` = 27 - 3 + 1 = 25.

### 3.2 Tính chất nổi bật của balanced ternary

Những tính chất sau là *language-level guarantees*, không phải implementation detail:

1. **Đảo dấu = đảo trit:** `-x` được tính bằng cách đảo từng trit (`+ ↔ -`, `0 → 0`). Không có two's complement quirk như `i32::MIN` không có dương tương ứng.
2. **Phép chia làm tròn không bias:** chia số `n` cho số dương, kết quả làm tròn về số gần nhất (round-half-to-even không cần — balanced ternary tự nhiên cho ra kết quả gần nhất).
3. **Hàm dấu là trit MSB khác 0 đầu tiên:** không cần phép so sánh.
4. **Phạm vi đối xứng:** với `n` trit, phạm vi là `[-(3ⁿ-1)/2, +(3ⁿ-1)/2]` — đối xứng quanh 0.

### 3.3 Phép toán số học

| Operator | Tên | Kiểu áp dụng |
|---|---|---|
| `+` `-` `*` | cộng, trừ, nhân | `Tryte`, `Integer`, `Long` |
| `/` | chia (làm tròn về gần nhất, không bias) | như trên |
| `%%` | mod (kết quả cùng dấu với chia balanced) | như trên |
| `**` | lũy thừa (right-associative) | như trên |
| `-` `!` `not` (unary) | đảo dấu / phủ định | như trên + `Trit` + `Trilean` |
| `<` `<=` `>` `>=` `==` `!=` | so sánh | như trên |

**Unary unification (đặc trưng tam phân):** Trong balanced ternary, "đảo dấu" số và "phủ định logic" là **cùng một phép toán** ở mức trit (đảo từng trit `+ ↔ -`, `0 → 0`). Triết hợp nhất: ba dạng `-x`, `!x`, `not x` đồng nghĩa và cùng map tới một AST node. Dev tự chọn theo ngữ cảnh: `-` cho số, `!` cho logic, `not` khi muốn dùng English keyword.

**Tràn (overflow):** mặc định **panic** — fail-fast, dễ phát hiện bug. Ba biến thể alternative cho hành vi chuyên biệt:

```triet
let x = a.add_and_truncate(b)   // wrap-around: cắt cụt trit cao, kết quả modular
let y = a.add_and_saturate(b)   // kẹp tại biên (clamp): max nếu vượt, min nếu dưới
let z = a.try_add(b)            // trả về Option<T>: Known(result) hoặc Unknown nếu overflow
```

Áp dụng tương tự cho `subtract`, `multiply`, `divide` — ví dụ `subtract_and_truncate`, `try_divide`.

**Lưu ý:** phép `negate` (`-x`) **không cần** biến thể overflow — phạm vi balanced ternary đối xứng quanh 0, nên `negate` luôn thành công (khác với two's complement binary nơi `negate(MIN)` overflow). Đây là một trong các *guarantees* của balanced ternary đã liệt kê ở §3.2.

**Use cases:**
- **default `+`** — strict logic, dev muốn biết overflow ngay (panic)
- **`add_and_truncate`** — modular arithmetic cố ý: hash, crypto, circular buffer
- **`add_and_saturate`** — DSP, audio/video, color clamping, progress bar
- **`try_add`** — caller xử lý explicit, chuỗi pipeline với Option<T>

### 3.4 Encoding nội bộ (informative)

Trit packing chuẩn dùng *radix-243*: 5 trit liên tiếp đóng gói thành 1 byte với bảng tra. MSB trước.

Ví dụ: trit string `+0-+0` = `(+1, 0, -1, +1, 0)`. Mã hóa balanced → unsigned: cộng (3⁵-1)/2 = 121 vào giá trị balanced của 5-trit (`81 + 0 - 9 + 3 + 0 = 75`), kết quả `75 + 121 = 196` → byte `0xC4`.

Encoding chi tiết và kiểm thử cross-platform đặc tả trong tài liệu riêng `crates/core/ENCODING.md`.

---

## 4. Logic 3 giá trị

### 4.1 Kiểu Trilean

`Trilean` có chính xác 3 giá trị: `false`, `unknown`, `true`. Ánh xạ với trit: `false → -1`, `unknown → 0`, `true → +1`.

Truth value mapping cho fuzzy reasoning (informative):
- `false = 0.0`
- `unknown = 0.5`
- `true = 1.0`

### 4.2 Bộ phép logic

Mặc định **Łukasiewicz Ł3**. Các phép Kleene K3 expose qua dạng có dấu `~` hoặc tên `kleene_*`.

| Phép | Symbol | Keyword | Hệ |
|---|---|---|---|
| NOT | `!a` | `not a` | universal |
| AND (= min) | `a && b` | `a and b` | universal |
| OR (= max) | `a \|\| b` | `a or b` | universal |
| Implication | `a => b` | `a implies b` | Ł3 (mặc định) |
| Implication | `a ~> b` | `a kleene_implies b` | K3 |
| XOR | `a ^ b` | `a xor b` | Ł3 (mặc định) |
| XOR | `a ~^ b` | `a kleene_xor b` | K3 |
| Biconditional | `a <=> b` | `a iff b` | Ł3 (mặc định) |
| Biconditional | `a <~> b` | `a kleene_iff b` | K3 |

**Quy tắc chung:** dấu `~` đánh dấu biến thể Kleene. Nhất quán cho mọi operator. AI/LLM học một lần, áp dụng khắp nơi.

### 4.3 Bảng chân lý

#### 4.3.1 NOT (cả hai hệ)

| `a` | `!a` |
|---|---|
| true | false |
| unknown | unknown |
| false | true |

#### 4.3.2 AND (cả hai hệ — = min)

| `a && b` | true | unknown | false |
|---|---|---|---|
| **true** | true | unknown | false |
| **unknown** | unknown | unknown | false |
| **false** | false | false | false |

#### 4.3.3 OR (cả hai hệ — = max)

| `a \|\| b` | true | unknown | false |
|---|---|---|---|
| **true** | true | true | true |
| **unknown** | true | unknown | unknown |
| **false** | true | unknown | false |

#### 4.3.4 Implication

Łukasiewicz `=>` (`min(1, 1-a+b)`):

| `a => b` | true | unknown | false |
|---|---|---|---|
| **true** | true | unknown | false |
| **unknown** | true | **true** | unknown |
| **false** | true | true | true |

Kleene `~>` (`max(1-a, b)`):

| `a ~> b` | true | unknown | false |
|---|---|---|---|
| **true** | true | unknown | false |
| **unknown** | true | **unknown** | unknown |
| **false** | true | true | true |

Khác biệt duy nhất: `unknown => unknown` = **true** (Łukasiewicz, vacuously true), `unknown ~> unknown` = **unknown** (Kleene, conservative).

#### 4.3.5 XOR và Biconditional

Định nghĩa qua implication:
- `a <=> b ≡ (a => b) && (b => a)` (cùng hệ)
- `a ^ b ≡ !(a <=> b)`

Khác biệt vẫn chỉ ở `unknown`-`unknown`:

| | Łukasiewicz | Kleene |
|---|---|---|
| `unknown <=> unknown` | true | unknown |
| `unknown ^ unknown` | false | unknown |

### 4.4 Strong operators (hoãn tới Ł∞)

Łukasiewicz có thêm strong AND `⊗` và strong OR `⊕`:
- `a ⊗ b = max(0, a + b - 1)`
- `a ⊕ b = min(1, a + b)`

Hữu ích cho fuzzy reasoning và biên xác suất Fréchet, nhưng trong Ł3 (3 giá trị rời rạc) chúng cộng ít giá trị thực dụng. Triết hiện tại **không expose**. Sẽ đưa vào khi mở rộng tới Ł∞ (continuous-valued).

### 4.5 Equality `==` và `!=`

Toán tử `==` là **value equality** — kiểm tra hai giá trị có cùng nội dung không. Trả về `Trilean` nhưng **không bao giờ tạo ra `unknown`** (chỉ `true` hoặc `false`).

```triet
true == true              // → true
unknown == unknown        // → true   (cùng giá trị, không phải biconditional)
unknown == true           // → false  (khác giá trị)
5_integer == 5_integer        // → true
"abc" == "abc"            // → true
```

Cùng semantic ở Ł3 và K3 — `==` không phải logical operator nên không phụ thuộc hệ logic.

Khi dev muốn **biconditional fuzzy** (lan truyền unknown), dùng `<=>` (Łukasiewicz) hoặc `<~>` (Kleene) explicitly:

```triet
unknown == unknown        // → true   (giá trị giống nhau)
unknown <=> unknown       // → true   (Łukasiewicz: vacuously equivalent)
unknown <~> unknown       // → unknown (Kleene: không biết chúng có tương đương không)
```

Lý do thiết kế: 99% ngôn ngữ modern (Python, Rust, Kotlin, Swift, Go) đều dùng `==` là value equality. AI/dev không bị surprise. SQL `NULL` semantics tránh được — không có "bug âm thầm khi so sánh".

### 4.6 Short-circuit evaluation

`&&` và `||` short-circuit:
- `false && _` → `false` (không eval RHS)
- `true || _` → `true` (không eval RHS)
- `unknown && _` → eval RHS
- `unknown || _` → eval RHS

Implication, XOR, biconditional **không** short-circuit (cần cả hai operands).

---

## 5. Biến và binding

### 5.1 Khai báo

```triet
let x = 5                       // immutable, type inferred
let y: Tryte = 5_tryte          // immutable, type explicit
let mutable count = 0           // mutable
constant PI_TIMES_3: Integer = 9 // compile-time constant
```

`let` mặc định **immutable**. `let mutable` cho phép gán lại. `constant` cho hằng số biết tại compile time.

### 5.2 Phạm vi (scope)

Block-scoped, lexical. Shadowing được phép trong cùng scope:

```triet
let x = 5
let x = x + 1           // x bây giờ là 6
let x: String = "hi"      // shadow, đổi cả type
```

---

## 6. Hàm

### 6.1 Định nghĩa

Hai dạng — block và single-expression:

```triet
// Block form
function add(a: Integer, b: Integer) -> Integer {
    a + b
}

// Single-expression form (= thay {})
function double(n: Integer) -> Integer = n * 2

// Return type inferred khi expression đơn
function triple(n: Integer) = n * 3
```

Block form: giá trị cuối cùng (không có `;`) là return value. Có thể dùng `return` explicit:

```triet
function abs(n: Integer) -> Integer {
    if n < 0 { return -n }
    n
}
```

### 6.2 Tham số

Tất cả tham số bắt buộc có type annotation. v0.2 không có (sẽ phasing dần):
- Default values — defer
- Named arguments — defer
- Variadic — defer
- Generic functions (`function id<T>(x: T) -> T`) — phase G.2 sau module system. Generic type *definitions* (struct/enum) đã có ở v0.2.

### 6.3 Closure (lambda)

```triet
let inc = |n: Integer| -> Integer { n + 1 }
let inc = |n: Integer| n + 1               // single-expression form
let inc = |n| n + 1                      // type inferred khi context cho phép
```

---

## 7. Control flow

### 7.1 if/else

`if` là **expression**, có giá trị:

```triet
let abs_n = if n < 0 { -n } else { n }

let category =
    if score >= 90 { "A" }
    else if score >= 80 { "B" }
    else if score >= 70 { "C" }
    else { "F" }
```

Điều kiện phải là `Trilean`. Ngữ nghĩa của `if` với 3 giá trị:

| `cond` | Hành vi |
|---|---|
| `true` | chạy nhánh `then` |
| `false` | chạy nhánh `else` (hoặc trả `Unit` nếu không có) |
| `unknown` | chạy nhánh `else` được? Xem §7.1.1 |

#### 7.1.1 Xử lý `unknown` trong điều kiện

Đây là quyết định AI-first quan trọng. Triết **bắt buộc** xử lý explicit khi điều kiện có thể `unknown`:

```triet
// Lỗi compile: cond là Trilean, có thể unknown — phải xử lý
if cond { do_something() }

// OK 1: dùng `if?` để đối xử unknown như false (giống boolean fallback)
if? cond { do_something() }

// OK 2: dùng match để xử lý cả 3
match cond {
    true => do_something(),
    false => do_other(),
    unknown => do_default(),
}

// OK 3: ép tới boolean 2 giá trị
if cond.assume_known() { ... }  // panic nếu unknown
if cond == true { ... }         // chỉ true mới chạy, unknown đối xử false
```

Lý do: nếu để `if cond` chạy mặc định trên 3 giá trị, dev rất dễ ngầm coi `unknown` là `false` mà không nhận ra — bug âm thầm. Triết force explicit. **AI-first:** LLM thấy lỗi compile, biết phải chọn cách xử lý.

### 7.2 Loops: `for`, `while`, `loop`

Triết cung cấp ba dạng loop cho ba nhu cầu khác nhau:

#### `for` — primary, iterate trên collection/range

```triet
for i in 0..100 { ... }                    // range
for i in 0..=100 { ... }                   // inclusive range
for item in items { ... }                  // iterator
for (idx, item) in items.enumerate() { ... }
```

`for` là form được khuyến nghị khi biết trước số lần lặp hoặc có collection.

#### `while` — condition-driven

```triet
while condition { ... }                    // condition: Trilean known
while? bool3_cond { ... }                  // condition: Trilean có thể unknown
                                           // unknown đối xử như false
```

Giống `if`/`if?`, Triết phân biệt `while` (cần điều kiện chắc chắn) và `while?` (chấp nhận unknown như false). Dùng cho: I/O đến EOF, polling, numerical iteration tới hội tụ, state machine.

#### `loop` — infinite, break-with-value

```triet
let result = loop {
    let x = read_input()
    if x.is_valid() { break x }
}
```

`loop` chạy vô hạn, thoát bằng `break expr` (truyền giá trị ra ngoài). Dùng cho event loop, retry logic, search-until-found.

#### `break` và `continue`

Cả ba dạng loop hỗ trợ:
- `break` — thoát loop
- `break expr` — chỉ trong `loop`, truyền giá trị ra
- `continue` — sang lượt tiếp

### 7.3 match

Pattern matching exhaustive, expression-oriented:

```triet
function classify(n: Integer) -> String =
    match n {
        0 => "zero",
        n if n > 0 => "positive",
        _ => "negative",
    }

function describe(b: Trilean) -> String =
    match b {
        true => "có",
        false => "không",
        unknown => "chưa rõ",
    }
```

Pattern khả dụng (v0.2):
- Literal: `0`, `5_tryte`, `true`, `"hello"`, `0t+0-+`
- Variable binding: `x` (capture giá trị)
- Wildcard: `_`
- Tuple: `(a, b, _)`
- Enum variant: `Some(x)`, `None` (v0.2)
- Guard: `pattern if condition`
- Or-pattern: `1 | 2 | 3`

Compiler bắt buộc match exhaustive. Nếu thiếu case → lỗi compile. AI-friendly: LLM được bắt buộc phải bao quát mọi nhánh.

---

## 8. Tuple

```triet
let pair: (Integer, Trilean) = (42, true)
let (x, y) = pair                       // destructure
let first = pair.0                      // index
```

Tuple là composite anonymous (kiểu được suy ra từ thành phần). Struct (named fields) và enum (named variants) là composite có tên — xem định nghĩa ở §6+.

---

## 9. Standard library tối thiểu (v0.2)

> **Module migration (v0.2.x):** Cú pháp `import std.io.println` (v0.2 baseline, dot-path) sẽ được bổ sung Python-style `from std.io import println` theo [ADR-0005](docs/decisions/0005-module-system.md). Path separator vẫn là `.`. Verbose keyword `function` thay cho `fn` đã chốt.

Module `std.io`:
```triet
function print(text: String) -> Unit
function println(text: String) -> Unit
function read_line() -> String
```

Module `std.text`:
```triet
function len(s: String) -> Integer
function concat(a: String, b: String) -> String
function from_integer(n: Integer) -> String
```

Module `std.assert`:
```triet
function assert(cond: Trilean, msg: String) -> Unit
```

Note: `std.assert` panic nếu `cond` là `false` HOẶC `unknown`. Lý do: assertion phải chắc chắn, `unknown` không đủ.

---

## 10. Memory model

Triết theo **Mojo-style memory model** — mục tiêu: cú pháp đơn giản gần Java/Python, performance gần Rust, ít cognitive overhead.

### 10.1 Triết lý

| Aspect | Quyết định |
|---|---|
| Stack types | Value semantics — copy mặc định khi gán/truyền |
| Heap types | ARC (Automatic Reference Counting) ngầm — không phải tracing GC, không phải explicit `Arc<T>` |
| Lifetime annotations | **KHÔNG** trong source code — compiler infer cho 99% trường hợp |
| Borrow checker | Đơn giản hóa: kiểm tra "no aliasing while mutable" tại scope-level |

**KHÔNG** theo Rust: ownership/lifetime annotations explicit (`'a`), `&T` vs `&mut T` tỉ mỉ, `String/&str/Cow/Box<str>` zoo. Mojo đã chứng minh rằng 90% safety của Rust đạt được mà không cần phức tạp đó.

### 10.2 Phân loại type

Theo quy tắc đặt tên đã chốt (xem §2.1) — **PascalCase tất cả**:

**Stack-allocatable** (kích thước cố định, copy trị):
- `Trit`, `Tryte`, `Integer`, `Long` (numeric)
- `Trilean` (logic)
- `Unit` (zero-sized)
- Tuples `(T1, T2, ...)`
- `T?` (nullable: T + 1-trit discriminator)

**Heap-allocated** (ARC-managed):
- `String` (UTF-8 owned, mutable qua `let mutable`)
- `Result<T, E>` ✅ (v0.4 stdlib — `std.result`; canonical error-handling type per §2.5)
- `List<T>`, `Set<T>`, `Map<K, V>` (post-v0.4 collections — defer to v0.5+)

`Option<T>` đã **deprecated trong stdlib** từ v0.4 (xem §2.5): `T?` là cách chính tắc cho "value may be absent" — `T?` đã có discriminator 1-trit bẩm sinh, wrapper `Option<T>` redundant. Local user-defined `enum MyOption<T> { Some(T), None }` vẫn hợp lệ nhưng không khuyến khích.

**Stack view** (composite, không sở hữu):
- `StringSlice` (post-v0.4 — view vào String, immutable, lifetime infer)

### 10.3 Function parameter conventions (Mojo-style)

```triet
// Mặc định: borrowed (read-only reference, không annotation)
function print_name(name: String) { ... }

// Mutable: từ khóa `mutable` ở parameter (rare)
function append(mutable buffer: String, suffix: String) { ... }

// Owned (transfer ownership, hiếm dùng): từ khóa `owned`
function consume(owned data: String) -> String { ... }
```

So với Rust: viết ngắn 30%, ít cognitive overhead 70%.

### 10.4 Implementation hiện tại (v0.4)

Triết hôm nay chạy trên **hai tier song song**, cả hai đều là development tiers per [VISION §4.3](VISION.md):

- **Tree-walking interpreter** (`triet-interpreter`, từ v0.1): tất cả giá trị copy theo trị; heap types dùng `Rc<T>` Rust runtime (≈ ARC simulation).
- **Bytecode VM** (`triet-ir`, từ v0.3): register-SSA IR + 53-opcode VM; differential test 11/11 byte-identical với tree-walker.

Borrow checker chưa cần — language hiện tại chưa expose references.

Native AOT compile (v2.0, LLVM) sẽ implement đầy đủ ARC + simplified borrow check. Trytecode backend (v∞) là production target cuối cùng khi phần cứng tam phân xuất hiện. Memory model precise (ARC opcodes, region-based vs reference-counted choice) **vẫn deferred** — v0.4 chỉ lock ABI surface, không lock memory representation; ADR riêng sẽ viết trước v0.9 JIT.

---

## 11. Ví dụ chương trình hoàn chỉnh

### 11.1 FizzBuzz

```triet
function fizzbuzz(n: Integer) -> String =
    match (n %% 3, n %% 5) {
        (0, 0) => "FizzBuzz",
        (0, _) => "Fizz",
        (_, 0) => "Buzz",
        _      => std.text.from_integer(n),
    }

function main() -> Unit {
    let mutable i = 1
    while? i <= 100 {           // while? giống if? — xử lý unknown như false
        std.io.println(fizzbuzz(i))
        i = i + 1
    }
}
```

### 11.2 Reasoning với missing data (showcase Łukasiewicz)

```triet
type Patient = (Trilean, Trilean, Trilean)    // (có_sốt, có_phát_ban, đã_tiêm_vaccine)

function risk_measles(p: Patient) -> Trilean {
    let (fever, rash, vaccinated) = p
    let symptoms = fever && rash
    let possibly_at_risk = symptoms && !vaccinated
    possibly_at_risk
}

// fever=true, rash=true, vaccinated=unknown
// → symptoms = true, !vaccinated = unknown
// → possibly_at_risk = true && unknown = unknown
// → "không đủ thông tin để khẳng định, cần xác minh tiêm chủng"
```

### 11.3 Showcase balanced ternary — kiểm tra dấu

```triet
// Dấu của số = trit MSB khác 0 đầu tiên
// Một phép — không cần if
function sign(n: Integer) -> Trit = n.first_nonzero_trit_or(0_trit)
```

---

## 12. Ngữ pháp (EBNF, không hoàn chỉnh, v0.2)

```ebnf
program       = item* ;
item          = function | const_decl | type_alias | import ;

function      = "function" IDENT "(" params? ")" return_type? body ;
params        = param ("," param)* ","? ;
param         = IDENT ":" type ;
return_type   = "->" type ;
body          = "=" expr | block ;
block         = "{" stmt* expr? "}" ;

stmt          = let_stmt | expr_stmt | return_stmt ;
let_stmt      = "let" "mutable"? IDENT (":" type)? "=" expr ;
expr_stmt     = expr ";" ;
return_stmt   = "return" expr? ";" ;

expr          = literal | IDENT | binop | unop | call | if_expr | match_expr
              | block | tuple | "(" expr ")" ;
binop         = expr OP expr ;
unop          = ("!" | "-" | "not") expr ;  // tất cả 3 form đồng nghĩa
call          = expr "(" args? ")" ;
if_expr       = ("if" | "if?") expr block ("else" block)? ;
match_expr    = "match" expr "{" arm ("," arm)* ","? "}" ;
arm           = pattern ("if" expr)? "=>" expr ;

type          = IDENT ;
pattern       = literal | IDENT | "_" | tuple_pat | or_pat ;
```

(Đầy đủ ngữ pháp BNF sẽ ở `docs/grammar.ebnf` khi parser hoàn thành.)

### 12.1 Operator precedence

Bảng ưu tiên từ **cao đến thấp** (cao = bind chặt hơn, đánh giá trước). Mỗi level ghi rõ associativity.

| Level | Operators | Associativity | Ghi chú |
|---:|---|---|---|
| 14 | Postfix: `?.` `.` `()` `[]` `!!` | left-chain | method call, field access, index, force-unwrap |
| 13 | `**` (exponent) | **right** | `2 ** 3 ** 2` = `2 ** (3 ** 2)` |
| 12 | Unary: `-` `!` `not` | prefix | (lower than `**`: `-2 ** 2` = `-(2 ** 2)` = -4) |
| 11 | `*` `/` `%%` | left | |
| 10 | `+` `-` (binary) | left | |
| 9 | `..` `..=` | **no chain** | `1..10..20` lỗi compile |
| 8 | `?:` (Elvis) | right | `a ?: b ?: c` = `a ?: (b ?: c)`; lower than arithmetic |
| 7 | `<` `<=` `>` `>=` | **no chain** | `a < b < c` lỗi compile |
| 6 | `==` `!=` | **no chain** | `a == b == c` lỗi compile |
| 5 | `^` `~^` `xor` `kleene_xor` | left | XOR (Łukasiewicz / Kleene) |
| 4 | `&&` `and` | left | |
| 3 | `\|\|` `or` | left | |
| 2 | `<=>` `<~>` `iff` `kleene_iff` | left | biconditional |
| 1 | `=>` `~>` `implies` `kleene_implies` | **right** | implication (math convention) |

**Biểu thức ví dụ:**

```triet
-2 ** 2                       // = -4 (math/Python convention)
(-2) ** 2                     // = 4 (parens override)
a + b * c ** 2                // = a + (b * (c ** 2))
name?.length ?: 0 + count * 2 // = name?.length ?: (0 + (count * 2))
a == b == c                   // ❌ comparison không chain
a == b and c == d             // = (a == b) and (c == d) — OK
flag and not other            // = flag and (not other)
p implies q implies r         // = p implies (q implies r) (right-assoc)
```

**Quy tắc cấm chain (no chain):** comparison (`<` `<=` `>` `>=` `==` `!=`) và range (`..` `..=`) không chain để tránh ambiguity giống SQL/Python. AI-first: lỗi compile sớm tốt hơn semantics surprise.

---

## 13. Open issues — đã quyết định

Cả 4 open issue của bản trước đã có ADR riêng trong `docs/decisions/`. Tóm tắt:

1. **Memory layout của `T?`** — discriminator 1 trit (không sentinel). [ADR 0001](docs/decisions/0001-nullable-memory-layout.md)
2. **F-string format spec** — subset của Rust format spec (`{}`, `{:width}`, `{:0width}`, `{:.precision}`); không alignment chars / hex / oct / bin / locale. [ADR 0002](docs/decisions/0002-fstring-format-spec.md)
3. **Iterator protocol** — trait `Iterator<T>` Rust/Mojo-style với `next() -> T?`; user-extensible từ v0.2. [ADR 0003](docs/decisions/0003-iterator-protocol.md)
4. **String multi-line indent** — strip common leading whitespace Java/Kotlin-style, closing-quote quyết định strip depth, tab+space mix là lỗi. [ADR 0004](docs/decisions/0004-multiline-string-indent.md)

Open issues mới sẽ append phía dưới. Hiện tại: trống.

---

## 14. Lộ trình các phiên bản tiếp theo (informative)

Lộ trình chi tiết với gates, deliverables, và ADRs: [`ROADMAP.md`](ROADMAP.md).

Tóm tắt phasing dài hạn:

- **v0.2** — struct, enum, generics ✅
- **v0.2.x** — module system ✅ ([ADR-0005](docs/decisions/0005-module-system.md))
- **v0.3** — bytecode VM + stable IR ✅ ([ADR-0007](docs/decisions/0007-ir-design.md), [ADR-0008](docs/decisions/0008-triv-binary-format.md), [ADR-0010](docs/decisions/0010-ternary-native-ir.md) ternary-native refactor)
- **v0.4** — Crate-Pack + stable ABI ✅ ([ADR-0011](docs/decisions/0011-abi-metadata-format.md), [ADR-0012](docs/decisions/0012-witness-table-dispatch.md), [ADR-0013](docs/decisions/0013-semver-linking-policy.md))
- **v0.5** — CAS packaging (hash-based identity) ✅ ([ADR-0014](docs/decisions/0014-hash-scheme-refinement.md), [ADR-0015](docs/decisions/0015-package-store-layout.md))
- **v0.6** — capability namespaces (`sys.*` / `dev.*` / `usr.*`) ✅ ([ADR-0016](docs/decisions/0016-capability-type-system.md), [ADR-0017](docs/decisions/0017-trilean-policy-hook.md), [ADR-0018](docs/decisions/0018-capability-loader-semantics.md)) — **hiện tại**
- **v0.7** — self-hosting compiler — *next*
- **v0.8** — concurrency model
- **v0.9** — JIT (Cranelift)
- **v1.0** — production stability
- **v2.0** — AOT native compile (LLVM)
- **v3.0** — microkernel POC
- **v∞** — backend cho phần cứng tam phân, khi xuất hiện
