# ADR 0022 — Trit-balanced ownership (research draft)

**Trạng thái:** **Exploratory draft. KHÔNG lock.** Deferred research — sẽ nghiên cứu sâu sau khi v0.7 self-hosting compiler hoàn tất. Mục đích duy nhất của ADR này: ghi nhận hướng tiếp cận trước khi context bay mất; cung cấp điểm xuất phát cho phase nghiên cứu sau.

**Origin:** Thảo luận giữa Giang Hoàng và AI assistant 2026-05-21, sau khi v0.7.5.2 lands. Trực giác xuất phát từ author: *"tam phân cân bằng +1, 0, -1 nó có thể đại diện cho 1 đối tượng đang tham chiếu đến 1 đối tượng khác, không bị tham chiếu, và bị đối tượng khác tham chiếu đến"*.

## §1 — Vấn đề nền tảng

Cấu trúc dữ liệu có **chu trình tham chiếu** (Doubly-Linked List, Graph, Tree với parent-ref, Observer pattern) là vấn đề kinh điển của memory management. Trong **mọi ngôn ngữ system-level**, cần một trong 3 cơ chế:

| Cơ chế | Ngôn ngữ ví dụ | Đặc trưng |
|---|---|---|
| **Garbage Collector (GC)** | Java, C#, Go, OCaml, Python | Runtime quét tìm node unreachable. Cycle không thành vấn đề vì mark-sweep không dùng refcount. Trả giá bằng pause time + heap overhead. |
| **Reference Count + Weak** | Swift, Rust (`Rc`/`Weak`), Obj-C | Strong ref tăng counter; weak không. Programmer **tự đánh dấu** ref nào weak. Cycle leak nếu khai báo sai (compiler không bắt). |
| **Manual / unsafe** | C, C++, Rust unsafe | Programmer tự manage. Không có safety net. |

Triết hiện chưa cam kết với cơ chế nào. v0.7 dùng **arena pattern** (Vector + Integer index) — đây là cách **bypass** vấn đề, không phải giải. Arena hoạt động tốt cho compiler AST nhưng không phù hợp cho stdlib data structures (LinkedList, Graph) khi user mong đợi API pointer-style.

VISION §3 ("Bản sắc tam phân") đặt câu hỏi: liệu **tam phân cân bằng** có thể đóng góp **cơ chế thứ 4** không?

## §2 — Đề xuất: định luật bảo toàn ownership

**Trực giác cốt lõi:** Trong tam phân cân bằng, mọi tổng đối xứng quanh 0. Cặp `+1` và `-1` triệt tiêu. Áp dụng cho references:

- **`~+ T`** (strong / owning ref): mang ownership. Đóng góp `+1` vào type-ref graph.
- **`~- T`** (weak / observing ref): không mang ownership. Đóng góp `-1`.
- **`~0 T`** (no ref): không có quan hệ. Đóng góp `0`. (Có thể trùng với `T?` zero state.)

**Định luật bảo toàn (CONJECTURE — chưa proof):**

> Trong type-ref graph của một chương trình, **mọi cycle phải có tổng trit = 0**.
>
> Cycle với tổng ≠ 0 là compile error.

Lý do trực giác: cycle tổng `> 0` → có nhiều strong ref hơn weak → khi root drop, vẫn còn strong ref giữ alive → leak. Cycle tổng `< 0` → có weak ref dangling không gắn với strong → use-after-free risk. **Cycle tổng = 0**: chính xác 1 strong path mang ownership, các path còn lại chỉ observe → có thể reclaim cleanly.

## §3 — Worked example: Doubly-Linked List

```triet
public struct Node<T> {
    value: T,
    next: ~+ Node<T>?,    // +1: own next node
    prev: ~- Node<T>?,    // -1: observe prev node (không own)
}

public struct DoublyLinkedList<T> {
    head: ~+ Node<T>?,    // +1: list owns chain qua head
    tail: ~- Node<T>?,    // -1: tail chỉ là weak — chain own qua head.next.next...
}
```

Type-ref graph:

