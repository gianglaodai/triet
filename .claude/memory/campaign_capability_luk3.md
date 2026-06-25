---
name: campaign_capability_luk3
description: ✅ CAPABILITY Ł3 (ADR-0069) NIÊM PHONG — coherence VISION §8 hoàn tất (null/logic/capability một đại số Ł3). ZST-token ngậm Ł3-Trit, borrowck-enforce, Defer runtime trap. origin b081184, gate 0·0·273·0. Lát 0-4 + §amend-A + §5 LOCK. Sổ đỏ kế: Partial-move & Struct-ZST + Import `.`→`::`. ĐỌC nếu đụng capability/mint/ZST-token/__triet_cap_check/E2211/E2212.
metadata:
  node_type: memory
  type: project
  originSessionId: capability-luk3-campaign
---

**Capability Ł3 (ADR-0069)** = chân thứ ba của COHERENCE VISION §8 — một đại số Ł3 duy nhất xuyên
**null (PA-3c) / logic (Trilean) / capability**. Mandate ternary-first (G+Giang 2026-06-22) thành
hiến pháp. ADR `docs/decisions/0069-zst-capability-token-luk3.md` 🔒 NIÊM PHONG (O+G ký từng lát).

## Fork chiến lược (O recon → G chốt C)
HAI thế giới capability: (1) **package-manifest** ADR-0016/0017/0018 — đại số Ł3 4-state ĐÃ code
(`triet-pack/types.rs:297` CapabilityLevel) nhưng **orphan** khỏi driver (`check_capabilities`
typecheck KHÔNG ai gọi); (2) **Hardware-Token ZST** (schema §10) — capability=ownership+move,
coherent No-Box nhưng "design only". **G chốt HƯỚNG C synthesis:** chôn 0016/0017/0018, cứu đại số
Ł3, xây trên cỗ máy ownership/move (No-Box) → ZST-token **ngậm** Ł3-Trit.

## Ánh xạ Ł3 ↔ vòng đời capability (tim coherence)
| Ł3 | Level | Ngữ nghĩa | Enforce | Cost |
|---|---|---|---|---|
| `Trit::Positive` | **Grant** | mint tự do, possession=quyền | typecheck + borrowck move/E2420 | 0 byte |
| `Trit::Zero` | **Ambient** | **receive-only** (M1): mint=E2211, nhận-qua-param OK | typecheck | 0 byte |
| `Trit::Negative` | **Deny** | cấm mint + cấm TIỆT sở hữu (param/binding type) | E2211 mint · **E2212** possess | — |
| `Trilean::Unknown` | **Defer** | mint→runtime hook, ≤0→trap | JIT `__triet_cap_check`+`trapnz user(2)` | 1 check |

## Lát (mỗi lát O verify máu, răng đỏ độc lập, restore byte-identical KHÔNG git checkout)
- **Lát 0 `8b06a28` — ZST token & cấm copy.** `capability X grant` decl (`Item::Capability` schema-gen)
  + `mint X` → ZST local 0-byte. **Chốt soundness: `is_copy` struct-rỗng→`all()`∅→Copy = bypass câm**
  (`triet-mir/lib.rs:666`). Ép non-copy 2 tầng: `MirType::Capability=>false` (mir) + `ctx_is_copy`
  (lower) — defense-in-depth, poison ĐƠN LẺ vẫn đỏ (che chéo), poison CẢ HAI → E2420 mất = bypass.
  Struct-rỗng-DỮ-LIỆU GIỮ Copy (short-circuit riêng). + `public capability`→refuse (N2, mirror import).
- **§amend-A `47eb283` — M1 Receive-only** (Giang cú pháp `capability`/`mint` contextual-kw; G chôn
  M2 possession-gated=nhân-bản-token-non-copy + M3 call-graph=action-at-a-distance). Ambient = O-Cap
  thuần: token đi xuống từ biên ngoài qua parameter, "không khí không tự sinh capability".
