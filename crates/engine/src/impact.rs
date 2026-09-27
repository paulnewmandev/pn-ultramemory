// SPDX-License-Identifier: Apache-2.0
//! Blast-radius analysis: who depends on a symbol, and how sure we are.
//!
//! [`Engine::impact`] walks **incoming** edges from one symbol, breadth first, and reports the
//! symbols that would feel a change to it, the files they live in and which of those files are
//! tests. Every caller carries the depth it was found at and the weakest confidence on the path
//! that led to it, so a reader can tell a fact from a guess without leaving the report.
//!
//! # The epistemic rule
//! The point of this operation is not the list, it is the honesty of the list. An index built
//! without a language server cannot see every caller: dynamic dispatch, reflection, a build script,
//! another repository. [`Epistemic`] therefore qualifies every answer:
//!
//! | Value | When | What it means |
//! |---|---|---|
//! | [`Epistemic::Unknown`] | no caller was found **and** the symbol is `Public` or its visibility is unknown | Nothing was found, and a caller outside the index may well exist. Saying "nothing depends on this" would be a lie. |
//! | [`Epistemic::Exact`] | every edge on every traversed path is `Resolved` or `Exact`, the symbol is `Private`, and the list was not cut short | The list is the whole answer: a private symbol can only be called from inside its own module, and every step was resolved by the syntax or by scopes. |
//! | [`Epistemic::LowerBound`] | anything else | What is listed does depend on the symbol; more may. |
//!
//! A symbol with no callers is never reported as safe to change. A private symbol with no callers
//! is [`Epistemic::Exact`] with an empty list, which *is* the strong claim, and a public one is
//! [`Epistemic::Unknown`], which is not.
//!
//! # What the report tells the caller to do next
//! [`ImpactReport::next`] holds **literal commands**, not advice: `pn-ultramemory expand 42`,
//! `pn-ultramemory recall "Config::validate"`. A program that hands a model the exact command to
//! run beats prose the model has to interpret, and it costs fewer tokens.
//!
//! # Determinism
//! Traversal order comes from [`pn_ultramemory_core::Storage::neighbors`], which is ordered, and
//! every list is sorted with explicit tie-breaks, so the same index always gives the same report.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Instant;

use pn_ultramemory_core::{Confidence, Direction, EdgeKind, SymbolId, SymbolRecord, Visibility};
use serde_json::{Map, Value, json};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::metrics::Event;

/// How many steps of callers are followed when the caller asks for no depth.
pub const DEFAULT_IMPACT_DEPTH: u32 = 2;

/// The deepest traversal accepted; a deeper request is lowered to it.
pub const MAX_IMPACT_DEPTH: u32 = 5;

/// How many callers are listed when the caller asks for no limit.
pub const DEFAULT_IMPACT_LIMIT: usize = 50;

/// The most same-name alternatives offered.
const MAX_ALTERNATIVES: usize = 5;

/// The most commands suggested in [`ImpactReport::next`].
const MAX_NEXT: usize = 3;

/// How many same-name symbols are looked up to fill the alternatives.
const ALTERNATIVE_LOOKUP: usize = 16;

/// The most callers read from one symbol in one step. It is one more than the caller's limit, so
/// that reaching the limit can be told apart from exhausting the callers, and it is capped so that
/// a single very popular symbol cannot make one step unbounded.
const MAX_FANOUT: usize = 500;

/// Path components that mark a directory of tests, compared without regard to case.
const TEST_DIRS: [&str; 4] = ["test", "tests", "spec", "__tests__"];

/// Fragments of a file name that mark a test file.
const TEST_FILE_MARKS: [&str; 3] = ["_test.", ".test.", ".spec."];

