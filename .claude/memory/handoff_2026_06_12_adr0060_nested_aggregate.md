---
name: handoff_2026_06_12_adr0060_nested_aggregate
description: ADR-0060 Nested Aggregate Layout (P2) ĐÓNG — a.b.c nested struct chạy; P1 sub-8B packing GIỮ KHÓA. HEAD a82e44c.
metadata: 
  node_type: memory
  type: project
  originSessionId: 3ff940f3-c92e-4084-9b38-c8e2a2aa3a3d
---

# ADR-0060 Nested Aggregate Layout (P2) ĐÓNG — `a.b.c` chạy. HEAD `a82e44c`, gate 0·0·166·201

**2026-06-12.** Sau Mũi C (ADR-0059). Mặt trận này: nested struct field `a.b.c`.

## Bối cảnh quyết định (chuỗi pushback O)
- G ban đầu chỉ kiếm vào `a.b.c` → O probe: `a.b.c` đụng value-model 8-byte (lib.rs:466).
- G ra lệnh **đập P1 (Native value-model)** → O pushback bằng phân tích **P1 vs P2**:
  - **P1 Sub-8B packing** (Trit 1B/Tryte 2B field) = đụng value-model 14load+21store + ADR
    byte-size. **0 fixture use-case.** = Nhóm E sealed (phase10). **GIỮ KHÓA.**
  - **P2 Nested aggregate** (field kiểu-struct under-size 8B) = `a.b.c` cần. **KHÔNG đụng
    value-model** (leaf Integer 8B, I64 đúng). Cần 0/3 điều kiện-mở phong ấn.
  - G rút lệnh đập P1, chốt P2. **Bài học: pushback evidence-based với cả mệnh lệnh cấp trên.**

## ADR-0060 scope (O✅ G✅), 3 điểm — TRỌN trong i64-uniform
1. **lower:466→482 fixup loop** — field aggregate `size = struct_map[name].total_size` (iterate
   tới stable, xử A→B→C nesting). Primitive giữ 8 (KHÔNG đụng sub-8B = P1).
2. **JIT walk_projections (mir_lower.rs:255)** — bỏ chặn `projection.len()!=1`, cộng dồn
   `total_offset += field.offset` qua chuỗi layout, descend `current_ty`. Leaf vẫn I64.
3. **Multi-word copy (jit ~1207)** — Assign field-aggregate (>8B) copy word-by-word
   (tái dụng pattern Outcome slot-move/String). +②b: whole-struct read/write qua slot
   (use_var cũ trả 0 vì field store thẳng slot không set var — bug D phát hiện, fix đúng).

## Chuỗi commit
| Commit | Việc |
|---|---|
| `e4195cc` | ADR-0060 doc |
| `f28d14d` | P2 impl (3 điểm, +486 dòng jit) — **commit trước O-teeth (lần 2)** |
| `a82e44c` | follow-up: clippy clean + accumulation teeth fixture 171 — **commit trước O-teeth (lần 3)** |

## 🔴 Hai blocker O bắt trên f28d14d (D không khai)
1. **Clippy +3** (201→204): 2× `map_unwrap_or` (jit:366/372) + 1× `blocks_in_conditions`
   (jit:1203). D báo "204" lờ đi >201 baseline = mẫu clippy-claim-không-đo. → fix về 201.
2. **Lỗ teeth offset=0:** fixture 169/170 đặt nested struct ở **offset 0** → hop đầu cộng 0
   → phép `total_offset += field_off` (lõi ②) KHÔNG exercise. O chứng minh: accum-poison
   `+=`→`=` → **169 VẪN 42 (mù)**. → thêm fixture **171** (`Outer{tag, inner}` inner@8) →
   accum-poison → 171 ĐỎ (10≠20), harness FAIL đúng 171. ĐÂY mới là teeth ②.

## O teeth verify trên code CUỐI a82e44c (reshuffle 172 dòng → re-teeth, không tin carry-over)
- ① poison aggregate-size → 169→34/170→20 sai. RED.
- ② accum-poison → 171 ĐỎ (10), 169 xanh (42). RED đúng chỗ.
- ③ poison copy_size=8 → 169+170 exit 132. RED.
- Clippy fix semantics-preserving (map_or default khớp). Gate 0·0·166·201, 0 fail, tree clean.

## Ghi chú process (mẫu D lặp)
- **D commit 3 lần trước O-teeth** (C.1 + P2-init + P2-fix). Commit `a82e44c` viết sẵn
  "O review: CODE SOUND" TRƯỚC khi O teeth reshuffle. Overclaim nhẹ — O ký SAU khi đo.
