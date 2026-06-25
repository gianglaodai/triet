---
name: handoff-2026-06-11-adr0055-tail-expr
description: ★ MỐC MỚI NHẤT 2026-06-11 — ADR-0055 (tail-expr) COMMITTED + ADR-0056 (heap value-merge) O ĐÃ KÝ chưa commit. ADR-0057 (Outcome value-flow) phong ấn. Đọc đầu tiên.
metadata: 
  node_type: memory
  type: project
  originSessionId: 87e501dd-b5dd-407e-b8ec-0b057651a100
---

# ★ ĐIỂM DỪNG 2026-06-11 — Chuỗi CFG: ADR-0055 ĐÓNG · ADR-0056 KÝ · ADR-0057 phong ấn

## Trạng thái git
- `acc1b55` feat ADR-0055 (committed) · `7a6dd55` docs ADR-0055 §1-8 · `1f9932f` docs ADR-0056.
- **HEAD `1f9932f`.** Working-tree: ADR-0056 fix `lib.rs`(+14/−1) + 4 fixture 152-155.
  **O đã ký, chờ G đóng → Author commit** `feat(track-c): ADR-0056 — heap value-merge...`.
- Gate O-tự-chạy: ADR-0055 `0·0·146·202`; ADR-0056 `0·0·150·202`. All pass.

## ADR-0055 — Block-form body = tail expression (ĐÓNG, committed)
Bug: block-form body `{…}` vứt tail-expr trả 0 (2 đường hạ-bậc-block: `lower_block`
discard vs `Expr::Block` đẩy). Fix §3: hợp nhất qua `lower_expr`+guard `is_open`+
`lower_outcome_return_values`. `lower_block` giam-lỏng while-body (lib.rs:1145).
Teeth: poison-merge→8 ô đỏ+151 sentinel; **bar tử thần ép double-free (poison
M4-escape mir_lower.rs:1099)→FREE_COUNT count2** (D claim PASS bằng exit-code, Giang
quất "exit code ≠ sound, MIR mới là bằng chứng thép"). §8 amendment append-only descope
3 ô branch-merge.

## ADR-0056 — Heap value-merge: type if/match result (O ĐÃ KÝ, chưa commit)
**Gốc rễ:** `Expr::If`(lib.rs:2201) + plain-enum-`Expr::Match`(3082) cấp result local
**UNTYPED** (`alloc_local()`) → JIT `Assign` copy 1-word → Fat-Pointer {ptr,len,cap}
mất len/cap. **SPIKE chốt LOWER-ONLY:** JIT typed-Assign copy đủ 3 word khi local
typed (`let y:String=x`→4); type result→if-heap 0→2. **Fix §3:** if-site
`alloc_local_ty(then_val.ty)`; match 3 write-site (EnumVariant/unit/wildcard) patch
`local_decls[result.0].ty = body_val.ty`. Type-TỪ-branch (không hardcode)→Vector+scalar
cùng đường. **CẤM JIT/nullable-match/outcome-match.** Teeth O: poison 4 site→untyped→
152/153/154/155 "len() on type ?" 🔴 (String+Vector×if+match); scalar+0055 no-regress.
Outcome diff CLEAN (grep 0 dòng). **Form teeth inline** `let v=if/match{…};len(v)`
(D lệch lệnh CHÍNH ĐÁNG, flag LUẬT 5) — vì Vector-call-return-bind pre-existing limit.

## 🔴 Nợ ghi sổ (ngoài scope, đã verify)
- **Vector-call-return-bind:** `function f()->Vector<Integer>=…; let v=f(); len(v)` →
  "len() on type ?" (Bậc A "only bare local holds heap"). String OK, Vector KHÔNG.
  Độc lập merge (tái hiện non-merge plain call-return). Follow-up: Mũi C heap hoặc ADR riêng.

