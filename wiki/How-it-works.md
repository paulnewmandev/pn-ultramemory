# How it works

Why a question costs 413 tokens instead of 16,109, and what the tool is doing to get there.

## Retrieval is a packing problem, not a search problem

Search ranks. This has to **fit**. That difference decides the whole design.

Every symbol can be shown at one of five levels, each including the one below it:

```
L0  name           estimate_tokens
L1  signature      fn estimate_tokens(text: &str) -> u32
L2  summary        + its first documentation sentence
L3  outline        + the names it calls
L4  source         + the whole body
```

`recall` solves a **multi-resolution knapsack**: choose one level per candidate to maximise
usefulness inside the budget. Three passes:

1. **Concave envelope.** Discard any level that is dominated — more expensive and no more useful
   than another. What remains is a frontier where a greedy pass is near-optimal.
2. **Greedy fill** over marginal usefulness per token.
3. **Leftover pass** that spends what rounding left behind.

An LP relaxation gives an upper bound, so every packing reports its own optimality gap. The result
is checked against an exact dynamic-programming solver over 3,000 random instances.

### Then it is measured, not estimated

A capsule prints its own `used`, which is part of what it costs — a fixed point. So the capsule is
rendered, measured, and if it is over budget the least valuable step is taken back and it is
measured again. `used` is what the output actually costs, not a prediction.

### Giving the slack back

The frame around the symbols — the query echo, the file table, the headers — has to be reserved
before anything is packed, and that estimate is generous on purpose: under-estimating it produces a
capsule that must be taken apart again.

The cost was that capsules came back well under budget. A budget of 200 returned **nothing at all**
while 174 of those tokens went unspent. So once a capsule fits, the gap is handed back to the packer
and the whole capsule is built again, keeping the wider result only when it still measures inside
the budget. A round that does not fit halves the amount offered rather than giving up, because the
frame is not a constant: the first symbol brings in the file table, so it costs far more than the
second.

## The graph

Every edge carries how sure the indexer was:

| | Means |
|---|---|
| `Exact` | Stated directly by the syntax |
| `Resolved` | Resolved through scopes or imports |
| `Heuristic` | A structural hint, such as a name unique in the repository |
| `Guess` | A name match with nothing else behind it |

`Exact` and `Resolved` are structural; the others are not. Any operation that walks edges reports
which kinds it followed.

### The epistemic envelope

`impact` never says "safe". It says one of three things:

| | |
|---|---|
| `exact` | The set is complete |
| `lower-bound` | At least these; there may be more |
| `unknown` | No caller found and the symbol is public — which does **not** mean nobody uses it |

That third answer is the point. A tool that said "nothing depends on this" because it failed to
resolve a dynamic call would be worse than useless.

## Indexing

Only files whose content hash changed are re-parsed, and changing one file re-resolves only the
edges that could have moved. Re-indexing this repository with nothing changed takes **13 ms**.

Twelve languages go through tree-sitter: Rust, Python, JavaScript, TypeScript, TSX, Go, Java, C,
C++, C#, Ruby, PHP. Every other language is still indexed, by a lexical pass that finds declarations
without understanding them — thinner, but nothing in your repository is invisible.

## Cost is a contract

The claim that a question costs about the same in a large repository as in a small one is held by
**tests**, not by a benchmark. A timing benchmark cannot hold it: it is noisy, machine-dependent,
and a regression hides inside its error bars.

So the suite sweeps repository size and asserts that `recall`, `map` and `outline` cost the same at
every size. Each flat claim is paired with a control over the same sweep that must **grow** — a flat
measurement on its own proves nothing, because a number that is always zero is also flat.

## Where nothing goes

No network symbol is linked into the binary: it cannot open a socket. A CI job runs the whole suite
inside a network namespace with no route out, so "offline" is checked rather than claimed. Nothing
is written inside your repository except by `docs apply`, which you asked for.
