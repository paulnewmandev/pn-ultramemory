# Install

![Five steps: get the binary, build the graph, register one agent, restart it, check it with doctor](https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/assets/diagrams/install.svg)

## 1. Get the binary

### macOS and Linux: one command

```bash
curl -fsSL https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/install.sh | sh
```

It picks the build for your system (macOS on Apple silicon or Intel, Linux on x86_64), downloads it
from the latest release, **refuses to install it unless it matches its published SHA-256**, and puts
it in `~/.local/bin`. It needs no root and edits no shell profile: when that folder is not on your
`PATH` yet, it prints the one line to add to `~/.zshrc` or `~/.bashrc`. Open a new terminal
afterwards and check:

```bash
pn-ultramemory --version
```

Two settings, both optional: `PN_ULTRAMEMORY_VERSION=v1.1.0` installs a given release, and
`PN_ULTRAMEMORY_BIN_DIR=/usr/local/bin` installs somewhere else. Read
[the script](https://github.com/paulnewmandev/pn-ultramemory/blob/main/install.sh) first if you
prefer; it is short.

### Windows

In PowerShell:

```powershell
$dir = "$env:LOCALAPPDATA\pn-ultramemory"
Invoke-WebRequest https://github.com/paulnewmandev/pn-ultramemory/releases/latest/download/pn-ultramemory-x86_64-pc-windows-msvc.zip -OutFile "$env:TEMP\pn-ultramemory.zip"
Expand-Archive "$env:TEMP\pn-ultramemory.zip" -DestinationPath $dir -Force
$bin = "$dir\pn-ultramemory-x86_64-pc-windows-msvc"
$user = [Environment]::GetEnvironmentVariable("Path", "User")
[Environment]::SetEnvironmentVariable("Path", "$user;$bin", "User")
```

Open a new terminal, then `pn-ultramemory --version`. The binary needs the Microsoft Visual C++
runtime, which most systems already have.

### By hand

Every release carries one archive per platform and a `.sha256` beside each, on the
[releases page](https://github.com/paulnewmandev/pn-ultramemory/releases/latest):

```bash
curl -fsSLO https://github.com/paulnewmandev/pn-ultramemory/releases/latest/download/pn-ultramemory-aarch64-apple-darwin.tar.gz
curl -fsSLO https://github.com/paulnewmandev/pn-ultramemory/releases/latest/download/pn-ultramemory-aarch64-apple-darwin.sha256
shasum -a 256 -c pn-ultramemory-aarch64-apple-darwin.sha256
tar xzf pn-ultramemory-aarch64-apple-darwin.tar.gz
mkdir -p ~/.local/bin && mv pn-ultramemory-aarch64-apple-darwin/pn-ultramemory ~/.local/bin/
```

The targets are `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu` and
`x86_64-pc-windows-msvc` (a `.zip`). On macOS, a binary downloaded with a browser instead of `curl`
is quarantined; see [Troubleshooting](Troubleshooting#macos-says-it-cannot-verify-the-developer).

### Or build it

For any platform without a release, such as Linux on ARM. It needs a Rust toolchain, and on Windows
the Visual Studio Build Tools, because the parsers are written in C.

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

Run the installer again, or replace the binary by hand. The index migrates itself the first time it
is opened; when a migration needs information the old index never recorded, it marks every file as
changed and the next `index` reads the repository once more. Memories are kept.
