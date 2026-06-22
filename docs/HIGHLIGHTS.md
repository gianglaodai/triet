# Triết — Điểm sáng ngôn ngữ

> Tài liệu này trả lời một câu hỏi: **Triết khác gì các ngôn ngữ khác, và lập
> trình viên được lợi gì?** — không phải lý thuyết compiler, mà là trải nghiệm
> thực tế.
>
> Trung thực về trạng thái: mỗi điểm được gắn nhãn
> **✅ Đã chạy** (kiểm chứng được hôm nay bằng `triet-driver`) hoặc
> **🎯 Định hướng** (đã thiết kế / từng chứng minh ở compiler v0.2–v0.10, đang
> chờ rebuild — xem [VISION.md](../VISION.md)). Đừng trộn lẫn hai loại.

---

## Phần I — Điểm sáng kiểm chứng được hôm nay

Bốn điểm dưới đây **không ngôn ngữ chính thống nào có đồng thời**. Mỗi ví dụ
trích từ một fixture đang xanh trong `crates/triet-driver/tests/fixtures/` —
chạy thật, không phải mockup.

### 1. `null` là trạng thái bẩm sinh, không phải "lỗi tỷ đô" vá víu

Mọi ngôn ngữ nhị phân coi giá trị "thiếu" là thứ gắn thêm: con trỏ rỗng nguy
hiểm (C, Java `NullPointerException`) hoặc một lớp bọc `Option<T>` (Rust, Swift).
Triết có **trit thứ ba** sẵn trong phần cứng kiểu — `-1 / 0 / +1` — nên "thiếu
thông tin" (`~0`) là một *trạng thái của giá trị*, không phải một con trỏ rỗng
hay một lớp bọc. Hệ quả: `T ⊂ T?` là quan hệ con **bẩm sinh**, mở rộng từ `T`
lên `T?` không cần nghi thức.

```triet
// fixture 43 → 5 ; fixture 48 → 42 ; fixture 49 → 0
function main() -> Integer {
    let x: Integer? = 5;       // widening T -> T? : không cần wrap
    return x ?: 0;             // Elvis: lấy giá trị, hoặc 0 nếu ~0
}

// match trên T? — hai nhánh đối xứng, compiler bắt buộc vét cạn
return match x {
    ~+ val => val,             // có giá trị
    ~0     => 0,               // thiếu (null bẩm sinh)
};
```

**Lập trình viên được gì:** không bao giờ gặp null-pointer âm thầm; ít nghi thức
`Option` hơn Rust ở chiều widening; "thiếu" và "có" là hai nhánh `match` mà
compiler ép phải xử lý đủ.

---

### 2. Logic 3 giá trị ở tầng kiểu — phân biệt "có thể chưa biết" với "chắc chắn biết"

Đây là thứ **không có ở ngôn ngữ chính thống nào**. `Trilean` mang 3 giá trị
Łukasiewicz Ł3: `true / false / unknown`. Nhưng điểm sắc bén là **refinement**:
typechecker tách `Trilean` (*có thể* là `unknown`) khỏi `Trilean!` (đã chứng
minh tĩnh **≠** `unknown`). Câu `if cond` đòi `Trilean!`; nếu anh đưa một
`Trilean` chưa chứng minh, compiler chặn ngay (E1033) thay vì đoán.

```triet
// fixture 07 → -1   (Trilean encode: true=+1, unknown=0, false=-1)
function main() -> Integer {
    let a = true;              // true/false là Trilean! (đã refine)
    let b = false;
    let result = a && b;       // Ł3 AND ; refinement được giữ
    return result;             // false = -1
}
```

So sánh nguyên thủy (`Integer == Integer`) cho ra `Trilean!`, nên `if n <= 1`
dùng được trực tiếp. Còn `unknown` là `Trilean` — muốn rẽ nhánh trên nó, anh
buộc phải xử lý trường hợp "chưa biết".

