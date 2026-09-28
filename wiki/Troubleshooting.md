# Troubleshooting

Start here:

```bash
pn-ultramemory doctor
```

It reports the repository it found, the data directory, the index and its age, which agents are
configured, and free disk space — with a hint on anything wrong.

---

## My agent does not see the tool

**Restart it.** An MCP server is read at startup. This is the most common cause by a distance.

Then check the entry is really there:

```bash
pn-ultramemory doctor
```

If it says an agent is *not configured*, the install did not write to it. Run
`pn-ultramemory install --agents <id> --dry-run` to see what it would do and where.

## `recall` returns nothing

```bash
pn-ultramemory stats
```

If `files: 0`, the index is empty — run `pn-ultramemory index` from the project root, not from a
subdirectory.

If files are indexed but symbols are few, your language is probably going through the lexical
fallback. Search still works; the graph is thinner.

## A file of mine is missing

The indexer honours `.gitignore`. Check the path is not ignored:

```bash
git check-ignore -v path/to/file
```

Files above 2 MB and minified files are skipped on purpose.

## The answer looks stale

Re-index. It re-reads only what changed:

```bash
pn-ultramemory index
```

If a `recall` refuses with a message about a file having changed since it was indexed, that is the
tool declining to show you code that no longer matches its description. Re-index and ask again.

## `install` touched agents I do not use

```bash
pn-ultramemory uninstall --agents <id>
```

It removes exactly the entry it wrote. Use `--agents` on install next time; `--dry-run` shows what
would happen first.

## The colours are wrong, or escapes appear in a file

Colour is only applied when the output is a terminal, so a pipe or a redirect should never contain
escapes. If it does, that is a defect worth reporting. To turn colour off everywhere:

```bash
NO_COLOR=1 pn-ultramemory stats
```

## It is slower than I expected

The first index reads every file. After that only changes are read. If every run is slow, something
is forcing a full re-index — check you are not passing `--force`, and that the data directory is
writable (`doctor` says).

## Windows: the binary will not start

The release binary is built for `x86_64-pc-windows-msvc` and needs the Microsoft Visual C++
runtime, which most systems already have. If it is missing, install the
[Visual C++ Redistributable](https://aka.ms/vs/17/release/vc_redist.x64.exe).

## Linux: `GLIBC_2.xx not found`

The release binary is built against glibc on a recent Ubuntu. On an older distribution, or on Alpine
(which uses musl), build from source instead:

```bash
cargo build --release
```

## Something else

Every error message names a command that would fix it. If one does not, that is a defect worth
reporting: <https://github.com/paulnewmandev/pn-ultramemory/issues>

Useful to include: the output of `pn-ultramemory doctor`, the exact command you ran, and what you
expected instead.
