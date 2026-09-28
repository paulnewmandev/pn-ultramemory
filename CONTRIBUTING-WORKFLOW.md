<!-- SPDX-License-Identifier: Apache-2.0 -->
# How changes reach `main`

`main` is protected. Nobody pushes to it directly, including the owner. Every change arrives as a
pull request from a branch or a fork.

```
    fork or branch  ──▶  your work  ──▶  pull request  ──▶  review  ──▶  main
                                              │
                                    ci, guards and the
                                    four ratchets must pass
```

## From a fork (you are not a collaborator)

```bash
# 1. Fork on GitHub, then
git clone https://github.com/<you>/pn-ultramemory
cd pn-ultramemory
git remote add upstream https://github.com/paulnewmandev/pn-ultramemory

# 2. Branch from an up-to-date main
git fetch upstream && git switch -c fix/the-thing upstream/main

# 3. Work, then check what CI will check
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
for g in headers refusal panics docs; do cargo run -q -p xtask -- $g; done

# 4. Push to your fork and open a pull request against main
git push origin fix/the-thing
```

## From a branch (you are a collaborator)

The same, without the fork:

```bash
git switch -c fix/the-thing origin/main
# work, check, then
git push -u origin fix/the-thing
```

## What a pull request has to clear

| | |
|---|---|
| `ci` | Format, lint, tests and documentation on macOS, Linux and Windows |
| `ci / hygiene` | File headers, version consistency, dependency policy |
| `guards` | The four ratchets, and the offline proof inside a network namespace |
| Review | One approval from a code owner |

A ratchet baseline may only shrink. If your change adds a finding, fix it rather than regenerating
the baseline — and if it is genuinely acceptable, say why in the pull request.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org). The release script reads them to decide
the next version:

| Prefix | Bump |
|---|---|
| `fix:` · `perf:` | patch |
| `feat:` | minor |
| `feat!:` or a `BREAKING CHANGE:` footer | major |

No trailers naming a tool, a model or a vendor. The commit author is the person responsible for the
change. See [AI_POLICY.md](AI_POLICY.md).
