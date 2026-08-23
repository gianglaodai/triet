---
name: feedback-verify-producer-before-consumer
description: "An O review principle (approved by G 2026-06-09) — flipping a field's type while the producer still round-trips through a parse means the migration never happened; the producer is fake."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 7f9fbd79-3ba3-4ebd-b376-fd8db532831b
---

**Review principle (proposed by O, approved by G on 2026-06-09 after B1a S2 round 3).** When reviewing a "migrate a type/representation" stage: **verify the PRODUCER before the CONSUMER.** Flipping a field type on the surface (e.g. `ty: String → ty: MirType`) while the producer still **emits the old representation and parses it back** into the new type = **NOT migrated, just repainted.**

**Why (the real bomb, B1a S2):** D submitted S2 with the field flipped to `MirType` ✓ and green unit tests ✓ — but `type_name() -> String` still emitted the string grammar (`"&0 "`, `"Vector<Integer>?"`) and `MirType::parse()` swallowed it back into the enum at 3 production sites. → `parse` (a shim marked "MUST KILL at S4") had become the **backbone of the producer**; deleting it at S4 would break the producer at the root → **G's invariant ③ shatters**, taking the compiler down. The string grammar was NOT killed — merely hidden behind a String→parse→enum round trip. Surface unit tests cannot catch that; it only surfaced when O **went looking for a place to plant teeth** (hunting for a producer to poison).

**How to apply:**
1. Reviewing a migration: grep `fn <producer>() -> <OldType>` plus every call of `NewType::parse(<producer>(...))`. If the producer still returns OldType → REJECT and demand a direct `Source → NewType` mapping.
2. A `parse`/bridge shim may live only inside `#[cfg(test)]` or at a one-time string→enum boundary; it is FORBIDDEN to be the main type-production path.
3. Teeth-driven: poison the producer (e.g. map `"String"→Unknown`) → a production fixture must go RED. Hunting for somewhere to poison is what usually exposes a disguised producer.

Related: [[feedback-poison-must-be-red]] (the same verification spirit), [[feedback-collaboration-loop]] (O is the checkpoint), [[mentor-o-persona]].
