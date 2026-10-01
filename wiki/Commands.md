# Commands

Every command and the flags that matter. `pn-ultramemory <command> --help` prints the same from the
binary itself.

## Everywhere

| Flag | Default | What it does |
|---|---|---|
| `-C, --repo <PATH>` | nearest parent with `.git` | The repository to work on |
| `--data-dir <PATH>` | your user data directory | Where the index lives; never inside your repository |
| `-f, --format <toon\|json\|text>` | `toon` | `toon` is compact for models, `json` is for programs, `text` is for people |
| `--delimiter <comma\|tab\|pipe>` | `comma` | Column separator of TOON tables; tab is marginally cheaper |
| `-q, --quiet` | off | Only the result: no progress, no notes |
| `--no-metrics` | off | Record no usage counters at all |

Environment: `PN_ULTRAMEMORY_REPO`, `PN_ULTRAMEMORY_HOME`, `PN_ULTRAMEMORY_NO_METRICS`,
`PN_ULTRAMEMORY_NO_HOOKS`, `PN_ULTRAMEMORY_SESSION`, `NO_COLOR`.

---

## Index

### `index [PATH…]`

Builds or refreshes the graph. Only files whose content changed are read again; with paths, only
those files.

| Flag | |
|---|---|
| `--force` | Read every file again, even unchanged ones |
| `--threads <N>` | Parsing threads. Default: the number of cores, at most eight |

`.gitignore` is honoured; files over 2 MB and minified files are skipped.

---

## Read

### `brief`

What a new session needs to know: size, languages, modules and their coupling, the most-called
symbols and every memory on record.

| Flag | |
|---|---|
| `-b, --budget <TOKENS>` | Default 1,200. Sections are given up to fit: busiest symbols first, recorded decisions last |

### `recall <question>`

The code that matters for a question, packed into a budget.

| Flag | |
|---|---|
| `-b, --budget <TOKENS>` | What the answer may cost |
| `--explain` | Add why each symbol is in the answer |
| `--path <PREFIX>` | Only look under this path |

### `outline <path>`

Every symbol one file declares, in order and nested. A tight budget lowers the detail; no symbol is
ever dropped.

| Flag | |
|---|---|
| `-b, --budget <TOKENS>` | Detail falls to fit it |

### `expand <symbol>`

The source of one symbol, by name or by the id other results print, in windows of at most 400 lines
or 6,000 tokens.

| Flag | |
|---|---|
| `--from <LINE>` · `--to <LINE>` | The window |

### `impact <symbol>`

What depends on a symbol, with an `epistemic` verdict: `exact`, `lower-bound` or `unknown`.

| Flag | |
|---|---|
| `--depth <N>` | Steps of callers to follow, at most 5 |
| `--min-confidence <guess\|heuristic\|resolved\|exact>` | The weakest edge to follow. Default `heuristic` |
| `--limit <N>` | Stop after this many callers |

### `map`

The whole repository inside a budget: per file its path, language, lines, symbol count and top
symbols.

| Flag | |
|---|---|
| `-b, --budget <TOKENS>` | What the map may cost |
| `--path <PREFIX>` | Only paths under this prefix |

---

## See

### `brain`

The whole repository as a 3D brain in the browser. See [The brain](Brain).

| Flag | Default | |
|---|---|---|
| `--max-nodes <N>` | 6,000 | The most symbols drawn; the most depended on are kept |
| `--min-confidence` | `heuristic` | The weakest edge drawn |
| `--lang <en\|es>` | `en` | Language of the page |
| `-o, --out <FILE>` | `brain.html` in the data directory | Where to write it |
| `--no-open` | off | Write it without opening the browser. It only opens when you are at a terminal anyway |

### `graph [SYMBOL]`

The graph as a diagram or as data: the neighbourhood of a symbol, or the most central symbols.

