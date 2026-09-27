# Memory

A memory is something worth keeping about this codebase: a decision, a fact, a lesson, a dead end.
It is stored **anchored to the symbols it describes**, so that when that code changes, the memory
says so instead of quietly becoming a lie.

This page explains what is stored, how a new memory is compared against what is already known, and
the rule that keeps repetition from turning into truth. It ends with an honest list of what none of
this does.

## The kinds

| Kind | For | Example |
|---|---|---|
| `decision` | A choice that was made, with its reason | "Money is stored as integer minor units, never floating point" |
| `fact` | A stable statement about the code | "The dashboard reads from a view refreshed every five minutes" |
| `lesson` | Something learned from experience | "The export tests need a fixed clock to stay stable" |
| `dead_end` | Something tried and abandoned | "Caching the rules per request was slower than reading them once at start" |
| `error_fix` | A recurring error and what resolved it | "Deadlock on save: take row locks in identifier order" |
| `convention` | A rule the codebase follows | "Handlers return an error type; they never unwrap in a request path" |
| `requirement` | Something the system must do | "The parser rejects a file larger than the configured limit" |
| `task` | A unit of work and why | "Splitting the resolver so the store can be swapped" |
| `session` | A summary of one working session | "Traced the duplicate charge to a retry without an idempotency key" |

