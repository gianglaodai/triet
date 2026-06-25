# ADR 0069 — ZST Capability Token mang Ł3-Trit (rewrite-era, borrowck-enforced)

> # ⚖️🩸 NGUYÊN LÝ CỐT LÕI (G khắc đá 2026-06-25)
> # Capability = ownership. Thằng gác cổng (Borrow Checker) PHẢI nắm dây thòng lọng.
> Một capability mà borrowck không enforce bằng memory-safety chỉ là **đồ làm cảnh**.
> Grant/Ambient/Deny phải check **chết tại compile-time**; chỉ `Defer` (Unknown) mới
> được văng ra runtime hook. Không có cửa bypass.

**Trạng thái:** Đề xuất (scaffold giấy trắng — recon-trước file:line, CHƯA code; chờ G ký từng nhát).
Áp dụng Bậc C+. Đây là **mặt trận chiến lược hoàn tất COHERENCE VISION §8**: một đại số Ł3 duy
nhất xuyên **null (PA-3c) / logic (Trilean) / capability** — chân thứ ba còn thiếu.

**Issue — hai thế giới rời rạc, không cái nào đóng VISION §8:**
- **Thế giới 1 (package-manifest, ADR-0016/0017/0018):** đại số Ł3 4-trạng-thái ĐÃ code
  (`CapabilityLevel{Deny=Trit::Negative, Ambient=Trit::Zero, Grant=Trit::Positive, Defer=Trilean::Unknown}`
  tại `crates/triet-pack/src/types.rs:297`; `check_capabilities()` tại
  `crates/triet-typecheck/src/capability_check.rs:131`) — **NHƯNG orphan**: `grep capability
  crates/triet-driver/src` = RỖNG. Enforcement chết trong pipeline thật. Cơ chế `.khi`/`dao`/
  cross-package-linker đằng sau nó phần lớn đã xóa hoặc unwired → **G chôn 0016/0017/0018**.
- **Thế giới 2 (Hardware-Token ZST, `spec/plans/phase6` + schema §10):** capability = ownership +
  move enforced bởi borrowck, ZERO runtime overhead, coherent với No-Box vừa đóng (Trục B) —
  **NHƯNG "design only", và cố tình KHÔNG có Ł3-level** (binary "có token hay không").

**Quyết định chiến lược (G chốt HƯỚNG C — Synthesis, 2026-06-25):** hợp nhất hai thế giới.
**Cứu đại số Ł3 của Thế giới 1, vứt cơ chế package-manifest; xây trên cỗ máy ownership/move
của Thế giới 2.** ZST-token **ngậm chặt** Ł3-Trit: Grant/Ambient/Deny enforced bằng máu của
borrowck tại compile-time; Defer văng ra runtime policy hook + trap khi vi phạm.

**Quan hệ ADR:**
- **CHÔN:** ADR-0016 (package capability manifest), 0017 (policy resolution), 0018 (provenance
  prompt) — package-manifest era, không áp rewrite single-file/JIT. Không đào xác ướp.
- **Kế thừa:** S6 ownership (ADR-0022 §2, 5-form reference), borrowck NLL E2420 (ADR-0025),
  No-Box move/Deinit machinery (ADR-0066/0067), trap-on-violation 2-signal (ADR-0044).
- **Amend:** schema §10 `HardwareToken` (`spec/schema/triet-schema.yaml:1684`) — xem §8.

---

## 0. Current Reality (recon O đo file:line, 2026-06-25 — khắc đá, không đoán)

