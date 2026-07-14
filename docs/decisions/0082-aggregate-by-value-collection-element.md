# ADR 0082 — Aggregate by-value làm element của Collection (Struct/Enum trong Vector/HashMap, KHÔNG native-packing)

> # 🩸 NGUYÊN LÝ CỐT LÕI (O đề, chờ G khắc đá)
> # `Vector<User>` phải chạy. NHƯNG cái giá KHÔNG được là xé nát bất biến
> # **"mọi value = một i64" (8B-granular)** — mỏ neo duy nhất giữ JIT còn giải được.
> # Tử huyệt KHÔNG phải kích thước — mà là **DROP-GLUE ĐỆ QUY**: drop một
> # `Vector<User>` (User chứa `String`) mà free chay memory element = **LEAK**;
> # byte-copy element ptr rồi drop hai lần = **DOUBLE-FREE**. ADR này khóa cái
> # bất biến bảo thủ + thiết kế cỗ máy free đệ quy, và **đạp chết sub-8B packing**
> # (B-β) khỏi phiên này.

**Trạng thái:** 📝 **DRAFT — chờ G đọc + ký. CHƯA một dòng code nào được viết.** Áp dụng Bậc C+.
Mở `Vector<UserStruct>` / `Vector<Enum>` và `HashMap<K, UserStruct>` / `HashMap<K, Enum>`
(value-side) by-value. Đây đúng là **P2** mà ADR-0077/0078 hứa và REFUSE ở biên P1.

**Scope đã chốt (G duyệt 2026-07-08):** = **B-α** (aggregate by-value element).
- ✅ IN: Struct/Enum làm element Vector, làm VALUE của HashMap.
- ⛔ OUT — **B-β sub-8B packing** (Trit=1B…): ĐẠP CHẾT. Giữ nguyên 8B-granular. Value-model i64 bất khả xâm phạm.
- ⛔ OUT — **B-γ multi-register struct return**: defer vô thời hạn.
- ⛔ OUT — Struct/Enum làm **KEY** của HashMap: đòi hash+eq đệ quy trên aggregate → campaign RIÊNG.
- ⛔ OUT — `get()` **by-value** một aggregate element: REFUSE (như String, ADR-0077) — lấy ra bằng `pop`/`remove` (move-out) hoặc mượn bằng `get_ref` (ADR-0079).

**Sibling/kế thừa:** ADR-0066/0067 (No-Box heap-in-aggregate — `collect_heap_leaves`,
drop-glue đệ quy, `LeafKind`), ADR-0076 (heap-`T?` sentinel-no-op R4), ADR-0077 (Typed Vector P1 —
fat-element ABI stride>8 by-pointer, element-free loop), ADR-0078/0080 (Typed HashMap value/key —
`emit_hashmap_free_value`), ADR-0079 (get-borrow — `get_ref` stride-conditional).
**KHÔNG đụng:** ADR-0068 (Box/recursive — CẤM CỬA), native multi-field layout thật (B-β — defer),
ADR-0081 (get-borrow-mutable — FROZEN, đòi deref-assign).

---

## Issue

ADR-0077/0078 mở collection cho **built-in element** (element-size HẰNG compile-time:
scalar/handle=8B, String=24B). Aggregate by-value bị KHÓA MÕM ở đúng một điểm:

- **`vector_elem_size` REFUSE Struct/Enum** — `mir_lower.rs:524-531`:
  `Struct(_) | Enum(_) | Capability(_) | Outcome{..} → Err(JitError::Unsupported("… by-value
  aggregate elements need native-layout, deferred to P2"))`. Đây là biên P1/P2 duy nhất.

Hệ quả: `Vector<Point>`, `HashMap<String, User>` không compile. Ngôn ngữ có struct, có collection,
nhưng KHÔNG bỏ được struct vào collection — đúng cái "vứt đi" mà ADR-0077 nguyên lý cốt lõi lên án,
một tầng sâu hơn.

**Cái bẫy phải né:** cái tên "native multi-field layout" dụ ta gói field sub-8B (Trit=1B) cho
"đúng chuẩn C". Đó là **B-β** — nó phá thẳng bất biến value-model i64 (JIT load/store mọi field
bằng `stack_load(I64, slot, off)`, `mir_lower.rs:633-770`), ép typed load/store I8/I16/I32 + ext ở
MỌI field site, đổi lấy vài byte mật độ **không ai xin**. ADR này KHÔNG làm B-β.

---

## Quyết định

Mở **Aggregate-by-value collection element (B-α)** qua đúng **một mở rộng có kiểm soát** của cỗ máy
đã có, dưới **một bất biến khóa cứng**.

### §1 — BẤT BIẾN NỀN (khóa cứng, đây là "định nghĩa byte-image" G yêu cầu)

