//! Triết parser: token stream → AST.
//!
//! Recursive descent parser với error recovery để diagnostics đẹp.
//! AI-first: lỗi compile cụ thể, dẫn dắt LLM/dev sửa đúng.

#![warn(missing_docs)]
