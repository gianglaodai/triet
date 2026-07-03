# TODO — Triết Backend (Track C)

Backlog sống cho chiến dịch kế. **Chỉ chứa việc CHƯA xong / phong-ấn.**
Ledger các phần ĐÃ đóng (per-step + commit-hash) → [`docs/TODO-ARCHIVE.md`](docs/TODO-ARCHIVE.md) + `git log` + `docs/decisions/`.

Mốc hiện tại: Gate `0·0·331·0`, origin/main = `37a0723`, synced sạch. **🏁 CHIẾN DỊCH READ-SIDE (CỤM A) KHÓA SỔ — G ký 2026-07-04, `37a0723`.** ✅ **A1 get-borrow generic-V**: 6 overload `get` V=Vector/HashMap (Int-key + String-key) → `(&0 V)?`; §AMEND-1 stride-conditional deref giữ invariant `&0 V` bit-for-bit dù local hay get_ref (thin→body_ptr, fat String→cell). ✅ **P0 BÁO ĐỘNG ĐỎ**: pre-existing String-key read SIGSEGV (latent từ ADR-0080 — get/get_ref/contains nhận `&0 HashMap` Reference-wrapped, key_stride chỉ bóc Nullable → default 8 → String-key 24B marshal by-value → hash rác → SIGSEGV) VÁ = unwrap `MirType::Reference` trước match HashMap; cắm cờ 335. ❄️ **A2 get-borrow-mutable** (ADR-0081): FROZEN → Cụm D (functional push/insert ⇒ `&0 mutable V` vacuous khi chưa có deref-assign). 🚫 **V=Nullable**: REFUSE/defer (lowerer chưa match `&0 Nullable`). O verify máu poison→RED độc lập (POISON-1 stride-deref→garbage · POISON-P0 Reference-unwrap→SIGSEGV 139 · overload-break 336/337→E1041); 5 fixture 333-337; restore md5 khớp. **⚠️ Kỷ luật D**: cảnh cáo thép "lần cuối dung túng ném-API-không-test" (D bẻ lệnh giữ String-key overload + thiếu fixture heap-value → G ép bổ sung 336/337). **🔑 ADR-0080 KM-P1a (key-typed `HashMap<String,V>` BACKEND) LANDED — Author+O+G ký 2026-07-03, `c003a5f`. Mechanism sound & sleeping (hand-built MIR + counting): slot `key_stride` 24B fat · `__triet_string_hash` FNV-1a + `hashmap_key_hash/eq` dispatch theo key_stride · key drop-obligation §AMEND-1 out-param ABI (D.1 map-drop-loop / D.2 insert-dup `is_update_out` / D.5 remove-resident `key_out_ptr` — free ở JIT call-site registry-routed → counting-testable, KHÔNG free trong thân shim). O verify 5 teeth poison→RED độc lập (map-drop-leak 1→0 · update-leak 2→1 · remove-leak 1→0 · content-hash alloc-indep cap=1_000_003 · rehash key-stride→NULL_SENTINEL). Chi tiết mục "🔨 ĐANG MỞ" dưới.** **🩹 BUG-E (Outcome-param ABI mis-tag + `~->` early-return heap double-free) ĐÓNG — O+G ký 2026-07-03, 2 WO liên tiếp (`ddb7841` param-ABI copy-in gap + `818602c` early-return heap-payload double-free, 3 site). Chi tiết đầy đủ ở mục "✅ ĐÓNG — Bug-E" bên dưới.** **🩸 GET-BORROW HEAP VALUE (ADR-0079) IMPLEMENTED/CLOSED — G ký 2026-07-01.** Read-side container khép: `get(&0 container,k)→(&0 V)?` zero-copy borrow (P1 V=String). Borrowck whole-container loan (U2 PropagatedLoan builtin + U3 mutate-while-borrowed E2440 cho consume insert/push + in-place remove/pop); JIT shim trả con-trỏ-slot (0 alloc), not-found→NULL_SENTINEL. O verify máu: content-read `length`→2/5 · source-level E2440 · 5 borrowck teeth poison. Slice A `a970540`. **🏁 TYPED HEAP-CONTAINER P1 ĐÓNG TRỌN — ADR-0077 (Vector) + ADR-0078 (HashMap) SEALED, G ký 2026-07-01.** `Vector<T>` + `HashMap<Integer,V>` (T/V = built-in heap: String/Vector/HashMap/Nullable) chạy sound end-to-end source qua JIT real-allocator: construct + push/insert(Move) + pop/remove(move-out `T?`/`V?`) + drop — không rỉ một byte. Element/value-SIZE = hằng compile-time (tách-tầng khỏi native-layout). **KEY-typed (`HashMap<String,V>`) + UserStruct value + get-clone/borrow heap value = defer Tầng 2/P2.** HM-P1b 3 vòng O-reject ép chân lý: garbage `lower_type` bỏ value-arg → vacuous-tooth (literal-temp no-drop-obligation) → named-local poison `arg_consumes[2]=false`→SIGABRT 134 ĐỎ. **🏁 KỶ NGUYÊN NULLABLE KHÉP HOÀN TOÀN — ADR-0076 SEALED (heap-`T?` trong aggregate field/payload, giao điểm B8 cuối).** Lát đơn atomic `6327890`: 5 mũi (gate-lift + field-layout sentinel + drop-arm `collect_heap_leaves` + construct/widen + borrowck). Cổ tức PA-3c: conditional-drop = sentinel-no-op, KHÔNG `brif`. O vồ double-free CASE B (match-present-bind-move → SIGABRT 134, borrowck im) → D đóng STATIC tag-niche-tombstone (KHÔNG dynamic-flag). O verify máu 3 tooth (Deinit-after-bind 134 · sinh-tử `is_copy(Nullable(heap))==false` 7-leak · drop-arm). `let s=b.s`→E2423 (Nợ defer giữ). **🔒🏁 CAPABILITY Ł3 (ADR-0069) NIÊM PHONG — COHERENCE VISION §8 HOÀN TẤT.** Đại số Ł3 khép kín ba chân: null(PA-3c) / logic(Trilean) / **capability**. ZST-token ngậm Ł3-Trit: Grant(+)/Ambient(0)/Deny(−) tĩnh zero-cost + Defer(Unknown) runtime trap `user(2)` fail-closed. Lát 0 `8b06a28` (ZST & cấm copy, 2-classifier defense-in-depth) · §amend-A `47eb283` (M1 receive-only) · Lát 2 `ca8272e` (possession E2212) · §5 `d84cd24` (mint-site lock) · Lát 3 `2dd4d5f` (Defer hook — O verify 4 răng, R-fail-closed boundary `≤` là tử huyệt) · Lát 4 demo fixture `278` (end-to-end →30). Mã mới: E2211 (mint non-grant) · E2212 (deny possession). **Mặt trận mandate ternary-first (G+Giang 2026-06-22) ĐÓNG.** **🏁 Heap-aggregate cluster ĐÓNG TRỌN** — ADR-0070 partial-move + ADR-0071 import `::` + WO-0073/74/75 (heap-nullable-return drop-glue · enum-field move-out · multi-level `h.inner.x` projection-path), origin/main = `0947482`.

