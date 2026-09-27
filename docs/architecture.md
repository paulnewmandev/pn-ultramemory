<!-- SPDX-License-Identifier: Apache-2.0 -->
# Architecture

How pn-ultramemory is put together, and why each boundary is where it is.

This describes what is **built**. Nothing here is design intent: every crate, port and flow below
exists in the tree and is covered by the test suite.

---

## The problem

An agent answering a question about a codebase has two bad options. It can read whole files, which
is complete and costs every token in them. Or it can grep, which is cheap and returns fragments
that never say what else is there.

The approach: parse the repository once into a graph of symbols and relationships, keep it in a
local database, and answer a question by **choosing how much of each relevant symbol to show** so
that the answer fits a token budget the caller sets.

Three decisions follow from that and shape everything else.

1. **Retrieval is a packing problem, not a search problem.** Search ranks; this has to *fit*. That
   is why the codec crate exists and why its core is a knapsack rather than a scorer.
2. **Confidence is part of the data.** A reference resolved through an import is not the same claim
   as a name that happens to match, so every edge carries which it was, and every answer that walks
   edges reports what it could not establish.
3. **The tool is local and offline.** No network call exists in the tree, which removes a whole
   class of design questions and is checked by a CI job that runs the suite inside a network
   namespace with no route out.

---

## Hexagonal structure

The centre knows nothing about SQLite, tree-sitter or the file system. It defines what it needs as
traits; the adapters implement them; the binary wires them together at startup.

```
          ┌──────────┐   ┌──────────┐   ┌──────────┐
 entry    │   cli    │   │   mcp    │   │  report  │
          └────┬─────┘   └────┬─────┘   └────┬─────┘
               └──────────────┼──────────────┘
                         ┌────▼─────┐
 use cases               │  engine  │
                         └────┬─────┘
                              │  depends only on the ports below
                         ┌────▼─────┐
 contracts               │   core   │  Storage · Extractor · SourceTree
                         └────┬─────┘  DocInserter · Clock
               ┌──────────────┼──────────────┐
          ┌────▼─────┐   ┌────▼─────┐   ┌────▼─────┐
 adapters │  store   │   │  index   │   │  codec   │
          │ (SQLite) │   │(tree-sit)│   │  (TOON)  │
          └──────────┘   └──────────┘   └──────────┘
```

**The rule, stated once:** `core` has no I/O and depends on no other crate in this workspace. The
`headers` guard checks that every file declares its layer in its module documentation, and the
dependency direction is enforced by the crate graph itself — `core` lists no workspace dependency,
so a violation does not compile.

Swapping SQLite for another store means implementing one trait. Adding a transport beside MCP means
adding one entry adapter. Neither touches the engine.

### The five ports

| Port | What it abstracts | The adapter that implements it |
|---|---|---|
| `Storage` | Symbols, edges, memories, files, usage counters | `pn-ultramemory-store` (SQLite, WAL, FTS5) |
| `Extractor` | Turning source text into symbols and references | `pn-ultramemory-index` (tree-sitter + fallback) |
| `SourceTree` | Listing and reading files | `pn-ultramemory-index` (the file system) |
| `DocInserter` | Where a documentation comment goes in a given language | `pn-ultramemory-index` |
| `Clock` | The current time | the binary (and a fixed clock in tests) |

`Clock` is a port for one reason: staleness and the decay of learned utility both depend on time, so
a test that cannot control time cannot check them.

---

## The crates

| Crate | Layer | Lines | What it holds |
|---|---|---:|---|
| `pn-ultramemory-core` | Domain | 2,319 | The vocabulary (`Confidence`, `Detail`, `MemoryKind`, `Provenance`, `Language`) and the five ports. No I/O, no workspace dependency. |
| `pn-ultramemory-codec` | Use case | 1,743 | Token estimation and the multi-resolution packer. |
| `pn-ultramemory-toon` | Use case | 5,435 | TOON 4.1 encode and decode. Passes all 538 official conformance fixtures. |
| `pn-ultramemory-index` | Exit adapter | 13,012 | tree-sitter for twelve languages, a lexical fallback for every other, and the file system. |
| `pn-ultramemory-store` | Exit adapter | 10,802 | SQLite: schema, migrations, bulk insert, edge resolution, full-text search. |
| `pn-ultramemory-engine` | Use case | 18,793 | Every operation: index, recall, outline, expand, impact, graph, map, memory, learning, docs, stats, bench. |
| `pn-ultramemory-mcp` | Entry adapter | 5,679 | Model Context Protocol over stdio, five revisions. |
| `pn-ultramemory-report` | Entry adapter | 12,421 | HTML, PDF and Markdown. **Zero dependencies**, including on this workspace. |
| `pn-ultramemory` | Entry adapter | 8,034 | The command line, hooks, colour, the progress bar and the agent installer. |
| `xtask` | Tooling | 3,850 | The four ratchets and the release-blocker check. |

