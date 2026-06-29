# TODO — Triết Backend (Track C)

Backlog sống cho chiến dịch kế. **Chỉ chứa việc CHƯA xong / phong-ấn.**
Ledger các phần ĐÃ đóng (per-step + commit-hash) → [`docs/TODO-ARCHIVE.md`](docs/TODO-ARCHIVE.md) + `git log` + `docs/decisions/`.

Mốc hiện tại: Gate `0·0·273·0`. **🔒🏁 CAPABILITY Ł3 (ADR-0069) NIÊM PHONG — COHERENCE VISION §8 HOÀN TẤT.** Đại số Ł3 khép kín ba chân: null(PA-3c) / logic(Trilean) / **capability**. ZST-token ngậm Ł3-Trit: Grant(+)/Ambient(0)/Deny(−) tĩnh zero-cost + Defer(Unknown) runtime trap `user(2)` fail-closed. Lát 0 `8b06a28` (ZST & cấm copy, 2-classifier defense-in-depth) · §amend-A `47eb283` (M1 receive-only) · Lát 2 `ca8272e` (possession E2212) · §5 `d84cd24` (mint-site lock) · Lát 3 `2dd4d5f` (Defer hook — O verify 4 răng, R-fail-closed boundary `≤` là tử huyệt) · Lát 4 demo fixture `278` (end-to-end →30). Mã mới: E2211 (mint non-grant) · E2212 (deny possession). **Mặt trận mandate ternary-first (G+Giang 2026-06-22) ĐÓNG.**

