# ADR 0070 — Partial-move & Field-level Move-state (ZST/Capability)

**Trạng thái:** **🔒 SEALED** (Mentor G ký 2026-06-25; O verify máu 5 teeth đỏ độc lập + restore byte-identical). Áp dụng cho rewrite-era (Bậc C). Mở khóa partial-move field-level cho ZST/Capability field, hoàn tất canonical proof của schema §10 `HardwareToken` (destructure `let vga = hw.vga`). Amend [ADR-0025](0025-borrow-checker-rules.md) §5.3 + §9 schema BorrowChecker. Sibling của [ADR-0069](0069-zst-capability-token-luk3.md) (capability Ł3 ZST token).

**Issue:** Cỗ máy capability (ADR-0069) đã ship đầy đủ Ł3 — nhưng Lát 4 (A2) phải **né** tầm nhìn gốc schema §10: capability được truyền qua *param riêng* (`use_vga(vga: VgaBuffer)`) thay vì gói trong một struct `Hardware { vga, uart }` rồi destructure (`let vga = hw.vga`). Lý do né: borrow checker hiện track move-state **per-Local** (`var_states: BTreeMap<Local, VarState>` — `checker.rs:153`), không có cách biểu diễn "`hw.vga` đã moved nhưng `hw.uart` còn sống". Hệ quả: Δ3 (`checker.rs:615–624`) **cấm tuyệt đối** mọi việc trích Move-type ra khỏi field (`CannotCopyMoveTypeOut`). Không có per-Place move-state thì không có partial-move, và schema §10 — bằng chứng cốt lõi rằng capability = ownership thuần, zero-cost — không bao giờ compile được.

§5.3 ADR-0025 đã khóa mô hình 3-state (Owned/Moved/Conditionally-moved) nhưng *per-binding* và ghi "chi tiết defer implementation phase". ADR này lấp đúng phần defer đó cho chiều **field-level**.

---

## Quyết định

**Nâng move-state của borrow checker từ per-Local → per-Place (field-aware), đối xứng với loan-tracking đã có** (`places_conflict`, `checker.rs:46–89`, vốn đã field-aware).

Bốn khóa cụ thể:

1. **Per-field move-state.** Mỗi `Local` mang thêm tập field đã moved (single-level field projection: `hw.vga`). Đọc một Move-type field ra (`let v = hw.vga`) ghi field đó vào tập moved — KHÔNG còn refuse.

2. **Phạm vi có RĂNG CƯA — chỉ ZST/Capability.** Partial-move chỉ mở cho field kiểu **`Capability`** (ZST, drop = no-op runtime, không heap → không double-free khả dĩ). Field **heap** (String/Vector/HashMap) hoặc Move-type khác trích ra khỏi struct **VẪN BỊ TỪ CHỐI** bằng `CannotCopyMoveTypeOut` — đỏ chót, **KHÔNG panic**. (Field Copy đã được phép sẵn vì `is_copy`=true bỏ qua Δ3.) Partial-move field heap đẩy sang sổ đỏ No-Box (đòi JIT nhúng dynamic drop-flag/tombstone chặn double-free runtime).

3. **Tái dùng E2420 UseAfterMove** (KHÔNG đẻ mã mới). Dùng lại bất kỳ phần đã moved đều là UseAfterMove:
   - `let v = hw.vga; hw.vga` → E2420 (field đã moved).
   - `let v = hw.vga; let w = hw` → E2420 (động vào xác chết đã bị moi mất một phần).
   - Sibling chưa moved (`hw.uart`) → hợp lệ.
   - Diagnostic CÓ THỂ ghi rõ "partially moved", nhưng **mã chuẩn = E2420**.

4. **Merge per-Place tại join CFG = hợp (union), conservative, đơn điệu.** Field moved trên BẤT KỲ nhánh predecessor nào → moved tại join. Union là phép đơn điệu (lattice tăng dần) ⇒ fixpoint **hội tụ** (không treo vô hạn — điều kiện sống còn cho dataflow per-Place).

