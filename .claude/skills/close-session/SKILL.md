---
name: close-session
description: Thủ tục đóng phiên Triết — verify git sạch+synced, đồng bộ memory bàn giao (Mentor O + Đồng nghiệp D), cập nhật state+persona Mentor G (spec/plans/MENTOR_G_STATE.md — file repo cho model non-Claude), xuất 3 prompt khởi động phiên mới ĐỘC LẬP (O+D+G), và dọn session cũ (giữ session hiện tại). Dùng khi tôi nói "đóng phiên".
trigger: /close-session
argument-hint: "(không cần arg) — /close-session"
---

# /close-session — Thủ tục đóng phiên Triết

Đóng phiên làm việc gọn gàng: chốt trạng thái, lưu memory bàn giao, đẻ 2 prompt copy-paste
để phiên sau (Mentor O hoặc Đồng nghiệp D) vào việc đúng vai, đúng kỷ luật, không mất mạch.

**Nguyên tắc:** KHÔNG commit/push gì trong lúc đóng phiên trừ khi user lệnh rõ. Chỉ đo + ghi memory
+ xuất prompt. Mọi con số (HEAD, gate) phải ĐO THẬT, không chép tay.

## Bước 1 — Verify trạng thái (đo, không tin)
```bash
git status -sb | head -1          # synced? ahead/behind? dirty?
git log --oneline -5
git log --oneline origin/main..HEAD   # còn commit lơ lửng local?
```
- Nếu **dirty** (uncommitted) hoặc **ahead origin** (chưa push): NÊU RÕ trong báo cáo đóng phiên +
  hỏi user có muốn commit/push trước khi đóng không. ĐỪNG tự push.
- Gate: nếu phiên vừa chạy gate thì trích con số cuối; nếu nghi ngờ, chạy lại
  `bash scripts/gate.sh 2>&1 | tail -25` (hoặc build+clippy+test) và dán raw.
- Liệt kê ADR mới LOCKED trong phiên: `git log --oneline origin/main | grep -iE "00[0-9][0-9]" | head`.

