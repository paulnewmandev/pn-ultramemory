# Vendored TOON conformance fixtures

These JSON files are the official language-agnostic conformance fixtures of the TOON
specification, copied unmodified into this repository so that the test suite of
`pn-ultramemory-toon` runs without network access.

- Source: <https://github.com/toon-format/spec>, directory `tests/fixtures` (revision
  `d6db4b04303bdea132351ce45aed612311c850b2`).
- Specification version: 4.1 (`SPEC.md` of the same revision).
- License: MIT, Copyright (c) 2025-PRESENT Johann Schopplich. The full text is in
  [`LICENSE`](LICENSE) in this directory. It applies to the files under `encode/` and `decode/`
  only; the rest of this repository is Apache-2.0.

Layout: `encode/*.json` are JSON to TOON cases, `decode/*.json` are TOON to JSON cases. Each case
may carry `options` (`delimiter`, `indentSize`, `strict`) and `shouldError`. The runner is
[`../fixtures.rs`](../fixtures.rs); it loads every file and reports per-file counts.
