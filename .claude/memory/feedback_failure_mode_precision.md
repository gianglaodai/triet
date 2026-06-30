---
name: feedback_failure_mode_precision
description: "D bốc phét failure-mode (claim SIGSEGV khi thực ra LEAK) — kỹ thuật phải chính xác tuyệt đối, không Hollywood"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: cfce150f-cc26-451d-b933-ca98ee4f57ce
---

**G ĐẠI KỴ "dramatic hóa" failure-mode. Compiler engineer phải nắm CHÍNH XÁC cái gì gây crash memory vs cái gì gây leak — claim sai = vứt.**

**Ca cụ thể (WO-NullableFieldMoveOut, 2026-06-30):** D báo cáo *"gỡ Site-3 dest-type propagation → SIGSEGV"* (nghĩ giống Phase 2 Struct/Enum field). O verify máu độc lập: gỡ Site-3 cho heap-`T?` (`String?`/`Vector?`/`HashMap?`) → **LEAK câm lặng (FREE==0), KHÔNG SIGSEGV**. Lý do KT: dest Unknown → JIT drop-glue mù (không nhận heap type) → `Drop(Unknown)` no-op → rò rỉ; KHÔNG có memcpy-qua-địa-chỉ-rác. SIGSEGV chỉ xảy ra cho **Struct/Enum field** (aggregate memcpy qua slot-rác), KHÔNG cho heap-scalar/`T?` (scalar-ptr copy 8B vào var).

**Why:** Site vẫn load-bearing (leak = unsound → tooth vẫn đỏ đúng), code KHÔNG sai — nhưng claim failure-mode sai chứng tỏ chưa-verify-máu, chỉ suy diễn từ ca khác. G: *"Làm compiler mà không nắm rõ cái gì gây crash, cái gì gây leak thì vứt. Không có chỗ cho Hollywood."*

**How to apply:** (1) O — KHÔNG tin failure-mode trong report của D; tự cắm poison đo đúng signal (SIGABRT 134 double-free vs SIGSEGV 139 bad-deref vs FREE==0 leak vs FREE==2 double-free-count — bốn signal KHÁC NHAU, đừng lẫn). (2) Phân biệt: aggregate (Struct/Enum) field no-slot → SIGSEGV (memcpy qua rác); heap-scalar/`T?` field no-slot → LEAK (drop-glue mù). (3) Khi viết WO/teeth, ghi đúng failure-mode kỳ vọng — nếu đoán SIGSEGV mà thực ra leak, counting-harness mới bắt được, fixture-crash KHÔNG. [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[mentor_o_persona]]