> **INV-B-α: Một layout, hai nhà, byte-identical.**
> Byte-image của một struct/enum trong **cell của collection** = byte-image của nó trong
> **StackSlot** — CÙNG `StructLayout`/`EnumLayout` (cùng field-offset, cùng 8B-granular size,
> cùng heap-leaf repr: String=24B fat {ptr@0,len@8,cap@16}, Vector/HashMap=8B handle). KHÔNG
> có layout thứ hai. KHÔNG gói sub-8B. `stride = total_size` từ `struct_layouts`/`enum_layouts`.

**Vì sao INV-B-α là load-bearing:** drop-glue đệ quy tính field-offset từ `struct_layouts`
(`collect_heap_leaves`, `mir_lower.rs:433`). Nếu image trong cell KHÁC image trên stack (vd ai đó
sau này pack lại để tiết kiệm), drop walk đọc sai offset → free ptr rác → SIGSEGV/double-free.
Một layout duy nhất = drop walk luôn đúng. Đây là lý do **giữ 8B-granular là sự SỐNG CÒN**, không
phải lười: nó giữ image trên stack (nơi field được `stack_store(I64)`) và image trong cell (nơi
drop walk đọc) **cùng một thứ, miễn phí**.

### §2 — Marshal side (nhập/xuất cell): CƯỠI NGUYÊN fat-element ABI, KHÔNG việc mới

ADR-0077 fat-element ABI đã generic theo `stride`, KHÔNG special-case String:
- **push** (`mir_lower.rs:3027-3059`): `stride > 8` → truyền `stack_addr` của element slot →
  shim `copy_nonoverlapping(elem, dst, stride)` (`4171`). Struct element **nằm sẵn trong
  `struct_slots`** → route by-pointer + memcpy tự động. Chỉ cần `vector_elem_size` trả `total_size`.
- **pop** (`4282`) / **hashmap_remove** (fat): `stride > 8` → memcpy ra `out_ptr` (dest slot), sret.
- **insert** (HashMap value fat, `3060+`): `value_stride > 8` → by-pointer, cùng đường.

⇒ Marshal side của B-α ≈ **thay đúng một hàm** (`vector_elem_size` trả size cho Struct/Enum).
Push/pop/insert/remove tổng quát hóa verbatim.

### §3 — KHÔNG mở mặt trận double-free mới (bằng chứng: MOVE byte-wise)

