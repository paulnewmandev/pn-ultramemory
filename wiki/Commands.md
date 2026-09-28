# Commands

Every command, what it answers, and the flags that matter. `--help` on any of them prints the same
thing from the binary itself.

## Global

Available on every command.

| Flag | Default | What it does |
|---|---|---|
| `-C, --repo <PATH>` | nearest `.git` parent | The repository to work on |
| `--data-dir <PATH>` | your user data directory | Where the index lives, never inside your repository |
| `-f, --format <toon\|json\|text>` | `toon` | `toon` is compact, `json` is for programs, `text` is for people |
| `--delimiter <comma\|tab\|pipe>` | `comma` | Column separator; tab is marginally cheaper in tokens |
| `-q, --quiet` | off | No progress, no notes — only the result |
| `--no-metrics` | off | Record no usage counters at all |

Environment: `PN_ULTRAMEMORY_REPO`, `PN_ULTRAMEMORY_HOME`, `PN_ULTRAMEMORY_NO_METRICS`,
`PN_ULTRAMEMORY_NO_HOOKS`, `PN_ULTRAMEMORY_SESSION`, `NO_COLOR`.

---

## Index

### `index`

Builds or refreshes the graph. Only files whose content changed are read again.

| Flag | |
|---|---|
| `--force` | Re-parse everything, even unchanged files |
| `--threads <N>` | Worker threads for parsing |

---

## Retrieve

### `brief`

Everything a new session needs to know about this repository: its size, its languages, its modules,
its busiest symbols and every memory already recorded. One call instead of an hour of explaining.

| Flag | |
|---|---|
| `-b, --budget <TOKENS>` | Default 1200. Sections are given up from the bottom to fit it |

Sections are printed in the order a reader needs them and given up in a different order: the
busiest symbols go first, because reading the code finds them again, and the recorded decisions go
last, because nothing else can recover them.

### `recall <question>`

The code that matters for a question, packed into a budget.

| Flag | |
|---|---|
| `-b, --budget <TOKENS>` | How much the answer may cost |
| `--explain` | Add why each symbol is in the answer |
| `--path <PREFIX>` | Only look inside paths starting with this |

### `outline <path>`

Every symbol one file declares, in order, nested, with signatures and first documentation
sentences. Never drops a symbol: a tight budget lowers the detail instead.

| Flag | |
|---|---|
| `-b, --budget <TOKENS>` | Detail falls to fit it |

### `expand <symbol>`

The source of one symbol, in windows of at most 400 lines or 6,000 tokens.

| Flag | |
|---|---|
| `--from <LINE>` · `--to <LINE>` | The window, counted from the symbol's first line |

### `impact <symbol>`

What depends on a symbol, and how sure that answer is.

| Flag | |
|---|---|
| `--depth <N>` | Steps of callers to follow, up to 5 |
| `--min-confidence <guess\|heuristic\|resolved\|exact>` | The weakest edge to follow |
| `--limit <N>` | Stop after this many callers |

### `map`

The whole repository inside a budget: per file its path, language, lines, symbol count and top
symbols.

| Flag | |
|---|---|
| `-b, --budget <TOKENS>` · `--path <PREFIX>` | |

### `graph`

The code graph as a picture or as data.

| Flag | |
|---|---|
| `--modules` | Modules instead of symbols |
| `--as <mermaid\|dot\|svg\|json>` | What to produce |
| `--depth <N>` · `--max-nodes <N>` · `--module-depth <N>` | Size |
| `--min-confidence` | The weakest edge to draw |
| `--lang <en\|es>` · `-o, --out <FILE>` | Labels and where to write |

---

## Remember

### `remember <kind> "<text>"`

Nine kinds: `decision`, `fact`, `lesson`, `dead-end`, `error-fix`, `convention`, `requirement`,
`task`, `session`.

| Flag | |
|---|---|
| `--about <SYMBOL>` | Anchor it. Repeat for several |
| `--by <user\|agent\|tool>` | Who wrote it. `tool` is treated as untrusted and never corroborates |

### `memories` · `forget <id>` · `reanchor <id>`

| Flag | |
|---|---|
| `--kind <KIND>` · `--stale` · `--limit <N>` | Filters for `memories` |

### `feedback <signal>` · `learn status|why|reset`

Signals: `useful`, `used`, `ignored`, `dead-end`, `corrected`.

| Flag | |
|---|---|
| `--memory <ID>` | Which memory the feedback is about |

---

## Report and connect

### `report`

| Flag | |
|---|---|
| `--as <html\|pdf\|md>` · `--lang <en\|es>` | |
| `-o, --out <FILE>` · `--title <TEXT>` · `--module-depth <N>` | |

### `docs gaps|apply|build`

| Command | Flags |
|---|---|
| `docs gaps` | `--path <PREFIX>` · `--limit <N>` · `--context` |
| `docs apply` | `<FILE>` or `-` for stdin · `--dry-run` |
| `docs build` | `--path <PREFIX>` · `--title <TEXT>` · `-o, --out <FILE>` |

### `stats` · `bench`

| Flag | |
|---|---|
| `--tasks <N>` · `--seed <N>` · `-b <TOKENS>` · `--baseline-files <N>` | For `bench` |

### `serve` · `mcp-config` · `install` · `uninstall` · `doctor`

| Flag | |
|---|---|
| `--agents <ID>` | Repeat for several. **Without it, every agent found** |
| `--scope <user\|project>` · `--dry-run` · `--force` | |
| `--name <NAME>` · `--command <PATH>` · `--pin-repo` | |

### `toon encode|decode` · `completions <shell>`

Shells: `bash`, `zsh`, `fish`, `powershell`, `elvish`.

---

## Exit codes

| | |
|---|---|
| `0` | Success |
| `1` | A failure of the environment: a file, a database, a permission |
| `2` | An invalid or refused request |
| `3` | Something named does not exist |

A hook always exits `0`, whatever happens, because a hook must never fail the tool call it was
attached to.
