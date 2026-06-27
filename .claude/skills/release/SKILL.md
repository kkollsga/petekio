---
name: release
description: Cut a petekIO release — goal-check against the phased-plan, gate (build + fmt + clippy + tests + golden suite), confirm the public surface still matches the locked API.md, bump the crate version, promote CHANGELOG, commit, and (with explicit approval) push + publish to crates.io (and PyPI once the py wheel ships), then tidy dev-docs.
---

# Release

## Preconditions
- **Must be a git repo with a clean-ish tree.** If petekIO isn't a git repo
  yet, release is premature — `git init` + an initial commit comes first (the
  `phased-plan` skill handles that). Don't publish from an un-versioned tree.
- Check no release is already staged:
  `git log origin/main..HEAD --oneline | grep -E "^\w+ release\("`.
  If it returns a commit, **keep that version** — fold work into the same
  `[x.y.z]` block (one version bump per push).
- On `main` (or a fold-into-main branch). If there's **unrelated uncommitted
  work**, don't block on it and don't sweep it in: **stage every release file
  explicitly by path** (`git add <file> …`, never `git add -A`/`.`) and leave
  the unrelated changes untouched. Verify with `git status --porcelain` that
  only release files are staged.

## Steps
1. **Goal check — did we achieve what we set out to do?** If this release ships
   a `phased-plan` project, read its plan (`dev-docs/plans/<slug>.md`) and the
   PR checklist (if any), and confirm every planned phase actually shipped. List
   any phase **dropped, deferred, or partially done**, and surface the gaps
   before bumping — finish now or carry to `dev-docs/todos.md`; don't let one
   vanish silently.
2. **Gate — all green before continuing.**
   - `cargo build --all-features`
   - `cargo fmt --all -- --check` **and**
     `cargo clippy --all-targets --all-features -- -D warnings`
   - `cargo test --all-features` (the Rust unit + **golden** suite — IRAP
     round-trip, minimum-curvature, `area_below`, bilinear resample).
   - If the `py` feature ships: `maturin develop --release` (confirm it printed
     `Installed`) then `pytest`.
3. **Confirm the public surface still matches the locked `API.md`.** `API.md` is
   the contract; a release must not silently widen or drift it. Diff the
   implemented public API against `API.md` (use `cargo public-api` if it's set
   up, else read the changed `pub` items). If the surface changed, the change
   needed sign-off (per `API.md`'s header) and `API.md` itself must already be
   updated to match — if it isn't, **stop** and reconcile before publishing.
4. **Bump version — patch by default** (`x.y.Z` → `x.y.Z+1`). If the changes
   warrant a **minor/major** bump (new feature, breaking change, scope
   expansion), STOP and ask one quick clarification before starting. Bump the
   single source of truth: the workspace `[workspace.package] version` in the
   root `Cargo.toml` (or the crate's `version` if it's a single crate). If
   petekIO is a multi-crate workspace, all members inherit that one version —
   keep them in lockstep, never per-crate drift.
5. **Promote CHANGELOG** `[Unreleased]` → `[x.y.z]` (dated).
6. **Commit** as the final phase: `release(x.y.z): ...` (version bump +
   CHANGELOG promotion in one commit).
7. **Push — invoking `/release` is the authorization.** Running this skill
   authorizes the `main` push it produces (the publish-triggering one) — no
   separate in-the-moment "push" prompt. Authorization is scoped to this one
   release run (the `release(x.y.z)` push + its CI fix-and-push loop) and lapses
   once published or the user pivots. All pre-push safeguards still apply: gate
   green, API surface reconciled, surgical staging, ff-merge clean.
   - **ff mechanic — push the branch HEAD straight to `main`, don't
     `checkout main`** (avoids dragging unrelated WIP across): confirm
     fast-forward (`git merge-base --is-ancestor origin/main HEAD`), then
     `git push origin HEAD:<branch>` (update any PR) and `git push origin HEAD:main`.
8. **Publish to crates.io.** `cargo publish` (dry-run first:
   `cargo publish --dry-run`). For a multi-crate workspace, publish **in
   dependency order** — `foundation` → `io` → `core` → `analysis` → `manager` →
   the umbrella crate — waiting for each to index before the next (a crate can't
   publish until its path-deps resolve on crates.io). Once the `py` wheel ships,
   also publish it to **PyPI** (via `maturin publish` / the wheels workflow).
9. **If a CI workflow exists, poll it until green** (`gh` Checks API). petekIO
   may have no CI yet — if so, the local gate (step 2) + a clean
   `cargo publish --dry-run` is the gate. CI fix-and-push loop: if a push fails
   on a shipped-code/infra bug (not a scope change), push `fix(...)`/`ci(...)`
   without re-asking until green; stop after ~3 iterations or any release-shape
   change.
10. **Verify published.** crates.io shows the crate(s) at `x.y.z`
    (`curl -s -H "User-Agent: petekio-release" https://crates.io/api/v1/crates/petekio | jq -r .crate.max_version`
    — the User-Agent header is required or the API returns null and looks like a
    failed publish). If a PyPI wheel was published:
    `curl -s https://pypi.org/pypi/petekio/json | jq -r .info.version`.
11. **Delete the released branch** if you worked one (the merge is done):
    `git branch -f main origin/main` → `git switch main` (zero-diff switch when
    `main == HEAD`, preserves WIP) → `git branch -d <branch>` (refuses if
    unmerged — don't `-D` past that) → `git push origin --delete <branch>`.
    Confirm any PR shows `MERGED`. Never delete `gh-pages` / `dependabot/*`.
12. **Tidy dev-docs — perform directly, no prompt** (the `/release` invocation
    is the authorization). Follow the **`dev-docs-cleanup`** logic: auto-purge
    the time-boxed dirs, then read **only `todos.md`** — archive the now-shipped
    plan to `dev-docs/bin/` and prune its entry, move other completed/stale
    docs to `bin/`, trim entries (read a backlinked doc only to confirm it
    shipped). Carry the step-1 gaps into `todos.md`. Don't read `designs/` or
    root `SPEC.md`/`API.md`, and don't sweep through `plans/`.

## Notes
- Keep responses under 400 tokens; write long diffs/logs to `dev-docs/temp/` and
  report the path.
- Version source of truth: root `Cargo.toml` `[workspace.package] version` (or
  the single crate's `version`). `API.md` is the *API* source of truth — keep
  the two reconciled at every release.
- This skill never bumps or pushes outside an explicit `/release` run; outside
  it, the default (ask before any push) stands — see CLAUDE.md "Commits &
  releases".
