# ADR 0003 — Iterator protocol cho `for`

**Trạng thái:** Quyết định shape, implement đầy đủ ở v0.2 (cùng generics). v0.1 hardcode `Range` + `Enumerate`; refactor đã đặt `advance_iterator` helper trong interpreter làm nền (commit `06025bb`).

**Issue:** SPEC §13 #3 — Trait `Iterator` cho `for` loop. v0.1 hardcode được, v0.2 phải có user-extensible protocol.

## Quyết định

**Hai trait Rust-style** (giống Mojo, Rust, Swift), `next()` trả `T?` (nullable primitive — KHÔNG `Option<T>`).

```triet
trait Iterator<T> {
    fn next(self: mut Self) -> T?
}

trait Iterable<T> {
    fn iter(self) -> Iterator<T>
}
```

`for x in expr { body }` desugar (compiler-internal):

```triet
let __iter = expr.iter()
loop {
    let __next = __iter.next()
    if? __next == null { break }
    let x = __next!!
    body
}
```

### Tại sao `T?` chứ không `Option<T>`?

- `T?` đã là primitive ở v0.1, không cần generics để định nghĩa.
- Iterator dùng nullable-primitive thì có thể tự define iterator cho user types **trước** khi `Option<T>` (v0.2 generic) ổn định.
- Không có ngữ nghĩa nào của `next()` cần phân biệt "có giá trị, value là null" với "hết stream" — `T?` đủ rõ. (Trường hợp cần phân biệt hai → wrap thêm: `Iterator<T?>` cho stream of nullables, `next()` trả `T??`.)
- Nhất quán với SPEC §2.5: `T?` = check-and-use, `Option<T>` = pipeline. Iterator vòng `next()` rõ ràng là check-and-use ngay sau gọi.

### Adapter pattern

`map`, `filter`, `take`, `skip`, `zip`, `chain`, `enumerate` — tất cả là method trên `Iterator<T>` trả `Iterator<U>` (lazy). Generics v0.2 mở khóa được.

`enumerate` ở v0.1 hardcode trong `Value::Enumerate` enum — sẽ refactor thành adapter struct dùng trait khi v0.2 ship.

## Lý do

- **Quen thuộc.** Rust/Mojo/Swift dùng pattern này. LLM được train trên data dùng pattern này nhiều → AI-first phù hợp.
- **Lazy by default.** Iterator chains không materialize tới khi consume — efficient cho large/infinite sequences.
- **Mutable receiver `mut self`.** Khớp với Mojo memory convention SPEC §10.3: stream advancement là mutation, gọi rõ `mut`.
- **Không phải push (visitor).** `for_each(|t| ...)` đơn giản nhưng không break/continue được clean → phá `for` semantics §7.2.

## Hậu quả

- v0.1 `Range` và `Enumerate` interpreter dispatch (`advance_iterator`) là internal-only equivalent của `Iterator::next()`. Khi v0.2 trait Iterator landing, các Value variant này wrap vào struct implements Iterator → user code không thay đổi.
- `for` desugar dùng `loop { ... break }` — không bind expression value (loop in §7.2 đã hỗ trợ break-with-value, nhưng for không cần). Compiler có thể optimize away khi backend Cranelift v0.3 đến.
- Trait `Iterable` tách khỏi `Iterator` cho phép một collection được iterate nhiều lần (`coll.iter()` hai lần OK), trong khi raw `Iterator` (đã in flight) không thể.

## Implementation roadmap

| Phase | Deliverable |
|---|---|
| v0.1 ✅ | Hardcoded `Range`, `Enumerate` qua `advance_iterator` (commit `06025bb`) |
| v0.2 | Trait `Iterator<T>`, `Iterable<T>`; refactor `Range`/`Enumerate` thành Iterable structs; adapter `map`/`filter`/`take`/`zip` |
| v0.3 | Performance pass: tránh allocation cho adapter chains (state machine fusion) |
