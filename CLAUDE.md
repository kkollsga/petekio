# petekIO — Claude Code Conventions

petekIO is a standalone Rust subsurface data-model + IO library (with optional
PyO3 bindings). The two committed sources of truth are **`SPEC.md`** (design
constitution + architecture) and **`API.md`** (the *locked* public API
contract). Read both before non-trivial work. petekIO follows the shared **petek
house style** (canonical: `petekSuite/dev-docs/petek-house-style.md`) — the rules
below are this library's slice of it. petekSuite is the control plane: it owns
agent assignment, actionable state, planning-graph writes, GitHub Actions, and
releases. An owning petekIO agent is spawned directly from petekSuite and keeps
its edits inside this repository.

## Data — test against `a local real-dataset folder`, never leak it into the repo

**You are allowed to test against data under `a local real-dataset folder`** (the
external folder of real subsurface datasets) — read it, run it through petekIO,
build local eval harnesses. **But never let it leak into the repo.** No information
derived from its *contents* may land in a repo, a published artifact, an inbox
message, the planning graph, or any committed/exported output. Concretely:

- **Never commit** a file copied/derived from there, and never paste its
  **contents** (coordinates, values, well/field names, survey rows, log samples)
  into committed code, fixtures, tests, examples, docs, commit messages,
  `CHANGELOG.md`, or a note to another repo's inbox. Reference the dataset by
  *path* and by *format*, never by content.
- **Committed tests/examples use SYNTHETIC data** — hand-authored to format spec
  (e.g. `tests/common/mod.rs` builds fixtures in a temp dir; `examples/data/` is
  synthetic). Real-data evaluation happens in a **harness that lives in the data
  folder**, whose output also stays there (print structure/counts, never values).
- The published crate ships **no** test/example data (`Cargo.toml` `exclude`s
  `/tests` + `/examples`); keep it that way.

## Working style

- **Keep each response under 400 tokens.** Summarise long command output and
  report the relevant evidence rather than pasting raw logs.
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
- Broad sweeps and any additional agent assignment are coordinated by
  petekSuite; report conclusions and evidence, not file dumps.

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
  the external **petekTools** library.
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
  `giga-segy`. Wrap deps behind the `io/` traits; don't reimplement a standard from scratch.
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
  change, or surface it explicitly to the petekSuite coordinator for its central
  owner-namespaced action index. First confirm it's a real defect, not deliberate
  behaviour — read the surrounding code/tests and check it against
  `SPEC.md`/`API.md`.
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

## Central coordination

petekSuite owns petekIO's agent lifecycle, actionable todo state, planning-graph
writes, GitHub Actions operations, and releases. Managed work arrives through a
directly spawned owning-library agent; do not create a local skill tree, inbox,
todo index, or MCP control file. Report newly discovered work and cross-library
seam evidence to the coordinator. The suite-level inbox is reserved for outside
projects, not communication between managed libraries.

## Planning graph — the cross-library source of truth

The petekSuite **planning graph** (`petekSuite/research/graph/research.kgl`) is
the single source of truth for inter-library contracts (including the
`ModelInputs` seam), decisions, and open questions. Read the relevant contract
before changing a shared seam. Report evidence, blockers, and proposed graph
updates to the coordinator; petekSuite performs and validates every graph write.

## Commits & releases

Commit format: `type: short description` (`feat`, `fix`, `docs`, `refactor`,
`test`, `chore`). Update `CHANGELOG.md` `[Unreleased]` for user-visible changes;
skip for internal refactors, CI, test-only, formatting.

Commit only when the coordinator's task grants commit authority. Pushing,
GitHub Actions dispatch, versioning, and publishing are exclusively controlled
by petekSuite; a library agent never infers that authority.

Version source of truth: root `Cargo.toml` `[workspace.package] version` (or the
single crate's `version`) — one bump per push, all workspace members in lockstep.

## Execution contract

The central petekSuite `run-library-task` skill scopes and supervises
single-library work; `coordinate` handles cross-library initiatives;
`manage-actions` and `release` own Actions and publishing. The petekIO agent
reproduces the issue, edits toward `SPEC.md`/`API.md`, runs the gates above,
commits only when authorised, and reports files, evidence, SHA, and deviations
to the coordinator.
