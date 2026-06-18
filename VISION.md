# Triết — Vision

> Một ngôn ngữ tam phân cân bằng, **AI-first**, thiết kế để không bao giờ đóng
> cánh cửa xuống tới hệ điều hành.

Tài liệu này là **north star** dài hạn của dự án. Mọi quyết định kiến trúc lớn
phải đối chiếu với tầm nhìn ở đây. Khi tầm nhìn cần thay đổi, sửa tài liệu này
trước, không phải sau.

> **Trung thực trước (đọc trước mọi thứ).** Triết là tác phẩm dài hạn của **một
> người** (Giang Hoàng), với phần hiện thực kỹ thuật được giao cho AI. Giá trị
> của nó nằm ở **craft + một giả thuyết đo được**, KHÔNG ở adoption hay ở việc
> chờ một sự kiện phần cứng. Tài liệu này cố tình KHÔNG hứa những thứ một dự án
> một-người không thể giao. Mỗi tham vọng ở đây được dán nhãn rõ là **ràng buộc**
> (kiểm chứng được, kỷ luật từng quyết định) hay **cảm hứng** (định hình bản sắc,
> không phải lời hứa). Lẫn lộn hai cái là cái bệnh đã giết phiên bản cũ của tài
> liệu này — xem §0.

---

## 0. Bài học: tại sao tài liệu này được viết lại

Ngày 2026-06-04, toàn bộ backend compiler v0.2–v0.10 (VM bytecode, interpreter,
self-host ~23K LOC, ~1637 test) bị **xóa vĩnh viễn** để rewrite từ nền lên. Lý
do không phải code tệ — mà vì **nền sai**: dự án đã tự lừa mình về việc nó *là*
cái gì.

Phiên bản cũ của VISION này viết như thể Triết là một **substrate thay-đổi-thế-giới,
đủ sức viết OS, chờ phần cứng tam phân xuất hiện**. Đó là sương mù: một mục tiêu
*không thể chứng minh sai* (không ai bác bỏ được "khi phần cứng tam phân tới…")
nên không kỷ luật được bất kỳ quyết định nào. Nó cũng dán nhãn "✅ shipped" cho
những trụ cột mà compiler chứa chúng nay **đã bị xóa**.

Bản viết lại này commit vào một kỷ luật: **mọi mục tiêu phải hoặc kiểm chứng
được, hoặc được gọi thẳng tên là cảm hứng — không có vùng xám ở giữa.**

---

## 1. Tại sao Triết tồn tại

Triết là một thí nghiệm có chủ đích: **một ngôn ngữ được thiết kế tường minh đến
mức tối đa, để kiểm tra một giả thuyết — rằng ngôn ngữ + diagnostic có thể được
chế tạo sao cho một AI hội tụ về code đúng nhanh hơn ở các ngôn ngữ mainstream.**

Bản sắc của nó là **tam phân cân bằng** `{-1, 0, +1}` và logic **Łukasiewicz Ł3**.
Hai thứ này không phải lời hứa hiệu năng — chúng là **căn tính thẩm mỹ và cảm
hứng** (Setun 1958, Łukasiewicz 1920), và chúng làm một số thứ *sạch ở tầng kiểu*:
- Số có dấu không cần two's complement.
- Logic ba giá trị (true / false / unknown) là first-class, không bolt-on.
- `null` bẩm sinh qua 1-trit discriminator, không phải patch về sau.

Giá trị của Triết, nói thẳng, là **(a)** một tác phẩm kỹ thuật mạch lạc với kỷ
luật cao (ADR-driven, schema-first, compiler-không-bao-giờ-panic), và **(b)** một
giả thuyết AI-first đo được (§5). Không phải "thế giới cần một ngôn ngữ thứ N".

## 2. Triết KHÔNG là gì (đọc sớm — chống scope creep)

- **Không phải đối thủ của Rust/C++/Go.** Triết không cạnh tranh adoption. Nó là
  một tác phẩm + một thí nghiệm.
- **Không phải lời hứa giao một OS.** OS-capable là một **ràng buộc thiết kế**
  (§7), không phải một đích đến trên roadmap. Dự án này sẽ không "build microkernel".
