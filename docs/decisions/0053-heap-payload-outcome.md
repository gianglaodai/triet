# ADR-0053 — Heap payload trong Outcome (`T~E` mang String/Vector/HashMap)

- **Status:** 🔒 LOCKED — G ký duyệt 2026-06-10 (§8 phán quyết G).
- **Date:** 2026-06-10
- **Khởi thảo:** Mentor O (technical-quality owner). §8 chốt + ký bởi G.
- **Chữ ký:** O ✅ (khởi thảo + grounded) · G ✅ (ký duyệt 2026-06-10).
- **Tiền nhiệm:** [ADR-0052](0052-outcome-abi-implementation.md) (Outcome 2-slot, **defer heap payload** §6),
  [ADR-0042](0042-ownership-across-boundary.md) (Deinit/move-across-boundary),
  [ADR-0040](0040-heap-aggregate-layout.md) (heap layout), [ADR-0049] (String slot {ptr,len,cap}).

---

## 1. Context — tại sao bây giờ

Outcome 2-slot (ADR-0052) chạy end-to-end cho **scalar payload (i64)**: producer, consumer,
propagate `~->`, map `~+>`/`~->`, E1039. Nhưng `~- error` hiện bị `is_scalar` guard chặn —
**error message thực (`~- "file not found"`) bị E1037 từ chối.** Một error-handling system không
gánh nổi String là maquette. Heap payload là nút thắt duy nhất đứng giữa Outcome và tính dùng được.

ADR-0052 §6 defer việc này có chủ đích: *"heap payload Outcome (String/Vector) — ownership/drop/
borrow qua multi-return."* Đây là **delta nền đụng ownership** — không phải lát desugar. Cần ADR
trước code.

## 2. Sự thật nền (đo từ code, KHÔNG giả định)

| Sự thật | Nguồn | Hệ quả |
|---|---|---|
| Outcome slot hiện = **16 byte** `{disc@0:i64, payload@8:i64}` | `mir_lower.rs:689` | payload chỉ chứa 1 i64 |
| String Triết = **24 byte** `{ptr@0, len@8, cap@16}` — KHÔNG phải fat-pointer 16-byte | `mir_lower.rs:770,917,1057` | ⚠️ **đính chính G: heap value là 24-byte, không 16** |
| Vector/HashMap cũng `{ptr,len,cap}` = 24 byte | `mir_lower.rs` shims | union payload max = 24 |
| `Statement::Drop(local)` lower theo **type TĨNH** của local: String→`__triet_string_free(ptr,cap)`, Vector→`vector_free`, Copy→no-op | `mir_lower.rs:1023-1070` | ⚠️ **Drop hiện không biết rẽ nhánh runtime** |
| `Statement::Deinit(local)` = ghi 0 tombstone (KHÔNG free) — chống double-free sau move | `mir.rs:214`, `lower.rs:1894` | dùng được làm "đã-move-ra" marker |
| M4: Drop bị skip nếu local nằm trong `Return.values` (move sang caller) | `mir_lower.rs:1031` | escape analysis có sẵn |

**Điểm cốt tử rút ra:** Drop hôm nay là **type-static** (mỗi local một type cố định → một free shim
cố định). Outcome mang heap thì **payload sống là type gì phụ thuộc disc RUNTIME** → Drop của một
Outcome phải **rẽ nhánh trên disc**. Đây là cơ chế MỚI, là lõi của ADR này.

---

## 3. Ba câu hỏi chí mạng (G đặt) — trả lời

### 3.1. Q1 — Tombstone semantics: ai dọn String của arm bị bỏ rơi?

**Đính chính tiền đề:** payload là **tagged union**, KHÔNG phải hai ô song song. Một Outcome tại một
thời điểm giữ **HOẶC** T (nếu disc=Pos) **HOẶC** E (nếu disc=Neg) — đọc disc biết ô đang sống là
type nào. **Không tồn tại "String ~- thoi thóp nằm bên cạnh khi rẽ vào ~+".** Khi disc=Pos, payload
slot CHỨA T; chưa từng có E nào được ghi ở đó để mà rò.

