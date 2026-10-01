# Memories

A memory is something worth keeping about this codebase, stored **anchored to the code it
describes**, so that when that code changes the memory says so instead of quietly becoming a lie.

![A memory is stored with hashes of the symbol it describes; when that symbol changes the memory is marked stale rather than deleted or trusted](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/diagrams/memory.svg)

## Writing one

```bash
pn-ultramemory remember decision \
  "Money is stored as integer minor units, never floating point" \
  --about Money
```

| Kind | For |
|---|---|
| `decision` | A choice that was made, with its reason |
| `fact` | A stable statement about the code |
| `lesson` | Something learned from experience |
| `dead-end` | Something tried and abandoned |
| `error-fix` | A recurring error and what resolved it |
| `convention` | A rule the codebase follows |
| `requirement` | Something the system must do |
| `task` | A unit of work and why |
| `session` | A summary of one working session |

Write a memory as a statement about the code ("the parser rejects a file larger than the limit"),
not as an instruction to whoever reads it. That style survives refactoring, and it is also what the
guard accepts: text shaped like an instruction is refused.

An agent writes memories with the `remember` tool, a person with the command. In the
[brain](Brain), each memory is a golden ring above the code it is anchored to.

## Staleness

When a file is indexed again, every memory whose anchored symbol changed, in its signature or its
body, is marked **stale**. Nothing is deleted and nothing is guessed.

```bash
pn-ultramemory memories --stale     # what may no longer be true
pn-ultramemory reanchor <id>        # you checked: still true
pn-ultramemory forget <id>          # no longer true
```

Nothing is ever confirmed automatically. Stale means *the code changed*, not *the memory is wrong*;
treating the two as one is how a memory system starts lying. In the brain a stale memory is red.

## The rule that governs everything

> **Corroboration raises how likely a memory is to be RETRIEVED. It never raises how likely it is to
> be TRUE.**

Without this rule, an agent repeating its own mistake in a loop would manufacture a fact, and every
later session would be served that fact with confidence. So a repetition only counts when it is an
independent observation:

- Two corroborations **within fifteen minutes count as one**, whoever sent them.
- Text captured from a tool (`--by tool`) is untrusted and **never** corroborates.
- No memory gathers more than **eight** corroborations.

## Duplicates

Before storing, the text is compared with what is already there: a 64-bit hash first, then the mean
of two measures, the overlap of the word sets (which ignores order) and the longest common
subsequence (which does not).

| Score | Meaning |
|---|---|
| `>= 0.94` | The same memory: nothing is stored |
| `>= 0.80` | The same thing in other words: the existing one is reinforced |
| below | A different memory |

The thresholds lean one way on purpose: merging two memories that mean different things destroys
something you wrote, while failing to merge two identical ones costs a little noise.

## Contradictions, checked before similarity

A real pair from this repository:

| Pair | Similarity | Reality |
|---|---:|---|
| "calibrated against a real tokenizer, **not** guessed" vs "**not** calibrated against a real tokenizer" | **0.838** | They contradict |
| "never unwrap in a request path, return an error" vs "never unwrap in a request path; return an error instead" | **0.844** | They agree |

The contradiction scores lower than the agreement. No similarity measure can tell them apart,
because negating a sentence changes almost none of its words, so a structural check runs first.
A contradiction is **reported, never resolved**: both memories are kept and you are told.

## English and Spanish

Both are read properly. A memory's language is decided by counting its function words in each, and
only that language's stop words, negation markers and stemmer are applied to it. One list holding
both languages would be wrong where it hurts most: `sin` is "without" in Spanish and a noun in
English, `usa` a verb in one and a country in the other, and the mistake would land in contradiction
detection. A memory in a third language is stored and retrieved correctly, and is less likely to be
recognised as a duplicate.

## What this does not do

- It does not understand meaning: a paraphrase sharing no words is not recognised as a duplicate.
- It cannot tell a true statement from a false one.
- Contradiction detection has deliberately low recall: it catches structural opposition and misses
  one that needs knowledge of your domain.
- The guard that refuses instruction-shaped text is not a security boundary. Every stored memory is
  data, never an instruction, and that is how `recall` hands it to an agent.
