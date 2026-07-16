---
name: campaign_typed_collections
description: "✅ Typed Vector/HashMap P1 (ADR-0077/0078) + Get-Borrow (ADR-0079) + key-typed HashMap<String,V> (ADR-0080) + Read-side Cụm A + CỤM B push+drop Slice A/B/C (Vector<Struct/Enum>, HashMap value) + VALUE MOVE-OUT D-1/D-2 (Vector pop, HashMap remove by-value, ADR-0082 §AMEND-2) + Vector::pop_front + KEY-AGGREGATE Slice 1 struct-key + Slice 2 enum-key (ADR-0083) + GET-BY-VALUE Copy-aggregate (ADR-0082 §AMEND-3, is_copy_aggregate↔MirType::is_copy, E1049 refuse heap-bearing, non-destructive copy-out) — KHÓA SỔ 2026-07-15 `12adc0b`. Full detail, MEMORY.md index only links here."
metadata: 
  node_type: memory
  type: project
  originSessionId: ac639140-8210-42c9-941b-8cfd203d270e
---

## ✅ ĐÓNG — ADR-0079 §AMEND GET_REF SLICE 2: `get(&0 c,k)` → `(&0 Agg)?` (O+G ký 2026-07-17, PUSHED)
origin/main = `8981135` (1 commit), gate `0·0·385·0`. **Trận cuối quét E1041/E1049 — KHÔNG ADR mới** (§AMEND ADR-0079, compose ADR-0084). `get(&0 Vector<Agg>,i)` / `get(&0 HashMap<scalarK,Agg>,k)` → `(&0 Agg)?` **zero-copy**, heap-bearing **ĐƯỢC PHÉP** (bia mộ E1049: `Vector<Tagged{String}>`→`&0 Tagged`→`.name` chạy). Fixtures 388-391 + `get_ref_aggregate_counting.rs`. **Model D = Sonnet 5.**

**🩸 RECON O LẬT KHUNG BÀN GIAO — "hạ tầng sẵn, chỉ bóp cò" là SAI.** Sổ 2026-07-16 ghi tầng 2 = "verify JIT route". Đo thật ra **2 MÌN**:
- **MÌN 1 — lowerer ÉP mọi aggregate về `_get_copy` bất kể `&0`** (`triet-lower/src/lib.rs:2501` test `is_aggregate_elem` TRƯỚC `is_borrow`; comment tại chỗ tự thú *"routed identically either way"*). Chỉ mở typecheck = typecheck hứa borrow, lowerer giao copy → `returns_borrow_of:None` → **0 loan** = đúng nỗi lo G. Fix: `is_aggregate_elem && is_borrow` → shim riêng.
- **MÌN 2 — 8B-aggregate deref-thin, LẦN THỨ BA của lớp lỗi** (sau Slice A T9, §AMEND-3 T9-masking). `_get_ref` deref khi `stride≤8` (ĐÚNG cho V=container: handle *là* body_ptr) nhưng `&0 Struct` từ local = **stack_addr** (`mir_lower.rs:3154`, Blocker-B Slice 1a) — địa chỉ bất kể size. `Id{value:Integer}` 8B → trả `42` thay con trỏ → field-read deref 42. **O poison → signal 11 SIGSEGV.**
- **Tầng 3 NHẸ hơn sổ:** `returns_borrow_of:Some(0)` đã có sẵn, keyed theo TÊN SHIM → route đúng = loan tự có. **Borrowck 0 dòng code.**

**Cơ chế:** typecheck arm MỚI đặt TRƯỚC arm get-by-value, chỉ match `Reference(BorrowReadOnly,_)` → helper `get_ref_aggregate_return_type` (`check/exprs.rs:2345`), sibling `get_by_value_aggregate_element` (`:2374`) match `arg_tys[0]` TRỰC TIẾP → **mutually exclusive by construction** = ranh-giới-tử-thần is_ref/non-ref. **KHÔNG `is_copy_aggregate` ở arm get_ref** (heap-bearing là ca dùng CHÍNH). D chọn **tách 2 shim Rust mới** `__triet_{vector,hashmap}_get_ref_agg` (không deref, mọi stride) thay vì cờ runtime — G khen "tỉnh táo", cùng nước cờ `_get_copy` §AMEND-3: tách symbol = 0 chi phí, 0 rủi ro hồi quy 333/336/337 (sống nhờ deref-thin). Vết dọn kèm: **E1049 message trỏ `get_ref(container,k)` — API KHÔNG TỒN TẠI** (env.rs không declare) → dẫn user vào E1041; sửa → `get(&0 container, k)`.

**🩸 O TEETH (cargo test, cp-snapshot restore md5 khớp mọi vòng, 0 git-checkout):** MÌN-2 khôi phục deref-thin → 389 **SIGSEGV 11** · loan→None → 388 **mất E2440** ("succeeded with 42") · copy-lén (ép `_get_copy`) → 388 mất E2440 **+** 390 bị **F3 guard JIT** chặn (**F3 vốn latent từ §AMEND-3 nay CẮN THẬT**) · `aggregate_needs_drop(Struct)→false` → counting **left:0 right:1** cả Vector lẫn HashMap.

**⚖ O TỰ ĐÍNH CHÍNH — WO của O SAI failure-mode, D BÁC ĐÚNG (lần 2 sau §AMEND-3):** O viết teeth-1 "gỡ loan → **SIGSEGV lọt**". D báo thực = **lọt câm** (compile + 42, exit 0). O không nhận lý lẽ suông → probe độc lập + **ép thêm bằng realloc** (buffer cũ free THẬT) → vẫn 42/exit 0. **Cơ chế: `&0` reference KHÔNG có drop obligation → dangling READ, không double-free, không signal deterministic** = **(a) bất-khả-observable**, không phải (b) test-yếu. Loan = lá chắn **compile-time**, tooth đỏ đúng tầng đó. G: *"Chào mừng đến Systems Programming — reference &0 đéo có drop obligation"*, khen O dựng realloc = verify-don't-trust.

**✅ D TỰ HẠ CLAIM CỦA CHÍNH MÌNH (ngược hẳn vết "bịa chứng phụ" Slice 2 enum-key):** counting tooth D định claim `==2 ⇒ double-free` → tự poison KÉP → count **VẪN 1** → đào MIR ra lý do (`length(t.name)` lower `move _15.name` → field MOVE khỏi bản copy → alias không bao giờ free lần 2, leak vào temp không drop) → **tự sửa xuống `==0 ⇒ leak`** + ghi "HONEST LIMIT" + **cấm người sau thêm claim `==2` mà không chứng minh**. G đánh giá "cực cao sự minh bạch".

**🚩 Nợ mới — `&0 Enum` SURFACE CHẾT (D tự khai, O probe xác nhận):** typecheck arm nhận `UserEnum` nhưng 0 fixture; O probe → payload-bind = typecheck refuse (`undefined name s`), disc-match = **lowerer refuse `LowerError` KHÔNG panic** (Track B rule 1 giữ). Refuse-guarded cả 2 tầng, **không crash** → G: "đóng gói ném sổ nợ, không cản release". Arm hứa `(&0 UserEnum)?` mà chưa có đường tiêu thụ.

**⚠️ Vết vận hành D:** vòng đầu chỉ đăng ký shim ở `main.rs`, **quên registry RIÊNG trong `integration_tests.rs`** → 4 fixture xanh qua binary tay nhưng harness gate "shim not registered" — D tự tìm/tự khai (**bài học: chạy ĐÚNG harness của gate, đừng tin binary tay**). Clippy base 0 → D đẻ 4, **tất cả code D** (không claim pre-existing), sửa THẬT bằng tách helper, **0 `#[allow]`**, re-verify teeth 2 sau refactor.

**Bài học Slice 2:** ① **recon-trước-khi-gõ cứu slice** — bàn giao "chỉ bóp cò" giấu 2 mìn; verify-before-WO là luật (lần 2 sau D1 phiên trước). ② **8B-aggregate masking = mẫu lặp lần 3** — luôn probe `total_size==8` khi mở đường aggregate mới. ③ **failure-mode của `&0` dangling = READ câm, KHÔNG signal** — đừng viết WO đòi SIGSEGV cho use-after-free của reference không drop-obligation. ④ tách symbol > cờ runtime khi phân luồng JIT. [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]] [[colleague_d_persona]] [[mentor_o_persona]]

## ✅ ĐÓNG — ADR-0082 §AMEND-3 GET-BY-VALUE Copy-aggregate (O+G ký 2026-07-15, PUSHED)
origin/main = `12adc0b` (feat `28f7c6f` + docs `12adc0b`), gate `0·0·361·0`. **§AMEND-3** (O đề, G/Giang rule scope, G co-sign). `get(container, k)` trả **by-value** aggregate **thuần Copy** (Struct/Enum không heap leaf) từ `Vector<Agg>` + `HashMap<scalar-K, Agg>` — **non-destructive** (element ở lại container, KHÔNG tombstone/free, bitwise-copy độc lập). Heap-bearing aggregate → **REFUSE E1049** (mã mới, trỏ `get_ref`). aggregate-key×aggregate-value → defer. **Model D = Sonnet 5.**