/// What to analyse, and how far.
///
/// # Examples
/// ```
/// use pn_ultramemory_core::Confidence;
/// use pn_ultramemory_engine::ImpactQuery;
///
/// let query = ImpactQuery {
///     symbol: "load_config".into(),
///     depth: Some(3),
///     min_confidence: Some(Confidence::Resolved),
///     limit: None,
/// };
/// assert_eq!(query.symbol, "load_config");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImpactQuery {
    /// The symbol, named the way [`Engine::resolve_symbol`] accepts: an id, a name, or `path:name`.
    pub symbol: String,
    /// How many steps of callers to follow. `None` means [`DEFAULT_IMPACT_DEPTH`], zero is raised to one
    /// and anything above [`MAX_IMPACT_DEPTH`] is lowered to it.
    pub depth: Option<u32>,
    /// The weakest edge to follow. `None` means [`Confidence::Heuristic`].
    pub min_confidence: Option<Confidence>,
    /// The most callers to list. `None` means [`DEFAULT_IMPACT_LIMIT`].
    pub limit: Option<usize>,
}

/// How much a report's list of callers can be trusted.
///
/// The rule that picks one of the three, and why the operation exists at all, is set out in the
/// documentation of this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Epistemic {
    /// The list is complete: every step was resolved and the symbol cannot be reached from outside
    /// its module.
    Exact,
    /// Everything listed does depend on the symbol, and more may.
    LowerBound,
    /// Nothing was found, and that says nothing: a caller outside the index may exist.
    Unknown,
}

impl Epistemic {
    /// The name used in output, with a hyphen where the variant has a word boundary.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_engine::Epistemic;
    ///
    /// assert_eq!(Epistemic::LowerBound.as_str(), "lower-bound");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::LowerBound => "lower-bound",
            Self::Unknown => "unknown",
        }
    }
}

/// One symbol that depends on the analysed symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    /// The qualified name of the caller.
    pub name: String,
    /// Path of its file, relative to the repository root.
    pub path: String,
    /// The line of its file where the dependency was found.
    pub line: u32,
    /// The weakest confidence on the path from the analysed symbol to this one.
    pub confidence: Confidence,
    /// How many steps away it is; `1` is a direct caller.
    pub depth: u32,
    /// What the edge that reached it means.
    pub kind: EdgeKind,
}

/// The answer to one impact question.
#[derive(Debug, Clone, PartialEq)]
pub struct ImpactReport {
    /// The symbol that was analysed.
    pub symbol: SymbolRecord,
    /// How much the list of callers can be trusted.
    pub epistemic: Epistemic,
    /// How many callers are one step away.
    pub direct: usize,
    /// How many callers were found in total.
    pub total: usize,
    /// The callers, shallowest and most certain first.
    pub callers: Vec<Caller>,
    /// The distinct files of the callers, sorted.
    pub files: Vec<String>,
    /// The subset of `files` that look like tests.
    pub tests: Vec<String>,
    /// Whether the limit cut the list short, so that more callers exist.
    pub truncated: bool,
    /// Other symbols with the same simple name, as `id path qualified_name`, in case the wrong one
    /// was analysed.
    pub alternatives: Vec<String>,
    /// The commands worth running next, at most three.
    pub next: Vec<String>,
}

/// Whether a path looks like a test file: a `test`, `tests`, `spec` or `__tests__` component, or a
/// file name carrying one of [`TEST_FILE_MARKS`].
fn is_test_path(path: &str) -> bool {
    if path
        .split('/')
        .any(|part| TEST_DIRS.iter().any(|dir| part.eq_ignore_ascii_case(dir)))
    {
        return true;
    }
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    TEST_FILE_MARKS.iter().any(|mark| name.contains(mark))
}

/// What the traversal remembers about one symbol it reached.
#[derive(Debug, Clone)]
struct Reached {
    /// The caller as it will be reported.
    caller: Caller,
    /// Its identity, used to order ties and to name it in a command.
    id: SymbolId,
}

/// What the traversal found, before it is shaped into a report.
#[derive(Debug, Default)]
struct Walk {
    /// Every symbol reached, keyed by identity so each is visited once.
    reached: BTreeMap<SymbolId, Reached>,
    /// Whether every edge that was followed is `Resolved` or `Exact`.
    all_structural: bool,
    /// Whether the limit stopped the walk with callers left to find.
    truncated: bool,
}

