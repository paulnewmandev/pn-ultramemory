# Quality harness

The compiler, clippy and the test suite hold three kinds of promise. This page describes the guards
that hold the ones they cannot: that an error message names a way forward, that library code does
not abort the process, that a docstring says more than the item's name, that the tests a release
depends on still exist, and that the suite makes no network calls.

The guards live in [`xtask`](../xtask), a development binary. It is never published and never
shipped, so its two dependencies (`syn` and `proc-macro2`) add nothing to what a user installs. They
exist so the guards read Rust the way the compiler does, rather than by matching text.

**Contents**

1. [The guards](#the-guards)
2. [Baselines are ratchets](#baselines-are-ratchets)
3. [Regenerating a baseline](#regenerating-a-baseline)
4. [The refusal shapes](#the-refusal-shapes)
5. [Release blockers](#release-blockers)
6. [The offline proof](#the-offline-proof)
7. [What these guards do not prove](#what-these-guards-do-not-prove)

## The guards

| Guard | Run it with | It enforces | Baseline |
|---|---|---|---|
| Headers | `cargo run -p xtask -- headers` | Every source file has its SPDX line and, for Rust, `//!` module documentation. | none |
| Refusal | `cargo run -p xtask -- refusal` | Every error message names a way forward. | `.ratchet/refusal.txt` |
| Panics | `cargo run -p xtask -- panics` | Library code does not `unwrap`, index or truncate. | `.ratchet/panics.txt` |
| Docs | `cargo run -p xtask -- docs` | A public item's first documented sentence adds to its name. | `.ratchet/docs.txt` |
| Release blockers | `cargo run -p xtask -- verify-tests .ratchet/release-blockers.txt` | Every test named as release-blocking still exists. | `.ratchet/release-blockers.txt` |
| Offline | `sh scripts/check-offline` | The test suite makes no network calls. | none |

Exit codes are the same for all of them: **0** passed, **1** the guard failed or could not do its
job, **2** the command line was wrong.

[`scripts/ratchet`](../scripts/ratchet) runs the whole set in one command, which is the usual way to
check a change locally:

```sh
sh scripts/ratchet              # run every guard, as CI does
sh scripts/ratchet --update     # regenerate every baseline
sh scripts/ratchet --diff docs  # show how one baseline would change, changing nothing
```

`headers` is a Rust reimplementation of [`scripts/check-headers`](../scripts/check-headers), which
remains the gate of record. The shell script does not run on Windows; the subcommand does, so the
whole harness is usable on every platform a contributor might have.

All of these run in the `guards` workflow, which **must be configured as a required status check on
`main`**. A guard that cannot block a merge is decoration.

## Baselines are ratchets

pn-ultramemory did not start with these guards, so it already has findings. A baseline records them
once, and the guard then blocks anything new. Four things can happen on a run:

| What happened | What the guard does | Why |
|---|---|---|
| A finding is not in the baseline | **Fails** | This is the guard's whole purpose. |
| A finding in the baseline is still there | Nothing | Steady state. |
| A baseline entry no longer occurs, and its file *was* analysed | **Notes it** and asks you to tighten the baseline | The guard looked and did not find it, so the improvement is real. |
| A baseline entry's file was *not* analysed | **Fails** | "Somebody fixed it" and "the analyser stopped seeing it" look identical from the baseline's side. A guard that cannot tell them apart is lying. |

The last row is the one people find surprising, and it is the most important. If a file is renamed,
deleted, moved out of scope or stops parsing, its baseline entries no longer match anything, and
nothing proves whether the findings were fixed. The guard says so instead of going quiet.

A guard that enumerates nothing also fails. An empty `crates/` directory, a manifest with no test
names, a header check that examined no file: each of those would otherwise be a green tick that
means nothing.

Two properties make the baselines survivable:

- **No entry carries a line or a column.** A refusal finding is keyed by file and message text; a
  panic finding by file, expression and enclosing function; a documentation finding by file and item
  path. Edit the line above a finding and nothing moves.
- **Entries are sorted by plain byte order.** If you post-process a baseline with shell tools, set
  `LC_ALL=C`. `comm(1)` produces silently wrong output under other collations.

Each baseline file begins with a comment block saying what it is, that it may only shrink, how to
regenerate it and what its guard does not prove. Comment lines are ignored when comparing.

## Regenerating a baseline

```sh
cargo run -p xtask -- refusal --update
cargo run -p xtask -- panics --update
cargo run -p xtask -- docs --update
```

Each rewrites its own baseline from the current tree, prints how the count changed, and exits 0.

**When regenerating is legitimate**

- You fixed findings and want the ratchet to record the smaller number. Do this; leaving the
  baseline loose means the next contributor can reintroduce what you just fixed.
- A file was renamed or moved, so its entries need re-anchoring. Check the count did not grow.
- You added a `refusal:by-design` marker, so a finding is now excused rather than listed.

**When it is not**

- The guard is failing and you want it to stop. That is the guard working.
- The count grew. If a new finding is genuinely acceptable, say why in the pull request; a reviewer
  looking at a baseline diff should be able to see, from the diff alone, whether the ratchet
  tightened or slipped.

## The refusal shapes

Some refusals cannot name a continuation, and pretending otherwise would be worse than silence. A
comment beside the message excuses it:

```rust
// refusal:by-design <shape>: <reason>
```

The marker must sit on the message's own line or on one of the three lines above it. The vocabulary
is **closed**: exactly three shapes. An unrecognised shape is a hard failure, not a warning,
because a typo would otherwise suppress silently. So is a marker with no reason, and so is a marker
that excuses nothing — a suppression that suppresses nothing is the same failure as a baseline
entry that matches nothing.

| Shape | Use it when | Example |
|---|---|---|
| `operator-knowledge` | Only the operator knows what to do next. | `// refusal:by-design operator-knowledge: only the operator knows which root is intended` beside `"several directories look like the project root"` |
| `world-action` | The fix is outside this tool: a file, a permission, a machine. | `// refusal:by-design world-action: freeing disk space happens outside this tool` beside `"the disk holding the database is full"` |
| `human-authority` | A person must decide, and naming a command would push them past the decision. | `// refusal:by-design human-authority: discarding stored memories is a person's decision` beside `"this would discard 412 stored memories"` |

A message counts as naming a continuation when it does any of these:

1. names a subcommand, matching `pn-ultramemory <verb>`;
2. contains a backticked command or flag, such as `` `cargo fmt --all` `` or `` `--force` ``;
3. uses the word *run* with a backticked span within 40 characters after it;
4. uses an instruction verb — *pass*, *use*, *set*, *try*, *install*, and a few more — with a
   backticked span within 40 characters after it.

Messages are collected by **carrier**, not by constructor, because the same sentence reaches a
reader by several routes: a `#[error("...")]` attribute; a literal inside the `Display`
implementation of a type whose name ends in `Error`; a literal handed to a constructor whose path
ends in `::invalid`, `::NotFound`, `::Corrupt` and the like; and a literal initialising a field
named `reason`, `hint` or `detail`.

## Release blockers

[`.ratchet/release-blockers.txt`](../.ratchet/release-blockers.txt) names the tests whose absence
means the product is broken: the TOON conformance suite, the packer against its exact solver, the
store against its reference rules, the MCP server against hostile input, and the index under
hostile and enormous sources.

It exists because **`cargo test <name>` exits 0 when the filter matches nothing**. Rename or delete
a test and every command that named it keeps reporting success while running nothing. `verify-tests`
asks cargo which tests exist and fails when a name in the manifest is not among them.

Editing the manifest is legitimate when a test is deliberately renamed, split or replaced: name
whatever took over the responsibility. Deleting a line because the check is red is not — the guard
is telling you that responsibility is now uncovered.

## The offline proof

[`scripts/check-offline`](../scripts/check-offline) runs the test suite inside a network namespace
with no route anywhere. It does three things in this order:

1. finds out whether the platform can create a namespace (`unshare -rn`, Linux only);
2. **proves the namespace really isolates the network**, by running a command inside it that would
   reach a well-known address and requiring it to fail. If it succeeds, the script exits 2, because
   the check would otherwise pass vacuously;
3. runs `cargo test --workspace --locked --offline` inside the namespace.

On macOS and Windows there is no `unshare`, so the script prints a distinct `UNAVAILABLE` note and
exits 0. It never passes silently. The blocking run is the `offline` job of the workflow, on Linux.

## Cost is a contract, not a benchmark

The central claim of this tool is that a question costs about the same to answer in a large
repository as in a small one. A timing benchmark cannot hold that claim: it is noisy, it is
machine-dependent, and a regression hides inside its error bars.

`crates/engine/tests/cost.rs` holds it instead. Each test sweeps the repository size, measures a
number the tool already reports, and asserts that the number belongs to a complexity class:

| Asserted flat in repository size | |
|---|---|
| The tokens a `recall` spends at a fixed budget | If this grew, the saving would be an artefact of the repository being small |
| The tokens a `map` spends at a fixed budget | |
| The tokens an `outline` spends on one file | An outline costs what its own file costs, not what the repository around it costs |
| Files re-read when nothing changed | Asserted as exactly zero |

### The control

A flat assertion on its own proves nothing: a measurement that is always zero is also flat. Every
flat claim is therefore paired with a measurement over the **same sweep** that must **grow** — the
candidates a recall considers, the files a map leaves out. A test that stopped measuring anything
then fails instead of passing quietly.

Nothing here is a stopwatch, so nothing here is flaky.

### What it does not prove

It says nothing about wall-clock time, memory use, or behaviour past the largest size swept. It
proves the shape of the cost curve over that range and nothing beyond it.

## What these guards do not prove

Honesty about limits is part of the design. A guard whose limits are not written down gets trusted
for things it never checked.

**The refusal ratchet**

- It does not prove a named continuation is the **right** one, only that one is named. A message
  telling the reader to run a command that does not exist satisfies it.
- It sees only the four carriers above. A message reaching a reader by any other route — assembled
  at run time, arriving from a dependency, held in a data file — is invisible to it.
- Two identical messages in one file collapse into one baseline entry, because the key holds no line
  number.
- It skips test code, on the grounds that a message a test prints cannot reach a user.

**The panic ratchet**

- It has **no type information**. It cannot distinguish a slice index from a map index, nor a
  widening cast from a truncating one, so it records every index expression and every integer cast.
  Over-reporting into a baseline is safe; under-reporting is not.
- It does not prove the absence of panics. Arithmetic overflow, division by zero, `RefCell` borrows,
  stack exhaustion and panics inside dependencies are all invisible to it.
- It overlaps with clippy on `unwrap`, `expect` and the panicking macros, which clippy already
  denies. Its own contribution is indexing and casts.
- It skips test code, and doctests live in comments, so they are never seen.

**The documentation ratchet**

- It cannot judge whether a docstring is **true**, **current** or **useful**; only that it adds a
  word to the item's name. "Parses the configuration into a banana" passes.
- Its stemmer is three suffix rules and a trim, not linguistics. It will accept a restatement in an
  unusual inflection, and occasionally flag a short but genuine sentence. Record the latter in the
  baseline and move on.
- It reads only the first sentence, and only public items under `crates/*/src`.

**The header gate**

- It checks that a line of module documentation exists, not that it is any good.
- It takes its file list from `git ls-files`, so a file git ignores is not checked.

**The release-blocker manifest**

- It does not run the tests, so it says nothing about whether they pass.
- It cannot prove the list is **complete**. No tool can: that is a judgement, written down so it can
  be held to account.
- Matching is by substring, exactly as `cargo test` matches, so a name that happens to be part of an
  unrelated test is satisfied by that unrelated test.

**The offline proof**

- It proves the paths **the tests take** make no network calls. An untested path could open a socket
  and the check would stay green.
- It says nothing about the release binary, which is built with different settings, nor about a
  dependency's build script, which runs before the namespace is entered.
- It cannot run on macOS or Windows at all.

**All of them together**

- They prove nothing about correctness, performance or security. They hold promises a reviewer would
  otherwise have to check by hand, every time, for ever — and that is all.
