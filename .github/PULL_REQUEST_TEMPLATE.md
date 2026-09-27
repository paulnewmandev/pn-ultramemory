## What and why

<!-- What does this change, and why is it needed? Link the issue it closes: "Closes #123". -->

## Type of change

<!-- Pick one. It decides the next version: breaking = major, feat = minor, fix or perf = patch. -->

- [ ] `fix`: bug fix, backward compatible (patch)
- [ ] `feat`: new backward-compatible functionality (minor)
- [ ] breaking change: incompatible public API (major), marked with `!` or a `BREAKING CHANGE:` footer
- [ ] `docs`, `test`, `refactor`, `ci` or `chore`: no release on its own

## Checklist

- [ ] Commits follow [Conventional Commits](https://www.conventionalcommits.org/) and carry a DCO sign-off (`git commit -s`)
- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace` passes, and new behavior has tests
- [ ] Every new file has its SPDX header and module documentation, and every new item has a docstring (`sh scripts/check-headers`)
- [ ] An entry was added under `## [Unreleased]` in `CHANGELOG.md` (not needed for `docs`, `test`, `ci`, `chore`)
- [ ] I did not change the version: maintainers do it with `scripts/bump-version`
- [ ] Architecture rule respected: `pn-ultramemory-core` still depends on no other workspace crate

## Notes for the reviewer

<!-- Design trade-offs, benchmarks, anything you are unsure about. -->