`__triet_vector_push` là **functional-MOVE, không clone sâu** (`4163-4177`):
`copy_nonoverlapping(old_data, new_data, old_len*stride)` dời element byte-exact sang buffer mới,
rồi `__triet_vector_free(vec)` free **CHỈ buffer cũ** (comment `4174`: "freeing elements here would
double-free"). Element heap-ptr (kể cả String bên trong struct) được **dời byte-exact, không nhân
đôi** → free đúng MỘT lần ở `Drop(v_new)`. Caller M3-zero `v_old` sau call → `Drop(v_old)` no-op.
**Chuỗi này generalize verbatim cho struct element** — String-trong-struct cưỡi nguyên, không thêm
nợ double-free. (Đây là điểm O đã verify tại `4166/4176` — không phải suy đoán.)

### §4 — Drop side (TỬ HUYỆT SOUNDNESS): Recursive Drop-Glue

Đây là phần G khắc: *"gọi đệ quy drop-glue của Struct để dọn String leaf bên trong."* Cỗ máy đã
tồn tại cho **stack struct**; B-α = tổng quát hóa nó về **address-based** để chạy trên element-cell.

**Đã có sẵn (tái dùng):**
- `collect_heap_leaves(name, base_off, body, depth, out)` (`433`) — descent đệ quy, trả flat
  `Vec<(offset, LeafKind)>`. Recurse nested struct, đẩy enum thành `LeafKind::Enum` (tag-switch
  runtime), heap-`T?` thành `LeafKind::Heap` (sentinel-no-op R4). DAG-terminating, depth-limit 64
  (`440` → ADR-0068 net). **Copy struct → trả rỗng.**
- `emit_enum_drop_glue_at(builder, body, enum_name, base_addr)` (`1457`) — address-based, đã dùng
  cho enum-trong-struct. Mẫu để nhân bản cho struct.
- `emit_heap_free_at(builder, addr, ty)` (`972`) — free một leaf (String: ptr@0/cap@16;
  Vector/HashMap: đệ quy element loop).
- `emit_vector_element_free_loop` (`1054`) / `emit_hashmap_free_value` (`1129`) — vòng free
  element, gọi `emit_heap_free_at` per element.

**Việc mới (đúng 3 mối nối):**

1. **Trích `emit_struct_drop_glue_at(builder, body, struct_name, base_addr)`** — nhân bản
   `emit_enum_drop_glue_at`, thân = walk `collect_heap_leaves` (hiện đang **inline** ở 3 site:
   `1952`, `2341`, `2481`), mỗi leaf:
   - `LeafKind::Heap(ty)` → `emit_heap_free_at(base_addr + off, ty)`
   - `LeafKind::Enum(name)` → `emit_enum_drop_glue_at(base_addr + off, name)`
   (Refactor 3 site inline → gọi helper: **tùy chọn, giảm rủi ro trùng lặp** — KHÔNG bắt buộc cho
   B-α, giữ surgical nếu G muốn.)

2. **`emit_heap_free_at` thêm nhánh Struct/Enum** (`972`, hiện early-return `!is_any_heap()` ở `978`):
   - `MirType::Struct(name)` → `emit_struct_drop_glue_at(addr, name)`
   - `MirType::Enum(name)` → `emit_enum_drop_glue_at(addr, name)`
   Sau đó element-free loop (`1102` gọi `emit_heap_free_at(elem_addr, eff)`) **tự đệ quy** cho
   struct element — không đụng vòng loop.

3. **`vector_elem_size` (`509`) trả `total_size`** cho Struct/Enum (thay `Err`) — lấy từ
   `struct_layouts`/`enum_layouts`. **Đổi chữ ký:** hiện `fn(ty)` static; cần `body` để tra layout
   → `fn(body, ty)` hoặc method. Ripple tới MỌI call-site stride (push/pop/insert/remove/free —
   `2873/2885/3001/3017/…`). Mechanical nhưng RỘNG → phải nằm trong touch-list WO.

### §5 — Guard: Copy-struct fast-path vs heap-bearing struct

Vòng element-free (`1062`) hiện guard `if !eff.is_any_heap() return` — **`Struct` KHÔNG
`is_any_heap()` → struct element bị SKIP → String leaf không bao giờ free → LEAK.** Guard phải đổi
thành: *cần loop iff element là heap **HOẶC** struct/enum-có-heap-leaf*. Predicate:
`aggregate_needs_drop(body, ty)` = `!collect_heap_leaves(name).is_empty()` (struct) /
enum có heap variant. **Copy struct (rỗng leaf) → vẫn no-op → byte-compat** với `Vector<Point>`
toàn scalar (KHÔNG loop, KHÔNG khai báo `__triet_string_free`).

### §6 — Read-side biên (khóa, KHÔNG code mới)

| op | Copy aggregate element | Heap-bearing aggregate element |
|---|---|---|
| `get(v,i)` by-value | ⚠️ defer/refuse (element-copy = shallow-copy heap-ptr → double-free) — REFUSE như String | ❌ REFUSE E-code |
| `get_ref(v,i)` (ADR-0079) | ✅ trả cell_ptr (stride>8 → `&0 Struct`, `4254`) | ✅ trả cell_ptr |
| `pop`/`remove` | ✅ move-out | ✅ move-out |
| `push`/`insert` | ✅ by-ptr memcpy | ✅ by-ptr memcpy |

`get` by-value một aggregate là **REFUSE** (kể cả Copy struct — nhất quán + tránh mở nhánh
shallow-copy). Đọc = `get_ref` (mượn) hoặc `pop`/`remove` (move-out). `get_ref` stride>8 đã trả
cell_ptr (`4254`) → `&0 Struct` chạy sẵn từ ADR-0079 §AMEND-1.

---

## Death points (mỗi cái kèm TÍN HIỆU LỖI — feedback_failure_mode_precision)

| # | Lỗ | Nếu thủng → tín hiệu | Chốt chặn |
|---|---|---|---|
| **DP-1** | element-free loop guard `is_any_heap()` skip struct | **LEAK câm** (`FREE==0`, không signal) | §5 predicate `aggregate_needs_drop` |
| **DP-2** | `emit_heap_free_at` early-return trên non-heap Struct | **LEAK câm** | §4.2 nhánh Struct/Enum |
| **DP-3** | `vector_elem_size` mis-size (trả 8 thay total_size) | stride sai → memcpy stomp field kế / drop đọc disc rác → **SIGSEGV 139** | §4.3 `total_size` từ layout + INV-B-α |
| **DP-4** | double-drop khi String-leaf-ptr bị nhân đôi (nếu push clone nông) | **SIGABRT 134** (double-free) | §3: push MOVE byte-wise, free buffer-only (`4176`) — verify giữ nguyên |
| **DP-5** | Copy-struct đi vào loop thừa / khai `__triet_string_free` phá byte-compat caller | fixture Copy-struct hiện có → **RED bất ngờ** | §5: rỗng-leaf → no-op |
| **DP-6** | nested `Vector<Vector<User>>` / `User{Vector<String>}` không đệ quy hết tầng | LEAK tầng trong | `collect_heap_leaves` + `emit_heap_free_at` Vector-branch (`987`) đã đệ quy; DAG depth-64 net (`440`) |
| **DP-7** | image trong cell ≠ image stack (INV-B-α vỡ) | drop walk đọc sai offset → **SIGSEGV/134** | §1 khóa một-layout; marshal = memcpy verbatim `total_size` |

---

## Slicing (đề xuất — G chốt)

- **Slice A — Vector<Struct>:** §4.3 vector_elem_size + §5 guard + §4.1 `emit_struct_drop_glue_at`
  + §4.2 nhánh Struct trong `emit_heap_free_at`. Teeth O: push N struct-có-String → drop →
  `FREE == N*(#String-leaf)` + buffer; pop → drop ownership sạch; Copy-struct → byte-compat.
- **Slice B — Vector<Enum>:** §4.2 nhánh Enum (tái dùng `emit_enum_drop_glue_at` verbatim). Teeth:
  enum element heap-variant vs Copy-variant, tag-switch free đúng arm.
- **Slice C — HashMap<K, Struct/Enum> value:** `emit_hashmap_free_value` value-loop cưỡi cùng
  `emit_heap_free_at` mở rộng → gần như tự chạy sau A/B. Teeth: insert/remove/drop value aggregate.
- **Slice D (nếu G duyệt) — refactor 3 site inline (`1952/2341/2481`) → gọi
  `emit_struct_drop_glue_at`:** giảm nợ trùng lặp; hoặc để nguyên (surgical).

Struct KEY, get-by-value aggregate, B-β, B-γ = **NGOÀI**, refuse-over-guess.

---

## Teeth (kế hoạch O verify máu — cp-snapshot, KHÔNG git checkout)

1. **T-LEAK (DP-1/2):** `Vector<User>` (User{name:String}) push 3 → drop; count FREE. Gỡ §5
   guard-fix → `FREE == 0` (leak) MỚI đúng poison đỏ; giữ fix → `FREE == 3`.
2. **T-DOUBLE (DP-4):** push→pop 1→drop; FREE == đúng số, **KHÔNG 134**. Cắm giả clone-nông →
   phải 134.
3. **T-STRIDE (DP-3):** `Struct{a:Integer, s:String, b:Integer}` (total 40B) push→get_ref field
   `b`; sai stride → đọc rác. Control-biến: đổi total_size hardcode → RED.
4. **T-COPY (DP-5):** `Vector<Point>` (Point{x,y:Integer}) push→drop → 0 String-free, byte-compat.
5. **T-NEST (DP-6):** `Vector<User>` với User{tags: Vector<String>} → drop → free đủ 2 tầng.
6. **T-ENUM (Slice B):** enum {A(String), B} vector → free đúng arm theo disc.
7. **T-REFUSE:** `get(v,i)` by-value aggregate → E-code; struct KEY → E-code. KHÔNG silent/panic.

**Mỗi teeth: gỡ guard → poison ĐỎ độc lập; restore md5 khớp. Tree đóng băng khi chấm.**

---

## Consequences

**Được:** `Vector<UserStruct>`, `HashMap<K,UserStruct>` chạy — collection thật sự tổng quát trên
type người dùng. Value-model i64 nguyên vẹn. Cỗ máy drop-glue đệ quy (`collect_heap_leaves`) lần
đầu chạy trên heap-cell thay vì chỉ stack-slot — nhưng **cùng một layout, cùng một walk**.

**Giá:** `vector_elem_size` đổi chữ ký (ripple rộng, mechanical). Một helper mới
(`emit_struct_drop_glue_at`). Guard element-loop phức tạp thêm một predicate.

**KHÔNG được (khóa):** sub-8B packing (B-β), multi-reg return (B-γ), aggregate KEY, get-by-value
aggregate. Mọi thứ ngoài scope → REFUSE bằng E-code hiện có hoặc mới, KHÔNG skeleton, KHÔNG panic.

**Nợ mở sau:** nếu tương lai cần mật độ bộ nhớ (B-β) hoặc perf return (B-γ), ADR RIÊNG, và phải
chứng minh trước rằng nó KHÔNG vỡ INV-B-α (một-layout) — vì lúc đó image stack và image cell buộc
phải rã đôi.

---

**Chữ ký:** O đề (2026-07-08). Chờ G đọc + ký. Author chốt hướng scope (B-α, đã duyệt).
CHƯA code cho tới khi G ký.
\n**G Ký (2026-07-08):** DUYỆT. Thiết kế tàn nhẫn, giữ vững được mỏ neo 8B-granular (INV-B-α) và vạch mặt DP-1 leak/DP-4 double-free chính xác. Tiến hành Slice A.

---

## §AMEND-1 — 2 lỗ ngoài touch-list, D phát hiện ở T0 probe (O rule, sau chữ ký G)

D probe `Vector<User>` (User{name:String}) ở bước T0 → lộ **2 thứ ngoài 6 touch-point WO**, một trong đó **cắt thẳng vào §3**. Ghi lại trung thực — không vá lặng.

### AMEND-1.1 — 🩸 §3 CÓ LỖ: MOVE byte-wise generalize verbatim Ở RUNTIME, nhưng M3 zero-guard compile-time thì KHÔNG

§3 gốc kết luận "chuỗi MOVE byte-wise generalize verbatim cho struct element" dựa trên **shim runtime** (`__triet_vector_push` dời byte + free buffer-only, `4166/4176`). Đúng — nhưng **thiếu một tầng**: M3 Zeroing-on-Move **compile-time** (`mir_lower.rs:3436-3443`) khi tombstone consume-arg chỉ special-case **đúng một type** (`layout.name == "String"`); struct-slot-backed local rơi vào `def_var(var, zero)` — zero **Variable**, KHÔNG zero **slot leaves**. Nhưng `Drop(struct_local)` đọc **SLOT** (qua `collect_heap_leaves` + `copy_base_addr`), không đọc Variable → slot còn String ptr gốc → **free lần 2** (lần 1 ở element-free-loop của `Drop(v)` sau khi byte-move). **`Vector<User>` → double-free 134.**

**Nguyên nhân gốc:** M3-tombstone và Drop-glue là **cặp song sinh** G đã mandate đối xứng ("free N tiers → zero N tiers"). Site tombstone-on-let-move (`1938-1968`) ĐÃ generalize đúng (walk `collect_heap_leaves`, zero mỗi leaf). Site tombstone-on-arg-consume (`3436`) thì CHƯA — String-only. Trước Slice A, đường struct-consume-arg chưa từng chạy (push struct bị refuse ở `vector_elem_size`) → **latent, Slice A là caller đầu tiên phơi ra**.

**RULING (O):** BLOCKING, vá TRONG Slice A (double-free nằm trên critical path — `Vector<User>` không thể ship kèm nó). Thêm **T7** vào WO: generalize guard `3436` thành struct-slot tombstone đối xứng với `1938` (dùng chung `collect_heap_leaves` walk — lý tưởng là trích helper `tombstone_slot_leaves` gọi từ CẢ `1938` lẫn `3436`, giữ cặp song sinh không rã). **Commit TÁCH** (pre-existing latent gap, luật ①b + tiền lệ WO-0075 C1 fixpoint-hole).

### AMEND-1.2 — ⚠️ `vector_elem_size` dùng CHUNG Vector + HashMap → T2 rò scope sang HashMap<K,Struct>

`vector_elem_size` phục vụ cả Vector-element LẪN HashMap-key/value (8 call-site T2 gồm 4 site HashMap). Mở Struct arm → `HashMap<Integer, User>` **source-reachable ngay** (D probe: typecheck+borrowck exit 0), NHƯNG T5 chỉ vá guard vector-loop (`1062`) — guard hashmap value-loop (`emit_hashmap_value_free_loop:1286`) VẪN `!eff.is_any_heap()` → **skip struct value → LEAK câm** khi Drop map. **Đúng hình dạng P0 latent của ADR-0080** (compile được, runtime sai lặng, 0 fixture bắt).

**RULING (O):** GIỮ biên G đã chốt — HashMap-value = Slice C, **KHÔNG mở ở A**. Thêm **T8**: guard REFUSE tường minh tại HashMap marshal/op emit-sites — key hoặc value là `Struct`/`Enum` → `Err(JitError::Unsupported("HashMap<_,aggregate> = ADR-0082 Slice C, chưa mở"))`. **Không silent, không leak.** `vector_elem_size` Struct arm giữ nguyên (tính size là đúng); chặn ở tầng HashMap-op. Slice C sau này gỡ T8-guard + vá `1286` + thêm teeth HashMap. Guard chỉ bắn trên Struct/Enum key/value → HashMap<Integer/String,scalar/String> hiện có KHÔNG ảnh hưởng.

**Bài học:** verify-cuts-my-own-ADR lần nữa — §3 tôi verify tầng shim mà bỏ tầng M3 compile-time; và một hàm size dùng-chung âm thầm mở hai mặt trận. Cả hai D bắt đúng khi refuse-tự-quyết, dừng-hỏi-O (luật ④).

## §AMEND-2 — Value move-out aggregate (Vector pop / HashMap remove by-value) — campaign D-1+D-2

**Bối cảnh:** ADR gốc §2–§4 + §AMEND-1 phủ **push+drop** (aggregate VÀO collection; Slice A/B/C). Chiều XUẤT — element ra khỏi collection **by-value** — bị REFUSE/defer suốt A/B/C (Slice B `98a3be2` AM1 refuse `__triet_vector_pop` Struct/Enum). §AMEND-2 khép chiều xuất: `Vector<T>` pop (D-1, `03a7638`+`f2e8bd8`) + `HashMap<K,V>` remove (D-2, `5644f6e`) trả aggregate by-value. Continuation B-α, **KHÔNG ADR-nền mới**.

### AMEND-2.1 — ① MOVE-OUT TOMBSTONE CONTRACT (load-bearing, bắt buộc)

Move-out = element rời quyền sở hữu container sang dest-local. Chống double-free (container-drop + dest-drop cùng free một leaf) đòi **source-tombstone BẮT BUỘC**, cơ chế theo container:

- **Vector pop — `len--`** (`__triet_vector_pop`). Cell đã pop không zero; `len--` loại nó khỏi tập drop (element-free-loop lặp `i < len`). **Load-bearing:** O poison bỏ `len--` → popped cell double-free (FREE 3).
- **HashMap remove — `state→2`** (shim) + value-free-loop gate `state==1` (`emit_hashmap_value_free_loop`). Cell value KHÔNG zero (xem ③); state=2 khiến map-drop bỏ qua. **Load-bearing CẢ HAI ĐẦU** (G-MANDATE): GATE-A (nới gate `state≥1`) → SIGSEGV · GATE-B (bỏ `state→2`) → double-free tcache SIGABRT.

**Quy định:** mọi move-out by-value tương lai (get-by-value, pop-front, drain) PHẢI có source-tombstone + teeth poison chứng minh load-bearing. **Deferred-KHÔNG-refuse = UB câm** (án lệ Slice A pop, xem ②).

### AMEND-2.2 — ② SỰ THẬT VỀ SLICE-A-BUG-1 (lịch sử không bóp méo)

Slice B AM1 refuse `__triet_vector_pop` Struct/Enum, comment gốc: *"needs recursive move-out tombstone… deferred"* — hàm ý move-out chưa sound vì **thiếu tombstone**. **SỰ THẬT (D-1b phơi ra):** refuse đó KHÔNG che bug tombstone bất-khả-sửa. Nó che **một tầng ABI CHƯA DỰNG:** pop-dest LUÔN `Nullable(Struct)` (`triet-lower/lib.rs:2460`), slot tag-prepend (ADR-0076 Phương án A, `tag@0/fields@+8`, `mir_lower.rs:1906`). Marshal cũ ghi fields@+0 → **đè tag word** → field-access (+8) đọc rác, drop-glue đọc tag rác → free bậy. Đây CHÍNH là "Slice A original pop double-free/invalid-pointer". `len--` source-tombstone chưa từng là vấn đề (miễn phí từ ADR-0077). **D-1b dựng tầng marshal:** out_ptr=`slot+8` (non-String), tag=`(shim_ret==NULL_SENTINEL)?SENTINEL:1`@`slot+0`.

**Bài học hiến pháp:** "deferred vì chưa sound" ≠ "deferred vì chưa dựng ABI". Refuse-tạm phải nói ĐÚNG lý do — sai nhãn chôn một khối việc khả-thi dưới mác bất-khả.

### AMEND-2.3 — ③ QUYẾT ĐỊNH STATE-GATE: KHÔNG zero value-cell (đánh đổi hiệu năng, có máu bảo chứng)

HashMap remove move value ra out_ptr NHƯNG **không** `write_bytes(vptr,0,value_stride)` (khác KEY path vốn zero — ADR-0080 §AMEND-1 ★SS(c)). An toàn phó thác DUY NHẤT cho `state→2` + gate `state==1`. G-MANDATE đòi chứng minh gate đủ chặt TRƯỚC khi chấp nhận bỏ zeroing.

**Kết quả (O poison độc lập, cp-snapshot restore md5 `267f1cbb`, baseline XANH trước mỗi phát):** GATE-A đỏ (SIGSEGV) · GATE-B đỏ (double-free tcache SIGABRT). Cả hai load-bearing.

**QUYẾT ĐỊNH (G ký 2026-07-11):** GIỮ thiết kế KHÔNG zero value-cell — state-gate đủ vững chặn double-free, tiết kiệm một `write_bytes`/remove. **Điều kiện treo:** nếu tương lai thêm code đọc vùng tombstone (iter/rehash/compact chạm cell state=2) → phải (a) gate `state==1` chỗ đó, HOẶC (b) khi đó mới zero value-cell. Teeth GATE-A/GATE-B canh gác vĩnh viễn — ai nới gate phải thấy chúng đỏ.

### Teeth D-1+D-2 (poison-cemented, O verify độc lập)

**Vector:** `len--`→FREE3 · T9-enum→SIGILL · present-tag loop-reuse 341/342→(1→0) · field_off→corpus SIGABRT.
**HashMap:** GATE-A→SIGSEGV · GATE-B→double-free SIGABRT · field_off→corpus 343 SIGABRT · present-tag 345/346→(1→0).
**Marshal dùng chung:** dest-bind fat `mir_lower.rs` = `vector_pop_fat || hashmap_remove_fat` → 1 poison present-tag đỏ CẢ 4 fixture loop-reuse (341/342/345/346).

**Nợ còn treo (campaign riêng):** get-by-value aggregate (get_ref value-aggregate = Cụm D/ADR-0081 FROZEN) · key-aggregate hash+eq đệ quy · pop-front/drain · B-γ multi-reg return.

**O ký 2026-07-11.** **G ký 2026-07-11:** DUYỆT. Tombstone contract, bóc trần Slice-A-BUG-1, và quyết định đánh đổi state-gate được ghi nhận chính xác theo thực tế chiến trường. Value move-out aggregate KHÓA SỔ.

## §AMEND-3 — get-by-value aggregate, **CHỈ Copy** (đọc-và-sao-chép, KHÔNG move-out, KHÔNG deep-Clone)

**Status:** 🚧 O ĐỀ 2026-07-14 — **chờ G co-sign**. Scope Giang chốt 2026-07-14: **Copy-aggregate ONLY**.

**Bối cảnh:** `get(container, k)` hiện có: scalar V → by-value `V?` (env.rs:346/438/448); heap-scalar V (String/Vector/HashMap trực tiếp) → E1047 → dùng `get_ref` borrow (exprs.rs:1174). **Aggregate V (Struct/Enum, scalar key)** → rơi thẳng **E1041 NoMatchingOverload**, không đường nào, KHÔNG phân biệt Copy vs heap-bearing (`is_heap_element` exprs.rs:2270 trả `false` cho MỌI UserStruct/UserEnum). §AMEND-3 mở get-by-value cho aggregate **THUẦN Copy**; heap-bearing REFUSE tường minh. Continuation B-α, **KHÔNG ADR-nền mới**.

### AMEND-3.1 — ⚖ RANH GIỚI Copy-vs-Clone LÀ RANH GIỚI SOUNDNESS (điều kiện-chặn O tự đặt)

get-by-value **KHÔNG phải move-out** (§AMEND-2.1). Element **Ở LẠI** container; một bản copy trả ra dest-local. Non-destructive → **KHÔNG source-tombstone** (quy định AMEND-2.1:258 KHÔNG áp — không có chuyển-quyền-sở-hữu để chống double-free).

Điều đó **sound KHI VÀ CHỈ KHI** aggregate **không có heap leaf** (Copy):
- **Copy aggregate** (`!aggregate_needs_drop` — mọi leaf scalar): bản copy = **bitwise-identical, KHÔNG chia sẻ heap allocation** với element trong container. Cả hai drop độc lập → **KHÔNG double-free**. SOUND. Tiền lệ: Slice A T-COPY `Vector<Point>`→FREE==0.
- **Heap-bearing aggregate** (có ≥1 String/Vector/HashMap leaf): bitwise copy **alias con trỏ heap** → hai chủ một allocation → drop hai lần → **double-free**. Muốn phủ đòi **deep-Clone đệ quy** (alloc mới mỗi leaf) = năng lực MỚI, đụng move-only **ADR-0042** + "Clone CẤM TIỆT (hidden alloc=rác)" ADR-0079. **KHÔNG được ngầm trên `get`.**

**Quyết định:** Copy-aggregate → get-by-value SOUND (bitwise memcpy). Heap-bearing → **REFUSE tường minh**, trỏ về `get_ref` (borrow). Deep-Clone tách campaign-nền riêng sau ADR `.clone()` tường minh (Copy/Clone trait + carve-out ADR-0042). **KHÔNG chôn heap-bearing dưới E1041 mù** (bài học hiến pháp AMEND-2.2: refuse phải nói ĐÚNG lý do).

### AMEND-3.2 — predicate `is_copy_aggregate` (typecheck, mirror `MirType::is_copy`)

Typecheck cần predicate MỚI `Type::is_copy_aggregate` (`types.rs:227`, cạnh `is_hashable_key:165`): scalar (Trit/Tryte/Integer/Long/Trilean) → Copy; String/Vector/HashMap → KHÔNG-Copy; `UserStruct{fields}` → mọi field Copy đệ quy; `UserEnum{variants}` → mọi payload Copy đệ quy; `Nullable(inner)` → Copy iff `inner` Copy.

**⚠ ĐÍNH CHÍNH (O sai ở bản đầu, D phát hiện — LUẬT 5 flag):** bất biến "`is_copy_aggregate ≡ !aggregate_needs_drop`" **KHÔNG chính xác**. `aggregate_needs_drop` (mir_lower:2226) **over-approximate**: nó gọi `collect_heap_leaves` vốn đẩy MỌI Enum field thành leaf vô điều kiện → một `struct{c: CopyEnum}` bị coi là "needs drop" dù thuần Copy. Predicate đúng phải mirror **`MirType::is_copy(Some(body))`** (`triet-mir/src/lib.rs:694` — "single source of truth for move/copy classification"), khớp field-for-field. Lệch giữa hai cái an toàn cho get-by-value (`aggregate_needs_drop` over-approx chỉ gây thêm **một no-op drop-glue pass** trên bản copy của Copy-enum — KHÔNG double-free vì enum thuần scalar free rỗng), nhưng không được ghi sai bất biến vào ADR.

**Producer-consumer đóng bằng SINGLE-SOURCE, không nhân đôi:** JIT defensive guard cho `_get_copy` gọi THẲNG `MirType::is_copy(Some(body))` (HashMap path, mir_lower:4130) — cùng một hàm, không thể drift. `Type::is_copy_aggregate` (typecheck) và `MirType::is_copy` (mir) là hai hàm hai crate NHƯNG cùng shape sound (heap→false); **O verify máu: poison `is_copy_aggregate` heap→Copy → `Vector<Tagged{String}>` get double-free 134** (typecheck gate load-bearing, non-vacuous).

### AMEND-3.3 — refuse boundary heap-bearing

`get(Vector<Aggregate-heap>, i)` / `get(HashMap<scalar-K, Aggregate-heap>, k)` → REFUSE với diagnostic đích (KHÔNG E1041 generic). **G rule 2026-07-14: E-code mới `E1049` `GetAggregateByValueRequiresClone`** — message "aggregate has a heap-allocated leaf; copy-by-value would alias it" + `[Fix]` "Use `get_ref(container, k)` to borrow the element instead". Tách khỏi E1047 (fix khác: get_ref vs pop/remove; ADR-0027 machine-fixable đòi `[Fix]` chính xác).

### AMEND-3.4 — JIT copy-out (reuse get_ref locate + memcpy stride, KHÔNG tombstone KHÔNG free)

Đường JIT = **get_ref định vị cell + `copy_nonoverlapping(stride)` → dest fat-slot**, **BỎ** source-tombstone/free (khác pop/remove §AMEND-2):
- Locate: `__triet_vector_get_ref`(mir_lower:5291) / `__triet_hashmap_get_ref` → cell_ptr (not-found → NULL_SENTINEL).
- Copy: `copy_nonoverlapping(cell_ptr, out_ptr, stride)` (pattern sẵn ở pop :5365/:5429, TRỪ tombstone).
- Dest: `Nullable(Aggregate)` fat-slot tag-prepend ADR-0076 (`tag@0/fields@+8`), tag = `(ret==NULL_SENTINEL)?SENTINEL:1`. **Dùng CHUNG dest-bind marshal** với `vector_pop_fat||hashmap_remove_fat` (mir_lower:3975/4069) — chỉ nguồn khác (get_ref-copy thay pop-shim).
- **KHÔNG free, KHÔNG len--, KHÔNG state→2** — non-destructive. (Copy-only bảo chứng: cell bytes copy ra = giá trị độc lập, container giữ nguyên bản gốc.)

### AMEND-3.5 — borrowck: đọc-không-tiêu-thụ, KHÔNG loan

get-by-value KHÔNG consume container, KHÔNG move-out element, trả **owned value độc lập** (khác get_ref trả `&0` propagated-loan ADR-0079). Args `[false, false]`; borrow container kết thúc ngay tại call (như scalar get). KHÔNG PropagatedLoan.

### Scope container + teeth (O verify máu, cp-snapshot restore md5 `a753366b`)

- **Vector + HashMap scalar-key** CÙNG slice này (G rule 2026-07-14 — dest-marshal dùng chung, chỉ khác nguồn get_ref shim). HashMap **aggregate-key** (ADR-0083) × aggregate-value = compose sau (defer, refuse rõ CẢ typecheck E1041 fall-through LẪN JIT `Err(Unsupported)`).
- **Răng độc nhất (producer-consumer, load-bearing):** poison `is_copy_aggregate` heap→Copy → `Vector<Tagged{String}>` get → **double-free 134** (MIR: container `Drop(_2)` + copied-out `Drop(_12)` cùng free String). Typecheck E1049 gate là an-toàn DUY NHẤT cho Vector (JIT có defensive guard đối xứng `MirType::is_copy` — latent, chỉ nổ nếu typecheck regress). Restore md5 khớp.
- **8B-heap-struct T9-masking (Slice A leak-câm):** `Wrapper{v:Vector<Integer>}` (total_size=8, single handle) → O probe sống → **E1049** (thin-path KHÔNG bao giờ nhận nó). Fixture 367 canh vĩnh viễn, non-vacuous (`Id{value:Integer}` 366 cũng 8B nhưng Copy → compile OK).
- **Positive:** 361 `Vector<Point>` get×2 cùng giá trị (non-destructive) + miss→`~0` · 362 `HashMap<Integer,Point>` · 363 `Vector<Color>` disc-only enum · 366 thin 8B Copy struct · counting route-lower `lower_source` FREE==1 (unrelated String only, không stomp).
- **D tự bắt+vá (khai thật):** thin-return-as-pointer SIGSEGV 363 → cờ `*_fat` (chỉ stride>8 vào copy-loop).

**O đề 2026-07-14. O KÝ 2026-07-15** — verify máu độc lập: gate CLEAN `0·0·361·0`; răng producer-consumer đỏ (double-free 134); 8B-masking refuse; predicate `Type::is_copy_aggregate` ↔ `MirType::is_copy` cùng sound (heap→false). D flag LỆCH LỆNH predicate divergence đúng LUẬT 5 (O chấp nhận: `aggregate_needs_drop` over-approx enum-field, mirror `MirType::is_copy` mới chuẩn — O đã đính chính bất biến sai của mình ở §AMEND-3.2).

**G CO-SIGN 2026-07-15:** DUYỆT. E1049 = chốt chặn sinh tử (không đồ trang trí) — O poison chứng minh load-bearing. 8B-heap-struct T9-masking O tóm cổ + fixture 367 = verify-don't-trust đúng tinh thần. D bác `≡ !aggregate_needs_drop` → D đúng O sai, `MirType::is_copy` single-source-of-truth chuẩn xác; tách shim `_get_copy` chặt đường sinh loan = kiến trúc sắc sảo. Bất biến ADR-0042 (move-only) + ADR-0079 (cấm hidden-clone) giữ vững tuyệt đối. **§AMEND-3 ĐÓNG BĂNG.** Nợ kế: Deep-Clone (campaign lớn riêng) · get_ref value-aggregate · drain. ⚰️ ADR-0068 Box/recursive VẪN CẤM CỬA.
