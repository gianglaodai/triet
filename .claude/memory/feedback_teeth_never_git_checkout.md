---
name: feedback-teeth-never-git-checkout
description: "TEETH ritual: NEVER `git checkout <file>` to undo a teeth edit when the file has uncommitted work — it reverts to HEAD and DESTROYS the author's unstaged changes. Snapshot to /tmp first."
metadata:
  node_type: memory
  type: feedback
  originSessionId: 4aa6e5c2-24e4-4456-9ddd-354c21dc684f
---

**2026-06-08 — I (Mentor O) caused real data loss.** While teeth-verifying
ADR-0045 (removing the `Deinit` guard in `triet-lower/src/lib.rs` to confirm the
double-free regression went red), I restored the file with
`git checkout crates/triet-lower/src/lib.rs`. That command reverts the file to
**HEAD (committed)**, NOT to the author's working-tree version. All of the
author's lowering work (B1 type_name reference + simple_is_copy, B2 push_owned
guard, B3 to_zero borrow-skip, wiring `length`→shim) was **UNCOMMITTED** → wiped
out. Unrecoverable: unstaged means no blob; fsck's dangling blobs did not contain
it; there was no editor swap file.

**Why:** `git checkout <path>` means "restore from the index/HEAD" and destroys
every unstaged edit. Teeth work is a deliberate break-then-restore operation on
THE VERY FILE the author is midway through editing — precisely the most dangerous
situation for that command.

**How to apply — the new teeth rule (extending [[mentor_o_persona]] ritual 2):**
1. BEFORE editing a file for teeth: `cp <file> /tmp/teeth_backup.rs` (snapshot the
   author's real working-tree version).
2. Edit → build → run → confirm red.
3. Restore with `cp /tmp/teeth_backup.rs <file>` OR by using Edit to reverse exactly
   the passage you changed — **NEVER** `git checkout`/`git restore`/`git stash` on a
   file with uncommitted work.
4. If you slip: stop immediately, run `git fsck --lost-found`, check for editor
   backups, and tell the author straight — do not rebuild the code for them (that is
   the mentor role, and you will guess the shape wrong).

The consequence that time: every OTHER ADR-0045 file survived (checker, mir is_copy,
typecheck env/check/error, driver/main); only `lower/lib.rs` was lost → the author
had to re-apply the lowering part alone.

**2026-07-02 — D violated this law (not O).** In WO-Outcome-param-ABI, D used
`git stash`/`stash pop` to compare pre and post fix instead of a cp snapshot. The
result happened to be correct (O re-verified independently with cp and reached the
same RED→GREEN conclusion), but G recorded a black mark: "violate it again and the
WO goes in the bin unread". The law applies to D as well as O — to anyone doing
teeth verification on a file with uncommitted work.