- **Không cá cược vào phần cứng tam phân.** Lợi thế phần cứng của tam phân là lý
  thuyết và đã thua nhị phân suốt 70 năm (§6). Triết không chờ Setun-2.
- **Không phải "AI-first đã được chứng minh".** Đó là một **giả thuyết chưa đo**
  (§5). Bất kỳ ai đọc tài liệu này thấy nó được tuyên như sự thật → đó là bug
  của tài liệu, báo lại.
- **Không phải fast-iteration scripting.** Trade-off ngược: stability cao hơn,
  pace chậm hơn. Đó là tính năng.

## 3. Trạng thái thật hôm nay (không tô vẽ)

Compiler hiện hành là **bản rewrite** (2026-06 trở đi). Một pipeline duy nhất:

```
.tri → lexer → parser → modules → typecheck   [REUSED, well-tested]
     → lower (AST→MIR) → mir → borrowck (NLL) → jit (Cranelift)   [NEW]
     → driver (check / run)
```

Đã chạy end-to-end trên JIT: scalar, arithmetic (range-enforced trap-on-overflow,
ADR-0044), logic Ł3/K3, control flow, đệ quy, flat struct (StackSlot + sret),
enum, String/Vector/HashMap (heap shim, move-only + Deinit), nullable `T?`
(PA-3c sentinel), NLL borrowck (E2420/E2440/E2450), MIR verifier.

**Chưa có (nói thẳng):** AOT native, self-host, native multi-field layout, borrow
params cho heap, Outcome 2-reg ABI, freestanding/no-std target, bề mặt raw-memory.
Heap types hiện **phụ thuộc allocator + std của Rust** qua `extern "C"` shim. Lưới
test: ~1086 workspace test + 72 fixture driver. Backlog thật ở [`TODO.md`](TODO.md).

> Hệ quả trung thực cho §7: Triết **chưa** OS-capable *trong hiện thực*. Nó đang
> *duy trì các ràng buộc* để không đóng cửa đó. Khác nhau một trời một vực.

## 4. Ba trụ cột thật (phục vụ một chủ: vòng hội tụ AI)

Đây là những trụ cột **kiểm chứng được**, và tất cả đều quy về §5:

### 4.1 — Tường minh triệt để (Explicit > implicit)
Export, capability, dependency, ownership, ABI surface — tất cả tường minh. Glob
imports, default-public, ambient capabilities, suy luận kiểu ngầm ở biên — **bị
cấm**. Ít đường để AI bịa, ít trạng thái ẩn để AI đoán nhầm.

### 4.2 — Diagnostic máy-sửa-được (ADR-0027 là hiến pháp)
Mọi lỗi theo format AI-first khóa ở [ADR-0027](docs/decisions/0027-diagnostic-format-standard.md):
header `EXXXX Tên` + body + span + khối `[Fix N]` mệnh lệnh (`Change/Wrap/Use/Add/
Replace/Move X to Y`). 

Để tránh lãng phí năng lực AI vào các tác vụ cơ học, diagnostic được chia làm hai lớp rõ rệt:
- **Machine-Applicable (Deterministic)**: Lỗi mà compiler biết chắc chắn cách sửa. Những lỗi này phải được sửa tự động hoàn toàn bởi công cụ của compiler (ví dụ: `triet fix`), tuyệt đối không bắt LLM làm thay vai trò của một AST transformer.
- **Intent-Dependent**: Lỗi đòi hỏi hiểu ý định của lập trình viên (ví dụ: mismatch kiểu dữ liệu phức tạp, vi phạm borrowck liên quan đến logic). Đây mới là nơi LLM phát huy vai trò thông qua vòng phản hồi diagnostic để sửa code. Đây là tài sản trung tâm của giả thuyết §5.

### 4.3 — Refuse over guess
Khi compiler không chắc → error rõ ràng, **không suy luận im lặng**. Một soundness
hole với test xanh tệ hơn một test đỏ. Borrowck giả định alias khi không chắc
(conservative). Sai phải lộ ngay để vòng hội tụ §5 bắt được.

## 5. Giả thuyết AI-first (CHƯA ĐO — đây là điều quan trọng nhất phải trung thực)