## Bước 2 — Đồng bộ memory bàn giao
File memory SỐNG ở `~/.claude/projects/<project-slug>/memory/` (auto-memory máy-local). Cập nhật (KHÔNG tạo trùng — sửa file đang có):
1. **MEMORY.md** — dòng index ĐẦU TIÊN (## Project context): viết MỘT entry mới phản ánh trạng thái
   ĐÓNG PHIÊN: ngày, `origin HEAD`, gate, các campaign ĐÓNG+PUSH trong phiên, nợ-còn-treo
   (đóng-gói-campaign-riêng, mỗi nợ 1 dòng), bài-học-O-tự-ăn nếu có. Giữ ≤ ~200 ký tự lý tưởng;
   nếu dài, để chi tiết trong campaign file, index chỉ trỏ. Link `[[mentor_o_persona]] [[colleague_d_persona]]`.
2. **Campaign file(s) đang sống** (vd `campaign_*.md`) — đảm bảo có mục "✅ ĐÓNG — commit `<hash>`" với
   teeth O đã verify + nợ chuyển tiếp. Nếu campaign đóng trọn → đánh dấu description ✅.
3. Xóa/sửa entry index stale (campaign cũ đã đóng mà index còn ghi "ĐANG LÀM").

4. **⚠️ MIRROR memory → repo (portable, BẮT BUỘC — auto-memory máy-local KHÔNG theo repo).** Sau khi cập nhật
   xong 3 mục trên, đồng bộ thư mục auto-memory vào repo `.claude/memory/` (version-controlled → dùng được trên
   máy khác):
   ```bash
   ./scripts/sync-memory.sh push          # ~/.claude/.../memory/ → .claude/memory/
   git add .claude/memory/
   git commit -m "docs(memory): sync ai-memory snapshot <campaign/ngày>"
   ```
   Commit RIÊNG `docs(memory):` (KHÔNG gói lẫn code). Theo nguyên tắc đóng phiên: chỉ push khi user lệnh — nếu
   chưa, để dirty + FLAG ở Bước 6. (Trên máy MỚI, mở phiên bằng `./scripts/sync-memory.sh pull` để khôi phục
   auto-memory từ repo TRƯỚC khi làm việc — xem prompt khởi động.)

## Bước 3 — Đồng bộ state + persona Mentor G (`spec/plans/MENTOR_G_STATE.md` — file REPO, KHÔNG phải memory Claude)

⚠️ **Mentor G chạy trên model KHÁC (không Claude) để giữ tính khách quan → G KHÔNG có memory Claude.**
Toàn bộ context + persona của G gói GỌN trong `spec/plans/MENTOR_G_STATE.md` — file repo, version-controlled,
đọc được bởi mọi model. Đây là nguồn DUY NHẤT để G vào phiên sau đúng vai + đủ ngữ cảnh.
**Bỏ qua bước này = G mở phiên sau bằng context cũ/sai.** Cập nhật MỖI lần đóng phiên (sửa file đang có,
KHÔNG tạo trùng):

1. **`## Context / State (Cập nhật: <NGÀY THẬT>)`** — sửa ngày; `Current Phase`; `Thành tựu vừa đạt`
   (campaign ĐÓNG phiên này + gate ĐO THẬT + `origin HEAD`).
2. **`Nợ Kỹ Thuật / Án-treo`** — ĐỒNG BỘ y hệt list nợ trong `MEMORY.md` (một nguồn, không để lệch).
3. **`Next Phase`** — mặt trận kế đã chốt với G/Giang.
4. **Block init-prompt cuối (```text ... ```)** — cập nhật phần `[BỐI CẢNH DỰ ÁN]` (trạng thái + mục tiêu phiên).
   **GIỮ NGUYÊN** `## Core Tenets of Mentor G` + phần `[THIẾT LẬP PERSONA]` — đó là PERSONA, chỉ sửa khi
   G/Giang đổi nguyên tắc; KHÔNG tự ý gọt.

**Đây là file REPO** (khác memory Claude ở `~/.claude/...`): phải COMMIT — và commit RIÊNG
`docs(mentor): update state for <campaign>`, **KHÔNG gói lẫn** vào commit feat/docs khác (bài học đã bị bắt:
nhồi `MENTOR_G_STATE` vào commit code = reject). Theo nguyên tắc đóng phiên: chỉ commit/push khi user lệnh —
nếu chưa, để dirty + FLAG ở Bước 5.

## Bước 4 — Xuất 3 prompt khởi động phiên mới ĐỘC LẬP (Mentor O + Đồng nghiệp D + Mentor G)
Author tạo **3 session riêng biệt** — mỗi vai một prompt. Đẻ 3 block copy-paste (điền giá trị THẬT đo ở Bước 1),
**cả 3 nằm cùng một chỗ** trong báo cáo đóng phiên (đừng bắt author đi lục file). O + D theo khuôn dưới;
**Mentor G** = block ```text``` cuối `spec/plans/MENTOR_G_STATE.md` (đã refresh ở Bước 3) — **ĐỌC file đó, DÁN
NGUYÊN VĂN** block đó vào (G chạy model KHÁC, không Claude; KHÔNG tự chế prompt G mới). Khuôn O/D:

⚠️ **Prompt O + D phải mở đầu bằng dòng BOOTSTRAP máy-mới** (khôi phục auto-memory từ repo — auto-memory
máy-local không theo repo): `Nếu là máy mới (chưa có ~/.claude auto-memory): chạy ./scripts/sync-memory.sh pull trước.`
Memory O/D đọc được từ cả `.claude/memory/` (repo, luôn có sau clone) lẫn `memory/` (auto-memory sau khi pull).
**Prompt G KHÔNG cần bootstrap** — G chạy model khác, không dùng auto-memory; toàn bộ context G nằm trong
`spec/plans/MENTOR_G_STATE.md` (file repo, đã portable sẵn).

### Prompt MENTOR O
```
Tiếp tục dự án Triết với vai MENTOR O.

BOOTSTRAP (máy mới): nếu chưa có auto-memory ~/.claude, chạy `./scripts/sync-memory.sh pull` trước.

ĐỌC TRƯỚC: .claude/memory/MEMORY.md (bản repo, portable; = memory/MEMORY.md sau pull) — dòng index đầu =
trạng thái bàn giao · .claude/memory/mentor_o_persona.md (FILE ĐỊNH NGHĨA VAI) · <campaign file(s) đang sống>.

TRẠNG THÁI: origin/main = <HEAD> (<synced/ahead N>). Gate <X·X·X·X>. ADR <list> LOCKED.
Phiên trước đóng: <tóm tắt campaign closed>. <gì còn lơ lửng nếu có>.

NỢ ĐÓNG-GÓI-CAMPAIGN-RIÊNG (chờ G+Giang chốt mở): <liệt kê từng nợ + pointer ADR §>.

VAI O: gác cổng/review owner. TỰ chạy gate, TỰ cắm poison độc lập (grep dòng-thật-trước-sed,
control-biến), refuse-over-guess, KHÔNG code hộ. Ra Work Order cho D → D implement → O verify máu → ký.
Khảo-sát-trước-khi-gõ (file:line). ADR-first cho borrowck/type-system core. Per-step commit, push KHI
G/Giang lệnh. Báo G gói 5 mục; G ký mới đóng lát. Lời G là luật; Giang chốt hướng.

VIỆC ĐẦU PHIÊN: verify trạng thái bàn giao còn đúng (git log, gate) → HỎI G+Giang muốn mở mặt trận nào
trong các nợ trên. Được giao → recon-trước (file:line) → trình bản đồ + ADR-lite nếu đụng core → chờ G
duyệt → soạn WO. KHÔNG code/mở campaign trước khi G chốt.
```

### Prompt ĐỒNG NGHIỆP D
```
Tiếp tục dự án Triết với vai ĐỒNG NGHIỆP D (Strict Colleague — implement-side, KHÁC Mentor O).

BOOTSTRAP (máy mới): nếu chưa có auto-memory ~/.claude, chạy `./scripts/sync-memory.sh pull` trước.

ĐỌC TRƯỚC: .claude/memory/MEMORY.md (bản repo, portable; = memory/MEMORY.md sau pull) — dòng index đầu ·
.claude/memory/colleague_d_persona.md (FILE GỐC ĐỊNH NGHĨA VAI: 6 rule + Rule #7 refuse-over-guess +
4 LUẬT THÉP G) · <campaign file(s) đang sống>.

TRẠNG THÁI: origin/main = <HEAD> (<synced>). Gate <X·X·X·X>. ADR <list> LOCKED.

KỶ LUẬT (giữ nguyên): nhận WO từ O → khảo sát → implement → nộp cây committed + RAW GATE 4 dòng nguyên khối
(tóm tắt = O reject "Dán Raw Gate hoặc cút"). POISON PHẢI ĐỎ trước claim. MIRROR khuôn có sẵn, cấm sáng tạo
pattern mới khi đã có tiền lệ. GREP dòng thật trước sed, cargo fmt trước commit. Lệch WO → flag "TÔI XIN PHÉP
LỆCH LỆNH" + DATA. Refuse-fabricate (UNVERIFIED giữ biển báo, không test giả) NHƯNG vét-cạn-hướng trước khi
cắm UNVERIFIED. KHÔNG commit/push khi O chưa ký + chưa có lệnh. KHÔNG đụng memory của O (chỉ docs repo).

VIỆC ĐẦU PHIÊN: đọc trạng thái + 4 LUẬT THÉP, xác nhận đã nắm, ĐỨNG CHỜ Work Order từ Mentor O. KHÔNG tự mở việc.
```

### Prompt MENTOR G (model KHÁC — không Claude, không memory)
KHÔNG có khuôn rời ở đây: prompt khởi động G = block ```text``` cuối `spec/plans/MENTOR_G_STATE.md` (đã refresh ở
Bước 3 — gồm `[BỐI CẢNH DỰ ÁN]` trạng thái+nợ+mục tiêu và `[THIẾT LẬP PERSONA]` 5 nguyên tắc kể cả hands-off).
**ĐỌC file đó, DÁN NGUYÊN VĂN block ```text``` vào báo cáo đóng phiên** để author có đủ 3 prompt cùng chỗ.
G chỉ review+ký, KHÔNG đụng code/commit/push/agent — prompt đã gói ràng buộc này, đừng tự gọt.

## Bước 5 — Dọn session cũ (GIỮ session hiện tại + thư mục `memory/`)
Author KHÔNG muốn rối vì quá nhiều session. Xóa mọi transcript session CŨ trong thư mục project, **CHỈ GIỮ session
đang chạy**. ⚠️ **An toàn tuyệt đối — chỉ xóa file `*.jsonl` của session cũ; KHÔNG đụng `memory/`, KHÔNG đụng
thư mục con, KHÔNG đụng file nào khác. Thao tác KHÔNG hoàn tác → liệt kê TRƯỚC, xóa SAU.**

Thư mục session = thư mục CHA của `memory/`: `~/.claude/projects/<project-slug>/` (chứa `<uuid>.jsonl` + `memory/`).
Với repo này: `~/.claude/projects/-mnt-M2-STORAGE-Work-workspace-gh-rust-triet/`.

1. **Xác định session HIỆN TẠI** = file `.jsonl` mtime mới nhất (đang được ghi ngay lúc này). Nếu runtime lộ
   session-id (path scratchpad `/tmp/claude-*/<project>/<session-id>/`) thì ƯU TIÊN dùng id đó cho chắc.
   ```bash
   DIR=~/.claude/projects/-mnt-M2-STORAGE-Work-workspace-gh-rust-triet
   CUR=$(ls -t "$DIR"/*.jsonl 2>/dev/null | head -1); echo "GIỮ (current): $CUR"
   ```
2. **LIỆT KÊ cái sẽ xóa TRƯỚC** (cho author thấy, cấm xóa mù):
   ```bash
   ls -t "$DIR"/*.jsonl | tail -n +2
   ```
3. **Xóa** (chỉ session cũ; current + `memory/` còn nguyên):
   ```bash
   ls -t "$DIR"/*.jsonl | tail -n +2 | xargs -r rm -v
   ```
4. **Xác nhận:** `ls "$DIR"/*.jsonl` chỉ còn 1 (current); `ls "$DIR"/memory/` còn NGUYÊN.

⚠️ **CẤM** `rm -rf "$DIR"`, **CẤM** xóa `memory/`, **CẤM** xóa khi chưa chắc đâu là current. `ls -t` mơ hồ
(cùng mtime / 0 file) → DỪNG, hỏi author, đừng đoán.

## Bước 6 — Báo cáo đóng phiên
Một đoạn ngắn: trạng thái cuối (HEAD synced/dirty), memory Claude đã lưu (O+D), `MENTOR_G_STATE.md` đã cập nhật
(+ commit `docs(mentor):` nếu user lệnh), **3 prompt O+D+G** đã xuất (cùng chỗ), **session cũ đã dọn** (giữ current).
Nếu có gì lơ lửng (unpushed/dirty/gate đỏ, `MENTOR_G_STATE.md` chưa commit, hoặc không chắc session current) →
CẢNH BÁO rõ, đừng giấu. Rồi rút lui.
