---
name: handoff-2026-07-16-getref-campaign
description: "Phiên 2026-07-16 — 5 commit pushed (enum-payload sizing, D1, D2, get_ref Slice 1a+1b). Slice 2 ĐÃ G-KÝ, sẵn giao D. Backlog parked."
metadata: 
  node_type: memory
  type: project
  originSessionId: 8ee07cdd-6a77-4e51-9e53-9eee20fbfe63
---

# Bàn giao 2026-07-16 — Campaign get_ref value-aggregate (Front 2 của E1049)

origin/main = `006b6c7` (synced sạch). Gate `0·0·381·0`. Bắt đầu phiên: `bf2ed16`.

## 5 commit pushed phiên này (theo thứ tự)
1. `9a1799c` **feat ADR-0067 §AMEND enum-payload-aggregate sizing** — co-fixpoint struct+enum trong triet-lower; enum variant ôm Copy-aggregate payload >8B đã constructible nhưng mis-sized (chốt 8B) → memory-stomp (poison O: 2 enum-16B kề → SIGILL 132). Fix: `resolve_aggregate_size` shared + Gauss-Seidel co-fixpoint (cap-64→Err). Lift E1048 (aggregate enum-key hashable). ABI không đổi. Fixtures 368-373.
2. `51f1da7` **fix refuse nullable-enum-payload (ADR-0065 §12.7)** — `E?` với enum có BẤT KỲ payload variant (scalar 16B/aggregate 24B) → SIGILL 132 (disc-niche chỉ implement unit-only). Refuse tại `OutcomeConstructor` nullable chokepoint (`lib.rs:1898`). Unit-only `E?` giữ (249/250). Fixtures 374-377. **Chẩn đoán D1 ban đầu của O SAI** (fix 1-dòng struct_map→enum_map inert; bug sâu hơn+rộng hơn) — O tự đính chính qua verify-before-WO.
3. `219dc56` **fix D2 SwitchInt synth_base collision (P0)** — MỌI hàm ≥2 multi-case match → Cranelift verifier crash exit 4 (KHÔNG chỉ nested). Root `mir_lower.rs:4663`: synth_base dùng chung `cfg.blocks.len()` mọi switch → switch#2 đè synthetic của switch#1. Fix: map `switch_synth_base` per-block. Fixtures 378-380 + inline lại 369/371/372 (bỏ workaround tách-hàm). D tự bắt biên `n_cases==1` (O quên trong WO) → Option lazy.
4. `d02c0c4` **feat ADR-0084 Slice 1a** — scalar field-read qua `&0`: `(&0 Point).x`→value. Auto-deref 1-tầng scalar-only + Projection::Deref wire + **Blocker-B vá** (`Statement::Borrow` stack_addr mọi struct/enum local, không chỉ String — trước SIGSEGV). Fixtures 381/382. WART: borrowck LEXICAL (không NLL); borrow local phải chết trước return owner (block/param) → E2450 (ADR-0046 21/24). NLL = hố đen defer.
5. `006b6c7` **feat ADR-0084 Slice 1b** — sub-borrow aggregate/heap field: `(&0 h).name`→`&0 String` zero-copy (pointer-arith, 0 copy). 4 tầng: typecheck (aggregate/heap→Reference), lowerer (Borrow [Deref,Field]), JIT (walk_projections offset+base-addr), borrowck (whole-object fallback + **reborrow-chase** neo loan lên h thật thay tmp). Fixtures 383-387. **Nuance O verify:** 386 assert E2450 (từ chase) nhưng chốt robust là E2400 (return-inference độc lập); chase load-bearing cho E2450 (cargo-test poison), KHÔNG cho soundness — nhưng đúng-nguyên-tắc + forward-looking Slice 2. **O nghi chase là dead-code (dùng binary) → cargo-test đính chính → tự thừa nhận sai.** D minh bạch.

