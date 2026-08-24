---
name: close-session
description: Triết session-closing procedure — verify git is clean and synced, update the handover memory (Mentor O and G persona memory), emit the startup prompt for Mentor O (both G and D are subagents spawned by O and need no separate prompt), and clean up old sessions (keeping the current one). Use when I say "close the session".
trigger: /close-session
argument-hint: "(no args) — /close-session"
---

# /close-session — Triết session-closing procedure

Close the working session cleanly: lock in the state, save the handover memory, and emit
copy-paste prompt so the next session (Mentor O) resumes with full context.

⚠️ **Both G and D are SUBAGENTS of O:**
- D lives at `.claude/agents/colleague-d.md` + `.claude/memory/colleague_d_persona.md`
- G lives at `.claude/agents/mentor-g.md` + `.claude/memory/mentor_g_persona.md`
⇒ This procedure **emits no separate G or D prompt**. O spawns both inside the session.

**Principle:** do NOT commit or push anything while closing unless the user explicitly orders it.
Only measure, write memory, and emit prompts. Every number (HEAD, gate) must be MEASURED, never
hand-copied.

## Step 1 — Verify the state (measure, do not trust)
```bash
git status -sb | head -1          # synced? ahead/behind? dirty?
git log --oneline -5
git log --oneline origin/main..HEAD   # any local commits still dangling?
```
- If **dirty** (uncommitted) or **ahead of origin** (unpushed): SAY SO EXPLICITLY in the closing
  report and ask the user whether to commit/push before closing. Do NOT push on your own.
- Gate: if the session just ran the gate, quote the final numbers; if in doubt, re-run
  `bash scripts/gate.sh 2>&1 | tail -25` (or build+clippy+test) and paste the raw output.
- List ADRs newly LOCKED this session: `git log --oneline origin/main | grep -iE "00[0-9][0-9]" | head`.

