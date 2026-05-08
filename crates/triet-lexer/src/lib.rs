//! Triết lexer: tokenize source code thành stream of tokens.
//!
//! Hỗ trợ ternary literal (`0t+0-+`), type-suffixed numbers (`5_tryte`),
//! Bool3 keywords, identifier với Unicode (cho phép tiếng Việt), f-string,
//! và toàn bộ operators của Triết. Tham chiếu [`SPEC.md`] §1.

#![warn(missing_docs)]
