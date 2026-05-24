# ADR 0022 — Trit-balanced Ownership (Con trỏ Tam phân)

**Trạng thái:** **Draft** (Đã phân tích thiết kế cốt lõi, chuẩn bị cho phase v0.8)

**Origin:** Khởi nguồn từ việc Triết cần một hệ thống quản lý bộ nhớ đủ chặt chẽ để viết Hệ điều hành (Kernel/OS) như Rust, nhưng không lặp lại sự phức tạp của Lifetime annotations (`<'a>`) hay sự dễ dãi ẩn giấu của Mojo.

## §1 — Vấn đề nền tảng

Cấu trúc dữ liệu có **chu trình tham chiếu** (Doubly-Linked List, Graph, Tree với parent-ref) là vấn đề kinh điển của system programming. 
Trong Rust, để giải quyết các cấu trúc này, lập trình viên thường phải cầu viện tới `Rc<RefCell<T>>`, `Weak<T>`, hoặc `unsafe`. Cú pháp vòng đời (Lifetime `'a`) của Rust rất mạnh mẽ để tránh Use-After-Free nhưng lại tạo ra rào cản nhận thức cực lớn (viral lifetime). Trong khi đó, các ngôn ngữ dùng Implicit ARC lại làm mất đi đặc tính Zero-cost, khiến chúng không phù hợp để viết Kernel.

**Mục tiêu của Triết:** 
- Đạt được độ chặt chẽ bộ nhớ như Rust (không data races, không memory leak).
- Vượt qua bài toán Lifetime bằng cách thay đổi tận gốc phương pháp tiếp cận: **Sử dụng Con trỏ Tam phân (Ternary Pointers)** kết hợp quy luật chặt chẽ cho mượn bộ nhớ.

## §2 — Đề xuất: Hệ thống Con trỏ Tam phân `&+`, `&0`, `&-`

Hệ thống sẽ dùng ký tự `&` để đánh dấu tham chiếu. Một con trỏ trong Triết có 3 trạng thái rành mạch tại Compile-time:

- **`&+ T` (Strong Owner / Chủ sở hữu):** Mang quyền sở hữu (`+1`). Vùng nhớ tuyệt đối không thể bị giải phóng khi `&+` còn sống. Tương đương `Arc<T>` / `Box<T>`.
- **`&- T` (Weak Observer / Kẻ quan sát):** Liên kết yếu (`-1`). Không bảo vệ vùng nhớ. Trình biên dịch bắt buộc lập trình viên phải kiểm tra sự tồn tại (upgrade) trước khi truy cập. Tránh chu trình. Tương đương `Weak<T>`.
- **`&0 T` (Neutral Borrow / Mượn trung lập):** Mượn tạm thời (`0`). Tham chiếu chỉ tồn tại trong một Scope (ví dụ: tham số hàm). Không làm thay đổi Reference Count ở Runtime (Zero-cost abstraction). Tương đương `&T` trong Rust.

**Hai Thiết quân luật (Strict Rules) của Trình biên dịch:**

1. **Luật Cấm `&0` trong Struct:** Con trỏ mượn trung lập `&0` **không bao giờ được lưu trữ vào Struct**. Nó chỉ được phép xuất hiện ở ranh giới gọi hàm (Function parameters/returns).
2. **Luật Ngắt Chu trình (Cycle Breaking):** Trình biên dịch cấm việc tạo ra các chu trình khép kín (vòng lặp) bằng toàn bộ con trỏ `&+` (sẽ tạo memory leak). Bạn buộc phải chèn ít nhất một con trỏ `&-` (Weak) để phá vỡ cấu trúc khép kín, bảo đảm tính có hướng của đồ thị bộ nhớ.
   > **Lưu ý:** Sự "Bao hàm" (Containment) không phải là Chu trình. Nếu Object A chứa Object B (quan hệ một chiều), bạn chỉ cần dùng `&+` để A sở hữu B. Luật này chỉ kích hoạt khi các object liên kết với nhau tạo thành một vòng tròn khép kín (ví dụ: A sở hữu B, B trỏ ngược lại A).

