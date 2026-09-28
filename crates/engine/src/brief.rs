// SPDX-License-Identifier: Apache-2.0
//! What a session needs to know about a repository before it is any use, in one budgeted answer.
//!
//! # The problem
//! A model's memory does not survive its session. Close the window, switch tools, hit a context
//! limit, and everything learned about the codebase is gone — including what a person spent an
//! hour explaining. The next session starts from nothing and rediscovers it by reading files,
//! which is the expensive thing this tool exists to avoid.
//!
//! What does survive is the graph on disk, and the memories anchored into it. A brief is those two
//! turned into the shortest honest answer to *what is this project*:
//!
//! 1. **Shape** — how large, in which languages, with how much known about it already.
//! 2. **Structure** — the modules, ordered by how much of the repository leans on them.
//! 3. **Where things happen** — the symbols most called, which are the de facto entry points.
//! 4. **What has already been decided** — the memories, stale ones marked as such.
//! 5. **What to ask next** — the exact commands, so a session does not have to guess.
//!
//! Section 4 is the one that cannot be recovered any other way. Structure can be re-derived by
//! reading code; a decision and its reason cannot. That is why it is printed after the structure
//! and given up before it: a reader needs the shape of a repository to place a decision inside it,
//! but if the budget will only carry one of the two, the decision is the one worth carrying.
//!
//! # Why it is budgeted like everything else
//! A briefing that costs three thousand tokens every time a session opens is a tax, not a saving.
//! Sections are printed in the order above and given up in a different one, which
//! [`Section::keep`] sets: the busiest symbols go first because reading the code finds them again,
//! then the modules, and what someone already decided goes last.
//!
//! # What it is not
//! Not a summary of the code: nothing here is generated prose, and nothing is inferred. Every line
//! is a number the index measured or a sentence a person wrote.

use serde_json::{Value, json};

use pn_ultramemory_codec::estimate_tokens;

use crate::engine::Engine;
use crate::error::EngineError;
use crate::insights::InsightOptions;

/// The budget a brief uses when the caller names none.
pub const DEFAULT_BRIEF_BUDGET: u32 = 1200;

/// The most modules listed, before the budget has any say.
const MAX_MODULES: usize = 12;

/// The most central symbols listed.
const MAX_CENTRAL: usize = 10;

/// The most memories listed.
const MAX_MEMORIES: usize = 12;

/// The fewest rows a section keeps before it is dropped entirely.
///
/// Two rows of a table plus its header say less than the header costs, so a section trimmed this
/// far is removed instead: the space is worth more to the section below it.
const MIN_ROWS: usize = 2;

/// What a session is told about a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brief {
    /// The repository's folder name.
    pub name: String,
    /// The value as it will be printed.
    value: Value,
    /// What this costs, measured over the printed form.
    pub tokens: u32,
    /// The budget it was built for.
    pub budget: u32,
}

impl Brief {
    /// The brief as a structured value.
    #[must_use]
    pub fn to_value(&self) -> Value {
        self.value.clone()
    }
}

/// One section of a brief, which the budget may shorten or drop.
struct Section {
    /// The key it is printed under.
    key: &'static str,
    /// Its rows, richest first, so trimming takes from the end.
    rows: Vec<Value>,
    /// What the budget gives up first. The lowest goes first.
    ///
    /// This is deliberately not the order the sections are printed in. A reader wants the shape of
    /// the repository before the decisions taken inside it, so structure is printed first; but
    /// structure can be re-derived by reading code and a decision cannot, so under pressure the
    /// structure is what goes.
    keep: u8,
}

