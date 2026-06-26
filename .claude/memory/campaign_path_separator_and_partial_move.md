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

## 🔧 PENDING — Infra Kỷ-Luật-Gate (D nộp, **O CHƯA verify, G CHƯA ký**, DIRTY 7 file)
G phán dọn hạ tầng TRƯỚC khi mở read-side heap. D nộp: **Fix A gate.sh** (bỏ `set -e`, verdict tường minh: clippy-sạch-grep-no-match KHÔNG còn exit-1 giả; tôn trọng cargo rc; exit0⟺sạch) · **Fix B** 6 counting file thêm `TEST_LOCK: Mutex<()>` + reset-under-lock (vector/hashmap/nullable_map/string_drop/string_match_move/block_tail). D claim 4 teeth (A-clean-exit0·A-real-red·B-no-flake 10×·B-teeth-alive). **CHƯA O verify máu.** WO teeth: A-real-red (gate KHÔNG thành con dấu cao su) + B-no-flake 10× là then chốt.

## Nợ defer (sổ đỏ — G+Giang chốt mở)
- **Read-side heap gap** (G chốt BƯỚC 2 sau infra): `let s = p.name` move heap field RA + match-arm bind heap payload — chặn bởi read-side type-prop String→Unknown. Cao giá trị (heap đang write-only).
- **ADR-0068 Box/recursive** (G HOÃN tiếp): allocator + iterative-drop + typecheck self-ref. Con quái vật, chưa allocator = tự sát.
- **No-Box completions:** `enum{Rec(Wrapper)}` payload-struct-chứa-heap · heap-field partial-move (ADR-0070 defer) · multi-level `hw.a.b`.
- ADR-0070 cosmetic: tên `ImportPath/ImportName` legacy sau khi `import` chết (rename UsePath/UseItem follow-up tùy chọn).

[[mentor_o_persona]] [[colleague_d_persona]] [[campaign_truc_b_heap_in_aggregate]] [[feedback_teeth_never_git_checkout]] [[feedback_poison_must_be_red]]