> **Trạng thái: GIẢ THUYẾT. Chưa có công cụ đo. Instrument sẽ xây sau.**
> Không một dòng nào trong tài liệu này được tuyên "đã đo" về AI-first.

### 5.1 Sự thật phũ phải đặt lên bàn trước
Cái quyết định "LLM sinh code đúng" **không phải độ sạch cú pháp — là khối lượng
corpus huấn luyện**. Một LLM đã nuốt hàng tỉ dòng Python; Triết có **0 dòng** trong
corpus. Một ngôn ngữ mới toanh là trường hợp **TỆ NHẤT** cho LLM, không phải tốt
nhất. Vì vậy luận điểm *"LLM viết Triết đúng ngay lần đầu hơn viết Rust"* là
**thua chắc** và Triết KHÔNG đặt cược vào nó.

### 5.2 Luận điểm duy nhất sống sót: vòng hội tụ
> **Khi LLM viết SAI (nó sẽ luôn sai vì 0 corpus), Triết đưa nó về xanh trong ÍT
> LƯỢT hơn** — nhờ explicit syntax (§4.1) + diagnostic máy-sửa-được (§4.2) +
> refuse-over-guess (§4.3). Không phải "phát một trúng"; là "hội tụ nhanh".

Đây là luận điểm chơi đúng vào ba trụ cột §4 *đã tồn tại trong code*, và nó
**đo được**.

### 5.3 Instrument dự kiến (sẽ xây sau — chưa có)
Khi xây, phép đo tối thiểu:
1. `N≈20` task nhỏ.
2. Cho LLM **spec + vài ví dụ trong context** (in-context learning thay corpus),
   yêu cầu sinh Triết.
3. Chạy qua `triet-driver`. Nếu fail → **đút nguyên diagnostic trở lại**, cho sửa,
   lặp.
4. Đo **turns-to-green** + **tỉ lệ tự-sửa-đúng từ diagnostic**.
5. Baseline trung thực: KHÔNG so first-try với Python (confound corpus) — so
   *vòng hội tụ*: diagnostic dẫn model sửa đúng, hay làm nó loanh quanh.

Mục tiêu dài hạn: biến turns-to-green thành một **gate hồi quy thiết kế** (kiểu
`scripts/gate.sh`). Thêm cú pháp / sửa diagnostic mà turns-to-green tăng = vừa làm
ngôn ngữ tệ đi cho AI. Đó là lúc "AI-first" ngừng là khẩu hiệu và thành kỷ luật.

## 6. Tam phân: bản sắc, không phải tham vọng

Trit / Tryte / Integer (27 trit) / Long (81 trit), balanced ternary arithmetic, và
Łukasiewicz Ł3 (mặc định) / Kleene K3 là **kiểu nguyên thủy first-class** — không
phải library trên hệ nhị phân. Đây là căn tính không thể lột bỏ của Triết.

> [!IMPORTANT]
> **Thừa nhận trung thực về biểu diễn:**
> Trên phần cứng nhị phân, "tam phân" của Triết phần lớn là câu chuyện ngữ nghĩa và tầng kiểu (type-level semantics), KHÔNG phải biểu diễn vật lý ở runtime. `Integer`/`Long` được compile thẳng ra số nhị phân `i64` có giới hạn giá trị nằm trong dải tam phân — không đóng gói trit, không chia-cho-3 khi tính toán cơ bản. Phép cộng `5 + 3 = 8` chạy bằng chỉ thị `iadd` native của CPU nhị phân.
>
> Ai gọi đây là "fake ternary ở runtime" — đúng, và ta **NHẬN**: bản sắc tam phân của Triết sống ở tầng kiểu + logic, không ở mạch điện nhị phân.

Nhưng nói thẳng về phần cứng: lợi thế tam phân là **radix economy** (~5% lý thuyết
trên `số_chữ_số × cơ_số`, base 3 gần *e* nhất) — một con số đồ chơi đã biết từ 1950.
Nhị phân thắng vì **kỹ thuật**: transistor bistable chống nhiễu vượt trội. Setun bị
bỏ vì thua công trạng kỹ thuật, không vì âm mưu. Memristor ternary là research-stage
"còn 5 năm nữa" suốt 20 năm.