```
DoublyLinkedList ──(~+)──> Node
DoublyLinkedList ──(~-)──> Node     [cycle 1 không tồn tại — DLL không bị tham chiếu lại]

Node ──(~+ next)──> Node            ┐
                                    ├─── cycle 2: +1 + -1 = 0 ✓
Node ──(~- prev)──> Node            ┘
```

Cycle 2 (qua next/prev) cân bằng → compile passes.

**Runtime drop semantics:**

1. Khi `DoublyLinkedList` drop:
   - `head: ~+` → giảm strong count của node đầu chuỗi → 0 → drop node đầu.
   - `tail: ~-` → clear pointer, không ảnh hưởng count.
2. Khi node A drop (do head ref bị giảm):
   - `A.next: ~+` → giảm strong count của B → 0 → drop B (đệ quy).
   - `A.prev: ~-` → clear pointer, không ảnh hưởng (A đã drop trước rồi nên prev không quan tâm).
3. Chain reclaim theo thứ tự, không leak.

**So sánh với Rust `Rc<RefCell<>>`:**

```rust
// Rust idiomatic — programmer phải nhớ Weak cho prev
struct Node<T> {
    value: T,
    next: Option<Rc<RefCell<Node<T>>>>,
    prev: Option<Weak<RefCell<Node<T>>>>,   // nếu quên Weak → leak silent
}
```

Triết-trit-balanced **bắt nhầm tại compile time**, Rust không bắt.

## §4 — So sánh 4 cơ chế

| | GC | Rc/Weak | Manual | **Trit-balanced** |
|---|---|---|---|---|
| Runtime overhead | Cao (mark-sweep) | Trung (refcount) | Không | Trung (refcount theo trit) |
| Programmer burden | Không | Phải nhớ đánh weak | Cao | Đánh `~+` / `~-` ở khai báo type |
| Cycle leak detection | Không (mark-sweep tự reclaim) | **Không bắt** — programmer phải đúng | Không bắt | **Compile error** nếu cycle ≠ 0 |
| Pause time | GC pause | Không | Không | Không |
| Deterministic drop | Không | Có | Có | Có |

Đặc trưng độc đáo: **cycle leak là compile error**. GC không cần bắt vì tự reclaim; Rc/Weak không bắt được; Triết-trit-balanced bắt được nhờ static analysis trên type-ref graph.

## §5 — Câu hỏi mở (cần research phase sau v0.7)

Đây là draft, **chưa proof**. Các câu hỏi cần trả lời trước khi promote thành Locked ADR:

### §5.1 — Lý thuyết

1. **Proof: cycle-balance ⟹ no-leak.** Liệu định luật "tổng cycle = 0" có thực sự đủ điều kiện để guarantee no-leak? Có counterexample không? (Ví dụ: 2 cycle chồng nhau qua một node chung.)
2. **Cycle bậc cao.** Cycle A→B→C→A (3 cạnh). Có thể phân `+1, +1, -2`? Hay phải `+1, 0, -1`? Hay không cho phép trit ngoài {-1, 0, +1}? Cần lock rule.
3. **Cycle bậc rất cao** (>10 node). Static analysis có khả thi không trên codebase lớn? Polynomial hay exponential?
4. **Self-cycle** (struct chứa ref tới chính nó). Tổng phải = 0 → buộc phải `~-`. Có thể là feature: cấm self-strong-ref. Cần xác nhận.
5. **Quan hệ với linear types / separation logic / ownership types** (Pony, Mezzo, ATS, Cyclone). Trit-balanced có phải special case của một lý thuyết đã có không?

### §5.2 — Thực hành

6. **Weak ref dereference khi target đã drop.** Trả về gì? `null`? Panic? `Trit::Zero`? — Đề xuất hiện tại: trả `T?` với null state, force-unwrap panic. Cần spec.
7. **Move semantics.** Di chuyển node giữa 2 lists — ownership transfer thế nào? Có phải `move` keyword như Rust?
8. **Mutate ownership.** `node.next = other_node` — strong ref bị overwrite thì target cũ drop ngay hay đợi cuối scope?
9. **Concurrent access.** Cycle-balance check là single-threaded reasoning. Multi-thread cần atomic refcount + Send/Sync analog?
10. **Tương tác với generics.** `Vector<~+ Node>` — strong ref nằm trong Vector, Vector own ai? Có cần "ownership variance"?
11. **Tương tác với Outcome.** `~+ Node ~? |e| ~- e` — `~+` ở đây là Trit::Positive (Outcome arm) hay strong ref? Cần resolve syntax conflict.

