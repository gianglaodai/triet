//! Triết type checker: verify types và infer khi annotation thiếu.
//!
//! Bidirectional type checking đơn giản hóa. Đặc biệt theo dõi nullable
//! `T?` riêng biệt với plain `T` — bắt buộc dev xử lý null trước khi dùng.

#![warn(missing_docs)]
