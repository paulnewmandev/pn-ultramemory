# The MCP tools

Nine tools reach your coding agent over the Model Context Protocol. This page says what each one
answers, when to reach for it, and what it takes.

![Nine tools in three groups: four read the code, two survey it, three write to the memory](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/diagrams/tools.svg)

## The nine

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

The whole list is **3,995 bytes** on the wire, about a thousand tokens, read once per session. A
test fails the build if it grows past 4,200: a tool that cannot pay for its own description every
session does not belong in the list.

## A session that works

![A session with no memory calls brief to learn the project, recall and outline to work, remember to keep a decision, and feedback to say what helped](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/diagrams/session.svg)

1. **`brief`** once at the start, when the project is not already known.
2. **`recall`** with a budget for each question; 800 is a good default. Use the code's own words.
3. **`outline`** before changing a file you have not read.
4. **`impact`** before changing anything other code calls.
5. **`expand`** only when the exact lines are needed.
6. **`remember`** when something is decided, anchored with `about`.
7. **`feedback`** when a result helped or misled.

Steps 6 and 7 are what make the next session better than this one. Without them every session
starts as ignorant as this one did.

## Arguments

### `brief`

| Argument | |
|---|---|
| `budget` | Tokens. Default 1,200 |

The shape of the repository, its modules with their coupling, its most-called symbols and every
memory on record, with the next commands to run. Sections are printed in the order a reader needs
them and given up in a different one: the busiest symbols go first under pressure, because reading
the code finds them again, and recorded decisions go last, because nothing else can recover them.

### `recall`

| Argument | |
|---|---|
| `q` | **Required.** A question, a symbol name or a path |
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
| `id` | **Required.** A symbol id from an earlier result, or a name |
| `from` · `to` | The window of lines |

### `impact`

| Argument | |
|---|---|
| `symbol` | **Required.** A symbol name |
| `depth` | Steps of callers to follow |

Read `epistemic` before the list: `exact`, `lower-bound` or `unknown`. It never says "safe".

### `map`

| Argument | |
|---|---|
| `budget` | Tokens |
| `path` | Only files under this prefix |

### `remember`

| Argument | |
|---|---|
| `kind` | **Required.** `decision`, `fact`, `lesson`, `dead-end`, `error-fix`, `convention`, `requirement`, `task` or `session` |
| `text` | **Required.** The memory itself, written as a statement about the code |
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
| `symbol` **or** `memory` | **One of the two.** What it is about |

It returns the updated utility, so the effect is visible. This is the only thing that feeds the
learning: without it, ranking never improves from use.

## The hooks

`install` also registers two hooks with agents that support them. Each answers with one line of
context or with nothing, never blocks a tool call, and always exits successfully:

| Event | What the agent is told |
|---|---|
| Session start | The repository is indexed (or how to index it), how large the index is, and that `brief` orients a new session |
| Before a tool call | When it is about to search the whole repository or read a very large file, at most once per session: a `recall` would cost fewer tokens |

`PN_ULTRAMEMORY_NO_HOOKS=1` turns them off.

## What no tool does

There is no `forget` and no `reanchor` over MCP, deliberately. A stale memory means *the code
changed*, not *the memory is wrong*, and deciding which is a person's call. Both live on the command
line:

```bash
pn-ultramemory memories --stale
pn-ultramemory reanchor <id>     # you checked: still true
pn-ultramemory forget <id>       # no longer true
```

The [brain](Brain) is for people too: an agent has `recall` and `brief`, which cost a few hundred
tokens; a 3D scene would cost it nothing useful.
