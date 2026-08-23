---
name: close-session
description: Triết session-closing procedure — verify git is clean and synced, update the handover memory (Mentor O), refresh Mentor G's state and persona (spec/plans/MENTOR_G_STATE.md — a repo file for the non-Claude model), emit 2 INDEPENDENT startup prompts (O + G; D is O's subagent and needs no prompt), and clean up old sessions (keeping the current one). Use when I say "close the session".
trigger: /close-session
argument-hint: "(no args) — /close-session"
---

# /close-session — Triết session-closing procedure

Close the working session cleanly: lock in the state, save the handover memory, and emit
copy-paste prompts so the next session (Mentor O + Mentor G) resumes in the right role, with the
right discipline, without losing the thread.

⚠️ **Since 2026-08-02, D is a SUBAGENT of O** (`.claude/agents/colleague-d.md`) — O spawns D inside
the session, and Giang no longer opens a D session. ⇒ This procedure **emits no D prompt and hands
over no separate state for D**. D starts from the Work Order O gives it at spawn time; D's persona
(`.claude/memory/colleague_d_persona.md`) stays as reference material and needs no per-session update.

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

## Step 3 — Update Mentor G's state and persona (`spec/plans/MENTOR_G_STATE.md` — a REPO file, NOT Claude memory)

⚠️ **Mentor G runs on a DIFFERENT model (not Claude) to preserve objectivity → G has no Claude
memory.** All of G's context and persona is packed into `spec/plans/MENTOR_G_STATE.md` — a
version-controlled repo file readable by any model. It is the ONLY source that lets G enter the next
session in the right role with enough context.
**Skipping this step = G opens the next session with stale or wrong context.** Update it EVERY time
the session closes (edit the existing file, do NOT create duplicates):

1. **`## Context / State (Updated: <REAL DATE>)`** — fix the date; `Current Phase`; `Recent
   achievements` (campaigns CLOSED this session + the MEASURED gate + `origin HEAD`).
2. **`Technical debt / suspended items`** — keep it IDENTICAL to the debt list in `MEMORY.md` (one
   source, never allowed to drift).
3. **`Next Phase`** — the next front agreed with G and Giang.
4. **The final init-prompt block (```text ... ```)** — update the `[PROJECT CONTEXT]` part (state +
   objective for the session). **LEAVE UNCHANGED** `## Core Tenets of Mentor G` and the
   `[PERSONA SETUP]` part — that is the PERSONA, changed only when G or Giang change the principles;
   never trim it on your own initiative.

**This is a REPO file** (unlike the Claude memory at `~/.claude/...`): it must be COMMITTED — and in
its OWN commit, `docs(mentor): update state for <campaign>`, **never bundled** into another feat/docs
commit (a lesson already learned the hard way: stuffing `MENTOR_G_STATE` into a code commit gets
rejected). Per the closing principle: commit/push only when the user orders it — otherwise leave it
dirty and FLAG it in Step 5.

## Step 4 — Emit 2 INDEPENDENT startup prompts (Mentor O + Mentor G)
The author opens **2 separate sessions** — one prompt per role. **There is no D prompt** (D is a
subagent that O spawns inside the session). Produce 2 copy-paste blocks (filled with the REAL values
measured in Step 1), **both in the same place** in the closing report (do not make the author hunt
through files). O follows the template below; **Mentor G** = the final ```text``` block of
`spec/plans/MENTOR_G_STATE.md` (refreshed in Step 3) — **READ that file and PASTE IT VERBATIM** (G
runs on a DIFFERENT model, not Claude; do NOT invent a new G prompt). O's template:

⚠️ **The O prompt must open with the new-machine BOOTSTRAP line** (restoring auto-memory from the
repo — machine-local auto-memory does not travel with the repo):
`If this is a new machine (no ~/.claude auto-memory yet): run ./scripts/sync-memory.sh pull first.`
O's memory is readable from both `.claude/memory/` (repo, always present after a clone) and `memory/`
(auto-memory, after a pull).
**The G prompt needs no bootstrap** — G runs on a different model and does not use auto-memory; all
of G's context lives in `spec/plans/MENTOR_G_STATE.md` (a repo file, already portable).

### MENTOR O prompt
```
Continue the Triết project as MENTOR O.

BOOTSTRAP (new machine): if there is no ~/.claude auto-memory yet, run `./scripts/sync-memory.sh pull` first.

READ FIRST: .claude/memory/MEMORY.md (the repo copy, portable; = memory/MEMORY.md after a pull) — the
first index line is the handover state · .claude/memory/mentor_o_persona.md (THE FILE THAT DEFINES THE
ROLE) · <the live campaign file(s)>.

STATE: origin/main = <HEAD> (<synced/ahead N>). Gate <X·X·X·X>. ADRs <list> LOCKED.
Closed last session: <summary of the campaigns closed>. <anything still dangling>.

DEBTS PACKAGED AS THEIR OWN CAMPAIGNS (awaiting G + Giang to open them): <each debt + the ADR § pointer>.

ROLE O: gatekeeper / review owner. Run the gate yourself, plant poison yourself (grep the real line
before sed, use control variables), refuse over guess, never write the code. Issue the Work Order →
**SPAWN D YOURSELF** (Agent tool, subagent_type "colleague-d" — D has been O's subagent since
2026-08-02; Giang no longer relays WOs. For the fix loop, message that same D via SendMessage instead
of spawning a new one) → verify with blood → sign. D's report is visible only to O ⇒ O must RELAY the
raw gate, the commit hash, and the poison results to G and Giang; summarizing is forbidden. Recon
before typing (file:line). ADR-first for the borrowck and type-system core. Per-step commits; push
when G or Giang order it. Report to G in the 5-section package; a slice closes only once G signs.
G's word is law; Giang decides direction.

Every document is written in English (`*.vi.md` is the only exception); speak Vietnamese with the author.

FIRST TASK OF THE SESSION: verify the handover state still holds (git log, gate) → ASK G and Giang
which front to open among the debts above. Once assigned → recon first (file:line) → present the map
plus an ADR-lite if it touches the core → wait for G's approval → then write the WO. Do not code or
open a campaign before G agrees.
```

### (There is no COLLEAGUE D prompt)
D is **O's subagent** as of 2026-08-02 — no separate session, no startup prompt. D's role and
discipline live in `.claude/agents/colleague-d.md` (the agent file O spawns with
`subagent_type: "colleague-d"`) and `.claude/memory/colleague_d_persona.md` (the source persona D
reads at step 0). Closing a session **touches nothing about D**.

### MENTOR G prompt (a DIFFERENT model — not Claude, no memory)
There is no separate template here: G's startup prompt IS the final ```text``` block of
`spec/plans/MENTOR_G_STATE.md` (refreshed in Step 3 — containing `[PROJECT CONTEXT]` with the state,
debts, and objective, plus `[PERSONA SETUP]` with the 5 principles including hands-off).
**READ that file and PASTE THE ```text``` BLOCK VERBATIM into the closing report** so the author has
both prompts in one place. G only reviews and signs — G never touches code, commits, pushes, or
agents — and the prompt already encodes those constraints, so do not trim it.

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
One short paragraph: the final state (HEAD synced/dirty), the Claude memory saved (O),
`MENTOR_G_STATE.md` updated (+ the `docs(mentor):` commit if the user ordered it), the **2 prompts
O + G** emitted (in one place), and **old sessions cleaned up** (current kept). If anything is left
dangling (unpushed/dirty/red gate, `MENTOR_G_STATE.md` uncommitted, or uncertainty about which
session is current) → WARN clearly, do not hide it. Then withdraw.
