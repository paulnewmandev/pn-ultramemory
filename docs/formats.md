# Output formats

Every command that returns structured results can print it in one of three formats, selected with
`--format` (or `-f`): **`toon`** (the default), `json` and `text`. The MCP server answers with the
`toon` form.

| Format | What it is | Use it for |
|---|---|---|
| `toon` | [TOON](https://github.com/toon-format/spec) (Token-Oriented Object Notation), spec 4.1. Structure in indentation, arrays of uniform rows declared once. | Feeding a coding agent. It is the default because it costs the fewest tokens of the structured formats. |
| `json` | Plain JSON, one document, pretty-printed when the output is a terminal. | Scripts and other programs. |
| `text` | Short human-readable lines. | Reading in a terminal. |

## Why TOON is the default

A capsule is mostly a list of symbols that all have the same fields. JSON repeats every field name
on every row, and TOON says them once:

```text
symbols[3]{id,f,lines,kind,name,d,text}:
  1042,0,10-58,function,parse_config,L1,"pub fn parse_config(path: &str) -> Result<Config, Error>"
  1043,0,60-71,function,load_defaults,L1,pub fn load_defaults() -> Config
  1051,2,5-19,method,Config::validate,L2,"pub fn validate(&self) -> bool - Checks every field."
```

**Measured**, on a realistic capsule (40 symbols taken from real Rust, TypeScript and Python
sources, 33 files, 4 memories), counting tokens with the `cl100k_base` tokenizer as a proxy:

| Representation | Tokens | Against TOON (comma) |
|---|---|---|
| JSON, pretty-printed (indent 2) | 3 667 | +72 % |
| JSON, compact, one object per row | 2 613 | +22 % |
| **TOON, comma delimiter (default)** | **2 134** | reference |
| TOON, tab delimiter | 2 102 | -1.5 % |
| TOON, pipe delimiter | 2 241 | +5 % |
| JSON, compact, **columnar** (field names once, rows as arrays) | 2 055 | -3.7 % |

What this does and does not show:

- TOON saves about **18 % against compact one-object-per-row JSON** and about **42 % against
  pretty-printed JSON**, which is what most tools return.
- A JSON document that a person deliberately designs in columnar form is about as small, and here
  a little smaller (2-4 %). TOON's advantage over that is not size: it is a **standard** with
  explicit array lengths (which expose truncation), deterministic quoting and a readable layout,
  with no bespoke schema for the reader to learn.
- The tab delimiter is marginally cheaper because signatures contain commas and so need quoting.
  The default stays comma, which is what the specification and most implementations use. Choose
  another with `--delimiter comma|tab|pipe`.
- `cl100k_base` is **not** the tokenizer of every model, so treat the percentages as indicative.
  Whether a model reads TOON as accurately as JSON has not been measured for this project.

## The capsule

`recall` returns a **capsule**: the code and memories that answer a query, packed to a token
budget. Its `toon` form has these parts, all optional except `capsule`:

```text
capsule:
  query: parse config
  budget: 1500
  used: 1288
  omitted: 12
files[2]{f,path}:
  0,crates/index/src/extract/rust.rs
  1,crates/core/src/language.rs
symbols[3]{id,f,lines,kind,name,d,text}:
  ...
memories[1]{id,kind,stale,text}:
  7,decision,no,Use iterative traversal to avoid stack overflow on deeply nested input
calls[2]{from,to,confidence}:
  extract_rust,walk_tree,resolved
```

| Part | Meaning |
|---|---|
| `capsule` | The query, the token budget, how many tokens the capsule uses, and how many candidates did not fit (`omitted`). |
| `files` | A table of the files mentioned, so a path is written once and rows refer to it by number `f`. |
| `symbols` | One row per symbol: identity `id` (use it with `expand`), file `f`, line range, `kind`, qualified `name`, the level of detail `d` and the `text` shown at that level. |
| `memories` | Memories anchored to the symbols shown, with their `kind`, and `stale` set to `yes` when the code they describe changed since they were written. |
| `calls` | Relationships between the symbols shown, with how sure the indexer is about each. |

The level of detail `d` is one of `L0` name only, `L1` signature, `L2` signature and one-line
summary, `L3` signature and the names it calls, `L4` full source.

**Source blocks.** Source code does not fit a one-line table cell without escaping every newline,
which costs tokens. Symbols shown at `L4` therefore put the marker `@1`, `@2`, ... in `text`, and
the raw code follows the TOON document, each block introduced by a header line:

```text
@1 crates/index/src/extract/rust.rs:10-58 extract_rust
pub fn extract_rust(source: &str) -> FileExtract {
    ...
}
```

The `json` format carries the same information as one JSON object with the source inline, and the
`text` format prints one block per symbol.

## The outline

`outline` describes one whole file. The `file` block carries what the description cost and what
reading the file would have cost, so the saving is a number the caller can see rather than a claim
this page makes.

```
file:
  path: crates/codec/src/tokens.rs
  lang: rust
  lines: 274
  symbols: 21
  detail: documented
  tokens: 1061
  whole_file_tokens: 2669
  saved: 0.602
symbols[21]{id,line,end,depth,kind,name,vis,sig,doc}:
  9155988507721105323,40,55,0,struct,Features,private,struct Features,The counts the estimate is built from.
  8835725579113062470,113,147,1,method,of,private,"fn of(text: &str) -> Self",Counts the features of a text in one pass.
```

| Field | Means |
|---|---|
| `detail` | `documented`, `signature` or `name` — the richest level that fitted |
| `depth` | How deeply the symbol nests; zero at the top of the file |
| `saved` | `1 - tokens / whole_file_tokens`, or absent when the file could not be read |
| `over_budget` | Present only when even bare names did not fit. Every symbol is still listed |
| `cheaper_to_read` | Present only when the file is short enough that reading it costs less |

**An outline never drops a symbol.** A tight budget lowers the detail instead, because a skeleton
missing three functions reads as *they are not there*.

## Converting

`pn-ultramemory toon encode` reads JSON on standard input and writes TOON. `pn-ultramemory toon
decode` does the reverse. Both accept `--delimiter` and `--indent`, and `decode` accepts
`--lenient` to skip the strict checks of the specification.
