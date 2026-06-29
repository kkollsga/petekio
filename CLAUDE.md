# petekIO — Claude Code Conventions

petekIO is a standalone Rust subsurface data-model + IO library (with optional
PyO3 bindings). The two committed sources of truth are **`SPEC.md`** (design
constitution + architecture) and **`API.md`** (the *locked* public API
contract). Read both before non-trivial work. The dev-docs + inbox + skills
system below is local working state — see `dev-docs/README.md` and
`inbox/README.md` for the canonical maps.

## Data confidentiality — NEVER leak `/Volumes/EksternalHome/Data`

Anything under **`/Volumes/EksternalHome/Data`** (the external data folder — real
datasets like the Cerisa/Duva modelling project) is **confidential and must never
enter any repo**. Concretely:

- **Never commit** a file copied/derived from there, and never paste its
  **contents** (coordinates, values, well/field names, survey rows, log samples)
  into committed code, fixtures, tests, examples, docs, commit messages, or
  `CHANGELOG.md`.
- **Test/example data stays in the data folder, not the repo.** Tests/notebooks
  resolve it via `PETEKIO_TEST_DATA` (see `tests/common/mod.rs`) and **skip when
  absent** — they never carry a committed copy. Golden fixtures committed to the
  repo must be **synthetic** (hand-authored to format spec), not real values.
- The published crate ships **no** test/example data (`Cargo.toml` `exclude`s
  `/tests` + `/examples`); keep it that way.
- This binds inbox notes and the planning graph too — reference the dataset by
  *path*, never by copying its content.

## Working style

- **Keep each response under 400 tokens.** For any long output, write it to a
  file (`dev-docs/temp/`, >1-day purge) and tell me the path instead of printing
  it.
- **Reproduce before fixing.** Before changing code, reproduce the issue and
  confirm the exact root cause with evidence. Don't apply a fix until the cause
  is verified.
