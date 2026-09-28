<p align="center">
  <img src="assets/logo/banner.svg" alt="pn-ultramemory" width="380">
</p>

<h1 align="center">pn-ultramemory</h1>

<p align="center">
  <b>A code-aware, learning memory for coding agents.</b><br>
  Fewer tokens, sharper recall, one small local binary.
</p>

<p align="center">
  <a href="README.md">🇬🇧 English</a> ·
  <a href="README.es.md">🇪🇸 Español</a>
</p>

<p align="center">
  <a href="https://github.com/paulnewmandev/pn-ultramemory/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/paulnewmandev/pn-ultramemory/actions/workflows/ci.yml/badge.svg?branch=main"></a>
  <a href="https://github.com/paulnewmandev/pn-ultramemory/actions/workflows/guards.yml"><img alt="Guards" src="https://github.com/paulnewmandev/pn-ultramemory/actions/workflows/guards.yml/badge.svg?branch=main"></a>
  <a href="LICENSE"><img alt="License: Apache-2.0" src="https://img.shields.io/badge/license-Apache--2.0-blue"></a>
  <a href="https://www.rust-lang.org"><img alt="Built with Rust" src="https://img.shields.io/badge/built%20with-Rust-000000?logo=rust&logoColor=white"></a>
  <img alt="Rust edition 2024" src="https://img.shields.io/badge/edition-2024-orange?logo=rust&logoColor=white">
  <img alt="unsafe code forbidden" src="https://img.shields.io/badge/unsafe-forbidden-success">
  <img alt="1335 tests passing" src="https://img.shields.io/badge/tests-1335%20passing-success">
</p>

<p align="center">
  <img alt="Telemetry: none" src="https://img.shields.io/badge/telemetry-none-success">
  <img alt="Network: never" src="https://img.shields.io/badge/network-never-success">
  <img alt="Price: free forever" src="https://img.shields.io/badge/price-free%20forever-blueviolet">
  <img alt="Status: new project" src="https://img.shields.io/badge/status-new%20project-orange">
  <a href="docs/releasing.md"><img alt="SemVer 2.0.0" src="https://img.shields.io/badge/semver-2.0.0-3f4551"></a>
</p>

---

## The problem

Your coding agent reads whole files to answer questions about your code. Most of what it reads
is not the answer, and you pay for every token of it.

**pn-ultramemory** indexes your repository into a graph of symbols and the relationships between
them, then answers a question with a **capsule**: the code that matters, at the level of detail
that fits, inside a token budget you set.

It also remembers the decisions and lessons you tell it, **anchored to the code they describe**,
so that when that code changes the memory says so instead of quietly becoming a lie.

One static binary. It never opens a network connection. It costs nothing.

<p align="center">
  <img src="assets/diagrams/why.svg" alt="Reading the files costs 16,109 tokens and finds the right code 85% of the time; a capsule costs 413 tokens and finds it 99% of the time" width="760">
</p>

---

## What it saves

Every figure here comes from `pn-ultramemory bench` on this repository. Nothing is projected.

| Question style | Budget | Finds the right code | Tokens used | Reading files instead | **Saved** |
|---|---:|---:|---:|---:|---:|
| From a description | 500 | **99 %** | 413 | 16,109 | **97.4 %** |
| From a description | 1000 | **99 %** | 802 | 16,109 | **95.0 %** |
| From a description | 2000 | **99 %** | 1,270 | 16,109 | **92.1 %** |
| From a name | 500 | **98 %** | 424 | 16,258 | **97.4 %** |
| From a name | 1000 | **100 %** | 718 | 16,258 | **95.6 %** |

> **Where these come from, and where they do not hold.** They are measured on this tool's own
> source: a Rust repository of about 260 files. The saving depends on the size of the project,
> because what is saved is what reading whole files would have cost. On a small site or a handful of
> scripts, reading them was never expensive, so there is less to save. Measure your own with
> `pn-ultramemory bench`.

<sub>100 sampled tasks per row, Apple M5. The baseline reads the three files a keyword search ranks
highest, which is what an agent without an index does — and it finds the right code only 85 % of the
time from a description and 41 % from a name, while costing forty times more. `task_success` is
reported as `unobservable`, because whether your agent then solved the problem is not something this
tool can measure, and it says so rather than inventing a number.</sub>

### Speed

| Operation | 266 files · 4,880 symbols · 22,458 edges |
|---|---|
| Full index, from nothing | **343 ms** |
| Re-index with nothing changed | **13 ms** |
| One recall | **7.2 ms** median · 8.7 ms at the 95th percentile |

