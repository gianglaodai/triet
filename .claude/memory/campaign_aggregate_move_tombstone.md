---
name: campaign_aggregate_move_tombstone
description: "✅ ĐÓNG 2026-07-27(f) — WO-Aggregate-Move-Tombstone: giết UB double-free TẤT ĐỊNH khi move aggregate heap-bearing (widening + gán lại). Nhãn 'policy-hole KHÔNG UB' sống 8 ngày vì SUY LOẠI chứ không ĐO. b998c76+ab99f6e, gate 0·clean·0·533·0. Lộ quả bom #2: SIGSEGV param-alias."
metadata: 
  node_type: memory
  type: project
  originSessionId: f8391b98-1c93-4cb6-a8cf-8e5d66f4073c
  modified: 2026-07-27T17:35:01.948Z
---

## ✅ ĐÓNG — `b998c76` (fix+teeth) + `ab99f6e` (ADR-0065 §16). O✅/G✅/Giang✅ 2026-07-27(f)

Gate `0·clean·0·533·0`. Fixture 528 → **533**.

## 🎯 SINH RA TỪ RECON 3 NHÃN BACKLOG (Giang chốt "recon-tươi trước khi đánh lớn")

O đề xuất recon 3 nhãn chưa ai đo thay vì mở campaign nặng — lý do là **thống kê của
chính dự án**: nhãn backlog đã sai **4 lần** (4 zombie đã chôn). Kết quả:

| Nhãn | Phán quyết |
|---|---|
| **N1 widening (E1120)** | 🔴 **SAI ở phần chưa đo** — phần đã đo (enum payload scalar) đúng là policy-hole; phần chưa đo (payload **heap**) = **double-free** |
| **§15.6 `Vector<Leaf?>`** | (a) refuse container: **nhãn ĐÚNG**, fail-closed · (b) *"local `Nullable(Struct-heap)` qua widening — chưa đo riêng"* = 🔴 **CHÍNH LÀ UB** |
| **Deep-Clone** | ✅ **nhãn ĐÚNG** — `get` heap-bearing aggregate → **E1049**, rào sống, feature campaign |

🔑 **Hai nhãn TỰ THÚ cùng một ô chưa đo** (TODO:544 *"chưa đo payload heap"* ·
ADR-0065 §15.6 *"chưa đo riêng"*). **Ô đó là ô có bom.**

## 💀 UB: double-free TẤT ĐỊNH, 4 dòng source thường, 0 `unsafe`

```tri
struct Leaf { s: String }
let p = Leaf { s: "hi" };
let a: Leaf? = p;        // free(): double free detected in tcache 2  (exit 134)
```
MIR: `_2 = move _0` **KHÔNG kèm `Deinit(_0)`** ⇒ cả hai local nằm trong `owned_locals`
⇒ `Drop(_0)` + `Drop(_2)` cùng allocation.

⚔ **BỆNH KHÔNG PHẢI "WIDENING" — G tự đặt tên sai, phép đo bác.** Cú pháp thường
ngày nhất, **0 dấu `?`**, cũng nổ cùng cơ chế:
```tri
let mutable a = Leaf { s: "aa" };
let p = Leaf { s: "hi" };
a = p;                   // → 134
```
⇒ đổi tên `WO-Widening-Struct-Heap-UB` → **`WO-Aggregate-Move-Tombstone`**. Nếu giữ
khung "widening", vá 2 trong 3, để lại quả bom to nhất.

## 📊 BẢNG 8 VỊ TRÍ (G lệnh quét 5, O đo 8 — 2 ô mở nằm NGOÀI danh sách của G)

| # | Vị trí | Trước | Sau |
|---|---|---|---|
| 1 | `let a: Leaf? = p` | 🔴 **134** | ✅ 0 |
| 2 | `return p` → `Leaf?` | ✅ E1121 | giữ |
| 3 | `take(p)` param `Leaf?` | ✅ JIT refuse (Deinit ĐÃ có, do arg-move ADR-0042 Q1) | giữ |
| 4 | `Container { f: p }` | ✅ E1100 | giữ |
| 5 / 5b | `Vector<Leaf?>` / `HashMap<_,Leaf?>` element | ✅ MIR verifier B8 | giữ |
| **6** | **`a = p` (`a: Leaf?`)** — G KHÔNG liệt | 🔴 **134** | ✅ 0 |
| **7** | **`a = p` (`a: Leaf`, KHÔNG nullable)** — G KHÔNG liệt | 🔴 **134** | ✅ 0 |

Biến thể #1 cũng 134: nguồn **param** · field **`Vector`** · **struct lồng**.
Sound sẵn: `let q = p` · widening **enum** (rơi xuống `is_move_binding`, có Deinit) ·
rvalue · Copy-struct. `use p` sau widening → **E2420 bắt đúng** (borrowck KHÔNG mù;
vỡ thuần ở đường **DROP**).

## 📍 FIX — 2 site, thuần THÊM 52 dòng, đều gated `!ctx_is_copy`

- **Site A `triet-lower/src/lib.rs` `Stmt::Let`/`is_struct_widening`**: `return Ok(())`
  **nhảy qua** khối `is_move_binding` — nơi DUY NHẤT phát `Deinit(v)`.
- **Site B `Stmt::Assignment`**: **không có nhánh tombstone nào cả**. D tự cắm guard
  `v != orig` chống tự-gán (`a = a` sẽ Deinit mất bản duy nhất) — **WO của O không
  yêu cầu**.