### Hình thức cụ thể

Canonical proof (schema §10, nay compile + chạy được):

```triet
capability VgaBuffer grant
capability UartPort  grant
struct Hardware { vga: VgaBuffer, uart: UartPort }   // all-ZST aggregate

function use_vga(v: VgaBuffer) -> Integer { return 10 }
function use_uart(u: UartPort) -> Integer { return 7 }

function main() -> Integer {
    let hw = Hardware { vga: mint VgaBuffer, uart: mint UartPort };
    let v = hw.vga;        // PARTIAL MOVE — vga ra khỏi hw, 0 byte runtime
    let a = use_vga(v);    // vga MOVED → 10
    let u = hw.uart;       // sibling — VẪN SỐNG (uart chưa moved)
    let b = use_uart(u);   // → 7
    return a + b;          // 17
    // hw.vga;             // ← nếu đụng vào: E2420 (vga đã moved)
    // let w = hw;         // ← nếu đụng vào: E2420 (hw đã partial-moved)
}
```

Răng cưa ranh giới (heap field — phải đỏ, không panic):

```triet
struct Box { name: String }
function main() -> Integer {
    let b = Box { name: "hi" };
    let n = b.name;        // E24xx CannotCopyMoveTypeOut — KHÔNG mở cho heap
    return 0
}
```

Răng dataflow CFG (field moved trong một nhánh, dùng tại join):

```triet
let hw = Hardware { vga: mint VgaBuffer, uart: mint UartPort };
if cond {
    let v = hw.vga;        // moved chỉ trên nhánh then
}
let again = hw.vga;        // JOIN — E2420 (union: moved trên ≥1 nhánh)
```

### Mô hình dữ liệu (định hướng — chi tiết ở Work Order)

`BlockState` thêm trường per-Local tập field đã moved, ví dụ:

```rust
partial_moves: BTreeMap<Local, BTreeSet<String>>   // local → {field names moved}
```

- Use của base `hw` (không projection) → moved nếu `var_states[hw]==Moved` **HOẶC** `partial_moves[hw]` khác rỗng.
- Use của `hw.f` → moved nếu base Moved **HOẶC** `f ∈ partial_moves[hw]`.
- `merge`: `partial_moves` lấy **union** field-set qua các predecessor.
- `StorageLive` / tái-gán fresh → xóa `partial_moves[local]`.

Single-level field depth (`hw.vga`) là phạm vi ADR này — `hw.a.b` lồng sâu → conservative whole-base move (hoặc defer), KHÔNG mở field-path đa cấp ở đây.

---

## Các phương án đã cân nhắc

| # | Phương án | Ưu | Nhược | Kết luận |
|---|-----------|---|-------|----------|
| 1 | **Per-Place move-state, scope ZST/Capability** (chọn) | Đối xứng loan-tracking đã có; sound zero-cost (ZST drop no-op); union monotone → fixpoint hội tụ; canonical §10 chạy | Đụng dataflow core, cần test CFG-branch kỹ | **CHỌN** |
| 2 | Per-Place move-state mở luôn cho **heap field** | Tổng quát hơn | Đòi JIT nhúng dynamic drop-flag runtime chặn double-free → tự sát phạm vi, đụng Cranelift | Defer → sổ đỏ No-Box |
| 3 | Giữ per-Local, "force whole-struct move" rồi cấm dùng lại base | Không đổi data structure | Giết sibling field (`hw.uart` chết oan) → KHÔNG phải partial-move, phản schema §10 | Bác |
| 4 | Mã lỗi mới E2421 cho partial-move-reuse | "Tường minh" | Ngữ nghĩa y hệt UseAfterMove → đẻ mã rác; E2421 đã là SelfOwnershipParadox | Bác — tái dùng E2420 |

---

## Hậu quả

