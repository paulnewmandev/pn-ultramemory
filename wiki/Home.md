# pn-ultramemory

A second brain for your codebase, shared by you and your coding agent. It indexes the repository
into a graph of symbols, answers questions with the code that matters inside a token budget,
remembers decisions anchored to the code they describe, and draws all of it as a 3D brain you can
fly through.

**[Install](Install) · [First hour](First-hour) · [Commands](Commands) · [The brain](Brain) · [MCP tools](MCP-tools) · [Memories](Memories) · [How it works](How-it-works) · [FAQ](FAQ) · [Troubleshooting](Troubleshooting)**

![This repository drawn as a brain: one glowing particle per symbol, coloured by folder, tests in the cerebellum, golden rings for memories](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/screenshots/brain-overview.jpg)

## In one paragraph

Your coding agent reads whole files to answer questions about your code. Most of what it reads is
not the answer, you pay for every token, and it forgets everything when the session ends. This tool
indexes the repository once into a graph of symbols and the relationships between them, then
answers a question with a **capsule**: the symbols that matter, each at the level of detail that
fits, inside a budget. On its own repository that is 464 tokens instead of 16,520, and it finds the
right code more often than reading files does. The graph and the memories stay on disk, so the next
session starts where this one ended.

## Two brains, one store

| | For | How |
|---|---|---|
| **The agent's** | A model that needs the right code now, cheaply | Nine MCP tools: `brief`, `recall`, `outline`, `expand`, `impact`, `map`, `remember`, `memories`, `feedback` |
| **Yours** | A person who wants to see the code's shape and what was decided | `pn-ultramemory brain`: search, open any symbol, follow its callers and callees, read its memories, copy it for an agent |

Both read the same index and the same memories, so what one learns the other sees.

## Where to start

| You want to | Go to |
|---|---|
| Get it running in five minutes | [Install](Install) |
| Know what to do once it is running | [Your first hour](First-hour) |
| See your codebase as a brain | [The brain](Brain) |
| Look up a command or a flag | [Commands](Commands) |
| Wire it into an agent and use it well | [The MCP tools](MCP-tools) |
| Keep decisions and lessons | [Memories](Memories) |
| Understand where the saving comes from | [How it works](How-it-works) |
| Decide whether it suits your project | [FAQ](FAQ) |
| Fix something | [Troubleshooting](Troubleshooting) |

## What it is not

It does not understand meaning: every comparison is over words, and nothing is translated. It
cannot tell a true statement from a false one; *stale* means the code changed. It does not measure
whether your agent solved your problem. And it is new, with one author and few users so far: try it,
measure it on your own code with `pn-ultramemory bench`, and report what breaks.