## Step 2 — Update the handover memory
The LIVE memory files are at `~/.claude/projects/<project-slug>/memory/` (machine-local auto-memory).
Update them (do NOT create duplicates — edit the existing files):
1. **MEMORY.md** — the FIRST index line (## Project context): write ONE new entry reflecting the
   CLOSING state: date, `origin HEAD`, gate, campaigns CLOSED and PUSHED this session, outstanding
   debts (one line each, packaged as their own campaigns), and any lesson O had to swallow. Aim for
   ≤ ~200 characters; if it runs long, keep the detail in the campaign file and let the index point
   at it. Link `[[mentor_o_persona]]`.
2. **The live campaign file(s)** (e.g. `campaign_*.md`) — make sure there is a "✅ CLOSED — commit
   `<hash>`" section with the teeth O verified plus any debt carried forward. If the campaign closed
   completely → mark the description ✅.
3. Delete or fix stale index entries (an old, closed campaign that the index still calls "IN
   PROGRESS").

4. **⚠️ MIRROR memory → repo (portable, MANDATORY — machine-local auto-memory does not travel with
   the repo).** After the three items above, sync the auto-memory directory into the repo at
   `.claude/memory/` (version-controlled → usable on another machine):
   ```bash
   ./scripts/sync-memory.sh push          # ~/.claude/.../memory/ → .claude/memory/
   git add .claude/memory/
   git commit -m "docs(memory): sync ai-memory snapshot <campaign/date>"
   ```
   A SEPARATE `docs(memory):` commit (never bundled with code). Per the closing principle: push only
   when the user orders it — if not, leave it dirty and FLAG it in Step 6. (On a NEW machine, open
   the session with `./scripts/sync-memory.sh pull` to restore auto-memory from the repo BEFORE
   working — see the startup prompt.)

## Step 3 — Update Mentor G's persona memory (`.claude/memory/mentor_g_persona.md` — if G lessons were learned)

⚠️ **Mentor G is an Opus subagent** spawned by O (`.claude/agents/mentor-g.md`).
If any new G lessons were learned or rulings made during the session, record them in
`.claude/memory/mentor_g_persona.md` under `## G's Lessons`, appending the **next `G-Law N`**
in sequence (currently up to G-Law 29 — never reuse a number, never fall back to the retired
circled numerals), and keep `## Session context` fresh.
No separate state file is needed.

## Step 4 — Emit startup prompt for Mentor O

The author opens **1 session as Mentor O**. Both G and D are subagents that O spawns inside the session.
Produce 1 copy-paste block (filled with the REAL values measured in Step 1).

⚠️ **The O prompt must open with the new-machine BOOTSTRAP line** (restoring auto-memory from the
repo — machine-local auto-memory does not travel with the repo):
`If this is a new machine (no ~/.claude auto-memory yet): run ./scripts/sync-memory.sh pull first.`

### MENTOR O prompt
```
Continue the Triết project as MENTOR O.

BOOTSTRAP (new machine): if there is no ~/.claude auto-memory yet, run `./scripts/sync-memory.sh pull` first.

READ FIRST: .claude/memory/MEMORY.md (handover state) · .claude/memory/mentor_o_persona.md (O's rituals and 25 laws) · spec/PROJECT_KNOWLEDGE.md (shared reference).

STATE: origin/main = <HEAD> (<synced/ahead N>). Gate <X·X·X·X>. ADRs <list> LOCKED.
Closed last session: <summary of the campaigns closed>. <anything still dangling>.

DEBTS PACKAGED AS THEIR OWN CAMPAIGNS (awaiting G + Giang to open them): <each debt + the ADR § pointer>.

ROLE O: gatekeeper / review owner / team lead. Run the gate yourself, plant poison yourself, refuse over guess, never write production code.
- To review architecture / get second opinion / sign off → **SPAWN G** (Agent tool, subagent_type "mentor-g", model Opus).
- To implement code/fixtures → **SPAWN D** (Agent tool, subagent_type "colleague-d") with a Work Order. Pick model by task difficulty (Sonnet 5 for complex/novel, Qwen/Gemma for mechanical/closed, or relay to DeepSeek via Author). For the fix loop, message that same D via SendMessage.
- Relay raw gate, commit hash, and poison results to Author.
- Pushing is O's exclusive right, after both O and G sign.

Every document is written in English (`*.vi.md` is the only exception); speak Vietnamese with the author.

FIRST TASK OF THE SESSION: verify the handover state still holds (git log, gate) → ASK G (spawn G) and Giang which front to open among the debts above. Once assigned → recon first (file:line) → present the map plus an ADR-lite if it touches the core → wait for G's approval → then write the WO for D.
```

### (No separate G or D prompts needed)
- **G** is O's subagent (`.claude/agents/mentor-g.md` + `.claude/memory/mentor_g_persona.md`).
- **D** is O's subagent (`.claude/agents/colleague-d.md` + `.claude/memory/colleague_d_persona.md`).

## Step 5 — Clean up old sessions (KEEP the current session and the `memory/` directory)
The author does not want to drown in sessions. Delete every OLD session transcript in the project
directory, **keeping ONLY the running session**. ⚠️ **Absolute safety — delete only the old sessions'
`*.jsonl` files; do NOT touch `memory/`, do NOT touch subdirectories, do NOT touch any other file.
This is irreversible → LIST FIRST, delete AFTER.**

The session directory is the PARENT of `memory/`: `~/.claude/projects/<project-slug>/` (containing
`<uuid>.jsonl` plus `memory/`). For this repo:
`~/.claude/projects/-mnt-M2-STORAGE-Work-workspace-gh-rust-triet/`.

1. **Identify the CURRENT session** = the `.jsonl` file with the newest mtime (being written right
   now). If the runtime exposes the session id (the scratchpad path
   `/tmp/claude-*/<project>/<session-id>/`), PREFER that id to be safe.
   ```bash
   DIR=~/.claude/projects/-mnt-M2-STORAGE-Work-workspace-gh-rust-triet
   CUR=$(ls -t "$DIR"/*.jsonl 2>/dev/null | head -1); echo "KEEP (current): $CUR"
   ```
2. **LIST what will be deleted FIRST** (show the author; never delete blind):
   ```bash
   ls -t "$DIR"/*.jsonl | tail -n +2
   ```
3. **Delete** (old sessions only; the current one and `memory/` remain untouched):
   ```bash
   ls -t "$DIR"/*.jsonl | tail -n +2 | xargs -r rm -v
   ```
4. **Confirm:** `ls "$DIR"/*.jsonl` shows only 1 (the current one); `ls "$DIR"/memory/` is INTACT.

⚠️ **NEVER** `rm -rf "$DIR"`, **NEVER** delete `memory/`, **NEVER** delete while unsure which session
is current. If `ls -t` is ambiguous (identical mtimes / no files) → STOP, ask the author, do not guess.

## Step 6 — Closing report
One short paragraph: the final state (HEAD synced/dirty), the memory saved (O + G),
the **startup prompt for O** emitted, and **old sessions cleaned up** (current kept).
If anything is left dangling (unpushed/dirty/red gate, or uncertainty about which
session is current) → WARN clearly, do not hide it. Then withdraw.
