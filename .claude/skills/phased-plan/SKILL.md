---
name: phased-plan
description: Run a large feature or refactor as a gated, phased project. Starts with an investigation phase (read SPEC.md + API.md, then Explore agents over the code via the code-review MCP) — NOT standard plan mode — then builds a custom gated phased plan, creates a branch + draft PR for CI tracking (if a git remote exists), and executes each phase autonomously (code → build → fmt/clippy → test → commit → push) until done. Ships only via the release skill.
---

# Phased plan

For any large feature or non-trivial refactor — and for picking up an item from
root `SPEC.md` §"Build phasing" (e.g. "Surface + IRAP-classic reader + ops").
**Demand this skill** when the user kicks off such work. Do **not** use standard
plan mode (`EnterPlanMode` / `ExitPlanMode`) — this skill builds its own gated
phased plan instead of the harness's generic plan.

## Sources of truth (read both before planning)
- **`SPEC.md`** (repo root) — the design constitution + architecture: layered
  one-way deps (`foundation → io → core → analysis → manager → py`), the
  manager substrate, conventions (NaN = undefined, `f64` default, immutable
  ops), and the build phasing. Your plan must obey the constitution.
- **`API.md`** (repo root) — the **locked public API contract**. Implement
  *toward* these exact signatures; **a phase never changes a signature** —
  that needs the sign-off called out in `API.md`'s header. If the work genuinely
  requires an API change, stop and get sign-off as a planning step, don't drift.

## Working dir: `dev-docs/` (gitignored)
All plans, scratch, and intermediates live under **`dev-docs/`** — gitignored
local working state. **The canonical layout + lifecycle is `dev-docs/README.md`
— read it; it's the source of truth, this is just the phased-plan-relevant
subset:**
- This project's plan → **`dev-docs/plans/<slug>.md`** (durable).
- Design choices/trade-offs you weigh → **`dev-docs/designs/`** (durable).
- Open threads → a lean one-line backlink in **`dev-docs/todos.md`** (detail in
  the linked durable doc, never inline).
- **Offload large output to `dev-docs/temp/` and report the path** (>1-day
  purge) instead of printing it — stays under the response token gate.
- Benchmarks: harnesses → **`bench/scripts/`**, regression rows →
  **`bench/results/results.csv`**, heavy generated grids/cubes/dumps →
  **`bench/out/`** (>14-day purge; never write artifacts next to the script).

## Phase −1 — Start fresh (recommend cleanup first)
Before investigating, **recommend the user run the `dev-docs-cleanup` skill**
so we start from a tidy `dev-docs/` and a current `todos.md`. Relevant
carried-over todos can then be folded into this plan — **only with the user's
go-ahead.** If they decline, proceed without it.

## Phase 0 — Investigation (get a feel for scale before committing to a plan)
- **Do not enter plan mode.** Investigate first, plan second.
- **Read-only until approval.** The main loop makes **zero edits** during
  Phase 0 and Phase 1 — no branch, no PR, no code, no file writes. All
  investigation goes through **read-only `Explore` agents**; nothing touches
  the working tree until the user approves the plan in Phase 1.
- Start from the spec: which `SPEC.md` build phase / `API.md` block does this
  work implement? Which layer(s) does it touch, and what's strictly *below* it
  that must already exist (deps point one way only)?
- Kick off **investigator agents** (`Explore`). Once code exists, equip them
  with the **code-review MCP** (`set_root_dir` to this repo → Cypher over the
  code graph + ripgrep) to map structure/callers/couplings. Early in the build,
  when there's little code, investigation is instead **reading the relevant
  dependency APIs** (`las_rs`, `geo`/`geozero`, `ndarray`, `rstar`, `giga-segy`)
  and the SPEC's intended shape. Fan out in parallel — one per subsystem.
  **Scale the count to blast radius:** 1–2 for a medium change, more only for a
  genuinely large one; don't over-spend on investigation.
- If this is a bug-driven fix: reproduce and confirm the **root cause with
  evidence** before planning the fix (CLAUDE.md "Working style").
- **Decide the safety net in Phase 0, and confirm it catches *this* class of
  change.** petekIO's net is **golden / analytic tests** (SPEC §"Build
  phasing"): round-trip a known IRAP file; bilinear resample vs a hand calc;
  `area_below` vs an analytic value; a worked deviation survey + the
  vertical-well degenerate case for minimum-curvature. For a behaviour-
  preserving refactor, first capture the *actual* outputs of the paths you're
  about to move with a throwaway scratch script (`dev-docs/temp/`) — don't
  trust your mental model; that's where latent bugs hide.
- Synthesize findings into a scale read: small/medium/large, risk hot spots,
  what could invalidate a naive plan.

## Phase 1 — Build the gated phased plan
- Write the plan to **`dev-docs/plans/<slug>.md`** (the durable copy; the PR
  description in Phase 2 mirrors it as a checklist).
- Break the work into numbered phases. Each phase must be independently
  **buildable, testable, committable** (bisectable), and must **respect the
  layered deps** — land a lower layer before the layer that consumes it.
- For each phase spell out: the change, the `API.md` signatures it realizes,
  the golden/unit tests that prove it, the green gate.
- No phase touches `Cargo.toml` version / CHANGELOG promotion — shipping is the
  `release` skill's job.
- Present the plan, then **invite revision: ask the user to revise or approve,
  and loop on their feedback until they approve.**