**Lập trình viên được gì:** "biến này có thể chưa biết" được kiểm soát ở
compile-time, không bằng convention hay comment. Dữ liệu SQL `NULL`, cảm biến
thiếu, fuzzy logic — biểu diễn tự nhiên, không hack bằng `enum { Yes, No, Maybe }`.

---

### 3. Số học tam phân — không có vùng tối "tràn số âm thầm"

Two's complement có những góc tối kinh điển: `INT_MIN` bất đối xứng, tràn số là
hành vi không xác định (C) hoặc wrap âm thầm (Rust release). Balanced ternary
cho dải số **đối xứng quanh 0**, dấu là trit cao nhất — không cần two's
complement. Và compiler **bắt buộc trap khi tràn** thay vì trả kết quả sai lặng
lẽ (ADR-0044).

```triet
// fixture 74 → 1000000   (trong dải, không trap)
function main() -> Integer {
    let a: Integer = 1000000;
    let b: Integer = 1;
    return a * b;
}

// fixture 76 → LỖI BIÊN DỊCH E1036
function main() -> Integer {
    let x: Integer = 3812798742494;   // vượt dải Integer → chặn ngay compile-time
    return x;
}
```

Tràn lúc chạy → SIGILL rõ ràng (không phải kết quả sai); literal vượt dải →
chặn ngay lúc biên dịch (E1036), không đợi tới runtime.

**Lập trình viên được gì:** lớp bug "wrap-around" kinh điển biến mất; tràn số là
tín hiệu nổ to, không phải con số sai âm thầm trôi qua hệ thống.

---

### 4. Cú pháp thiết kế CHO LLM sinh đúng ngay lần đầu

Đa số ngôn ngữ tối ưu cho người gõ ít phím: `fn`, `pub`, `mut`, `::`. Triết cố
tình đi ngược — **mơ hồ thấp quan trọng hơn ngắn gọn**:

| Triết | Ngôn ngữ khác | Vì sao |
|---|---|---|
| `function` | `fn` | đọc rõ, LLM ít nhầm |
| `public` / `public(package)` | `pub` / `pub(crate)` | tường minh phạm vi |
| `mutable` | `mut` | không viết tắt |
| `crate.foo.bar` | `crate::foo::bar` | dot-path quen thuộc |
| `from std.io import println` | `use std::io::...` | explicit, cấm glob |

```triet
// fixture 06 → 55 : cú pháp đọc như văn xuôi
function fib(n: Integer) -> Integer {
    if n <= 1 {
        return n;
    }
    return fib(n - 1) + fib(n - 2);
}
```

Error message cũng theo chuẩn máy-sửa-được (ADR-0027): header `EXXXX` + khối fix
mệnh lệnh "Change X to Y" — để bất kỳ ai (người hay LLM) sửa đúng phát đầu.

**Lập trình viên được gì:** trong kỷ nguyên code do AI sinh, **độ chính xác
lần-đầu** đáng giá hơn số ký tự tiết kiệm. Glob import, default-public, ambient
capability — bị cấm, nên không có "ma" ẩn.

---

### 5. An toàn bộ nhớ kiểu Rust — nhưng **bỏ được `<'a>`**

Triết mượn cái mạnh nhất của Rust (static borrow check, không GC) rồi **bỏ đi
cái khó nhất**: annotation lifetime `<'a>`. Triết **không có cú pháp đó**. Compiler
suy lifetime bằng 3 quy tắc elision; chỉ khi thật sự mơ hồ mới bắt anh refactor
(E2400), thay vì bắt anh chú thích mọi nơi. Reference cũng theo tam phân `+/0/-`:
`&+` chủ sở hữu duy nhất, `&0` mượn theo scope, `&-` quan sát yếu.

```triet
// fixture 94 → LỖI E2440 : độc quyền mutable, đúng như Rust nhưng không cần annotation
function main() -> Integer {
    let m = "hello";
    let a = &0 mutable m;
    let b = &0 mutable m;     // hai mượn-mutable cùng lúc → bị chặn
    return 0;
}

// fixture 81 → LỖI E2400 : compiler tự suy lifetime, chỉ kêu khi không suy nổi
function bad_no_param() -> &0 String {
    return "literal";          // không có input borrow để buộc lifetime → refactor
}
```