**Cơ chế (§A):** ranh giới Copy-vs-Clone LÀ ranh giới soundness (O đặt điều kiện-chặn): Copy aggregate (không heap leaf) → bản copy bit-identical KHÔNG chia sẻ heap → drop độc lập, KHÔNG double-free; heap-bearing → bitwise copy alias con trỏ → hai chủ → double-free → REFUSE, deep-Clone tách campaign-nền riêng. **Predicate `Type::is_copy_aggregate` (types.rs:227) mirror `MirType::is_copy` (mir:694, single-source-of-truth) — KHÔNG `!aggregate_needs_drop`** (over-approx enum-field via `collect_heap_leaves` đẩy mọi Enum field làm leaf vô điều kiện). Shim mới `__triet_{vector,hashmap}_get_copy` (`returns_borrow_of:None`) tách khỏi get_ref → borrowck KHÔNG synth PropagatedLoan (§AMEND-3.5); reuse thân get_ref dưới symbol thứ 2. JIT copy-out: thin (stride≤8) ghi thẳng deref'd return (get_ref thin trả VALUE không pointer), fat (>8) load-loop; shared defensive `!is_copy` guard (Rule#7, latent — chỉ nổ nếu typecheck regress).

**🩸 O VERIFY MÁU (cp-snapshot restore md5 `a753366b`):**
- **Răng producer-consumer (load-bearing):** poison `is_copy_aggregate` heap→Copy → `Vector<Tagged{String}>` get → **double-free 134** (MIR phơi container `Drop(_2)` + copied-out `Drop(_12)` cùng free String). Typecheck E1049 gate = an-toàn DUY NHẤT cho Vector (JIT guard latent). Restore md5 khớp.
- **🎯 8B-heap-struct T9-masking (Slice A leak-câm) — D KHÔNG test, O BẮT:** `Wrapper{v:Vector<Integer>}` (total_size=8, single handle → thin-path, thiếu refuse = leak/double-free CÂM không signal) → O probe sống → **E1049 refuse đúng**. Thêm fixture 367 canh vĩnh viễn, non-vacuous (`Id{value:Integer}` 366 cũng 8B nhưng Copy → compile OK).
- Positive 361/362/363/366 + counting FREE-count route-lower (`lower_source`) xanh. E1049 harness assert code-string (`e.contains("E1049")`).

**⚖ D LỆCH LỆNH ĐÚNG (LUẬT 5) — D đúng O sai:** D bác giả định WO gốc của O `is_copy_aggregate ≡ !aggregate_needs_drop`, chỉ ra over-approx enum-field, chọn `MirType::is_copy`. G khen "soundness chuẩn xác". **O tự đính chính §AMEND-3.2.** Nước cờ tách shim `_get_copy` chặt đường sinh loan = G khen "kiến trúc sắc sảo". D tự bắt+vá SIGSEGV thin-return-deref (363) khai thật; F3 gom guard đối xứng 1 chỗ.

**⚠️ VẾT D (Sonnet 5):** ① thiếu fixture 8B-masking (O bù F2) · ② comment JIT bịa ví dụ `Wrapper{v:Vector<String>}` cho thin-path (case đó bị E1049 refuse, không reachable — mẫu "bịa chứng phụ trong doc" Slice 2; O bắt F1) · ③ claim "fmt ran" nhưng edits cuối chưa fmt (LUẬT 2 slip — pre-commit hook `cargo fmt --check` bắt, O fmt lại). Kết luận + implementation lõi đúng; O verify bù 3 vết.

**Bài học §AMEND-3:** ① Copy-vs-Clone = ranh giới soundness, thu hẹp Copy-only né deep-Clone/ADR-0042 (khuôn pop_front/Slice A). ② producer-consumer 2 crate (typecheck `Type` ↔ mir `MirType`) verify TRỰC TIẾP cả hai, không tin tên hàm — poison chứng minh gate load-bearing. ③ 8B-heap masking (thin-path, total_size=8 ôm handle) = mẫu leak-câm lặp lại từ Slice A — luôn probe case này khi mở by-value aggregate. ④ pre-commit fmt-hook là lưới cuối bắt LUẬT 2 slip của D. **🔴 Deep-Clone heap-bearing = campaign LỚN riêng** (ADR `.clone()` + carve-out ADR-0042 + codegen clone đệ quy). [[feedback_poison_must_be_red]] [[feedback_verify_producer_before_consumer]] [[colleague_d_persona]]

## ✅ ĐÓNG — ADR-0083 §AMEND-1 KEY-AGGREGATE Slice 2 (Enum keys, O+G ký 2026-07-13, PUSHED)
origin/main = `91c273a`, gate `0·0·354·0`. §AMEND-1 (G ký cuối ADR-0083). `HashMap<Enum,V>` enum-key (payload **unit/scalar/String** + enum-as-struct-leaf) sound: insert/get/get_ref/contains/remove/drop. **KHÔNG ADR mới** (Slice 2 đã scope-defer trong ADR gốc). **Model D = Sonnet 5** (Giang chọn thử tiered; O verify máu bù).

**Cơ chế (§A):** ABI KHÔNG đổi (fnptr-in-header + §6 dispatch + collision-shield = Slice 1 verbatim). Walker model **flat `KeyLeaf` → recursive emission** `emit_key_hash_value`(mir_lower:558)/`emit_key_eq_value`(:680): enum arm = **disc-mix vào FNV + `brif`-chain over ACTIVE variant only** (mirror `emit_enum_drop_glue_at:1886`), eq = disc-short-circuit-NE + per-active-variant leaf compare. **CHỈ đọc disc@0 + active-variant declared leaves @+8, KHÔNG BAO GIỜ raw fixed-width image** = thuốc giải garbage/padding/size-mismatch G lo. Key free-loop **REUSE THẲNG `emit_enum_drop_glue_at`** (G gật 100%). `enum_payload_variants:802` choke. Typecheck `is_hashable_key/leaf/enum_payload` (types.rs). Overload `exprs.rs:1196` thêm UserEnum. `key_marshal` fallback `enum_slots`.

**🩸 O VERIFY MÁU (cp-snapshot restore md5 `80fd7ce7`):**
- **DP-E2 reassign-garbage (G MANDATE, G tự hào):** poison tail-read @off+16 → 358 **MISS -1** (healthy 42) → **NON-VACUOUS** — tail rác sau `let mutable k=Big(String); k=Small(1)` THẬT SỰ tồn tại (@16..32 = stale {len,cap} của Big), walker active-leaves-only né đúng. (354 collateral -993 = poison active; 355 String-payload giữ 42 content-deterministic.)
- **DP-E6 §6-reverse** (đảo fnptr↔stride `hashmap_key_hash:5658`) → 354 enum-key **crash 134** → shield gánh cả key-class enum mới (chỉ nổ khi poison, không lãng xẹt).
- Baseline walker JIT THẬT: 354=42007·355=42·356=42007(enum-as-struct-leaf)·357=42007(unit)·358=42. Struct Slice-1 **KHÔNG hồi quy** 352=42007/353=42 (dù walker viết lại flat→emission).

**⚖ DESCOPE — G RÚT LỆNH "nested-enum MỞ" (probe O quyết):** enum variant ôm **aggregate payload (Struct/Enum)** → **REFUSE E1048**. O **lift refuse + probe** `HashMap<Shape,_>` roundtrip (`Shape::Dot(Point)`) → **MISS -1** → chứng minh lowerer fix-8B enum-payload (fixup pass chỉ struct field, bỏ enum payload) → aggregate >8B truncate on marshal → **silent MISS thật**. D descope = refuse-over-guess ĐÚNG, G khen "cứu rỗi bộ nhớ". **🔴 NỢ MỚI G MỞ: "Enum-Payload-Aggregate Sizing Fix"** (`triet-lower/src/lib.rs`) — đóng thì mở khóa nested-enum/enum-struct-payload key.

**⚠️ D BỊA CHỨNG PHỤ (mẫu #9, dù kết luận đúng):** report + doc-comment types.rs ghi *"enum-in-enum fails MIR verifier even in plain match"* — **O P1 probe bác** (`enum Inner/Outer` plain match chạy=7). G: "khóa feature CHỈ được vì sự thật compiler, KHÔNG bịa; đưa rác vào doc = phá hoại di sản". Sửa doc: giữ sự thật (truncate→MISS, neo O-probe -1), xóa claim MIR-verifier. **O tự sửa doc-fix cuối** (Sonnet-D quota-cut giữa chừng nhiều lần; doc = documentation của finding O verify, không phải feature-code D).

**Bài học Slice 2:** ① disc-switch-walker mirror drop-glue = tổng quát hóa sạch (active-leaves-only né garbage — DP-E2 chứng minh cả padding rác THẬT lẫn walker né ĐÚNG). ② refuse-over-guess thắng cả G-ruling khi probe lộ pre-existing rot (lowerer enum-payload-sizing). ③ verify claim-phụ của D độc lập — kết luận đúng KHÔNG miễn trừ chứng cứ bịa. ④ Sonnet đủ sức phần lõi disc-switch nhưng vết = 1 doc bịa + nhiều quota-cut. [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]] [[colleague_d_persona]]