- **Hard stop — wait for an explicit go-ahead.** Do not create the branch, open
  the PR, or write any code until the user says proceed (e.g. "proceed", "go
  ahead", "approved", "ship it"). A simple proceed is enough. Until then, stay
  read-only.
- Once approved, **do not pause between phases.**

## Phase 2 — Branch + draft PR (the CI tracking handle)
- **If the repo isn't a git repo yet** (greenfield): with the user's OK,
  `git init`, add `.gitignore`-respecting initial commit, before branching.
- Create a feature branch: `feat/<slug>` or `refactor/<slug>` (never work the
  project directly on `main`).
- **If a GitHub remote exists:** push the branch and **open a draft PR against
  `main`** so CI runs per push while nothing publishes; put the phased plan into
  the **PR description as a checklist** (one box per phase) — plan + progress +
  CI status in one place.
- **If there's no remote yet:** skip the PR/CI handle — just commit one phase at
  a time locally (still on the feature branch). Note in the report that CI
  tracking is deferred until a remote is added.

## Phase 3 — Execute each phase (the autonomous loop)
For every phase, in order:
1. Implement the phase's code + its tests, toward the locked `API.md`
   signatures, in the correct layer.
2. **Local green gate before committing:**
   - `cargo build` (add `--all-features` if the phase touches the `py` feature).
   - `cargo fmt --all -- --check` **and** `cargo clippy --all-targets --all-features -- -D warnings`.
   - `cargo test` (the Rust unit + golden tests). For a `py`-feature phase:
     `maturin develop` then `pytest`.
   - **Tooling discipline (transfers directly — don't relearn it the hard way):**
     never read a gate's status through a `tail`/`head` pipe (a pipeline's exit
     code is the *last* command's — run the gate bare or use `set -o pipefail`).
     After `maturin develop`, confirm it actually printed `Installed` — a build
     error upstream leaves the *old* `.so` in place and the next `pytest`
     silently tests stale code. After any **Rust behaviour** change run
     `cargo test`, not just `pytest` — the golden/engine assertions live on the
     Rust side.
3. Update `CHANGELOG.md` `[Unreleased]` for user-visible changes (not the
   version block).
4. **Commit** the phase (`feat(...)` / `refactor(...)` / `fix(...)`), one
   commit per phase.
5. **If a remote exists, push** → CI runs on the PR for that phase; tick the
   phase's checkbox. Otherwise the local commit is the phase boundary.
6. **Retire any `todos.md` action this phase completed.** If the phase fully
   closes a backlog thread, do the same soft-delete tidy `dev-docs-cleanup`
   performs — at phase-commit time, not as a separate pass:
   - **Fully done** → remove the backlink line from `todos.md` and move its
     supporting `plans/<doc>.md` to `dev-docs/bin/` (7-day grace).
   - **Partially done** → leave the doc; trim the entry to only what's left.
   - **Shared doc** (a `plans/` file backing several todos) → remove only the
     closed entry; move the doc to `bin/` *only* once no live backlink points at
     it.
   `dev-docs/` is gitignored, so this is local bookkeeping alongside the commit.
   Note each retirement in the report-out.
7. Continue into the next phase. If a phase's CI comes back red, fold the fix
   into the loop before merging — don't leave the PR red.

Stop mid-plan only for a genuine blocker (unfixable test, architectural surprise
invalidating a later phase, or a forced `API.md` change needing sign-off).
Surface it; don't push through.

**Bugs that surface mid-plan — fix them as they surface** (CLAUDE.md "no bugs
left behind"). When executing a phase reveals a defect:
- **In scope** (same file/layer you're touching): reproduce + confirm the root
  cause, then fix it as its **own bisectable phase** (insert `Phase Nb` with its
  own golden/unit test + commit + CHANGELOG entry if user-visible). Don't fold a
  behaviour change into a mechanical-refactor commit — keep bisection clean.
- **Out of scope** (different layer): don't silently leave it. Reproduce,
  confirm, file it to `dev-docs/plans/consider-for-future.md` with a `todos.md`
  backlink, and add a cheap golden/regression test if one fits.
Record every surfaced bug in the **report-out** below.

## Phase 4 — Perf gate
Before declaring done, run new + existing benchmarks per CLAUDE.md "Performance
protocol" (`cargo bench` / criterion, **release build**, min over median,
NaN-aware array loops are the usual hot spots). Record numbers to
`dev-docs/bench/results/results.csv`; heavy built grids → `bench/out/`. Fix
regressions now, not in a follow-up.

## Report out (when the plan completes, before Ship)
Keep it under the 400-token rule; link the plan doc for detail:
- **Phases** done (one line each) + the PR link / final commit shas.
- **Bugs surfaced** during execution and each one's disposition: *fixed in
  Phase Nb* or *filed to backlog*. Mandatory even if empty ("no bugs surfaced").
- **Perf gate** result (pre/post min + verdict: flat / regression / improved).
- **`todos.md` changes**: actions *retired* and items *added*.
- **Plan deviations** (inserted phases, re-scopes, any `API.md` sign-off) + why.

## Phase 5 — Ship (only on request)
When the user asks to ship, run the **`release`** skill: gate, bump the crate
version, promote the CHANGELOG, commit, and — with explicit approval — push
`main` and verify the publish (crates.io; PyPI wheel once the `py` feature
ships). This skill never bumps, never pushes `main`.

## Notes
- Keep responses under 400 tokens; write long diffs/logs to `dev-docs/temp/` and
  report the path.
- Branch pushes during the loop are routine (no publish). Only the **`main`**
  push at `release` time is the approval-gated one.
- New `io/` reader? Land its golden round-trip test in the same phase — a reader
  without a fixture round-trip isn't trusted (the petekIO analogue of "passes
  not in the corpus aren't trusted").
