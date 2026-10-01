# Your first hour

What to do once it is installed, in the order that pays off. Every step runs on your own code, so
by the end you know whether it helps *you*, not whether it helps a benchmark.

## 1. Index, and see what it found

```bash
pn-ultramemory index
pn-ultramemory brief
```

`brief` is what an agent reads at the start of a session: the size of the repository, its
languages, its modules with how much they depend on each other, its most-called symbols and every
memory on record. If the symbol count looks far too low, one of your languages is probably going
through the lexical fallback rather than a full parser; see [FAQ](FAQ#which-languages-are-parsed-properly).

Check the busiest symbols in `central`. They should be things your code really leans on. If you
see a method name that the standard library or a framework also uses, the receiver of those calls
was not recognised; tell us, with the language.

## 2. Look at it

```bash
pn-ultramemory brain
```

Your repository as a brain: each folder a region, tests in the cerebellum. Press `/`, type the name
of something you know, open it, and follow what calls it and what it calls. Five minutes here tells
you more about the shape of an unfamiliar codebase than an hour of reading. See [The brain](Brain).

## 3. Ask something you already know the answer to

Do not skip this one:

```bash
pn-ultramemory recall "how are prices rounded" -b 800
```

You should see the symbols you expected near the top, at mixed levels of detail, with `used:` under
800. Use the words your code uses: the search is over words, and nothing is translated. If the
answer is wrong, find out why before you wire it into an agent; `--explain` says why each symbol is
there.

## 4. Watch the budget work

```bash
pn-ultramemory recall "how are prices rounded" -b 200
pn-ultramemory recall "how are prices rounded" -b 800
pn-ultramemory recall "how are prices rounded" -b 3000
```

At 200 you get names and a signature or two. At 800, signatures and summaries. At 3000, whole
functions. As the budget falls, the *detail* falls, not the answers; the best few always keep
their signature.

## 5. Read a file without reading it

```bash
pn-ultramemory outline src/some/large/file.ts
```

Every symbol the file declares, in order and nested, with signatures and first documentation
sentences. It reports what it cost and what reading the file would have cost, and for a short file
it tells you that reading it outright is cheaper.

## 6. Find out what a change would break

```bash
pn-ultramemory impact PriceCalculator
```

Read `epistemic` before the list. `exact` means the set is complete, `lower-bound` that there may be
more, and `unknown` that no caller was found and the symbol is public, which does **not** mean
nobody uses it.

## 7. Write your first memory

```bash
pn-ultramemory remember decision \
  "Prices are rounded half-up at the last step, never per line item" \
  --about PriceCalculator
```

Open the brain again: the memory is a golden ring above `PriceCalculator`. When that code changes,
the ring turns red and `memories --stale` lists it. See [Memories](Memories).

## 8. Connect your agent

```bash
pn-ultramemory install --agents claude-code     # or whichever you use
pn-ultramemory doctor
```

Restart the agent, then ask it something about your code and watch whether it calls `brief` and
`recall` instead of reading files. If it does not, say so once in your prompt; most agents keep the
hint for the session.

## A habit worth forming

Re-index after switching branches or pulling. Only what changed is read again, so it costs
milliseconds when nothing did:

```bash
pn-ultramemory index
```