| # | Phát hiện | Chứng cứ (file:line) | Hệ quả thiết kế |
|---|---|---|---|
| 1 | ZST construction `Cap {}` (struct rỗng) **không lower** — rơi xuống nhánh biến → "undefined local variable: Cap". Struct rỗng **DECL** thì parse/typecheck/lower OK. | probe `triet-driver run` (struct-construct path, `triet-lower`) | **Lát-0 prerequisite**: phải cho ZST construct được trước mọi thứ. |
| 2 | `is_copy` cho `Struct` = `s.fields.iter().all(\|f\| f.ty.is_copy(...))`. Struct rỗng → `all()` trên iterator RỖNG → **`true` → COPY**. | `crates/triet-mir/src/lib.rs:666` | ZST token mặc định bị xếp **Copy** → borrowck KHÔNG move-track → bypass câm. **Token BẮT BUỘC ép non-copy.** |
| 3 | Substrate enforce **ĐÃ CÓ**: E2420 use-after-move gated trên `is_copy(Some(body))`; `ParameterPassing::Move`; 5-form `ReferenceForm`. Test `write_twice(vga: &+ mutable VgaBuffer)` + `consume(vga)` chạy đúng pattern ZST-move. | `crates/triet-borrowck/src/checker.rs:388,618,690,948,981` + `lib.rs:339` | **move = transfer + thu hồi = E2420 — đã xây.** Không phải làm lại. |
| 4 | Đại số Ł3 4-state ĐÃ code nhưng orphan khỏi driver pipeline. | `triet-pack/src/types.rs:297`; `triet-typecheck/src/capability_check.rs:131`; driver = ∅ | **Cứu ánh xạ Trit↔level; vứt cơ chế manifest.** |
| 5 | Trap infra: `TrapCode::unwrap_user(1)` (trapnz → SIGILL family), range-check ADR-0044 dùng sẵn. | `crates/triet-jit/src/mir_lower.rs:2509` | `Defer` runtime-hook **tái dùng** infra này (xem §5). |
| 6 | Schema §10: *"No special AST node ... No ACL, no syscall, **no runtime check** — capability = ownership."* | `spec/schema/triet-schema.yaml:1684,1742` | Synthesis **phá** giả định "no runtime check" (Defer có check) + thêm khai báo cú pháp → **ADR amend §10**, không lặng lẽ. |

---

## 1. Ánh xạ Ł3-Trit ↔ vòng đời capability (lõi coherence)

Đại số Ł3 (Łukasiewicz 3-trị) ánh xạ thẳng lên vòng đời một capability token. Đây CHÍNH là
chân thứ ba của coherence — cùng một `Trit{Positive, Zero, Negative} + Unknown` đã chạy ở
null (PA-3c sentinel) và logic (Trilean):

| Ł3 value | Level | Ngữ nghĩa capability | Enforce ở đâu | Chi phí runtime |
|---|---|---|---|---|
| `Trit::Positive` | **Grant** | Token được phép **mint** tự do; possession = quyền. Pure W2 ownership. | typecheck (cho mint) + borrowck (move/E2420) | **0 byte** |
| `Trit::Zero` | **Ambient** | **RECEIVE-ONLY (M1, G ký 2026-06-25 — xem §amend-A).** File MẤT quyền `mint X` (E2211 như Deny). Cách DUY NHẤT xài `X` = caller truyền token ZST qua **parameter** (signature explicit). Token đi xuống từ biên ngoài (entry-point), KHÔNG sinh trong chương trình. | mint → E2211; **possession qua param/binding = HỢP LỆ** | 0 byte |
| `Trit::Negative` | **Deny** | Token **KHÔNG được mint** + **CẤM TIỆT mọi sở hữu** (kể cả nhận qua parameter → lỗi). | mint → E2211; **possession (param/binding type) → E2212** | (không tồn tại) |
| `Trilean::Unknown` | **Defer** | Token mint được nhưng **có điều kiện**; mọi guarded-op emit runtime policy-hook check; deny → **trap**. | typecheck cho mint + JIT chèn runtime check | **1 check + trap** (cái giá DUY NHẤT) |