## ✅ ĐÓNG — ADR-0083 KEY-AGGREGATE HashMap Slice 1 (Struct keys, O+G ký 2026-07-13, PUSHED)
origin/main = `1c08a67` (feat `0ebd763`+`1c08a67`; TODO-freeze `10c4ed1`), gate `0·0·347·0`. **ADR-0083 MỚI** (G ký). `HashMap<Struct,V>` — struct làm KEY (leaves scalar/String/nested-struct) sound end-to-end: insert/get/get_ref/contains/remove/drop.

**Semantics (§1 — de-risk lớn nhất, O recon mở đường):** key-eq/hash = **structural content/bit-equality đệ quy trên physical layout, KHÔNG dính operator `==`/Ł3** (tiền lệ ADR-0080 `Ord≠Hash`). → key-aggregate KHÔNG mở lại đầm lầy Trilean. Đây là điều kiện-chặn O tự đặt khi đề xuất mặt trận, recon phơi bằng chứng thỏa → G duyệt đi tiếp.

**ABI G-MANDATE (§2/§6 — G BÁC thiết kế stride-branch ĐẦU của O):** header cố định 24B `[refcount@0][packed@4][hash_fn@8][eq_fn@16]` + fnptr-in-header null-sentinel (Integer/String→NULL, Struct→`func_addr` walker). **§6 dispatch fnptr-TRƯỚC-stride = lá chắn SIZE-COLLISION-TRAP:** `struct{3×Integer}`=24B TRÙNG String `key_stride`=24; disambiguate bằng stride → đọc struct thành FatStr `{ptr,len}` → deref rác → SIGSEGV. **`hash_fn!=NULL` LÀ discriminator, KHÔNG phải stride.** fnptr calling-conv: `hash_fn(key_ptr)->i64` raw FNV (shim tự `%cap`), `eq_fn(a,b)->1/0`. Rehash chạy TRONG insert → fnptr phải cư trú header.

**JIT walkers (§3):** `build_key_hash_walker`/`build_key_eq_walker` (mir_lower:637/695) đệ quy `collect_key_leaves:554` — scalar→FNV-mix i64 · String→`__triet_string_hash(ptr,len)` · nested→đệ quy; eq short-circuit `brif`. Emit 1 FuncId/key-layout, địa chỉ qua `declare_func_in_func`+`func_addr` (**spike func_addr fail-fast TRƯỚC walker — G mandate**). Key free-loop đệ quy §4 (mirror `aggregate_needs_drop` Slice C). Typecheck `is_hashable_key`/`is_hashable_leaf` (types.rs:163/177) + E1048 (Enum-key/collection-leaf/Nullable-leaf/Outcome REFUSE).

**⚙ Quy trình (D subagent Opus, NHIỀU lần bị quota/session-limit cắt giữa chừng):** O verify bàn giao → recon feasibility → **G ruling ABI (BÁC stride-branch, mandate fnptr-header + null-sentinel)** → O draft ADR-0083 → D subagent triển khai qua nhiều lần resume → O verify máu → 2 vòng D-fix → ký. Bài học vận hành: **verify-don't-trust áp cả EXECUTABLE — rebuild TRƯỚC mọi lần chạy binary; hai gate cargo song song tranh `target/`-lock → dọn process trước; `pkill -f` có thể tự giết shell (exit 144).**

**🩸 O bắt 2 BLOCKER (gate xanh 347 nhưng green≠done — Track B rule 3):**
- **Blocker A — lookup ops CHƯA WIRE source:** probe driver rebuild-sạch → `get`/`get_ref`/`contains` struct-key = **E1041** (danh sách overload KHÔNG có variant `HashMap<Struct,_>`; chỉ insert/remove reachable). "Compiles"-only + stand-in walker che lấp (không viết nổi roundtrip vì get chưa wire). D vá: wire overload `exprs.rs:1190` + 2 fixture roundtrip THẬT.
- **Blocker B — walker correctness chỉ stand-in/compile-only:** code LÕI `build_key_*_walker` không test nào assert runtime (correctness dùng stand-in Rust `k3_int_hash`/`kstr_hash`; "compiles" không execute; drop-count dùng free-loop §4 KHÔNG chạm hash/eq §3). D thêm fixture 352 (int roundtrip→42007) + 353 (String-leaf content-collide→42) chạy walker JIT THẬT.

**⚔ MÂU THUẪN O bắt + giải bằng máu (★SS(c) [[feedback_poison_must_be_red]]):** comment fixture 353 claim "hash-poison ptr-mix → RED -1, load-bearing tooth" NHƯNG report D (d) nói "vacuous". O tự chạy ptr-mix → **353=42 DETERMINISTIC 5/5 (VACUOUS)** → D report ĐÚNG, fixture-comment SAI. **Cơ chế O chứng minh (toán+máu):** allocator căn-16 → hai String ptr chung low-4-bit → `hash mod cap` TRÙNG với mọi `cap≤16` → ptr-vs-content hash BẤT-PHÂN-BIỆT ở bucket. Răng thật của 353 = **eq-content** (eq→ptr-identity → 353=-1). D sửa comment (KHÔNG đụng logic). **Bài học vàng: hash-content KHÔNG tooth được bằng functional-roundtrip cap-nhỏ (alignment mask); cap-LỚN + assert-hash-TRỰC-TIẾP mới bắt — ADR-0080 tooth #5 `cap=1_000_003` là mẫu (D bắt qualify chuẩn; O tự đính chính note "hash vô-phương tooth" hơi quá).**

**🩸 O VERIFY MÁU (cp-snapshot restore md5 `0fd4b450` mọi vòng, KHÔNG git checkout):** §6-reverse → 352 **SIGSEGV 139** (collision-trap G đòi máu, load-bearing) · eq-content-String→ptr-identity → 353 **-1** RED (352 giữ 42007 cô lập) · baseline walker JIT THẬT 352=42007/353=42 (MIR dump = `__triet_hashmap_get(struct_key)`) · ptr-mix hash → 42 vacuous · **diff BASE-vs-committed = CHỈ 1 doc-comment → logic byte-identical, máu áp verbatim** (khỏi verify lại logic). Gate độc lập cuối `1c08a67`: 0·0·347·0.