### Tích cực
- Hoàn tất canonical proof schema §10 `HardwareToken`: capability = ownership thuần, destructure-move, zero-cost — *chạy thật* chứ không còn "design only".
- Borrow checker có per-Place move-state — nền cho mọi partial-move tương lai (kể cả heap khi No-Box mở).
- Đối xứng hóa: move-tracking nay cùng độ phân giải field như loan-tracking.

### Tiêu cực
- Tăng độ phức tạp `BlockState` + `merge` + use-check (ba điểm phải sửa đồng bộ).
- Single-level field depth — `hw.a.b` chưa hỗ trợ (conservative).

### Rủi ro cần mitigate
- **Fixpoint treo / vỡ soundness ở merge** — bắt buộc union (monotone), bắt buộc test CFG-branch (răng #4). Đây là rủi ro G đã chỉ đích danh.
- **Răng cưa heap rò rỉ** — phải có test thọc String ra khỏi struct → `CannotCopyMoveTypeOut` đỏ, không panic (răng #5).
- **JIT all-ZST struct** — `struct Hardware` toàn field ZST = 0 byte; StructAlloc/field-read 0-byte có thể chạm edge-case Cranelift StackSlot size-0. Bắt buộc **probe Step 0** trước khi fix, refuse-over-guess.

---

## Ngày hiệu lực

- Rewrite-era Bậc C, sau ADR-0069 — kích hoạt khi Work Order ADR-0070 đóng (O verify máu + G ký).
- Amend [ADR-0025](0025-borrow-checker-rules.md) §5.3 (mở rộng move-state per-Place) — KHÔNG hồi tố, KHÔNG revisionism mô hình per-Local cũ (per-Local là subset của per-Place khi projection rỗng).
- Amend schema §9 `BorrowChecker` (ghi nhận field-level move granularity).
- Heap-field partial-move (write-side, lúc ADR seal): KHÔNG áp dụng — defer sổ đỏ No-Box.
- **Read-side update (2026-06-26, WO Read-side heap field move-out + Addendum):**
  single-level **heap-SCALAR** field move-out (`let s = p.name` với `name:
  String/Vector/HashMap`) NAY ĐÃ MỞ — borrowck ghi partial-move, JIT tombstone
  heap-leaf ở base-slot (free đúng 1 lần). Lower type-prop heap field (String→
  String thay vì Unknown) đi kèm. **Heap-STRUCT field move-out (`let m = h.inner`
  với `inner` là struct chứa heap) VẪN HOÃN (E2423):** chặn upstream bởi
  construction-into-field double-free pre-existing (ADR-0067, đã verify). Re-mở
  khi construction-into-field tombstone source. Fixture nắp quan tài:
  `300_field_moveout_heapstruct_e2423.tri`.

---

## ✚ AMEND — Phase 2: heap-STRUCT field move-out MỞ (G bless 2026-06-27)

Construction-into-field double-free (ADR-0067) đã vá (commit `e2b5c36`,
AMEND ADR-0067 — Deinit source tombstone), nên nắp quan tài mở được. Phase 2
unlock **single-level heap-STRUCT** field move-out (`let m = h.inner`, `inner`
là struct chứa heap leaf bất kỳ độ sâu). 3 code-site (recon O ban đầu chỉ ra 2 —
**thiếu site 3**; D probe SIGSEGV tới đáy, bổ sung):

1. **Borrowck** (`checker.rs` allow-arm): thêm `MirType::Struct(_)` vào nhánh ghi
   partial-move. Use-after-move kế thừa miễn phí (`partial_move_invalidates`
   theo tên field, type-agnostic): reuse field đã move → **E2420**; sibling field
   đọc tiếp OK; whole-base / multi-level → invalidated. Enum-field + multi-level
   extraction VẪN refuse **E2423** (defer — chưa use-case).
2. **JIT** (`mir_lower.rs` read-side block): heap-STRUCT field → `collect_heap_leaves(
   name, field_off, ..)` (hàm đã có `base_offset`) trả leaf ở **absolute offset
   trong slot CHA**; zero per-LeafKind (Heap@abs, Enum@abs+8). Đối xứng byte với
   Deinit struct-branch (`base_offset=0`→`field_off`). Free đúng 1 lần.
3. **Lower type-propagation** (`lib.rs` `Expr::FieldAccess`): **TỬ HUYỆT site 3.**
   Trước đây field kiểu `MirType::Struct(_)` rơi `alloc_local()` = **Unknown** →
   JIT pre-pass KHÔNG cấp stack-slot cho dest move-out → aggregate-copy ghi qua
   địa chỉ rác → **SIGSEGV**. Vá: propagate type thật `Struct(_)` (song song
   nhánh nullable-aggregate / heap-scalar đã có) → dest có slot thật.

   **Quyết định type-system (G bless):** tầng Lower PHẢI propagate type thật cho
   `MirType::Struct(_)` field read, KHÔNG dùng `Unknown`. Lý do: (a) dest-slot
   allocation cho move-out; (b) vá luôn **latent-bug truncation 8B** — mọi
   `let x = obj.copyStruct` với Copy-struct >8B trước đó đọc qua SSA-register 8B
   (cắt cụt); nay aggregate-copy đúng width. `Unknown` chỉ còn cho scalar leaf
   (load i64). Propagate đúng type = con đường Soundness duy nhất.

**Teeth (O verify máu, độc lập):** `struct_field_moveout_phase2_counting`
(FREE==1, cap==5, poison JIT Struct-arm → count==2 double-free); negative
fixtures `301` (reuse → E2420), `302` (multi-level → E2423); fixture `300` FLIP
(E2423 → EXPECT:0). Regression: 263/264/265 + Phase-1 + nested/enum-in-struct
counting XANH. Gate `0·0·297·0`. Copy-struct >8B field read verified (đọc đúng,
base reusable — Copy semantics giữ).

**Defer:** Enum-field move-out · multi-level (`h.inner.x`) extraction · true-recursive (ADR-0068).

## ✚ AMEND — Phase 2b: Enum-field move-out MỞ (WO-0074, G ký 2026-06-29)

`let e = h.msg` (`msg` là enum mang heap payload) trước đây refuse **E2423**
(allow-arm chỉ {Capability, heap-scalar, Struct}). Construct + base-Drop của
enum-in-struct đã chạy từ ADR-0067; Phase 2b chỉ mở **đường move-out**. 3 site
đối xứng Phase 2 (commit `e0b1ed7`):

1. **Lower** (`lib.rs` `Expr::FieldAccess`): thêm `matches!(field_ty,
   MirType::Enum(_))` vào gate cấp typed-slot → dest mang `Enum` → JIT pre-pass
   cấp enum-slot. Thiếu → dest Unknown → no-slot → aggregate-copy ghi địa chỉ rác
   → **SIGSEGV** (đối xứng tử huyệt site-3 Phase 2).
2. **Borrowck** (`checker.rs` allow-arm): thêm `MirType::Enum(_)`. `partial_moves`
   key = tên field đơn ("msg"), **data-structure KHÔNG đổi**. Nullable/Outcome
   field-move VẪN refuse.
3. **JIT** (`mir_lower.rs` move-out tombstone): arm `Enum(_)` zero **chỉ
   payload-ptr@`field_off+8`** (disc giữ) → base tag-switch Drop đọc ptr=0 → free
   no-op. Đối xứng leaf-Enum tombstone (`abs+8`).

**Teeth (O verify máu, độc lập, cp-snapshot):** 5 răng — borrowck allow (poison
→ E2423), double-free (poison JIT → FREE 2 vs 1), leak (cực âm `==1`), cap+count
đồng thời (`STR_CAP==5 && STR_FREES==1`, assertion-guard), **SIGSEGV in-suite**
(poison Lower → child subprocess signal 11 / code 139, crash cô lập). Gate
`0·0·303·0` (counting/subprocess là test-binary riêng → corpus 303 đứng yên).

**Defer:** multi-level (`h.inner.x`) extraction → AMEND Phase 3 dưới · true-recursive (ADR-0068).

## ✚ AMEND — Phase 3: Multi-level extraction (projection-path move-state) [G ký 2026-06-29]

### Lý do mổ
`let x = h.inner.x` (≥2 Field projection) refuse **E2423** vì `single_field` trả
`None` cho multi-level. Trước khi xây **Capability Ł3** (cần track capability của
field lồng nhau), borrowck phải track **projection-path** — nếu không Ł3 gãy ở
đúng chỗ này. G phán: dọn móng trước khi xây lâu đài.

**Đường nứt gốc:** `partial_moves: Map<Local, Set<String>>` — key là MỘT tên
field. KHÔNG biểu diễn được "`inner.x` moved nhưng `inner.y` sống". Nâng key lên
**projection-path**.

### 1. Data-model: `Set<String>` → `Set<Vec<String>>` (KHÔNG Trie)
```rust
partial_moves: BTreeMap<Local, BTreeSet<Vec<String>>>   // local → {moved paths}
// h.msg → ["msg"]   |   h.inner.x → ["inner","x"]   |   whole h → []
```
Trie bị bác: tập moved-path mỗi local nhỏ (vài field), prefix-scan O(paths×depth)
không đáng kể. `Vec<String>` giữ idiom union hiện có, không over-engineer.

### 2. Quan hệ PREFIX-CONFLICT (tim của cascade)
P (đọc) và M (moved) **xung đột** ⟺ một là prefix của cái kia (kể cả bằng):
`conflict(P,M) ⟺ is_prefix(P,M) ∨ is_prefix(M,P)`.

| đọc P | moved M | quan hệ | kết |
|---|---|---|---|
| `[inner,x]` | `[inner,x]` | exact | ❌ DEAD |
| `[inner,x]` | `[inner]` | M prefix P (cha đã đi) | ❌ DEAD |
| `[inner]` | `[inner,x]` | P prefix M (đọc cha chạm con) | ❌ DEAD |
| `[]` | `[inner,x]` | whole-base | ❌ DEAD |
| `[inner,y]` | `[inner,x]` | divergent | ✅ **LIVE** (sibling leaf) |
| `[other]` | `[inner,x]` | divergent | ✅ **LIVE** (sibling branch) |

Single-level là ca riêng (M=`[f]` exact; P=`[]` whole-base) → backward-compat 100%.

### 3. Cascade 3 hàm
- `single_field` (checker.rs:403) → **`projection_path(place) -> Option<Vec<String>>`**:
  mọi proj Field → path đầy đủ; gặp non-Field proj → `None` (conservative
  whole-base); caller coi `None`/`[]` = whole-base.
- `partial_move_invalidates` (416): `moved.iter().any(|m| prefix_conflict(&p, m))`
  thay `moved.contains(f)` — subsume logic cũ (§2 chứng minh).
- allow-arm record (702-721): `Some(path) if !path.is_empty() && (Cap|heap|Struct|
  Enum)` → `insert(path)`; non-Field proj / non-move-type → vẫn **E2423**.

### 4. 🩸 Lỗ hổng fixpoint CHẾT NGƯỜI — bịt NGAY trong amendment này (commit tách)
Fixpoint check (checker.rs:520-521 entry + 541-542 exit) chỉ so `var_states` +
`active_loans`, **KHÔNG so `partial_moves`**. Vì partial-move KHÔNG set
base→Moved (base vẫn Owned), delta `partial_moves` bị **âm thầm vứt** → trong
vòng lặp, move ở iteration-1 không xâm nhập entry iteration-2 qua back-edge →
**UAM bỏ sót = UNSOUND**. Đây là lỗ **CÓ SẴN** (latent cho cả single-level,
chưa ai test loop+partial-move). Vá: thêm `&& new_entry.partial_moves ==
entry_states[b].partial_moves` (entry) + `|| new_exit.partial_moves !=
exit_states[b].partial_moves` (exit). **Commit riêng, NẰM TRƯỚC commit feature**
(G mandate: 1 commit vá bug lõi, 1 commit feature — git history sạch).

Union-merge (231-238) đổi `Set<String>`→`Set<Vec<String>>` union; monotone, hội
tụ; intersection UNSOUND (quên move nhánh anh em). Luận chứng KHÔNG đổi.

### 5. Reassignment clear — sub-path KHÓA bằng negative tooth (G chốt)
Whole-base re-assign / `StorageLive` fresh → `partial_moves.remove(local)` (xóa
toàn bộ path) — ĐÚNG. **Sub-path re-assign** `h.inner = fresh` sau khi move
`h.inner.x` (cần `retain(|m| !is_prefix([inner], m))`): **KHÔNG mở Phase 3** —
đéo có use-case thì đéo mở. Cắm cờ tóm cổ + diagnostic tử tế + **negative tooth
chứng minh đã khóa**. Mở khi Giang có nhu cầu.

### 6. Phạm vi
- **PART A (HEART — ca mổ tim):** §1-5 borrowck core + §4 fixpoint. Cái G ký.
- **PART B (chi — reuse Phase 2):** JIT tombstone leaf multi-level ở absolute
  offset qua `walk_projections` (đã trả `(ty, abs_off)`); Lower `place_result_type`
  (lib.rs:1561) đã loop mọi Field proj → multi-level leaf-type đã resolve. Verify
  Site-1 phủ khi soạn WO.
- **OUT:** non-Field projection (Index/Deref) · sub-path reassign (§5) · true-recursive (ADR-0068 CẤM).

### 7. Teeth (8 răng — phong cách WO-0074, máu đổ TRƯỚC khi vá)
| # | Tooth | Scenario | Vá | Poison → RED |
|---|---|---|---|---|
| A sibling-live | move `h.inner.x`, đọc `h.inner.y` | ✅ no error | base-only invalidate → false UAM |
| B ancestor-dead | move `h.inner.x`, đọc `h.inner` | ❌ UAM | gỡ "P prefix M" → no error |
| C exact-dead | move `h.inner.x`, đọc lại `h.inner.x` | ❌ UAM | gỡ exact → no error |
| D whole-base-dead | move `h.inner.x`, đọc `h` | ❌ UAM | gỡ "[] prefix" → no error |
| E sibling-branch-live | move `h.inner.x`, đọc `h.other` | ✅ no error | over-conservative → false UAM |
| F ⚔ merge-union | move `h.inner.x` 1 nhánh CFG, join, đọc | ❌ UAM | union→intersection → no error |
| G 🩸 fixpoint-loop | loop move+re-read qua back-edge | ❌ UAM | gỡ `partial_moves` khỏi fixpoint check → no error |
| H runtime | `let x=h.inner.x` chạy | FREE==1 | gỡ JIT multi-level tombstone → FREE==2 |
| (neg) sub-path-locked | `h.inner=fresh` sau move `h.inner.x` | ❌ diagnostic khóa | (§5 — chứng minh đã khóa) |

F + G là xương sống soundness. A-G borrowck (check-mode); H JIT counting.

### 8. Phương án đã cân nhắc
(a) **`Vec<String>` path** ✅ — đơn giản, đúng idiom, scan rẻ. (b) Trie/radix —
premature, 0 lợi ích đo. (c) Place-id interning — over-engineering. (d) Giữ
refuse multi-level — chặn Ł3 nested, bác.

### 9. Hậu quả
**Tích cực:** projection-path move-state = nền Ł3 nested capability; bịt lỗ
fixpoint có sẵn; multi-level mở. **Tiêu cực:** `Vec<String>` clone nhiều hơn
`&str` (chấp nhận — borrowck không nóng); cascade chạm 4 site core + fixpoint.
**Rủi ro:** merge/fixpoint chỗ chết người (tooth F+G gác); sub-path reassign khóa (§5).
