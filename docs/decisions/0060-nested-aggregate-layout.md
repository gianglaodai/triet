# ADR-0060 — Nested Aggregate Layout (P2): struct-in-struct sizing + JIT nested projection

- **Status:** 🔓 APPROVED (scope) — chờ thi công. Khởi thảo Mentor O 2026-06-12, grounded từ probe driver-run + line-cite + tách bạch P1/P2.
- **Date:** 2026-06-12
- **Khởi thảo:** Mentor O (probe `a.b.c` chạm đáy JIT; tách P2 nested-aggregate khỏi P1 sub-8B packing).
- **Chữ ký:** O ✅ (root cause proven bằng driver-run; flat-struct sound / nested-broken đo trực tiếp; tách P1/P2) · G ✅ (duyệt scope 3 điểm 2026-06-12 — rút lệnh đập P1 value-model sau phân tích YAGNI, giữ Nhóm E khóa).
- **Liên quan:** [phase10-native-struct-layout.md](../../spec/plans/phase10-native-struct-layout.md) (P1 sub-8B packing — Nhóm E sealed, GIỮ KHÓA), [ADR-0049](0049-fat-pointer-abi.md) (String 3-word fat-pointer copy — tiền lệ multi-word), [ADR-0057](0057-jit-outcome-slot-move.md) (Outcome slot-move word-by-word — tiền lệ multi-word), [ADR-0050](0050-mir-type-enum.md) (MirType — Struct/Enum bare).

---

## 1. Context — `a.b.c` vỡ; flat struct sound; P1 ≠ P2

`a.b.c` (nested field access) là nợ-móng thật (TODO Phase 4 dòng 7). Probe O 2026-06-12 đo
bằng `triet-driver run`, tách bạch hai tầng từng bị gộp nhầm:

| Tầng | Bản chất | Chạm value-model? | Use-case | Quyết |
|---|---|---|---|---|
| **P1 — Sub-8B packing** | field `Trit`(1B)/`Tryte`(2B) tại offset thật → `stack_load(I64)` đọc tràn | **CÓ** (14 load + 21 store I64→typed-width + extend) | **0 fixture** | **GIỮ KHÓA** (Nhóm E, phase10) |
| **P2 — Nested aggregate** | field kiểu-Struct/Enum bị cấp 8B → store tràn / data-loss | **KHÔNG** (leaf=Integer 8B, I64 đúng) | `a.b.c` (Integer) thật | **ADR NÀY** |

**Đo: cái gì SOUND hôm nay (KHÔNG được regress):**
- Flat multi-field struct: `Point{x,y}; p.x+p.y` → 3. Whole-copy `let p2=p; p2.x+p2.y` → 3.
  Param by-pointer `sum(p:Point)` → 3. 3-field `t.a+t.b+t.c` → 6. **Tất cả xanh.**

**Đo: cái gì VỠ (chỉ nested aggregate):**
- `Outer{inner:Inner, tag}; o.inner.x` → CHECK OK (lower+borrowck nuốt nested projection),
  **RUN: `JIT unsupported: nested projections not supported`** (`mir_lower.rs:272` load + `:381` store).
- `Outer{inner:i, tag:7}; o.tag` → **7 (chạy NHƯNG sai âm thầm)**: construction chỉ copy
  1 word của Inner (mất `i.y`); tag@8 còn nguyên vì inner bị under-size 8B che lỗi.

## 2. Root cause — ĐO TỪ CODE, ba điểm

1. **Layout under-size field aggregate.** `triet-lower/src/lib.rs:466` hardcode
   `(f.name, ty, 8, 8)` — MỌI field 8B kể cả field kiểu-struct. `Outer{inner:Inner(16B)}` →
   inner cấp 8B (thiếu 8B). `StructLayout::compute` (mir) cộng `offset += size` đúng theo
   input, nhưng input giả.
2. **JIT nested projection bị chặn cứng.** `mir_lower.rs:272`/`:381`: `if projection.len() != 1
   { Err("nested projections not supported") }`. Field-offset BẢN THÂN đã sạch (dùng
   `field.offset`, không `index*8` — phase10 Q2).
3. **Construction copy 1-word.** Default `Statement::Assign` (`mir_lower.rs:1137-1139`):
   `val = load_place(source); store_place(dest, val)` — một i64. Field kiểu-struct (≥2 word)
   mất các word sau word đầu.

## 3. Decision (G duyệt scope 2026-06-12). MỘT chiến dịch, ba điểm.

**Vá nested aggregate TRỌN trong value-model i64-uniform. Leaf vẫn Integer 8B → `stack_load(I64)`
giữ nguyên. P1 (sub-8B packing) GIỮ KHÓA.**

