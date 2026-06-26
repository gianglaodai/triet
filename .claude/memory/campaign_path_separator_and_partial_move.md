---
name: campaign_path_separator_and_partial_move
description: "✅ ĐÓNG — ADR-0070 Partial-move (ZST/Capability field) + ADR-0071 Path `::`/`use`/enum-variant. Hai sổ đỏ dọn liên tiếp 2026-06-26. Infra Kỷ-Luật-Gate D nộp PENDING O-verify."
metadata: 
  node_type: memory
  type: project
  originSessionId: b937933d-79d7-4b00-9795-d55815fb3258
---

# Chiến dịch: Partial-move (ADR-0070) + Path-separator/use (ADR-0071)

Phiên 2026-06-26 (vai Mentor O). Hai sổ đỏ G giám sát, dọn liên tiếp. origin/main = `c831274` synced.

## ✅ ADR-0070 Partial-move & Struct-ZST — SEALED, commit `d3aa4ce`
`let v = hw.vga` field-level move-state = lõi Borrow-Checker. **Quyết định:** nâng move-state per-Local→**per-Place** (`partial_moves: BTreeMap<Local, BTreeSet<String>>` trong BlockState, union-merge monotone → fixpoint hội tụ). Scope răng cưa: CHỈ ZST/Capability field (heap-field-move defer No-Box — đòi JIT dynamic drop-flag). E2420 reuse (không đẻ mã mới). **0B true-ZST sizing** (O phán checkpoint: 8B phản bội ADR-0069 niêm phong "0 byte"). 3 file: checker.rs (Δ3 capability-allow + heap-refuse E2423 giữ + 6 use-site invalidate) · lib.rs (gate C allow-list + sizing 0,1) · mir_lower.rs (leaf-less non-copy struct→Drop no-op). 6 fixtures 279-284. **O verify 5 teeth đỏ độc lập** (P-field-key·P-merge union→intersection·P-Δ3-heap no-panic·P-reread·Step3-JIT) + byte-identical. D probe Step-0 bắt 8B-vs-0B + mixed-struct offset (run→105). Schema §10 HardwareToken destructure-move CHẠY THẬT. G ký.

