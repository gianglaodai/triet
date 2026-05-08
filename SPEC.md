# Triết — Đặc tả ngôn ngữ v0.1

> Triết (哲) là một ngôn ngữ lập trình **balanced ternary, AI-first**, lấy cảm hứng từ Setun (Liên Xô, 1958). Phiên bản v0.1 đặc tả semantics cốt lõi cho một interpreter tree-walking. Hiệu năng và bytecode/native compile thuộc v0.2+.

---

## 0. Triết lý thiết kế

Mọi quyết định trong tài liệu này phục vụ ba mục tiêu, theo thứ tự ưu tiên:

1. **AI-first.** Cú pháp và semantics tối ưu cho việc LLM sinh code đúng ngay lần đầu. Ưu tiên: explicit > implicit, regular > exception, keyword > ký hiệu khi mơ hồ, low ambiguity > terseness.
2. **Tam phân là first-class.** Trit, balanced ternary arithmetic, và logic 3 giá trị Łukasiewicz là kiểu/phép toán nguyên thủy — không phải library bên trên hệ nhị phân.
3. **Production-grade ở Ł3, mở rộng được tới Ł∞.** v0.1 dùng giá trị rời rạc 3 mức {-1, 0, +1}. Đường tiến hóa tới logic vô hạn giá trị (fuzzy/probabilistic) phải không đập bỏ semantics hiện tại.

Không phải mục tiêu (non-goals) cho v0.1:
- Hiệu năng tối ưu (interpreter tree-walking, OK chậm)
- Concurrency/async
- FFI với C/Rust runtime
- Module system phức tạp (đơn module phẳng)
- Generics đầy đủ (chỉ type alias đơn giản)

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
fn  let  mut  const  type  if  else  match  return
true  false  unknown  not  and  or  xor  iff  implies
kleene_implies  kleene_xor  kleene_iff
Trit  Tryte  Integer  Long  Trilean  Text
import  module  pub
```

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

#### 1.5.3 Text (string)

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
let msg: Text = f"Câu trả lời là {n}"
let calc: Text = f"Tổng: {a + b}, gấp đôi: {(a + b) * 2}"

// Định dạng explicit (qua trait Display):
let pretty: Text = f"Giá: {price:#.2}"     // số thập phân, 2 chữ số
```

Quy tắc: chỉ string có prefix `f` mới interpret `{expr}`. Đảm bảo string không có `f` luôn nguyên bản — không hallucinate khi LLM gen code có chứa `{` `}` ngẫu nhiên.

Escape `{` và `}` trong f-string: dùng `{{` và `}}`.

---

## 2. Hệ thống kiểu (Type system)

### 2.1 Kiểu nguyên thủy

| Kiểu | Số trit | Phạm vi | Mô tả |
|---|---|---|---|
| `Trit` | 1 | `{-1, 0, +1}` | Đơn vị thông tin tam phân cơ bản |
| `Tryte` | 9 (= 3²) | `±9_841` | Số nguyên nhỏ |
| `Integer` | 27 (= 3³) | `±3_812_798_742_493` | **Số nguyên mặc định** |
| `Long` | 81 (= 3⁴) | rất lớn | Số nguyên lớn |
| `Trilean` | 1 | `{false, unknown, true}` | Logic 3 giá trị |
| `Text` | — | UTF-8 string | Chuỗi văn bản |
| `Unit` | — | `()` | Không có giá trị (giống void) |