impl Engine {
    /// Everything a session needs to know about this repository, inside a budget.
    ///
    /// `name` is what to call the repository. The engine reaches its files through a port and has
    /// no path of its own, which is the point of the port, so the caller names it.
    ///
    /// `budget` is a number of tokens; `None` means [`DEFAULT_BRIEF_BUDGET`]. The answer always
    /// carries the repository's shape and what to ask next, however small the budget, because a
    /// brief that says nothing is worse than no brief.
    ///
    /// # Errors
    /// Returns whatever the storage reports.
    ///
    /// # Examples
    /// ```text
    /// let brief = engine.brief("my-project", Some(800))?;
    /// assert!(brief.tokens <= 800);
    /// ```
    pub fn brief(&self, name: &str, budget: Option<u32>) -> Result<Brief, EngineError> {
        let budget = budget.unwrap_or(DEFAULT_BRIEF_BUDGET);
        let insights = self.insights(&InsightOptions {
            module_depth: 2,
            max_hotspots: MAX_CENTRAL,
            max_memories: MAX_MEMORIES,
            max_undocumented: 0,
        })?;
        let index = &insights.stats.index;

        let name = name.trim();
        let name = if name.is_empty() { "repository" } else { name };

        let languages: Vec<String> = insights
            .languages
            .iter()
            .take(4)
            .map(|row| format!("{} {}", row.name, row.files))
            .collect();

        let mut head = serde_json::Map::new();
        head.insert("repo".into(), Value::from(name));
        head.insert("files".into(), Value::from(index.files));
        head.insert("symbols".into(), Value::from(index.symbols));
        head.insert("edges".into(), Value::from(index.edges));
        head.insert("languages".into(), Value::from(languages.join(", ")));
        head.insert("memories".into(), Value::from(index.memories));
        if index.stale_memories > 0 {
            head.insert("stale_memories".into(), Value::from(index.stale_memories));
        }
        if index.files == 0 {
            head.insert(
                "note".into(),
                Value::from(
                    "this repository has no index yet; run `pn-ultramemory index` and ask again",
                ),
            );
        }

        let sections = sections_of(&insights);

        let next = vec![
            Value::from("recall \"<question>\" -b 800"),
            Value::from("outline <path>"),
            Value::from("impact <symbol>"),
            Value::from("memories --stale"),
        ];

        let (value, tokens) = fit(&head, sections, &next, budget);
        Ok(Brief {
            name: name.to_owned(),
            value,
            tokens,
            budget,
        })
    }
}

/// The sections of a brief, in the order they are printed.
fn sections_of(insights: &crate::insights::Insights) -> Vec<Section> {
    // Printed in the order a reader needs them; given up in the order `keep` sets.
    vec![
        Section {
            keep: 1,
            key: "modules",
            rows: insights
                .modules
                .iter()
                .take(MAX_MODULES)
                .map(|m| {
                    json!({
                        "name": m.name,
                        "files": m.files,
                        "symbols": m.symbols,
                        "in": m.incoming,
                        "out": m.outgoing,
                    })
                })
                .collect(),
        },
        Section {
            keep: 0,
            key: "central",
            rows: insights
                .hotspots
                .iter()
                .take(MAX_CENTRAL)
                .map(|h| {
                    json!({
                        "name": h.name,
                        "kind": h.kind.as_str(),
                        "path": h.path,
                        "line": h.line,
                        "callers": h.callers,
                    })
                })
                .collect(),
        },
        Section {
            keep: 2,
            key: "known",
            rows: insights
                .memories
                .iter()
                .take(MAX_MEMORIES)
                .map(|m| {
                    json!({
                        "id": m.id.0,
                        "kind": m.kind.as_str(),
                        "stale": m.stale_since.is_some(),
                        "text": m.text,
                    })
                })
                .collect(),
        },
    ]
}

/// Assembles the brief, trimming from the last section until it fits.
///
/// The head and `next` are never trimmed: the first says what the repository is and the second says
/// how to ask for more, and a brief without either is not worth its own header.
fn fit(
    head: &serde_json::Map<String, Value>,
    mut sections: Vec<Section>,
    next: &[Value],
    budget: u32,
) -> (Value, u32) {
    loop {
        let value = compose(head, &sections, next);
        let tokens = measure(&value);
        if tokens <= budget {
            return (value, tokens);
        }
        // Take a row from the section the budget gives up first; drop one that is down to its
        // floor, because a table of one row costs more in its header than it carries.
        // Borrowed straight out of the iterator rather than taken by a position computed from it:
        // the two forms are the same here, and this one cannot become wrong if the list changes
        // between finding the section and shortening it.
        let Some(section) = sections
            .iter_mut()
            .filter(|section| !section.rows.is_empty())
            .min_by_key(|section| section.keep)
        else {
            return (value, tokens);
        };
        if section.rows.len() <= MIN_ROWS {
            section.rows.clear();
        } else {
            section.rows.pop();
        }
        if sections.iter().all(|section| section.rows.is_empty()) {
            let bare = compose(head, &sections, next);
            let cost = measure(&bare);
            return (bare, cost);
        }
    }
}

/// Builds the value from its parts, leaving out any section the budget emptied.
fn compose(head: &serde_json::Map<String, Value>, sections: &[Section], next: &[Value]) -> Value {
    let mut root = serde_json::Map::new();
    root.insert("brief".into(), Value::Object(head.clone()));
    for section in sections {
        if !section.rows.is_empty() {
            root.insert(section.key.to_owned(), Value::Array(section.rows.clone()));
        }
    }
    root.insert("next".into(), Value::Array(next.to_vec()));
    Value::Object(root)
}

/// What a brief costs, measured over the text that will actually be printed.
fn measure(value: &Value) -> u32 {
    let text = pn_ultramemory_toon::encode(value, &pn_ultramemory_toon::EncodeOptions::default());
    estimate_tokens(&text)
}