### It stays flat as you grow

Cost is not a claim here, it is a **test**. The suite sweeps repository size and asserts that
`recall`, `map` and `outline` cost the same at every size, each paired with a control measurement
that must **grow** over the same sweep — so a test that stopped measuring anything fails instead of
passing quietly.

---

## At the terminal

```console
$ pn-ultramemory index
indexing ━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 266/266
✓ 266 files, 4880 symbols, 22458 edges  in 0.34s
```

A real progress bar: the indexer reports each file as it finishes, so the number moving is work
actually done and not a guess at how long it will take.

Results are coloured — keys in bold, figures in blue, a green tick when something succeeds. All of
it appears **only when a person is reading**. Pipe the output and you get the same bytes you always
did:

```console
$ pn-ultramemory recall "how are tokens estimated" -b 300 | wc -c   # zero escape sequences
$ pn-ultramemory -f json stats | jq .index.symbols                  # still valid JSON
```

`NO_COLOR` and `TERM=dumb` turn colour off everywhere. The progress bar goes to standard error, so
it never touches the result you are capturing. Running in a terminal also prints the mark:

```
●───◉───●  ╔═╗╔╗╔   ╦ ╦╦  ╔╦╗╦═╗╔═╗╔╦╗╔═╗╔╦╗╔═╗╦═╗╦ ╦
 ╲  │  ╱   ╠═╝║║║═══║ ║║   ║ ╠╦╝╠═╣║║║║╣ ║║║║ ║╠╦╝╚╦╝
  ╰─●─╯    ╩  ╝╚╝   ╚═╝╩═╝ ╩ ╩╚═╩ ╩╩ ╩╚═╝╩ ╩╚═╝╩╚═ ╩
```

---

## Install

> Asking an agent to install this for you? Give it the link to this repository and tell it to follow
> [AGENTS.md](AGENTS.md) — it is written to be read by a model and carried out step by step.

```bash
git clone https://github.com/paulnewmandev/pn-ultramemory
cd pn-ultramemory
cargo build --release          # Rust edition 2024
```

```bash
pn-ultramemory index           # build the graph
pn-ultramemory install --agents claude-code   # or cursor, codex, gemini, windsurf, zed, …
                                              # without --agents it registers with every
                                              # agent it finds; --dry-run shows what it
                                              # would touch and changes nothing
pn-ultramemory doctor          # check it all worked
```

---

## Every command

### Retrieve

```console
$ pn-ultramemory recall "how are tokens estimated" -b 600
capsule:
  query: how are tokens estimated
  budget: 600
  used: 525
  omitted: 15
files[9]{f,path}:
  3,crates/codec/src/tokens.rs
symbols[5]{id,f,lines,kind,name,d,text}:
  7890647377742991523,3,149-157,method,"Features::estimate",L3,"fn estimate(self) -> f64 - Combines the counts into an estimated number of tokens."
also[12]: MAX_CONTEXT_TOKENS,measure,symbol_cost,printed_tokens,estimate_tokens,…
```

| Command | What it answers |
|---|---|
| `recall <question>` | *Which code matters for this?* — packed to a budget |
| `outline <path>` | *What is in this file?* — every symbol, complete, for a fraction of the tokens |
| `expand <symbol>` | *Show me the actual source* — in windows of lines |
| `impact <symbol>` | *What breaks if I change this?* — with how sure the answer is |
| `map` | *What is this repository?* — the whole thing inside a budget |
| `graph` | *Draw it* — Mermaid, DOT, SVG or JSON |

### Remember

| Command | What it does |
|---|---|
| `remember <kind> "<text>" --about <symbol>` | Store a decision, anchored to code |
| `memories --stale` | What may no longer be true |
| `reanchor <id>` · `forget <id>` | Confirm it, or drop it |
| `feedback <signal> --memory <id>` | Tell it how a result turned out |
| `learn status` · `learn why` · `learn reset` | Inspect or clear what it learned |

Nine kinds: `decision`, `fact`, `lesson`, `dead-end`, `error-fix`, `convention`, `requirement`,
`task`, `session`.

### Report and connect

| Command | What it does |
|---|---|
| `report --as html\|pdf\|md --lang en\|es` | A report you can send someone |
| `docs gaps` · `docs apply` · `docs build` | Undocumented symbols, apply docs, build a reference |
| `stats` · `bench` | Numbers, and the benchmark above |
| `serve` · `mcp-config` | MCP over stdio, and the snippet that connects a client |
| `install` · `uninstall` · `doctor` | Setup, exactly reversible |
| `toon encode\|decode` · `completions` | Format conversion, shell completions |