**Công D:** report (d) TRUNG THỰC (tự khai ptr-mix vacuous, pivot eq-poison thay vì ngụy trang test mù — mẫu #14 xử ĐÚNG lần này); qualify tooth #5 cap-lớn chuẩn; nhiều lần quota-cut vẫn commit WIP không mất code. Defect duy nhất = comment dối (sửa 1 vòng, KHÔNG đụng logic).

**Nợ Slice 2 defer (🚩 cắm cờ, campaign-nền riêng):** Enum-key (discriminant + padding-bits rác + variant-size — cô lập) · Nullable-leaf key · hash-caching · white-box walker-output hash tooth (cap-lớn direct-assert). **⚠️ BOM FIX-2 zero-@8 (Slice B) GIỮ NGUYÊN, chưa đụng.** ⚰️ ADR-0068 Box CẤM CỬA.

## ✅ ĐÓNG — `Vector::pop_front` (ADR-0082 B-α continuation, O+G ký 2026-07-12, PUSHED)
origin/main = `5462c5b`, gate `0·0·345·0`. 1 commit gộp code+counting-tooth (G lệnh "test đi liền code"). Move-out phần tử **ĐẦU** by-value (`T?`), sibling của `pop` (back). **KHÔNG ADR mới.**

**Recon lật khung bàn giao:** danh sách "get-by-value/pop-front/drain — continuation rủi ro thấp" là **vỏ bọc sai**. O recon phơi: `pop_front`/`drain` KHÔNG tồn tại surface (0 shim/typecheck); `get-by-value` aggregate = **deep-clone** (elem ở lại collection → nhân đôi heap) đụng move-only ADR-0042 + coupled ADR-0081 FROZEN → cần ADR Copy/Clone; `drain` đòi iteration protocol → cần ADR. **Chỉ pop_front là continuation thật** (tái dùng ABI D-1). G duyệt thu hẹp scope, TRỤC XUẤT get-by-value + drain thành campaign-nền riêng đóng băng. **O đính chính G "bổ sung token/AST surface":** `pop`/`push`/`get` = builtin-identifier gọi `pop(v)` (fixture 319:11), KHÔNG keyword/method → `pop_front` chỉ declare typecheck env, **0 dòng lexer/parser/schema**. G: "cố tình nhắc AST để xem tụi bay có nhìn architecture không".

**Semantics (G chốt):** O(n) shift **XUỐNG** `[1..len]→[0..len-1]`, `len--` tombstone. KHÔNG ring-buffer (phá INV-B-α "một layout hai nhà" + đập lại alloc/get/push/pop shim). Muốn queue O(1) → sau đẻ type `Queue`. Doc-comment shim ghi "no O(1) promise".

**7 site (grep-verified 6 ABI-site `__triet_vector_pop` mirror đủ, 0 sót — mandate G "sót 1 line gạch PR"):** env.rs declare · lower arm (dest `Nullable` thừa hưởng tag-prepend) · `mir/lib.rs` BuiltinShimMeta `mutates_arg=Some(0)`→**E2440 borrowck** · jit:3084 fat-gate · jit:3518 arg-vals out_ptr · shim `__triet_vector_pop_front:4833` · driver ShimSymbol + integration harness. `⑤⑥⑦` D grep tự phơi = wiring THẬT (D phán đoán đúng). **Shim:** B1 rút[0] TRƯỚC shift (fat→`copy_nonoverlapping`→out_ptr disjoint) · B2 `ptr::copy` memmove (overlap len≥3) · B3 `len--` no-zero (slot cuối rác nhưng ngoài drop-set).

**O poison máu độc lập (cp-snapshot KHÔNG git-checkout, restore md5 `d90caa4f` mọi vòng):**
- **T-G1 order (mandate):** push 1,2,3 → `pop_front`(1)·`pop`(3)·`pop_front`(2) = **132**. Shift giữ survivor giữa; interleave front/back len nhất quán.
- **T-G2a `len--` (mandate):** bỏ len-- → fixture 351 fat → **SIGABRT 134** `free(): double free`. Tombstone load-bearing. (350 scalar EXIT 0 — không heap không manifest, đúng failure-mode.)
- **T-O1 site-3 fat-gate:** bỏ pop_front khỏi `vector_pop_fat` → **JIT compile-refuse** "unexpected String return" (fat-return-without-slot bắt ở JIT-compile, KHÔNG SIGSEGV runtime như O dự — ghi đúng failure-mode). Site-3 load-bearing.
- **🩸 O BẮT LỖ VÒNG-1 (chặn ký):** D chỉ wire pop_front vào **fixture-harness** (integration) = bắt crash+sai-giá-trị nhưng **LEAK CÂM** (không free=không crash); B2 shift là **code MỚI** không counting-net thường trực. O trả D thêm **counting-tooth `vector_string_pop_front_then_drop_no_double_free`** (`typed_vector_counting.rs`, mirror pop): push 3/pop_front 1/drop → **FREE==3**. O tự poison len-- độc lập → **FREE 4≠3 RED** (non-vacuous); control pop-back tooth **XANH** = cô lập pop_front-only. D parameterize `build_push_pop_drop(…,pop_shim)` (4 caller cũ bất biến) = refactor sạch, báo trước.
- **⚔ T-G2b memmove — BÁO TRUNG THỰC NON-MANIFEST:** đổi `ptr::copy`→`copy_nonoverlapping`, len3 → **KHÔNG đỏ** (350→103, 351→0). Lý do: front-pop shift **XUỐNG** (dst=`data` < src=`data+stride`) → copy tiến không đè byte chưa đọc → memcpy-safe hướng này. **KHÔNG giả đỏ.** Nhưng overlapping `copy_nonoverlapping` là **UB theo hợp đồng Rust bất kể manifest** → `ptr::copy` GIỮ (UB-hygiene). G: "nếu giả vờ báo đỏ cờ này tao đã tế sống — giữ memmove là chuẩn mực kỹ sư Rust".

**Nợ campaign-nền đóng băng (G chốt lôi ra Phase sau):** 🚩 **get-by-value** aggregate (ADR Copy/Clone move-only) · 🚩 **drain** (ADR Ownership-Iteration) · 🚩 **BOM FIX-2 zero-@8 Slice B** (coupling frontend refuse enum-payload multi-heap-leaf) · key-aggregate `HashMap<agg,_>` hash+eq đệ quy · get_ref V=Nullable · borrow-params `&+ T` · B-γ multi-reg return · AOT · self-host · Facade `public use`. ⚰️ ADR-0068 Box CẤM CỬA.

## ✅ ĐÓNG — VALUE MOVE-OUT AGGREGATE: Vector pop + HashMap remove by-value (ADR-0082 B-α §AMEND-2, O+G ký 2026-07-11, PUSHED)
origin/main = `3e0975d`, gate `0·0·340·0`. 4 commit: `03a7638`(D-1a) · `f2e8bd8`(D-1b) · `5644f6e`(D-2) · `3e0975d`(§AMEND-2). Khép **chiều XUẤT** (element ra khỏi collection by-value) — bù cho A/B/C chỉ phủ push+drop.

**Scope:** `Vector<T>` pop + `HashMap<K,V>` remove trả aggregate (Struct/Enum) by-value. Tách D-1 (Vector) / D-2 (HashMap) vì source-tombstone khác cơ chế (G lệnh TÁCH). D-1 tách tiếp D-1a (Enum, disc-sentinel) / D-1b (Struct, tag-prepend).

**Cơ chế (khắc §AMEND-2):**
- **Move-out tombstone contract (①):** Vector `len--` (`__triet_vector_pop`, cell không zero, len-- loại khỏi drop-set) · HashMap `state→2` shim + value-free-loop gate `state==1` (`emit_hashmap_value_free_loop:1441`). CẢ HAI load-bearing.
- **D-1b = fix Slice-A-BUG-1 THẬT (②):** pop-dest LUÔN `Nullable(Struct)` (`lib.rs:2460`) → slot tag-prepend `tag@0/fields@+8` (ADR-0076, `mir_lower.rs:1906`). Marshal cũ ghi fields@+0 → đè tag → free bậy. AM1 refuse (Slice B) KHÔNG che bug bất-khả-sửa mà che **tầng ABI CHƯA DỰNG**. D-1b dựng: out_ptr=`slot+8`, tag=`(ret==SENTINEL)?SENTINEL:1`@`slot+0`. Dest-bind fat DÙNG CHUNG `vector_pop_fat||hashmap_remove_fat` (`:3561`) → D-2 thừa hưởng, chỉ vá out_ptr riêng (`:3443` field_off=8+enum_slots).
- **State-gate no-zero decision (③):** HashMap remove KHÔNG zero value-cell (khác key path). G-MANDATE đòi chứng minh gate đủ chặt → GIỮ no-zero (perf).

**🎯 O TỰ THU HỒI BÁO ĐỘNG GIẢ (bài học nặng):** giữa verify D-1b, O hoảng "fixture 338 crash `free(): invalid pointer`" → REJECT hụt. SAI: chạy `./target/release/triet-driver` mà KHÔNG rebuild sau edit D = **STALE BINARY**. Rebuild sạch từ cây đang test → 338/T3/loop-reuse đúng hết, 3 vòng deterministic. **LUẬT KHẮC: verify-don't-trust áp cả lên executable — LUÔN rebuild từ cây đang test TRƯỚC khi chạy binary.** Nghi thức #1 mở rộng.

**⚔ O ÉP PRESENT-TAG LOAD-BEARING (bác ★SS(c) của D):** D-1b vòng-1 present-tag-write (tag=1 khi present) KHÔNG có teeth; D biện "rác stack hiếm khi trùng NULL_SENTINEL" = xác suất, không soundness. O ép: `while` back-edge tái dùng dest-slot → empty-pop để SENTINEL@tag → present-pop misroute nếu bỏ tag-write. (b) test-yếu KHÔNG phải (a) bất-khả. D vòng-2 thêm fixture loop-reuse 341/342 (Vector) + 345/346 (HashMap). Poison keep-stale (2 site CHUNG) → cả 4 đỏ (1→0), straight-line 338/339/343/344 không đổi.

**O 🩸 TEETH (poison-cemented, cp-snapshot restore md5 mọi vòng):**
- Vector (D-1): `len--`→FREE3 · T9-enum→SIGILL · field_off→corpus SIGABRT · present-tag 341/342→(1→0). md5 f44c1235(D-1a)/127b594e(D-1b).
- HashMap (D-2, **G-MANDATE**): **GATE-A** (value-loop `state==1`→`≥1`)→SIGSEGV · **GATE-B** (shim bỏ `state→2`)→double-free tcache SIGABRT · field_off→corpus 343 SIGABRT · present-tag 345/346→(1→0). md5 267f1cbb. **Cả hai gate ĐỎ QUẠCH → state-gate giữ vững → G duyệt no-zero-cell.**

**Công D ghi nhận:** tự sửa số Hollywood ==4→==3 TRƯỚC khi báo (ngấm [[feedback_failure_mode_precision]]) · tự bắt key-aggregate-remove-refuse thiếu JIT teeth (chết typecheck E1048) → thêm hand-built MIR · LUẬT 3 cleanup orphan.

**⚖ Commit history:** D-1 tách 2 commit sạch (D-1a enum / D-1b struct) bằng snapshot-swap (O tái dựng D-1a counting = head-483 + HEAD struct-refuse, verify green mỗi commit). D-2 một cục.

**Nợ chuyển tiếp (campaign riêng, phiên sau):** get-by-value aggregate + get_ref value-aggregate (Cụm D/ADR-0081 FROZEN) · key-aggregate `HashMap<aggregate,_>` hash+eq đệ quy · pop-front/drain · B-γ multi-reg return. Đều REFUSE tường minh có teeth canh.

---

## ✅ ĐÓNG — CỤM B Slice B: `Vector<Enum>` push+drop (ADR-0082 B-α continuation, G ký 2026-07-09, PUSHED)
origin/main = `c22da0a`, gate `0·0·331·0`. 8 commit: `c8b8aa6`(S1+S2) · `3bede0c`(S3) · `98a3be2`(AM1) · `a665e96`(AM2) · `a6a41c2`(FIX-1+FIX-2) · `638b455`(teeth) + 2 docs (`c22da0a` state).

**Scope:** enum by-value element của Vector (heap-payload variants), **push+drop SOUND**, **pop/by-value move-out REFUSE** (deferred). Tái dùng `emit_enum_drop_glue_at` (address-based, ACTIVE-arm tag-switch) + INV-B-α. **KHÔNG cần ADR mới.**

**Bản đồ O recon:** sizing đã có (`EnumLayout.total_size`), drop-glue đã có (`emit_enum_drop_glue_at`). Việc = S1 `vector_elem_size` Enum arm · S2 `emit_heap_free_at:1067` Enum branch (TRƯỚC `is_any_heap` early-return, DP-2) · S3 marshal enum-element đọc `enum_slots` KHÔNG `struct_slots`/Variable (5 site, mẫu `:3404`).

**🩸 BUG-1 (pop UB, PRE-EXISTING SLICE A) — O tự bắt qua tooth pop.** `Vector<UserStruct>` pop → double-free/invalid-pointer; **verify TÁI HIỆN trên binary `1e49058`** (worktree) → pre-existing, KHÔNG regression. Slice A teeth CHỈ push+drop, chưa từng test pop. "get-by-value/pop aggregate" = nợ DEFERRED nhưng **deferred-KHÔNG-refuse = UB câm shape P0**. **AM1 vá:** REFUSE `__triet_vector_pop` element Struct/Enum (message "deferred… recursive move-out tombstone"), rào cả A lẫn B. get-by-value đã bị typecheck chặn → pop = đường move-out DUY NHẤT lọt JIT. **AM2:** cắt 3 hunk pop-side S3 (enum_slots dead sau AM1), giữ S3a/S3b push.

**🎯 BUG-2 (push+drop UNSOUND, HAI bug che nhau) — poison-must-be-red CỨU MẠNG.** First-draft named-tooth O ĐẾM NHẦM (Drop(local) vs vector-drop) → 10/10 xanh GIẢ. **Chỉ vì poison S2 KHÔNG đỏ** (poison-insensitive) O mới đào: (1) **BUG-1b** `aggregate_needs_drop:1663` có nhánh Struct nhưng KHÔNG Enum → Enum rơi `is_any_heap()`=false → element-free loop bail `:1164` → S2 UNREACHABLE → **elements LEAK**. (2) **BUG-2b** enum named-local KHÔNG tombstone khi push-consume (`tombstone_slot_leaves` keyed struct_layouts, enum ∈ enum_layouts) → Drop(local) free lần 2. **Che nhau:** named-case local-drop free đúng cái vector leak → net 2 "giả sound", driver clean. Chứng minh: **enum-inline=0 vs struct-inline-CONTROL=2** (method validated). **FIX-1** aggregate_needs_drop Enum arm (any heap-bearing variant, đối xứng Struct + khớp filter emit_enum_drop_glue) · **FIX-2** zero payload ptr @base+8 tại arg-consume enum branch (đối xứng Deinit `:2138`, KHÔNG disc@0). Một commit hai fix (tránh trung gian double-free).

**⚠️ BOM HẸN GIỜ (coupling, cắm cờ):** FIX-2 zero-@8 ĐỦ CHỈ VÌ frontend refuse enum-payload multi-heap-leaf — O tự chọc verify: `V(Pair)` struct-payload → lower REFUSE · `V(String,String)` multi-field → parse REFUSE. Mọi heap payload reachable = single handle @8. Nếu refusal đó gỡ → FIX-2 phải walk MỌI leaf.

**O 11 TEETH poison-cemented** (cp-snapshot restore md5 khớp mọi vòng, ĐỘC LẬP): `vector_enum_inline_push_drop` (BUG-1 anchor, INLINE non-masking; poison FIX-1→**0 leak**) · `vector_enum_named_push_drop_no_double_free` (BUG-2 anchor; poison FIX-2→**4 double-free**, inline giữ 2 tách bạch) · `vector_{struct,enum}_pop_refused` (AM1; poison→struct-pop **compile-SUCCEEDS** phơi lỗ Slice A, `compile_expect_refuse` đăng ký `__triet_vector_pop` để refuse NON-vacuous) · active-arm=1 · scalar=0 · nest=2 · struct-control=2.

**Bài học phiên:** ① **poison-must-be-red là thứ CHẶN false-green** — O suýt cement named-tooth misattributed đúng shape P0; poison S2 không đỏ = tín hiệu đào. ② một tooth NAMED có thể maskable (local-drop giả vector-drop) → **INLINE anchor non-masking bắt buộc cho leak**. ③ `compile_expect_refuse` phải đăng ký shim của op bị refuse, nếu không bắt nhầm "missing-shim" = vacuous. ④ deferred PHẢI refuse (không chỉ "không làm") — Slice A pop là bằng chứng UB câm. ⑤ `aggregate_needs_drop`/tombstone/move-out phải phủ Enum ĐỐI XỨNG Struct. [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]]