**Triết KHÔNG cược vào phần cứng tam phân.** Backend tam phân (trytecode) là *cảm
hứng cuối chân trời*, không phải milestone. Giá trị của tam phân ở đây là **thẩm mỹ
ngôn ngữ và sự sạch sẽ tầng kiểu** (null/sign/3-trạng-thái), không phải hiệu năng
phần cứng.

> [!NOTE]
> **Nợ thiết kế về biểu diễn dữ liệu dày (G nêu 2026-06-18):**
> Khi cần lưu trữ mảng/vector Trit lớn, ta đối mặt với trade-off:
> 1. Đóng gói chặt lý thuyết (base-3 packing): Tiết kiệm bộ nhớ tối đa nhưng đòi hỏi phép nhân/chia cho 3 để trích xuất, vốn cực kỳ chậm trên CPU nhị phân.
> 2. Đóng gói thực dụng (2-bit/trit): Phí ~25% không gian so với **đóng gói nhị phân tối ưu (~1.6 bit/trit, vd 5 trit / 8-bit)** — KHÔNG phải so với "1-bit" (một trit mang log₂3 ≈ 1.585 bit, không nhét nổi vào 1 bit). Trích xuất bằng bit-mask/shift, tránh phép chia. (Lưu mỗi Trit *trần* trong i64 mới là phung phí ~40×.) Tuy nhiên, bất kỳ phép toán số học song song nào trên mảng packed này cũng phải unpack trước (hoặc dựng bộ cộng SWAR bitwise tốn kém) vì quy tắc lan truyền số dư (carry propagation) của nhị phân khác tam phân.
>
> Đây là bài toán biên-biểu-diễn hẹp, chưa giải, sẽ được quyết định bằng ADR khi có nhu cầu FFI nhị phân hoặc dữ liệu dày.

## 7. OS-capable: một RÀNG BUỘC, không phải một đích đến

Đây là phân biệt sống còn, đã được tranh luận và chốt (2026-06-18):

- **Tiền đề hợp lệ:** no-GC + memory-safe + native → đủ tư cách viết OS *trên phần
  cứng nhị phân*. Rust đã chứng minh: Redox, Hubris, driver trong Windows/Android.
  Đây KHÔNG phải fantasy. Triết lấy cảm hứng đúng chỗ này từ Rust.
- **Nhưng KHÔNG phải đích đến.** "Build một OS/microkernel" là multi-hundred-
  person-year — một dự án một-người không bao giờ chạm tới. Đặt nó làm milestone =
  tái phạm tội sương mù §0.

Vì vậy OS-capable ở đây là một **ràng buộc thiết kế, kiểm chứng trên từng feature**:

> **Ngôn ngữ KHÔNG BAO GIỜ được đòi một managed runtime hay GC bắt buộc. Core phải
> biểu đạt được trong môi trường freestanding (không-OS). Phải có đường tới raw
> pointer + manual memory model khi cần.**

| Phải giữ (ràng buộc) | Phải cấm (đóng cửa OS) |
|---|---|
| Manual memory model (ownership/RAII, không GC bắt buộc) | GC bắt buộc kiểu JVM |
| Core biểu đạt được freestanding/no-std | Managed runtime singleton ép buộc |
| Đường tới raw pointer / FFI / syscall primitive (thiết kế) | Type erasure / sandbox runtime ép buộc |
| Trit/Tryte/Integer/Long fixed-size, không padding ambiguity | "Implementation-defined" mơ hồ ở ABI |

Ràng buộc này **test được trên mỗi feature**: "feature này có ép GC không? có chạy
không-std được không?" — và nó kỷ luật quyết định ngay hôm nay, kể cả khi OS không
bao giờ được viết. **Đó là toàn bộ giá trị của nó.**

> Trạng thái: Triết hiện **chưa** thỏa ràng buộc này trong hiện thực (heap sau shim
> Rust/std, chưa freestanding — §3). Đây là **ràng buộc đang được duy trì hướng
> tới**, không phải năng lực đã có. Khoe khác đi là nói dối.

## 8. Di sản đã thiết kế — đã từng build — đang chờ rebuild