Vậy nghĩa vụ drop thật là **drop glue có điều kiện của chính Outcome**, khi một Outcome owned rời
scope mà **chưa bị match tiêu thụ**:

```
drop_outcome(o):                      // glue MỚI, JIT sinh khi Drop(local: Outcome{T,E})
    switch o.disc:
      Positive(1) →  drop_as<T>(o.payload)   // nếu T heap: free; nếu T scalar: no-op
      Negative(-1) → drop_as<E>(o.payload)   // nếu E heap: free; nếu E scalar: no-op
      Zero(0) →      no-op                    // (T?~E null state — không có payload)
```

- Đây là `SwitchInt`/`If` trên disc ngay trong drop glue — **Drop hết type-static, thành disc-dynamic.**
- **Khi match BIND payload** (`~- e => use(e)`): payload **move** vào local `e` → `e` thành owner →
  `e` drop ở cuối scope của nó. Outcome `o` phải **`Deinit`** (tombstone) NGAY sau khi move, để
  `drop_outcome(o)` cuối scope KHÔNG free lại (double-free). Đây đúng pattern Deinit-sau-move của
  ADR-0042 Q1, mở rộng cho payload union.
- **Arm KHÔNG được chọn:** không có gì để dọn — nhánh đó không chạy runtime, và union chưa từng giữ
  giá trị của arm kia.

> **Răng (teeth O):** match `~- e` bind String, KHÔNG Deinit `o` → `__triet_string_free` gọi 2 lần
> (e drop + o drop glue) → double-free → SIGABRT. Test phải đỏ nếu thiếu Deinit.

### 3.2. Q2 — Map overwrite: `~->` tạo String lỗi mới, String cũ drop ở đâu?

`o ~-> |e| body` (map error). Trong neg_bb (error arm) khung CFG-merge có sẵn (APP.2c):

```
neg_bb:
    e := o.payload           // bind String lỗi CŨ (move-out khỏi o.payload)
    new := eval(body)        // body có thể tiêu thụ e (concat) HOẶC bỏ rơi e (~-> |e| "const")
    result.disc := Neg
    result.payload := new    // String lỗi MỚI vào result
    Deinit(o)                // o.payload đã move-out → tombstone
    Goto merge
```

Hai trường hợp `e`:
- **body tiêu thụ `e`** (vd `e + " (retry failed)"`): borrowck thấy `e` move vào concat → `e`
  không còn owned → không Drop. String cũ thành một phần String mới. OK.
- **body KHÔNG tiêu thụ `e`** (vd `~-> |e| "fixed message"`): `e` thành **orphan** owned local →
  cơ chế **Drop-on-scope-pop có sẵn** (lower.rs:214) sinh `Drop(e)` ở cuối neg_bb scope → free
  String cũ. KHÔNG rò.

pos_bb (success passthrough): copy T payload từ `o` sang `result` = **move** heap (không deep-copy
ptr) → `Deinit(o)` sau copy.

> **Điểm soundness gắt nhất:** `result` và `o` **không được cùng nghĩ mình sở hữu một ptr**. Sau
> mỗi arm copy/move payload sang `result`, BẮT BUỘC `Deinit(o)`. Thiếu → cả `o` (drop glue) và
> `result` (drop glue / return) đều free cùng ptr → double-free.
>
> **Răng:** `~-> |e| "const"` (bỏ rơi e) — poison gỡ `Drop(e)` scope-pop → leak (valgrind/leak
> shim đỏ). Poison gỡ `Deinit(o)` → double-free SIGABRT.

### 3.3. Q3 — Layout: StackSlot phình bao nhiêu?

**Đính chính:** G hỏi "24 hay 32 byte để chứa disc + fat-pointer 16-byte". Nhưng heap value Triết
là **24-byte `{ptr,len,cap}`**, không phải 16. Free shim cần cả `cap` → cap PHẢI đi cùng trong slot.

