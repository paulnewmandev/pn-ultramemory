# How it works

Why a question costs a few hundred tokens instead of sixteen thousand, and what the tool does to get
there.

![The repository is indexed into a graph; a question finds seeds, the graph is walked, candidates are ranked and packed to a budget, and the capsule is measured](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/diagrams/how.svg)

## 1. Indexing

Each file is hashed; only files whose content changed are parsed again. Twelve languages go through
tree-sitter (Rust, Python, JavaScript, TypeScript, TSX, Go, Java, C, C++, C#, Ruby, PHP), which
yields declarations, documentation, signatures and the names each symbol calls, uses or inherits.
Every other language is indexed by a lexical pass that finds declarations without understanding
them: thinner, but nothing in the repository is invisible.

On this repository: a full index from nothing in about 430 ms, about 200 ms after changing one file,
12 ms when nothing changed.

## 2. Resolution: turning names into edges

A reference is a name written inside a symbol, before anyone knows what it means. Resolution links
it to the symbols that carry that name, in files of the same language family, and says how sure it
is:

| Situation | Linked to | Confidence |
|---|---|---|
| A symbol of the same file has the name | up to 4 of them, nearest first | `Resolved` |
| Exactly one symbol in the index has it | that one | `Heuristic`, or `Guess` when the receiver does not point at it |
| Several do, and the receiver points at exactly one | that one | `Heuristic` |
| Several do | up to 4, sharing the longest path prefix | `Guess` |

### The receiver

A method name alone is weak evidence. `$request->validate()` in a Laravel controller is Laravel's,
not your one `CouponService::validate`; `items.is_empty()` is the standard vector's, not your one
`is_empty`. Before this rule, those calls became confident edges, and the "busiest symbols" of a
repository were whatever shared a name with the standard library.

So a reference keeps the last word of its receiver: `couponservice` for `$this->couponService`,
`storage` for `self.storage`, nothing for `self` or `this`, and an empty word for an expression such
as `items()`. The word **points at** a candidate when it names:

1. **its type**: the enclosing class, or the part of a qualified name before the method
   (`Engine` in `Engine::recall`, for a Rust `impl` written in another file), equal or, from four
   letters, the end of a longer name;
2. **or its place**: its file (`utils` for `utils.py`) or one of its directories.

Among several candidates a match by type wins over a match by place, so `engine.recall()` is
`Engine::recall` and not the `mod recall` that lives under `crates/engine/`. A call on a receiver
that points at nothing is a `Guess`, which `impact`, `brief`, `graph` and the brain leave out by
default; `--min-confidence guess` shows it.

## 3. Retrieval is packing, not search

Search ranks. A capsule has to **fit**, and that decides the design.

![Five levels of detail for one symbol, from its name to its whole source, each costing more than the one below](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/diagrams/levels.svg)

```
L0  name           estimate_tokens
L1  signature      fn estimate_tokens(text: &str) -> u32
L2  summary        + its first documentation sentence
L3  outline        + the names it calls
L4  source         + the whole body
```

### Finding candidates

A question first matches symbol names, then every word of the question against names, signatures
and documentation. When that finds little (a question rarely repeats a symbol's documentation word
for word), stop words are dropped, and then one search matches **any** of the words, scored as a
whole so that a word found in eight symbols weighs more than one found in two hundred. The best
matches are expanded along the graph, symbols used together in the past are added, and what was
learned from `feedback` adjusts the order.

### Packing

`recall` chooses one level per candidate to maximise value inside the budget: a multiple-choice
knapsack. Dominated levels are dropped (a concave envelope), a greedy pass takes the most value per
token, and a final pass spends what is left. An LP relaxation bounds the optimum, and the packer is
checked against an exact solver over 3,000 random instances.

### Measuring, then fitting

The capsule is printed, measured, and trimmed by the step that loses least value per token until it
fits; `used` is what the output really costs, not a prediction. The three most relevant symbols
keep at least their signature while anything else can still be given up, because an answer whose
best match is a bare name has not answered. Slack the frame did not need is offered back to the
packer, and a wider capsule is kept only if it still measures inside the budget.

The file table lists only the files a printed row points at. A symbol shown by name only goes to the
`also` list, which carries no path.

## 4. The epistemic envelope

`impact` never says "safe":

| | |
|---|---|
| `exact` | The set is complete |
| `lower-bound` | At least these; there may be more |
| `unknown` | No caller found and the symbol is public, which does **not** mean nobody uses it |

A tool that said "nothing depends on this" because it failed to resolve a dynamic call would be worse
than useless.

## 5. Cost is a contract

That a question costs about the same in a large repository as in a small one is held by **tests**,
not by a benchmark: timings are noisy and a regression hides in their error bars. The suite sweeps
repository size and asserts that `recall`, `map` and `outline` cost the same at every size, each
paired with a control over the same sweep that must **grow**, because a measurement that is always
zero is also flat.

## 6. Nothing leaves

No network code is linked into the binary, and a CI job runs the whole suite inside a network
namespace with no route out, so "offline" is checked rather than claimed. The [brain](Brain) page
runs in your browser under a policy that forbids it any request. Nothing is written inside your
repository unless you ask: `docs apply`, or a `report` written to the current directory.