Năm "trụ cột" của compiler cũ KHÔNG bị vứt; chúng đã được thiết kế kỹ, **từng được
build (v0.2–v0.10), rồi bị xóa cùng compiler cũ**. ADR còn sống là di sản thật;
hiện thực thì chưa được rebuild trong pipeline mới. Trạng thái trung thực:

| Thiết kế | ADR (còn sống) | Hiện thực trong rewrite |
|---|---|---|
| CAS Packaging (hash-based identity, Unison-inspired) | 0014, 0015 | **Đã xóa, chưa rebuild** |
| Module System (hierarchical, explicit export) | 0005 | Frontend REUSED, đang chạy |
| Stable ABI (witness tables, refuse-to-link) | (v0.4 design) | **Đã xóa, chưa rebuild** |
| Crate-Pack & Hybrid Linking | 0011–0013 | `triet-pack` còn, **chưa wire** |
| OS-Native Capability (Trit-level + Ł3 Unknown) | 0016, 0017, 0018 | **Đã xóa, chưa rebuild** |

Điểm **nhất quán (coherence)** đáng giữ trong số này: **capability tam phân** (`-1`
deny / `0` ambient / `+1` grant native) + **Łukasiewicz capability checking**
(`Unknown` resolved bởi runtime policy). Đây không phải là tính năng "novel" chưa từng có
(nhị phân hoàn toàn làm được bằng `enum Permission`), mà là sự đồng bộ hóa (coherence) — dùng
chung một đại số Ł3 thống nhất cho cả kiểm soát lỗi, logic tính toán và phân quyền trong ngôn ngữ.
Đáng rebuild — khi tới lượt. Phần còn lại (CAS/ABI/module/link) là *prior art làm tốt*: giữ
vì craft và vì §7.

## 9. Nguyên tắc thiết kế (commit hard)

| Nguyên tắc | Ý nghĩa |
|---|---|
| **Stability over speed** | Mọi quyết định kiến trúc có ADR. Không "ship đại rồi sửa". |
| **Refuse over guess** | Compiler không chắc → error rõ ràng, không suy luận im lặng. |
| **Explicit > implicit** | Export, capability, dependency, ABI — tường minh. Glob, default-public, ambient — cấm. |
| **Soundness > test color** | Test xanh không chứng minh đúng. Adversarial self-audit trước khi nói "xong". |
| **Prior art over invention** | Đứng trên vai Unison/Mojo/Pony/Swift/Genode. Phát minh chỉ ở chỗ tam phân thực sự khác. |
| **Honest over impressive** | Không "✅ shipped" cho thứ đã xóa. Không "đã đo" cho giả thuyết. Không "OS-capable" cho ràng buộc chưa thỏa. |

## 10. Phạm vi & pace

Đây là tác phẩm dài hạn của một người, làm cùng AI. Pace **chậm là tính năng**.
Thước đo thành công KHÔNG phải adoption hay viết được OS — mà là: **(a)** một
ngôn ngữ + compiler mạch lạc, đúng đắn, kỷ luật; **(b)** giả thuyết AI-first ở §5
được trang bị công cụ đo và bắt đầu cho ra số. Mọi thứ khác — tam phân hardware,
OS thật, ecosystem — là *cảm hứng*, không phải cam kết.

## 11. Tham chiếu

**Ngôn ngữ:** [Unison](https://www.unison-lang.org/) (CAS, hash AST) ·
[Mojo](https://docs.modular.com/mojo/) (ABI metadata) ·
[Pony](https://www.ponylang.io/) (object capabilities) ·
[Swift](https://www.swift.org/) (stable ABI) ·
[Rust](https://www.rust-lang.org/) (no-GC memory safety; bằng chứng OS-trên-nhị-phân:
[Redox](https://www.redox-os.org/), [Hubris](https://hubris.oxide.computer/)).

**OS / capability:** [Genode](https://genode.org/) · [seL4](https://sel4.systems/) ·
[Plan 9](https://9p.io/plan9/) namespaces.

**Lý thuyết:** Łukasiewicz Ł3 (1920) · Setun (Brusentsov, 1958) — *cảm hứng lịch
sử, không phải lộ trình kỹ thuật*.

---

*Tầm nhìn này là cam kết dài hạn về craft và trung thực — không phải về adoption
hay về việc chờ phần cứng. Pace chậm là tính năng, không phải bug.*
