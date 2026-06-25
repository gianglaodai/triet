---
name: feedback-verify-producer-before-consumer
description: "Nguyên tắc review O (G chuẩn thuận 2026-06-09) — flip field type mà producer còn round-trip qua parse = chưa migrate, fake producer."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 7f9fbd79-3ba3-4ebd-b376-fd8db532831b
---

**Nguyên tắc review (O đề xuất, G chuẩn thuận 2026-06-09 sau B1a S2 Vòng 3).** Khi duyệt một stage "migrate type/representation": **verify PRODUCER trước CONSUMER.** Flip field type trên bề mặt (vd `ty: String → ty: MirType`) mà producer còn **đẻ ra biểu diễn cũ rồi parse ngược** vào type mới = **CHƯA migrate, chỉ sơn vỏ.**

**Why (bom thật B1a S2):** D nộp S2 với field flip sang `MirType` ✓ + unit test xanh ✓ — nhưng `type_name() -> String` vẫn đẻ string-grammar (`"&0 "`, `"Vector<Integer>?"`) rồi `MirType::parse()` nuốt ngược về enum tại 3 production site. → `parse` (shim "MUST KILL at S4") thành **xương sống producer**; xóa ở S4 thì producer gãy từ gốc → **bất biến G ③ vỡ**, kéo sập compiler. String-grammar KHÔNG bị diệt — chỉ giấu sau round-trip String→parse→enum. Unit test trên bề mặt không bắt được; chỉ lộ khi O **đào chỗ để teeth** (tìm producer để poison).

**How to apply:**
1. Duyệt migrate: grep `fn <producer>() -> <OldType>` + mọi call `NewType::parse(<producer>(...))`. Nếu producer còn trả OldType → REJECT, đòi map trực tiếp `Source → NewType`.
2. `parse`/bridge shim chỉ được sống trong `#[cfg(test)]` hoặc biên string→enum một-lần; CẤM làm đường sinh type chính.
3. Teeth-driven: poison producer (vd map `"String"→Unknown`) → fixture production phải ĐỎ. Đào chỗ poison thường lộ producer ngụy trang.

Liên quan [[feedback-poison-must-be-red]] (cùng tinh thần verify), [[feedback-collaboration-loop]] (O = chốt chặn), [[mentor-o-persona]].
