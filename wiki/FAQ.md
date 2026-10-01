# FAQ

## Will it help *my* project?

If reading files to answer questions is expensive for you, probably. What it saves is what reading
whole files would have cost:

| | |
|---|---|
| **Most help** | Large codebases, deep call graphs, unfamiliar code, questions that touch many files |
| **Less help** | A handful of scripts, a small site, a project you already hold in your head |

Do not take the published numbers as yours. Measure:

```bash
pn-ultramemory bench
```

It runs on **your** repository and prints your hit rate and token cost against reading files.

## How good is it at plain questions?

Good when the question uses the code's own words, weaker when it shares none. The search is over
words: nothing is embedded. "validate coupon discount on order" finds `CouponService::validate`
first.

A question in Spanish about code written in English gets help from a built-in glossary of about
180 programming and business words: "¿cómo se estima el número de tokens?" finds
`estimate_tokens`, "aplicar un descuento al pedido" finds `applyDiscount`. The question keeps its
own words too, so code documented in Spanish is found as before. Words outside the glossary, and
other languages, are searched as written; a question that reads as English is never translated.

`bench` builds its questions from each symbol's own documentation, so its numbers measure finding a
known thing, not answering any question, and it says so in its output.

## Which languages are parsed properly?

Twelve go through tree-sitter, with symbols, documentation and references:

Rust · Python · JavaScript · TypeScript · TSX · Go · Java · C · C++ · C# · Ruby · PHP

**Every other language is still indexed** by a lexical pass that finds declarations without
understanding them. Nothing is invisible, but for those files the graph is thinner: symbols and text
search, fewer resolved edges.

## Why did some calls disappear from `impact` after upgrading?

They were wrong. A call such as `$request->validate()` used to count as a call to the only `validate`
your code declares, because only the method's name was read. Calls are now read with their receiver,
and one whose receiver does not point at the candidate is only a `Guess`, which `impact` leaves out
by default. `--min-confidence guess` shows them again. See [How it works](How-it-works#the-receiver).

## What is the brain for, if my agent has the tools?

For you. The agent has `brief` and `recall`, which cost a few hundred tokens. A person needs to see
the shape of the code, find their way around an unfamiliar repository, read what was decided, and
point the agent at the right place: open a particle and *Copy context for an agent*. See
[The brain](Brain).

## Is my code sent anywhere?

No. There is no network code in the binary. A CI job runs the whole test suite in a network
namespace with no route out, so this is checked on every change rather than promised. The brain page
carries a security policy that forbids it any request. No account, no telemetry.

## Does it write in my repository?

Not unless you ask. The index, the memories and the counters live in your user data directory, and
so does the brain page. `docs apply` writes documentation you supplied into your sources; `report`
writes its file to the current directory unless you pass `-o`.

## How is this different from embedding search over my code?

It is not a similarity search. It parses the code into symbols and real relationships, then
**packs** an answer to fit a budget:

- It knows what calls what, so `impact` answers "what breaks" rather than "what looks similar".
- It knows how sure it is about each edge, and says so.
- It fits a budget exactly instead of returning the top *k* of something.

The price is that it does not understand meaning, so a question has to share words with the code.

## Why TOON and not JSON?

TOON prints uniform lists as tables with the column names written once, so a capsule of forty
symbols costs about 20% less than compact JSON. `-f json` is there for programs.

## Can I use it without an agent?

Yes. `recall`, `outline`, `impact`, `map`, `brain`, `graph` and `report` all work from the command
line. The MCP server is one way in, not the only one.

## Does `install` really write to every agent?

Only if you give it no `--agents`, which is rarely what you want:

```bash
pn-ultramemory install --agents cursor     # just yours
pn-ultramemory install --dry-run           # see what it would touch, change nothing
```

It writes only its own entry, and `uninstall` removes exactly that entry, leaving the file byte for
byte as it was.

## Can I trust the memories it stores?

As much as you trust whoever wrote them; the tool never decides a memory is true. What it guarantees
is narrower and more useful: a memory that repeats does not become more true, a memory whose code
changed is marked stale, and a memory that contradicts another is reported rather than merged. See
[Memories](Memories).

## How mature is it?

New: one author and few users so far. The behaviour is covered by more than 1,300 tests and every
gate passes on macOS, Linux and Windows, but it has not met many codebases yet, which is a different
kind of confidence. Try it, measure it, and report what breaks.

## What does it cost?

Nothing. Apache-2.0, no paid tier, no account. Use it, change it, sell it, fork it.
