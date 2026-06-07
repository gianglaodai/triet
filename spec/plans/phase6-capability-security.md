# Phase 6 — Capability Security via S6 Ownership

**Status:** Design only — not implemented (2026-06-04)
**See also:** `TODO.md` (live backlog + debt registry). REPORT-2026-06-04.md đã xóa — git history giữ.

**Dependency note:** Phase numbering ≠ build order. Capability security depends on
Phase 5 (S6 ownership) and Phase 2 (borrowck). Hardware Token pattern requires
ZST compile-time tokens enforced by the borrow checker — not yet built.
**Phụ thuộc:** Phase 5 (S6 Ownership Integration), Phase 2 (borrow checker)

---

## 1. Triết lý: Capability = Ownership + Move Semantics

Capability security truyền thống dùng một trong hai cơ chế:
- **ACL (Access Control List):** OS duy trì bảng quyền, kiểm tra runtime → chậm, tốn RAM, kernel complexity.
- **User/Kernel mode switch:** CPU chuyển giữa Ring 0 và Ring 3 qua syscall → tốn ~100-1000 CPU cycles mỗi lần.

**Triết dùng cơ chế thứ ba: Ownership + Move Semantics.**

Nếu một tài nguyên chỉ có **đúng 1 owner** tại mọi thời điểm, và việc chuyển giao tài nguyên **tiêu hủy quyền truy cập của người chuyển**, thì:
- Không cần ACL — ownership chain chính là capability chain.
- Không cần syscall — mọi kiểm tra xảy ra ở compile-time qua borrow checker.
- Không cần kernel mode — code chạy ở Ring 0 với compile-time safety guarantee.

**Nguyên lý:** *"Nếu anh không có con trỏ tới tài nguyên, anh không thể truy cập nó. Và borrow checker đảm bảo anh không thể copy con trỏ đó — chỉ có thể move."*

---

## 2. Hardware Token Pattern (đã chứng minh với S6)

**Nguyên tắc then chốt:** Hardware resource handles là **Zero-Sized Types (ZST)**.
Chúng chỉ tồn tại ở compile-time để Borrow Checker kiểm tra ownership. Ở runtime,
chúng có kích thước = 0 byte — không có gì để copy lên stack. Địa chỉ phần cứng
(vd: 0xB8000 cho VGA) là hằng số trong driver, không phải dữ liệu trong struct.

```triet
// ALL hardware resource types are ZST — compile-time tokens only.
// They carry NO runtime data (no buffers, no addresses).

public struct VgaBuffer {}       // ZST: sizeof = 0
public struct UartPort {}        // ZST: sizeof = 0
public struct InterruptController {} // ZST: sizeof = 0
public struct PhysicalMemory {}  // ZST: sizeof = 0

// The Hardware struct aggregates ZST tokens.
// Since all fields are ZST, Hardware itself is ZST.
// Pass-by-value `hw: Hardware` copies ZERO bytes on the stack.

public struct Hardware {
    vga: VgaBuffer,
    uart: UartPort,
    pic: InterruptController,
    memory: PhysicalMemory,
}

// The bootloader creates the ONE AND ONLY Hardware instance.
// Because it's ZST, this is a compile-time event — zero runtime cost.

public function kernel_main(hw: Hardware) -> Unit {
    // Destructure: move ZST fields out of hw.
    // Each move transfers ownership (compile-time) but copies 0 bytes (runtime).
    let vga: VgaBuffer = hw.vga;
    let uart: UartPort = hw.uart;
    let pic: InterruptController = hw.pic;
    let mem: PhysicalMemory = hw.memory;
    // hw consumed — ZST, no drop code needed

    // Each driver receives a ZST token.
    // The driver hardcodes the hardware address (e.g., VGA = 0xB8000).
    vga_driver(vga);     // vga MOVED — compile-time, 0 bytes
    uart_driver(uart);
    pic_driver(pic);
    memory_manager(mem);

    // vga_driver(vga);  // E2420 UseAfterMove — compile-time error!
}
```

**Tại sao dùng ZST:**
- `VgaBuffer` không chứa địa chỉ 0xB8000 — địa chỉ này là hằng số cứng trong driver
- Nếu `VgaBuffer` chứa buffer thật (vd: `[u8; 4000]`), mỗi lần move copy 4KB → thảm họa
- Với ZST, toàn bộ capability chain có **zero runtime overhead**: không stack copy, không heap allocation, không register spill
- Borrow checker vẫn kiểm tra đầy đủ ownership — nhưng toàn bộ check xảy ra ở compile-time

**Bằng chứng:** Borrow checker từ chối mọi nỗ lực dùng lại tài nguyên đã move.
Không ACL. Không runtime check. Không syscall. **Không copy dữ liệu.**

---

## 3. Delegation (Phân quyền)

