# ADR-0004: Local only, no telemetry, no network by default

**Status:** Accepted

## Context

The tool reads private source code and stores what agents learn about it. Users must be able to
trust that none of it leaves their machine. Several comparable tools state that they make no
network calls while still shipping update checks or event forwarders, which damages that trust.

## Decision

- **No telemetry**, and no analytics of any kind.
- **No network calls by default.** Anything that would use the network is an explicit command run
  by the user, and never a background behavior.
- **No data leaves the machine** unless the user exports it.
- The tool keeps its data in its own directory and does not write into the user's repository unless
  asked.
- A test in CI will verify the no-network property by running the binary with networking blocked.

## Consequences

- Users, and companies with strict policies, can adopt it without a review of outbound traffic.
- There is no usage data, so decisions rely on the public benchmarks, issues and discussions.
- Update notification is not automatic. Users update through their package manager.
- Any future networked feature, such as team sync, has to be opt-in, separate and open.

## Alternatives considered

| Option | Why not |
|---|---|
| Opt-out telemetry | Trust is the product. Even well-intentioned telemetry undermines it. |
| Opt-in telemetry | Adds code and a permanent question mark to a tool that reads private code. |
