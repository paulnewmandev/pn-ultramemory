# The MCP tools

Nine tools reach your coding agent over the Model Context Protocol. This page says what each one
answers, when to reach for it, and what it costs.

---

![Nine tools in three groups: four for reading the code, two for surveying it, three for writing to the memory](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/diagrams/tools.svg)

## Why there is a `brief` at all

A model's memory ends with its window. Close the tab, switch tools, hit a context limit, and
everything it learned about your codebase is gone — including the hour you spent explaining it.

The graph on disk does not end, and neither do the memories anchored into it. `brief` is how a new
session picks both back up in one call.

```
brief:
  repo: your-project
  files: 268
  symbols: 4924
  edges: 22580
  languages: rust 248, sql 4, cpp 1, typescript 1
  memories: 2
modules[11]{name,files,symbols,in,out}:
  crates/engine,57,1109,561,614
  crates/index,51,893,9,354
  ...
central[10]{name,kind,path,line,callers}:
  Result,type,crates/store/src/error.rs,17,473
  ...
known[2]{id,kind,stale,text}:
  1,decision,false,"Money is stored as integer minor units, never floating point"
next[4]: recall "<question>" -b 800, outline <path>, impact <symbol>, memories --stale
```

It is budgeted like everything else. Sections are printed in the order a reader needs them —
structure, then the busiest symbols, then what is already decided — and **given up in a different
order**: the busiest symbols go first, because reading the code finds them again, and what someone
already decided goes last, because nothing else can recover it.

---

![A session with no memory calls brief to learn the project, recall and outline to work, remember to keep a decision, and feedback to say what helped](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/diagrams/session.svg)

## All nine

| Tool | Ask it | Instead of | Writes |
|---|---|---|---|
| `brief` | What is this project | Reading a README and guessing | |
| `recall` | Which code matters for this question | Reading several files | |
| `outline` | What is in this file | Reading the whole file | |
| `expand` | The exact source of one symbol | Reading around it | |
| `impact` | What breaks if I change this | Grepping for the name | |
| `map` | Which files exist and what is in them | Listing the tree | |
| `remember` | Keep this decision, anchored to the code | A comment nobody reads | yes |
| `memories` | What do we already know; what went stale | Asking again | |
| `feedback` | That answer helped, or did not | Nothing | yes |

### `brief`

| Argument | |
|---|---|
| `budget` | Tokens. Default 1200 |

### `recall`

| Argument | |
|---|---|
| `q` | **Required.** A question, a symbol or a path |
| `budget` | Tokens |
| `explain` | Add why each symbol is in the answer |

### `outline`

| Argument | |
|---|---|
| `path` | **Required.** A file in the repository |
| `budget` | Tokens. The detail falls to fit; symbols are never dropped |

### `expand`

| Argument | |
|---|---|
| `id` | **Required.** A node id from an earlier result |
| `from` · `to` | The window, counted from the symbol's first line |

### `impact`

| Argument | |
|---|---|
| `symbol` | **Required.** A qualified symbol name |
| `depth` | Hops of callers to follow |

Read the `epistemic` field before the list: `exact`, `lower-bound` or `unknown`. It never says
"safe".

### `map`

| Argument | |
|---|---|
| `budget` | Tokens |
| `path` | Only files whose path starts with this |

### `remember`

| Argument | |
|---|---|
| `kind` | **Required.** One of the nine kinds |
| `text` | **Required.** The memory itself |
| `about` | Symbols it concerns |

### `memories`

| Argument | |
|---|---|
| `kind` | One kind only |
| `stale` | Only memories whose anchored code changed |
| `limit` | How many at most |

### `feedback`

| Argument | |
|---|---|
| `signal` | **Required.** `used`, `useful`, `ignored`, `dead_end` or `corrected` |
| `symbol` **or** `memory` | **One of the two.** What the feedback is about |

This is the one that closes the loop. Until it existed the learning subsystem could only be fed from
the command line, while the thing actually calling `recall` was the agent — so in practice nothing
was ever learned. It returns the updated utility, so you can watch the multiplier move.

---

## What the list costs

**3,995 bytes**, read once per session — roughly a thousand tokens, or two `recall` calls. That
is the array as it goes out on the wire, not a re-serialisation of it, which is a different and
smaller number.

That number is a test, not a note. A tool that cannot pay for its own description every session does
not belong in the list, and the ceiling in `crates/mcp/tests/stateless.rs` fails the build when the
payload grows past it. Raising it is a decision someone has to make deliberately.

---

## A session that works well

1. **`brief`** once, at the start, if you do not know the project.
2. **`recall`** with a budget for each question. `-b 800` is a good default.
3. **`outline`** before changing a file you have not read.
4. **`impact`** before changing anything other code calls.
5. **`expand`** only when you need the exact lines.
6. **`remember`** when a decision is made, anchored with `about`.
7. **`feedback`** when something helped or did not.

Steps 6 and 7 are what make the next session better than this one. Skipping them still works; it
just means every session starts as ignorant as this one did.

---

## What no tool does

There is no `forget` and no `reanchor` over MCP, and that is deliberate. A memory going stale means
*the code changed*, not *the memory is wrong* — deciding which is a person's call. Both live on the
command line, where a person is already standing:

```bash
pn-ultramemory memories --stale
pn-ultramemory reanchor <id>     # you checked: still true
pn-ultramemory forget <id>       # no longer true
```