/// The sort key of a caller: shallowest first, then most certain, then by location and name.
fn caller_key(entry: &Reached) -> (u32, u8, &str, u32, &str, i64) {
    let certainty = match entry.caller.confidence {
        Confidence::Exact => 0,
        Confidence::Resolved => 1,
        Confidence::Heuristic => 2,
        Confidence::Guess => 3,
    };
    (
        entry.caller.depth,
        certainty,
        entry.caller.path.as_str(),
        entry.caller.line,
        entry.caller.name.as_str(),
        entry.id.0,
    )
}

impl ImpactReport {
    /// The report as a structured value, with the callers as one uniform table so that TOON writes
    /// the column names once. Empty lists are left out.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut head = Map::new();
        head.insert(
            "symbol".into(),
            Value::from(self.symbol.qualified_name.as_str()),
        );
        head.insert("id".into(), Value::from(self.symbol.id.0));
        head.insert("kind".into(), Value::from(self.symbol.kind.as_str()));
        head.insert("path".into(), Value::from(self.symbol.path.as_str()));
        head.insert("line".into(), Value::from(self.symbol.span.start_line));
        head.insert(
            "visibility".into(),
            Value::from(self.symbol.visibility.as_str()),
        );
        head.insert("epistemic".into(), Value::from(self.epistemic.as_str()));
        head.insert("direct".into(), Value::from(self.direct));
        head.insert("total".into(), Value::from(self.total));
        if self.truncated {
            head.insert("truncated".into(), Value::from(true));
        }

        let mut root = Map::new();
        root.insert("impact".into(), Value::Object(head));
        if !self.callers.is_empty() {
            let rows: Vec<Value> = self
                .callers
                .iter()
                .map(|caller| {
                    json!({
                        "name": caller.name,
                        "path": caller.path,
                        "line": caller.line,
                        "depth": caller.depth,
                        "kind": caller.kind.as_str(),
                        "confidence": caller.confidence.as_str(),
                    })
                })
                .collect();
            root.insert("callers".into(), Value::Array(rows));
        }
        for (key, list) in [
            ("files", &self.files),
            ("tests", &self.tests),
            ("alternatives", &self.alternatives),
            ("next", &self.next),
        ] {
            if !list.is_empty() {
                let items = list.iter().map(|item| Value::from(item.as_str())).collect();
                root.insert(key.to_owned(), Value::Array(items));
            }
        }
        Value::Object(root)
    }

    /// The report as short human-readable lines, without a trailing newline.
    #[must_use]
    pub fn render_text(&self) -> String {
        let mut lines = vec![
            format!(
                "{} ({}, {}:{}, {})",
                self.symbol.qualified_name,
                self.symbol.kind.as_str(),
                self.symbol.path,
                self.symbol.span.start_line,
                self.symbol.visibility.as_str(),
            ),
            format!(
                "{}: {} direct, {} in total{}",
                self.epistemic.as_str(),
                self.direct,
                self.total,
                if self.truncated { ", cut short" } else { "" },
            ),
        ];
        for caller in &self.callers {
            lines.push(format!(
                "  d{} {} {} {} {}:{}",
                caller.depth,
                caller.confidence.as_str(),
                caller.kind.as_str(),
                caller.name,
                caller.path,
                caller.line,
            ));
        }
        for (title, list) in [
            ("files", &self.files),
            ("tests", &self.tests),
            ("alternatives", &self.alternatives),
            ("next", &self.next),
        ] {
            if list.is_empty() {
                continue;
            }
            lines.push(format!("{title}:"));
            for item in list {
                lines.push(format!("  {item}"));
            }
        }
        lines.join("\n")
    }
}

