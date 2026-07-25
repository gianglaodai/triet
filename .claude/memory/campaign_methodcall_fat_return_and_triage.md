---
name: campaign_methodcall_fat_return_and_triage
description: "🏁 WO-MethodCallFatReturn (ADR-0065 §14.7 AMEND đóng copy #3 is_fat_ret) + TRIAGE toàn sổ nợ 2026-07-25(h). O tự bắt 1 false-alarm (P1) + 1 zombie-label (KM-P1b)."
metadata: 
  node_type: memory
  type: project
  originSessionId: bea909b1-7952-44fb-9f1e-e3c4d5ba9035
  modified: 2026-07-25T18:51:20.433Z
---

# WO-MethodCallFatReturn + Triage sổ nợ — phiên 2026-07-25(h)

origin/main **`45919cd`** (synced), gate `0·clean·0·463·0`. 2 commit: `c88a5ae` (WO) + `45919cd` (triage sổ).

## 🏁 WO-MethodCallFatReturn (ADR-0065 §14.7 AMEND — đóng copy #3)

**Vấn đề:** `is_fat_ret` có **3 bản sao** (§14.7): #1 callee `Ctx::new :446` · #2 caller `Expr::Call :3358` · #3
caller `Expr::MethodCall :5501`. Copy #1/#2 unwrap `Nullable` → xử `Enum`/`Struct?`/`Enum?` return qua sret;
**copy #3 KHÔNG unwrap** (`matches!(callee_ret, Struct(_))` trần, thiếu `is_enum_ret`) → ba shape đó rơi
**E1100 over-refuse**. Fail-closed (Err trước emit MIR, comment `:5493` cố ý refuse-over-guess) → **KHÔNG UB**.
Fixtures 448/453 là **tripwire cố ý** phiên 07-20 (EXPECT=ERROR, "đừng fix :5219").

**Fix:** mirror copy #2 vào copy #3 — thêm `is_enum_ret`+unwrap `Nullable` ở `is_fat_ret`; `sret_layout_name`
unwrap inner; nhánh `is_enum_ret → EnumAlloc`+`ReturnShape::Enum`; message refuse bỏ chữ "Enum". Args ordering
copy #3 `[sret, receiver, explicit]` giữ nguyên. **Vector/HashMap/Reference GIỮ refuse** (biên đỏ G).

**Teeth O (poison ĐỘC LẬP granular — mạnh hơn revert-blanket của D):** Poison A neuter `is_enum_ret=false`
→ 453+469 refuse, **448 pass** (enum-path độc lập struct); Poison B struct-unwrap→match-trần → 448 refuse,
**453+469 pass** (struct-path độc lập enum). Hai path **orthogonal** — không fixture nào cưỡi fix cái khác.
Biên đỏ probe: `Vector<Integer>?` method-return → E1100 (unwrap không rò). Fixtures 448→10, 453→5, 469
bare-Enum exhaustive-match→5. D khai 1 lệch thật (fixture 179 substring message). D=Sonnet 5, 0 bế tắc.

## 📋 TRIAGE TOÀN SỔ NỢ (Giang lệnh trước close-session, 5 mũi recon song song)

**Không khoản nào là hố UB sống — tất cả fail-closed.** Kết quả (verify code+probe HEAD `c88a5ae`):

- **🩸 P1 Vector-scalar-return (O tự cắm phiên này) = FALSE ALARM.** Vector/HashMap = **single-i64 handle**
  (con trỏ→heap-buffer 3-field), KHÁC String (3-field trong slot 24B→sret). `ReturnShape::Scalar` ĐÚNG;
  caller `:3507`+callee `:466` **đối xứng Scalar**. Probe `make()->Vector`→42, fixture 166→3. **Rút cờ.**
  O cắm do đọc-code-nửa-vời không probe — mẫu "kết luận trước khi đo", refuse-over-guess áp cả claim O.
- **KM-P1b HashMap<String,V> = ZOMBIE LABEL.** Đã đóng `381979e`+KHÓA SỔ; TODO ghi "[ ] D đang code" stale.
  Probe String-key insert+get→1, E1048 wired. Tick [x] + close.
- **N1 widening = NHÃN LỆCH.** Không "refuse E1120 sạch" — E1120 **LỌT** ở widening/fast-path
  (`let x:E?=E::V(42)`/`=~0` exit 0). **G phân loại POLICY-HOLE KHÔNG UB** (đo 2026-07-20, ADR-0065 §13).
- **Panic Nhóm C (layout `.unwrap()`) = ĐO reachability → KHÔNG P1 ICE.** `i64_to_usize` debug_assert
  compile-out release, NHƯNG **KHÔNG builtin nào nhận i64-user làm size** (`vector_new`/`hashmap_new` 0-arg,
  grow gấp đôi từ header) → âm bất-khả (floor), overflow cần exabyte không tồn tại (OOM-null bắt trước).
  Bất-khả-từ-source, defensive. 🚩 tripwire: re-verify nếu thêm builtin capacity-hint/resize/repeat.
- **Panic Nhóm B (host-ISA) = PARK chính đáng** (RATIONALE môi-trường-fatal, như rustc thiếu LLVM).
- **REAL đúng nhãn (campaign riêng):** ADR-0088 double-nullable · Deep-Clone · drain · §15.6 `Vector<Leaf?>` refuse.
- **🆕 phát hiện phụ:** (a) `for/loop/break/continue` parse+typecheck qua nhưng lowerer refuse E1100
  (`lib.rs:2144`) — bẫy câm-một-nửa, liên quan design drain; (b) `T??` khai-trần (ngoài `get`) lọt typecheck
  → MIR verifier báo "heap-nullable B8" SAI HƯỚNG (verifier over-match inner `Integer?`) — ADR-0088 chưa phủ.
- **🔵 NỢ VERIFY TREO — ADR-0084 field-auto-deref (Slice 1a/1b):** code land `d02c0c4`+`006b6c7` + corpus xanh
  (381→30/383→5/385→4/387→E2440) NHƯNG **ADR vẫn DRAFT chờ O ký** + **tooth-386 VACUOUS** (file ghi E2450
  nhưng binary CLI cho **E2400** typecheck-fatal; E2450 chỉ thấy qua test-harness gộp-đa-pha — răng không cắn
  user thật). Phiên sau: O mở mũi verify riêng + soi tooth-386 + G ký. Bài học luật O #15/#21.

## Bài học phiên
1. **Refuse-over-guess áp cả claim của O** — P1 false-alarm do flag "nghi bom" mà không probe. Đọc-code-nửa-vời
   → kết luận sai. Lần thứ 12 cùng gốc "hành động trước khi đo" (persona luật 18/20).
2. **Nhãn backlog KHÔNG đáng tin — triage bằng probe lộ 3 rác** (1 false-alarm O, 1 zombie, 1 mislabel) trong
   một lượt. Cùng quy luật recon-trước-WO cứu 4 lần các phiên trước.
3. **Tooth-harness-gộp-pha có thể VACUOUS** (386): test PASS nhưng răng không cắn user thật (CLI chặn pha sớm).

→ [[campaign_aggregate_nullable]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]]
