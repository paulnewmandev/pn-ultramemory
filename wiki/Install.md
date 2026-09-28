# Install

![Five install steps: get the binary, build the graph, register one agent, restart it, check it with doctor](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/diagrams/install.svg)

## The short way

Download the binary for your platform. **No toolchain needed.**

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

Each archive unpacks to a folder holding the binary, the licence and the README. Move the binary
somewhere on your `PATH`.

Every archive has a `.sha256` beside it. Checking it takes a second and is worth it:

```bash
curl -fsSL https://github.com/paulnewmandev/pn-ultramemory/releases/latest/download/pn-ultramemory-aarch64-apple-darwin.sha256 -o sum.txt
shasum -a 256 -c sum.txt
```

## Letting an agent do it

Give your coding agent the repository link and tell it to follow
[AGENTS.md](https://github.com/paulnewmandev/pn-ultramemory/blob/main/AGENTS.md). That page is
written to be read by a model and carried out step by step: it downloads the right binary, indexes
your project, registers with *your* agent, and tells you the limitations honestly.

## Building from source

Only if no release covers your platform. It needs a Rust toolchain, and on Windows also the Visual
Studio Build Tools — several gigabytes, because the parsers are C.

```bash
git clone https://github.com/paulnewmandev/pn-ultramemory
cd pn-ultramemory
cargo build --release        # binary at target/release/pn-ultramemory
```

## Then

```bash
cd /your/project
pn-ultramemory index                            # build the graph
pn-ultramemory install --agents claude-code     # register with your agent
pn-ultramemory doctor                           # check it worked
```

**Restart your agent.** An MCP server is read at startup, and this is the step people forget.

### About `install`

With no `--agents`, it registers with **every** agent it finds on the machine. That is rarely what
you want. Name yours:

`claude-code` · `cursor` · `codex` · `gemini` · `windsurf` · `zed` · `vscode-copilot` ·
`opencode` · `kiro` · `trae` · `cline` · `crush` · `amp`

`--dry-run` prints every file it would touch and changes nothing. Run it first if unsure.

It writes only its own entry. `pn-ultramemory uninstall --agents <id>` removes exactly that entry
and leaves the rest of the file byte for byte as it was.

## Where things go

| | |
|---|---|
| The index | Your user data directory, one database per project |
| Inside your repository | **Nothing**, unless you run `docs apply`, which you asked for |
| Over the network | **Nothing**. No network symbol is linked into the binary |