So sánh trực diện: cùng độ an toàn của Rust (use-after-move, độc quyền mutable,
không dangling), nhưng người viết **không gõ một ký tự lifetime nào**.

Sau Bậc D (ADR-0049): heap mutation thật (`append` với realloc + fat-pointer
writeback) chạy qua 5-boundary round-trip — fixture 100 — vẫn không cần annotation.

```triet
// fixture 100 → 1 : String qua 5 boundary: param fat, append realloc,
// sret return, caller append tiếp, eq content — không hề có <'a>
```

**Lập trình viên được gì:** rào cản học tập lớn nhất của Rust (`<'a>`, `'static`,
HRTB) biến mất; vẫn được an toàn bộ nhớ tĩnh, không GC, không null-deref —
kể cả với mutation in-place trên heap.

---

## Phần II — Định hướng kiến trúc (đã thiết kế, đang rebuild)

Bốn điểm dưới đây là **lý do tồn tại dài hạn** của Triết. Chúng đã được thiết
kế đầy đủ (ADR) và từng chạy ở compiler v0.2–v0.10, nhưng backend hiện tại
(rewrite 2026-06-04) **chưa rebuild** chúng. Trình bày ở đây như *trajectory*,
không phải feature dùng được hôm nay. Chi tiết: [VISION.md](../VISION.md).

### 6. ABI ổn định **bẩm sinh** — vá đúng gót chân Achilles của Rust/C++
`Trit/Tryte/Integer/Long` kích thước cố định: không struct padding ambiguity,
không endianness, không overflow ambiguity. ABI primitives ổn định *trước khi
viết dòng compiler nào*. Generics qua biên package dùng witness tables
(Swift-style) thay vì monomorphize phá binary compat. → *VISION §3.3.*

### 7. Capability là Trit, không phải boolean cấp/cấm
Quyền truy cập tài nguyên OS (`sys.*`/`dev.*`/`usr.*`) là **Trit**: `-1` deny /
`0` ambient / `+1` grant. Khi `unknown` → giải quyết lúc runtime bởi policy qua
logic Ł3 — thứ logic 2-giá-trị (Pony, seL4) không biểu đạt được. VISION gọi đây
là "trụ cột novel nhất". → *VISION §3.5, §5.*

### 8. Định danh code bằng hash nội dung (CAS) — giết DLL Hell ở gốc
Module định danh bằng **hash nội dung**, không phải version string. Chạy song
song N phiên bản không xung đột; build deterministic; nhiều app dùng chung một
hàm chỉ load 1 bản vào RAM. Tách `iface_hash` khỏi `impl_hash` → sửa thân hàm
không trigger rebuild downstream. Đi theo vai Unison/Nix. → *VISION §3.1.*

### 9. IR tách rời backend — lý do JVM không viết được OS mà Triết nhắm tới
JVM bake managed runtime + GC vào IR → vĩnh viễn không xuống được hardware.
Triết tách rạch ròi **IR là spec, backend là implementation**: cùng source
`.tri` → cùng IR → nhiều backend (VM dev → Cranelift JIT → LLVM AOT → trytecode
native khi phần cứng tam phân xuất hiện). Người dùng không sửa code khi đổi
target. → *VISION §4.*

### 10. Sở hữu S6: bỏ được `unsafe`, bỏ được GC, weak là công dân ngôn ngữ
Mô hình sở hữu S6 mượn borrow-check của Rust rồi bỏ tiếp ba thứ nữa (ngoài
`<'a>` ở mục 5):
- **Không keyword `unsafe`.** Hành vi nguy hiểm (raw pointer, FFI, transmute,
  MMIO) khai báo ở *một chỗ* — manifest `dao.package` — thay vì rải rác
  `unsafe {}` khắp code. Audit tập trung, không phải đi săn.
