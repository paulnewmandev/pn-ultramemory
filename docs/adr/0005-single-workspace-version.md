# ADR-0005: One version for the whole workspace, bumped by a script

**Status:** Accepted

## Context

The workspace has several crates that are developed and released together. Versioning each crate
independently invites drift, and a hand-edited version number invites mistakes: forgetting the
lockfile, the changelog, or an internal dependency.

## Decision

- The whole workspace has **one version**, written once in `[workspace.package]` of the root
  `Cargo.toml`, and every crate inherits it. The starting version is **1.2.36**.
- The version follows **Semantic Versioning 2.0.0**, with the public API defined in
  [releasing.md](../releasing.md).
- Commits follow **Conventional Commits**, so the type of change can decide the bump.
- `scripts/bump-version` performs the bump (`major`, `minor`, `patch`, or `auto` from the commit
  history), updates `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md` together, and can commit and tag.
  `scripts/bump-version --check` runs in CI and fails when anything disagrees.
- Contributors do not change the version. Maintainers do, when they cut a release.

## Consequences

- One number describes a release, and it is impossible to ship inconsistent crate versions.
- A change in a small crate bumps the version of the whole workspace.
- The script is plain POSIX shell, so it has no dependencies and can be read in a minute.
- The version number is a promise of compatibility, so breaking changes have a real cost and are
  recorded in the changelog.

## Alternatives considered

| Option | Why not |
|---|---|
| Independent version per crate | More flexible, but only worthwhile if crates have separate audiences and release cadences. Not yet. |
| Manual edits | Error-prone. The script and the CI check remove the whole class of mistakes. |
| A release tool such as `cargo-release` | Capable, but another dependency for a task a short script does. It can be reconsidered as needs grow. |
