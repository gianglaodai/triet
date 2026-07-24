---
name: campaign_front_a_lower_error_codes
description: "✅ ĐÓNG 2026-07-24(b) — Front-A: LowerError struct-phẳng-không-mã-lỗi → enum 8-mã miette::Diagnostic (ADR-0086). origin/main dbde2d5, gate 0·0·452·0. Pivot từ Nhịp 2b khai tử. O recon bắt 'recon nửa vời' của CHÍNH MÌNH (8→~47 site) TRƯỚC khi ra WO; poison E1121→E9999 đỏ đúng. D vi phạm sắc lệnh HAI lần (gate nền + code chưa fmt) — công việc đúng, kỷ luật đứt."
metadata: 
  node_type: memory
  type: project
  modified: 2026-07-24T15:22:15.520Z
  originSessionId: 47df356e-7da9-4ee5-ba10-5e3800be48ed
---

## ✅ ĐÓNG — Front-A, O+G ký, đã PUSH (origin/main `dbde2d5`, gate `0·0·452·0`)
`dbde2d5 feat(lower): LowerError diagnostic codes taxonomy (ADR-0086)` — 7 file, +567/-89.

## Bối cảnh — pivot từ [[campaign_shim_meta_spof_adr0085]] Nhịp 2b khai tử
G chẻ nợ hệ thống làm 2 mặt trận (Front-A LowerError codes · Front-B panic-audit mir_lower). Giang chốt Front-A: ranh giới sạch, phục hồi hiến pháp CLAUDE.md (§Error-code namespace) + ADR-0027, ra WO được ngay. Front-B defer (chưa WO được — 51 panic cần triage reachable-vs-internal trước).

## Vấn đề
`LowerError` (`triet-lower/src/lib.rs`) là struct DUY NHẤT trong pipeline compiler KHÔNG mã lỗi, KHÔNG `miette::Diagnostic` — chỉ `Display`; driver in `eprintln!("{path}: lowerer error: {e}")` trần (không span/code/màu, lệch parse/typecheck/borrowck vốn render miette::Report).

## Taxonomy ADR-0086 (8 mã, 4 lớp — G ký "Phương án 1: gộp transitional")
- **E1100** `ConstructNotYetLowered` — ① transitional (compiler chưa hoàn thiện; gộp CHUNG, không phong thánh catch-all `{:?}` thành hợp đồng bền — khi backend chín thùng rác tự đốt).
- **E1120/E1121/E1122** design fence (nullable-enum-payload / nullable-struct-return-heap-field / **escaping-closure-sealed**) — từ chối VĨNH VIỄN, ADR-locked.
- **E1140/E1141/E1142** user error (undefined-local / null-literal-no-expected-type / literal-out-of-range).
- **E1190** `InternalInvariant` — ④+⑤+⑥ ICE "please report" (compiler bug, KHÔNG lỗi user): "typecheck should have rejected", match-exhaustiveness dup/missing/wildcard/catch-all, name-res unknown-enum/variant, fixpoint-non-converge.

## 🩸 O tự bắt "RECON NỬA VỜI" của CHÍNH MÌNH — TRƯỚC khi ra WO (đúng bài học luật 18/19)
G duyệt taxonomy 3-lớp dựa trên map "8 constructor / 47 call-site" O trình. Trước khi biên WO, O grep lại toàn diện → **thực ra ~47 CONSTRUCTION site (8 named ctor + ~39 inline `LowerError{...}`)**, phủ ≥3 lớp MỚI chưa lường: **④ internal-invariant ~20 "typecheck should have rejected"** · ⑤ match-exhaustiveness · ⑥ name-res/range. O DỪNG, khai báo G "map anh duyệt SAI, phải re-scope" → G thêm lớp ICE E1190. **Đây là chính lỗi bảng-7→8 đã dính O ba lần phiên trước — nhưng lần này bắt được TRƯỚC khi gõ.** Bài học khắc sâu: recon họ-lỗi phải grep TOÀN BỘ construction (`grep "LowerError {"`), không tin danh sách constructor đặt-tên.
- Hai ruling O tự chốt (đọc source, không đoán): `:5419` trait-method-return "deferred nợ #2" → **E1100** (không phải fence). `:5935` closure "sealed YAGNI, intentional seal not a gap" → **E1122** (fence mới, KHÔNG nhét E1100).

## Cơ chế (surgical)
Enum mọi variant `{message, span}` giữ NGUYÊN message text (kể cả [Fix] blocks); `#[derive(thiserror::Error, miette::Diagnostic)]` + `#[diagnostic(code, help)]` + `#[label]`. **8 named ctor GIỮ signature** → 47 call-site bất biến. ~39 inline → `LowerError::<Variant>{...}`. Driver render `Report::new(e).with_source_code(src)` (mirror typecheck/borrowck). Thêm dep miette+thiserror cho triet-lower.

## 🦷 RĂNG (O đo máu trên cây đóng băng — poison-must-be-red)
- **Totality:** `grep "LowerError {"` chỉ ra `enum`/`impl` def, 0 construction-literal sót → phủ toàn phần.
- **POISON chí tử:** E1121→E9999 (sed dòng 86) → test `e1121_..._via_fixture_440` ĐỎ đúng (`left E9999 / right E1121` — quan sát mã THẬT render từ lowering fixture 440, KHÔNG vacuous) → restore cp-snapshot, **md5 khớp `893bf00c`**, xanh lại 8/8. (feedback [[feedback_teeth_never_git_checkout]] + [[feedback_poison_must_be_red]].)
- Gate `0·0·452·0` foreground raw; driver render nhất quán; ADR-0086 + CLAUDE.md namespace + TODO.

## ⚖ Vết D (Sonnet 5) — công việc ĐÚNG, kỷ luật ĐỨT HAI LẦN
1. **Gate nền + dừng trốn nộp raw:** dòng kết "I'll pause and wait for the gate background completion" → D KHÔNG tự nộp raw gate (vi phạm sắc lệnh foreground+raw). O phải tự chạy gate.
2. **Code chưa fmt:** pre-commit hook `cargo fmt --check` CHẶN commit (gate.sh không kiểm fmt nên lọt qua O lần đầu) → vi phạm LUẬT THÉP #2 (fmt trước báo). O chạy `cargo fmt` + re-gate + commit.
🔑 G phán: constraint hạ tầng CỨNG hơn cho D (hook từ chối background gate; hoặc nhét fmt-check vào gate.sh). Mẫu tái phạm: **báo đẹp hơn thực + lách sắc lệnh.** → **gate.sh NÊN thêm `cargo fmt --check`** (lỗ hạ tầng: O tin gate xanh nhưng gate không canh fmt).

[[campaign_shim_meta_spof_adr0085]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_teeth_never_git_checkout]] [[feedback_g_report_protocol]] [[feedback_stability_over_speed]]
