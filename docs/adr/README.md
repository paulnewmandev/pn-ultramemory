# Architecture Decision Records

An ADR is a short document that captures one significant decision: the situation, what was decided,
and what follows from it. They explain *why* the project is the way it is, so that nobody has to
guess and nobody repeats a debate that was already settled.

| ADR | Decision | Status |
|---|---|---|
| [0001](0001-rust.md) | Build in Rust | Accepted |
| [0002](0002-apache-2.0.md) | License under Apache-2.0 | Accepted |
| [0003](0003-hexagonal-architecture.md) | Hexagonal architecture in a Cargo workspace | Accepted |
| [0004](0004-local-only-no-telemetry.md) | Local only, no telemetry, no network by default | Accepted |
| [0005](0005-single-workspace-version.md) | One version for the whole workspace, bumped by a script | Accepted |

## How to add one

1. Copy the structure of an existing ADR into `NNNN-short-title.md` with the next number.
2. Set the status to *Proposed*, and open a pull request. Discussion happens in the review.
3. When it is merged, the status becomes *Accepted*. A later ADR can *supersede* it, and the old
   one is then marked *Superseded by NNNN* but never deleted.

Every ADR has the sections **Context**, **Decision**, **Consequences** and **Alternatives
considered**.