Nhờ 2 quy tắc này, Triết **loại bỏ hoàn toàn khái niệm Lifetime (`'a`)**. Các cấu trúc dữ liệu bắt buộc phải tự quản lý vòng đời rõ ràng qua `&+` và `&-`. Các tham chiếu mượn `&0` tự động bị giới hạn bởi hàm đang thực thi (Borrow Elision).

## §3 — 3 Bài Test Giới Hạn (Litmus Tests) chứng minh sức mạnh ở mức Kernel

Để chứng minh hệ thống này thay thế được Rust `lifetime` và `unsafe` khi viết HĐH, chúng ta xét 3 trường hợp:

### Test 1: Cây Tiến trình & Trình lập lịch (Doubly-Linked Process Tree)
**Thử thách:** Quản lý hàng đợi Scheduler, Process Cha và Process Con (Đồ thị có chu trình).
**Giải pháp Triết:**
```triet
public struct Process {
    // Process Cha sở hữu các con (+1)
    children: Vector<&+ Process>,  

    // Process Con trỏ ngược lên Cha, dùng Kẻ quan sát yếu (-1) để phá vỡ chu trình
    parent: &- Process?,           

    // Scheduler trỏ tới Process tiếp theo (Weak)
    next_in_queue: &- Process?
}
```
*Kết quả:* Trình biên dịch duyệt Graph và thấy không có chu trình `&+` khép kín nào (đã bị ngắt bởi `&-` của parent). Không Memory Leak, không tốn Mutex, giải quyết thanh thoát không cần `unsafe`.

### Test 2: Cấu trúc tham chiếu nội tại (Self-Referential Network Packet)
**Thử thách:** Một struct vừa chứa bộ đệm (Buffer), vừa chứa con trỏ trỏ thẳng vào giữa bộ đệm đó để đọc Header. Rust cấm điều này mà không có `Pin` / `unsafe`.
**Giải pháp Triết:** Nhờ luật cấm `&0` trong struct, lập trình viên bị ép phải thiết kế theo Data-Oriented.
```triet
public struct NetworkPacket {
    buffer: &+ Vector<Tryte>,  // Sở hữu byte array
    header_offset: Integer,    // Chỉ lưu vị trí, cấm lưu con trỏ &0
}

// Khi cần đọc, sinh ra &0 ngay lúc bay (Zero-cost)
public function get_header(packet: &0 NetworkPacket) -> &0 Header {
    return packet.buffer.slice(packet.header_offset)
}
```

### Test 3: Truy xuất Thanh ghi Vật lý (Memory-Mapped I/O)
**Thử thách:** Đọc ghi vào địa chỉ RAM vật lý mà không có `unsafe`.
**Giải pháp Triết:** Thay `unsafe` bằng **Capability Tam phân** (`sys::` / `dev::`).
```triet
public function blink_led(reg: &0 HardwareRegister) {
    // Hàm này chỉ biên dịch thành công nếu module sở hữu Capability +1 (sys/dev)
    // Con trỏ reg là &0 (mượn không chi phí)
    sys::write_memory(reg.address, 0xFF) 
}
```

## §4 — Sự phân tầng Mặc định (Sensible Defaults) ở Application

Để giữ sự thân thiện (Developer Experience) ở tầng ứng dụng:
- Trong namespace `usr::`, nếu không ghi ký hiệu:
  - Khai báo trong Struct → Mặc định hiểu là `&+`
  - Khai báo ở Tham số hàm → Mặc định hiểu là `&0`
- Khi code Kernel (`sys::`, `dev::`), mọi mặc định bị vô hiệu, bắt buộc lập trình viên phải gõ `&+`, `&0`, `&-` rõ ràng để kiểm soát từng byte và từng chu kỳ CPU.

## §5 — Next Steps (Sau v0.7)
1. Cập nhật Parser/Lexer để hỗ trợ cú pháp `&+`, `&0`, `&-`.
2. Nghiên cứu thực tế đồ thị tham chiếu với `&+` và `&-` trên các bài toán cấu trúc dữ liệu phức tạp.
3. Cài đặt thuật toán phân tích Borrow/Elision cho `&0`.