**Nợ chuyển tiếp:** Slice C `HashMap<_,aggregate>` value (⚠️ value-free-loop có latent P0-shape cùng họ BUG-1 — recon `aggregate_needs_drop`+value-loop trước) · `Vector<aggregate>` pop/get-by-value move-out (recursive move-out-tombstone: dest leaf-marshal + buffer + source) · scalar-enum disc round-trip chưa observe source (nullable-enum-match chưa lower) · coupling FIX-2. Đều REFUSE tường minh có teeth canh.

---

## ✅ ĐÓNG — CỤM B Slice A: `Vector<UserStruct>` aggregate by-value element (ADR-0082 B-α §AMEND-1, G ký 2026-07-08, PUSHED)
origin/main = `1e49058`, gate `0·0·331·0`. 7 commit: ADR `2802ce0` + C1 `d1774a3` + C2 `c93b6b3` + C3 `6e01ef4` + C4 `90ce297` + C5 `67e18c9` + C6 `1e49058`.

**Mặt trận:** G tuyên "CỤM B — Native multi-field layout". O recon vạch mặt **CÁI BẪY "native layout"** = gộp 3 việc rủi ro/giá trị lệch trời vực → ép G/Giang chốt scope:
- **B-α (CHỌN):** struct/enum by-value làm element Vector/HashMap-value. NĂNG LỰC MỚI, rủi ro THẤP (cưỡi fat-element ABI ADR-0077 sẵn). = Slice A.
- **B-β (ĐẠP CHẾT):** gói sub-8B thật (Trit=1B). PHÁ value-model i64, chỉ mật độ. Refuse đầu cơ.
- **B-γ (defer vô thời hạn):** multi-reg struct return.

**INV-B-α (bất biến nền G khắc):** *một layout, hai nhà, byte-identical* — image struct trong cell collection = image trong StackSlot (cùng `StructLayout`, 8B-granular, `stride=total_size`). Giữ 8B-granular = SỐNG CÒN: drop-walk `collect_heap_leaves` tính offset từ `struct_layouts`; nếu cell≠stack → free ptr rác. Là quyết định BẢO THỦ (bảo vệ value-model), KHÔNG đại phẫu.

**Cỗ máy (80% tái dùng):** `collect_heap_leaves` (jit:433) recursive descent struct→leaf ĐÃ có cho stack; `emit_enum_drop_glue_at` (jit:1457) address-based. Slice A = 3 mối nối:
- **C1 body-threading** (`d1774a3`): thread `body:&Body` qua free-fn family (`emit_heap_free_at`/`emit_vector_free_value`/`emit_vector_element_free_loop`/`emit_hashmap_free_value`) — JitContext KHÔNG cache layouts global, phải thread. Gate byte-identical.
- **C2 T7** (`c93b6b3`): trích helper `tombstone_slot_leaves` dùng chung Deinit (1938) + M3 (3436) — cặp song-sinh Drop-walk (G mandate "free N tiers → zero N tiers").
- **C3 T2+T8** (`6e01ef4`): `vector_elem_size(body,Struct)`→total_size (Enum vẫn Err=Slice B); `refuse_hashmap_aggregate_kv` wired 5 site.
- **C4 T3/T4/T5** (`90ce297`): `emit_struct_drop_glue_at` + `emit_heap_free_at` nhánh Struct TRƯỚC early-return (DP-2) + `aggregate_needs_drop` guard (DP-1, Copy-struct→rỗng→no-op).

