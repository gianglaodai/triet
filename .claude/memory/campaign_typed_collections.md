---
name: campaign_typed_collections
description: "Typed Vector/HashMap P1 (ADR-0077/0078) + Get-Borrow Heap Value (ADR-0079) + Bug-E Outcome-param/early-return double-free — full detail, MEMORY.md index only links here."
metadata: 
  node_type: memory
  type: project
  originSessionId: ac639140-8210-42c9-941b-8cfd203d270e
---

## 🎯 MẶT TRẬN A ĐANG MỞ — key-typed `HashMap<String,V>` — ADR-0080 APPROVED, WO KM-P1a phát (2026-07-03)
**ADR-0080 `docs/decisions/0080-hashmap-string-key.md` — Author+O+G ký, PUSHED `26452e0` (origin/main).**
O BÁC amend ADR-0038 (Comparable=`Ord`, không phải `Hash` — trộn = nát kiến trúc); ADR MỚI toanh,
BÁC `Hashable` trait (trait system mới Tier-1). **Quyết chốt D1–D5:**
- **D1** slot: `key_stride` song song `value_stride`, **24B fat trọn ổ** (`{ptr,len,cap}`). BÁC 16B
  (`__triet_string_free` cần cap thật). Lý do vật lý bắt buộc 24B: String KHÔNG lưu `len` trên heap
  (ADR-0049 §6.3) → slot phải chứa len để hash/eq. `key_stride∈{8,24}` **kiêm discriminator** (8=Integer
  identity giữ byte-compat, 24=String content).
- **D2/D3** shim MỚI `__triet_string_hash(ptr,len)` FNV-1a (mirror `cap_id_hash` mir_lower.rs:3372);
  eq tái dùng `__triet_string_eq`:3542 (đã có). Cấm dynamic dispatch.
- **D5** key ∈ {Integer,String} đóng băng; khác → REFUSE typecheck.
- **Mũi D nợ máu — 5 death-point** (Giang: TỐI QUAN TRỌNG). O vạch thêm **#5 remove-free-resident-key**
  ngoài 4 điểm Author list: (1) map-drop free mọi key, (2) insert-dup trảm key move-in dư, (3) insert=Move
  key consume, (4) get/remove/contains = borrow `&0 String` (bất đối xứng), (5) remove free resident key.
- **Teeth split 2 slice:** KM-P1a (backend/shim, hand-built MIR + counting) dập máu **#1 drop-leak /
  #2 update-leak / #3 remove-leak / #5 content-hash / #7 rehash-stride**. **#4 (Move) / #6 (borrow) /
  #8 (REFUSE) / #9 (source compat)** = KM-P1b (typecheck/borrowck). Giang đòi báo cáo đẫm máu #1→#7
  → land SAU KM-P1b. Poison BẮT BUỘC (G lệnh): Map-drop-leak + Update-leak, đo FNV counting FREE-count.

**WO KM-P1a đã phát cho D** (file `crates/triet-jit/src/mir_lower.rs`): Mũi A slot + B hash/eq shim +
D.1/D.2/D.5 key-free/dup-trảm/remove-free + rehash key-stride. Verify hand-built MIR + counting (lối
HM-P1a `a0e60d8` ngủ đông proven-MIR, source E1003 tới khi P1b mở). Chờ D nộp cây + raw gate → O verify máu.
Sites: slot helpers :4054/4068/4075/4082, alloc :4110, insert :4186, get :4284, remove :4363,
rehash :4211-4238, value-free-loop mẫu emit_hashmap_value_free_loop :1133.
C (native multi-field layout) + D (get-borrow-mutable) vẫn lùi, không hủy. [[future_comparable_trait_and_monad_gap]]

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