## ✅ ADR-0071 Path `::` + `use` + enum-variant `::` — SEALED, commit `4a7da96` (Lát1) + `c831274` (Lát2)
**Supersede ADR-0005** dot-path+Python-import. **AST pha lê: `::` TĨNH (path/type/enum-variant) · `.` ĐỘNG (field/method).** Giang chốt PA-B Rust-model + brace-group `use a::{x,y}` + bắt-buộc-qualify. G phán Reading A "giết không tha".
- **Lát 1** (`4a7da96`): lexer `::`(ColonColon)+`use`, giết `import`/`from`. `Item::Use{path,group}` schema-first (codegen, KHÔNG hand-edit generated). Resolver route 2-đường-cũ (group rỗng→Whole bind-leaf, non-empty→From) — E2100/E2101/visibility/as bảo toàn. 4 teeth (P-colon-token·P-longest-match·P-old-keyword·P-resolver). Driver KHÔNG chạy resolver → resolve-correctness test ở triet-modules (LUẬT 5 split, O ver-đúng-kiến-trúc).
- **Lát 2** (`c831274`): `Color::Red`(+payload)→`Expr::EnumLiteral` (node sẵn) + `Pattern::EnumVariant{name:Some}`. **Giết 3 cơ chế ngầm:** ① pattern guess-hack (check.rs:892 scrutinee-scan) ② expr in-scope-enum-scan (resolve_enum_variant + 2 call-site) ③ 3 dot-hack (MethodCall/FieldAccess/Call-FieldAccess→variant). **E1018 AmbiguousEnumVariant KHAI TỬ** (emitter duy nhất = scan). Bare un-qualified→E1002 mọi nơi; import-bound `use X::{V}` chừa (env.lookup TRƯỚC scan). **§2.A:** enum-match Variable-arm = catch-all binding (đối xứng scalar has_scalar_catch_all ADR-0064 §8) — hệ quả "bare=binding 100%", refuse-narrow heap-payload catch-all. Dọn dead `expr_resolutions` (rule #4: field+4 consumer+threading+type+21 caller; check() 4-tuple→3). Sweep ~30 fixtures+examples+docs.

### ⚔ Bài học O verify-don't-trust (Lát 2 — đáng ghi)
- **Tooth-label láo:** D nhãn tooth "P-pattern-guess-resurrect" (re-add typecheck guess). O đào: guess-hack nay **INERT** — lower định tuyến Variable→catch-all theo AST, KHÔNG consult resolution → re-add guess = no-op, tooth **VACUOUS**. Guard load-bearing THẬT = **§2.A catch-all** (poison nó→293 E1026). → relabel P-catch-all + **sharpen 293**: scrutinee `Color::Red`=tên-arm → binding/variant cùng ra 99 (không phân biệt); sửa scrutinee→`Color::Red` arm bare `Green` (KHÁC tên) → binding bắt Red=99, match-on-name-lén→E1026. Đừng tin nhãn, tin vết răng.
- **Grep thô suýt nhầm:** poison P-scan/P-dot → lỗi DỜI typecheck→lower (E1002→"undefined local variable" / E1015 biến mất); grep "undefined" khớp cả 2 → tưởng không flip. Harness `e.contains("undefined name")` mới đúng (294+29 fail). Verify ở mức HARNESS không grep thô.

## ✅ SEALED — Infra Kỷ-Luật-Gate `9263501` (O verify máu 2026-06-26 máy mới + G ký)
O verify HEAD=`4004df8` (infra `--stat` rỗng vs `9263501` → identical). **4 teeth máu, restore cp byte-identical (md5 khớp):** A-clean-exit0 (gate exit 0, `0·0·289·0`) · **A-real-red** (inject `let unused=1`→build 2 warning, gate exit 1 — KHÔNG con dấu cao su) · **B-no-flake** (`cargo test --workspace` ×10 = 10/10 clean) · **B-teeth-alive** (neuter `FREE_COUNT.fetch_add`→2 FAILED, assert chain sống dưới Mutex). Fix A gốc đúng (set-e+pipefail dập grep-rỗng→exit1 giả; capture `$?` + verdict tường minh). Fix B `unwrap_or_else(into_inner)` chống poison-cascade, reset under-lock. **G KÝ. Kế: BƯỚC 2 read-side heap.**

## 🔧 (history) PENDING — Infra Kỷ-Luật-Gate (committed `9263501` [PENDING O-VERIFY], chuyển máy)
G phán dọn hạ tầng TRƯỚC khi mở read-side heap. D nộp + **committed `9263501` để chuyển máy** (message ghi rõ PENDING O-VERIFY, KHÔNG phải đã-ký): **Fix A gate.sh** (bỏ `set -e`, verdict tường minh: clippy-sạch-grep-no-match KHÔNG còn exit-1 giả; tôn trọng cargo rc; exit0⟺sạch — sanity gate ĐÃ exit=0) · **Fix B** 6 counting file thêm `TEST_LOCK: Mutex<()>` + reset-under-lock (vector/hashmap/nullable_map/string_drop/string_match_move/block_tail). D claim 4 teeth (A-clean-exit0·A-real-red·B-no-flake 10×·B-teeth-alive). **⚠️ CHƯA O verify máu — phiên sau VIỆC ĐẦU: verify 4 teeth (đặc biệt A-real-red = gate KHÔNG thành con dấu cao su + B-no-flake cargo test --workspace 10× liên tiếp) trên commit `9263501`, restore byte-identical; đạt → G ký; KHÔNG đạt → D sửa.** Nếu cần revert: `git revert 9263501`.

## ✅ BƯỚC 2 SEALED — Read-side heap-SCALAR field move-out, commit `2323e0d` (2026-06-26, O verify + G ký)
`let s = p.name` chạy cho heap-scalar (String/Vector/HashMap). **3 Δ:** Δ1 borrowck `checker.rs` guard `Capability(_) || is_any_heap()` ghi `partial_moves` (reuse/whole-base→E2420 qua `partial_move_invalidates` sẵn) · Δ2 JIT `mir_lower.rs` Assign heap-field-move: sync String fat-ptr len@8/cap@16 (free nhận cap → rác=UB) + tombstone leaf ở base-slot (Drop base→ptr=0 no-op) · **Δ4 lower `lib.rs`** (NGOÀI WO ban đầu, O ADMIT sót tầng lower trong recon): field-read temp mang type thật → cấp heap slot + Drop; Unknown temp = LEAK. **NARROW (G phán):** heap-STRUCT field-move trảm khỏi guard (`Struct(_)` bỏ) + JIT Struct-arm xóa — vì construction-into-field DOUBLE-FREE pre-existing (ADR-0067, verified exit 134) → bom câm không test được. **O verify máu 6 teeth poison→đỏ→restore:** count→2 (Δ2), count→0 leak (Δ4), 296/297→green (Δ1 rec), 300 coffin-lid re-add Struct→mất E2423, 295/299 sibling-alive, 298 multi-level refused. Gate `0·0·295·0`. **Recon-lesson: O bác "type-prop String→Unknown" là đúng-typecheck-sai-lower; probe MIR phải soi type temp ở local_decls, không chỉ thấy `_3 = move _0.name`.**

## Nợ defer (sổ đỏ — G+Giang chốt mở)
- **heap-STRUCT field move-out** (NEW, BƯỚC 2 narrow): `let m = o.inner` (Inner chứa heap) — REFUSED E2423. Chặn bởi ADR-0067 construction-into-field double-free. Re-mở CHỈ khi construction sound + có fixture run. Fixture 300 ghim nắp quan tài.
- **match-arm bind heap payload move-out** (BƯỚC 2 gạt riêng): `match get(){~+ s => s}` move RA — vướng blocker Lower KHÁC: call hàm trả nullable-aggregate → `lowerer does not support Identifier`. Recon riêng.
- **ADR-0068 Box/recursive** (G HOÃN tiếp): allocator + iterative-drop + typecheck self-ref. Con quái vật, chưa allocator = tự sát.
- **No-Box completions:** `enum{Rec(Wrapper)}` payload-struct-chứa-heap · heap-field partial-move (ADR-0070 defer) · multi-level `hw.a.b`.
- ADR-0070 cosmetic: tên `ImportPath/ImportName` legacy sau khi `import` chết (rename UsePath/UseItem follow-up tùy chọn).

[[mentor_o_persona]] [[colleague_d_persona]] [[campaign_truc_b_heap_in_aggregate]] [[feedback_teeth_never_git_checkout]] [[feedback_poison_must_be_red]]