**§AMEND-1 — 2 lỗ ngoài touch-list D bắt ở T0 probe (O rule SAU chữ ký G):**
1. **§3 CÓ LỖ (O tự ăn):** O verify "MOVE byte-wise generalize verbatim" chỉ ở tầng shim runtime, BỎ SÓT M3 zero-guard compile-time (`3436` String-only) → struct-arg-consumed rơi `def_var(var,zero)` (zero Variable, KHÔNG zero slot leaves) → Drop(struct) đọc SLOT → **double-free 134**. T7 vá (commit tách latent-proof: trước T2 struct bị refuse ở vector_elem_size nên đường chưa reachable).
2. **`vector_elem_size` dùng chung Vector+HashMap:** mở Struct → `HashMap<Integer,User>` marshal-reachable NHƯNG value-free-loop guard (`1286`) vẫn `is_any_heap` → skip struct → **LEAK câm** (đúng P0-shape ADR-0080). T8 refuse tường minh giữ biên Slice C.

**🎯 O TỰ BẮT BUG GATE 331-FIXTURE BỎ LỌT (T9, bằng chứng sống mandate G):** poison-teeth O viết (`vector_userstruct_counting.rs`) lôi ra **leak câm 8B-heap-struct** — struct `total_size==8` (bọc đúng 1 Vector/HashMap handle) → `stride==8` → push nhánh scalar `use_var(self.var(elem))` đọc **Cranelift Variable** (chưa def cho struct-local) thay **struct-slot** → buffer nhận 0 → drop free 0 → leak. **ÁN-LỆ:** struct-local sống ở StackSlot KHÔNG Variable; đọc 8B struct = `stack_load(slot,0)` KHÔNG `use_var`. C5 T9 vá đối xứng push (`3189` stack_load) + pop (`3457` stack_store), mirror concat/bung_fields pattern.

**O 7 TEETH (C6 `1e49058`), 4 POISON-CEMENTED** (cp-snapshot, restore md5 khớp mọi vòng):
- T-DOUBLE (T7): healthy FREE==2 · poison revert M3→String-only → **FREE==4** double-free.
- T-LEAK (T5): poison guard→`is_any_heap` → **FREE==0** leak.
- T9-8B (T9): poison push→`use_var` → **FREE==0** leak.
- T8-refuse: poison neuter guard → **compile SUCCEEDED** (leak risk).
- + 3 positive: T-REFUSE-Enum (`Vector<Enum>`→JitError Slice B) · T-COPY (`Vector<Point>`→FREE==0 byte-compat) · T-NEST (`Vector<Tagged{Vector<String>}>`→FREE==2 recurse 2 tiers).

**Bài học phiên:** ① O verify cắt CẢ §3 của chính O (verify shim-runtime bỏ tầng M3 compile-time). ② một hàm size dùng-chung âm thầm mở 2 mặt trận. ③ D dừng-báo-O đúng luật ④ ở T0 (spike thấy bug → không tự nới scope). ④ 4-commit-slice T7-tách-trước honor mandate G. [[feedback_failure_mode_precision]] [[feedback_poison_must_be_red]]

**Nợ chuyển tiếp (đóng-gói-campaign-riêng):** Slice B `Vector<Enum>` · Slice C `HashMap<_,aggregate>` value · aggregate KEY (đòi hash+eq đệ quy) · get-by-value aggregate (dùng get_ref/pop) · B-β sub-8B (đạp chết) · B-γ multi-reg return (defer). Đều REFUSE tường minh có teeth canh.

---

## ✅ ĐÓNG — Read-side Cụm A: get-borrow generic-V + P0 String-key SIGSEGV (ADR-0079 §AMEND-1, G ký 2026-07-04, PUSHED)
origin/main = `96f4241`, gate `0·0·331·0`. feat `37a0723` + docs `96f4241`. **Read-side container khép hoàn toàn cho V=container.**

**A1 get-borrow generic-V:** env.rs 6 overload `get` V∈{Vector<Integer>,HashMap<Integer,Integer>} qua
Vector<V>/HashMap<Integer,V>/HashMap<String,V> → `(&0 V)?` zero-copy borrow. Read-only `len(inner)` sẵn.

**§AMEND-1 (O viết, G ký "Invariant là ĐỊNH LUẬT"):** JIT `__triet_{vector,hashmap}_get_ref` stride-conditional
deref — thin V (value_stride≤8, handle) → `*cell` (body_ptr); fat V (>8, String 24B) → cell (inline len/cap).
Giữ INVARIANT `&0 V` **bit-for-bit identical** dù lấy từ local hay get_ref. Nếu không: `__triet_vector_len`
mong body_ptr, get_ref trả cell_ptr → `len` đọc `*cell`=body_ptr=garbage. String thoát nạn vì fat-24B inline.
Accessor sẵn: `vector_stride` (jit:4018) · `hashmap_value_stride` (jit:4345). **Bác fix-consumer (sửa `len`
deref cell): phá `len(&0 v)` từ local (local truyền body_ptr).**

**⚔ O TỰ ĂN — recon "A1 thuần env.rs" SAI một nửa:** O ban đầu tuyên "A1 không chạm JIT, borrowck type-agnostic".
POISON-1 (content-read tooth, `len(ref_vec)`=3 chứ KHÔNG routing-only) phơi ra thin-handle indirection blocker.
D dừng đúng luật báo O. O nhận sai, KHÔNG đổ cho D. **Bài học: content-read tooth (đọc nội dung THẬT) > routing
tooth (chỉ present/absent) — routing xanh giả, release crash runtime.** [[feedback_poison_must_be_red]]

**P0 BÁO ĐỘNG ĐỎ — pre-existing String-key read SIGSEGV (latent từ ADR-0080 `381979e`):**
get/get_ref/contains nhận `&0 HashMap` (**Reference-wrapped**) ≠ insert (owned HashMap). key_stride extraction
(`mir_lower.rs:3175`) chỉ `nullable_payload().unwrap_or` → Reference không tới arm HashMap → **default key_stride=8**
→ String key (stride 24) marshal **by-value 8B** → hash đọc vùng nhớ rác → **SIGSEGV 139**. insert thoát vì §AMEND-1
ADR-0080 chọc thẳng insert-flow (owned map). Integer-key read chạy nhờ default-8 tình cờ đúng. **0 fixture đời nào
test String-key get/contains runtime → latent câm dưới chữ ký "KHÓA SỔ".** VÁ: unwrap `MirType::Reference { inner, .. }`
trước match HashMap. Root-cause O đào bằng đọc code (không probe mù). G đoán đúng 100% ("pass-by-value 8B kiểu Integer").

**❄️ A2 get-borrow-mutable (ADR-0081) FROZEN → đày Cụm D (Phase 3 Ownership):** `push`/`insert` là functional
(clone+free-old+trả handle MỚI) → mutate inner qua `&0 mutable` ĐÒI write-back handle vào cell → P1 CẤM write-back
(deref-assign chưa wire) ⇒ `&0 mutable V` **VACUOUS cho Vector/HashMap** (chỉ pop/remove shrink dùng được). G:
"không nửa vời, không lỗ ngách bẩn". Mở lại khi core có deref-assign + drop-in-place qua con trỏ. Kiến trúc mặt-borrowck
(returns_borrow_form + exclusive-loan conflict cả READ) đã đúng — vấn đề là core functional-mutate.

**🚫 V=Nullable REFUSE/defer** — lowerer chưa match `&0 Nullable<T>` (không có đường dùng inner). Refuse-over-guess.

**O verify máu (poison→RED độc lập, cp-snapshot restore md5 khớp):** POISON-1 stride-deref revert→garbage `94…` ·
POISON-P0 Reference-unwrap revert→SIGSEGV 139 · POISON overload-break 336/337→E1041. Fixtures 333-337 (5):
333 Int-key content-read(3) · 334 borrowck-track(E2440) · 335 P0 scalar String-key(142) · 336 String-key get_ref
Vector(2) · 337 String-key get_ref HashMap(1). Gate `0·0·331·0` CLEAN độc lập.

**⚠️ KỶ LUẬT D — bẻ lệnh trực tiếp G:** O ra lệnh "gỡ 2 String-key overload" (G ký "Integer-Key ONLY, merge tách");
D **tự quyết GIỮ** overload + gộp P0 (vì P0 làm chúng sound). Kỹ thuật đúng NHƯNG bẻ lệnh đã-ký + **thiếu fixture
heap-value String-key** (O phải tự probe mới biết len=2/1) — **lặp lại Y NGUYÊN tội lỗ P0 vừa vá**. G nuốt tức
chấp nhận scope rộng nhưng cảnh cáo thép: *"lần cuối dung túng ném-API-không-test, lần sau đuổi cổ"* + ép D bổ sung
336/337. [[colleague_d_persona]] [[feedback_failure_mode_precision]]

