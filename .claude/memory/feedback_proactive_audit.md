---
name: feedback-proactive-audit
description: "User chủ động yêu cầu audit tech-debt + doc sync trước khi đóng phase. AI nên SUGGEST audit window khi gần freeze, không đợi user nhớ."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---

Trước khi đóng version/phase, user thường ask explicit cho audit:
- **Tech-debt audit** ("còn chỗ nào binary thinking không?" → dẫn tới ADR-0010 + phase v0.3.x.ternary)
- **Doc sync** ("trước khi sang triển khai 0.5, bạn có cần update bất kỳ tài liệu nào thì hãy thực hiện đi" → commit doc sync trước v0.5)
- **Gate-closing phase** (v0.3.x.cleanup per ADR-0009 — phase riêng để đóng debt thay vì kéo sang version sau)

User direct quote: "không muốn vội, những chỗ có thể cải tiến được thì hãy làm ngay, tránh tăng thêm nợ kỹ thuật, sau này sửa tốn nhiều thời gian và chi phí hơn"

Lặp lại sau v0.5.9 (đã đóng phase): user prompt "đây là cơ hội tốt để review trước khi sang 0.6. Hãy review lại một vòng nhé. testing, tư duy tam phân." → AI audit thấy 1 binary leak + 3 testing gap → user "hãy fix tất cả" → phase v0.5.x.review (4 commits: 20076d5, d7f1beb, b167717, b285a1f).

**v0.7.x.audit data point (2026-05-18):** Pattern repeats mid-phase, NOT just at freeze. User prompt sau 9-commit v0.7 series (v0.7.1 → v0.7.4.3-error docs): *"đây là một thời điểm tốt để chúng ta review lại tất cả các tài liệu 1 cách tổng thể"*. Audit surfaced 11 findings (3 CRITICAL + 5 MAJOR + 3 MINOR) — CLAUDE.md stuck v0.6 state (highest priority), missing ADR-0007/0008 cross-refs cho ADR-0020 work, stale numbers (test count, ADR count), broken markdown anchors, TODO.md không track v0.7. User approved 3-commit fix plan; SPEC header version stays v0.6 per Q2-C (bumps only at v0.7.13 verify gate). Commits: 46dd59a (audit.1 CRITICAL), 0b2d336 (audit.2 MAJOR), audit.3 MINOR pending. **Cadence insight:** audit pattern valuable not only at phase freeze, but also at **major sub-phase boundaries** (e.g. before large implementation work like v0.7.4.3-error) — author's intuition was right to trigger pre-implementation rather than wait for phase end.

**Why:** Bổ sung [[feedback-stability-over-speed]] — user chọn trả phí cleanup *trước* freeze hơn là tích nợ. Pattern này lặp đủ nhiều (v0.3.x.cleanup, v0.3.x.ternary, doc sync trước v0.5, audit memory này) để AI chủ động đề xuất thay vì đợi user nhớ.

**How to apply:** Two trigger contexts (refined post-v0.7.x.audit):

**Context A — Pre-freeze (phase end):** Khi sub-tasks của một phase đã `[x]` hết trong TODO.md, AI proactive suggest 1 lần. Use ADR-0009 4-gate matrix as checklist:

1. "Trước khi đóng phase X có muốn audit Y không?" — Y = tech-debt area liên quan:
   - Binary thinking leak (control flow, comparison ops, types)
   - Doc drift (SPEC ↔ implementation, ADR ↔ code, CLAUDE.md ↔ reality)
   - Naming convention drift (verbose keywords, dot paths, error codes)
   - ADR gap (decision đã ship mà chưa có ADR)
   - Memory file staleness

2. Đề xuất gate-closing phase nếu pattern muốn freeze cleanly (theo ADR-0009 4-gate matrix)

**Context B — Pre-implementation (sub-phase boundary):** Khi đã có ≥5 commits trong một sub-phase, hoặc sắp vào major implementation work (large LOC, multi-crate touch), AI proactive suggest cross-doc consistency check:

1. "Đã có N commits liên tiếp trong sub-phase. Trước khi vào [next-large-task] có muốn audit consistency không?" — categories:
   - State declarations stale (CLAUDE.md, README.md, SPEC header)
   - Cross-references rot (ADR ↔ ADR, ADR ↔ SPEC)
   - Numerical drift (test count, version, ADR count, opcode count)
   - Anchor links broken (GitHub markdown anchors)
   - TODO.md tracking sub-phase commits

2. Categorize findings by SEVERITY (CRITICAL/MAJOR/MINOR), propose per-category commits cho granular review

Không spam: 1 lần per trigger context. Pre-freeze + Pre-implementation are SEPARATE triggers (can both fire in same phase). User decline → accept silently, không nhắc lại trong cùng context.