`report` having no dependency at all — not even on `core` — is deliberate. A renderer that cannot
reach a database or a parser cannot leak either into a file someone is about to send to a colleague.
The cost is that nothing converts an `Insights` into a `ReportData` by itself, so the CLI does it,
in one module that is the only place the two vocabularies meet.

---

## The graph

Three kinds of node, and edges that carry how sure the indexer was.

| Node | Comes from |
|---|---|
| **File** | The source tree: path, language, size, line count |
| **Symbol** | The extractor: name, qualified name, kind, signature, documentation, visibility, span, parent, outline, and two hashes — one of the signature, one of the body |
| **Memory** | A person or an agent: kind, text, provenance, and the anchors tying it to symbols |

Edges between symbols carry a `Confidence`:

| | Means |
|---|---|
| `Exact` | Stated directly by the syntax |
| `Resolved` | Resolved through scopes or imports |
| `Heuristic` | A structural hint, such as a name unique in the repository |
| `Guess` | A name match with nothing else behind it |

`Exact` and `Resolved` are **structural**; the other two are not. Any operation that walks edges
reports which kinds it followed, so a caller can tell a fact from an inference.

Two hashes per symbol, not one: a change to a signature is a change to the contract, and a change to
a body is not. A memory anchored to a symbol goes stale when either moves, and the pair is what
lets a future version distinguish the two.

---

## Five levels of detail

```
L0  Name        the name alone
L1  Signature   + how it is called
L2  Summary     + the first sentence of its documentation
L3  Outline     + the names it calls
L4  Source      + the whole body
```

The levels are a total order and every level includes the one below it. That is what makes packing
tractable: a symbol's cost is monotone in its level, so the choice is which rung to stop at.

---

## Packing

`recall` solves a **multi-resolution knapsack**. Each candidate has one option per level, with a
token cost and a utility; the packer chooses one option per candidate to maximise total utility
within the budget.

It runs in three passes:

1. **Concave envelope.** For each candidate, discard any level that is dominated — more expensive
   and no more useful than another. What remains is a concave frontier, which lets a greedy pass
   over marginal utility per token be near-optimal.
2. **Greedy fill** over a max-heap of marginal gains.
3. **Leftover pass** that spends what rounding left behind.

An LP relaxation gives an upper bound, so every packing reports its own `optimality_gap()`. The
result is checked against an exact dynamic-programming solver over 3,000 random instances.

### Then it is measured, not estimated

A capsule prints its own `used`, which is part of what it costs — a fixed point. So the assembled
capsule is rendered, measured, and if it is over budget the least valuable step is taken back and it
is measured again. `used` is therefore what the output actually costs, not a prediction of it.

### Giving back the slack

The frame around the symbols — the query echo, the file table, the headers — has to be reserved
before anything is packed, and that estimate is deliberately generous, because under-estimating it
produces a capsule that has to be taken apart again.

The cost is that capsules came back well under budget. On this repository a budget of 200 returned
**nothing at all** while 174 of those tokens went unspent. So once a capsule is known to fit, the
gap is handed back to the packer and the whole capsule is built again, keeping the wider result only
when it still measures inside the budget. A round that does not fit halves the amount offered rather
than giving up, because the frame is not a constant: showing the first symbol brings in the file
table, so it costs far more than the second.

---

## The flow of a recall

```
question
  │
  ├─ seeds      exact name matches, then full-text, then a relaxed search
  │             when the first two found too little
  ├─ expand     the best seeds' neighbours through the graph, and the
  │             symbols that were used together with them before
  ├─ rank       relevance × a learned multiplier, clamped to [0.5, 1.5]
  ├─ prepare    read each candidate's source once, build its five options
  ├─ memories   anchored to the candidates, or matching the text, within
  │             a quarter of the budget
  ├─ pack       the knapsack above
  ├─ compose    files, symbols, relations, memories, notes
  └─ settle     render, measure, shrink or refill until it fits
```