**Tính coherent:** ba giá trị tĩnh (Pos/Zero/Neg) giải quyết hoàn toàn compile-time = zero-cost
(đúng tinh thần W2 schema §10). Chỉ `Unknown` — bản chất "chưa biết" của Ł3 — mới defer ra
runtime. Đây không phải thỏa hiệp; đây là **ngữ nghĩa Ł3 đúng**: Unknown nghĩa là "không chứng
minh được tĩnh", nên phải hỏi runtime. Hệt như `Trilean` Unknown không thể vào `if` (E1033) mà
phải resolve.

---

## 2. Cú pháp bề mặt (Giang chốt 2026-06-25 — `capability` decl + `mint`)

```triet
capability VgaBuffer grant     // Grant: mint tự do, zero-cost
capability DiskWrite defer     // Defer: mint → runtime hook check
capability RawPort   deny      // Deny: mint = E2211, possession = E2212 (cấm tiệt)
capability UartPort  ambient   // Ambient: mint = E2211; CHỈ nhận qua param (receive-only)

function kernel_main(hw: Hardware) -> Unit {
    let vga = mint VgaBuffer;   // OK (grant) — ZST, 0 byte runtime
    vga_driver(vga);            // vga MOVED (transfer quyền)
    // vga_driver(vga);         // E2420 UseAfterMove — quyền đã thu hồi
}
```

- **Keyword mới: `capability`** (item decl) + **`mint`** (prefix op khởi tạo token). Không nằm
  trong refuse-list ADR-0026 v2 §6 (actor/spawn/receive/send/async/await) → hợp lệ.
- **`grant`/`ambient`/`deny`/`defer` = contextual keyword** (chỉ mang nghĩa ở vị trí level sau
  `capability Name`) → **KHÔNG reserve toàn cục**, không cấm user dùng làm identifier nơi khác.
  (Quyết định đúng-đắn của O; giảm xáo trộn surface — như `refined` không phải global keyword.)
- Một `capability X <level>` định nghĩa **ZST type** `X` (sizeof = 0) + gắn Ł3-level. Khác
  `struct X {}` thường ở chỗ: (a) luôn non-copy (xem §6), (b) mang level, (c) chỉ khởi tạo qua
  `mint` (không `X {}`).

---

## 3. Điểm móc AST/HIR (trả lời G câu a)

`capability X grant` → AST item **mới** `Item::Capability { name, level, span }`, KHÔNG nhồi vào
`UserStruct` (giữ `UserStruct` sạch; capability ≠ struct dữ liệu). Level = enum 4-state tái dùng
ánh xạ Ł3 của Thế giới 1.

