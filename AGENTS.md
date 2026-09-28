<!-- SPDX-License-Identifier: Apache-2.0 -->
# Setting up pn-ultramemory

**You are a coding agent and someone asked you to install this tool in their project. This page is
for you. Follow it in order and stop where it says to stop.**

It gives the person a local code graph of their repository so that you can answer questions about
their code with a small budgeted capsule instead of reading whole files.

---

## 0. Before anything, tell them what you are about to do

Two of these steps change files outside the project directory. Say so, in one line, and wait for
them to agree:

> This installs a binary and registers it as an MCP server in the coding agents on this machine, so
> I can query your code graph instead of reading whole files. It writes nothing inside your
> repository and never connects to the network.

If they only want to try it without touching any agent configuration, do steps 1 and 2 and stop.

---

## 1. Get the binary

**Prefer a release build. It needs no toolchain.**

```bash
# macOS (Apple silicon)
curl -fsSL https://github.com/paulnewmandev/pn-ultramemory/releases/latest/download/pn-ultramemory-aarch64-apple-darwin.tar.gz | tar xz
# macOS (Intel)
curl -fsSL https://github.com/paulnewmandev/pn-ultramemory/releases/latest/download/pn-ultramemory-x86_64-apple-darwin.tar.gz | tar xz
# Linux
curl -fsSL https://github.com/paulnewmandev/pn-ultramemory/releases/latest/download/pn-ultramemory-x86_64-unknown-linux-gnu.tar.gz | tar xz
```

On Windows, download `pn-ultramemory-x86_64-pc-windows-msvc.zip` from the same releases page and
unzip it.

Each archive unpacks to a folder holding the binary. Move it somewhere on `PATH`, or use its full
path in every command below.

**Only if no release exists for their platform**, build from source. Say first that this needs a
Rust toolchain, and on Windows also the Visual Studio Build Tools, which are several gigabytes:

```bash
git clone https://github.com/paulnewmandev/pn-ultramemory
cd pn-ultramemory && cargo build --release      # binary at target/release/pn-ultramemory
```

Check it runs before going on:

```bash
pn-ultramemory --version
```

---

## 2. Build the graph

From the root of **their** project:

```bash
pn-ultramemory index
```

It prints how many files, symbols and edges it found. Nothing is written inside their repository:
the index goes in their user data directory, one database per project.

Try it before telling them it works:

```bash
pn-ultramemory recall "<something you already know is in their code>" -b 800
```

If that returns symbols from their project, it is working. If it returns nothing, say so — do not
report success.

---

## 3. Register it with **their** agent, not every agent

`install` with no arguments writes to **every** coding agent it finds on the machine. That is
usually not what someone wants. Name the one they are actually using:

```bash
pn-ultramemory install --agents claude-code     # or cursor, codex, gemini, windsurf,
                                                # zed, vscode-copilot, opencode, kiro,
                                                # trae, cline, crush, amp
```

Two flags worth knowing:

| | |
|---|---|
| `--dry-run` | Prints every file it would touch and changes nothing. Run this first if unsure |
| `--scope project` | Writes into the project instead of the user's configuration |

It writes only its own entry and leaves the rest of the file byte for byte as it was.
`pn-ultramemory uninstall --agents <id>` removes exactly that entry.

Then confirm:

```bash
pn-ultramemory doctor
```

**Restart their agent** — an MCP server is read at startup. Say this explicitly; it is the step
people forget.

---

## 4. How to use it once it is running

Five tools arrive over MCP. Reach for them instead of reading files:

| Tool | Ask it | Instead of |
|---|---|---|
| `recall` | Which code matters for this question | Reading several files |
| `outline` | What is in this file | Reading the whole file |
| `impact` | What breaks if I change this | Grepping for the name |
| `expand` | The exact source of one symbol | Reading around it |
| `remember` | Store a decision, anchored to the code | A comment nobody reads |

Two habits that make the difference:

- **Set a budget.** `recall "..." -b 800` costs about 800 tokens. Without one it uses the default.
- **Re-index after a branch change.** `pn-ultramemory index` re-reads only what changed; it takes
  milliseconds when nothing did.

### Memories

Write them as statements about the code, anchored to it:

```bash
pn-ultramemory remember decision "Money is stored as integer minor units, never floating point" \
  --about Money
```

English and Spanish are both read properly — duplicates and contradictions are found in either.
A memory whose anchored code changes is marked stale; it is never deleted and never guessed at.

---

## 5. What to tell them honestly

Do not oversell it. These are true and they should hear them from you:

- **It is new.** One author, no external users yet. Treat it as something to try, not as
  infrastructure.
- **The saving depends on the repository.** The published figures come from measuring this tool on
  its own source, a Rust repository of about 260 files. On a small site or a handful of scripts,
  reading the files was never expensive, so there is less to save. They can measure their own with
  `pn-ultramemory bench`.
- **Twelve languages are parsed properly** (Rust, Python, JavaScript, TypeScript, TSX, Go, Java, C,
  C++, C#, Ruby, PHP). Every other language is still indexed, by a simpler lexical pass, so nothing
  is invisible but the graph is thinner.
- **It is Apache-2.0 and free**, with no account, no telemetry and no network call.

---

## If something goes wrong

| | |
|---|---|
| `pn-ultramemory doctor` | Says what is installed, what is indexed, and what is wrong |
| The tool is not found after `install` | Their agent was not restarted |
| `recall` returns nothing | The index is empty — run `index` from the project root |
| A path is not indexed | Check it is not ignored by `.gitignore` |

Every error message names the command that would fix it. If one does not, that is a defect worth
reporting at <https://github.com/paulnewmandev/pn-ultramemory/issues>.