## 🎯 SLICE 2 — ĐÃ G-KÝ DUYỆT, SẴN GIAO D (việc đầu phiên sau)
Trận cuối quét E1041/E1049. **Recon O xong, G ký, CHƯA soạn WO cho D.**

**Bản chất:** hạ tầng đã sẵn — chỉ "bóp cò". Shim `__triet_{vector,hashmap}_get_ref` TYPE-AGNOSTIC (slot-ptr 8B, ADR-0079 "no JIT change"); container loan `returns_borrow_of` (MIR Body meta, `checker.rs:1199`) đã khóa mutate-while-borrowed E2440 cho heap V (ADR-0079 U3 tested); Slice 1b sub-borrow đọc field xong.

**3 tầng việc:**
1. **Typecheck dispatch** (`exprs.rs:~1222` arm get_ref + arm Vector ~1241): mở get_ref cho aggregate V (UserStruct/UserEnum) → `(&0 V)?`. Hiện chỉ `v.is_heap()`. ⚠️ **RANH GIỚI TỬ THẦN (G nhấn):** phân nhánh `is_ref`→get_ref vs non-ref→get-by-value §AMEND-3. **TUYỆT ĐỐI KHÔNG trộn** (get_ref lén copy = phá zero-copy). Phủ `get(&0 Vector<Agg>,i)` + `get(&0 HashMap<K,Agg>,k)`.
2. **JIT call-lowering:** đảm bảo 100% aggregate-V `&0`-form → `__triet_*_get_ref` (cell_ptr element trong buffer), **KHÔNG get_copy** (G: "lệch get_copy là cạo đầu"). Stride element = total_size (đúng sau ADR-0067 §AMEND).
3. **Borrowck:** set `returns_borrow_of` cho overload aggregate-V get_ref → loan phủ container.

**TEETH G ĐÒI (đẫm máu):**
- **Poison-1 (nỗi lo G):** `let r=get(&0 c,k); c.remove(k)/pop(c); r.field` → E2440. Poison = gỡ `returns_borrow_of` → phải thấy SIGSEGV lọt (chứng minh meta = khiên sinh tử).
- **Poison-2 (chống copy-lén):** ép JIT chạy get_copy thay get_ref → đỏ (E2440/crash) → chứng minh đúng shim.
- **Khải Hoàn Môn (positive, bia mộ E1049):** `Vector<Tagged{String}>` lấy `&0 Tagged` → đọc `.name` length. **remove là HashMap-only (`remove(HashMap,K)→V? mutate in-place`); Vector dùng pop/push.**
- ADR: **§AMEND ADR-0079** (mở get_ref aggregate V), compose ADR-0084. KHÔNG ADR mới.

## Backlog PARKED (chờ G+Giang mở)
- 🔴 **Deep-Clone heap-bearing aggregate** (Front 1, G park — "đường kẻ lười, cắn RAM"; get_ref là zero-copy đúng triết lý). ADR `.clone()` tường minh + carve-out ADR-0042 move-only + codegen clone đệ quy. Mở get-by-value heap-bearing (E1049 REFUSE).
- **full-support nullable-enum-payload** (G park — có workaround sound: struct-wrap `W?` / thêm variant None). line-491 `Nullable(Enum)→struct_map` (nên enum_map) latent-masked bởi D1 refuse, gấp vào full-support (untestable standalone ở .tri).
- **Borrowck NLL/lexical wart** (G: defer vô thời hạn, KHÔNG đụng ADR-0046/flush_all_for_return).
- drain (ADR Iteration) · contains-allow value-aggregate · get_ref V=Nullable · hash caching · borrow-params `&+ T` · AOT · self-host · Facade public use. ⚰️ ADR-0068 Box CẤM CỬA.

[[campaign_typed_collections]] [[feedback_poison_must_be_red]] [[feedback_verify_producer_before_consumer]] [[colleague_d_persona]] [[mentor_o_persona]]