---

## Flags

### Everywhere

| Flag | Default | What it does |
|---|---|---|
| `-C, --repo <PATH>` | nearest `.git` parent | The repository to work on |
| `--data-dir <PATH>` | your user data directory | Where the index lives — never inside your repository |
| `-f, --format <toon\|json\|text>` | `toon` | `toon` is compact, `json` is for programs, `text` is for people |
| `--delimiter <comma\|tab\|pipe>` | `comma` | Column separator of TOON tables; tab is marginally cheaper |
| `-q, --quiet` | off | No progress, no notes — only the result |
| `--no-metrics` | off | Record no usage counters at all |
| `-h, --help` · `-V, --version` | | |

Environment: `PN_ULTRAMEMORY_REPO`, `PN_ULTRAMEMORY_HOME`, `PN_ULTRAMEMORY_NO_METRICS`,
`PN_ULTRAMEMORY_NO_HOOKS`, `PN_ULTRAMEMORY_SESSION`, `NO_COLOR`.

### Per command

| Command | Flags |
|---|---|
| `index` | `--force` re-parse everything · `--threads <N>` |
| `recall` | `-b, --budget <TOKENS>` · `--explain` why each symbol is here · `--path <PREFIX>` |
| `outline` | `-b, --budget <TOKENS>` |
| `expand` | `--from <LINE>` · `--to <LINE>` |
| `impact` | `--depth <N>` up to 5 · `--min-confidence <guess\|heuristic\|resolved\|exact>` · `--limit <N>` |
| `graph` | `--modules` · `--module-depth <N>` · `--depth <N>` · `--max-nodes <N>` · `--min-confidence` · `--as <mermaid\|dot\|svg\|json>` · `--lang <en\|es>` · `-o, --out <FILE>` |
| `map` | `-b, --budget <TOKENS>` · `--path <PREFIX>` |
| `remember` | `--about <SYMBOL>` repeatable · `--by <user\|agent\|tool>` |
| `memories` | `--kind <KIND>` · `--stale` · `--limit <N>` |
| `feedback` | `--memory <ID>` · signal: `useful`, `used`, `ignored`, `dead-end`, `corrected` |
| `report` | `--lang <en\|es>` · `--as <html\|pdf\|md>` · `-o, --out <FILE>` · `--module-depth <N>` · `--title <TEXT>` |
| `bench` | `--tasks <N>` · `--seed <N>` · `-b, --budget <TOKENS>` repeatable · `--baseline-files <N>` |
| `docs gaps` | `--path <PREFIX>` · `--limit <N>` · `--context` |
| `docs apply` | `<FILE>` or `-` for stdin · `--dry-run` |
| `docs build` | `--path <PREFIX>` · `--title <TEXT>` · `-o, --out <FILE>` |
| `install` | `--agents <ID>` repeatable · `--scope <user\|project>` · `--dry-run` · `--force` · `--name <NAME>` · `--command <PATH>` · `--pin-repo` |

---

## How it works

<p align="center">
  <img src="assets/diagrams/how.svg" alt="Your code is indexed into a graph; a question finds seeds, walks the graph, ranks and packs candidates to a budget, and the capsule is measured before it is returned" width="760">
</p>

### Five resolutions per symbol

<p align="center">
  <img src="assets/diagrams/levels.svg" alt="Each symbol can be shown at one of five levels, from its name alone to its whole source, each costing more than the one below" width="760">
</p>

`recall` solves a **multi-resolution knapsack**: one level per symbol, maximising usefulness inside
the budget. Checked against an exact dynamic-programming solver over 3,000 random instances.

When the budget is tight it **degrades in steps** instead of breaking: full source, then signatures,
then a plain list of names. At 150 tokens it still returns five useful names, which is what makes it
work for a small model.

### An outline never drops a symbol

```console
$ pn-ultramemory outline crates/codec/src/tokens.rs
file:
  path: crates/codec/src/tokens.rs
  lines: 274
  symbols: 21
  detail: documented
  tokens: 1061
  whole_file_tokens: 2669
  saved: 0.602
```

Every symbol the file declares, in order, nested the way it nests. When a budget is too small the
**detail** falls — documentation, then signatures, down to bare names — and the symbol list stays
whole, because a skeleton missing three functions reads as *they are not there*.

