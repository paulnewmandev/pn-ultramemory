# The brain

```bash
pn-ultramemory brain              # --lang es for Spanish
```

The whole repository as one scene you fly through. It is the second brain a person reads, next to
the one the agent queries: the same index and the same memories, drawn.

![One symbol opened: its fibres lit, pulses travelling along them, and a panel with its signature, documentation, a memory, its callers and its callees](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/screenshots/brain-symbol.jpg)

## Reading the picture

| You see | It is |
|---|---|
| A particle | One symbol: a function, a method, a class, a type, a constant |
| Its size | How connected it is; types are a little larger |
| Its colour | Its region: the first two folders of its path |
| A region of the cortex | One folder, placed in mirrored pairs over the two hemispheres, the largest first |
| The cerebellum | The tests |
| A fibre | A relationship, curving under the cortex; it fades from the caller to the callee |
| A pulse | A signal running along a fibre, from caller to callee |
| A golden ring | A memory, floating above the code it is anchored to |
| A red ring | A stale memory: the code it describes changed since it was written |
| The dust on the surface | The shape of a brain, drawn even when a repository is small |

## Using it

| Do | To |
|---|---|
| Drag | Turn it |
| Right-drag, or Shift and drag | Move it |
| Scroll, or pinch | Zoom |
| `/` or Ctrl/Cmd+K | Search symbols, paths, documentation and memories |
| Click a particle | Open it: signature, documentation, memories, what calls it, what it calls |
| Click a name in the panel | Go to that symbol |
| Double-click, or `f` | Fly to it |
| Click a region | Light only that folder |
| `Esc` | Close, then clear, then fly home |
| `r` | Stop or start the slow rotation |

The address bar keeps the selection (`#s=<id>`), so reloading the page reopens what you were
reading. The switches on the left turn fibres, signals, memories, the cortex, labels and rotation
on and off.

## Handing it to an agent

**Copy context for an agent** puts this on the clipboard:

```
# Engine::recall (method)
crates/engine/src/recall.rs:200

pub fn recall(&self, query: &RecallQuery) -> Result<Capsule, EngineError>

Answers a query with a capsule: the symbols and memories that matter for it, …

called by (2): run_code (crates/cli/src/main.rs:236), …
calls (13): Result (crates/store/src/error.rs:17), EngineError (…), …

Memories:
- [lesson] The three most relevant symbols keep their signature while anything else can still be lowered

next: pn-ultramemory expand <id> · pn-ultramemory impact <id>
```

A few hundred tokens that tell a model where the thing is, what it touches, and what was already
decided about it, before it reads a single file.

## What it draws, and what it leaves out

Every symbol of the index up to `--max-nodes` (6,000 by default). Beyond that, the symbols other code
depends on most are kept, then the public ones; the page says how many were left out. Module
declarations are never drawn, because the file is already there through its symbols.

Fibres are the edges at `--min-confidence` and above, `heuristic` by default, so a call that only
matches a name (`$request->validate()` and your one `validate`) is not drawn. Pass
`--min-confidence guess` to see those too.

## Where it lives, and what it may do

`brain` writes one HTML file, `brain.html` in the data directory unless you pass `-o`, and opens it
in your browser when you are at a terminal (`--no-open` only writes it). Nothing is written inside
your repository.

The page is plain WebGL with no library, and it carries a Content-Security-Policy that names no
source for connections, images, fonts or frames. The browser itself refuses any request the page
could make, so nothing it shows can leave your machine. Every string from your code is escaped
before it is embedded and written with `textContent`, never parsed as HTML.

It generates in about a quarter of a second for 5,000 symbols, and the file is about a megabyte.
The page needs WebGL; if a browser has it turned off, it says so.