## ✅ ĐÓNG TRỌN — key-typed `HashMap<String,V>` (ADR-0080 + §AMEND-1, Author+O+G ký, PUSHED 2026-07-03(b))
origin/main = `381979e`, gate `0·0·326·0`. **Campaign Typed Collections P1 (A) KHÓA SỔ.** `HashMap<String,V>`
+ `HashMap<String,String>` (key ∥ value cùng heap) sound end-to-end từ `.tri` source → JIT real-allocator,
không rỉ một byte.

**ADR-0080** (`26452e0`) — O BÁC amend ADR-0038 (Comparable=`Ord` ≠ `Hash` — trộn = nát kiến trúc) + BÁC
`Hashable` trait (trait system mới Tier-1, dựng giờ sụp móng). ADR mới toanh. **D1** slot `key_stride` ∥
`value_stride` **24B fat** (BÁC 16B: `__triet_string_free` cần cap; String KHÔNG lưu len trên heap ADR-0049
§6.3 → slot phải chứa len để hash/eq); `key_stride∈{8,24}` kiêm discriminator. **D2/D3** `__triet_string_hash`
FNV-1a + `__triet_string_eq` sẵn, cấm dynamic dispatch. **D5** key∈{Integer,String}, khác→REFUSE. **Mũi D
nợ máu 5 death-point** — O vạch thêm **#5 remove-free-resident-key** ngoài 4 điểm Author: (1) map-drop free
key (2) insert-dup trảm key move-in dư (3) insert=Move key (4) get/remove/contains=borrow `&0` bất đối xứng
(5) remove free resident key.

**§AMEND-1** (`72bdf7e`) — **D lật vacuous-tooth** (recon KM-P1a): free viết TRỰC TIẾP trong thân Rust shim
= static link-time call, BYPASS JIT symbol-table (`with_shims:808` substitution) → counting harness MÙ →
teeth #2/#3 rỗng từ đầu. O verify độc lập (symbol-table + VALUE out_ptr precedent :2952) → nhận dao, retract
WO literal. Fix = out-param ABI: `is_update_out` (insert D.2) + `key_out_ptr` (remove D.5) → free đẩy ra JIT
call-site registry-routed, countable. Bất biến: resident key ≠ lookup key (cấm free `k`).

**KM-P1a backend** (`c003a5f`) — Mũi A slot 24B fat (header packing `reserved = key_stride<<16|value_stride`)
· B `__triet_string_hash` + `hashmap_key_hash/eq` dispatch runtime theo key_stride · D.1 `emit_hashmap_key_free_loop`
· D.2/D.5 out-param free registry-routed · rehash key-stride memcpy. Hand-built MIR + counting (source E1003
tới P1b). D tự bắt bug: key-free-loop compile-time đăng ký `__triet_string_free` cho MỌI map kể cả Integer →
3 test cũ vỡ → gate compile-time trên `key_ty`. **O 5 teeth poison→RED độc lập** (map-drop-leak 1→0 · update-leak
2→1 · remove-leak 1→0 · content-hash cap=1_000_003 · rehash key-stride→SENTINEL).

**KM-P1b source** (`381979e`) — C1 typecheck generic-K∈{Int,String} (`env.rs`) + String-key overload
get/len/contains/is_empty + get_ref parity · C2 **E1048 UnsupportedHashMapKey** hard-REFUSE (`exprs.rs:1011`
gate `sub_map["K"]∉{Int,String}`) · D3 borrowck insert `arg_consumes[true,true,true]` key=Move type-aware
(is_copy per-call, KHÔNG code mới) · D4 get/remove/contains giữ borrow `[false,false]`. **Lower-bug D vá thật**:
`lower_type`/`lower_type_simple` (triet-lower) hardcode Integer key vô điều kiện → `HashMap<String,V>` annotation
âm thầm rớt về Integer → đọc 1st type-arg. **Bug D tự bắt**: D3 phá D.2 KM-P1a (M3-zero chạy TRƯỚC free-redundant-
key → key dư leak) → đảo thứ tự D.2/D.5 trước M3 (regression #2 cũ verified vẫn RED). **O 7 teeth poison→RED
độc lập** (★SS(a) key-leak 2→1 · ★SS(b) value-leak 2→1 · ★SS(c) tombstone double-free SIGABRT 134 · #4 insert-Move
134 · #6 lookup-borrow E2420 · #8 E1048 non-vacuous Tryte+Struct · regr #2 D.2/M3-reorder).

**⚔ BÀI HỌC — O đính chính D ở ★SS(c)** (G khen "đỉnh cao verify-don't-trust"): D báo ★SS(c) "2 lớp phòng thủ
redundant, poison từng lớp đều sống, phải poison cả hai mới SIGABRT" → hạ chuẩn tooth xuống "chỉ chứng minh bất
biến ngoài". O KHÔNG nhận narrative — mổ độc lập: KEY path CÓ 2 lớp (state==1 check + `write_bytes` zero key cell
@4831), nhưng **VALUE path CHỈ 1 lớp** — remove memcpy value ra out_ptr mà KHÔNG zero value cell (không có
`write_bytes` đối xứng) → **value-loop state-check (`:1306`) LÀ load-bearing đơn lẻ**. Single-poison một dòng đó
→ SIGABRT 134. D under-analyze memory-model của chính mình, dừng ở (b)-tưởng-(a); O ép tiếp lộ yết hầu. Mẫu
[[feedback_poison_must_be_red]] + nghi thức O #4 (phân biệt defensive-vô-nghĩa vs hazard-thật bằng poison có máu).

**Defer Tầng-2+ (không hủy):** `HashMap<_,UserStruct>` P2 native-layout · get-clone/borrow heap value ·
get-borrow-mutable key · generic V-overload (P1 chỉ String) · hash caching · C native multi-field layout.
[[future_comparable_trait_and_monad_gap]] [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]]

## ✅ ĐÓNG — Bug-E: Outcome-param ABI + `~->` early-return heap double-free (O+G ký 2026-07-03)
origin/main = `81fae69`, gate `0·0·326·0`. Giang tự phát hiện viết
`examples/outcome_ternary_family.tri` (push thẳng main, ngoài session): truyền
`T~E`/`T?~E` làm tham số hàm → tính SAI LẶNG LẼ. G chốt silent-wrong-answer nặng
hơn crash → dừng A/C/D, dồn lực.

**WO1 param-ABI copy-in gap** (`ddb7841`): callee prologue cấp StackSlot rỗng cho
MỌI Outcome-typed local kể cả tham số (`mir_lower.rs:1453`); vòng bind tham số
(`:1644-1684`) có nhánh copy-in cho String/Enum nhưng THIẾU Outcome — con trỏ caller
(đã đúng, `:2676`) bị bỏ xó. Fixtures 328/329/330 (scalar/nullable/interleaved-offset).
⚠️ D dùng `git stash` so pre/post — vi phạm [[feedback_teeth_never_git_checkout]] lần
đầu, G ghi sổ đen, O verify lại độc lập bằng cp ra cùng kết luận.

**WO2 early-return heap double-free** (`818602c`), O tự mở rộng test ngoài phạm vi
WO1 (probe `String~Integer` param) → SIGABRT 134 → cô lập: bug KHÔNG cần tham số
hàm, tái hiện chỉ 1 local. 3 site cùng thiếu pattern HP.4
(`copy_heap_outcome_payload`/`bind_heap_outcome_payload` + `Deinit`):
- Site A `lib.rs:~5163` (success-arm passthrough unwrap, `~->` early-return)
- Site B `lib.rs:~5023` (error-arm bind `e`, `~->` early-return)
- Root cause CHUNG `lib.rs:~1947` (`Expr::OutcomeConstructor` heap-payload branch —
  dùng chung MỌI `~+ v`/`~- e` trong ngôn ngữ, vô hại literal/temp nhưng double-free
  khi payload là named-local có drop-obligation — đúng tình huống Site B tự tạo).

G ký mở rộng phạm vi tại chỗ (không phải đụng tủ khóa A/C/D — gốc rễ CHÍNH campaign
đang mở). Fixtures 331/332 (named-local, [[feedback_poison_must_be_red]]). O verify
máu ĐỘC LẬP cả 3 site — poison TỪNG site một: 5040→332 đỏ/331 không đổi ·
5176→331 đỏ/332 không đổi · 1957→332 đỏ (fixture-count tụt 258 vì TOÀN BỘ corpus
chạy chung 1 process, crash cắt cụt phần sau alphabet — KHÔNG hồi quy diện rộng, O
tự phân tích raw output xác minh). Restore md5 khớp mọi lần, gate CLEAN 326.