It also says when the file is short enough that **reading it outright is cheaper**, rather than
charging you tokens for a worse answer than `cat`.

### The epistemic envelope

`impact` never says "safe". It says one of three things:

| | |
|---|---|
| **`exact`** | The set is complete |
| **`lower-bound`** | At least these; there may be more |
| **`unknown`** | No caller found and the symbol is public — which does **not** mean nobody uses it |

Every edge carries how sure the indexer was: `Guess`, `Heuristic`, `Resolved`, `Exact`.

<p align="center">
  <img src="assets/diagrams/memory.svg" alt="A memory is stored with hashes of the symbol it describes; when that symbol changes the memory is marked stale rather than deleted or trusted" width="760">
</p>

### The rule that governs memory

> **Corroboration raises how likely a memory is to be RETRIEVED. It never raises how likely it is
> to be TRUE.**

Without it, an agent repeating its own mistake in a loop would manufacture a fact, and the tool
would serve that fact with confidence to every later session. So: two repetitions **within 15
minutes count as one**, whoever sent them; text captured from a tool (`--by tool`) **never**
corroborates; and no memory gathers more than **8** corroborations.

### Contradictions are checked before similarity

A real case from this repository shows why:

| Pair | Similarity | Reality |
|---|---:|---|
| "calibrated against a real tokenizer, **not** guessed" vs "**not** calibrated against a real tokenizer" | **0.838** | They contradict |
| "never unwrap in a request path, return an error" vs "never unwrap in a request path; return an error instead" | **0.844** | They agree |

**The contradiction scores lower than the agreement.** No text similarity measure can tell them
apart, because negating a sentence changes almost none of its words. Without the structural check, a
memory could be reinforced by its own opposite.

---

## The graph

```bash
pn-ultramemory graph --modules --as svg -o graph.svg
```

No other code graph is drawn this way, and that is deliberate.

- **An edge is a filled shape, not a line.** Two cubics share their control points, so the fibre is
  wide where it leaves the caller and comes to a point where it arrives. **The taper carries the
  direction**, which is why there is not one arrowhead in the picture. A hundred arrowheads are
  noise; a hundred tapers are a texture.
- **Fibres are bundled at both ends.** The second is the one that matters: a code graph is not a
  tree calling outwards, it is a few symbols that everything calls *into*. Bundling by target is
  what makes fibres converge on a soma the way processes do.
- **Every fibre stops short of the soma it points at.** That gap is the synaptic cleft, and the
  swelling on its near side is the terminal.
- **A fibre takes the colour of the module it leaves**, so a connection can be traced back by colour
  alone.
- **The glow is three flat discs**, not a blur filter: a filter is the slowest thing in a drawing
  this size, and three steps of falling opacity already read as light.

Pure SVG and CSS. No JavaScript, no external font, no request. Light and dark follow the viewer, the
palette is colourblind-safe (Okabe–Ito), and the same input always produces the same bytes.

---

## Works with the agent you already use

`pn-ultramemory install` registers itself everywhere it finds an agent, writing **only its own
entry** and leaving the rest of the file byte for byte as it was.

| | | | |
|---|---|---|---|
| Claude Code | Codex CLI | Cursor | Gemini CLI |
| Windsurf | Zed | Visual Studio Code | opencode |
| **Kiro** | **Trae** | Cline | Crush |
| Amp | | | |

<sub>`uninstall` removes exactly what `install` wrote — a round trip leaves the file identical, which
is what the installer's 60 tests check. Trae's path is marked *unverified*: it is offered with
`--agents trae` and never written to by default.</sub>

Five tools are offered over MCP — **`recall`**, **`outline`**, **`impact`**, **`remember`** and
**`expand`** — and the whole tool list costs **2,139 bytes** of the agent's context.

---

## Architecture

Hexagonal: the core knows nothing about SQLite, tree-sitter or the file system.

```
          ┌──────────┐   ┌──────────┐   ┌──────────┐
 entry    │   cli    │   │   mcp    │   │  report  │
          └────┬─────┘   └────┬─────┘   └────┬─────┘
               └──────────────┼──────────────┘
                         ┌────▼─────┐
 use cases               │  engine  │   index · recall · outline · remember
                         └────┬─────┘   impact · graph · map · docs · bench
                              │
                         ┌────▼─────┐
 contracts               │   core   │   Storage · Extractor · SourceTree · Clock
                         └────┬─────┘   no I/O, no internal dependencies
               ┌──────────────┼──────────────┐
          ┌────▼─────┐   ┌────▼─────┐   ┌────▼─────┐
 adapters │  store   │   │  index   │   │  codec   │
          │ (SQLite) │   │(tree-sit)│   │  (TOON)  │
          └──────────┘   └──────────┘   └──────────┘
```