- **Schema-first:** thêm node `Capability` + enum `CapabilityLevel{Grant,Ambient,Deny,Defer}` vào
  `spec/schema/triet-schema.yaml`, chạy codegen → `crates/triet-syntax/src/generated/`. **KHÔNG
  hand-edit generated** (Track B rule #2). `CapabilityLevel` của triet-pack (types.rs:297) là
  ánh xạ Ł3 đã chứng minh — schema mirror nó, nhưng đây là node AST rewrite-era độc lập (không
  kéo theo wire-format/manifest của 0016).
- **Type repr:** typecheck `Type` (`crates/triet-typecheck/src/types.rs`) thêm variant
  `Capability { name, level }` HOẶC tái dùng `UserStruct` rỗng + cờ. **Khóa theo recon Lát 1**
  (refuse-over-guess: chưa đo xong typecheck Type có chịu được variant mới không).
- **`mint X`** → AST `Expr::Mint { capability_name, span }`. Lower → khởi tạo ZST local (0 byte,
  giống `_ = const ()` nhưng typed Capability). Đây cũng vá Lát-0 (finding #1) cho ZST.

---

## 4. Luật enforce của Borrow Checker (trả lời G câu b — dây thòng lọng)

Capability token là **ZST non-copy**. Toàn bộ enforce **tái dùng** cỗ máy đã có (finding #3),
KHÔNG viết engine mới:

1. **Possession = ownership.** Có token ⇔ giữ một local kiểu Capability chưa bị move/drop.
2. **Move = transfer quyền.** `f(vga)` move token vào callee → caller mất quyền. Dùng lại
   `vga` → **E2420 UseAfterMove** (đã chạy, checker.rs). Đây là "thu hồi" G đòi.
3. **Không copy = không nhân quyền.** Token **bắt buộc** `is_copy == false` (§6), nên không có
   chuyện một token bị nhân đôi để hai bên cùng giữ quyền. Đây là chốt soundness sống-còn.
4. **Deny chặn mint + CẤM sở hữu.** `mint X` (deny) → **E2211** tại mint-site. Thêm: `X` xuất hiện
   làm KIỂU của bất kỳ binding/parameter/field → **E2212 CapabilityNotPossessable** (Deny cấm tiệt
   mọi hình thức sở hữu, kể cả nhận qua signature). Không token nào tồn tại được.
5. **Ambient = receive-only (M1).** `mint X` (ambient) → **E2211** (file mất quyền tự đúc). NHƯNG
   `X` làm kiểu parameter/binding = **HỢP LỆ** — caller truyền token xuống (O-Cap thuần: authority
   đi qua signature, không khí không tự sinh capability). Phân biệt với Deny: Ambient cho-nhận,
   Deny cấm-nhận. Resolve hoàn toàn compile-time, không bao giờ runtime.
6. **Grant = zero-cost.** mint hợp lệ → token ZST; runtime chỉ thấy địa chỉ phần cứng hardcode
   trong driver (W2 nguyên bản). 0 byte copied.

> **Bất biến soundness (teeth sẽ chứng minh bằng máu):** một guarded resource được "lấy" **đúng
> một lần** — borrowck chứng minh qua move-exactly-once. Poison `is_copy→true` cho Capability →
> double-take phải LỌT (E2420 không bắn) → đỏ. (Tái dùng nghi thức poison No-Box.)

---

## 5. Defer → runtime policy hook + trap (trả lời G câu c)

`Defer` (Ł3 Unknown) là trường hợp DUY NHẤT chạm runtime. Khi `mint X` với `X` level `defer`:

- Token vẫn mint được (ZST), borrowck vẫn move-track như Grant — **memory-safety không nới**.
- Nhưng JIT chèn, **tại mint-site** (hoặc guarded-op đầu tiên — khóa theo recon), một call tới
  **runtime policy hook** `extern "C" fn __triet_cap_check(cap_id: i64) -> i64` (Rust shim, cùng
  họ `__triet_*` ở `mir_lower.rs`). Hook trả Ł3-Trit: `+1` allow / `-1` deny / `0` (Unknown →
  treat-as-deny, fail-closed).
- **Deny → trap.** Hook trả ≤ 0 → `trapnz` (`TrapCode::unwrap_user(N)`, finding #5). Đề xuất
  **trap-code RIÊNG** (vd `unwrap_user(2)`) tách khỏi range-check (ADR-0044 dùng `user(1)`) → phân
  biệt được "capability denied" với "arithmetic overflow" khi điều tra core dump. SIGILL family.
- **Fail-closed:** policy hook vắng mặt / panic / trả Unknown → coi như deny → trap. Capability
  KHÔNG được mở khi không chứng minh được phép. (Đúng tinh thần refuse-over-guess.)

> Đây là chỗ ADR **phá** schema §10 "no runtime check". Hợp lý: §10 chỉ mô tả trường hợp tĩnh
> (Grant). `Defer` là opt-in CÓ Ý của lập trình viên vào một quyết định động — anh ta tự chọn trả
> giá 1 check. Grant/Ambient/Deny vẫn 0-cost. Zero-cost-by-default được giữ; runtime chỉ khi khai
> báo `defer`.

---

## 6. Sửa lỗ `is_copy` (finding #2 — chốt soundness)

`MirType` thêm phân loại Capability **luôn `is_copy == false`** — KHÔNG đi qua nhánh `Struct`
`all(empty)==true`. Hai hướng (khóa theo recon Lát 1):
- (a) `MirType::Capability(name)` variant mới → match arm trả `false` thẳng tại `lib.rs:666`.
- (b) Nếu tái dùng `MirType::Struct` cho ZST: thêm cờ "is_capability" + short-circuit `false`
  TRƯỚC `all()`.

**Teeth bắt buộc:** poison nhánh này về `true` → token mint→move→reuse phải thôi bắn E2420 →
test đỏ. (Nếu poison không đỏ = test trang trí, theo `feedback_poison_must_be_red`.)

---

## 7. Kế hoạch lát (scaffold — mỗi lát recon→WO→D code→O verify máu→O ký→G ký)

- **Lát 0 ✅ ĐÓNG (`8b06a28`):** `capability X grant` decl + `mint X` ZST 0-byte + `is_copy==false`
  (2-classifier defense-in-depth) + non-grant refuse E2211 + `public capability` refuse. Vá finding #1+#2.
  (Nuốt luôn phần Grant/Deny-mint của Lát 1.)
- **Lát 2 — Ambient receive-only + Deny no-possession (M1, G ký §amend-A):**
  (a) `mint X` ambient → E2211 message riêng "receive-only" (Lát 0 đang gộp bucket → tách).
  (b) **NEW possession-check:** `X` level `deny` làm KIỂU param/binding/field → **E2212**; ambient/grant
  làm kiểu = HỢP LỆ. Cổng: deny-param → E2212 · ambient-param → typecheck OK · mint-ambient → E2211.
- **Lát 3 — Defer runtime hook (§5):** shim + trap-code riêng + fail-closed. Cổng: hook deny → trap
  (SIGILL), hook allow → chạy; teeth poison fail-closed (hook vắng → phải trap).
- **Lát 4 — Hardware aggregate:** `struct Hardware { vga: VgaBuffer, ... }` ZST-aggregate + destructure
  move (schema §10 example end-to-end). Cổng: `kernel_main(hw)` re-use field sau move → E2420.

---

## Các phương án đã cân nhắc

**Hướng tổng thể (G chốt C):**
- **A — Wire W1 orphan vào pipeline:** rẻ, Ł3 4-state live nhanh. **Loại:** capability tách rời
  ownership; borrowck không nắm dây → "đồ làm cảnh" (G). Hai hệ rời rạc, không coherent No-Box.
- **B — Xây thuần W2 ZST-token:** coherent No-Box. **Loại:** bỏ phí đại số Ł3 4-state đã khắc đá;
  tự đẻ lại Trit-level → rủi ro soundness (G).
- **C — Synthesis (CHỐT):** ZST-token ngậm Ł3-Trit, borrowck-enforce, Defer→runtime. Đắt/khó/đụng
  borrowck-core — nhưng là con đường DUY NHẤT đóng VISION §8 không lỗ hổng.

**Cú pháp (Giang chốt — `capability` decl):**
- `capability X grant` + `mint` (CHỐT): tách capability khỏi struct thường; không đụng `UserStruct`.
- `@grant struct X {}` annotation: giữ schema §10 "no AST node" nhưng cần thêm trường AST trên
  `UserStruct` + codegen; lẫn capability với struct dữ liệu. **Loại.**
- Thuần W2 bỏ level khỏi token: đơn giản nhất nhưng **KHÔNG đúng mandate G** ("token ngậm chặt
  Ł3-Trit"). **Loại.**

**Trap-code Defer:**
- Tái dùng `user(1)` (range-check): lẫn capability-deny với arithmetic-overflow trong core dump.
- **`user(2)` riêng (đề xuất):** phân biệt được — chọn.

---

## Hậu quả

### Tích cực
- **Đóng COHERENCE VISION §8** — chân capability của đại số Ł3 (cùng `Trit + Unknown` với null/logic).
- Capability = memory-safety: borrowck chứng minh resource lấy đúng-một-lần, zero-cost cho Grant.
- Tái dùng 100% cỗ máy move/E2420 (No-Box) + trap (ADR-0044) — không engine mới.
- Chôn 0016/0017/0018 dứt khoát; capability sống trong pipeline thật (driver), hết orphan.

### Rủi ro cần mitigate (teeth)
- **R-copy-bypass (sống-còn):** token bị xếp Copy → double-take không bị E2420 → nhân quyền câm.
  Teeth: poison `is_copy→true` → re-use-after-move phải thôi đỏ.
- **R-deny-leak:** `mint deny` không bắn E2211 → quyền cấm vẫn sinh token. Teeth: poison check Deny.
- **R-defer-fail-open:** hook vắng/Unknown mà KHÔNG trap → bypass. Teeth: poison fail-closed → mint
  defer với hook-deny phải SIGILL; gỡ trap → phải lọt.
- **R-ambient-collapse-sai:** ambient resolve nhầm Deny→Grant. Teeth: ambient-trong-deny-scope.

## §amend-A — Ambient = Receive-only (M1, G ký 2026-06-25)

Scaffold §1 ban đầu ghi "Ambient = inherit caller's level → collapse Grant/Deny" — **mơ hồ, KHÔNG
cơ chế trong single-file JIT** (không package-hierarchy như W1). G mổ 3 model recon, chôn 2:
- **M2 Possession-gated** (mint nếu giữ token) — VỨT: cho `mint` dựa possession = tự nhân bản token
  non-copy = phá bất biến ZST move-only Lát 0.
- **M3 Call-graph reachability** — VỨT: action-at-a-distance, phá local-reasoning (hàm B rớt compile
  vì hàm A đâu đó gọi nó).
- **M1 Receive-only (CHỐT)** — O-Cap thuần, nhất quán ZST move-only:
  1. `capability X ambient` → file MẤT quyền tự đúc: `mint X` = **E2211** (message "receive-only").
  2. "Inherit từ caller" CỤ THỂ = caller truyền token ZST qua **parameter**; authority đi xuống từ
     biên ngoài (entry-point như `kernel_main(hw)`), KHÔNG sinh trong chương trình ("không khí không
     tự sinh capability").
  3. **Phân biệt Deny:** Deny cấm TIỆT mọi sở hữu (param/binding type của deny-cap → **E2212**);
     Ambient cấm mint nhưng CHO nhận qua signature. ⇒ Ambient = "explicit trên function signature,
     không tà đạo implicit".

## Amendment schema §10 (`HardwareToken`)
ADR này **sửa** hai câu của §10:
1. *"No special AST node"* → có `Item::Capability` + `Expr::Mint` (capability ≠ struct dữ liệu thuần;
   pure-W2 destructure vẫn dùng cho Hardware aggregate ở Lát 4).
2. *"No runtime check"* → đúng cho Grant/Ambient/Deny (tĩnh); **`Defer` thêm runtime hook + trap**
   (opt-in, cái giá duy nhất). Zero-cost-by-default được giữ.
(Schema patch đi kèm Lát 1, codegen-driven, không hand-edit generated.)

## Ngày hiệu lực
- Bậc C+: Lát 0→4 theo thứ tự, mỗi lát G ký riêng.
- Defer (Lát 3) là lát đụng runtime — review kỹ nhất.

---

**Chữ ký ADR-0069:** O ✍️ (recon + draft 2026-06-25) · **G ✅ (ký duyệt 2026-06-25 — ánh xạ Ł3 "ký bằng hai tay", fail-closed = chân lý, trap-code `user(2)` riêng bắt buộc, thứ tự lát giữ nguyên)** · Giang ✅ (chốt hướng C + cú pháp `capability`/`mint`)

**§amend-A (Ambient = M1 Receive-only):** O ✍️ (gói M1 2026-06-25) · **G ✅ (PHÁN M1, chôn M2/M3 — "tà đạo implicit, phá ZST move-only/local-reasoning")** · Giang ⏳

**Lát 0 ✅ ĐÓNG+PUSH `8b06a28` (O+G ký).** Lát 2 = WO đang phát.
