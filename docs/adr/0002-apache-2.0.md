# ADR-0002: License under Apache-2.0

**Status:** Accepted

## Context

The project should be free to copy, modify, redistribute and reproduce, by individuals and by
companies, so that it can be forked, sponsored and adopted without legal friction. Licenses that
restrict commercial use or hosting (Elastic License, PolyForm, Business Source, SSPL) are not open
source, discourage forks and corporate contributors, and were rejected from the start.

## Decision

License the project under the **Apache License 2.0**. Contributions are accepted under the same
license (inbound equals outbound), and contributors keep their copyright and sign their commits
with the Developer Certificate of Origin. There is no contributor license agreement.

Every source file carries `SPDX-License-Identifier: Apache-2.0`, and dependencies are restricted to
permissive licenses by `cargo-deny`.

## Consequences

- Anyone may use, modify, distribute and sell the software, including in closed-source products,
  provided they keep the notices. There is no copyleft.
- The license includes an explicit patent grant, which companies value.
- The project cannot be combined into a GPLv2-only work, because Apache-2.0 is not compatible with
  GPLv2 (it is compatible with GPLv3).
- Any hosted service the maintainers might offer later cannot be protected by the license, and has
  to compete on quality instead.

## Alternatives considered

| Option | Why not |
|---|---|
| MIT | Simpler, but with no patent grant. |
| MIT OR Apache-2.0 (the Rust convention) | Adds compatibility with GPLv2 works. It can be adopted later with the agreement of all contributors, and DCO sign-off keeps that possible. |
| GPL or AGPL | Free, but the copyleft discourages corporate adoption and sponsorship. |
