# FAQ

## Will it help *my* project?

Probably, if reading files to answer questions is currently expensive for you. The saving is what
reading whole files would have cost, so:

| | |
|---|---|
| **Most help** | Large codebases, deep call graphs, unfamiliar code, many files per question |
| **Less help** | A handful of scripts, a small site, a project you already hold in your head |

Do not take the published numbers as yours. Measure:

```bash
pn-ultramemory bench
```

That runs on **your** repository and prints your hit rate and your token cost against the baseline.
It takes a minute or two.

## Which languages are parsed properly?

Twelve go through tree-sitter, with full symbol and reference extraction:

Rust · Python · JavaScript · TypeScript · TSX · Go · Java · C · C++ · C# · Ruby · PHP

**Every other language is still indexed**, by a lexical pass that finds declarations without
understanding them. So nothing in your repository is invisible, but for those files the graph is
thinner: you get symbols and text search, with fewer resolved edges.

## Is my code sent anywhere?

No. There is no network code in the binary — not a client, not a symbol. A CI job runs the whole
test suite inside a network namespace with no route out, so this is checked on every change rather
than promised.

No account, no telemetry, no usage reporting.

## Does it write in my repository?

No, with one exception you ask for: `docs apply`, which inserts documentation you wrote into your
source files. Everything else — the index, the memories, the usage counters — lives in your user
data directory, one database per project.

## How mature is it?

New. One author, days old, no external users yet. The version is 1.0.0 because it is the first
release, not because it has been through a long history.

What that means practically: the behaviour is covered by more than 1,300 tests and every gate is
green on macOS, Linux and Windows, but it has not been used by many people on many codebases, which
is a different kind of confidence. Treat it as something to try.

## How is this different from an embedding search over my code?

It is not a search. Nothing is embedded and nothing is scored by similarity to your question in
vector space. It parses your code into symbols and real relationships, then **packs** an answer to
fit a budget. The difference shows up in three places:

- It knows what calls what, so `impact` can answer "what breaks" rather than "what looks similar".
- It knows how sure it is about each edge, and says so.
- It fits a budget exactly rather than returning the top *k* of something.

## Why TOON and not JSON?

TOON prints uniform lists as tables with the columns named once, so a capsule of forty symbols is
about 20% cheaper than compact JSON and far cheaper than indented JSON. `-f json` is there whenever
a program is reading.

## Can I use it without an agent?

Yes. Everything works from the command line: `recall`, `outline`, `impact`, `map`, `graph`,
`report`. The MCP server is one way in, not the only one.

## What does it cost?

Nothing, in every sense. Apache-2.0, no paid tier, no account, nothing to upgrade to. You may use
it, change it, sell it and fork it.

## Does `install` really write to every agent?

Yes, if you give it no `--agents`. That is rarely what you want:

```bash
pn-ultramemory install --agents cursor     # just yours
pn-ultramemory install --dry-run           # see what it would touch, change nothing
```

It writes only its own entry, and `uninstall` removes exactly that entry — a round trip leaves the
file byte for byte as it was, which is what sixty tests check.

## What is the epistemic envelope for?

So `impact` can never mislead you. It reports `exact` when the set is complete, `lower-bound` when
there may be more, and `unknown` when it found no caller and the symbol is public. A tool that said
"nothing depends on this" because it failed to resolve a dynamic call would be worse than useless.

## Can I trust the memories it stores?

Trust them as much as you trust whoever wrote them — the tool never decides a memory is true. What
it does guarantee is narrower and more useful: a memory that repeats does not become more true, a
memory whose code changed is marked stale, and a memory that contradicts another is reported rather
than merged. See [Memories](Memories).