- ⛔ **XOÁ nhánh `is_struct_widening` KHÔNG phải fix** (O đo: vẫn 134 `invalid pointer`,
  ca param đổi thành JIT refuse). Nhánh có lý do tồn tại (đổi type local sang Nullable).
- `ctx_is_copy` **KHÔNG phải thủ phạm** — nó descend đúng (`Nullable`→`Struct`→field
  `String`→false).

🔑 **Gốc rễ dưới lowerer (D chứng minh, O chỉ dám gọi nghi can):** aggregate
`ty_total_size > 8` rơi vào nhánh JIT "Multi-word copy" có comment tự nhận
*"Struct/enum types are Copy in Bậc A — no M1 zeroing needed"* — **SAI với struct
heap-bearing** ⇒ Zeroing-on-Move tự động không bao giờ chạm, tombstone phải tường minh.

## 🦷 VÌ SAO SỐNG 8 NGÀY

Nhánh **có** fixture phụ thuộc (tắt nó → corpus SIGILL), nhưng `231`/`234`/`235`/`237`
**đều dùng `Pt { x: Integer, y: Integer }` = Copy-struct** ⇒ biến thể heap chưa ai chạm.
**Vùng mù luật HP.3**: guard áp cho N biến thể thì teeth phải poison TỪNG biến thể.

Và nhãn được dán "policy-hole, KHÔNG UB" **hai lần**, cả hai lần **suy loại** từ N1
(enum) chứ không **đo** trên struct — hai lowering site khác hẳn nhau.

## 🩸 TEETH + POISON (O tự cắm, độc lập)

5 fixture `537/538/539/541/542` + `aggregate_move_tombstone_counting.rs` **route-lower
pointer-dedup**: ghi giá trị con trỏ, `dup = freed.len() - distinct_freed.len()`, assert
`dup == 0` **tách bạch** khỏi tín hiệu leak.

🔑 **Bộ đếm trần KHÔNG ĐỦ** cho bug mà bản chất LÀ double-free: free 2 lần cùng con trỏ
cho `count==2`, trùng khít "2 allocation hợp lệ". Khuôn anh em
(`heap_nullable_struct_local_counting.rs`) chỉ đếm trần — WO phải đòi dedup.

Poison 3 site (`if false &&`) → **cả 5 fixture ĐỎ 134**, counting test abort. Restore
md5 khớp.

## 🔴 HAI VIỆC LEO THANG (D đo, KHÔNG tự sửa — đúng lệnh WO)

**1. LEAK old-dest.** `a = p` không free giá trị cũ của `a`: `allocated=2, freed=1,
dup=0`. **Không phải regression** (patch chỉ thêm tombstone cho NGUỒN; trước patch vừa
leak vừa double-free). → G: **ghi sổ, campaign riêng `WO-Assign-Drop-Old-Dest`**
(leak << corruption trong thang ưu tiên).

**2. 💀 QUẢ BOM #2 — SIGSEGV 139 param-alias.** O dựng **worktree sạch tại `04cb5d3`**
build riêng để verify claim pre-existing của D:

| Probe | Baseline (chưa vá) | Sau vá |
|---|---|---|
| `function take(p: Leaf) { let q = p; }` (`is_move_binding`, patch KHÔNG chạm) | **139** | **139** |
| widening từ param | 134 | 139 |

⇒ **Patch KHÔNG đẻ ra nó.** Bug đã sống sẵn: prologue JIT không copy-in struct-by-value
param ⇒ `Variable` của param **là con trỏ thô alias bộ nhớ caller**; `Deinit` fallback
scalar zero mất chính địa chỉ đó → `Drop` load từ 0 → SIGSEGV. **Fixture 540 KHÔNG
tạo** — SIGSEGV giết cả binary integration-test (luật 15).

## ⚖ D BÁC O — ĐÚNG (lần 2 trong phiên)

WO của O lệnh sửa `TODO.md:574-577` xoá cụm "POLICY-HOLE, KHÔNG UB". **D từ chối**:
dòng đó là entry **N1/E1120 nói về nullable ENUM widening** — mà chính phép đo của O
xác nhận enum widening **có Deinit, chạy sạch**. Làm theo chữ của O là **nhét tuyên bố
SAI vào sổ**. D sửa đúng chỗ (ADR-0065 §15.6 gạch ngang + §16 mới).

D còn: tự mở rộng bán kính ra **Enum heap-payload** (ngoài bảng 8 ô của O), chứng minh
được Site C mà O chỉ dám gọi nghi can, Rule-7 probe nhánh `_ =>` (panic → 0 test chạm →
giữ code + ghi thẳng "UNTESTED", **không dán nhãn future-proof khống**).

## MẶT TRẬN KẾ (Giang chốt, ghi trong TODO.md `bd0f4c7`)

**`WO-Param-Aggregate-CopyIn`** theo thứ tự **recon-trước → trình bản đồ → G duyệt →
soạn WO**. ⚠️ **Cơ chế gốc rễ là chẩn đoán của D, O CHƯA verify độc lập** — O mới xác
nhận triệu chứng + tính pre-existing. Phiên sau **PHẢI tự đo bằng MIR/JIT dump**, không
chép khung của G hay D. ⛔ G cấm chuyển sang chiến dịch tính năng trước khi đóng bom này.

[[campaign_drain_fifo_teeth]] [[campaign_forgot_nullable_sweep]] [[campaign_aggregate_nullable]] [[campaign_truc_b_heap_in_aggregate]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_failure_mode_precision]] [[feedback_poison_must_be_red]]
