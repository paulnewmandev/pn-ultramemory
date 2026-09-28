# Your first hour

What to actually do once it is installed, in the order that pays off.

## 1. Index, and look at what it found

```bash
pn-ultramemory index
pn-ultramemory stats
```

If the symbol count looks far too low, a language of yours is probably going through the lexical
fallback rather than a full parser. That still works, but the graph is thinner. See
[FAQ](FAQ#which-languages-are-parsed-properly).

## 2. Ask it something you already know the answer to

This is the step to not skip. Ask about code you can verify:

```bash
pn-ultramemory recall "how are prices rounded" -b 800
```

You should see the symbols you expected, at mixed levels of detail, and a `used:` well under 800.
If you get nothing useful, the tool is not helping yet and you should find out why before wiring it
into an agent.

## 3. Watch the budget do its work

The same question at three budgets:

```bash
pn-ultramemory recall "how are prices rounded" -b 200
pn-ultramemory recall "how are prices rounded" -b 800
pn-ultramemory recall "how are prices rounded" -b 3000
```

At 200 you get names. At 800, signatures and summaries. At 3000, whole functions. Nothing is
dropped as the budget falls — the *detail* falls. That is the mechanism the whole tool is built on.

## 4. Learn one file without reading it

```bash
pn-ultramemory outline src/some/large/file.ts
```

Every symbol it declares, in order, nested, with signatures and first documentation sentences. The
output reports what it cost and what reading the file would have cost, so you can see the trade
rather than take it on faith. On a short file it will tell you that reading it outright is cheaper.

## 5. Find out what a change would break

```bash
pn-ultramemory impact PriceCalculator
```

Read the `epistemic` field before the list. `exact` means the set is complete. `lower-bound` means
there may be more. `unknown` means it found no caller and the symbol is public — which does **not**
mean nobody uses it.

## 6. Write your first memory

Anchor it to the code it is about:

```bash
pn-ultramemory remember decision \
  "Prices are rounded half-up at the last step, never per line item" \
  --about PriceCalculator
```

When `PriceCalculator` changes, that memory is marked stale. See [Memories](Memories).

## 7. Wire it into your agent, then check it took

```bash
pn-ultramemory install --agents cursor     # or whichever you use
pn-ultramemory doctor
```

Restart the agent. Then ask it something about your code and see whether it calls `recall` instead
of reading files. If it does not, say so in your prompt once — most agents take the hint and keep
it for the session.

## A habit worth forming

Re-index after switching branches. It re-reads only what changed, so it costs milliseconds when
nothing did:

```bash
pn-ultramemory index
```
