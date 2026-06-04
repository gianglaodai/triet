# Phase 5 — S6 Ownership Pipeline Integration

**Status:** In progress
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
