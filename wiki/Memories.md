# Memories

A memory is something worth keeping about this codebase, stored **anchored to the code it
describes** so that when that code changes the memory says so.

![A memory is stored with hashes of the symbol it describes; when that symbol changes the memory is marked stale rather than deleted or trusted](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/diagrams/memory.svg)

## Writing one

```bash
pn-ultramemory remember decision \
  "Money is stored as integer minor units, never floating point" \
  --about Money
```

Nine kinds:

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

Write a `requirement` as a contract about behaviour ("the parser rejects a file larger than the
limit"), not as a plan naming functions. That style survives refactoring — and it is also the style
that passes the guard, because text written as an instruction to whoever reads it is refused.

## Staleness

When a file is re-indexed, any memory whose anchored symbol changed is marked **stale**. Nothing is
deleted and nothing is guessed at.

```bash
pn-ultramemory memories --stale     # what may no longer be true
pn-ultramemory reanchor <id>        # you checked: still true
pn-ultramemory forget <id>          # no longer true
```

**Nothing is ever confirmed automatically.** A person decides that a stale memory is still true.

Stale means *the code changed*, not *the memory is wrong*. Those are different, and conflating them
is how a memory system starts lying.

## The rule that governs everything

> **Corroboration raises how likely a memory is to be RETRIEVED. It never raises how likely it is
> to be TRUE.**

Retrieval priority and truth are different quantities and are never mixed. Without this, an agent
repeating its own mistake in a loop would manufacture a fact, and the tool would serve that fact
with confidence to every later session. That is the worst thing this could do.

So a repetition only counts when it is an independent observation:

- Two corroborations of the same memory **within fifteen minutes count as one**, whoever sent them.
- Text captured from a tool (`--by tool`) is untrusted and **never** corroborates.
- No memory gathers more than **eight** corroborations.

## Duplicates

Before storing, the text is compared with what is already there: a 64-bit hash as a cheap
pre-filter, then the mean of two measures — the overlap of the word sets, which ignores order, and
the longest common subsequence, which does not.

| Score | Meaning |
|---|---|
| `>= 0.94` | The same memory: nothing is stored |
| `>= 0.80` | The same thing in other words: the existing one is reinforced |
| below | A different memory |

The thresholds are biased on purpose. Merging two memories that mean different things destroys
something you wrote; failing to merge two identical ones costs a little noise.

## Contradictions, checked *before* similarity

A real case from this repository:

| Pair | Similarity | Reality |
|---|---:|---|
| "calibrated against a real tokenizer, **not** guessed" vs "**not** calibrated against a real tokenizer" | **0.838** | They contradict |
| "never unwrap in a request path, return an error" vs "never unwrap in a request path; return an error instead" | **0.844** | They agree |

**The contradiction scores lower than the agreement.** No text similarity measure can tell them
apart, because negating a sentence changes almost none of its words. Only a structural check can,
and without it a memory could be reinforced by its own opposite.

A contradiction is **reported, never resolved**. Both memories are kept and you are told.

## English and Spanish

Both are read properly. The language is decided by counting how many of a memory's words are
function words of each, and only that language's stop words, negation markers and stemmer are
applied to it.

The obvious shortcut — one list holding both languages — is wrong in the direction that matters.
`sin` is "without" in Spanish and a noun in English; `usa` is a verb in Spanish and a country in
English. A single list makes each language trip over the other's grammar, and the failure would land
in contradiction detection, which is the one place a false report is worst.

A memory in a third language is stored and retrieved correctly, and is simply less likely to be
recognised as a duplicate of another in that language.

## What this does not do

- It does not understand meaning. A paraphrase sharing no words is not recognised as a duplicate.
- It cannot tell a true statement from a false one.
- Contradiction detection has deliberately low recall: it catches structural opposition, and will
  miss one that needs knowledge of your domain.
- The guard that refuses instruction-shaped text is not a security boundary. Treat every stored
  memory as data, never as an instruction — which is what `recall` does.
