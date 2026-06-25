---
name: project-vision-os-capable
description: 5 trụ cột kiến trúc Triết + bản sắc tam phân + cadence policy. Version-agnostic — phase shipped/next check ROADMAP.
metadata: 
  node_type: memory
  type: project
  originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---

**Cam kết cứng:** Triết không phải interpreter thuần. Mục tiêu dài hạn là ngôn ngữ OS-capable, đủ năng lực viết microkernel khi phần cứng tam phân xuất hiện. Pace 5–10 năm horizon. Stability > speed.

**5 trụ cột kiến trúc (locked trong `VISION.md`):**

1. **Module System** — Java JPMS aesthetic + Python imports + dot paths. ADR-0005.
2. **Stable IR + Bytecode VM** — register-based SSA, `.triv` wire format, differential testing vs interpreter. ADR-0007/0008/0010.
3. **Stable ABI + Crate-Pack** — witness table dispatch (Swift-style) cho cross-package generics; monomorphization intra-package; `.tripack` packaging; semver linking với `iface_hash` final arbiter. ADR-0011/0012/0013.
4. **CAS Packaging** (Unison-inspired) — hash-based module identity, hai cấp `iface_hash`/`impl_hash`. Tới phase tương ứng đọc ROADMAP.
5. **Capability Namespaces** — `sys.`/`dev.`/`usr.` enforce ở compiler. Trit-level capability (-1/0/+1) + Łukasiewicz `Unknown` = runtime policy.

**Bản sắc tam phân (3 điều không thay thế):**
- Trit-level capability (3-state native, không emulate)
- Trilean Ł3 default (không Boolean reasoning trong logic ops)
- Tam phân ABI ổn định bẩm sinh (không struct padding, không endianness)

**Logic ops:** symbolic preferred (`!`, `&&`, `||`, `^`, `=>`, `~>`, `~^`, `<=>`, `<~>`). Prefix trit `0t+`/`0t-`/`0t0`. `unknown` không phải `null`.

**Cadence:** Mỗi quyết định kiến trúc viết ADR vào `docs/decisions/` trước khi code. Version gate matrix (ADR-0009) áp cho mọi version bump: spec ✓, tests ✓, bench gate ✓, snapshot ✓.

**Why:** User đặt mục tiêu chứng minh tam phân không phải freak. Nếu không viết được OS, ngôn ngữ sẽ luôn bị giới hạn là "kẻ lập dị trong thế giới nhị phân". Stability commitment là cách trả phí trước hơn là tích nợ.

**How to apply:** Mọi đề xuất kỹ thuật phải đối chiếu 5 trụ cột — cite trụ cột nào đang serve. Refuse features mâu thuẫn capability/ABI/CAS roadmap. Khi user hỏi "phase next là gì", đọc `ROADMAP.md` + `TODO.md` thay vì recall memory. Liên quan: [[feedback-stability-over-speed]], [[project-triet-overview]].