### ✅ ĐÓNG — **Heap-Nullable trong aggregate field/payload** (ADR-0076 SEALED, G ký 2026-06-29)
~~`struct S{x:String?}` / `enum Bag{Has(String?)}` refused (rào B8)~~ → **ĐÓNG SẬP — KỶ NGUYÊN NULLABLE KHÉP.** [ADR-0076](docs/decisions/0076-heap-nullable-aggregate-field.md) SEALED: heap-`T?` (String?/Vector?/HashMap?) ở field/payload nay construct + whole-move + drop sound. Lát đơn atomic `6327890`. **5 mũi:** ① gate `is_field_payload_lowerable` +`is_any_heap()` · ② field-layout sentinel @offset (String?=24B fat, Vector?/HashMap?=8B handle, null=NULL_SENTINEL) · ③ `collect_heap_leaves` arm `Nullable(heap)`→drop vô điều kiện reuse shim · ④ construct/widen store fat-ptr/sentinel @offset · ⑤ borrowck Move-classify + E2423 move-out refuse. **Cổ tức PA-3c:** conditional-drop = sentinel-no-op (ptr@offset ∈ {ptr→free, sentinel→no-op, 0→no-op}), 0 `brif` Cranelift. **O vồ double-free CASE B** (match-present-bind-move heap-aggregate → SIGABRT 134, borrowck im, MỚI do gate-lift) → D đóng STATIC tag-niche-tombstone (KHÔNG dynamic-drop-flag). O verify máu 3 tooth đỏ độc lập (Deinit-after-bind→134 ·3 biến thể · sinh-tử `is_copy(Nullable(heap))==false`→7 counting LEAK · drop-arm→leak). Fixtures FLIP 180/230/236/255→run + 311/312 present-bind + 310→E2423 + counting `heap_nullable_field_counting` 9/9. **Nợ defer giữ (ADR-0070):** partial-heap-field-move-out `let s=b.s` (đòi dynamic-drop-flag).

