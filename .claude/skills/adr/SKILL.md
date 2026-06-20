---
name: adr
description: Scaffold a new Architecture Decision Record in docs/decisions/ following the project template.
trigger: /adr
argument-hint: "<number> <title> — e.g., /adr 0008 bytecode-binary-format"
---

# /adr — Create Architecture Decision Record

Creates a new ADR document at `docs/decisions/<number>-<slug>.md` following the established format from ADR-0005 and ADR-0007.

## Template

```markdown
# ADR <number> — <title>

**Trạng thái:** Đề xuất | Quyết định | Thay thế. Áp dụng cho v<version>+. <one-line scope>.

**Issue:** <why this decision matters now, what breaks without it>

## Quyết định

<the decision itself, clearly stated>

### Hình thức cụ thể

<concrete examples, diagrams, or code snippets showing the decision in practice>

## Các phương án đã cân nhắc

| # | Phương án | Ưu | Nhược | Kết luận |
|---|-----------|---|-------|----------|
| 1 | <option> | | | |
| 2 | <option> | | | |

## Hậu quả

### Tích cực
- 

### Tiêu cực
- 

### Rủi ro cần mitigate
- 

## Ngày hiệu lực

- v<version>+ — <what kicks in at each version>
- Không áp dụng hồi tố cho <prior versions>.
```

## Instructions

1. Determine the next ADR number by counting files in `docs/decisions/`.
2. Create `docs/decisions/<NNNN>-<slug>.md` using the template above.
3. Fill in all sections. Status defaults to "Đề xuất" unless the user confirms otherwise.
4. Cross-reference any prior ADRs that relate to this one.
5. Update `ROADMAP.md` and `TODO.md` if this ADR introduces a new sub-task.

## Rules

- ADR numbers are 4-digit zero-padded (e.g. `0008`, `0014`).
- Slug uses dashes, not underscores. Keep it under 60 chars.
- Every ADR MUST document alternatives and their rejection rationale.
- Use Vietnamese (not English) for body text — match existing ADRs.