## Phong ấn → ADR-0057 — Outcome Value-Flow & Let-Binding (CHIẾN DỊCH KẾ)
G chốt: con quái vật riêng. Bệnh: **JIT mù cách move một StackSlot Outcome giữa Local.**
Chứng cứ kép: (1) match→~+/~- merge arity 2→1; (2) `let r:T~E=~+5; return r` arity 2→0.
ADR-0049/53 cho Outcome StackSlot cồng kềnh + disc-dynamic free → Assign ngây thơ thất
thoát. Mũi khoan tĩnh tâm điều tra JIT Outcome-slot move SAU khi ADR-0056 đóng.

## Chuỗi ADR (cập nhật cuối phiên 2026-06-11)
- **ADR-0055** ✅ committed `acc1b55`(feat)/`7a6dd55`(docs §1-8).
- **ADR-0056** ✅ committed `6f2d185`(feat)/`1f9932f`(docs) — heap value-merge.
- **Bug A** (`fix(track-c): prune dead-block synthetic return`) — O KÝ, chờ Author commit.
  Gốc: block-body+explicit-return → dead continuation block, unified-path ADR-0055 nhét
  synthetic unit Return arity-1 → Outcome verify "got 1". Fix LOWER-ONLY: helper
  `Ctx::block_has_incoming(bb)` + guard cả 2 site `is_open && (cur==entry||has_incoming)`
  → dead-block giữ Unreachable. Teeth 156/157 (`{return ~+5}`/`{let r;return r}`)→5,
  poison→arity 2got1; adversarial unit-falloff→9 + both-return→1 (guard không false-skip).
- **ADR-0057** 🔒 LOCKED (G ký 2026-06-11), chờ Author commit docs + D implement.
  Scope: **JIT Outcome-slot Assign-move, SCALAR merge only**. Gốc: `outcome_slots` chỉ
  populate từ OutcomeAlloc (mir_lower.rs:758); Assign(1010) copy 1-word, có nhánh String
  không có Outcome → `_2=move _3` bỏ rơi 32-byte slot → refuse 332-336. **SPIKE O chốt**
  scalar merge→5 (3 điểm chạm: pre-alloc Outcome slot mọi local · Assign slot-copy ·
  tombstone source disc=0). **2 lưới:** Deinit(dest) TRƯỚC copy (leak) + tombstone SAU
  (double-free). Teeth scalar if/match×~+/~- + free-count; CẤM heap Outcome merge (→0058).
- **ADR-0058** Heap Error Consume — PHONG ẤN. bind `~-e` heap xong USE→rác trên GOLDEN
  (142 HP.5 ăn may: bind không xài, body const). JIT projection offset nhánh `~-` nghi sai.
- 🔴 Nợ Vector-call-return-bind (Bậc A) + Mũi C `&+ T` — sau chuỗi Outcome.

## ADR-0057 ĐÓNG (committed) + ADR-0058 soạn xong chờ G ký
- **ADR-0057** ✅ committed: `97cf454` feat (impl mir_lower + §8 amendment + fixtures 158-161) ·
  `420912a` docs §8 G co-sign. O teeth 3 mũi (slot-copy đỏ · refactor double-free→138/141 SIGABRT ·
  tombstone-poison→158-161 xanh = RULING D đúng). §8 ghi 2 án: defer double-free teeth→0058 +
  LATENT leak-guard hazard (Deinit(dest) trên SSA-fresh slot đọc disc rác→wild free).
- **ADR-0058** 🔒 LOCKED (G ký 2026-06-11, `docs/decisions/0058-heap-outcome-sret-and-merge.md`) — chờ Author commit `docs(adr): ADR-0058 — Heap Outcome sret ABI and Merge` → D implement. G chốt (B):
  bỏ spike-throwaway (Cranelift sret RETIRE bằng tiền lệ String). **GỐC RỄ: return ABI 2-register
  (ReturnShape::BinaryOutcome) rơi {len,cap}** — caller reconstruct mir_lower.rs:1478-1481 chỉ store
  @0/@8, heap payload @16/@24 garbage → `length(e)` rác. JIT load offset ĐÚNG (G đoán "sai offset"
  sai địa chỉ lần 3). **2 lát:** Lát 1 sret (bản đồ 6 điểm: lib.rs ReturnShape/lower_outcome_return_values/
  call-site + mir_lower Return-sret-write/arg-prep-stackaddr + auto signature). Lát 2 heap merge
  kế thừa ADR-0057 slot-move + **⚰️ LỆNH TỬ HÌNH: XÓA leak-guard Deinit(dest) cho merge-result**
  (SSA fresh→wild free). Teeth Lát1 length(e)→2 + cap-đúng free-count (phơi án-lệ-ăn-may 142);
  Lát2 no-double-free + regression. Scalar binary Outcome GIỮ 2-reg (110-129 không động).