### §5.3 — Migration

12. **Arena hiện tại của parser.tri.** Khi promote trit-balanced, có cần rewrite Arena pattern không? Hay arena vẫn là tùy chọn hợp lệ song song?
13. **`Rc`-style fallback.** Nếu cycle quá phức tạp không thể static-balance, có cần fallback runtime refcount như Rust `Rc`? — Khả năng cần tier 2 (runtime-checked) cho cases compile-time không quyết được.

## §6 — Prior art (chưa exhaustive)

Sẽ research kỹ ở phase nghiên cứu. Sketch ban đầu:

- **Linear types** (Wadler 1990, Linear Haskell): mỗi value dùng đúng 1 lần. Khác trit-balanced ở chỗ không cho phép multi-ref. Trit-balanced cho phép +1 strong + nhiều -1 weak.
- **Ownership types** (Clarke et al. 1998): annotate ownership domain. Cùng tinh thần nhưng chưa dùng polarity tam phân.
- **Rust `Rc`/`Weak`**: runtime mechanism, không phải static check. Trit-balanced đẩy lên compile time.
- **Cyclone / ATS**: static ownership tracking, không có polarity tam phân.
- **Pony / Mezzo reference capabilities**: rich annotation system, chưa có cấu trúc cân bằng quanh 0.
- **Cycle detection algorithms** (mark-sweep variants, Bacon-Rajan): runtime cycle reclaim. Trit-balanced thay bằng compile-time prevention.

**Câu hỏi mở:** đã có ai publish "polarity-typed references" hoặc "balanced ternary ownership" chưa? Author tin là chưa (Triết là dự án duy nhất nghiên cứu tam phân cân bằng nghiêm túc trong recent decades), nhưng cần xác nhận literature search.

## §7 — Tại sao defer đến post-v0.7

1. **v0.7 chưa cần.** Self-hosting compiler dùng arena pattern hoàn toàn cho AST, IR, modules. Không có data structure cyclic trong compiler tự nó.
2. **Spec lock = irreversible.** Memory model là quyết định v1.0-level. Lock sai → migration đau đớn (xem ADR-0011 abi_version migration). Phải đảm bảo lý thuyết solid trước khi lock.
3. **Cần literature search.** Trit-balanced có thể là special case của ownership type system đã publish — research phase cần tìm ra trước khi tự nhận là "novel".
4. **Cần prototype.** Trước khi promote, cần thực sự implement một prototype trên Triết v0.8 (sau khi self-hosting xong) để xem corner cases.

**Earliest research window:** v0.7.13 (verify gate). **Earliest implementation window:** v0.8.x. **Earliest spec lock:** v1.0.

## §8 — Trạng thái draft + next steps

- **Trạng thái:** Exploratory. KHÔNG implementation. KHÔNG syntax reserved. ADR có thể bị reject hoàn toàn sau research phase.
- **Khi v0.7 hoàn tất:** mở "v0.8.x.research-ownership" phase.
  1. Literature search 30 ngày — tìm polarity types, ownership systems, separation logic.
  2. Formal proof attempt — cycle-balance ⟹ no-leak.
  3. Prototype trên Triết v0.8 — implement DLL + Graph + Tree-with-parent.
  4. Decision point: promote thành Locked ADR HOẶC reject + chọn cơ chế khác (GC / Rc/Weak / hybrid).
- **Không làm trong v0.7:** không reserve syntax `~+ T` / `~- T` (xung đột với ADR-0020 Outcome arms — phải resolve khi promote).

## Liên kết

- [VISION.md §3](../../VISION.md) — Bản sắc tam phân
- [ADR-0010](0010-ternary-native-ir.md) — Ternary-native IR (precedent: tam phân = mechanism, không chỉ data)
- [ADR-0020](0020-outcome-error-handling.md) — Outcome (precedent: trit-polarity ở value level)
- [ADR-0021](0021-trilean-refinement.md) — Trilean! refinement (precedent: compile-time type refinement)
