# ADR-0003: Hexagonal architecture in a Cargo workspace

**Status:** Accepted

## Context

The project will need to swap and extend things over time: the storage engine, the parser, the
transport used by each agent, and the retrieval strategy. Forks should be able to change one of
those without understanding everything else, and the rules of the domain must be testable without a
database, a parser or a network.

## Decision

Use **ports and adapters** (hexagonal architecture), one Cargo crate per concern, all named
`pn-ultramemory-*` and living under `crates/`:

- `pn-ultramemory-core` is the domain. It has no I/O and depends on no other workspace crate. It
  defines the ports as traits.
- Use-case crates (`index`, `recall`, `memory`, `learn`, `codec`) depend only on the domain.
- Adapter crates (`store`, `mcp`, `adapters`, and the `pn-ultramemory` binary) connect to the
  outside world and depend inward.

The dependency rule is that arrows point inward. It is checked in review today, and a CI check
that reads `cargo metadata` was not needed: `core` declares no workspace dependency, so a
violation of the rule does not compile.

## Consequences

- The domain and the packer are pure and fully testable, as the existing oracle tests show.
- A different storage engine is one new adapter that implements the `Storage` port.
- More crates and more boilerplate at the seams than a single crate would need.
- New contributors must learn the dependency rule, and this is documented in
  [architecture.md](../architecture.md) and enforced by review.

## Alternatives considered

| Option | Why not |
|---|---|
| One crate | Simple at first, but the boundaries erode, and every change compiles and tests everything. |
| Layered architecture with the store at the bottom | The domain would depend on the storage engine, which makes it hard to test and to replace. |