- **Không GC, không `Rc`/`Arc` thủ công.** Định lý vô-chu-trình: `&+` duy nhất +
  move làm chu trình toàn-strong *bất khả thi về toán học* → không cần cycle
  collector. Share immutable cross-thread thì compiler tự chèn refcount ở
  boundary, user không viết `Arc::clone`.
- **`&-` weak observer ở tầng cú pháp**, deref trả thẳng `T?` — Rust phải nhập
  `Weak<T>` từ thư viện.
- **BYOS:** `async`/`await`/`actor` *không phải keyword* → tránh phân mảnh
  runtime kiểu tokio-vs-async-std. → *SPEC §10, ADR-0022/0025/0026.*

> Phần *đã chạy* của S6 (borrow params `&0`/`&+`, độc quyền mutable E2440, suy
> lifetime không-`<'a>`) nằm ở **mục 5, Phần I** — kiểm chứng được hôm nay. Bốn
> gạch đầu dòng trên là phần còn-design.

---

## Tóm một câu

Cái khiến Triết khác biệt và **kiểm chứng được hôm nay**: tam phân first-class →
null bẩm sinh + logic 3 giá trị có refinement ở tầng kiểu + số học trap-on-overflow
+ an toàn bộ nhớ kiểu Rust nhưng bỏ được `<'a>`, gói trong một cú pháp thiết kế
cho AI. Cái khiến Triết **đáng tồn tại dài hạn**: ABI ổn định bẩm sinh, capability
tam phân, CAS packaging, sở hữu S6 không-`unsafe`-không-GC, và một IR đủ thấp để
một ngày viết được OS — kể cả trên phần cứng tam phân.

---

## Phần III — Gieo mầm (ý tưởng chưa cam kết) 🌱

> Đây **không phải** điểm sáng đã có — là vùng ý tưởng để quay lại sau. Chưa ADR
> (trừ chỗ ghi rõ), chưa thiết kế, có thể bị bác. Ghi lại để không quên.
>
> Hai nhóm: **(A) ý tưởng tam phân** (tầng 1–3 dưới) đi qua bộ lọc gimmick;
> **(B) học từ ngôn ngữ khác** (ngoài trục tam phân — mục "Học từ Odin").

**Bộ lọc trước khi nhận một ý tưởng *tam phân* vào đây:** tam phân chỉ là điểm sáng
khi domain **có sẵn** cấu trúc 3 trạng thái đối xứng quanh một điểm trung tính
(`-1 / 0 / +1` = âm / trung tính / dương). Nếu phải *ép* một thứ thành "cho đủ
3" mà không có điểm trung tính thật → đó là gimmick, **từ chối**.

### Tầng 1 — khớp hoàn hảo, kiểm chứng được trong backend hiện tại

