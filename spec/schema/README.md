# Triết Schema — Code Generation

## Nguyên tắc

**ĐÂY LÀ SINGLE SOURCE OF TRUTH.** Mọi thay đổi về type system, AST, hay
ownership model phải bắt đầu từ `triet-schema.yaml`. Sau khi sửa schema,
chạy code generator để cập nhật Rust source. **Không được viết tay struct
định nghĩa type/AST trong Rust code.**

Lý do: schema-driven ngăn chặn byte-drift giữa Rust compiler host và Triết
self-host compiler (sau này). Một file schema → sinh code cho cả Rust lẫn
Triết. Không còn "quên sync 1 trong 2 bên" như mentor đã cảnh báo.

## Cách dùng

```bash
# Sinh code Rust từ schema
python3 spec/schema/codegen.py --target rust --schema spec/schema/triet-schema.yaml

# Sinh code Triết từ schema (khi ngôn ngữ đạt 1.0)
python3 spec/schema/codegen.py --target triet --schema spec/schema/triet-schema.yaml

# Kiểm tra schema hợp lệ
python3 spec/schema/codegen.py --validate spec/schema/triet-schema.yaml
```

## Sinh ra những gì?

Từ schema, code generator sinh ra:

| File | Nội dung |
|---|---|
| `types.rs` | `enum Type`, `enum PrimitiveType`, `enum ReferenceForm`, ... |
| `ast_expr.rs` | `enum Expr` với tất cả variants |
| `ast_stmt.rs` | `enum Stmt` |
| `ast_item.rs` | `enum Item`, `struct FunctionDef`, `struct StructDef`, ... |
| `ast_operator.rs` | `enum BinaryOperator`, `enum UnaryOperator` |
| `visitor.rs` | `trait ExprVisitor`, `trait TypeVisitor`, ... |
| `display.rs` | `impl Display` cho tất cả types |
| `serde_impl.rs` | `Serialize`/`Deserialize` implementations |

## Cấu trúc schema

```yaml
{
  "definitions": {
    "TypeName": {
      "kind": "enum|struct|specification",
      "variants": [...],     // cho enum
      "fields": [...],       // cho struct
      "description": "..."
    }
  }
}
```

Mỗi definition có:
- `kind`: `enum` | `struct` | `type_constructor` | `specification`
- `variants`: danh sách variant (enum)
- `fields`: danh sách field (struct)
- `description`: mô tả bằng tiếng Anh

Mỗi field có:
- `name`: tên field
- `type`: kiểu dữ liệu (Rust type name hoặc reference tới definition khác)
- `ownership`: `owned` | `borrow` | `move` — S6 ownership annotation
- `description`: mô tả

## Thêm một AST node mới

1. Sửa `triet-schema.yaml` — thêm variant vào `Expr` hoặc `Stmt`
2. Chạy `codegen.py --target rust`
3. Build: `cargo build` — codegen output biên dịch
4. Cập nhật parser/typecheck/lowerer để xử lý variant mới
5. Viết tests
6. Commit cả schema lẫn generated code

## Quy tắc bất di bất dịch

1. **Schema first, code sau.** Không bao giờ thêm variant vào `Type` hay `Expr` trong Rust code trước.
2. **Generated code không sửa tay.** Nếu generated code có vấn đề, sửa codegen, không sửa output.
3. **Schema là documentation.** Mọi description trong schema phải đầy đủ để người mới hiểu được ngữ nghĩa.
4. **Ownership annotation trên mọi field.** Mỗi field có kiểu phức hợp phải ghi rõ `owned`, `borrow`, hay `move`.