- **`API.md` is a contract, not a suggestion.** Implement toward its exact
  signatures. Changing a signature needs sign-off (see `API.md`'s header) and an
  edit to `API.md` itself — never let the code silently drift from it.

## Code analysis

- **Use the code-review MCP for code analysis once code exists** — `set_root_dir`
  to this repo, then Cypher over the code graph + ripgrep (`grep`). Prefer it
  over ad-hoc file reads when mapping structure, finding callers, or tracing the
  layered deps. Early in the build (little code), analysis is reading `SPEC.md`/
  `API.md` and the dependency crates' APIs instead.
- **Spin up `Explore` agents** to parallelize broad sweeps and keep the
  conclusions, not the file dumps, in context.

## Architecture — the design constitution (from `SPEC.md`)

- **Strictly layered, one-way deps:** `foundation → algorithms → io → core →
  analysis → manager → py`. A layer imports only from below — never sideways,
  never up. A change that needs to point the other way is a design smell;
  rethink it.
- **Algorithms = isolated, QC-able, discipline-grouped kernels** (`SPEC.md` §9).
  High-value numeric routines live in `algorithms/<discipline>/` (e.g. `wells`)
  as pure, type-light functions (primitives + `foundation` types, no domain/IO
  coupling), one home per formula, with analytic QC tests. Domain types call in;
  don't inline a formula. Keeps each kernel cheap to QC and cheap to lift into
  the external **petekAlgorithms** library.
- **Manager substrate, no per-item loops.** Load once into a `GeoData` project;
  operations broadcast across the collection (views = read-only filtered
  subsets).
- **Domain objects carry their operations** (arithmetic, filters, interpolation,
  stats) as methods/traits — fluent, chainable, immutable (ops return *new*
  objects; mutation is explicit `set_*`).
- **Open/closed:** extend by adding readers/operations/artifacts, not by editing
  existing types.
- **Compartmentalized:** one module/topic, one type/responsibility. Soft limits
  — module ≲600 lines, type ≲300, method ≲50.
- **Compose, don't reinvent:** `las_rs`, `geo`/`geozero`, `ndarray`, `rstar`,
  `giga-segy`. Wrap deps behind the `io/` traits; don't green-room a standard.
- **Conventions:** `f64::NAN` = undefined (arithmetic propagates NaN, stats skip
  it); `f64` default storage; one `GeoError` enum (`thiserror`) +
  `Result<T, GeoError>` everywhere; Rust core + *thin* PyO3 (bindings only
  marshal).

## Build & test

```bash
cargo build --all-features
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features          # Rust unit + golden tests
cargo bench                         # criterion; release build, for perf only
# Python bindings (py feature):
maturin develop                     # --release for any perf measurement
pytest
```

**Tooling discipline (don't relearn it the hard way):**
- **Never read a gate's status through a `tail`/`head` pipe** — a pipeline's
  exit code is the *last* command's, so `cargo clippy | tail && echo ok` prints
  "ok" even when clippy failed. Run the gate bare, or `set -o pipefail`, or
  `cmd; echo "exit=$?"`.
- **After `maturin develop`, confirm it printed `Installed`.** A build error
  upstream leaves the old `.so` in place → the next `pytest` silently tests
  stale code.
- **After any Rust *behaviour* change, run `cargo test`, not just `pytest`.**
  The golden/engine assertions live on the Rust side.

## Code health

Each pass through a file should leave it more compartmentalized than you found
it.

- **No bugs left behind.** Fix a pre-existing bug you encounter in the same
  change, or surface it explicitly (file a `todos.md` item) rather than stepping
  over it. First confirm it's a real defect, not deliberate behaviour — read the
  surrounding code/tests and check it against `SPEC.md`/`API.md`.
- **Golden tests are the safety net.** A correctness path lands with the
  golden/analytic test that proves it: IRAP round-trip, bilinear resample vs a
  hand calc, `area_below` vs an analytic value, a worked deviation survey + the
  vertical-well degenerate case. A new `io/` reader without a round-trip fixture
  isn't trusted.
- **Fixing a bug — scan for the *class*.** The reported symptom is rarely the
  only one; probe with scratch fixtures (`dev-docs/temp/`) before declaring
  scope.
- A measured perf change is only a "fix" if it measurably improves perf.

## Performance protocol

Before any perf-related change: baseline first (write/extend a criterion bench,
record numbers); **release build only**; trust `min` over `median` for sub-ms
benches. Heavy built grids/cubes → `dev-docs/bench/out/`; the regression rows →
`dev-docs/bench/results/results.csv`. See `dev-docs/bench/README.md`.

## Inbox hygiene

**Always use the inbox skills for cross-project communication** — never
hand-read or hand-write inbox files. Incoming → **`read-inbox`** (triage
`unread/`, lift durable info to `dev-docs/` + lean `todos.md` backlinks, route,
archive, purge). Outgoing → **`notify`** (resolve the target under `Koding/`,
compose per the schema, drop into its `inbox/unread/`). The canonical map is
`inbox/README.md`; natural correspondents are `las-rs`/`Sheetio` (IO deps),
`SimulatoRS` (the consumer), and `mcp-servers` (one inbox for the whole
ecosystem — never resolve a name to `mcp-servers/<subdir>/`).

## Planning graph — the cross-library source of truth

The petekSim **planning graph** (`research/graph/research.kgl`, served by the
`contract` MCP) is the single source of truth for the inter-library contracts
(the `ModelInputs` seam, the layered architecture), decisions, and open
questions. Reach for it on anything cross-cutting — read the contract before
changing a shared seam; record blocking issues and choices there, not only in
local docs. Contribute **without cluttering**: runtime types only (`Question` /
`Decision` / `Artifact` / `Task` — never the managed research nodes
Algorithm/AlgorithmSpec/Tool/…; raise a `Question` if one is wrong or missing);
**MERGE on id, never CREATE**; one node per concept; `write_scope` to those
types; stamp `git_sha` + `modified_by='petekio'`. No direct graph access → route
it through the **inbox** to petekSim, who curates it in.

## Commits & releases

Commit format: `type: short description` (`feat`, `fix`, `docs`, `refactor`,
`test`, `chore`). Update `CHANGELOG.md` `[Unreleased]` for user-visible changes;
skip for internal refactors, CI, test-only, formatting.

**Pushing requires explicit, in-the-moment approval.** Default is *don't push*.
Approval is one-shot — it covers exactly that one `git push` and does not carry
to a later commit or branch.

**Exception — invoking the `release` skill IS push authorization for that
release** (the publish-triggering `main` push + its CI fix-and-push loop),
scoped to that one run. Every pre-push safeguard still applies: gate green, the
public surface reconciled with `API.md`, surgical staging that excludes
unrelated WIP, ff-merge clean.

Version source of truth: root `Cargo.toml` `[workspace.package] version` (or the
single crate's `version`) — one bump per push, all workspace members in lockstep.

## The skills (wired into these rules)

- **`phased-plan`** — run any non-trivial, multi-step change as gated phases
  (investigate → plan → branch + draft PR → autonomous build/test/commit loop →
  perf gate → hand to release). Don't use generic plan mode for large work.
- **`add-todo`** — the single authority on `todos.md` entry shape; capture work
  as a lean backlink + a `plans/` detail doc.
- **`dev-docs-cleanup`** — purge the time-boxed dirs + a todos-driven tidy. Run
  before a new phased-plan and at the end of a release.
- **`read-inbox`** / **`notify`** — the receive / send sides of the inbox.
- **`release`** — ship: goal-check, gate, reconcile `API.md`, bump, promote
  CHANGELOG, publish, tidy. Run only when asked.