The learned multiplier can only **re-rank**. It cannot introduce a symbol the search did not find,
and it cannot remove one, so a corrupted learning state degrades the order of an answer and never
its contents.

---

## Memory

A memory is anchored to the symbols it describes. The anchor stores the symbol's qualified name, its
path, and the two hashes. When a file is re-indexed, any memory whose anchored symbol changed is
marked **stale** — shown with a mark, never deleted and never guessed at. A person decides that a
stale memory is still true.

### The rule the whole subsystem exists to hold

> **Corroboration raises how likely a memory is to be RETRIEVED. It never raises how likely it is
> to be TRUE.**

Retrieval priority and truth are different quantities and are never mixed. Without this, an agent
repeating its own mistake in a loop would manufacture a fact, and the tool would serve that fact
with confidence to every later session.

So a repetition only counts when it is an independent observation: two corroborations of the same
memory **within fifteen minutes count as one**, whoever sent them; text captured from a tool never
corroborates at all; and no memory gathers more than eight corroborations.

### Contradiction is checked before similarity

Negating a sentence changes almost none of its words, so a denial scores as highly similar. On this
repository, a real contradiction scored **0.838** while a real agreement scored **0.844** — the
contradiction looked *less* similar than the agreement. No text measure can separate them. A
structural check can, and without it a memory could be reinforced by its own opposite.

A candidate that contradicts is therefore disqualified from ever counting as the same memory, and
both memories are kept with the conflict reported. See [memory.md](memory.md).

---

## Learning

Three signals, each recorded against a symbol or a memory:

| | |
|---|---|
| **Shown** | It was included in an answer |
| **Used** | The caller said it helped (`feedback used`, `useful`) |
| **Ignored** | The caller said it did not (`ignored`, `dead-end`, `corrected`) |

Utility is a Beta posterior with exponential decay, so old evidence fades rather than accumulating
forever. It produces a multiplier clamped to `[0.5, 1.5]`. Symbols retrieved together build
co-access edges, which seed the expansion step of a later recall.

**Nothing learned can create a fact.** The whole channel affects ranking and nothing else.

---

## Storage

SQLite, one file, in your user data directory — never inside your repository.

| | |
|---|---|
| **WAL** | Readers do not block the writer, so `serve` and a terminal can share a database |
| **FTS5** | Full-text search over names, signatures and documentation, in the same transaction as the data |
| **Bulk insert** | 48,000 symbols per second, by batching and preparing statements once |
| **Incremental** | A file whose content hash is unchanged is not re-parsed. Re-indexing this repository with nothing changed takes **13 ms** |
| **Scoped resolution** | Changing one file re-resolves only the edges that could have moved, not the whole graph |

---

## Cost is a contract

The central claim — that a question costs about the same in a large repository as in a small one —
is held by tests, not by a benchmark.

A timing benchmark cannot hold it: it is noisy, machine-dependent, and a regression hides inside its
error bars. So `crates/engine/tests/cost.rs` sweeps the repository size, measures a number the tool
already reports, and asserts that it is **flat** in that size.

Each flat claim is paired with a control over the same sweep that must **grow**. A flat measurement
on its own proves nothing — a number that is always zero is also flat — so without the control, a
test that stopped measuring anything would pass quietly.

---

## Invariants

| | |
|---|---|
| A capsule never exceeds its budget | unless the budget is smaller than an empty capsule, which it says |
| An outline never drops a symbol | the detail falls instead; it says when it went over |
| `impact` never says "safe" | it says `exact`, `lower-bound` or `unknown` |
| Nothing is written inside your repository | except `docs apply`, which you asked for |
| No network call exists | checked in CI inside a network namespace |
| Library code does not panic | held by the `panics` ratchet |
| Every error names a way forward | held by the `refusal` ratchet |
| The same input gives the same bytes | for every rendered output |

---

## Testing

| Kind | What it covers |
|---|---|
| **Unit** | Each module's own rules, beside the code |
| **Integration** | Each operation end to end against the real SQLite and tree-sitter adapters |
| **Property** | The packer against an exact solver; the pre-filter against the full score; budgets over thousands of generated values |
| **Conformance** | All 538 official TOON fixtures |
| **Cost-class** | Flat-and-grows assertions over a swept repository size |
| **Hostile** | 200+ malformed MCP inputs; secrets, control characters and huge inputs through the memory path |
| **Ratchets** | Four baselines that may only shrink; an entry matching nothing fails the build |

See [quality.md](quality.md) for what each guard does **not** prove.