| Crate | Lines | What it is |
|---|---:|---|
| `core` | 2,319 | The contracts. No I/O, no dependency on any other crate here |
| `codec` | 1,743 | Token estimation and the multi-resolution packer |
| `toon` | 5,435 | TOON 4.1 — passes all **538** official conformance fixtures |
| `index` | 13,012 | tree-sitter for 12 languages, plus a fallback for every other |
| `store` | 10,802 | SQLite with WAL and FTS5 |
| `engine` | 18,793 | Every use case |
| `mcp` | 5,679 | Model Context Protocol over stdio, five revisions |
| `report` | 12,421 | HTML, PDF and Markdown — **zero dependencies** |
| `cli` | 8,034 | The command line, hooks, colour and the installer |
| `xtask` | 3,850 | The quality guards |
| | **83,035** | **1,335 tests** |

Parsed with tree-sitter: **Rust, Python, JavaScript, TypeScript, TSX, Go, Java, C, C++, C#, Ruby,
PHP**. Every other language is still indexed by a lexical fallback, so nothing in your repository is
invisible.

---

## What it will not do

An honest tool says what it cannot do.

- **It does not understand meaning.** Every text comparison is over words. A paraphrase sharing no
  words is not recognised as a duplicate.
- **It reads English and Spanish.** A memory in another language is stored and retrieved correctly,
  but is rarely recognised as a duplicate of another in that language.
- **It cannot tell a true statement from a false one.** Stale means *the code changed*, not *the
  memory is now wrong*. A person decides that.
- **Contradiction detection has deliberately low recall.** It catches structural opposition. It will
  miss one that needs knowledge of your domain.
- **`task_success` is not measured.** Whether your agent solved the problem is outside what this
  tool can observe.

---

## Quality

Nothing merges unless all of this is green.

| | |
|---|---|
| **1,335 tests** | unit, integration, property, conformance and cost-class |
| `cargo clippy --all-targets -- -D warnings` | zero findings |
| `cargo doc -D warnings` | zero findings |
| `cargo deny check` | advisories, bans, licences, sources |
| **Permissive licences only** | an allowed licence nothing uses any more **fails** the check |
| `#![forbid(unsafe_code)]` | in every crate |

Four ratchets guard what tests cannot. Each has a baseline that **may only shrink**, and a baseline
entry that stops matching anything **fails the build**, so a guard cannot be quietly disabled:

| Ratchet | What it holds |
|---|---|
| `headers` | Every file carries its licence header and module documentation |
| `refusal` | Every error message names a way forward |
| `panics` | Library code does not panic; indexing and casts are checked |
| `docs` | A public item's first sentence must add something to its name |

---

## Documentation

| | |
|---|---|
| [docs/architecture.md](docs/architecture.md) | The layers and why they are separate |
| [docs/memory.md](docs/memory.md) | What is stored, how duplicates and contradictions are found |
| [docs/formats.md](docs/formats.md) | TOON, capsules, and every output shape |
| [docs/benchmark.md](docs/benchmark.md) | How the numbers above are produced |
| [docs/quality.md](docs/quality.md) | Every guard, and what each does **not** prove |
| [docs/glossary.md](docs/glossary.md) | The words this project uses precisely |

**To use it:** the [wiki](https://github.com/paulnewmandev/pn-ultramemory/wiki) — install, your
first hour, every command, FAQ and troubleshooting.
**To have an agent install it:** hand it [AGENTS.md](AGENTS.md).
**To contribute:** [CONTRIBUTING-WORKFLOW.md](CONTRIBUTING-WORKFLOW.md).

---

## Licence and contributing

Apache-2.0. Use it, change it, sell it, fork it — see [LICENSE](LICENSE) and
[TRADEMARKS.md](TRADEMARKS.md).

[CONTRIBUTING.md](CONTRIBUTING.md) · [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) ·
[SECURITY.md](SECURITY.md) · [SUPPORT.md](SUPPORT.md) · [GOVERNANCE.md](GOVERNANCE.md)

<p align="center"><sub>
Built by <a href="https://github.com/paulnewmandev">Paul Newman</a>. No telemetry. No account.
No network. Free, and staying that way.
</sub></p>