### ✅ ĐÓNG — **Path `.` → `::` + `use` + enum-variant ::** (ADR-0071 SEALED, G ký 2026-06-26)
~~Import `.`→`::`, dot-variant `Color.Red`, bare-variant~~ → **ĐÓNG SẬP TRỌN BỘ.** [ADR-0071](docs/decisions/0071-path-separator-and-module-import.md) SEALED, supersede ADR-0005 dot-path+Python-import. **AST pha lê: `::` tĩnh (path/type/enum-variant) · `.` động (field/method).**
- **Lát 1** (`4a7da96`): lexer `::`+`use`/giết import-from · `Item::Use{path,group}` (schema-first, brace-group) · resolver route 2-đường-cũ. 4 teeth (P-colon-token/longest-match/old-keyword/resolver).
- **Lát 2** (commit kế): `Color::Red`(+payload) via EnumLiteral + Pattern::EnumVariant{name:Some}. **Giết 3 cơ chế ngầm** (① pattern guess-hack ② expr in-scope-scan ③ 3 dot-hacks). **E1018 khai tử** (nguyên nhân chết). §2.A Variable=catch-all (đối xứng scalar). Bare un-qualified→E1002 mọi nơi; import-bound `use` chừa (env.lookup). Dọn dead `expr_resolutions` (rule #4, 21 caller). 5 teeth (O bóc tooth-vacuous P-catch-all + sharpen 293 scrutinee≠arm). Reading A (G phán giết-không-tha).

### ✅ ĐÓNG — **Partial-move & Struct-ZST** (ADR-0070 SEALED, G ký 2026-06-25)
~~`let v = hw.vga` field-level move-state~~ → **ĐÓNG SẬP.** [ADR-0070](docs/decisions/0070-partial-move-field-level-move-state.md) SEALED: borrow-checker mọc per-Place move-state (`partial_moves: BTreeMap<Local, BTreeSet<String>>`, union-merge monotone), capability ZST field sống đàng hoàng trong struct. **3 file:** `checker.rs` (Δ3 cho capability single-field, refuse heap E2423 giữ nguyên, 6 use-site invalidate), `lib.rs` (gate C allow-list capability + sizing 0B true-ZST per ADR-0069), `mir_lower.rs` (leaf-less non-copy struct → Drop no-op). **6 fixtures 279-284** (cap run→17 · use-after E2420 · whole-base E2420 · cfg-branch E2420 · heap-refused E2423 · mixed-struct offset run→105). O verify máu **5 teeth đỏ độc lập** (P-field-key · P-merge union→intersection · P-Δ3-heap no-panic · P-reread · Step3-JIT) + restore byte-identical. Schema §10 `kernel_main(hw)` destructure-move = canonical proof Hardware-Token nay CHẠY THẬT. **Nợ defer (No-Box):** partial-move field HEAP (đòi JIT dynamic drop-flag) · multi-level `hw.a.b` (conservative whole-base).

### ✅ ĐÓNG — **Capability Ł3** (ADR-0069 SEALED, G ký 2026-06-25)
~~Capability runtime Ł3 4-state~~ → **NIÊM PHONG.** Capability Ł3 — đã NIÊM PHONG bởi [ADR-0069](docs/decisions/0069-zst-capability-token-luk3.md) (synthesis ZST-token ngậm Ł3-Trit, CHÔN ADR-0016/0017/0018). Chi tiết: header trên.

**🏁 TRỤC B LÁT 2 NO-BOX (ADR-0067) ĐÓNG SẬP TRỌN BỘ — 2a+2b+2b+ hàn kín, không rỉ một byte (O verify 4 răng đỏ độc lập + G co-sign).** **⚰️ nhát 2b+ ENUM-IN-STRUCT FIELD ĐÓNG** — cầu nối `collect_heap_leaves`↔`emit_enum_drop_glue`: `struct Wrapper{msg:Msg, tag:Integer}` (Msg.Text(String)) construct+move+drop sound, FREE_COUNT==1, bịt lỗ enum-kẹt-giữa-struct. **2b+-A** `LeafKind{Heap,Enum}` — collect push enum-leaf KHÔNG đệ quy (payload tag-dependent) · **2b+-B** tách `emit_enum_drop_glue_at(base_addr)` address-based, slot-based cũ→wrapper mỏng (2b byte-identical) · **2b+-C** Drop dispatch `Enum→emit_enum_drop_glue_at(copy_base_addr)` + Deinit zero payload@abs+8 KHÔNG disc@abs+0 · **2b+-D** gate `is_nested_enum` song song `is_nested_struct`. **death-line #2 (lỗ THẬT D đào):** lib.rs merged-arm `Struct|Enum→struct_map` rơi `_=>8` → enum field under-size 8B (đáng 32B) → slot under-size + offset sai → SIGSEGV; vá = dời `enum_layouts` lên TRƯỚC struct-fixpoint + tách arm `Enum→enum_map` (enum-sizing độc lập struct → ordering sound). 4 răng O cắm poison độc lập đỏ: death-line#2→SIGABRT134 · R-leak→Drop-Unsupported · ⚔R-wrong-variant(cross-wire)→2 fail · R-double-free-move→count≠1. Fixtures 269/270 + counting `enum_in_struct_counting`. Nợ latent (G ký để nguyên, surgical): `Nullable(Enum)` sizing arm dùng struct_map→8 (correct-now vì gate refuse Nullable(heap); đồng bộ khi mở ADR-0062 §6 heap-nullable-enum). **⚰️ Trục B Lát 2 nhát 2b TOP-LEVEL ENUM-HEAP ĐÓNG.** ADR-0067 §2b — `enum Msg{Text(String),Code(Integer),Empty}` top-level: construct + move + drop sound, free CHỈ payload variant active (tag-switch runtime). **D recon-trước bắt gap payload-layout** (analog 1a STEP-4, O verify+rule IN-SCOPE): **2b-0a** enum payload size heap-aware (String→24, lib.rs:603 — M-1 struct-fixup không chạm enum) · **2b-0b** fat-store String payload vào enum_slot (analog STEP-4). **2b-1** gate gỡ refuse leaf (EnumLiteral+EnumVariant, refuse struct-transitive/Nullable-heap) · **2b-2** `emit_enum_drop_glue` N-arm brif tag-switch (free active variant only) · **2b-3** Deinit tombstone CHỈ ptr@8 KHÔNG disc@0 (disc=0 variant hợp lệ, khác Outcome). 4 răng đỏ: R-enum-leak(0)·R-enum-double-free-move(2)·⚔R-enum-wrong-variant(cross-wire Text/Buf→shim sai)·R-enum-cap(rác≠5). Fixtures 266/267/268 + counting `enum_heap_payload_counting`. Vector/HashMap payload sound sẵn (thin 8B). **⚰️ Trục B Lát 2 nhát 2a NESTED-FLAT ĐÓNG (chờ ký).** ADR-0067 §2a — mở khóa `struct Outer{inner:Inner}` (Inner chứa heap, non-recursive): construct + move + drop sound mọi tầng lồng. **2a-1** M-2 nới `is_nested_struct` (bare Struct layout-resolve ALLOW; **CHỈ Struct KHÔNG Enum** — enum-payload=2b; Nullable-heap/box GIỮ refuse) · **2a-2** `collect_heap_leaves` đệ quy compile-time (depth-64→JitError, DÙNG CHUNG Drop+Deinit đối xứng Sinh-Tử, trả flat abs-offset) · **2a-3** move tái dùng 1b/1c (0 dòng). 3 răng đỏ: R-leak-nested (Unsupported refuse) · R-double-free-nested (FREE==2) · R-recursive-creep (stack-overflow SIGABRT). Fixtures 263/264/265 + **257 FLIP** (negative→positive, LUẬT 3 O-signoff) + counting `struct_nested_heap_counting` + unit `collect_heap_leaves_recursive`. **🏁 Trục B Lát 1 (heap-in-struct FLAT) HOÀN TẤT — 1a+1b+1c+1d, B8 thủng cho heap-leaf field.** Nhát 1d LOCK & SEAL (chờ ký): niêm phong Vector/HashMap field + struct use-after-move — **0 dòng compiler** (mechanism type-generic 1a/1b/1c đã phủ), thuần fixtures 260/261/262 + counting teeth. 3 răng đỏ độc lập: R-leak-vec (cut is_vec → vec leak 0) · R-leak-hmap · **ISOLATION SCALPEL** (poison riêng is_vec → Mixed{Vector,String}: vec=0 leak, str=1 sống — dispatch per-field-type) · R-e2420. Counting test serialize Mutex (3 test chung counter, gate chạy song song). **Lát 1 đủ:** heap-leaf field (String/Vector/HashMap) construct + whole-move (arg 1b + assign 1c) + inline drop-glue (1a KCN-1) + tombstone + use-after-move E2420 = sound + locked. **Nhát 1c:** assign-move `let q=p` true-move (D1 ctx_is_copy + D2 Deinit ATOMIC), LOWER-ONLY. **Nhát 1b:** arg-move (A copy_base_addr unify + B to_zero ctx_is_copy + C Deinit walk). **Nhát 1a:** M-1 sizing + M-2 B8-relax + KCN-1 + STEP 4 fat-store. **Trước đó:** ADR-0065 §12.8/§12.7 Nullable Aggregate Trục A. ADR-0068 Box/recursive `&+` **G HOÃN** (chưa có allocator + iterative-drop — xem Nợ defer). **Nợ defer No-Box (chưa use-case):** payload-struct-chứa-heap (`enum{Rec(Wrapper)}` — collect đệ quy TRONG arm) · 2c true-recursive + box (ADR-0068, cần allocator + iterative-drop). **Nợ Lát 1.x:** partial-move `let s=p.name` / match-arm bind heap payload (blocked read-side gap String→Unknown) · field-reassign.

### 🟡 Sổ nợ Tech-Debt Hạ tầng — counting-test parallel isolation
Các test free-count (`nullable_map_heap_output_counting`, `vector_nullable_drop_counting`, …) dùng process-global `AtomicUsize` + no-mangle shim → flake hiếm dưới `cargo test --workspace` tải nặng (đo: `map_vector_output_freed_once` đỏ 1 lần, xanh 6+ lần isolation/release/re-run). Cần `--test-threads=1` hoặc subprocess isolation (hạ tầng N7 đã có cho một số). KHÔNG chặn nhát 1a (code orthogonal). Ghi nợ theo lệnh G.

---

## 🟢 BACKLOG MỞ

### 🔨 ĐANG MỞ — Key-typed `HashMap<String,V>` (ADR-0080 + §AMEND-1, Author+O+G ký 2026-07-03)
Key mang content hash/eq + drop-obligation. ADR mới (BÁC amend ADR-0038 Comparable: `Ord ≠ Hash`; BÁC `Hashable` trait). Key ∈ {Integer, String} đóng băng. Slot `[key@key_stride | value@value_stride | state]`.
- [x] **KM-P1a BACKEND** (`c003a5f`) — Mũi A slot key_stride 24B fat · B `__triet_string_hash` FNV-1a + dispatch · D.1/D.2/D.5 key drop-obligation (§AMEND-1 out-param `is_update_out`/`key_out_ptr`, free registry-routed) · rehash key-stride. Hand-built MIR + counting, sleeping. O verify 5 teeth (#1 map-drop / #2 update / #3 remove / #5 content-hash / #7 rehash).
- [ ] **KM-P1b TYPECHECK+BORROWCK** (WO G-duyệt 2026-07-03, D đang code) — mở source `.tri` end-to-end:
  - **C1** typecheck generic-ize K∈{Integer,String} (`env.rs:342-391`): `hashmap_new<K,V>`/`insert<K,V>`/`remove<K,V>` + String-key overload get/contains/len. Seed K từ expected_type_stack.
  - **C2** REFUSE K∉{Integer,String} → mã lỗi MỚI `E10xx UnsupportedHashMapKey` (G: lấy E-code cao nhất rảnh, đập ở cửa typecheck, không defer-mềm).
  - **D3** borrowck insert-key = **Move** (String consume, Integer no-op) — mirror value-consume type-aware (`checker.rs:1224-1227`).
  - **D4** borrowck lookup-key = **borrow `&0 String`** (get/remove/contains KHÔNG consume) — tái dùng mô hình `&0` ADR-0079. **Lằn ranh sinh tử** (G): gõ lộn `consume=true` cho get = user double-free.
  - **Teeth (G ưu tiên #1):** ★SS `HashMap<String,String>` construct→insert→**remove→drop** — key-loop ∥ value-loop miễn nhiễm tombstone (state=2): gut key-loop→key leak · gut value-loop→value leak · double-free probe drop-lại-nấm-mồ→SIGABRT 134. + #4 insert-Move double-free · #6 lookup-borrow · #8 REFUSE E10xx · #9 Integer-key source compat.
- Defer Tầng-2+: `HashMap<_, UserStruct>` (P2 native) · get-clone/borrow heap value · get-borrow-mutable key · hash caching.

### ✅ ĐÓNG — Get-Borrow Heap Value from Container (ADR-0079 IMPLEMENTED/CLOSED, G ký 2026-07-01)
Đọc giá trị heap trong container **không đập hộp, không clone ngầm** — `get(&0 container, k) → (&0 V)?` zero-copy borrow (P1: V=String). Lấp lỗ read-side sau ADR-0077/0078. Mô hình loan = **whole-container** (G ký). Not-found → `(&0 V)?` nullable-borrow (PA-3c, NULL_SENTINEL). Slice A `a970540` + Slice B (gộp commit đóng).
- [x] **U1 typecheck/env** — `get(&0 Vector<String>,k)` + `get(&0 HashMap<Integer,String>,k) → (&0 String)?` overload (`env.rs`); value-position E1047 GIỮ.
- [x] **U2 borrowck builtin-return-borrow** — `returns_borrow_of: Some(0)` trên get_ref; PropagatedLoan source = whole container (source-tracing qua intermediate borrow `_tmp=&0 m`).
- [x] **U3 borrowck mutate-while-borrowed** — `mutates_arg: Some(0)` (remove/pop) + consume (insert/push); active loan → **E2440** (conflicting.dest thật).
- [x] **U4 lower/JIT** — `__triet_{hashmap,vector}_get_ref` trả con-trỏ-slot zero-copy (0 memcpy/alloc); not-found → NULL_SENTINEL `~0`. +F-d Copy-source skip-conflict.
- [x] **Teeth O (verify máu, poison cp-snapshot):** 5 borrowck (E2450·E2440 insert/remove/pop·negative) + source-level (content-read `length`→2/5·not-found `~0`·E2440 insert/remove). Fixtures 325/326 (route) + 327 (content-read guard).
- Defer: ~~generic V-overload~~ **ĐÓNG (Cụm A `37a0723`, V=Vector/HashMap Int+String-key)** · get-borrow-mutable (`&0 mutable`) **→ ADR-0081 FROZEN, đày Cụm D**. **key-typed: BACKEND `c003a5f` + source KM-P1b `381979e` ĐÓNG.**

### ✅ ĐÓNG — Read-side Cụm A: get-borrow generic-V + P0 String-key SIGSEGV (ADR-0079 §AMEND-1, G ký 2026-07-04, `37a0723`)
- [x] **A1 get-borrow generic-V** — env.rs 6 overload `get` V∈{Vector<Integer>,HashMap<Integer,Integer>} qua Vector<V>/HashMap<Integer,V>/HashMap<String,V> → `(&0 V)?`. JIT §AMEND-1 stride-conditional deref (get_ref): thin V (stride≤8)→`*cell`=body_ptr, fat V (String)→cell. Invariant `&0 V` bit-for-bit local≡get_ref.
- [x] **P0 String-key read SIGSEGV** (BÁO ĐỘNG ĐỎ, pre-existing từ ADR-0080) — get/get_ref/contains nhận `&0 HashMap` (Reference-wrapped) ≠ insert (owned); key_stride extraction chỉ bóc Nullable → default 8 → String-key 24B marshal by-value → SIGSEGV. VÁ: unwrap `MirType::Reference` trước match HashMap. Fixture 335 (contains+get+not-found →142).
- [x] **Fixtures 333-337** — 333 Int-key content-read(3) · 334 borrowck-track(E2440) · 335 P0 scalar String-key(142) · 336 String-key get_ref Vector(2) · 337 String-key get_ref HashMap(1). O verify poison→RED: POISON-1/POISON-P0/overload-break→E1041.
- Defer: get-borrow-mutable (A2 FROZEN, Cụm D) · V=Nullable get_ref (lowerer chưa match `&0 Nullable`) · V=UserStruct (P2 native-layout).

### 🏛️ Facade pattern (`public use` re-export) — Amend ADR-0005 §76 (G chốt 2026-06-29, ghi sổ)
Tách Logical Tree (API surface) khỏi Physical Tree (file layout) — lấy cái ngon của Rust
(`pub use` facade) mà GIỮ DNA explicit của Triết. **Bối cảnh:** Triết KHÔNG 1-1 Java
(ADR-0005 §17/§96/§155 đã tách bằng `module foo` declaration); auto-discovery (file=module
ngầm) bị BÁC — nghịch ADR-0005 (reject A1/A3/A5) + nghịch refuse-over-guess + phá hermetic build
(file `.bak`/generated đổi API). Việc THẬT còn thiếu = re-export, đã defer ở **ADR-0005 §76**
("Re-exports defer sang v0.3+") + parser đã chừa chỗ (`triet-parser/src/item.rs:78-85` refuse
`public use` với "not yet implemented", refuse-over-guess thay vì drop `public` silently).
- [ ] **Amend ADR-0005 §76**: bật `public use X` re-export — facade-file (`module.tri`/inline) chìa
      mặt tiền phẳng; dev xé file tự do trong bóng tối, chỉ `public use` mới phơi ra. Xây TRÊN
      `public`/`public(package)` hiện có — **MỘT cơ chế visibility, KHÔNG đẻ `export...from` song song** (coherence).
- [ ] **🌱 Ghim — Capability-aware facade (hạt giống đẳng cấp, O đề xuất + G bless):** facade chặn
      không chỉ visibility mà cả **năng lực** — re-export một type KHÔNG đồng nghĩa re-export quyền
      (capability token Ł3) để khởi tạo nó. Module system mang ngữ nghĩa Ł3 ở API boundary → vượt Rust/TS.
- **Timing (G chốt):** TUYỆT ĐỐI KHÔNG code lúc này. Chờ tới khi xây `std` hoặc Package Manager
  (nhu cầu facade mới rõ). Trade-off đã biết: mất zero-indirection navigation → cần tooling go-to-def bù.

### ✅ ĐÓNG — Bug-E: Outcome-param ABI mis-tag + `~->` early-return heap double-free (O+G ký 2026-07-03)
~~Outcome (`T~E`/`T?~E`) làm tham số hàm mis-tag `disc` (sai lặng lẽ) + `~->`
early-return double-free heap payload~~ → **ĐÓNG SẬP, 2 WO liên tiếp, cùng chiến
dịch.**

**WO1 — Outcome-param callee copy-in gap** (`ddb7841`): callee prologue cấp StackSlot
rỗng cho MỌI Outcome-typed local kể cả tham số (`mir_lower.rs:1453`), nhưng vòng bind
tham số chỉ có nhánh copy-in cho String/Enum, thiếu Outcome — con trỏ caller truyền
vào (đã đúng, `:2676`) bị bỏ xó, `_N.disc`/`_N.payload` đọc rác. Fix mirror đúng
khuôn String/Enum tại `:1644-1684`. Fixtures 328/329/330 (scalar/nullable/interleaved-
offset). O verify độc lập cp-snapshot (không stash — D vi phạm `feedback_teeth_never_git_checkout`
lần đầu, G ghi sổ đen cảnh cáo, kết quả tình cờ đúng).

**WO2 — `~->` early-return heap-payload double-free** (`818602c`), phát hiện khi O tự
mở rộng test ngoài phạm vi WO1 (`String~Integer` param) → SIGABRT 134. Cô lập: bug
KHÔNG liên quan tham số hàm, tái hiện chỉ với 1 local. 3 site cùng thiếu pattern
HP.4 (`copy_heap_outcome_payload`/`bind_heap_outcome_payload` + `Deinit`):
Site A (`lib.rs:~5163`, success-arm passthrough unwrap), Site B (`:~5023`, error-arm
bind `e`), root cause chung (`:~1947`, `Expr::OutcomeConstructor` heap-payload branch
— DÙNG CHUNG cho mọi `~+ v`/`~- e` trong ngôn ngữ, vô hại với literal/temp nhưng
double-free khi payload là named-local có drop-obligation — đúng tình huống Site B
tự tạo ra). Fixtures 331/332 (named-local, theo LUẬT `feedback_poison_must_be_red`).
O verify máu độc lập cả 3 site — poison từng site một, xác nhận đỏ đúng biến thể
(331 XOR 332), restore md5 khớp, gate CLEAN 326.

**Bài học:** silent-wrong-answer (WO1) được G xếp NẶNG hơn crash (WO2) — nhưng cả hai
đều bắt nguồn từ cùng một lỗ hổng lớp: thiếu hoàn thiện pattern copy-in/Deinit khi mở
rộng bề mặt Outcome sang ABI tham số + early-return. 0 fixture cũ từng chạm surface
này vì luôn dùng literal/temp (không có drop-obligation) — đúng LUẬT NAMED-LOCAL đã
ghi từ HM-P1b.

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

### ✅ ĐÓNG — **Heap-Nullable (KỶ NGUYÊN NULLABLE KHÉP)** — ADR-0062/0065/0076 SEALED
`T?` cho `T` heap (String/Vector/HashMap/Struct/Enum) ĐÓNG TRỌN mọi vị trí:
top-level (ADR-0062 ptr-sentinel) + aggregate `Enum?`/`Struct?` (ADR-0065 niche/tag-prepend)
+ field/payload B8 (ADR-0076 `994afc8`). Gate `MirError::HeapNullableNotLowered` còn SỐNG
nhưng giờ chỉ refuse heap-nullable trong **recursive/Box** (ADR-0068 CẤM CỬA) — mọi vị trí
non-recursive đã lower.

**Ruling β (G ký 2026-06-18, vẫn hiệu lực):** gate ở LOWER (`Body::verify()`), KHÔNG typecheck
— stdlib khai heap-nullable làm API (`env.get`/`fs.read -> String?`); declaration vô hại
(stub `= ~0`), chỉ *compilation* mới gác. ✅ 5 mũi gốc (ptr-sentinel · widening · conditional-drop
· Elvis/match `~+/~0` · `?+>` map) landed qua ADR-0062; aggregate qua ADR-0065; field/payload
qua ADR-0076 (cổ tức PA-3c: conditional-drop = sentinel-no-op, 0 `brif`). Fixtures 189-237 + 311/312.
- [x] **Gap #2 — expected-type propagation (ADR-0072 🔒 SEALED 2026-06-27, 3-slice).**
  - [x] **Slice 1** `c9a46e6` — `lower_expr` thêm param `expected: Option<&MirType>`, 61 site=`None`, byte-identical (O verify MIR-diff rỗng toàn corpus).
  - [x] **Slice 2** `2c900fb` — leaf-consumer (`OutcomeConstructor`/`NullLiteral`) đọc `expected`; wire 4 nguồn (function-body/return/let-init/struct-field); đập 3 Bug-B redirect. **Mở `T?`-return scalar** (303 Integer?, 305 let-Integer?-trong-Outcome-fn). Fallback §2.5 chuyển-tiếp (gỡ ở Slice 3). O verify: gate 0·0·299·0 + byte-identical 297 cũ + 2 poison `OutcomeAlloc on non-Outcome` đỏ + defense-in-depth 2 guard.
  - [x] **Slice 3 (kết liễu)** — transparent forwarding `expected` xuống Block tail + If then/else + **13 arm-body** (8 WO + wc.body×4 + arm_for_present×1) + helper `lower_value_keyed_match`; gỡ sạch fallback §2.5; **nhổ `c.sig.return_type` khỏi input constructor** (chỉ còn 4 nguồn return-position hợp pháp: body-tail/return-non-sret/return-sret/Expr::Return); extract `emit_outcome_zero`. Mở `~+`/`~0` trong if/match/block-arm context≠sig (306/307/308) + **309 negative khóa luật untyped-ctor-bị-từ-chối** + 157 annotated. O verify máu: gate 0·0·303·0 + byte-identical 299/299 (worktree baseline) + 3 poison R-fwd đỏ + grep C sạch. Diagnostic tổng quát hóa (hết nói "~0 null" cho ~+/~-).
- [x] ✅ **WO-0075 — multi-level extraction `let x = h.inner.x` (ĐẠI PHẪU Nợ B — projection-path move-state, ADR-0070 §AMEND Phase 3, G ký 2026-06-29).** ADR `b74e03e` + C1 `3826924` + C2 `bd614f3` (push main). Nâng `partial_moves` từ field-name → **projection-path** (`Set<String>`→`Set<Vec<String>>`), mở móng Capability Ł3 nested. **2 commit tách (G mandate git-sạch):** C1 = vá **lỗ fixpoint-hole CÓ SẴN** (fixpoint check chỉ so var_states+active_loans, KHÔNG so partial_moves → partial-move không set base→Moved → delta âm thầm vứt → UAM lọt qua back-edge trong loop), XANH ĐỘC LẬP trước; C2 = feature projection-path. **Cascade:** `single_field`→`projection_path() -> Option<Vec<String>>`; helper `prefix_conflict(p,m)=p[..min]==m[..min]` (exact/ancestor/descendant/whole-base DEAD, sibling LIVE); allow-arm ghi path đầy đủ; §F sub-path reassign (`h.inner=fresh` sau move `h.inner.x`) KHÓA bằng **E2424 SubPathReassignUnsupported mới** (ADR-0027 format). JIT Site-G nới gate `[Field]`→`all-Field` (`walk_projections` trả abs-offset mọi depth); Lower Site-H **no-op** (`place_result_type` đã loop multi-level — MIR probe pin). **9 tooth, O verify máu độc lập (cp-snapshot, control-biến, KHÔNG git checkout):** G🩸 fixpoint-loop (gỡ partial_moves khỏi fixpoint→UAM lọt `got:[]`, structural-guard back-edge thật); F⚔ merge-union (gỡ union→join quên `got:[]`, structural-guard diamond ≥2 preds); B+D prefix exact-only poison → B(ancestor)+D(whole-base) ĐỎ, **A/C/E/F XANH=control đặc hiệu**; H runtime (revert JIT gate→`left:2 right:1` double-free); E2424-lock (skip diagnostic→`got:[]`). **LUẬT THÉP #3:** `cannot_move_multilevel_field_out` retarget→`cannot_move_non_field_projection_out` (Payload→E2423, ca CÒN refuse) — negative coverage BẢO TỒN dời mục tiêu (hành vi cũ bị ADR G-ký lật); fixtures 298/302 `*_e2423`→`*_run` real-allocator soundness witness (double-free→SIGABRT, clean-run trong corpus 303). Gate `0·0·303·0` (counting binary riêng). Backward-compat 100% (Phase 2/2b regression XANH). **NỢ B ĐÓNG TRỌN. Còn defer: non-Field projection (Index/Deref) · sub-path reassign mở · ADR-0068 recursive CẤM.**
- [x] ✅ **WO-0074 — enum-field move-out `let e = h.msg` (Heap-aggregate Phase 3 — Nợ A, G ký 2026-06-29).** `e0b1ed7` (push main). Single-level move-out của heap-carrying enum field ra khỏi struct base (construct + base-Drop enum-in-struct đã có từ ADR-0067; WO này chỉ mở đường move-out). **3 site đối xứng heap-struct campaign:** Site-1 `lower/lib.rs` thêm `matches!(field_ty, Enum(_))` vào gate type-slot → dest mang Enum → JIT cấp enum-slot (không thì Unknown→no-slot→SIGSEGV); Site-2 `borrowck/checker.rs` thêm `Enum(_)` vào allow-guard — **`partial_moves` key = field-name đơn ("msg"), data-structure KHÔNG đổi** (Nullable/Outcome vẫn refused); Site-3 `jit/mir_lower.rs` arm `Enum(_) => zero payload-ptr@field_off+8` (disc giữ, đối xứng leaf-Enum `abs+8`). **5 tooth, O verify máu độc lập (cp-snapshot, KHÔNG git checkout):** T1 borrowck (gỡ Site-2 → E2423 `obj.msg`/`Msg`); T2 double-free (gỡ Site-3 → FREE `left:2 right:1`); T3 leak (phủ định kép `==1`); T4 cap+count đồng thời (`STR_CAP==5 && STR_FREES==1`, assertion-guard byte-copy); **T5 ⚔ SIGSEGV in-suite** (gỡ Site-1 → child `wait_status(139)`=signal 11, crash CÔ LẬP subprocess, test-runner sống). File mới `enum_field_moveout_counting.rs` + `enum_field_moveout_subprocess.rs` + tooth-1 inline `checker.rs`. Gate `0·0·303·0` (counting/subprocess là test-binary riêng → KHÔNG nhúc nhích corpus 303; WO §1 "≥305" của O là giả định sai, ghi nhận). **NỢ B (multi-level `h.inner.x`) ĐÓNG BĂNG — đụng borrowck core `partial_moves` key, cần ADR-0070 amend riêng.**
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
- [x] **gate.sh giòn — exit 1 giả khi clippy=0** (G ghi sổ 2026-06-18) — VÁ `9263501` (chore(infra) Kỷ-Luật-Gate: exit code trung thực + counting-test Mutex isolation). gate.sh nay exit 0 ⟺ cây sạch; vận hành ổn định qua WO-0073/74/75 (`0·0·303·0`). *(Commit gốc mang nhãn `[PENDING O-VERIFY]` — teeth tự-kiểm gate-bắt-đỏ là việc O, ngoài scope WO này.)*

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
