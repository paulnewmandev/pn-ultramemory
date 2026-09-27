# Glossary

The shared vocabulary of the project. Code, documentation and discussions use these words with
exactly these meanings.

| Term | Meaning |
|---|---|
| **Adapter** | Code that connects the domain to the outside world (a store, a parser, a protocol). Entry adapters receive requests, exit adapters implement ports. |
| **Anchor** | The link between a memory and the symbol or file it describes. It records a hash of the symbol so that a change can be detected. |
| **Budget** | The maximum number of tokens a capsule may use. |
| **Candidate** | A symbol that may enter a capsule, with every level it can be shown at. |
| **Capsule** | The context handed to an agent for one question: symbols at chosen levels of detail, plus relevant memories. |
| **Card** | The compact, deterministic description of a symbol: kind, name, signature, first documentation sentence, visibility, location, content hash. |
| **Confidence** | How sure the indexer is that an edge exists: `guess`, `heuristic`, `resolved` or `exact`. |
| **Convention** | A rule the codebase follows, promoted from lessons that repeat. |
| **Detail** | The level at which a symbol is rendered, from L0 (`Name`) to L4 (`Source`). |
| **Edge** | A relationship between two nodes, such as a call, an import or an anchor. |
| **Envelope** | The concave hull of a candidate's (tokens, value) options. Only its vertices are worth choosing in the greedy pass. |
| **Learned edge** | A relationship inferred from how agents used symbols together. It is stored separately and never treated as fact. |
| **Memory** | A recorded piece of knowledge: a decision, fact, lesson, dead end, error fix, convention, task or session. |
| **Node** | A file, symbol, community or memory in the graph. |
| **Pack (verb)** | To choose a level of detail for each candidate so total value is maximal within the budget. |
| **Pack (noun), language pack** | A declarative description of one language: its grammar, queries, comment syntax and fixtures. |
| **Port** | An interface defined by the domain that an adapter implements. |
| **Provenance** | Who produced a memory: `tool`, `agent` or `user`. |
| **Relevance** | How closely a candidate relates to the current question. It multiplies utility. |
| **Signal** | An observation used for learning: an item was used, ignored, corrected, or preceded passing tests. |
| **Stale** | A memory whose anchored symbol has changed since the memory was written. |
| **Symbol** | A named program element: function, method, class, type, constant, route, test and so on. |
| **Utility** | How useful an option is on its own, before relevance is applied. In learning, how useful an item has proved to be when recalled. |

**Outline.** A whole file described completely: every symbol it declares, in order, nested, with
signatures and first documentation sentences. Distinct from a capsule, which answers *which code
matters* by dropping symbols; an outline answers *what is in here* and never drops one.

**Fibre.** An edge as the graph draws it: a filled shape, wide where it leaves the caller and
tapering to a point where it arrives. The taper carries the direction, which is why the drawing has
no arrowheads.

**Bundle.** A group of fibres leaving one node in nearly the same direction, or arriving at one node
from nearly the same side, drawn along a shared direction. Bundling by target is what makes a hub
read as a cell body receiving an arbor rather than as a star.

**Cleft.** The gap a fibre stops short of the node it points at, and where its terminal sits. A
fibre touching its target reads as a wire soldered on.

**Cost class.** Whether a measurement is flat in the repository size or grows with it. Asserted as a
contract in the test suite, each flat claim paired with a control that must grow, so a test that
stopped measuring anything fails rather than passing quietly.

**Refill.** The pass that hands unspent budget back to the packer after a capsule is known to fit.
The frame around the symbols has to be reserved before anything is packed, and that estimate is
generous on purpose; without the refill a budget of 200 tokens returned nothing while 174 of them
went unspent.
