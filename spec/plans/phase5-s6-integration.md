# Phase 5 — S6 Ownership Pipeline Integration

**Status:** Partial — borrowck done (2/5 codes), full test suite not yet (2026-06-04)
**See also:** `TODO.md` (live backlog + debt registry). REPORT-2026-06-04.md đã xóa — git history giữ.

**Dependency note:** Phase numbering ≠ build order. S6 integration depends on
Phase 4 (lowerer emits borrow ops) and Phase 2 (borrowck validates them).

**Done (2026-06-04):** Borrow checker (E2420, E2440), AST→MIR lowering (5 reference forms),
ParameterPassing in MIR signatures, pipeline end-to-end (`.tri → borrowck → JIT`).
**Not done:** E2450 (dead — lowerer never emits Drop), E2400/E2403 (regression vs v0.10
borrowck), comprehensive S6 ownership test corpus, loop-carried borrows,
closures, weak observer upgrade, full ADR-0026 concurrency model.
**Phụ thuộc:** Phase 4 (AST→MIR lowering), Phase 2 (borrow checker)

## Lộ trình

| Sub-task | Nội dung | Verify |
|---|---|---|
| 5.1 | Lower `Expr::Borrow` → `Statement::Borrow` | MIR chứa đúng ReferenceForm |
| 5.2 | Lower `ParameterPassing` → MIR signature | owned/mutable mapped correctly |
| 5.3 | Test E2440: double `&0 mutable` borrow bị từ chối | borrowck báo NllExclusivityViolation |
| 5.4 | Test E2420: use-after-move bị từ chối | borrowck báo UseAfterMove |
| 5.5 | Test NLL: sequential borrow hợp lệ được chấp nhận | borrowck pass |

## Nguyên tắc

- S6 ownership = compile-time concept. Runtime = raw pointers.
- Lowerer chỉ chuyển AST→MIR. Borrow checker kiểm tra lỗi.
- Không thay đổi borrow checker — nó đã hoạt động đúng từ Phase 2.
