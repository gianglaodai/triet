---
name: campaign_typed_collections
description: "✅ Typed Vector/HashMap P1 (ADR-0077/0078) + Get-Borrow (ADR-0079) + Bug-E + key-typed HashMap<String,V> (ADR-0080) + Read-side Cụm A (String-key SIGSEGV) + CỤM B Slice A Vector<UserStruct> + Slice B Vector<Enum> push+drop (ADR-0082 B-α) — KHÓA SỔ 2026-07-09 `c22da0a`. Full detail, MEMORY.md index only links here."
metadata: 
  node_type: memory
  type: project
  originSessionId: ac639140-8210-42c9-941b-8cfd203d270e
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