### Điểm 1 — Layout sizing (lower)
`lib.rs:466`: field kiểu `MirType::Struct(name)`/`Enum(name)` → `size = struct_map[name].total_size`
(bội số 8 khi leaf=Integer), `align = 8`. Field primitive giữ `8, 8` (KHÔNG đụng sub-8B = P1).
`struct_map` đã sẵn ở `lower:472`. Cần thứ tự topo (nested def trước outer) hoặc 2-pass —
probe thi công xác định.

### Điểm 2 — JIT nested offset-walk (load_place + store_place)
Bỏ chặn `projection.len() != 1`. Walk chuỗi projection: bắt đầu type = `local.ty`, mỗi
`Field(name)` → tìm field trong layout hiện tại, **cộng dồn `field.offset`**, descend vào
`field.ty`'s layout (tra `body.struct_layouts`). Leaf load/store vẫn `I64` tại offset tổng.

### Điểm 3 — Multi-word copy cho construct/assign field-aggregate
Khi Assign dest hoặc source là field/local kiểu aggregate (≥2 word): copy word-by-word
`while off < size { stack_load(I64, src, base_src+off); stack_store(dest, base_dest+off); off+=8 }`.
**Tái dụng TIỀN LỆ:** Outcome slot-move (`mir_lower.rs:1127-1132`) + String 3-word
(`:1140-1156`, `:921-930`). `size` = aggregate layout `total_size`.

## 4. Teeth (ranh giới sinh tử) — route-lower qua `lower_source`/driver-run, CẤM hand-build MirBuilder

### Positive (fixture mới, số kế = max+1, D check `ls fixtures`)
- `Outer{inner:Inner{x,y}, tag}; return o.inner.x + o.inner.y + o.tag` → giá trị đúng (đọc
  nested 2 cấp + flat field cùng struct).
- Nested write: `o.inner.x = 5; return o.inner.x`.
- **No-regress flat:** giữ fixture flat-struct hiện có xanh (Point/param/3-field).

### Poison (mỗi điểm một độc dược, đo trực tiếp)
- **Điểm 1 poison:** revert `lib.rs:466` về hardcode `8` cho field aggregate → `o.inner.y`
  trả **sai giá trị** (đọc đè lên tag / data-loss) HOẶC layout sai → giá trị lệch. Test phải đỏ.
- **Điểm 2 poison:** revert bỏ-chặn → `JIT unsupported: nested projections not supported`
  quay lại. Test đỏ.
- **Điểm 3 poison:** ép construction copy 1-word (bỏ vòng while) → `o.inner.y` mất (trả rác/0).
  Test đỏ.
- ⚠️ Phân biệt: đây là teeth **giá-trị-sai** (observable bằng kết quả lệch), KHÔNG cần
  SIGABRT. Nếu poison làm tràn slot ghi đè field kế → có thể giá trị sai cụ thể; teeth bắt
  bằng EXPECT số đúng. Khôi phục `cp` /tmp, CẤM git checkout.

## 5. RA NGOÀI scope (GIỮ KHÓA)
- **P1 sub-8B packing** (Trit 1B/Tryte 2B trong struct) — Nhóm E sealed. 0 fixture. Value-model
  load-width KHÔNG đổi. Mở khi Giang viết fixture Trit-in-struct thật + ADR byte-size mapping.
- **Tuple types** — chưa có cú pháp. Native-pack Outcome (C4) — 0 producer.
- Aggregate field kiểu **heap** (String/Vector trong struct) — đã chặn `lib.rs:2503-2510` B8
  (`is_copy` reject). Move-type-in-struct là campaign riêng (drop/ownership), không phải P2.

## 6. Consequences
- (+) `a.b.c` nested ≥2 cấp chạy đúng, khép nợ Phase 4. Tuple/enum-payload-struct tương lai
  dùng lại cơ chế offset-walk + multi-word copy.
- (+) Value-model i64-uniform GIỮ NGUYÊN — blast radius nhỏ (lower:466 + 2 JIT fn + Assign),
  KHÔNG đụng 35 site I64-width của P1.
- (−) Multi-word copy tăng instruction cho aggregate assign — chấp nhận (đúng theo size).
- (−) Enum payload chứa struct + nested qua param/sret cần offset-walk lan tới callee field-load
  (`mir_lower.rs:921-952`, `:706-712`) — **probe thi công phải xác nhận** không bỏ sót.

## 7. Chỉ thị tác chiến
1. **D thi công** theo 3 điểm; có thể chia 2 lát (đọc nested trước: điểm 1+2+3-construct →
   `o.inner.x` đúng; rồi nested-write store-walk) — D đề xuất, O duyệt slice.
2. Gửi O review + **raw gate dòng đầu** (auto-reject nếu không raw) → **O teeth tay TRƯỚC
   commit** (poison 3 điểm, đo giá-trị-sai/JIT-error trên code CUỐI) → G ký → commit.
   **KHÔNG nhảy cóc** (bài học C.1).
3. Mỗi lát: cập nhật TODO.md + handoff. Phong ấn P1 ghi rõ vẫn đóng.
