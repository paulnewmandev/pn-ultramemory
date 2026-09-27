# Security Policy

pn-ultramemory is designed to run on developers' machines, read their source code and store what
coding agents learn about it. That makes security a core property, not an afterthought. Thank you
for helping keep it and its users safe.

## Supported versions

Security fixes are made on the latest minor release of the current major version.

| Version | Supported |
|---|---|
| Latest 1.x release | Yes |
| Older releases | No. Please upgrade. |

## Reporting a vulnerability

**Please do not open a public issue, discussion or pull request for a vulnerability.**

Use either channel:

1. **GitHub private vulnerability reporting (preferred).** Go to the
   [Security tab](https://github.com/paulnewmandev/pn-ultramemory/security/advisories/new) of the
   repository and choose *Report a vulnerability*.
2. **Email** [paul.newman.dev@gmail.com](mailto:paul.newman.dev@gmail.com) with the subject
   `pn-ultramemory security`.

Please include:

- The affected version (`pn-ultramemory --version`) and your operating system.
- A clear description of the problem and its impact.
- Steps to reproduce, or a minimal proof of concept.
- Whether you plan to publish your findings, and when.

### What to expect

This is a volunteer-maintained project, so these are targets and not contractual promises.

| Step | Target |
|---|---|
| Acknowledge your report | Within 5 business days |
| First assessment (accepted, needs more information, or declined, with reasons) | Within 10 business days |
| Fix or mitigation for a confirmed vulnerability | Depends on severity. Critical issues come first. |
| Coordinated public disclosure | Once a fix is released, and no later than 90 days after your report unless we agree otherwise |

We will credit you in the advisory and the changelog unless you prefer to stay anonymous.

### Safe harbor

If you make a good-faith effort to follow this policy, avoid privacy violations and data
destruction, and give us reasonable time to respond, we will not pursue legal action against you
for your research.

## What is in scope

pn-ultramemory reads untrusted input (repositories, tool output) and stores text that a language
model will later read, so the most interesting problems are of these kinds:

| Area | Examples |
|---|---|
| **Memory poisoning and prompt injection** | Stored text that makes an agent follow attacker instructions; a memory that is presented as trusted when it came from an untrusted source. |
| **Path handling** | Path traversal or symlink escapes when reading or expanding files outside the indexed repository. |
| **Resource exhaustion** | A malicious repository that hangs or exhausts memory or disk in the parser or indexer. |
| **Secret exposure** | Credentials or personal data written into the store, or logged, that should have been redacted. |
| **Network behavior** | Any outbound connection made without an explicit user action. The default is none. |
| **Supply chain** | Tampering with release artifacts, CI workflows, dependencies or the update path. |

Out of scope: vulnerabilities in third-party software that is not part of this repository (report
those upstream), issues that require an attacker who already has full control of the user's
account, and findings without a demonstrated security impact, such as missing hardening headers on
a tool that serves nothing.

## Security design principles

These are commitments the design is held to, and any violation is a bug worth reporting.

| Principle | Meaning |
|---|---|
| **Local only** | Everything stays on the machine. No telemetry and no network calls by default. |
| **Memory is data, not instructions** | Stored memories are shown to a model as context and are never executed or obeyed. |
| **Provenance on everything** | Each memory records whether it came from a tool, an agent or a human. Only human-verified content may be promoted to a convention. |
| **Least surprise for the user's repository** | The tool does not write into the user's repository unless asked. Its data lives in its own directory. |
| **Bounded resources** | Limits on file size, parse time and recursion protect against hostile inputs. |
| **Reproducible, verifiable releases** | Pinned toolchain, locked dependencies, license and advisory checks in CI, and provenance for release artifacts. |

Most of these describe the target design of the project as it is built. The
[CHANGELOG](CHANGELOG.md) records what exists in each release, and the
[architecture document](docs/architecture.md) describes what is built; nothing in it is design intent.
