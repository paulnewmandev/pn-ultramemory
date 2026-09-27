// SPDX-License-Identifier: Apache-2.0
//! Deterministic generators of large repositories: Rust, Python and TypeScript files of about two
//! hundred lines each, with documentation and calls inside a file and between files.
//!
//! The same `(files, seed)` always writes byte-identical files. Every function of a file is planned
//! from the file's index and the seed alone, so a file can name the functions of any other file
//! without generating it, and the whole repository is produced in constant memory.

use core::fmt::Write as _;

use std::path::Path;

/// A small deterministic pseudo-random generator (xorshift64 star), so that no crate is needed.
#[derive(Debug, Clone)]
pub(crate) struct Rng(u64);

impl Rng {
    /// A generator for `seed`. Any seed works, including zero.
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03 | 1)
    }

    /// The next 64-bit value.
    pub(crate) fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value below `bound`, or zero when `bound` is zero.
    pub(crate) fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        usize::try_from(self.next_u64() % (bound as u64)).unwrap_or(0)
    }

    /// A random element of a non-empty slice.
    pub(crate) fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// Verbs that name what a function does.
const VERBS: &[&str] = &[
    "parse",
    "load",
    "render",
    "validate",
    "encode",
    "decode",
    "resolve",
    "compute",
    "normalize",
    "merge",
    "split",
    "filter",
    "collect",
    "flush",
    "apply",
    "build",
    "format",
    "fetch",
    "store",
    "dispatch",
    "schedule",
    "index",
    "scan",
    "verify",
    "sign",
    "compress",
    "expand",
    "transform",
    "register",
    "lookup",
    "update",
    "remove",
    "refresh",
    "emit",
    "route",
    "watch",
    "retry",
    "batch",
    "sample",
    "rank",
    "project",
    "join",
    "seal",
    "trace",
    "migrate",
    "export",
    "import",
    "audit",
];

/// Nouns that name what a function works on.
const NOUNS: &[&str] = &[
    "config",
    "token",
    "session",
    "request",
    "response",
    "packet",
    "record",
    "schema",
    "cursor",
    "cache",
    "buffer",
    "channel",
    "queue",
    "route",
    "header",
    "payload",
    "ticket",
    "invoice",
    "account",
    "profile",
    "metric",
    "event",
    "signal",
    "lease",
    "shard",
    "ledger",
    "manifest",
    "bundle",
    "checkpoint",
    "snapshot",
    "policy",
    "quota",
    "tenant",
    "catalog",
    "query",
    "stream",
    "frame",
    "chunk",
    "digest",
    "region",
    "plan",
    "stage",
    "vector",
    "matrix",
    "artifact",
    "template",
    "mapping",
    "registry",
];

/// Topics, one per file; they name the file and give its documentation a subject.
const TOPICS: &[&str] = &[
    "ledger",
    "gateway",
    "pipeline",
    "scheduler",
    "inventory",
    "telemetry",
    "identity",
    "storage",
    "routing",
    "billing",
    "catalog",
    "session",
    "webhook",
    "archive",
    "cluster",
    "indexer",
    "notifier",
    "exporter",
    "importer",
    "planner",
    "resolver",
    "renderer",
    "tracker",
    "balancer",
    "verifier",
    "collector",
    "encoder",
    "monitor",
    "registry",
    "throttle",
    "uploader",
    "reporter",
];

/// Adjectives that vary the documentation sentences.
const QUALIFIERS: &[&str] = &[
    "stable",
    "compact",
    "ordered",
    "cached",
    "validated",
    "batched",
    "incremental",
    "sorted",
    "normalized",
    "filtered",
    "pending",
    "verified",
];

/// The areas of the repository, the first directory below the language root.
const AREAS: &[&str] = &[
    "core", "net", "store", "api", "cli", "auth", "billing", "search", "sync", "ui", "jobs",
    "metrics",
];

/// The second directory.
const SUBS: &[&str] = &[
    "cache", "codec", "queue", "model", "util", "http", "fs", "text",
];