```
Outcome<T,E> slot = disc(8) + payload_union(max(sizeof(T), sizeof(E)))
  - payload scalar (Integer/Trit/…) = 8  → slot 16 byte (như hiện tại, Bậc A)
  - payload heap (String/Vector/HashMap) = 24 {ptr,len,cap}  → slot = 8 + 24 = 32 byte
    layout: {disc@0, ptr@8, len@16, cap@24}
```

**Kết luận Q3: 32 byte** (disc@0, payload heap @8..32), không phải 24. Lý do thẳng: heap value Triết
24-byte. Slot size = `8 + max(payload sizes)`, alignment 8.

**Lựa chọn thu nhỏ (đề xuất DEFER, ghi để G biết, KHÔNG làm trong lát này):**
- C4 Packed (ADR-0052 §6): bit-pack disc vào trit thừa của ptr (ptr align 8 → 3 bit thấp = 0) →
  bỏ ô disc 8-byte → slot 24-byte. **Tối ưu sau, không phải bây giờ** — YAGNI cho tới khi 32-byte chạy.

---

## 4. Decision (đã chốt theo phán quyết G §8 — chờ chữ ký LOCKED)

1. **Outcome slot size động theo payload union:** scalar → 16-byte (giữ nguyên Bậc A); heap → **32-byte**
   `{disc@0, ptr@8, len@16, cap@24}`. Lowerer tính từ `max(sizeof(T), sizeof(E))`. **(G CHỐT 32-byte —
   YAGNI, KHÔNG ép Packed; tối ưu cache-line là việc tương lai khi cấu trúc đã vững.)**
2. **Drop glue disc-dynamic, INLINE trong MIR CFG — KHÔNG shim:** `Statement::Drop(local: Outcome{heap})`
   lower thành cụm `SwitchInt(disc) → {Pos: free-as-T, Neg: free-as-E, Zero: no-op}` **chèn thẳng vào
   đồ thị MIR/Cranelift**, KHÔNG bọc `__triet_outcome_drop`. Outcome scalar → Drop no-op (như nay).
   **(G CHỐT inline — shim che mắt borrowck + cản optimizer; inline phơi mọi di biến ownership trên CFG,
   double-free rà CFG tóm ngay. Đánh đổi: MIR phình — chấp nhận vì minh bạch tuyệt đối.)**
3. **`Deinit(o)` sau mọi move-out payload — ngữ nghĩa CỤ THỂ cho Outcome:**
   `Statement::Deinit(local)` hiện = "ghi 0 tombstone" (`mir.rs:209-214`). Với Outcome StackSlot,
   **`Deinit(o)` ⟺ `stack_store(Zero(0))` vào ô disc (offset 0)**. Vì drop glue (điểm 2) no-op khi
   disc=Zero, ghi disc:=0 làm glue của `o` thành no-op → chống double-free sau khi payload đã move ra.
   - **KHÔNG đụng ptr/len/cap:** chúng để stale là vô hại — glue thấy Zero, không bao giờ đọc ptr.
   - **Tái dùng Zero=no-op:** binary `T~E` cấm Zero ở mức user (E1025), nhưng tombstone disc=0 là
     sentinel NỘI BỘ post-move, KHÔNG quan sát được bởi `match` (giá trị đã move ra, borrowck cấm
     match lại `o` đã moved). Không xung đột E1025.
   - **Thứ tự BẮT BUỘC:** `Deinit(o)` đặt SAU mọi đọc payload của `o`, NGAY khi payload chuyển sở hữu
     (match-bind, `~+>`/`~->` copy-to-result, passthrough). Cùng giao thức ADR-0042 Q1, mở rộng cho union.
4. **Borrowck:** payload move-out đánh dấu `o` moved (đã có move-tracking M3/M3+); drop glue của `o`
   bị bỏ qua nếu `o` moved/Deinit'd. `~+>`/`~->` chain: result owns, inner Deinit'd — borrowck phải
   thấy đúng chuỗi sở hữu để không báo use-after-move sai VÀ không bỏ sót double-free.
5. **`is_scalar` guard gỡ cho heap khi payload là heap-type hợp lệ** (String/Vector/HashMap) — E1037
   chỉ còn chặn type thực sự không drop được (struct/enum nested chưa có drop glue — giữ defer).

