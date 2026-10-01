# Install

![Five steps: get the binary, build the graph, register one agent, restart it, check it with doctor](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/diagrams/install.svg)

## 1. Get the binary

Download the build for your platform. No toolchain is needed.

```bash
# macOS, Apple silicon
curl -fsSL https://github.com/paulnewmandev/pn-ultramemory/releases/latest/download/pn-ultramemory-aarch64-apple-darwin.tar.gz | tar xz

# macOS, Intel
curl -fsSL https://github.com/paulnewmandev/pn-ultramemory/releases/latest/download/pn-ultramemory-x86_64-apple-darwin.tar.gz | tar xz

# Linux, x86_64
curl -fsSL https://github.com/paulnewmandev/pn-ultramemory/releases/latest/download/pn-ultramemory-x86_64-unknown-linux-gnu.tar.gz | tar xz
```

On **Windows**, download `pn-ultramemory-x86_64-pc-windows-msvc.zip` from the
[releases page](https://github.com/paulnewmandev/pn-ultramemory/releases/latest) and unzip it.

Each archive unpacks to a folder with the binary, the licence and the README. Move the binary
somewhere on your `PATH`. Every archive has a `.sha256` beside it:

```bash
curl -fsSL https://github.com/paulnewmandev/pn-ultramemory/releases/latest/download/pn-ultramemory-aarch64-apple-darwin.sha256 -o sum.txt
shasum -a 256 -c sum.txt
```

### Or build it

Needed when no release covers your platform, and for features that are on `main` but not yet
released (the [brain](Brain) and receiver-aware resolution, until the next release). It needs a
Rust toolchain, and on Windows the Visual Studio Build Tools, because the parsers are written in C.

```bash
git clone https://github.com/paulnewmandev/pn-ultramemory
cd pn-ultramemory
cargo build --release          # the binary is target/release/pn-ultramemory
```

## 2. Build the graph

From the root of **your** project:

```bash
pn-ultramemory index
```

It prints how many files, symbols and edges it found. Then ask it something you already know is in
your code, to see it work before you trust it:

```bash
pn-ultramemory recall "<something you know is in the code>" -b 800
```

## 3. Connect your agent

Name the agent you use. Without `--agents`, every agent found on the machine is configured, which is
rarely what you want.

```bash
pn-ultramemory install --agents claude-code --dry-run    # see the change first
pn-ultramemory install --agents claude-code
```

Agent ids: `claude-code` · `cursor` · `codex` · `gemini` · `windsurf` · `zed` · `vscode-copilot` ·
`opencode` · `kiro` · `trae` · `cline` · `crush` · `amp`

It writes only its own entry and leaves the rest of the file byte for byte as it was.
`--scope project` writes into the project instead of your user configuration.
`pn-ultramemory uninstall --agents <id>` removes exactly that entry.

## 4. Restart the agent

An MCP server is read when the agent starts, and only then. This is the step people forget, and the
usual reason someone reports that the tools never appeared.

## 5. Check

```bash
pn-ultramemory doctor
```

It reports the repository, the data directory, the index, the agents configured, the hooks and the
free disk space, with the command that fixes anything wrong.

## Letting an agent do it

Give your coding agent the repository link and tell it to follow
[AGENTS.md](https://github.com/paulnewmandev/pn-ultramemory/blob/main/AGENTS.md). It is written for
a model to carry out step by step, asks before touching anything outside your project, and tells you
the limitations honestly.

## Where things go

| | |
|---|---|
| The index, memories and usage counters | Your user data directory, one database per project |
| The brain page | `brain.html` in that same directory, unless you pass `-o` |
| Inside your repository | **Nothing** unless you ask: `docs apply` writes the documentation you supply, and `report` writes to the current directory unless you pass `-o` |
| Over the network | **Nothing**. No network code is linked into the binary |

## Upgrading

Replace the binary. The index migrates itself the first time it is opened; when a migration needs
information the old index never recorded, it marks every file as changed and the next `index` reads
the repository once more. Memories are kept.