/// The language of a generated file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lang {
    /// Rust, four files in ten.
    Rust,
    /// Python, three files in ten.
    Python,
    /// TypeScript, three files in ten.
    TypeScript,
}

/// One function of a plan.
#[derive(Debug, Clone)]
pub(crate) struct FnPlan {
    /// The verb of the name.
    pub verb: &'static str,
    /// The noun of the name.
    pub noun: &'static str,
    /// Whether the topic is appended to the name.
    pub suffixed: bool,
    /// The adjective used in its documentation.
    pub qualifier: &'static str,
}

/// Everything that decides what one file contains.
#[derive(Debug, Clone)]
pub(crate) struct FilePlan {
    /// The language.
    pub lang: Lang,
    /// The path relative to the repository root.
    pub path: String,
    /// The topic of the file.
    pub topic: &'static str,
    /// Its functions, in order.
    pub fns: Vec<FnPlan>,
}

impl FnPlan {
    /// The name in snake case.
    pub(crate) fn snake(&self, topic: &str) -> String {
        if self.suffixed {
            format!("{}_{}_{}", self.verb, self.noun, topic)
        } else {
            format!("{}_{}", self.verb, self.noun)
        }
    }

    /// The name in camel case.
    pub(crate) fn camel(&self, topic: &str) -> String {
        let mut name = self.verb.to_owned();
        name.push_str(&capitalize(self.noun));
        if self.suffixed {
            name.push_str(&capitalize(topic));
        }
        name
    }

    /// The name as written in the language of the file.
    pub(crate) fn name(&self, lang: Lang, topic: &str) -> String {
        match lang {
            Lang::Rust | Lang::Python => self.snake(topic),
            Lang::TypeScript => self.camel(topic),
        }
    }

    /// The one-sentence documentation of the function.
    pub(crate) fn sentence(&self, topic: &str) -> String {
        format!(
            "{}s the {} of a {} so that callers always get a {} result.",
            capitalize(self.verb),
            self.noun,
            topic,
            self.qualifier
        )
    }
}

/// Upper-cases the first letter of a word.
pub(crate) fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// How many functions a file has.
const FUNCTIONS_PER_FILE: usize = 14;

/// Decides what file `index` of a repository generated with `seed` contains.
pub(crate) fn plan_file(seed: u64, index: usize) -> FilePlan {
    let mut rng = Rng::new(seed ^ ((index as u64 + 1).wrapping_mul(0xA24B_AED4_963E_E407)));
    let lang = match index % 10 {
        0..=3 => Lang::Rust,
        4..=6 => Lang::Python,
        _ => Lang::TypeScript,
    };
    let topic = TOPICS[(index + usize::try_from(seed % 7).unwrap_or(0)) % TOPICS.len()];
    let area = AREAS[index % AREAS.len()];
    let sub = SUBS[(index / AREAS.len()) % SUBS.len()];
    let path = match lang {
        Lang::Rust => format!("crates/{area}/{sub}/{topic}_{index}.rs"),
        Lang::Python => format!("services/{area}/{sub}/{topic}_{index}.py"),
        Lang::TypeScript => format!("web/{area}/{sub}/{topic}_{index}.ts"),
    };
    let mut fns: Vec<FnPlan> = Vec::new();
    while fns.len() < FUNCTIONS_PER_FILE {
        let verb = *rng.pick(VERBS);
        let noun = *rng.pick(NOUNS);
        if fns.iter().any(|f| f.verb == verb && f.noun == noun) {
            continue;
        }
        fns.push(FnPlan {
            verb,
            noun,
            suffixed: rng.below(3) == 0,
            qualifier: rng.pick(QUALIFIERS),
        });
    }
    FilePlan {
        lang,
        path,
        topic,
        fns,
    }
}