- **Lát 2 `ca8272e` — possession-check.** `resolve_type` (chokepoint mọi annotation param/let/field/
  return): deny-as-type → **E2212**; ambient/grant possessable. mint ambient→E2211 "receive-only".
- **§5 `d84cd24` — G LOCK check tại MINT-SITE** (KHÔNG guarded-op: ZST bốc hơi runtime; check ở
  guarded-op = nhét runtime-check khắp use-site = giết bản chất ZST).
- **Lát 3 `2dd4d5f` — Defer runtime hook (trùm cuối).** `Expr::Mint` defer → `Statement::CapabilityCheck`
  (MIR variant mới, populate-lower+consume-JIT cùng commit rule#4) → JIT `__triet_cap_check(cap_id)`
  → `icmp SignedLessThanOrEqual 0` → `trapnz unwrap_user(2)` (SIGILL, RIÊNG khỏi arithmetic user(1)).
  `CAP_POLICY: AtomicI64` default **0=Unknown=fail-closed**. Test subprocess (`capability_defer_trap.rs`,
  N7 + fork-bomb guard `_TRIET_CAP`): allow(+1)→exit0 · deny(−1)→SIGILL · unknown(0)→SIGILL. ⚔ **răng
  R-fail-closed = đổi `icmp sle`→`slt` ở Cranelift IR → Unknown(0) lọt → unknown_traps đỏ** (boundary
  `≤` load-bearing — G tuyên dương "không tin ý định thằng code, chỉ tin nhát chém CPU").
- **Lát 4 `278`→30 — demo A2.** G chốt A2 (capabilities qua param riêng) thay full struct-aggregate:
  `struct Hardware{vga}` destructure-move đòi **partial-move** = lõi Borrow-Checker, KHÔNG nhồi vào
  ADR capability (scope creep "mổ tim xong đừng mổ nốt dây chằng"). Demo phô diễn 4 level, run→30.

## Mã lỗi mới
- **E2211** CapabilityLevelUnsupported — mint non-grant (deny/ambient/defer).
- **E2212** CapabilityNotPossessable — deny-capability làm kiểu (param/binding/field).

## 🔴 Sổ đỏ — 2 campaign độc lập KẾ (G đích thân giám sát khi mở)
1. **Partial-move & Struct-ZST:** `let v = hw.vga` field-level move-state = con quái vật lõi
   Borrow-Checker/Memory-Management (ADR riêng + poison rã-struct/move-nửa/xài-nửa-kia) + dọn **B8 gate
   `triet-lower/src/lib.rs:72`** (lầm ZST-capability-field với heap → reject `struct Hardware{vga}`).
   Mở khóa schema §10 destructure-move canonical proof + Lát-4-full đã defer.
2. **Import `.` → `::`:** Giang nhận chọn `.` theo quán tính Python/Java; G đòi `::` cho trong sáng AST.
   **ĐẢO ADR-0005** (dot-paths LOCKED) → cần ADR MỚI supersede (KHÔNG revisionism câm). Sweep rộng:
   lexer/parser/mọi examples+fixtures/docs (SPEC + CLAUDE.md bảng §Language convention).

## Bài học O tự ăn (phiên này)
- **close-session suýt push mù:** auto-memory máy-local sparse (3-dòng MEMORY.md, phiên KHÔNG pull lúc
  mở) vs repo rich (66 dòng). `sync-memory.sh push` = `rm repo/*.md` + cp auto → **clobber 44 file**.
  DỪNG khi đo (wc -l + diff), sửa repo trực tiếp + pull đồng bộ, KHÔNG push. (Look-at-target-before-
  overwrite.)

🔒🏁 **CAPABILITY Ł3 NIÊM PHONG — coherence ba chân kiềng vững. Trục đóng sập.**

[[mentor_o_persona]] [[colleague_d_persona]] [[project_vision_os_capable]] [[campaign_truc_b_heap_in_aggregate]]