1. **So sánh 3 chiều `compare() -> Trit`** — `less=-1 / equal=0 / greater=+1`.
   Khác Rust: `Ordering` là enum *không cộng được*; Trit **là số** → kết quả so
   sánh đút thẳng vào số học (`sign`, sort key, lexicographic fold
   `cmp_a*3 + cmp_b`). C `strcmp` trả int dấu mơ hồ; Triết trả Trit có kiểu.
   **Cùng insight "discriminant là số, không phải enum":** Outcome `~+/~0/~-`
   đã là tam phân bản địa — discriminant của nó là một Trit (+1 ok / 0 absent /
   −1 err), có thể fold chuỗi Outcome bằng số học Trit (min-Trit = "fail nếu
   bất kỳ cái nào fail", giống Ł3 AND) thay vì chuỗi `match` lồng. Cả hai chờ
   rebuild (Trait system cho #1, Outcome lowering cho mầm Outcome).
   *Trạng thái: đã LOCKED — ADR-0038, chờ Trait system.*

2. **Làm tròn không thiên lệch bẩm sinh** — trong balanced ternary, **cắt cụt
   phần phân số = làm tròn đến số gần nhất**, luôn luôn, vì phần phân số toán học
   nằm gọn trong `[-½, +½]`. Không cần "round half to even" (banker's rounding),
   không bias tích lũy. Điểm sáng cho fixed-point / DSP / tài chính. Nhị phân
   **không thể** sánh — đây là tính chất của *cơ số*, không phải thuật toán vá.
   *Trạng thái: ý tưởng — kiểm chứng được vì thuần số học, không cần Trait/ML.*

3. **Tri-state config / feature flag với `inherit = 0`** — thay `Option<bool>`
   (vốn mơ hồ "chưa set" vs "kế thừa"): một `Trit` field `+1` bật / `-1` tắt /
   `0` kế thừa caller — cùng semantics ambient của capability. Cascade kiểu
   CSS/env-override thành phép ưu tiên trên Trit, không phải cây `if let Some`.
   *Trạng thái: ý tưởng — kiểm chứng được (struct field + số học).*

### Tầng 2 — narrative định vị, chưa phải feature

4. **Coherence — một đại số Ł3 đâm xuyên mọi tầng (bản chất vật lý lên làm vua).**
   Bản chất vật lý của Triết là `{-1,0,+1}`: **cùng một** đại số Łukasiewicz Ł3
   chạy liền mạch — **vật lý** (`null` qua 1-trit discriminator, không patch về
   sau) → **logic** (Kleene/Łukasiewicz first-class, `unknown` là giá trị thật) →
   **capability** (`-1` deny / `0` ambient / `+1` grant, resolve `Unknown` bằng
   runtime policy). MỘT trục tư duy, không ba cơ chế chắp vá.
   Đối lập OOP: rải `null` khắp nơi rồi vá bằng `Optional` / NullPointerException-
   lúc-runtime; Triết neo nullability vào CÙNG đại số với logic và phân quyền — đó
   là **coherence** ([VISION §8](../VISION.md)), thứ không thể thay bằng tổ hợp
   ngôn ngữ khác. *Trạng thái: null ✅ + logic ✅ đã build; capability Ł3 = nhiệm
   vụ chiến lược sau heap-in-struct (ADR-0016/0017/0018). Đây là phòng tuyến giá
   trị THẬT của Triết — KHÔNG phải giả thuyết AI.*

### Tầng 3 — hệ quả nhỏ, gom làm nice-to-have

- **`sign(x) -> Trit` zero-cost** — dấu *chính là* trit cao nhất; nhị phân cần
  branch, Triết đọc 1 trit.
- **Three-way merge / VCS** (ours/base/theirs), **voting/consensus** (phiếu
  `-1/0/+1` cộng tự nhiên), **clamp trả Trit** (clamp dưới / không / trên).

### Học từ Odin — SoA + array programming (ngoài trục tam phân) 🌱

*Không phải ý tưởng tam phân — là điểm sáng performance học từ [Odin](https://odin-lang.org/).*

**Odin làm gì:** directive `#soa[N]T` khai báo "mảng struct" nhưng compiler lưu
dạng **Structure-of-Arrays** (mỗi field một mảng liền) thay vì **Array-of-Structs**
(struct xen kẽ). Mấu chốt: truy cập vẫn viết `arr[i].field` y hệt AoS —
**layout tách khỏi cách viết code**. Cộng *array programming* (toán tử
component-wise trên fixed-array), *swizzling* (`v.xyz`), `soa_zip`.

**Tại sao nhanh:** vòng lặp chỉ đụng 1–2 field qua N phần tử (game/ECS/physics)
→ SoA nạp đúng dữ liệu cần, không bẩn cache; field liền nhau → SIMD/vectorize dễ.
Cốt lõi data-oriented design.

**Điểm Triết nên học:** tách **mặt ngữ nghĩa** (nghĩ theo struct, dễ đọc) khỏi
**mặt layout** (compiler chọn bố cục tối ưu cache) — đổi layout bằng 1 directive,
không viết lại code. Cộng hưởng code rõ ràng (explicit) + tốc độ DOD, không đánh đổi.

**Giao thoa bản sắc Triết (chỗ thành lợi thế riêng, không chỉ học lỏm):**
- SoA của mảng `Trit`/`Tryte` → packing tam phân + layout tự nhiên cho **ternary
  SIMD** trên trytecode backend (v∞).
- **Cộng hưởng BitNet b1.58 (bản-sắc-tam-phân, KHÔNG phải claim AI):** Microsoft
  2024 ("Era of 1-bit LLMs") dùng trọng số ∈ `{-1,0,+1}` — chính là balanced
  ternary (*b1.58* vì `log₂3≈1.58`). SoA của mảng trit = layout tối ưu cho
  ternary-weight (`Trit` thật, không emulate qua int8). Odin không có mảnh "Trit
  native"; Triết có. *Trạng thái: Triết chưa có gì về ML — quan sát cộng hưởng,
  không phải feature.*

**⚠ Cảnh báo (đọc trước khi đụng):**
- Phụ thuộc **native multi-field layout** — Triết CHƯA CÓ (mọi value còn là `i64`,
  native layout là Bậc C future work). SoA nằm **sau** đó → mầm xa, đừng nhảy
  hàng trước fat-pointer/native layout.
- SoA làm khó borrowck S6: `&0 mutable arr[i]` = mượn nhiều mảng cùng lúc;
  aliasing của "element reference" trong SoA phải nghĩ kỹ với 5 reference form.
- **ABI-visibility là ADR bắt buộc** trước implement — hướng an toàn: SoA là tối
  ưu *intra-package* (như monomorphization), cross-package luôn AoS canonical để
  giữ trụ cột ABI ổn định.

### Thời điểm xem xét — bốn cửa dependency

> Cách đọc: mỗi mầm mở khóa khi một **cửa dependency** mở, không phải theo số
> version. Đây là suy luận từ phụ thuộc kỹ thuật (ROADMAP.md đang stale). Lưu ý
> `spec/` phase 1–6 là design cho cái *đã/đang* làm; hai dependency lớn nhất —
> **Trait system** và **native multi-field layout** — chưa có phase doc nào.

| Cửa | Mở khi | Mầm xem xét lúc đó | Ghi chú |
|---|---|---|---|
| **A — Ngay / song song** | thuần `triet-core` số học, không chặn gì | **#2 rounding**, **signum→Trit** (tầng 3) | Viết ADR *chốt tính chất* được ngay; implement #2 khi có kiểu fixed-point/phân số đầu tiên |
| **B — Trait system mở** | cột mốc lớn kế sau heap/Bậc D | **#1 `compare()→Trit`** (ADR-0038 LOCKED) | Kéo Ord/Eq/Hash + operator overloading ra cùng |
| **C — Capability rebuild** | Phase 6 Hardware Token + Phase 7 namespace | **#3 tri-state config `inherit=0`** | Cùng semantics `CapabilityLevel` 4-state → tổng quát hóa một lượt |
| **D — Native multi-field layout xong** | sau Bậc C/D (hết "mọi value là i64") | **SoA/array programming (Odin)** → rồi **#4 BitNet** | Bậc D fat-pointer (String 24B StackSlot) là bước nền đầu; native struct layout (TODO L43) vẫn `[ ]` → SoA chưa mở được; ADR ABI-visibility + aliasing-S6 phải có *trước* |

**Thứ tự cửa:** A song song bất kỳ lúc nào → B / C (không nhất thiết nối tiếp,
tùy thứ tự rebuild Trait vs Capability) → D xa nhất.

**⚠ Kỷ luật ưu tiên (Mentor O):** đường chính hiện tại là **Chiến Dịch Trả Nợ**
(Bậc D fat-pointer đã đóng `08b0acd`, O+G ký; A1/A2/A3 xong; kế tiếp B1 Type
System — xem `TODO.md`). Mọi mầm trên là *"khi tới cửa thì xem"*, **không phải
làm ngay**. Cám dỗ lớn nhất là kéo SoA/BitNet lên sớm vì chúng "kêu" — nhưng
chúng ở Cửa D, xa nhất. Cửa duy nhất động được ngay mà không tạo nợ là **Cửa A**,
và chỉ ở mức *viết ADR chốt tính chất*, không implement.
