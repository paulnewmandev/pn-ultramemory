# Governance

pn-ultramemory is an open project with a simple, transparent governance model. It is built so that
it can grow beyond a single person without changing hands.

## Roles

| Role | Who | Responsibilities |
|---|---|---|
| **Maintainer** | Paul Newman ([@paulnewmandev](https://github.com/paulnewmandev)) | Reviews and merges changes, cuts releases, owns the roadmap, enforces the Code of Conduct. |
| **Contributor** | Anyone who sends a change, an issue or a review | Follows [CONTRIBUTING.md](CONTRIBUTING.md) and the [Code of Conduct](CODE_OF_CONDUCT.md). |

The project currently has one maintainer. Adding more is an explicit goal, so that reviews are
faster and no single person is a point of failure.

## How decisions are made

- **Everyday changes** are decided in pull requests by *lazy consensus*: if a maintainer approves
  and nobody raises a reasoned objection, it merges.
- **Design and architecture decisions** are written down as Architecture Decision Records in
  [docs/adr](docs/adr). Proposals start as an issue or discussion, and the ADR records the outcome
  and the alternatives that were rejected.
- **Breaking changes** need an issue that explains the migration path, and result in a major
  version (see [docs/releasing.md](docs/releasing.md)).
- **Disagreements** are resolved by evidence first (a test, a benchmark, a reproduction). If that
  is not enough, the maintainers decide and record the reasoning.

## Becoming a maintainer

Maintainers are added when a contributor has, over time:

1. Landed several non-trivial changes that met the project's standards.
2. Reviewed other people's changes constructively.
3. Shown good judgment on the project's principles: local-only, deterministic, documented, small.

Any maintainer can nominate a contributor, and the existing maintainers agree by consensus. New
maintainers receive review rights first, and release and publishing rights after a period of
shared releases. `CODEOWNERS` is split by area as the team grows.

## Continuity

- Releases are made with a scripted, documented process ([docs/releasing.md](docs/releasing.md)),
  so any maintainer can perform them.
- Build and test configuration lives in the repository and is pinned, so the project can be built
  by anyone.
- The license is Apache-2.0. Anyone may fork the project at any time.

## Changing this document

Changes to governance are proposed by pull request and need approval from every maintainer.