Một driver có thể **sub-delegate** một phần tài nguyên cho child.
Có 2 loại token:
- **Static token (ZST, 0 byte):** tài nguyên toàn cục cố định (vd: toàn bộ VGA buffer ở 0xB8000). Không cần metadata vì địa chỉ đã hardcode.
- **Dynamic token (có metadata):** tài nguyên đã được chia cắt (vd: vùng màn hình trái). Chứa metadata nhỏ (offset, length) để child biết phạm vi được phép truy cập. Metadata là VÀI BYTE — không copy buffer gốc.

```triet
// Static token — ZST, địa chỉ VGA hardcode trong driver
public struct VgaBuffer {}

// Dynamic token — BORROWS the buffer, does NOT own it.
// Lifetime of VgaRegion is bound to VgaDriver via &0 mutable.
public struct VgaRegion {
    base: &0 mutable VgaBuffer,  // borrow từ VgaDriver (8 byte)
    offset_x: Integer,           // tọa độ X (8 byte)
    offset_y: Integer,           // tọa độ Y (8 byte)
    width: Integer,              // chiều rộng (8 byte)
    height: Integer,             // chiều cao (8 byte)
}
// sizeof(VgaRegion) = 40 byte metadata — KHÔNG copy buffer

public struct VgaDriver {
    buffer: &+ VgaBuffer,  // unique OWNER của VGA buffer
}

public function VgaDriver::delegate_left_panel(
    self: &0 mutable VgaDriver   // BORROW self, không MOVE
) -> VgaRegion {
    // S6 Lifetime Elision Rule 2: single input borrow → output borrow tied to it.
    // VgaRegion's lifetime is automatically bound to self.
    let region: VgaRegion = VgaRegion {
        base: &0 mutable self.buffer,  // reborrow từ &0 mutable self
        offset_x: 0,
        offset_y: 0,
        width: 40,
        height: 25,
    };
    return region;
    // region returned — its lifetime tied to self via elision Rule 2
    // Caller can use region as long as self is alive
    // self is NOT moved — caller VẪN sở hữu VgaDriver
}

public function kernel_main(hw: Hardware) -> Unit {
    let vga: VgaBuffer = hw.vga;          // ZST token — 0 byte
    let driver: VgaDriver = VgaDriver { buffer: &+ vga };

    let left_panel = driver.delegate_left_panel();
    text_console(left_panel);    // left_panel MOVED — 40 byte metadata

    let right_panel = driver.delegate_right_panel();
    graphics_shell(right_panel); // right_panel MOVED — 40 byte metadata
}
```

**Tính chất quan trọng:**
- Mỗi delegation là **1 move** — compile-time ownership transfer
- **Static token (ZST):** copy 0 byte — dùng cho tài nguyên toàn cục
- **Dynamic token:** copy metadata nhỏ (vài chục byte) — KHÔNG copy buffer gốc
- Child không thể ghi ngoài vùng được delegate (offset + width/height giới hạn)
- Compile-time: mọi vi phạm → E2420/E2440

---

## 4. So sánh với mô hình truyền thống

| Cơ chế | ACL (Unix) | Capability (seL4) | **Triết S6** |
|---|---|---|---|
| Kiểm tra | Runtime (kernel) | Runtime (kernel) | **Compile-time** |
| Chi phí | >1000 cycles/syscall | >100 cycles/syscall | **0 cycles (ZST tokens)** |
| Phân quyền | setuid/getuid | capability transfer | **Move semantics (ZST)** |
| Thu hồi quyền | Không | Có (revocation) | **Tự động khi move (0 byte)** |
| Safety guarantee | Best-effort | Formal verification | **Borrow checker** |

---

## 5. Phase 6 kế hoạch

| Sub-task | Nội dung | Verify |
|---|---|---|
| 6.1 | Viết test `.tri`: Hardware Token destructure → E2420 ngăn double-take | borrowck bắt lỗi |
| 6.2 | Viết test `.tri`: delegation pattern (driver delegate cho child) | borrowck pass |
| 6.3 | Viết `.tri` demo: multi-driver kernel với VGA + UART | toàn bộ pipeline |

---

## 6. Quan hệ với capability namespace (ADR-0016/0017/0018) — defer

Hệ thống `sys.*`/`dev.*`/`usr.*` namespace từ bản nháp v0.6-v0.8 là **tầng trên** — nó kiểm soát việc *khai báo capability* trong package manifest, không phải *kiểm tra ownership* của capability instance.

Hardware Token pattern (Phase 6) là **tầng dưới** — nó kiểm soát *quyền sở hữu tài nguyên* tại runtime qua S6 ownership, compile-time.

Hai tầng này bổ sung cho nhau:
- Namespace: "Driver này có được phép truy cập VGA không?" (policy)
- Ownership: "Driver này có đang giữ con trỏ VGA không?" (mechanism)

Namespace integration được defer đến Phase 7.