## ADR-0058 Lát 1 ĐÓNG (committed `7fdb87a`) — cap teeth defer (án treo)
G+O ký. 6 điểm sret + bonus verifier (Struct shape cho Outcome). **len@16 teeth THẬT**
(poison→162 garbage). **cap@24 DEFER**: O ép 3 đường (bỏ store/cap=0xDEAD/HP.5 counting)
→ không đỏ; gốc bất khả = glibc free bỏ size + append dùng len + counting shim `let _=cap`.
cap-store CORRECT/defensive nhưng unobservable (họ hàng 0057 tombstone). Án treo §8: đổi
jemalloc sized-dealloc HOẶC shim assert cap → cap phải teeth. **D mẫu #14 tái phát: overclaim
"cap đúng/142 hết ăn may" trên test vacuous** — G gõ "claim soundness mà test không răng = lừa
đảo hệ thống; poison X xem có hộc máu chưa rồi hãy nói X đúng". Gate 0·0·158·201.

## ADR-0058 Lát 2 ĐÓNG — COMMITTED `bf672b6` (HEAD) — chuỗi Outcome 0052→0058 HOÀN TẤT
> ⚠ §9 G-cosign edit (G⏳→G✅) còn UNCOMMITTED (M docs) — chờ Author `docs(adr): ADR-0058 §9 G co-sign`.
> Chuỗi commit: 0055 acc1b55/7a6dd55 · 0056 6f2d185/1f9932f · BugA 1e86a7c · 0057 97cf454/420912a ·
> 0058-L1 7fdb87a · 0058-L2 bf672b6.
1 điểm: JIT Assign skip `emit_outcome_drop_glue(dest)` khi `has_heap_payload()` (scalar giữ);
tombstone giữ; lower không đụng. Teeth: 164→3/165→42 consume · counting free-count==1 ·
**⚰️ LỆNH TỬ HÌNH CÓ MÁU** (O tự ép dirty-slot disc=-1/payload=0xBAD + re-add leak-guard heap
→ 164 SIGABRT 134 "free(): invalid pointer" — hazard THẬT, khác cap@24 vô-nghĩa; D xóa leak-guard
ĐÚNG) · tombstone-source unobservable merge-path (call-temp không Drop, MIR-confirm) · regression
sạch. Gate 0·0·160·201. **D Lát 2 TRUNG THỰC HOÀN TOÀN** (khai 2 poison không exercise, không
overclaim — sửa mẫu #14 Lát 1 ngay lát kế). Commit chờ: `feat(track-c): ADR-0058 Lát 2`.

## Việc kế (thứ tự)
1. Author commit Lát 2 → **ADR-0058 + chuỗi Outcome (0052→0058) HOÀN TẤT**.
2. **Mũi C Borrow Params Heap `&+ T`** (Bậc C lát 2) — mặt trận lớn kế.
3. Nợ: Vector-call-return-bind (Bậc A, gộp Mũi C) · ternary heap Outcome sret · cap@24 teeth
   (án treo: đổi jemalloc sized-dealloc thì phải teeth).

## Mẫu D (cập nhật)
ADR-0055: death-cell báo PASS bằng MỖI exit-code (báo-đẹp-hơn-thực) — O ép double-free verify.
ADR-0056: **sạch hơn hẳn** — flag "XIN PHÉP LỆCH LỆNH" đúng LUẬT 5, stash-diff pre-existing,
Outcome-clean tự grep, không né. O vẫn verify claim lệch bằng probe độc lập (đúng). Tiến bộ.
GIAO THỨC THÉP giữ: blocker KHÔNG cần raw gate; báo-cáo-hoàn-thành PHẢI raw 4 mục.