## 5. Phân lát đề xuất (mỗi lát gate xanh; D làm sau khi ADR ký)
- **HP.0 Spike borrowck (O/probe, KHÔNG Production):** đội do thám thử ranh giới move-tracking M3+ trên
  chuỗi `~+>`/`~->` heap (temporaries qua map liên hoàn). Mục tiêu: xác định move-tracking hiện đủ hay
  cần loan-tracking mới — TRƯỚC khi D viết HP.4. Throwaway, không ship.
- **HP.1 Layout + Producer:** slot 32-byte heap, `~+ str`/`~- str` store {ptr,len,cap}. Check-mode
  fixture (tới MIR verify). Chưa drop.
- **HP.2 Drop glue disc-dynamic:** SwitchInt drop glue, free đúng arm. RUN fixture: Outcome<_,String>
  rời scope không match → đúng 1 free (leak shim = 0, double-free = 0).
- **HP.3 Match consumer + Deinit:** bind heap payload, Deinit o, e drop ở scope. RUN.
- **HP.4 Map `~+>`/`~->` heap:** passthrough move + Deinit; map tiêu thụ/bỏ rơi e đúng drop.

## 6. Teeth dự kiến (O sẽ áp — KHÔNG đưa D blueprint, chỉ nêu bất biến)
- Double-free: bind/move payload mà thiếu `Deinit(o)` → SIGABRT.
- Leak: `~-> |e| "const"` bỏ rơi e mà thiếu `Drop(e)` scope-pop → leak-count > 0.
- Drop-wrong-arm: drop glue free-as-T khi disc=Neg (đọc nhầm/bỏ SwitchInt) → free sai layout / SIGABRT.
- Layout: cap@24 đọc nhầm offset → free(ptr, rác) → crash.
- Scalar regression: Outcome<Integer,Integer> drop glue phải VẪN no-op (không sinh free cho scalar).

## 7. Consequences
- **Tích cực:** Outcome dùng được thật (`~- "msg"`); mở khóa nút thắt lớn nhất của error-handling.
- **Đắt:** Drop hết type-static → một class drop glue disc-dynamic mới (đụng JIT + borrowck + verifier).
- **ABI:** hàm `-> String~String` trả Outcome 32-byte — multi-return/sret ABI phải khớp (nối ADR-0052 §3.3).
- **Defer (giữ phong ấn):** C4 Packed (24-byte tối ưu) · nested struct/enum payload (chưa drop glue) ·
  TernaryOutcome heap (sau khi binary heap chạy + Mũi A ternary scalar của D xong).

---

## 8. Phán quyết G (CHỐT 2026-06-10 — đóng §8)
1. **Layout: CHỐT 32-byte.** KHÔNG ép Packed. Premature optimization là cội nguồn thảm họa — đang
   chọc vào Drop disc-dynamic nhạy cảm với rủi ro leak lơ lửng, đừng xáo offset để vắt 8 byte. Packed
   (nhét disc vào pointer-tag/padding) là việc tương lai khi cấu trúc đã vững + cần ép cache-line.
   **YAGNI triệt để — móng 32-byte cho an toàn.**
2. **Drop glue: CHỐT INLINE trong MIR CFG.** Shim `__triet_outcome_drop` che mắt borrowck + làm khó
   optimizer. Chèn thẳng `SwitchInt(disc) → free_T/free_E` vào đồ thị MIR. MIR phình một chút, đổi lại
   **minh bạch tuyệt đối** — mọi di biến ownership phơi trên CFG, double-free rà CFG tóm ngay.
3. **Borrowck chain `~+>`/`~->`: CHẤP THUẬN SPIKE PROBE** trước Production. Fat-pointer qua chuỗi map
   liên hoàn đẻ hàng loạt temporaries; borrowck tracking hụt → lifetime temp kết thúc sớm (use-after-free)
   hoặc sống dai (leak). Tung đội do thám thử ranh giới borrowck TRƯỚC khi D đụng tay Production (HP.4).

**§8 đóng. ADR-0053 sẵn sàng cho G ký duyệt chặng cuối.**