| Flag | Default | |
|---|---|---|
| `--as <mermaid\|dot\|svg\|json>` | `mermaid` | What to produce |
| `--modules` · `--module-depth <N>` | off · 2 | Directories instead of symbols, and how many levels name one |
| `--depth <N>` · `--max-nodes <N>` | 2 · 60 | Size |
| `--min-confidence` | `heuristic` | The weakest edge drawn |
| `--lang <en\|es>` · `-o, --out <FILE>` | | Labels, and where to write |

---

## Remember

### `remember <kind> "<text>"`

Kinds: `decision`, `fact`, `lesson`, `dead-end`, `error-fix`, `convention`, `requirement`, `task`,
`session`.

| Flag | |
|---|---|
| `--about <SYMBOL>` | Anchor it to a symbol. Repeat for several |
| `--by <user\|agent\|tool>` | Who wrote it. Default `user`. `tool` is untrusted and never corroborates |

### `memories` · `reanchor <id>` · `forget <id>`

| Flag | |
|---|---|
| `--kind <KIND>` · `--stale` · `--limit <N>` | Filters for `memories` |

`reanchor` confirms that a stale memory is still true; `forget` drops one. Both are deliberately
command-line only. See [Memories](Memories).

### `feedback <signal>` · `learn status|why|reset`

Signals: `useful`, `used`, `ignored`, `dead-end`, `corrected`. Name a symbol, or `--memory <ID>`.
`learn why` shows what was learned about one target; `learn reset` forgets it all.

---

## Report and measure

### `report`

| Flag | |
|---|---|
| `--as <html\|pdf\|md>` · `--lang <en\|es>` | Format and language |
| `-o, --out <FILE>` · `--title <TEXT>` · `--module-depth <N>` | Default file: `pn-ultramemory-report.<ext>` in the current directory |

### `docs gaps|apply|build`

| Command | Flags |
|---|---|
| `docs gaps` | `--path <PREFIX>` · `--limit <N>` · `--context` (ready to hand to an agent) |
| `docs apply` | `<FILE>` or `-` for standard input · `--dry-run` |
| `docs build` | `--path <PREFIX>` · `--title <TEXT>` · `-o, --out <FILE>` |

`docs apply` is the one command that writes into your sources, because that is what you ask it for.

### `stats` · `bench`

| Flag for `bench` | |
|---|---|
| `--tasks <N>` · `--seed <N>` | How many symbols to sample, and the seed that repeats a run |
| `-b, --budget <TOKENS>` | A budget to test. Repeat for several |
| `--baseline-files <N>` | How many whole files the baseline reads. Default 3 |

---

## Connect

### `install` · `uninstall`

| Flag | |
|---|---|
| `--agents <ID>` | Repeat for several. **Without it, every agent found is configured** |
| `--scope <user\|project>` | Your configuration, or the project's |
| `--dry-run` | Print every change, make none |
| `--force` | Replace an entry that was edited by hand |
| `--name <NAME>` · `--command <PATH>` | The name the agent shows, and the binary it starts. Defaults: `pn-ultramemory`, this executable |
| `--pin-repo` | Pin the entry to this repository, so the agent always works on it |

Agent ids: `claude-code`, `cursor`, `codex`, `gemini`, `windsurf`, `zed`, `vscode-copilot`,
`opencode`, `kiro`, `trae`, `cline`, `crush`, `amp`.

### `doctor` · `serve` · `mcp-config`

`doctor` checks the setup and names the fix for anything wrong. `serve` runs the MCP server on
standard input and output. `mcp-config` prints the snippet that connects a client by hand
(`--name`, `--command`).

### `toon encode|decode` · `completions <shell>`

Convert between JSON and TOON. Shells: `bash`, `zsh`, `fish`, `powershell`, `elvish`.

---

## Exit codes

| | |
|---|---|
| `0` | Success |
| `1` | The environment failed: a file, the database, a permission |
| `2` | An invalid or refused request |
| `3` | Something named does not exist |

A hook always exits `0`, whatever happens: a hook must never fail the tool call it is attached to.
