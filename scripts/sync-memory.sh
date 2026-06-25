#!/usr/bin/env bash
# sync-memory.sh — đồng bộ AI-memory (Mentor O / Đồng nghiệp D persona + campaign +
# index) giữa repo và thư mục auto-memory máy-local của Claude Code.
#
# Claude Code auto-load + auto-write memory ở ~/.claude/projects/<slug>/memory/ —
# thư mục NÀY là máy-local, KHÔNG theo repo → sang máy khác là mất. Script này
# mirror nó vào repo (.claude/memory/) để version-control + portable.
#
#   ./scripts/sync-memory.sh push   # auto-memory → repo   (gọi khi ĐÓNG phiên)
#   ./scripts/sync-memory.sh pull   # repo → auto-memory   (gọi khi MỞ phiên trên máy mới)
#
# <slug> = đường dẫn tuyệt đối của repo với mọi '/' đổi thành '-' (quy ước Claude Code).
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
REPO_MEM="$REPO_ROOT/.claude/memory"
SLUG="$(printf '%s' "$REPO_ROOT" | sed 's#/#-#g')"
AUTO_MEM="$HOME/.claude/projects/$SLUG/memory"

MODE="${1:-}"
case "$MODE" in
  push)
    if [ ! -d "$AUTO_MEM" ]; then
      echo "ERROR: auto-memory không tồn tại: $AUTO_MEM" >&2
      echo "  (chạy trên máy chưa có phiên Claude nào? dùng 'pull' trước.)" >&2
      exit 1
    fi
    mkdir -p "$REPO_MEM"
    # Xóa file .md cũ trong repo-mirror rồi copy lại (để file bị xóa ở auto-memory
    # cũng biến mất ở mirror — tránh memory zombie). KHÔNG đụng file non-.md.
    rm -f "$REPO_MEM"/*.md
    cp "$AUTO_MEM"/*.md "$REPO_MEM"/
    echo "PUSH: $AUTO_MEM → $REPO_MEM ($(ls "$REPO_MEM"/*.md | wc -l) file)"
    ;;
  pull)
    if [ ! -d "$REPO_MEM" ]; then
      echo "ERROR: repo-mirror không tồn tại: $REPO_MEM" >&2
      exit 1
    fi
    mkdir -p "$AUTO_MEM"
    cp "$REPO_MEM"/*.md "$AUTO_MEM"/
    echo "PULL: $REPO_MEM → $AUTO_MEM ($(ls "$AUTO_MEM"/*.md | wc -l) file)"
    echo "  (slug: $SLUG)"
    ;;
  *)
    echo "usage: $0 {push|pull}" >&2
    echo "  push: auto-memory (~/.claude/...) → repo (.claude/memory/)  [khi ĐÓNG phiên]" >&2
    echo "  pull: repo (.claude/memory/) → auto-memory (~/.claude/...)  [khi MỞ phiên máy mới]" >&2
    exit 2
    ;;
esac
