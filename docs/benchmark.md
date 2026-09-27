# The offline benchmark

`pn-ultramemory bench` measures two things on **your** repository: whether a capsule contains the
code that was asked for, and what that capsule costs in tokens compared with the crudest thing an
agent can do without an index — reading whole files. It needs no network, no API key and no
language model, so anyone can rerun it and get the same numbers.

**Contents**

1. [What it measures](#what-it-measures)
2. [The command](#the-command)
3. [Reading the output](#reading-the-output)
4. [Why there is no single score](#why-there-is-no-single-score)
5. [What each derivation means](#what-each-derivation-means)
6. [What this does not measure](#what-this-does-not-measure)

## What it measures

A **task** is one documented symbol taken from your own index: a function, method, class, struct or
interface whose documentation has at least five words. A seeded generator draws a sample of them, so
`--seed 1` always draws the same sample from the same index.

Each task yields two queries, and so two **families**:

| Family | The query is | Why it exists |
|---|---|---|
| `description` | the first sentence of the symbol's documentation, with every word that also appears in the symbol's own name removed | asks for code by what it does, without handing over its name |
| `name` | the symbol's name split into lowercase words (`parseHTTPConfig` becomes `parse http config`) | the easiest question there is: a half-remembered identifier |

The removal in the `description` family is what keeps the measurement honest. Without it the query
would contain the answer and the result would say more about string matching than about retrieval. A
word counts as belonging to the name when it is one of its words, or when one of the two is a prefix
of the other and the shorter has at least three characters, so `parses` does not survive the name
`parse` and `configuration` does not survive `config`.

For every task and every budget the ordinary `recall` path runs, with explanations off:

- a **hit** is the wanted symbol appearing in the capsule;
- the **cost** is the capsule's own `used` count.

The **baseline arm** runs once per task, because reading files has no budget. It scores every indexed
file by how many *distinct* words of the query it contains, case-insensitively, reads the top
`--baseline-files` of them in full, and reports a hit when the file that declares the wanted symbol is
among them. Its cost is the estimated tokens of those whole files. Each file is read once per run and
only its word set and its token cost are kept; a repository larger than 256 MB of source is sampled
with a fixed stride in path order, and the files the tasks are about are always kept, so the baseline
is never denied a hit it could have had.

## The command

```sh
pn-ultramemory index                       # the benchmark measures what is indexed
pn-ultramemory bench                       # 100 tasks, seed 1, budgets 500/1000/2000, 3 files
pn-ultramemory bench --tasks 200 --seed 7 --budget 1000 --budget 4000 --baseline-files 5
pn-ultramemory bench -f json > bench.json  # any of the three output formats
```

| Flag | Default | Meaning |
|---|---|---|
| `--tasks N` | 100 | how many documented symbols to ask about |
| `--seed N` | 1 | fixes the sample; the same seed draws the same tasks |
| `--budget TOKENS` | 500, 1000, 2000 | repeat the flag for several budgets |
| `--baseline-files N` | 3 | how many whole files the baseline reads per query |

A budget outside the engine's own limits is clamped before it is measured, and the row reports the
budget that was actually used. Running the benchmark does **not** teach the learner anything: the
session record is cleared before each query, so no run biases the next one.

## Reading the output

The report is one long table, one row per figure, so that every number carries its own name and its
own derivation:

```text
indexed_symbols: 4735
indexed_files: 260
metrics[48]{family,budget,tasks,metric,value,derivation}:
  description,1000,100,hit_rate,0.99,measured
  description,1000,100,mean_tokens,795.03,measured
  description,1000,100,baseline_hit_rate,0.89,measured
  description,1000,100,baseline_mean_tokens,15267.93,measured
  description,1000,100,tokens_saved_ratio,0.948,measured
  description,1000,100,p50_ms,17.842,measured
  description,1000,100,p95_ms,22.917,measured
  description,1000,100,task_success,,unobservable
notes[8]: ...
```

| Figure | Meaning |
|---|---|
| `hit_rate` | share of tasks whose wanted symbol was in the capsule |
| `mean_tokens` | mean tokens a capsule used, never above the budget |
| `baseline_hit_rate` | share of tasks whose file the lexical ranking chose |
| `baseline_mean_tokens` | mean tokens of reading those whole files |
| `tokens_saved_ratio` | `(baseline − ours) / baseline`. **It can be negative**, and on a repository of a handful of files it usually is: there is nothing to save when reading everything is cheap. |
| `p50_ms`, `p95_ms` | latency of one `recall`, in milliseconds |
| `task_success` | always empty. See below. |

`tasks` is the number of tasks that were actually run, which can be lower than `--tasks`: a task whose
query came out empty (a documentation sentence made entirely of words from the name) is skipped rather
than counted as a failure.

Every figure except `p50_ms` and `p95_ms` is byte-identical between two runs of the same seed over the
same index. Those two are wall-clock measurements and cannot be. If you are comparing two reports,
compare everything else.

## Why there is no single score

A composite score is the most requested number here and the least defensible one. Three reasons:

1. **Weights become the target.** A score is a set of weights chosen once by whoever wrote it. From
   then on the code is optimized for those weights rather than for the reader, and nobody can see it
   happening from the score alone.
2. **A score can rise while the tool gets worse.** Retrieval that returns more plausible but wrong
   results can raise a hit-rate-and-tokens blend, because a dead end costs the score nothing and
   costs its reader a full read, a wrong edit and a retry. Cheap dead ends are exactly what a memory
   for agents must not produce, and exactly what a single number hides.
3. **The figures are not commensurable.** Tokens are money, latency is patience and a hit is a
   probability. Adding them requires an exchange rate that does not exist.

So each figure stands on its own, and the figure a reader most wants — whether an agent finished the
job — is reported as empty rather than invented.

## What each derivation means

| `derivation` | It means | Trust it for |
|---|---|---|
| `measured` | counted directly from what this run did | comparing runs of this tool on this repository |
| `proxy` | stands in for something that was not measured | direction, not magnitude |
| `unobservable` | this benchmark cannot see it at all; the value is always empty | nothing — it is there so the gap is visible instead of silently missing |

`value` is deliberately an option: an unobservable figure serializes as `null` and never as `0`,
because a zero is indistinguishable from a measurement of total failure.

## What this does not measure

- **Not real work.** The queries come from the documentation of the symbols themselves, so this is
  *known-item retrieval*: the answer exists, is documented, and is described in words its author
  chose. Real questions are vaguer, are about code that may not exist, and often have no single right
  symbol.
- **Not task success.** A hit means the symbol is present in the capsule. It says nothing about
  whether a model read it, understood it, or produced a working change. No model ran.
- **Not a strong baseline.** Reading whole files by lexical word overlap is the *floor*, not the state
  of the art. It is not grep with a good pattern, not a language server, not an embedding search, and
  not a competent human choosing files. Beating it is necessary, not sufficient.
- **Not billed tokens.** Counts come from this project's own estimator, fitted against the
  `cl100k_base` tokenizer to about 4.5 % mean absolute error on held-out files. Another model's
  tokenizer differs, and any claim about money must use the provider's own counts.
- **Not portable between repositories.** Results depend on the size, the language mix and above all
  the documentation of the repository they were measured on. A number from one codebase says nothing
  about another; rerun it on yours.
- **Not a measure of what was learned.** The benchmark reads whatever learned evidence the repository
  already carries, exactly as normal use would. For a clean comparison, run
  `pn-ultramemory learn reset` first.
- **Not a quality gate.** Small samples move. With 100 tasks a hit rate is worth roughly ±10
  percentage points; treat a difference smaller than that as noise, and raise `--tasks` before
  believing one.