/// The name of a function of another file that a function calls, chosen from the seed.
fn foreign_callee(seed: u64, files: usize, index: usize, slot: usize, k: usize) -> String {
    let mut rng = Rng::new(seed ^ ((index * 131 + slot * 17 + k) as u64).wrapping_mul(0x9E37));
    let mut other = rng.below(files.max(1));
    if other == index && files > 1 {
        other = (other + 1) % files;
    }
    let plan = plan_file(seed, other);
    let choice = &plan.fns[rng.below(plan.fns.len())];
    choice.name(plan.lang, plan.topic)
}

/// Writes the source of a Rust file.
fn rust_source(seed: u64, files: usize, index: usize, plan: &FilePlan) -> String {
    let topic = plan.topic;
    let record = format!("{}Record{}", capitalize(topic), index);
    let mut out = format!(
        "//! The {topic} part of the {} layer.\n\nuse std::collections::BTreeMap;\n\n",
        plan.path.split('/').nth(1).unwrap_or("core")
    );
    let _ = write!(
        out,
        "/// A stored {topic} entry.\npub struct {record} {{\n    /// The identifier.\n    pub id: u64,\n    /// The label.\n    pub label: String,\n}}\n\n"
    );
    let _ = write!(
        out,
        "impl {record} {{\n    /// Creates an empty entry with the given identifier.\n    pub fn new(id: u64) -> Self {{\n        Self {{ id, label: String::new() }}\n    }}\n\n    /// Describes the entry in one line.\n    pub fn describe(&self) -> String {{\n        format!(\"{{}}:{{}}\", self.id, self.label)\n    }}\n}}\n\n"
    );
    for (slot, f) in plan.fns.iter().enumerate() {
        let name = f.snake(topic);
        let local = plan.fns[(slot + 1) % plan.fns.len()].snake(topic);
        let foreign_a = foreign_callee(seed, files, index, slot, 0);
        let foreign_b = foreign_callee(seed, files, index, slot, 1);
        let _ = writeln!(out, "/// {}", f.sentence(topic));
        let _ = write!(
            out,
            "pub fn {name}(input: &str, limit: usize) -> Vec<String> {{\n    let mut out = Vec::new();\n    let head = {local}(input, limit);\n    let tail = {foreign_a}(input, limit);\n    let mut seen: BTreeMap<String, usize> = BTreeMap::new();\n    for part in input.split(',') {{\n        let count = seen.entry(part.to_string()).or_insert(0);\n        *count += 1;\n        if part.len() > limit {{\n            out.extend(head.clone());\n        }} else {{\n            out.extend({foreign_b}(part, limit));\n        }}\n    }}\n    out.extend(tail);\n    out\n}}\n\n"
        );
    }
    out
}

/// Writes the source of a Python file.
fn python_source(seed: u64, files: usize, index: usize, plan: &FilePlan) -> String {
    let topic = plan.topic;
    let class = format!("{}Store{}", capitalize(topic), index);
    let mut out = format!(
        "\"\"\"The {topic} part of the {} layer.\"\"\"\n\nfrom collections import defaultdict\n\n\n",
        plan.path.split('/').nth(1).unwrap_or("core")
    );
    let _ = write!(
        out,
        "class {class}:\n    \"\"\"Keeps {topic} entries in memory.\"\"\"\n\n    def __init__(self, capacity):\n        self.capacity = capacity\n        self.items = []\n\n    def add(self, item):\n        \"\"\"Adds one entry and drops the oldest when full.\"\"\"\n        self.items.append(item)\n        if len(self.items) > self.capacity:\n            self.items.pop(0)\n        return len(self.items)\n\n\n"
    );
    for (slot, f) in plan.fns.iter().enumerate() {
        let name = f.snake(topic);
        let local = plan.fns[(slot + 1) % plan.fns.len()].snake(topic);
        let foreign_a = foreign_callee(seed, files, index, slot, 0);
        let foreign_b = foreign_callee(seed, files, index, slot, 1);
        let _ = write!(
            out,
            "def {name}(text, limit):\n    \"\"\"{}\"\"\"\n    out = []\n    head = {local}(text, limit)\n    tail = {foreign_a}(text, limit)\n    seen = defaultdict(int)\n    for part in text.split(','):\n        seen[part] += 1\n        if len(part) > limit:\n            out.extend(head)\n        else:\n            out.extend({foreign_b}(part, limit))\n    out.extend(tail)\n    return out\n\n\n",
            f.sentence(topic)
        );
    }
    out
}

