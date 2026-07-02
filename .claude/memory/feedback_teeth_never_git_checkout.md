---
name: feedback-teeth-never-git-checkout
description: "TEETH ritual: NEVER `git checkout <file>` to undo a teeth edit when the file has uncommitted work — it reverts to HEAD and DESTROYS the author's unstaged changes. Snapshot to /tmp first."
metadata:
  node_type: memory
  type: feedback
  originSessionId: 4aa6e5c2-24e4-4456-9ddd-354c21dc684f
---

**2026-06-08 — tôi (Mentor O) gây mất dữ liệu thật.** Trong lúc teeth-verify
ADR-0045 (gỡ guard `Deinit` ở `triet-lower/src/lib.rs` để xác nhận double-free
regression đỏ), tôi khôi phục bằng `git checkout crates/triet-lower/src/lib.rs`.
Lệnh đó revert file về **HEAD (đã commit)**, KHÔNG phải về bản working-tree của
author. Toàn bộ việc lower của author (B1 type_name reference + simple_is_copy,
B2 push_owned guard, B3 to_zero borrow-skip, wiring `length`→shim) **CHƯA
commit** → bị xóa sạch. Không cứu được: unstaged nên không có blob; dangling
blob của fsck không chứa nó; không có editor swap.

**Why:** `git checkout <path>` = "restore từ index/HEAD", phá mọi sửa chưa staged.
Teeth là thao tác cố ý phá-rồi-khôi-phục trên CHÍNH file author đang sửa dở —
đúng tình huống nguy hiểm nhất cho lệnh này.

**How to apply — quy tắc teeth mới (bổ sung [[mentor_o_persona]] nghi thức 2):**
1. TRƯỚC khi sửa file để teeth: `cp <file> /tmp/teeth_backup.rs` (snapshot bản
   working-tree thật của author).
2. Sửa → build → chạy → xác nhận đỏ.
3. Khôi phục bằng `cp /tmp/teeth_backup.rs <file>` HOẶC bằng Edit đảo đúng đoạn
   đã sửa — **KHÔNG BAO GIỜ** `git checkout`/`git restore`/`git stash` trên file
   có uncommitted work.
4. Nếu lỡ tay: dừng ngay, `git fsck --lost-found`, kiểm editor backup, báo author
   thẳng — không tự dựng lại code hộ (vai mentor + sẽ đoán sai form).

Hệ quả lần này: mọi file ADR-0045 KHÁC còn sống (checker, mir is_copy, typecheck
env/check/error, driver/main); chỉ `lower/lib.rs` mất → author phải re-apply
riêng phần lower.

**2026-07-02 — D vi phạm luật này (KHÔNG phải O).** WO-Outcome-param-ABI: D dùng
`git stash`/`stash pop` để so pre/post-fix thay vì cp-snapshot. Kết quả tình cờ
đúng (O verify lại độc lập bằng cp, ra cùng kết luận RED→GREEN), nhưng G ghi sổ
đen cảnh cáo: "lần sau còn vi phạm, WO vứt sọt rác khỏi cần đọc". Luật áp cho
CẢ D, không chỉ O — bất kỳ ai teeth-verify trên file có uncommitted work.
