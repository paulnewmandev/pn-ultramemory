<!-- SPDX-License-Identifier: Apache-2.0 -->
# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). The version is bumped with
[`scripts/bump-version`](scripts/bump-version); see [docs/releasing.md](docs/releasing.md).

## [Unreleased]

## [1.2.36] - 2026-09-27

The first complete version: everything below is implemented, tested and reachable from the command
line and from MCP.

### Added — retrieval

- **`recall`** — answers a question with a capsule of the code that matters, packed into a token
  budget. Multi-resolution knapsack over five levels of detail (name, signature, summary, outline,
  source), checked against an exact dynamic-programming solver over 3,000 random instances. The
  capsule is rendered and measured rather than estimated, so `used` is what the output costs.
- **`outline`** — describes one whole file: every symbol it declares, in order, nested, with
  signatures and first documentation sentences. A tight budget lowers the detail instead of dropping
  symbols, and the result reports when reading the file outright would be cheaper.
- **`expand`** — the source of one symbol, in windows of at most 400 lines or 6,000 tokens.
- **`impact`** — what depends on a symbol, with an epistemic envelope: `exact`, `lower-bound` or
  `unknown`. It never reports "safe".
- **`map`** — the whole repository within a token budget.
- **`graph`** — the code graph as Mermaid, DOT, SVG or JSON, for symbols or for modules.

### Added — memory and learning

- **`remember` / `memories` / `forget` / `reanchor`** — memories of nine kinds, anchored to the
  symbols they describe and marked stale when that code changes.
- Duplicate detection: a 64-bit pre-filter, then the mean of a word-set overlap and a longest common
  subsequence.
- Contradiction detection, checked **before** similarity. A contradiction scores *lower* than an
  agreement on this repository (0.838 against 0.844), so no text measure can separate them; a
  structural check can, and without it a memory could be reinforced by its own opposite.
- Bounded corroboration: two repetitions within fifteen minutes count as one, text captured from a
  tool never corroborates, and no memory gathers more than eight. Corroboration raises how likely a
  memory is to be retrieved and never how likely it is to be true.
- **`feedback` / `learn`** — usage signals feeding a Beta posterior with decay, producing a ranking
  multiplier clamped to `[0.5, 1.5]`. It can re-rank an answer; it can never create a fact.

### Added — indexing and storage

- Incremental indexing: only changed files are re-parsed, and changing one file re-resolves only the
  edges that could have moved. Re-indexing this repository with nothing changed takes 13 ms.
- tree-sitter for twelve languages (Rust, Python, JavaScript, TypeScript, TSX, Go, Java, C, C++, C#,
  Ruby, PHP) and a lexical fallback for every other, so nothing in a repository is invisible.
- SQLite with WAL and FTS5. Bulk insert at 48,000 symbols per second.

### Added — interfaces

- **MCP server** over stdio with five tools (`recall`, `outline`, `impact`, `remember`, `expand`)
  across five protocol revisions. The whole tool list costs 2,139 bytes of an agent's context.
- **`install` / `uninstall` / `doctor`** — registers as an MCP server in thirteen coding agents,
  writing only its own entry. A round trip leaves the file byte for byte as it was.
- **`report`** — a self-contained HTML page, a PDF written byte by byte, or Markdown, in English or
  Spanish, with no JavaScript and no external request.
- **`docs gaps` / `docs apply` / `docs build`** — undocumented public symbols, applying
  documentation written for them, and a Markdown API reference for any language.
- **`stats`**, **`bench`**, **`toon`**, **`completions`**, **`mcp-config`**.

### Added — at the terminal

- A progress bar during indexing, driven by the indexer reporting each file as it finishes.
- Colour on results: keys, table headers and figures. Shown **only** when a person is reading —
  piped output carries no escape sequence, and `NO_COLOR` and `TERM=dumb` are honoured.
- An ASCII mark and wordmark, built from a glyph table so the letters cannot drift out of alignment.

### Added — the graph's drawing

- Edges are drawn as filled tapered fibres rather than stroked lines: wide where they leave the
  caller, coming to a point where they arrive. The taper carries the direction, so the drawing has
  no arrowheads.
- Fibres are bundled at both ends, which is what makes a hub read as a cell body receiving an arbor
  rather than as a star.
- Every fibre stops short of the node it points at, and the terminal sits in that gap.
- Pure SVG and CSS: no JavaScript, no external font, no request. Colourblind-safe palette.

### Added — quality

- 1,335 tests: unit, integration, property, conformance, hostile-input and cost-class.
- **Cost-class assertions**: the suite sweeps repository size and asserts that `recall`, `map` and
  `outline` cost the same at every size, each paired with a control that must grow over the same
  sweep — so a test that stopped measuring anything fails instead of passing quietly.
- Four ratchets (`headers`, `refusal`, `panics`, `docs`) whose baselines may only shrink, and where
  a baseline entry matching nothing fails the build.
- `#![forbid(unsafe_code)]` in every crate; `clippy --all-targets -D warnings` and `cargo doc
  -D warnings` clean; permissive licences only, with an allowed licence nothing uses any more
  failing the check.
- A CI job that runs the suite inside a network namespace with no route out, so "no network call"
  is checked rather than claimed.

### Added — project

- Cargo workspace, Rust edition 2024, pinned toolchain, one shared version.
- `scripts/bump-version`, `scripts/check-headers`, `scripts/ratchet`, `scripts/check-offline`.
- Community health files, issue and pull request templates, and two CI workflows.

[Unreleased]: https://github.com/paulnewmandev/pn-ultramemory/compare/v1.2.36...HEAD
[1.2.36]: https://github.com/paulnewmandev/pn-ultramemory/releases/tag/v1.2.36
