# Troubleshooting

Start here:

```bash
pn-ultramemory doctor
```

It reports the repository it found, the data directory, the index, the agents configured, the hooks
and the free disk space, and names the command that fixes anything wrong.

---

## `pn-ultramemory: command not found`

The binary is not in a folder on your `PATH`. The installer puts it in `~/.local/bin` and prints the
line to add when that folder is missing; add it to `~/.zshrc` or `~/.bashrc` and **open a new
terminal**:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Until then, `~/.local/bin/pn-ultramemory` works by its full path. An agent that starts the server
uses the full path `install` wrote, so it does not depend on your `PATH`.

## macOS says it cannot verify the developer

A binary downloaded with a browser is quarantined by macOS; the installer and `curl` do not have this
problem. Clear the flag on the binary you downloaded:

```bash
xattr -d com.apple.quarantine /path/to/pn-ultramemory
```

## My agent does not see the tools

**Restart it.** An MCP server is read when the agent starts. This is the most common cause by far.

If `doctor` says the agent is *not configured*, the install did not write to it. Run
`pn-ultramemory install --agents <id> --dry-run` to see what it would do, and where.

## `recall` returns nothing, or the wrong thing

```bash
pn-ultramemory stats
```

- `files: 0`: the index is empty. Run `pn-ultramemory index` from the project root.
- Few symbols for many files: the language goes through the lexical fallback. Search still works;
  the graph is thinner.
- The right code exists but is not found: use the words the code uses. The search is over words and
  nothing is translated. `--explain` shows why each symbol was chosen.

## A file of mine is missing

The indexer honours `.gitignore`:

```bash
git check-ignore -v path/to/file
```

Files over 2 MB and minified files are skipped on purpose.

## The answer looks out of date

Index again; only what changed is read:

```bash
pn-ultramemory index
```

If `recall` declines to show a symbol because its file changed since it was indexed, that is the
tool refusing to show code that no longer matches its description. Index and ask again.

## The first `index` after an upgrade reads everything

Expected, once. When a new version needs information the old index never recorded, it marks every
file as changed so that the next `index` records it. Memories are kept.

## `impact` lists fewer callers than before

Calls whose receiver does not point at the symbol (a framework's `$request->validate()`, a vector's
`.is_empty()`) are now `Guess` and left out by default. To see every edge, including the guesses:

```bash
pn-ultramemory impact <symbol> --min-confidence guess
```

## The brain does not open, or stays black

- Run it at a terminal, or open the file it printed (`written:`) yourself. With `--no-open`, or when
  the output is not a terminal, it only writes the file.
- A black page that says WebGL is off: turn hardware acceleration on in the browser settings.
- Slow on a very large repository: draw fewer symbols, for example `--max-nodes 3000`.
- An empty brain that tells you to index: there is no index for this repository yet.

## `install` touched agents I do not use

```bash
pn-ultramemory uninstall --agents <id>
```

It removes exactly the entry it wrote. Next time, pass `--agents`, and `--dry-run` first.

## Escape codes appear in a file

Colour is only used when the output is a terminal, so a pipe or a redirect should never contain
escapes. If one does, please report it. To turn colour off everywhere:

```bash
NO_COLOR=1 pn-ultramemory stats
```

## It is slower than expected

The first index reads every file; after that only changes are read. If every run is slow, something
forces a full index: check you are not passing `--force`, and that the data directory is writable
(`doctor` says).

## Windows: the binary does not start

The release is built for `x86_64-pc-windows-msvc` and needs the Microsoft Visual C++ runtime, which
most systems have. If it is missing, install the
[Visual C++ Redistributable](https://aka.ms/vs/17/release/vc_redist.x64.exe).

## Linux: `GLIBC_2.xx not found`

The release is built against glibc on a recent Ubuntu. On an older distribution, or on Alpine
(musl), build from source with `cargo build --release`.

## Something else

Every error message names a command that would fix it; if one does not, that is a defect worth
reporting at <https://github.com/paulnewmandev/pn-ultramemory/issues>. Include the output of
`pn-ultramemory doctor`, the exact command, and what you expected.
