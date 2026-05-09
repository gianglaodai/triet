# 0006. Ternary Packaging and Versioning Vision

## Ngày ban hành
2026-05-10

## Trạng thái
**Accepted (Tầm nhìn dài hạn cho v0.4 / v0.5)**

## Bối cảnh

Trong quá trình phát triển hệ thống module (v0.2.x) và chuẩn bị cho Trụ cột 3.1 (CAS Packaging), một lập luận triết học về việc ứng dụng **logic tam phân cân bằng (Balanced Ternary: -1, 0, +1)** vào kiến trúc quản lý package (Packaging) và Versioning đã được đưa ra. Triết lý của ngôn ngữ Triết không chỉ là sử dụng tam phân ở mức toán học hay logic (boolean), mà còn ở kiến trúc phần mềm.

Tài liệu này ghi nhận lại ý tưởng đột phá đó, định hướng cách Triết sẽ quản lý version và dependency trong tương lai, đồng thời ghi chú lại các lo ngại về mặt kỹ thuật để đối chiếu khi chúng ta thực sự code hệ thống Package Manager.

## Luận điểm & Quyết định thiết kế

### 1. Radix Economy (Hiệu suất cơ số)
**Lập luận ban đầu:** Về mặt toán học, cơ số 3 (gần với số $e$) cho hiệu năng biểu diễn thông tin tốt nhất. Do đó, hệ tam phân quản lý được một không gian định danh lớn hơn và gọn gàng hơn hệ nhị phân.
**Đánh giá & Định hướng:** Đúng về mặt lý thuyết thông tin. Điều này ủng hộ mạnh mẽ cho kiến trúc **CAS (Content-Addressable Storage)** của Triết. Khi chúng ta băm (hash) các gói thư viện, không gian địa chỉ được biểu diễn (hoặc nén) dưới dạng tam phân sẽ mang lại hiệu suất biểu diễn tối ưu.

### 2. Versioning theo "Trạng thái ổn định"
**Lập luận ban đầu:** Thay vì SemVer (1.2.3) truyền thống, phiên bản của Triết mang ý nghĩa về bản chất sự thay đổi thông qua 3 trạng thái:
- `0 (Neutral)`: Phiên bản baseline ổn định.
- `+1 (Positive)`: Bản mở rộng, thêm tính năng mới.
- `-1 (Negative)`: Bản refactor, dọn dẹp, tối ưu hóa (giữ nguyên API).

**Đánh giá & Định hướng:** Đây là một ý tưởng **đột phá**. Nó biến Versioning từ định lượng thành **định tính (Semantic Intent)**. 
- **Quyết định:** Package Manager của Triết (ở v0.5) sẽ sử dụng mô hình versioning dựa trên Vector Tam Phân kết hợp CAS. Một phiên bản sẽ là chuỗi các quyết định: `[Hash của bản gốc, +1, +1, -1, 0]`. Trình quản lý gói và AI sẽ tự động hiểu được thư viện này đang phình to ra hay đang được gọt giũa.

### 3. Phân cấp Cây ba nhánh (Ternary Tree) cho Module
**Lập luận ban đầu:** Thay vì cấu trúc cây thư mục tùy ý, namespace được chia thành 3 nhánh logic tự nhiên:
- `Nhánh giữa (0)`: Core logic.
- `Nhánh phải (+1)`: High-level API, extensions.
- `Nhánh trái (-1)`: Low-level, hardware driver.
Ví dụ: `sys.io.0`, `sys.io.+1`, `sys.io.-1`.

**Đánh giá & Định hướng:** Một mô hình đối xứng hoàn hảo. Nó liên kết trực tiếp với **Trụ cột 3.5 (Capability System)**.
- **Ghi chú kỹ thuật (Lo ngại):** Đặt tên thư mục hoặc module là `sys.io.-1` sẽ gây khó đọc cho lập trình viên (User ergonomics). 
- **Giải pháp:** Cấu trúc Ternary Tree này sẽ được áp dụng như một **ràng buộc về mặt siêu dữ liệu (metadata)** thay vì tên gọi vật lý. Chúng ta sẽ giới thiệu cú pháp chỉ định "tầng": `module sys.io (layer: -1)`. Từ đó compiler tự động siết chặt Capability (ví dụ tầng `-1` bắt buộc người dùng phải có Explicit Grant mới được gọi).

### 4. Giải quyết xung đột Dependency bằng Ternary CMP
**Lập luận ban đầu:** Hàm so sánh tam phân `CMP(a, b)` trả về `-1, 0, 1` có thể giúp Package Manager tự động giải quyết các node phụ thuộc cực nhanh mà không cần duyệt cây phức tạp, vì bản thân version đã mang tính so sánh.

**Ghi chú kỹ thuật (Lo ngại cốt lõi):**
Bài toán phân giải gói (Dependency Resolution) về mặt lý thuyết đồ thị là một bài toán SAT (thỏa mãn ràng buộc), việc chỉ có toán tử `<=>` 3 chiều không phá vỡ được giới hạn toán học để chuyển SAT thành bài toán $O(1)$. 
*Tuy nhiên*, sự kết hợp giữa **Ternary Vector Versioning** (điểm 2) và tìm kiếm **Trạng thái ổn định (0)** có thể cho phép chúng ta thay thế thuật toán duyệt đồ thị truyền thống bằng **Ternary Search Tree**. Thay vì tìm kiếm "phiên bản tương thích lớn nhất" (như Cargo/NPM), Triết sẽ tìm "phiên bản có trạng thái 0 gần nhất với Hash yêu cầu". Điều này có tiềm năng cắt giảm độ phức tạp tính toán rất lớn.

## Hệ quả
Tài liệu này không tác động trực tiếp đến mã nguồn của Phase v0.2 hiện tại. Nó đóng vai trò là "North Star" (kim chỉ nam) để nhắc nhở các AI và các nhà phát triển sau này khi kiến trúc hệ thống Package Manager (v0.5) và Capability (v0.6) được thiết kế. Không được phép thiết kế SemVer nhị phân truyền thống cho ngôn ngữ Triết.