impl Engine {
    /// Walks incoming edges from `root`, breadth first, to `depth`.
    ///
    /// Each symbol is visited once and keeps the shallowest depth and the weakest confidence seen
    /// on the path to it. The analysed symbol itself is skipped, so recursion does not report a
    /// symbol as its own caller.
    fn walk_callers(
        &self,
        root: &SymbolRecord,
        depth: u32,
        min_confidence: Confidence,
        limit: usize,
    ) -> Result<Walk, EngineError> {
        let mut walk = Walk {
            all_structural: true,
            ..Walk::default()
        };
        if limit == 0 {
            walk.truncated = true;
            return Ok(walk);
        }
        let fanout = limit.saturating_add(1).min(MAX_FANOUT);
        let mut frontier: VecDeque<(SymbolId, u32, Confidence)> =
            VecDeque::from([(root.id, 0, Confidence::Exact)]);
        while let Some((id, step, reached_with)) = frontier.pop_front() {
            if step >= depth {
                continue;
            }
            let neighbors = self
                .storage()
                .neighbors(id, Direction::In, min_confidence, fanout)?;
            for neighbor in neighbors {
                if !neighbor.edge.confidence.is_structural() {
                    walk.all_structural = false;
                }
                let found = neighbor.symbol.id;
                if found == root.id {
                    continue;
                }
                let confidence = reached_with.min(neighbor.edge.confidence);
                if let Some(seen) = walk.reached.get_mut(&found) {
                    seen.caller.confidence = seen.caller.confidence.min(confidence);
                    continue;
                }
                if walk.reached.len() >= limit {
                    walk.truncated = true;
                    return Ok(walk);
                }
                walk.reached.insert(
                    found,
                    Reached {
                        caller: Caller {
                            name: neighbor.symbol.qualified_name,
                            path: neighbor.symbol.path,
                            line: neighbor.edge.line,
                            confidence,
                            depth: step + 1,
                            kind: neighbor.edge.kind,
                        },
                        id: found,
                    },
                );
                frontier.push_back((found, step + 1, confidence));
            }
        }
        Ok(walk)
    }

    /// Other symbols that share the analysed symbol's simple name, as `id path qualified_name`.
    fn same_name_alternatives(&self, symbol: &SymbolRecord) -> Result<Vec<String>, EngineError> {
        let found = self
            .storage()
            .find_symbols(&symbol.name, ALTERNATIVE_LOOKUP)?;
        Ok(found
            .into_iter()
            .filter(|other| other.id != symbol.id)
            .take(MAX_ALTERNATIVES)
            .map(|other| format!("{} {} {}", other.id, other.path, other.qualified_name))
            .collect())
    }