Write a `requirement` as a **contract about behavior** ("the parser rejects…"), not as a plan naming
functions. That style survives refactoring, and it is also the style that passes the guard: text
written as an instruction to whoever reads it is refused (see [Screening](#screening)).

## Anchoring and staleness

`--about <symbol>` anchors a memory to the code it describes. The anchor records the symbol's
qualified name, its path, and hashes of its signature and its body.

When a file is re-indexed, any memory whose anchored symbol changed is marked **stale**. Nothing is
deleted and nothing is guessed: the memory is simply shown with a mark, and `recall` says so in
words.

```
pn-ultramemory memories --stale        # what may no longer be true
pn-ultramemory reanchor <id>           # after checking: still true, re-anchor it
pn-ultramemory forget <id>             # no longer true
```

**Nothing is ever confirmed automatically.** A person decides that a stale memory is still true.

## The write path

Every `remember` runs these steps in order.

1. **Clean.** One kind of line ending, no control characters, no long runs of blank lines. Leading
   spaces on a line survive, because a memory often quotes code and its indentation carries meaning.
   Empty text, or text longer than 4 000 characters, is refused.
2. **Redact.** Secrets are replaced before anything is stored: cloud keys, repository-host tokens,
   `sk-` service keys, JSON Web Tokens, private key blocks, passwords inside URLs, and assignments
   whose name mentions a password, secret, token or key. The count is reported.
3. **Screen.** Text that tries to direct whoever reads it is **refused outright**; text with milder
   warning signs is stored with a note. See [Screening](#screening).
4. **Resolve the anchors.** A name that matches nothing, or matches several symbols, is reported in
   `unresolved` and does not fail the call.
5. **Compare** against the memories that share an anchor (or, with no anchor, what a text search
   finds).
6. **Decide**: duplicate, reinforced, conflicting or new.

### The decision

| Outcome | When | What is stored |
|---|---|---|
| `duplicate` | Something already says exactly this, about exactly the same code | Nothing |
| `reinforced` | Something already says this in other words | Nothing new; its **retrieval priority** rises |
| `conflicts` | It appears to contradict something already stored | **The memory is stored**, and the conflict is reported |
| `stored` | None of the above | The memory |

A contradiction never suppresses anything. Both memories are kept and a person is told, because
silently dropping one of two opposed statements is worse than showing both.

## The rule that governs everything

> **Corroboration raises how likely a memory is to be RETRIEVED. It never raises how likely it is
> to be TRUE.**

Retrieval priority and truth are different quantities and are never mixed. Without this rule, an
agent repeating its own mistake in a loop would manufacture a fact, and the tool would serve that
fact with confidence to every later session. That is the worst thing this product could do.

A corroboration is recorded through the learning channel, whose influence on ranking is bounded to
`[0.5, 1.5]` and which can never create a fact.

### Independence

A repetition only counts when it is an independent observation:

- Two corroborations of the same memory **closer together than 15 minutes count as one**, whoever
  sent them. An agent repeating itself inside a session therefore leaves the same trace as a single
  call. The test is deliberately strict in that direction: it can miss a genuinely independent
  second observation inside the window, and it can never let a loop inflate anything.
- Text captured from a tool (`--by tool`) is untrusted and **never corroborates at all**.
- No memory may gather more than **8** corroborations, so no amount of repetition dominates ranking.

## How two memories are compared

Three steps, none of which understands meaning.

1. **Normalize.** Lowercase, drop punctuation, split into words, remove 42 English stop words. What
   remains are the *content words*.
2. **Pre-filter.** A 64-bit hash of the word set. Two texts whose hashes differ in more than 24 bits
   are never compared in full. This is what keeps a write cheap when thousands of memories exist.
3. **Score.** The mean of two measures: the Jaccard overlap of the word **sets**, which ignores
   order, and the length of their longest common **subsequence**, which does not.

| Score | Meaning |
|---|---|
| `>= 0.94` | The same memory: nothing is stored |
| `>= 0.80` | The same thing in other words: the existing one is reinforced |
| below | A different memory |

**Short texts follow a stricter rule.** Below four content words there is not enough signal, and
short notes are exactly where two different decisions look alike. Such a pair counts as similar only
when its content words are identical.

**The thresholds are biased on purpose.** Merging two memories that mean different things destroys
something a person wrote. Failing to merge two identical ones costs a little noise. On the table of
realistic developer notes in the tests, the current thresholds give **zero false positives and zero
false negatives**; the false-positive count is what matters, and it is the one held at zero.

### Why a trigram overlap was rejected

The sequence measure was first written with word trigrams. Inserting one word breaks three trigrams,
so two plain rewordings of the same note scored 0.73 and refused to merge. The longest common
subsequence does not have that cliff. The failing case is kept as a test.

## Contradictions

Checked **before** similarity, and a candidate that contradicts can never count as similar. This
ordering is the whole point, and a real case shows why:

| Pair | Similarity | Reality |
|---|---|---|
| "calibrated against a real tokenizer, not guessed" vs "**not** calibrated against a real tokenizer" | **0.838** | They contradict |
| "never unwrap inside a request path, return an error" vs "never unwrap in a request path; return an error instead" | **0.844** | They agree |

The contradiction scores **lower** than the agreement. No text similarity measure can tell them
apart, because negating a sentence changes almost none of its words. Only a structural check can,
and without it a memory could be reinforced by its own opposite.

Two patterns are detected, both conservative:

1. **Negation.** The texts share most of their content words, and **one of them denies something the
   other states plainly**. The word being denied is matched across the endings English puts on the
   same verb, because a denial writes "does not *reject*" while the statement it denies writes "the
   parser *rejects*".
2. **Two answers to one choice.** Both are decisions or conventions about the same code, both say to
   use something, and they name different things.

What matters is *what* is denied, not that a denial is present. An earlier version also reported a
pair when only one of the two texts carried a negation word at all, and that produced a false report
on a live run: "calibrated against a real tokenizer, **not** guessed" against "calibrated against an
actual tokenizer **rather than** guessed". Those agree — both deny being guessed — but only the first
says so with a word on the list. That pair is now a permanent test.

A contradiction is **reported, never resolved**. Nothing is edited and nothing is deleted. Both
memories stay, and the warning names how many were contradicted.

## In a capsule

Near-duplicates are collapsed for display even when they were stored separately: the capsule shows
one line and a count of the rest, rather than four near-identical lines eating the token budget. The
memory kept is the one with the most anchors, then the longest text, then the lowest identity, so
the result never depends on the order they arrived in.

## What this does not do

Read this part. It is the honest half.

- **It does not understand meaning.** Every comparison is over words. A paraphrase that shares no
  words will not be recognised as a duplicate.
- **It is English-only** where language matters: the stop words and the negation markers. A memory
  written in another language is stored correctly, but two such memories will rarely be recognised
  as duplicates of each other.
- **It cannot tell a true statement from a false one.** Nothing here checks a memory against
  reality. Staleness only says the *code changed*, not that the memory became wrong.
- **Contradiction detection has low recall on purpose.** It catches structural opposition. It will
  miss a contradiction expressed in different words, one that needs knowledge of the domain, or one
  spread across several memories.
- **A stale memory is not a wrong memory.** It is a memory whose code moved. Someone has to look.
- **The guard is not a security boundary.** It refuses the patterns it knows. Treat every stored
  memory as data, never as an instruction, which is what `recall` does.