### ✅ ĐÓNG — **Path `.` → `::` + `use` + enum-variant ::** (ADR-0071 SEALED, G ký 2026-06-26)
~~Import `.`→`::`, dot-variant `Color.Red`, bare-variant~~ → **ĐÓNG SẬP TRỌN BỘ.** [ADR-0071](docs/decisions/0071-path-separator-and-module-import.md) SEALED, supersede ADR-0005 dot-path+Python-import. **AST pha lê: `::` tĩnh (path/type/enum-variant) · `.` động (field/method).**
- **Lát 1** (`4a7da96`): lexer `::`+`use`/giết import-from · `Item::Use{path,group}` (schema-first, brace-group) · resolver route 2-đường-cũ. 4 teeth (P-colon-token/longest-match/old-keyword/resolver).
- **Lát 2** (commit kế): `Color::Red`(+payload) via EnumLiteral + Pattern::EnumVariant{name:Some}. **Giết 3 cơ chế ngầm** (① pattern guess-hack ② expr in-scope-scan ③ 3 dot-hacks). **E1018 khai tử** (nguyên nhân chết). §2.A Variable=catch-all (đối xứng scalar). Bare un-qualified→E1002 mọi nơi; import-bound `use` chừa (env.lookup). Dọn dead `expr_resolutions` (rule #4, 21 caller). 5 teeth (O bóc tooth-vacuous P-catch-all + sharpen 293 scrutinee≠arm). Reading A (G phán giết-không-tha).

### ✅ ĐÓNG — **Partial-move & Struct-ZST** (ADR-0070 SEALED, G ký 2026-06-25)
~~`let v = hw.vga` field-level move-state~~ → **ĐÓNG SẬP.** [ADR-0070](docs/decisions/0070-partial-move-field-level-move-state.md) SEALED: borrow-checker mọc per-Place move-state (`partial_moves: BTreeMap<Local, BTreeSet<String>>`, union-merge monotone), capability ZST field sống đàng hoàng trong struct. **3 file:** `checker.rs` (Δ3 cho capability single-field, refuse heap E2423 giữ nguyên, 6 use-site invalidate), `lib.rs` (gate C allow-list capability + sizing 0B true-ZST per ADR-0069), `mir_lower.rs` (leaf-less non-copy struct → Drop no-op). **6 fixtures 279-284** (cap run→17 · use-after E2420 · whole-base E2420 · cfg-branch E2420 · heap-refused E2423 · mixed-struct offset run→105). O verify máu **5 teeth đỏ độc lập** (P-field-key · P-merge union→intersection · P-Δ3-heap no-panic · P-reread · Step3-JIT) + restore byte-identical. Schema §10 `kernel_main(hw)` destructure-move = canonical proof Hardware-Token nay CHẠY THẬT. **Nợ defer (No-Box):** partial-move field HEAP (đòi JIT dynamic drop-flag) · multi-level `hw.a.b` (conservative whole-base).

**🏁 TRỤC B LÁT 2 NO-BOX (ADR-0067) ĐÓNG SẬP TRỌN BỘ — 2a+2b+2b+ hàn kín, không rỉ một byte (O verify 4 răng đỏ độc lập + G co-sign).** **⚰️ nhát 2b+ ENUM-IN-STRUCT FIELD ĐÓNG** — cầu nối `collect_heap_leaves`↔`emit_enum_drop_glue`: `struct Wrapper{msg:Msg, tag:Integer}` (Msg.Text(String)) construct+move+drop sound, FREE_COUNT==1, bịt lỗ enum-kẹt-giữa-struct. **2b+-A** `LeafKind{Heap,Enum}` — collect push enum-leaf KHÔNG đệ quy (payload tag-dependent) · **2b+-B** tách `emit_enum_drop_glue_at(base_addr)` address-based, slot-based cũ→wrapper mỏng (2b byte-identical) · **2b+-C** Drop dispatch `Enum→emit_enum_drop_glue_at(copy_base_addr)` + Deinit zero payload@abs+8 KHÔNG disc@abs+0 · **2b+-D** gate `is_nested_enum` song song `is_nested_struct`. **death-line #2 (lỗ THẬT D đào):** lib.rs merged-arm `Struct|Enum→struct_map` rơi `_=>8` → enum field under-size 8B (đáng 32B) → slot under-size + offset sai → SIGSEGV; vá = dời `enum_layouts` lên TRƯỚC struct-fixpoint + tách arm `Enum→enum_map` (enum-sizing độc lập struct → ordering sound). 4 răng O cắm poison độc lập đỏ: death-line#2→SIGABRT134 · R-leak→Drop-Unsupported · ⚔R-wrong-variant(cross-wire)→2 fail · R-double-free-move→count≠1. Fixtures 269/270 + counting `enum_in_struct_counting`. Nợ latent (G ký để nguyên, surgical): `Nullable(Enum)` sizing arm dùng struct_map→8 (correct-now vì gate refuse Nullable(heap); đồng bộ khi mở ADR-0062 §6 heap-nullable-enum). **⚰️ Trục B Lát 2 nhát 2b TOP-LEVEL ENUM-HEAP ĐÓNG.** ADR-0067 §2b — `enum Msg{Text(String),Code(Integer),Empty}` top-level: construct + move + drop sound, free CHỈ payload variant active (tag-switch runtime). **D recon-trước bắt gap payload-layout** (analog 1a STEP-4, O verify+rule IN-SCOPE): **2b-0a** enum payload size heap-aware (String→24, lib.rs:603 — M-1 struct-fixup không chạm enum) · **2b-0b** fat-store String payload vào enum_slot (analog STEP-4). **2b-1** gate gỡ refuse leaf (EnumLiteral+EnumVariant, refuse struct-transitive/Nullable-heap) · **2b-2** `emit_enum_drop_glue` N-arm brif tag-switch (free active variant only) · **2b-3** Deinit tombstone CHỈ ptr@8 KHÔNG disc@0 (disc=0 variant hợp lệ, khác Outcome). 4 răng đỏ: R-enum-leak(0)·R-enum-double-free-move(2)·⚔R-enum-wrong-variant(cross-wire Text/Buf→shim sai)·R-enum-cap(rác≠5). Fixtures 266/267/268 + counting `enum_heap_payload_counting`. Vector/HashMap payload sound sẵn (thin 8B). **⚰️ Trục B Lát 2 nhát 2a NESTED-FLAT ĐÓNG (chờ ký).** ADR-0067 §2a — mở khóa `struct Outer{inner:Inner}` (Inner chứa heap, non-recursive): construct + move + drop sound mọi tầng lồng. **2a-1** M-2 nới `is_nested_struct` (bare Struct layout-resolve ALLOW; **CHỈ Struct KHÔNG Enum** — enum-payload=2b; Nullable-heap/box GIỮ refuse) · **2a-2** `collect_heap_leaves` đệ quy compile-time (depth-64→JitError, DÙNG CHUNG Drop+Deinit đối xứng Sinh-Tử, trả flat abs-offset) · **2a-3** move tái dùng 1b/1c (0 dòng). 3 răng đỏ: R-leak-nested (Unsupported refuse) · R-double-free-nested (FREE==2) · R-recursive-creep (stack-overflow SIGABRT). Fixtures 263/264/265 + **257 FLIP** (negative→positive, LUẬT 3 O-signoff) + counting `struct_nested_heap_counting` + unit `collect_heap_leaves_recursive`. **🏁 Trục B Lát 1 (heap-in-struct FLAT) HOÀN TẤT — 1a+1b+1c+1d, B8 thủng cho heap-leaf field.** Nhát 1d LOCK & SEAL (chờ ký): niêm phong Vector/HashMap field + struct use-after-move — **0 dòng compiler** (mechanism type-generic 1a/1b/1c đã phủ), thuần fixtures 260/261/262 + counting teeth. 3 răng đỏ độc lập: R-leak-vec (cut is_vec → vec leak 0) · R-leak-hmap · **ISOLATION SCALPEL** (poison riêng is_vec → Mixed{Vector,String}: vec=0 leak, str=1 sống — dispatch per-field-type) · R-e2420. Counting test serialize Mutex (3 test chung counter, gate chạy song song). **Lát 1 đủ:** heap-leaf field (String/Vector/HashMap) construct + whole-move (arg 1b + assign 1c) + inline drop-glue (1a KCN-1) + tombstone + use-after-move E2420 = sound + locked. **Nhát 1c:** assign-move `let q=p` true-move (D1 ctx_is_copy + D2 Deinit ATOMIC), LOWER-ONLY. **Nhát 1b:** arg-move (A copy_base_addr unify + B to_zero ctx_is_copy + C Deinit walk). **Nhát 1a:** M-1 sizing + M-2 B8-relax + KCN-1 + STEP 4 fat-store. **Trước đó:** ADR-0065 §12.8/§12.7 Nullable Aggregate Trục A. **Kế (G phán 2026-06-25):** **⚖️ CAPABILITY Ł3 (ADR-0016/0017/0018) — MẶT TRẬN CHIẾN LƯỢC mandate balanced-ternary-first.** Coherence (VISION §8) hiện 2/3 (null Ł3 ✓ + logic Ł3 ✓, capability Ł3 = 0); bơm đại số Ł3 xuyên capability để khớp hạ tầng Drop-Glue/No-Box vừa xây. ADR-0068 Box/recursive `&+` **G HOÃN** (không đâm bãi mìn pointer-heap khi lõi capability còn què). **Nợ defer No-Box (chưa use-case):** payload-struct-chứa-heap (`enum{Rec(Wrapper)}` — collect đệ quy TRONG arm) · 2c true-recursive + box (ADR-0068, cần allocator + iterative-drop). **Nợ Lát 1.x:** partial-move `let s=p.name` / match-arm bind heap payload (blocked read-side gap String→Unknown) · field-reassign.

### 🟡 Sổ nợ Tech-Debt Hạ tầng — counting-test parallel isolation
Các test free-count (`nullable_map_heap_output_counting`, `vector_nullable_drop_counting`, …) dùng process-global `AtomicUsize` + no-mangle shim → flake hiếm dưới `cargo test --workspace` tải nặng (đo: `map_vector_output_freed_once` đỏ 1 lần, xanh 6+ lần isolation/release/re-run). Cần `--test-threads=1` hoặc subprocess isolation (hạ tầng N7 đã có cho một số). KHÔNG chặn nhát 1a (code orthogonal). Ghi nợ theo lệnh G.

---

## 🟢 BACKLOG MỞ

### ⚖️ KHẮC ĐÁ — Capability Ł3 = nhiệm vụ chiến lược cốt lõi SAU Trục B (G+Giang 2026-06-22)
Quyết định **ternary-first** ([VISION §1/§5](VISION.md)) neo giá trị Triết vào **coherence**
([VISION §8](VISION.md)): một đại số Ł3 duy nhất xuyên null / logic / **capability**. Coherence
mới xây **2/3** — `T?` ✅ + Ł3/K3 ✅, **capability Ł3 = 0** (ADR-0016/0017/0018 thiết kế còn
sống, hiện thực đã xóa cùng compiler cũ). Đây là **chân độc-nhất nhất**; thiếu nó coherence
chỉ là giấy → Triết tụt xuống toy-language chắp vá.
- [ ] **Rebuild capability runtime** (Trit `-1` deny / `0` ambient / `+1` grant + Ł3 `Unknown`
      resolved bởi runtime policy) — mở **NGAY SAU khi Trục B kết thúc**. ADR-0016/0017/0018 làm
      móng thiết kế; hiện thực mới trên MIR/JIT. KHÔNG còn là "làm khi tới lượt". Chi tiết:
      [ROADMAP §ƯU TIÊN CHIẾN LƯỢC SAU TRỤC B](ROADMAP.md).

### 🔴 Chiến dịch CFG Tail-Expression — ưu tiên 1 (soundness)
Wire nốt ADR-0055: block tail-expr gánh giá trị cuối hàm.
return-scope đã khóa (ADR-0020 §3.8): `return` = early-exit + cọc-tiêu-mode, KHÔNG phải throw.

- [x] **ĐẬP TRƯỚC TIÊN (soundness):** 🔴 expr-body fat-struct return không route sret → **SIGILL 132**. `4d51faa`
      Free fn `f() -> Point = Point{...}` emit `Return(struct)` by-value thay vì ghi sret slot;
      block-body (`{ return ... }`) chạy đúng. Crash/soundness hole có sẵn, độc lập trait/nợ#2.
      *Soundness trước syntax (G 2026-06-17).* — ADR-0055 lát 1: helper SSOT `emit_struct_sret_copy`
      route tail-Return qua sret y hệt Stmt::Return; teeth 182/183/184 poison→SIGILL.
- [x] Wire tail-expr gánh giá trị cuối hàm → giảm `return` cuối thân (happy-path). `a0eff46`
      ADR-0055 lát 2 A-hẹp: phần lớn ĐÃ wire bởi ADR-0055+0056/0057/0058 (probe 20+ dạng:
      literal/expr/if/match/nested/while-tail/struct/heap-if/heap-match/outcome/nullable-widen
      đều chạy). Còn đúng MỘT bất đối xứng tail: `= ~0` báo lỗi trong khi `return ~0` chạy →
      mirror null-~0 special-case sang tail-path. Fixtures 185-188. Gap #2 (`{ ~0 }`/if-arm)
      đẩy Heap-Nullable (fail y hệt ở return/let, không phải tail-asymmetry).

### 🟣 Chiến dịch Heap-Nullable — saga ~5 lát
`T?` cho `T` heap (String/Vector/HashMap/Struct/Enum). Hiện **GATE ở LOWER** bằng
`MirError::HeapNullableNotLowered` (`Body::verify()` refuse — KHÔNG ở typecheck).

**Ruling β (G ký 2026-06-18):** gate ở LOWER, KHÔNG typecheck — vì stdlib khai
heap-nullable làm API (`env.get`/`path.parent`/`text.from_bytes`/`fs.read -> String?`).
Declaration vô hại (stub `= ~0`); chỉ *compilation* mới miscompile (Bậc A nullable =
single-i64 sentinel, không chứa nổi fat-pointer 24B). Nếu sau cần chặn sớm ở Pass-1 →
Option-2 (gate free-fn `resolve_type_expr_with_params`, đổi chữ ký + dedup).

- [ ] 1. **ADR repr — (a) ptr-sentinel** (G nghiêng): slot `{ptr,len,cap}`, `ptr == NULL_SENTINEL` = null; null-check project `.ptr`, không so cả slot.
- [ ] 2. Widening `String → String?` + `~0` materialize ptr-sentinel.
- [ ] 3. JIT conditional Drop (`if ptr != SENTINEL → drop payload`).
- [ ] 4. Elvis `?:` + match `~+/~0` heap (project `.ptr`, move payload).
- [ ] 5. `?+>` map/flatMap heap (unwrap move + Deinit/tombstone tránh double-free).
- [ ] Gỡ gate `HeapNullableNotLowered` (+ `find_heap_nullable`/`is_scalar_nullable_payload` helper ở triet-mir) khi móng landed.
- [x] **Gap #2 — expected-type propagation (ADR-0072 🔒 SEALED 2026-06-27, 3-slice).**
  - [x] **Slice 1** `c9a46e6` — `lower_expr` thêm param `expected: Option<&MirType>`, 61 site=`None`, byte-identical (O verify MIR-diff rỗng toàn corpus).
  - [x] **Slice 2** `2c900fb` — leaf-consumer (`OutcomeConstructor`/`NullLiteral`) đọc `expected`; wire 4 nguồn (function-body/return/let-init/struct-field); đập 3 Bug-B redirect. **Mở `T?`-return scalar** (303 Integer?, 305 let-Integer?-trong-Outcome-fn). Fallback §2.5 chuyển-tiếp (gỡ ở Slice 3). O verify: gate 0·0·299·0 + byte-identical 297 cũ + 2 poison `OutcomeAlloc on non-Outcome` đỏ + defense-in-depth 2 guard.
  - [x] **Slice 3 (kết liễu)** — transparent forwarding `expected` xuống Block tail + If then/else + **13 arm-body** (8 WO + wc.body×4 + arm_for_present×1) + helper `lower_value_keyed_match`; gỡ sạch fallback §2.5; **nhổ `c.sig.return_type` khỏi input constructor** (chỉ còn 4 nguồn return-position hợp pháp: body-tail/return-non-sret/return-sret/Expr::Return); extract `emit_outcome_zero`. Mở `~+`/`~0` trong if/match/block-arm context≠sig (306/307/308) + **309 negative khóa luật untyped-ctor-bị-từ-chối** + 157 annotated. O verify máu: gate 0·0·303·0 + byte-identical 299/299 (worktree baseline) + 3 poison R-fwd đỏ + grep C sạch. Diagnostic tổng quát hóa (hết nói "~0 null" cho ~+/~-).
- [x] ✅ **WO-0073 — heap-nullable-RETURN drop-glue (cờ đỏ ADR-0072 §6 NHỔ TẬN GỐC, G ký 2026-06-29).** `3738eb5` (push main). File `heap_nullable_return_present_counting.rs` — **7 cell counting-tooth** cho `~+ <heap>`-present return (String?/Vector?/HashMap?), 2 shape: **expr-body** (A/B/C/D: `= ~+ x` + match-consume) + **named-local explicit-return** (E/F/G: `{let s; return ~+ s;}`). O verify máu độc lập (cp-snapshot, KHÔNG git checkout): **leak-tooth** (elide drop-glue) → 7/7 RED FREE→0; **double-free-tooth** (gỡ M4 1982) → E/F/G RED FREE→2, A/B/C/D INERT(1). **Sự thật kiến trúc** (G bắt sửa doc 2 vòng): expr-body = lowerer **escape-by-omission** (callee KHÔNG emit Drop; O đo: bỏ guard ptr==0 + M4 off → tổng free-call==1) → double-free bất khả → M4-tooth INERT; named-local = `flush_all_for_return` emit Drop(s), **M4 load-bearing**. Cả leak lẫn double-free đóng. **Bài học O: verify-don't-trust cắt cả WO của chính O** (spec double-free-tooth ban đầu sai — tưởng M4 gác expr-body; D bắt, nới scope +3 cell named-local).

### 🟣 ADR-0065 Nullable Aggregate (`Enum?` & `Struct?`) — 🔒 LOCKED, 2 lát
Bất biến hợp nhất: `tag_cell == i64::MIN ⟺ null`. Rào **B8 §4**: aggregate-nullable CHỈ
chứa Copy field/payload — KHÔNG drop-glue/alloc/free. Value-model i64 KHÔNG đụng.

- [x] **Lát 1 — `Enum?` (disc-sentinel niche, 0 byte).** `1748510` (feat) + `e9bd3e0` (§9.1).
      disc@0 == i64::MIN = null (discriminant thật ∈ {0,1,2,…}); widening no-op. 5 delta:
      A gate(triet-mir 1399) · B slot-alloc(triet-jit) · C walk_proj unwrap · E result-retype
      (lower, idiom ADR-0056) · F `~0` store disc@0. Fixtures 225-230 (present payload-less/
      payload, ~0 null, Elvis, widening, B8 heap refuse). Poison E/B→SIGSEGV139, F→Trap132.
- [x] **Lát 2 — `Struct?` (tag 8B, Phương án A, β).** slot `{tag@0:i64, fields@8…}`,
      total = struct.total_size + 8. tag@0 == i64::MIN = null / +1 = present. **6 delta:**
      Delta 0 lowerer (Struct→Struct? widening → fresh local + Assign, KHÔNG retype in-place —
      §9.2, recon-miss của O vá in-scope) · 1 gate += `Struct(_)` · 2 slot-alloc +8 (skip
      sret/param/String) · 3 walk +8 (helper `nullable_struct_base_offset`) · 4a widening
      tag=1 + copy→+8 · 4b **β** whole-slot N+8 tag-first (`T?→T?` propagate, KHÔNG refuse).
      Lệch-lệnh chuẩn thuận: `is_aggregate` + slot-loop skip `Struct("String")` (borrowck builder
      type String-local thành `Struct("String")` slot-less → tránh deref SIGSEGV). Fixtures
      231-237; **237 = teeth tag-store** (reassign-widen-over-null, slot tái-dùng MIN). O verify
      máu P1-P5 RED: P1 walk→231/234 sai · P2 4a-1word→SIGILL · P3 tag-store→237 đỏ (231 tươi
      KHÔNG bắt) · P4 4b-tag→234/235 đỏ · P5 B8 gate→236+180. Gate `0·0·232·0`.

### 🟢 Perf — ADR-0044 §iii (không chặn)
- [ ] **D1 Codegen opt range-check 1-instruction:** `(val−MIN) >ᵤ 2M` unsigned-sub trick + fallback `bor` gộp 2 icmp. Cắt nửa instruction mỗi Add/Sub.
- [ ] **D2 Constant folding pass:** toán hạng const in-range → tính compile-time, bỏ trap block.

### Khác
- [ ] **D2 HashMap reject-MIN** (ADR-0043 Q6): `insert` reject `i64::MIN` — GIỮ defense-in-depth.
- [ ] **gate.sh giòn — exit 1 giả khi clippy=0** (G ghi sổ 2026-06-18): dòng cuối `clippy … | grep -- "-->" | sort -u | wc -l` dưới `set -o pipefail` → clippy 0 warning ⇒ grep no-match ⇒ exit 1 ⇒ script exit 1 dù output 4 dòng sạch. Đếm log lởm. Vá ở chiến dịch dọn CI (vd `grep -c` hoặc `|| true` có kiểm). KHÔNG ảnh hưởng soundness C-track.

---

## 🔒 PHONG ẤN Nhóm E — YAGNI (G defer 2026-06-10)

Mở khi có tiền đề (value-model thoát single-i64 / producer thật). KHÔNG build tạm.

- Native struct multi-field layout — cần value-model + ADR byte-size mapping + fixture Trit/Tryte-in-struct. Spike: `spec/plans/phase10-native-struct-layout.md`.
- Packed Outcome ABI — đi kèm Native.
- Multi-value return (>1 value) — cần producer thật (Outcome/tuple). Spike: `spec/plans/phase11-c5-multivalue-return.md`.
- `&+` / `&-` borrow forms — phong ấn (ADR-0059).

---

## ✅ ĐÃ ĐÓNG — tóm tắt (chi tiết: [`docs/TODO-ARCHIVE.md`](docs/TODO-ARCHIVE.md) + git + ADR)

- **Phase 4 Aggregate:** struct/enum/String/Vector/HashMap (ADR-0042 B7-lift · 0043 HashMap · 0060 nested `a.b.c`).
- **Phase 5 Bậc C borrow:** ADR-0044 trap-overflow · 0045/0046 `&0` borrow + return-elision · 0047 read-ops · 0048 mutable-borrow · 0059 `&0` heap.
- **Bậc D Fat-Pointer ABI:** ADR-0049 (param/return fat-String sret, slot = chân lý duy nhất).
- **Outcome `T~E`:** ADR-0050 MirType · 0051 borrowck-merge · 0052 producer · 0055→0058 CFG/heap sret.
- **Trait Tier 1:** ADR-0061 (`594abd9`) — dispatch + Comparable (ADR-0038) + match-on-Trit.
- **Nullable `?+>` scalar:** ADR-0039 (`73532b4`) — map/flatMap, `?->` → E1046.
- **Chiến dịch Trả Nợ** (2026-06-09→10): A (3 bom) · B1 type-system (ADR-0050) · B2 borrowck-merge (ADR-0051) · C1/C2/C6 feature-gaps · OP Outcome-producer.
- **Chiến dịch Cleanup "Đại Hốt Xà Bần"** (2026-06-17→18): LoweringInput refactor · fat-return trait sret · heap-nullable LOWER-gate · return-scope ADR-0020 §3.8.
