# pn-ultramemory

A local, code-aware memory for coding agents. It indexes your repository into a graph of symbols
and answers a question with the code that matters inside a token budget you set.

**[Install](Install) · [First hour](First-hour) · [Commands](Commands) · [MCP tools](MCP-tools) · [How it works](How-it-works) · [Memories](Memories) · [FAQ](FAQ) · [Troubleshooting](Troubleshooting)**

---

![One question costs 16,109 tokens read as files, or 413 as a capsule, and the capsule finds the right code more often](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/diagrams/why.svg)

## In one paragraph

Your coding agent reads whole files to answer questions about your code. Most of what it reads is
not the answer, and you pay for every token. This indexes the repository once into a graph of
symbols and the relationships between them, then answers a question with a **capsule**: the code
that matters, at the level of detail that fits, inside a budget. On this repository that is 413
tokens instead of 16,109 — and it finds the right code 99% of the time against 85% for reading
files.

It also remembers decisions you tell it, anchored to the code they describe, so when that code
changes the memory says so instead of quietly becoming a lie.

![Nine tools in three groups: read the code, survey it, write to the memory](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/diagrams/tools.svg)

## Where to start

| You want to | Go to |
|---|---|
| Get it running in five minutes | [Install](Install) |
| Know what to do once it is running | [First hour](First-hour) |
| Look up a command or a flag | [Commands](Commands) |
| Wire it into an agent and use it well | [The MCP tools](MCP-tools) |
| Understand why it saves what it saves | [How it works](How-it-works) |
| Store decisions and lessons | [Memories](Memories) |
| Decide whether it suits your project | [FAQ](FAQ) |
| Fix something | [Troubleshooting](Troubleshooting) |

## What it is not

It does not understand meaning; every text comparison is over words. It cannot tell a true
statement from a false one — *stale* means the code changed, not that the memory became wrong. It
does not measure whether your agent solved your problem, and says `unobservable` rather than
inventing a number. And it is new: one author, days old. Treat it as something to try.