    /// Answers one impact question: who depends on a symbol, and how sure we are.
    ///
    /// The epistemic rule that qualifies the answer is the part that matters most; it is set out
    /// in the documentation of this module. Side effect: a metrics event is written; nothing is
    /// stored.
    ///
    /// # Errors
    /// Returns [`EngineError::NotFound`] or [`EngineError::Ambiguous`] when the query does not name
    /// exactly one symbol, [`EngineError::Invalid`] for an empty name, and a storage error when the
    /// index cannot be read.
    ///
    /// # Examples
    /// ```no_run
    /// # fn demo(engine: &pn_ultramemory_engine::Engine) -> Result<(), pn_ultramemory_engine::EngineError> {
    /// use pn_ultramemory_engine::ImpactQuery;
    ///
    /// let query = ImpactQuery { symbol: "load_config".into(), ..ImpactQuery::default() };
    /// let report = engine.impact(&query)?;
    /// println!("{}", report.epistemic.as_str());
    /// # Ok(())
    /// # }
    /// ```
    pub fn impact(&self, q: &ImpactQuery) -> Result<ImpactReport, EngineError> {
        let started = Instant::now();
        let symbol = self.resolve_symbol(&q.symbol)?;
        let depth = q
            .depth
            .unwrap_or(DEFAULT_IMPACT_DEPTH)
            .clamp(1, MAX_IMPACT_DEPTH);
        let min_confidence = q.min_confidence.unwrap_or(Confidence::Heuristic);
        let limit = q.limit.unwrap_or(DEFAULT_IMPACT_LIMIT);

        let walk = self.walk_callers(&symbol, depth, min_confidence, limit)?;
        let mut entries: Vec<Reached> = walk.reached.into_values().collect();
        entries.sort_by(|a, b| caller_key(a).cmp(&caller_key(b)));
        let top_caller = entries.first().map(|entry| entry.id);
        let direct = entries
            .iter()
            .filter(|entry| entry.caller.depth == 1)
            .count();
        let callers: Vec<Caller> = entries.into_iter().map(|entry| entry.caller).collect();

        let files: Vec<String> = callers
            .iter()
            .map(|caller| caller.path.clone())
            .collect::<BTreeSet<String>>()
            .into_iter()
            .collect();
        let tests: Vec<String> = files
            .iter()
            .filter(|path| is_test_path(path))
            .cloned()
            .collect();

        let open_to_the_world =
            matches!(symbol.visibility, Visibility::Public | Visibility::Unknown);
        let epistemic = if callers.is_empty() && open_to_the_world {
            Epistemic::Unknown
        } else if walk.all_structural && !walk.truncated && symbol.visibility == Visibility::Private
        {
            Epistemic::Exact
        } else {
            Epistemic::LowerBound
        };

        let mut next = Vec::with_capacity(MAX_NEXT);
        next.push(format!(
            "pn-ultramemory expand {}",
            top_caller.unwrap_or(symbol.id)
        ));
        if walk.truncated {
            next.push(format!(
                "pn-ultramemory impact {} --limit {}",
                symbol.id,
                limit.saturating_mul(2).max(2)
            ));
        } else if !callers.is_empty() && depth < MAX_IMPACT_DEPTH {
            next.push(format!(
                "pn-ultramemory impact {} --depth {}",
                symbol.id,
                depth + 1
            ));
        }
        next.push(format!(
            "pn-ultramemory recall \"{}\"",
            symbol.qualified_name
        ));
        next.truncate(MAX_NEXT);

        let total = callers.len();
        self.record(Event::Impact {
            callers: u32::try_from(total).unwrap_or(u32::MAX),
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        });
        Ok(ImpactReport {
            alternatives: self.same_name_alternatives(&symbol)?,
            symbol,
            epistemic,
            direct,
            total,
            callers,
            files,
            tests,
            truncated: walk.truncated,
            next,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Caller, Epistemic, Reached, caller_key, is_test_path};
    use pn_ultramemory_core::{Confidence, EdgeKind, SymbolId};

    /// A caller with the given ordering-relevant fields.
    fn caller(depth: u32, confidence: Confidence, path: &str, line: u32, id: i64) -> Reached {
        Reached {
            caller: Caller {
                name: "f".to_owned(),
                path: path.to_owned(),
                line,
                confidence,
                depth,
                kind: EdgeKind::Calls,
            },
            id: SymbolId(id),
        }
    }

    /// Test files are recognised by a directory component or by the shape of the file name, in
    /// either case ignoring the case of the letters.
    #[test]
    fn test_paths_are_recognised() {
        for path in [
            "tests/config.rs",
            "test/config.rs",
            "spec/models_spec.rb",
            "web/__tests__/api.ts",
            "src/Tests/Thing.cs",
            "src/config_test.go",
            "web/api.test.ts",
            "web/api.spec.ts",
        ] {
            assert!(is_test_path(path), "{path}");
        }
        for path in [
            "src/config.rs",
            "src/latest/thing.rs",
            "src/contest.rs",
            "web/testify.ts",
            "src/protester/a.rs",
        ] {
            assert!(!is_test_path(path), "{path}");
        }
    }

    /// Callers sort by depth, then by descending certainty, then by path, line, name and identity.
    #[test]
    fn callers_sort_shallowest_and_most_certain_first() {
        let mut entries = [
            caller(2, Confidence::Exact, "a.rs", 1, 3),
            caller(1, Confidence::Guess, "a.rs", 9, 1),
            caller(1, Confidence::Exact, "b.rs", 4, 2),
            caller(1, Confidence::Exact, "a.rs", 4, 5),
            caller(1, Confidence::Exact, "a.rs", 4, 4),
        ];
        entries.sort_by(|a, b| caller_key(a).cmp(&caller_key(b)));
        let order: Vec<i64> = entries.iter().map(|entry| entry.id.0).collect();
        assert_eq!(order, [4, 5, 2, 1, 3]);
    }

    /// The epistemic names are the ones the output format promises.
    #[test]
    fn epistemic_names_are_stable() {
        assert_eq!(Epistemic::Exact.as_str(), "exact");
        assert_eq!(Epistemic::LowerBound.as_str(), "lower-bound");
        assert_eq!(Epistemic::Unknown.as_str(), "unknown");
    }
}
