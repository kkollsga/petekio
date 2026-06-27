---
name: read-inbox
description: Process inbox/unread/ — read each message, lift durable info into a dev-docs/ detail file, add a lean backlink to dev-docs/todos.md, route actionable items to the right project's inbox, append a Status footer and move the message to inbox/read/, and auto-purge inbox/read/ entries older than 7 days.
---

# read-inbox

Triage `inbox/unread/` (feedback / bug / coordination notes, named
`YYYY-MM-DD-from-<sender>-<topic>.md`). The goal: nothing important stays
trapped in a message — it lands as a durable `dev-docs/` note plus a lean
`todos.md` backlink — and `unread/` ends empty. See CLAUDE.md "Inbox hygiene".

## 1. Auto-purge the read archive (always first)
At skill start, hard-delete `inbox/read/` entries older than 7 days. The
durable record lives in `dev-docs/`, so the week-old archive copy is
redundant:

```bash
find inbox/read -type f -mtime +7 -print -delete
```

Report what was purged (path list, or "nothing aged out").

## 2. Read every unread message
List `inbox/unread/`. Read each file fully. For each, decide: does it carry
durable info, an open action, a decision, or is it a no-action ack?

## 3. Lift durable info → dev-docs/ + todos
Route per the `dev-docs/README.md` layout map:
- **Actionable** content → file it as a todo using the **`add-todo`** skill's
  entry rules (it's the authority on todo shape): classify → the right
  `todos.md` section, scope the detail into a `plans/` doc (reuse one by theme),
  add the lean one-line backlink + the source-message link. A message that
  surfaces *several* actions is add-todo's **batch mode** — decompose, group by
  theme, file each; don't scatter one doc per line.
- **Design choice / trade-off** content → a **`dev-docs/designs/`** reference
  doc instead of a `plans/` doc (no todo — it's reference, not an action).
- **A request to change the locked surface** (a signature in `API.md`, a
  constitution point in `SPEC.md`) → file it as a todo *and* flag it in the
  step-6 summary; the contract needs sign-off, never a silent edit.
- A no-action ack needs no todo — just note it in the move footer (step 5).

Don't restate the todo-entry format here — follow add-todo. This skill owns the
inbox-specific parts: the per-message triage, routing (step 4), the Status
footer, and archival (step 5).

## 4. Route actionable items to the party who can act
If a message carries an **actionable task for another project**, file a note to
their inbox via the **`notify`** skill (it resolves the target under `Koding/`
and writes `YYYY-MM-DD-from-petekio-<topic>.md` into their `inbox/unread/`).
Natural targets: `las-rs` / `Sheetio` (an IO bug we trace to a dependency),
`SimulatoRS` (a downstream-consumer heads-up), or `mcp-servers` (the whole
ecosystem, one inbox). Only route if there's genuinely something for them to
do — don't clutter their `unread/`.

## 5. Append Status footer, move to read/
Append a one-line footer to the message before archiving:
`## Status (petekio, <date>): <lifted to dev-docs/...; todo added | routed to X | no action>`
then move it from `inbox/unread/` to `inbox/read/`. `unread/` must end empty —
every message is either lifted+tracked, routed, or a logged no-action ack.

## 6. Flag to the user
Surface a short summary: **new todos** added (with their detail-file paths),
anything **routed** elsewhere, any **contract-change** request (API/SPEC), and
any item that **needs a user decision**. Recommend keep/drop for anything
ambiguous.

## Output discipline
Keep the response under 400 tokens. If the triage write-up is long, put the
full report in `dev-docs/temp/inbox-triage.md` (ephemeral, 1-day purge) and
report that path; surface only new-todos + decisions inline.

## Relationship to the other skills
Shares `dev-docs/todos.md` with `dev-docs-cleanup` (same lean-index +
detail-file convention) and `phased-plan` (which folds relevant todos into a
new plan on the user's go-ahead). Pass the current date in — `<date>` is the
session date, not a guess.