## ✅ ĐÓNG — Get-Borrow Heap Value (ADR-0079, G ký 2026-07-01, PUSHED `4fa0298`, gate 321)
`get(&0 container,k) → (&0 V)?` zero-copy borrow (P1 V=String), thay E1047 ở vị trí
mượn. Clone CẤM TIỆT (hidden alloc=rác). Mô hình loan: mượn 1 value = mượn CẢ
container (borrowck không đặt tên được `map[k]` qua hash-shim opaque → conservative
whole-container freeze). Not-found → nullable-borrow (NULL_SENTINEL, tái dùng PA-3c).

Slice A borrowck (`a970540`): U2 `returns_borrow_of` trên get_ref → PropagatedLoan
builtin (tái dùng ADR-0046) · U3 `mutates_arg` (remove/pop in-place) — active loan →
E2440. Slice B (`f57d9b8`): U1 overload concrete · U4 `__triet_{hashmap,vector}_get_ref`
shim zero-copy, not-found→NULL_SENTINEL · F-d Copy-source skip-conflict.
⚠️ 2 vòng O-reject: remove/pop lọt lưới (U3 ban đầu chỉ kiểm consume) → D thêm
`mutates_arg`. O verify: 5 borrowck teeth poison-sensitive + content-read
`length(ref_str)`→2/5 + fixture 327 content-read guard (325/326 chỉ ROUTE không đọc
content — bài học lặp từ HM-P1b fx322). Defer: generic V-overload (P1 chỉ String) ·
get-borrow-mutable · key-typed.

## ✅ ĐÓNG — Typed HashMap P1 trọn vẹn (ADR-0078, G ký 2026-07-01, gate 318)
`HashMap<Integer,V>` (V heap) sound end-to-end qua JIT real-allocator:
insert(Move)/remove(move-out `V?`)/drop. HM-P1b typecheck-open (`f5c11e1`+`2f100fb`):
dedicated `Type::HashMap(K,V)` (đập UserStruct) + generic `hashmap_new<V>`/`insert<V>`/
`remove<V>` (key=Integer cứng, seed V từ expected_type_stack) + get-heap E1047 +
insert=Move. ⚠️ 3 vòng O-reject: (1) garbage non-det — `lower_type`/`lower_type_simple`
hard-code `HashMap(Integer,Integer)` bỏ value-arg → stride=8 → fat String đọc rác;
(2) vacuous-tooth — SIGABRT 134 dùng String LITERAL = temporary KHÔNG drop-obligation
→ poison TRƠ; O chứng minh bằng MIR (literal KHÔNG Drop, named-local CÓ) — LUẬT
NAMED-LOCAL khắc đá; (3) sạch.

HM-P1a storage backend (`a0e60d8`, gate 315): value-typed `HashMap<Integer,T>` (T
heap) machinery sound (ngủ đông — source E1003 lúc đó, proven hand-built MIR).
MirType::HashMap(Box<K>,Box<V>) · slot value-stride inline stride-in-header ·
JIT-emitted free-loop registry-routed · remove shim move-out tombstone + out-ptr-
sentinel. 3 tầng độ khó: T1 value=Vector-reuse · T2 key-typed=hash/eq MỚI (DEFER,
đúng mặt trận A vừa chốt) · T3 typecheck UserStruct→dedicated Type::HashMap. ⚠️ 3
vòng reject: phantom hash · tooth VACUOUS fat-rehash 0 test · 17 clippy dán nhãn
"pre-existing" sai.

## ✅ ĐÓNG — Typed Vector P1 trọn vẹn (ADR-0077, G ký 2026-06-30, gate 312/315)
`Vector<T>` (String/Vector/HashMap/Nullable element) construct+push+pop+drop sound
end-to-end. Element-SIZE built-in = HẰNG compile-time (tách-tầng khỏi native-layout),
REFUSE Vector<UserStruct/Enum> ở biên P1. Slice A backend (`76405aa`): MirType::Vector
→Vector(Box) · stride-in-header · JIT-emitted element-free loop (chống vacuity, D bắt
shim-internal free bỏ qua registry) · by-ptr fat ABI + pop shim. Slice B typecheck-open
(`951790e`): tái dùng máy generic-fn v0.7.4.1 (extract_type_params+substitute, KHÔNG
HM-unify) · get-heap→E1047 refuse · push=Move. P1.5 pop-wire (`1977a93`, gate 315): 3
nối dây frontend + bugfix D tự phát hiện (empty-fat-pop ghi NULL_SENTINEL vào out_ptr).
O nhiều teeth SIGABRT 134 real-allocator (poison consume/len--/sentinel).

[[feedback_poison_must_be_red]] [[feedback_teeth_never_git_checkout]]
[[feedback_failure_mode_precision]] [[mentor_o_persona]] [[colleague_d_persona]]

## 2026-07-10 — CỤM B SLICE C: `HashMap<K,aggregate>` VALUE (ADR-0082 B-α cont., G ký, PUSHED)
origin/main `6d9e144`, gate `0·0·331·0`. 3 commit: `6ec2630`(F1–F4 + T4 unit) · `36ba45f`(teeth) · `6d9e144`(docs). **Scope:** value-aggregate (Struct/Enum) **insert+drop+alloc SOUND** (mirror Slice A/B element push+drop); get/get_ref/contains/remove + key-aggregate REFUSE.
**4 fix / 4 MÌN (recon O, file:line thật):**
- F1 `emit_hashmap_value_free_loop:1387` guard `is_any_heap()`→`aggregate_needs_drop` (Struct/Enum ≠ is_any_heap → guard phẳng bail → leak; mirror Vector element loop 1186).
- F2 `aggregate_needs_drop` Enum-arm: `for`-loop đệ quy + `?` thay `.any(payload.ty.is_any_heap())` phẳng — **defense-in-depth LATENT** (frontend refuse enum-payload-aggregate; unit test T4 pin trực tiếp trên hand-built EnumLayout, bypass frontend).
- F3 marshal `hashmap_insert` value HAI ĐẦU S3-gap (đối xứng vector_push 3255–3280): ĐẦU-A fat (>8B) value ở `enum_slots` không chỉ `struct_slots`; **ĐẦU-B** 8B-aggregate value (ôm 1 handle, stride==8) → `stack_load(slot,0)` KHÔNG `use_var` (else-branch cũ đọc Variable rỗng → garbage → leak câm; C5/T9 Slice A/B tái sinh).
- F4 refuse tách: helper mới `refuse_hashmap_aggregate_key` (key-only) @alloc(3239)+insert(3296); giữ `refuse_hashmap_aggregate_kv` (K+V) @remove-probe(3073)+remove(3359)+get-family(3431). WO gốc G nói 3 site, O đếm ra 5.
**🩸 O tự bắt lỗ G bỏ sót ở WO = MÌN-3 ĐẦU-B** (8B value ôm handle → use_var garbage → LEAK CÂM, 331 fixture không thấy) → tooth T3 riêng.
**⚖ D "lệch lệnh" có tri thức (G duyệt):** get/get_ref/contains/key chết ở typecheck (E1041 NoMatchingOverload/E1002 undefined/E1048) → JIT-refuse = defense-in-depth → hand-built MIR (án-lệ ADR-0078); chỉ remove chạm JIT. **O probe 5 `.tri` source độc lập verify = đúng tuyệt đối.**
**O verify 4+1 poison→RED độc lập** (cp-snapshot restore md5 `62ab04…`): F1→T1/T2/T3 FREE `0 vs 2` · F2→T4 `needs_drop==false` · F3-ĐẦU-A→T2 compile-fail "fat value without slot" · F3-ĐẦU-B→T3 FREE 0 (chỉ T3 → INLINE-anchor cô lập) · neuter 2 refuse-helper→6 refuse tooth "compilation SUCCEEDED". Failure-mode = FREE-count-wrong (leak, KHÔNG SIGSEGV).
**Teeth:** T1 `hashmap_struct_value_insert_drop_frees_string_field` · T2 `hashmap_enum_value_insert_drop_frees_string_payload` · T3 `hashmap_8b_struct_value_insert_drop_frees_wrapped_vector` · T4 unit `aggregate_needs_drop_enum_recurses_into_struct_payload` · 6 refuse (remove source-level + get/get_ref/contains/key-alloc/key-insert hand-built MIR). Repurpose `hashmap_struct_value_refused_at_jit`→`..._remove_refused_at_jit` (Luật 3; coverage insert-Struct-value→T1).
**⚠️ Bom hẹn giờ FIX-2 zero-@8 (Slice B) giữ nguyên.** **Nợ Slice C defer:** value move-out (get/remove by-value — nấm mồ chung Vector pop) · get_ref borrow value-aggregate (Cụm D) · contains-allow value-aggregate · key-aggregate hash+eq đệ quy.
**Mặt trận kế:** value move-out aggregate (recursive move-out-tombstone: dest leaf-marshal + buffer/cell tombstone + source) HOẶC key-aggregate — G/Giang chốt.