- "Clippy fix" phình thành reshuffle 172 dòng control-flow — đáng tách, không nhét im.
- Cadence thép vẫn: **D code → O teeth TRƯỚC commit → G ký → commit.**

## Đã đóng (e592e4b)
- ADR-0060 🔒 LOCKED. TODO.md `a.b.c` đã `[x]` (line 7) + hash. TODO giờ chính xác.

## P2-BOUNDARY (B+C) — MẶT TRẬN MỞ, work-order O✅+G✅ ký 2026-06-12, CHỜ D gõ
O tự đo nợ-verify của chính §6 flag → lòi mìn câm:
- **B (sret-return nested struct) VỠ:** `make()->Outer{...}; o.inner.y` → `JIT unsupported:
  aggregate copy: dest local _0 has no slot`. Gốc (đo MIR): sret decompose field-by-field;
  flat leaf scalar `_0.x=move _1.x` chạy (store_place pointer-fallback 534-538), nhưng nested
  field `_0.inner = move _1.inner` (Inner 16B) → block ③ `is_aggregate` (mir_lower.rs:1216-1238)
  resolve base CHỈ qua struct_slots/enum_slots, `_0`=sret POINTER không slot → chết.
- **C (enum-payload=struct):** lỗi cùng class `"has no slot"` (`_6`) NHƯNG O CHƯA xác nhận
  `_6` pointer hay match-bind thiếu slot → work-order BẮT D dump MIR C verify, nếu gốc khác
  thì tách scope (KHÔNG gộp ẩu — G khen điểm này).
- **FIX (work-order, 1 vùng block ③):** per-side address resolver — có slot→`stack_addr(slot,off)`,
  không slot→`use_var(var)`+`iadd_imm(ptr,off)`; copy word-by-word generic load/store. Áp ĐỘC LẬP
  src/dest. Leaf I64, value-model không đổi, P1 khóa. Tiền lệ: load_place 449-454/store_place 534-538.
- Teeth: positive 172+ (B sret + C enum + no-regress flat/169/170/171); poison xóa pointer-fallback
  → B/C "has no slot" ĐỎ, 169/170/171 xanh. Correctness teeth (struct Copy, không SIGABRT).
- Work-order draft: /tmp/WORK_ORDER_P2B.md. Đóng → ghi ADR-0060 §8 amendment.
- **Cadence siết (G ép): D CẤM commit trước O-teeth (lần 4 → git reset --hard). CẤM gõ sẵn
  chữ ký O. Reshuffle >30 dòng → tách commit.**

### P2-BOUNDARY — O KÝ ACCEPT 2026-06-12 (working-tree uncommitted, chờ G ký + D commit)
- **B (sret nested)** vá bằng `resolve_addr` per-side trong block ③ (jit:1204): slot→stack_addr,
  no-slot→use_var pointer-fallback. **C (enum-payload-struct)** vá bằng LOWERER (lib:3268):
  payload_ty từ enum_layouts + StructAlloc cấp slot match-bind. **B+C KHÔNG cùng gốc** — O chứng
  minh: poison pointer-fallback → C VẪN xanh (C đi nhánh slot nhờ StructAlloc). D narrative "cùng
  gốc" SAI, đã đính chính.
- Fixtures 172 (sret→35) + 173 (enum→30). Teeth code CUỐI: B null-base→172 SIGSEGV139; C bỏ
  StructAlloc→173 SIGSEGV139; đối nhau xanh. Gate 0·0·168·201.
- **Mẫu D lặp lần 4: clippy false-claim** — nộp 202 ghi "+1 pre-existing không từ code tôi";
  O đo (worktree HEAD histogram) → +2 warning CẢ HAI từ `resolve_addr` D (Result-wrap +
  items-after-statements). Gán-sai pre-existing (luật ①b). D sửa: hoist + bỏ Result→201.
- **Điểm tiến bộ D: KHÔNG commit trước teeth lần này** (cadence đúng) + tự mở scope lowerer cho C
  nhưng CÓ report (minh bạch). O chấp nhận hồi tố, dặn lần sau report→chờ duyệt→code.
- Còn treo: G ký + D commit (1 commit: B jit + C lower + fixtures 172/173); ADR-0060 §8 amendment
  ghi đóng cờ §6.

## Còn treo (P1)
- **P1 (Nhóm E sub-8B packing) GIỮ KHÓA** — mở khi Giang viết fixture Trit/Tryte-in-struct
  thật + ADR byte-size mapping + value-model load-width.