`Trit` và `Trilean` đều là 1-trit về biểu diễn nhưng **khác kiểu** ở mức ngôn ngữ — `Trit` là số (`-1`, `0`, `+1`), `Trilean` là chân lý (`false`, `unknown`, `true`). Conversion phải explicit (xem §2.4).

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
type Username = Text
```

Alias không tạo kiểu mới, chỉ là tên thay thế (giống `type` của Rust, không như `newtype`).

### 2.4 Conversion

Tất cả conversion giữa kiểu **explicit**. Không có implicit coercion.

```triet
let x: Tryte = 5_tryte
let y: Integer = x.to_integer()                       // Tryte → Integer (không mất mát)
let z: Tryte = (1000).to_tryte()                      // Integer → Tryte, panic nếu tràn
let w: Maybe<Tryte> = (1000).try_to_tryte()           // → Unknown nếu tràn
let v: Trilean = Trilean.from_trit(t)                 // Trit → Trilean
let t: Trit = b.to_trit()                             // Trilean → Trit (false→-1, unknown→0, true→+1)
```

Khi cần chuyển đổi với hành vi overflow chuyên biệt (saturating, truncating), dùng *narrowing conversion methods* tương tự arithmetic:
- `to_tryte()` — panic on overflow (default)
- `to_tryte_and_saturate()` — kẹp tại biên Tryte
- `to_tryte_and_truncate()` — cắt cụt trit cao
- `try_to_tryte()` — trả về `Maybe<Tryte>`

### 2.5 Nullable types `T?`

Bất kỳ kiểu nào cũng có thể "có thể null" qua hậu tố `?`:

```triet
let name: Text? = get_name()       // có thể là null
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
    std.io.println(name)    // name lúc này được narrow về Text
}

// Safe call: trả về null nếu chuỗi bị null ở đâu đó
let len: Integer? = name?.length

// Elvis: thay null bằng giá trị mặc định
let len: Integer = name?.length ?: 0

// Force unwrap: panic nếu null
let must: Text = name!!

// match trên null
match name {
    null => "khuyết danh",
    n    => f"xin chào {n}",
}
```

#### `T?` KHÔNG đồng nhất với `Maybe<T>`

Đây là một quyết định thiết kế quan trọng. Triết cung cấp **hai cơ chế song song** cho hai nhu cầu khác nhau:

| Đặc điểm | `T?` (nullable) | `Maybe<T>` (wrapper, v0.2+) |
|---|---|---|
| Triết lý | Đơn giản: có/không có | Monadic: pipeline biến đổi |
| Verb-first naming | `?.`, `?:`, `!!` operators | `get`, `get_or(default)`, `get_or_else { compute() }` |
| API | `?.`, `?:`, `!!`, smart cast | `map`, `flat_map`, `filter`, `fold`, `get`, `get_or`, `get_or_else` |
| Đối tượng dev | OOP-friendly | FP-friendly |
| Cú pháp ngắn? | Có (`?` operator) | Không (gọi method) |
| Auto-convert? | **Không** | **Không** |

Hai kiểu **không tự động convert** lẫn nhau. Dev chuyển explicit:

```triet
let n: Text? = "hello"
let m: Maybe<Text> = n.to_maybe()     // T? → Maybe<T>
let n2: Text? = m.to_nullable()       // Maybe<T> → T?

// Type khác nhau → compiler chặn nhầm lẫn
fn takes_nullable(x: Text?) { ... }
fn takes_maybe(x: Maybe<Text>) { ... }

takes_maybe(n)         // ❌ lỗi compile
takes_maybe(n.to_maybe())  // ✓
```

Lý do thiết kế:
- **Intent rõ ở type signature.** Thấy `T?` = check-and-use. Thấy `Maybe<T>` = pipeline.
- **AI-first:** AI/dev đọc type biết ngay nên dùng API nào. Không có nhiều cách làm cùng một việc.
- **Không Frankenstein:** Kotlin gắn `let`/`also`/`run` lên nullable đã chứng minh trộn hai paradigm vào một type tạo confusion.
- **OOP devs không cần biết monadic.** FP devs không bị ràng buộc bởi nullable.

V0.1 chỉ có `T?` (compiler primitive). `Maybe<T>` đặc tả ở v0.2 khi có generic + enum.

### 2.6 Type inference

Inference thực hiện local, theo Hindley-Milner đơn giản hóa. Annotation bắt buộc tại:
- Tham số function
- Return type của function (trừ khi function có thân là expression đơn và type suy được dễ)
- Tham số type của generic (chưa có ở v0.1)

```triet
let x = 5                       // suy ra Integer
let y: Tryte = 5                // explicit, literal coerced tới Tryte (đặc biệt cho integer literal)
fn double(n: Integer) = n * 2     // return type suy được = Integer
fn id(n: Integer) -> Integer { n }  // explicit, bắt buộc khi block form
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
| `-` (unary) | đảo dấu (= đảo trit) | như trên + `Trit` |
| `<` `<=` `>` `>=` `==` `!=` | so sánh | như trên |