/// Writes the source of a TypeScript file.
fn typescript_source(seed: u64, files: usize, index: usize, plan: &FilePlan) -> String {
    let topic = plan.topic;
    let entry = format!("{}Entry{}", capitalize(topic), index);
    let mut out = format!(
        "// The {topic} part of the {} layer.\n\n",
        plan.path.split('/').nth(1).unwrap_or("core")
    );
    let _ = write!(
        out,
        "/** A stored {topic} entry. */\nexport interface {entry} {{\n  id: number;\n  label: string;\n}}\n\n"
    );
    for (slot, f) in plan.fns.iter().enumerate() {
        let name = f.camel(topic);
        let local = plan.fns[(slot + 1) % plan.fns.len()].camel(topic);
        let foreign_a = foreign_callee(seed, files, index, slot, 0);
        let foreign_b = foreign_callee(seed, files, index, slot, 1);
        let _ = write!(
            out,
            "/** {} */\nexport function {name}(input: string, limit: number): string[] {{\n  const out: string[] = [];\n  const head = {local}(input, limit);\n  const tail = {foreign_a}(input, limit);\n  const seen = new Map<string, number>();\n  for (const part of input.split(',')) {{\n    seen.set(part, (seen.get(part) ?? 0) + 1);\n    if (part.length > limit) {{\n      out.push(...head);\n    }} else {{\n      out.push(...{foreign_b}(part, limit));\n    }}\n  }}\n  out.push(...tail);\n  return out;\n}}\n\n",
            f.sentence(topic)
        );
    }
    out
}

/// The source text of file `index` of a repository of `files` files generated with `seed`.
pub(crate) fn file_source(seed: u64, files: usize, index: usize) -> (String, String) {
    let plan = plan_file(seed, index);
    let text = match plan.lang {
        Lang::Rust => rust_source(seed, files, index, &plan),
        Lang::Python => python_source(seed, files, index, &plan),
        Lang::TypeScript => typescript_source(seed, files, index, &plan),
    };
    (plan.path, text)
}

/// What was written.
#[derive(Debug, Clone)]
pub(crate) struct GeneratedRepo {
    /// The paths of the files, in generation order.
    pub paths: Vec<String>,
    /// The total size of the sources in bytes.
    pub bytes: u64,
    /// The total number of lines.
    pub lines: u64,
}

/// Writes `files` generated files below `root` and reports what was written.
pub(crate) fn generate_repo(root: &Path, files: usize, seed: u64) -> GeneratedRepo {
    let mut repo = GeneratedRepo {
        paths: Vec::with_capacity(files),
        bytes: 0,
        lines: 0,
    };
    for index in 0..files {
        let (path, text) = file_source(seed, files, index);
        super::write(root, &path, &text);
        repo.bytes += text.len() as u64;
        repo.lines += text.lines().count() as u64;
        repo.paths.push(path);
    }
    repo
}

#[cfg(test)]
mod tests {
    use super::{Rng, file_source, plan_file};

    /// The generator is deterministic and its values stay below the bound.
    #[test]
    fn rng_is_deterministic_and_bounded() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..1000 {
            let x = a.below(13);
            assert_eq!(x, b.below(13));
            assert!(x < 13);
        }
        assert_eq!(Rng::new(0).below(0), 0);
    }

    /// Files are about two hundred lines and identical on every call.
    #[test]
    fn files_are_stable_and_sized() {
        for index in 0..30 {
            let (path, text) = file_source(1, 30, index);
            assert_eq!((path.clone(), text.clone()), file_source(1, 30, index));
            let lines = text.lines().count();
            assert!((120..320).contains(&lines), "{path}: {lines} lines");
        }
        assert_ne!(
            plan_file(1, 5).path,
            plan_file(2, 5).path.replace("_5", "_x")
        );
    }
}