**Tràn (overflow):** mặc định **panic** — fail-fast, dễ phát hiện bug. Ba biến thể alternative cho hành vi chuyên biệt:

```triet
let x = a.add_and_truncate(b)   // wrap-around: cắt cụt trit cao, kết quả modular
let y = a.add_and_saturate(b)   // kẹp tại biên (clamp): max nếu vượt, min nếu dưới
let z = a.try_add(b)            // trả về Maybe<T>: Known(result) hoặc Unknown nếu overflow
```

Áp dụng tương tự cho `subtract`, `multiply`, `divide` — ví dụ `subtract_and_truncate`, `try_divide`.

**Lưu ý:** phép `negate` (`-x`) **không cần** biến thể overflow — phạm vi balanced ternary đối xứng quanh 0, nên `negate` luôn thành công (khác với two's complement binary nơi `negate(MIN)` overflow). Đây là một trong các *guarantees* của balanced ternary đã liệt kê ở §3.2.

**Use cases:**
- **default `+`** — strict logic, dev muốn biết overflow ngay (panic)
- **`add_and_truncate`** — modular arithmetic cố ý: hash, crypto, circular buffer
- **`add_and_saturate`** — DSP, audio/video, color clamping, progress bar
- **`try_add`** — caller xử lý explicit, chuỗi pipeline với Maybe<T>

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

Hữu ích cho fuzzy reasoning và biên xác suất Fréchet, nhưng trong Ł3 (3 giá trị rời rạc) chúng cộng ít giá trị thực dụng. Triết v0.1 **không expose**. Sẽ đưa vào khi mở rộng tới Ł∞ (continuous-valued, v0.2+).

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
let x = 5                   // immutable, type inferred
let y: Tryte = 5_tryte      // immutable, type explicit
let mut count = 0           // mutable
const PI_TIMES_3: Integer = 9 // compile-time constant
```

`let` mặc định **immutable**. `let mut` cho phép gán lại. `const` cho hằng số biết tại compile time.

### 5.2 Phạm vi (scope)

Block-scoped, lexical. Shadowing được phép trong cùng scope:

```triet
let x = 5
let x = x + 1           // x bây giờ là 6
let x: Text = "hi"      // shadow, đổi cả type
```

---

## 6. Hàm

### 6.1 Định nghĩa

Hai dạng — block và single-expression:

```triet
// Block form
fn add(a: Integer, b: Integer) -> Integer {
    a + b
}

// Single-expression form (= thay {})
fn double(n: Integer) -> Integer = n * 2

// Return type inferred khi expression đơn
fn triple(n: Integer) = n * 3
```

Block form: giá trị cuối cùng (không có `;`) là return value. Có thể dùng `return` explicit:

```triet
fn abs(n: Integer) -> Integer {
    if n < 0 { return -n }
    n
}
```

### 6.2 Tham số

Tất cả tham số bắt buộc có type annotation. Triết v0.1 không có:
- Default values
- Named arguments
- Variadic
- Generics

(Sẽ thêm dần ở v0.2+.)

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
fn classify(n: Integer) -> Text =
    match n {
        0 => "zero",
        n if n > 0 => "positive",
        _ => "negative",
    }

fn describe(b: Trilean) -> Text =
    match b {
        true => "có",
        false => "không",
        unknown => "chưa rõ",
    }
```

Pattern khả dụng v0.1:
- Literal: `0`, `5_tryte`, `true`, `"hello"`, `0t+0-+`
- Variable binding: `x` (capture giá trị)
- Wildcard: `_`
- Tuple: `(a, b, _)`
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

Tuple là kiểu duy nhất composite tại v0.1. Struct và enum thuộc v0.2.

---

## 9. Standard library tối thiểu (v0.1)

Module `std.io`:
```triet
fn print(text: Text) -> Unit
fn println(text: Text) -> Unit
fn read_line() -> Text
```

Module `std.text`:
```triet
fn len(s: Text) -> Integer
fn concat(a: Text, b: Text) -> Text
fn from_integer(n: Integer) -> Text
```

Module `std.assert`:
```triet
fn assert(cond: Trilean, msg: Text) -> Unit
```

Note: `std.assert` panic nếu `cond` là `false` HOẶC `unknown`. Lý do: assertion phải chắc chắn, `unknown` không đủ.

---

## 10. Memory model (informative)

V0.1 dùng interpreter tree-walking với:
- Tất cả giá trị copy theo trị (value semantics)
- Không có reference, pointer, borrow checker
- Garbage collection cho `Text` qua `Rc` của Rust runtime

Điều này tạm thời đơn giản hóa — sẽ thiết kế lại memory model nghiêm túc khi tiến tới native compile (v0.3+). Có thể xem xét: ownership như Rust, hoặc reference counting với weak refs, hoặc tracing GC. Quyết định để mở.

---

## 11. Ví dụ chương trình hoàn chỉnh

### 11.1 FizzBuzz

```triet
fn fizzbuzz(n: Integer) -> Text =
    match (n %% 3, n %% 5) {
        (0, 0) => "FizzBuzz",
        (0, _) => "Fizz",
        (_, 0) => "Buzz",
        _      => std.text.from_integer(n),
    }

fn main() -> Unit {
    let mut i = 1
    while? i <= 100 {           // while? giống if? — xử lý unknown như false
        std.io.println(fizzbuzz(i))
        i = i + 1
    }
}
```

### 11.2 Reasoning với missing data (showcase Łukasiewicz)

```triet
type Patient = (Trilean, Trilean, Trilean)    // (có_sốt, có_phát_ban, đã_tiêm_vaccine)

fn risk_measles(p: Patient) -> Trilean {
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
fn sign(n: Integer) -> Trit = n.first_nonzero_trit_or(0_trit)
```

---

## 12. Ngữ pháp (EBNF, không hoàn chỉnh, v0.1)

```ebnf
program       = item* ;
item          = function | const_decl | type_alias | import ;

function      = "fn" IDENT "(" params? ")" return_type? body ;
params        = param ("," param)* ","? ;
param         = IDENT ":" type ;
return_type   = "->" type ;
body          = "=" expr | block ;
block         = "{" stmt* expr? "}" ;

stmt          = let_stmt | expr_stmt | return_stmt ;
let_stmt      = "let" "mut"? IDENT (":" type)? "=" expr ;
expr_stmt     = expr ";" ;
return_stmt   = "return" expr? ";" ;

expr          = literal | IDENT | binop | unop | call | if_expr | match_expr
              | block | tuple | "(" expr ")" ;
binop         = expr OP expr ;
unop          = ("!" | "-" | "not") expr ;
call          = expr "(" args? ")" ;
if_expr       = ("if" | "if?") expr block ("else" block)? ;
match_expr    = "match" expr "{" arm ("," arm)* ","? "}" ;
arm           = pattern ("if" expr)? "=>" expr ;

type          = IDENT ;
pattern       = literal | IDENT | "_" | tuple_pat | or_pat ;
```

(Đầy đủ ngữ pháp BNF sẽ ở `docs/grammar.ebnf` khi parser hoàn thành.)

---

## 13. Open issues (cần quyết trước khi implement)

Tất cả 5 open issues của bản trước đã được giải quyết. Open issues mới phát sinh ở v0.1:

1. **Memory layout của `T?`** — dùng discriminator trit/byte riêng (đơn giản, +1 trit/value) hay sentinel value (compact nhưng phức tạp với type không có "unused" representation)? Hiện thiên về discriminator.
2. **F-string format spec** — cú pháp `{val:#.2}` lấy từ Python/Rust hay đơn giản hóa? Cần đặc tả khi implement.
3. **Iterator protocol** — trait `Iterator` cần thiết kế cho `for` loop. v0.1 có thể hardcode cho range và một số collection cơ bản.
4. **String multi-line indent stripping** — `"""...."""` có nên strip indentation chung như Java text blocks không?

---

## 14. Lộ trình các phiên bản tiếp theo (informative)

- **v0.1** (đặc tả này) — interpreter tree-walking, semantics đầy đủ
- **v0.2** — struct, enum, generics, module system, Ł∞ (fuzzy continuous)
- **v0.3** — bytecode VM với JIT (Cranelift)
- **v0.4** — concurrency model (cần thiết kế)
- **v1.0** — production stability commitment, AOT native compile (LLVM/Cranelift)
- **v2.0+** — backend cho phần cứng tam phân giả định, nếu/khi có
