// SPDX-License-Identifier: Apache-2.0
//! Integration tests of the hook: the invariants that matter more than the wording.
//!
//! Every answer must be exactly one line of JSON, never an error, never a panic, and the advice must
//! be given at most once per session. The binary has no library target, so the modules under test
//! are compiled into this test crate with `#[path]`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests unwrap and panic to fail loudly"
)]

#[path = "../src/error.rs"]
pub mod error;
#[path = "../src/hook.rs"]
pub mod hook;

#[allow(
    dead_code,
    reason = "each test binary uses a different subset of the helpers"
)]
mod common;

use std::path::Path;
use std::time::Duration;

use common::Repo;
use hook::{CONTEXT_KEY, HookContext, run_hook};
use serde_json::{Value, json};

/// A repository, its data directory, and the context that points at both.
fn setup() -> (Repo, HookContext) {
    let repo = Repo::new();
    repo.mkdir("repo");
    repo.mkdir("data");
    let ctx = HookContext::new(repo.at("repo"), repo.at("data"));
    (repo, ctx)
}

/// The text of an answer, or `None` when the answer says nothing.
fn context_of(answer: &str) -> Option<String> {
    assert!(
        !answer.contains('\n'),
        "the answer must be one line: {answer:?}"
    );
    let value: serde_json::Value = serde_json::from_str(answer)
        .unwrap_or_else(|error| panic!("{answer:?} is not JSON: {error}"));
    let object = value.as_object().expect("a JSON object");
    assert!(
        object.len() <= 1,
        "the answer must hold at most one field: {answer:?}"
    );
    match object.get(CONTEXT_KEY) {
        None => {
            assert!(
                object.is_empty(),
                "the only field may be {CONTEXT_KEY}: {answer:?}"
            );
            None
        }
        Some(value) => Some(value.as_str().expect("a string").to_owned()),
    }
}

/// Every event and every input yields one line of JSON with at most our one field.
#[test]
fn every_answer_is_one_line_of_json() {
    let (_repo, ctx) = setup();
    for event in ["session-start", "pre-tool", "post-tool", "nonsense"] {
        for input in ["", "{}", "null", "{\"tool_name\":\"Grep\"}"] {
            let answer = run_hook(event, input, &ctx);
            let _ = context_of(&answer);
        }
    }
}

/// A session with no index says how to build one; a session with an index says how big it is.
#[test]
fn the_session_line_follows_the_index() {
    let (repo, ctx) = setup();
    let first = context_of(&run_hook("session-start", "", &ctx)).expect("a line");
    assert!(first.contains("no index"), "{first}");
    assert!(first.contains("pn-ultramemory index"), "{first}");

    repo.write("data/index.db", "not really a database, but it exists");
    let second = context_of(&run_hook("session-start", "", &ctx)).expect("a line");
    assert!(second.contains("is indexed"), "{second}");
    assert!(second.contains("recall"), "{second}");
    assert!(
        !second.contains("files,"),
        "no counts are known yet: {second}"
    );

    repo.write("data/stats.json", r#"{"files": 412, "symbols": 9137}"#);
    let third = context_of(&run_hook("session-start", "", &ctx)).expect("a line");
    assert!(third.contains("412 files, 9137 symbols"), "{third}");
    assert!(third.len() < 240, "{} characters is too many", third.len());

    for broken in [
        "",
        "{",
        "[]",
        r#"{"files": "many"}"#,
        r#"{"symbols": 1}"#,
        "null",
    ] {
        repo.write("data/stats.json", broken);
        let line = context_of(&run_hook("session-start", "", &ctx)).expect("a line");
        assert!(line.contains("is indexed"), "{broken:?} gave {line}");
    }
}

/// A repository-wide search is advised about, exactly once, and the advice is never a denial.
#[test]
fn a_wide_search_is_advised_about_once() {
    let (repo, ctx) = setup();
    let call =
        r#"{"session_id":"abc123","tool_name":"Grep","tool_input":{"pattern":"load_config"}}"#;
    let first = context_of(&run_hook("pre-tool", call, &ctx)).expect("advice");
    assert!(first.contains("recall"), "{first}");
    assert!(!first.to_lowercase().contains("deny"), "{first}");
    assert!(!first.to_lowercase().contains("block"), "{first}");
    assert!(
        repo.exists("data/hooks/advised-abc123"),
        "a marker should have been left"
    );
    assert_eq!(
        context_of(&run_hook("pre-tool", call, &ctx)),
        None,
        "advice must be given once"
    );

    let other = r#"{"session_id":"different","tool_name":"Grep","tool_input":{"pattern":"x"}}"#;
    assert!(
        context_of(&run_hook("pre-tool", other, &ctx)).is_some(),
        "a new session may be advised"
    );
}

/// What counts as expensive, and what does not.
#[test]
fn only_expensive_calls_are_advised_about() {
    let wide: &[&str] = &[
        r#"{"tool_name":"Grep","tool_input":{"pattern":"x"}}"#,
        r#"{"tool_name":"Grep","tool_input":{"pattern":"x","path":"."}}"#,
        r#"{"tool_name":"Grep","tool_input":{"pattern":"x","path":"/"}}"#,
        r#"{"tool_name":"Glob","tool_input":{"pattern":"**/*.rs","path":"src"}}"#,
        r#"{"toolName":"search_files","toolInput":{"query":"config"}}"#,
        r#"{"tool":"ripgrep","arguments":{"regex":"fn main"}}"#,
        r#"{"name":"find","params":{}}"#,
        r#"{"tool_name":"GrepTool","input":{"pattern":"a","dir":"./"}}"#,
    ];
    let narrow: &[&str] = &[
        r#"{"tool_name":"Grep","tool_input":{"pattern":"x","path":"src/engine"}}"#,
        r#"{"tool_name":"Bash","tool_input":{"command":"ls"}}"#,
        r#"{"tool_name":"Edit","tool_input":{"file_path":"a.rs"}}"#,
        r#"{"tool_name":"Write","tool_input":{}}"#,
        r#"{"tool_input":{"pattern":"x"}}"#,
        r#"{"tool_name":42,"tool_input":{}}"#,
        "{}",
    ];
    for (index, call) in wide.iter().enumerate() {
        let (_repo, ctx) = setup();
        assert!(
            context_of(&run_hook("pre-tool", call, &ctx)).is_some(),
            "case {index} should be advised about: {call}"
        );
    }
    for call in narrow {
        let (_repo, ctx) = setup();
        assert_eq!(
            context_of(&run_hook("pre-tool", call, &ctx)),
            None,
            "{call}"
        );
    }
}

/// A read call as an agent would send it, with the path encoded rather than pasted.
///
/// Pasting a path into a JSON string with `format!` is wrong on Windows, where a path is full of
/// backslashes and a backslash in JSON starts an escape: `"C:\\Users\\..."` is not valid JSON, the
/// call never parses, and the test fails for a reason that has nothing to do with what it checks.
fn read_call(path: &Path, extra: Option<(&str, u32)>) -> String {
    let mut input = serde_json::Map::new();
    input.insert("file_path".into(), json!(path.to_string_lossy()));
    if let Some((key, value)) = extra {
        input.insert(key.to_owned(), json!(value));
    }
    json!({ "tool_name": "Read", "tool_input": Value::Object(input) }).to_string()
}

/// A path with a capital letter in it is advised about too. On a case-sensitive file system a
/// folded path names nothing, so this is the case that caught the hook lower-casing a path before
/// handing it to the file system: it passed on macOS and Windows and failed on Linux.
#[test]
fn a_path_with_capitals_is_not_folded() {
    let (repo, ctx) = setup();
    repo.write("repo/Big File.rs", &"// a line of source\n".repeat(8_000));
    let call = read_call(&repo.at("repo/Big File.rs"), None);
    assert!(
        context_of(&run_hook("pre-tool", &call, &ctx)).is_some(),
        "{call}"
    );
}

/// A whole read of a large file is advised about; a windowed read and a small file are not.
#[test]
fn reading_a_whole_large_file_is_advised_about() {
    let (repo, ctx) = setup();
    repo.write("repo/big.rs", &"// a line of source\n".repeat(8_000));
    repo.write("repo/small.rs", "fn main() {}\n");
    let big = repo.at("repo/big.rs");
    let small = repo.at("repo/small.rs");

    let whole = read_call(&big, None);
    assert!(
        context_of(&run_hook("pre-tool", &whole, &ctx)).is_some(),
        "{whole}"
    );

    let (_repo2, ctx2) = setup();
    let windowed = read_call(&big, Some(("limit", 50)));
    assert_eq!(context_of(&run_hook("pre-tool", &windowed, &ctx2)), None);

    let tiny = read_call(&small, None);
    assert_eq!(context_of(&run_hook("pre-tool", &tiny, &ctx2)), None);

    let missing = r#"{"tool_name":"Read","tool_input":{"file_path":"/no/such/file"}}"#;
    assert_eq!(context_of(&run_hook("pre-tool", missing, &ctx2)), None);
}

/// Reserved and unknown events say nothing.
#[test]
fn reserved_and_unknown_events_say_nothing() {
    let (_repo, ctx) = setup();
    let call = r#"{"tool_name":"Grep","tool_input":{"pattern":"x"}}"#;
    for event in [
        "post-tool",
        "PostTool",
        "pre_tool",
        "session_start",
        "",
        "  ",
        "hook",
    ] {
        assert_eq!(context_of(&run_hook(event, call, &ctx)), None, "{event:?}");
    }
}

/// A context that is switched off says nothing, and neither does the environment switch.
#[test]
fn hooks_can_be_switched_off() {
    let (repo, mut ctx) = setup();
    repo.write("data/index.db", "x");
    assert!(context_of(&run_hook("session-start", "", &ctx)).is_some());
    ctx.enabled = false;
    assert_eq!(context_of(&run_hook("session-start", "", &ctx)), None);
    assert_eq!(
        context_of(&run_hook("pre-tool", r#"{"tool_name":"Grep"}"#, &ctx)),
        None
    );
}

/// Nothing an agent can send makes the hook panic, hang or answer with more than one line.
#[test]
fn no_input_can_break_the_hook() {
    let deep_object = format!(
        "{}{}",
        "{\"a\":".repeat(600),
        "1".to_owned() + &"}".repeat(600)
    );
    let deep_array = format!("{}1{}", "[".repeat(2_000), "]".repeat(2_000));
    let huge = format!(
        r#"{{"tool_name":"Grep","tool_input":{{"pattern":"{}"}}}}"#,
        "x".repeat(1024 * 1024)
    );
    let lossy = String::from_utf8_lossy(&[0xff, 0xfe, 0x00, 0x41, 0xc3, 0x28]).into_owned();
    let hostile: Vec<String> = vec![
        String::new(),
        " ".to_owned(),
        "\n".to_owned(),
        "\0".to_owned(),
        "\u{feff}{}".to_owned(),
        "null".to_owned(),
        "true".to_owned(),
        "false".to_owned(),
        "0".to_owned(),
        "-1e999".to_owned(),
        "\"a string\"".to_owned(),
        "[]".to_owned(),
        "[[[]]]".to_owned(),
        "{".to_owned(),
        "}".to_owned(),
        "{}{}".to_owned(),
        "{\"a\"}".to_owned(),
        "{\"a\":}".to_owned(),
        "{,}".to_owned(),
        "{\"tool_name\":}".to_owned(),
        "{\"tool_name\":null}".to_owned(),
        "{\"tool_name\":[]}".to_owned(),
        "{\"tool_name\":{}}".to_owned(),
        "{\"tool_name\":\"\"}".to_owned(),
        "{\"tool_name\":\"Grep\"".to_owned(),
        "{\"tool_name\":\"Grep\",}".to_owned(),
        "{\"tool_name\":\"Grep\",\"tool_input\":null}".to_owned(),
        "{\"tool_name\":\"Grep\",\"tool_input\":[]}".to_owned(),
        "{\"tool_name\":\"Grep\",\"tool_input\":\"x\"}".to_owned(),
        "{\"tool_name\":\"Grep\",\"tool_input\":{\"pattern\":null}}".to_owned(),
        "{\"tool_name\":\"Grep\",\"tool_input\":{\"path\":12}}".to_owned(),
        "{\"tool_name\":\"Read\",\"tool_input\":{\"file_path\":\"\"}}".to_owned(),
        "{\"tool_name\":\"Read\",\"tool_input\":{\"file_path\":\"/\"}}".to_owned(),
        "{\"tool_name\":\"Read\",\"tool_input\":{\"file_path\":\"/dev/zero\"}}".to_owned(),
        "{\"tool_name\":\"Read\",\"tool_input\":{\"file_path\":\"../../etc/passwd\"}}".to_owned(),
        "{\"session_id\":\"../../escape\",\"tool_name\":\"Grep\"}".to_owned(),
        "{\"session_id\":\"\",\"tool_name\":\"Grep\"}".to_owned(),
        format!(
            "{{\"session_id\":\"{}\",\"tool_name\":\"Grep\"}}",
            "s".repeat(4_000)
        ),
        "{\"session_id\":{\"a\":1},\"tool_name\":\"Grep\"}".to_owned(),
        "{\"tool_name\":\"Gr\\u0000ep\"}".to_owned(),
        "{\"tool_name\":\"Grep\\n\\r\\t\"}".to_owned(),
        "{\"tool_name\":\"\u{1f600}grep\"}".to_owned(),
        "{\"tool_name\":\"GREP\",\"tool_input\":{\"pattern\":\"x\"}}".to_owned(),
        "{\"\":\"\"}".to_owned(),
        "{\"a\":\"\\ud800\"}".to_owned(),
        "{\"a\":1,\"a\":2}".to_owned(),
        "{\"tool_name\":\"Grep\",\"tool_input\":{\"pattern\":\"**\"}}".to_owned(),
        lossy,
        deep_object,
        deep_array,
        "x".repeat(200_000),
        huge,
    ];
    assert!(hostile.len() >= 40, "only {} hostile inputs", hostile.len());
    let (_repo, ctx) = setup();
    for input in &hostile {
        for event in ["session-start", "pre-tool", "post-tool", "surprise"] {
            let answer = run_hook(event, input, &ctx);
            assert!(
                !answer.contains('\n'),
                "{event} {input:.40?} answered on several lines"
            );
            let value: serde_json::Value = serde_json::from_str(&answer)
                .unwrap_or_else(|error| panic!("{event} {input:.40?}: {error}"));
            let object = value.as_object().expect("a JSON object");
            assert!(object.len() <= 1, "{event} {input:.40?} answered {answer}");
            for key in object.keys() {
                assert_eq!(key, CONTEXT_KEY, "{event} {input:.40?} used another field");
            }
        }
    }
    assert!(
        !ctx.repo.join("../../escape").exists(),
        "a session name must not escape"
    );
    let markers = std::fs::read_dir(ctx.data_dir.join("hooks"))
        .map(Iterator::count)
        .unwrap_or_default();
    assert!(
        markers <= 4,
        "{markers} markers is too many for these sessions"
    );
}

/// Answering never takes anywhere near the deadline, even with an unreadable data directory.
#[test]
fn answering_is_quick() {
    let (_repo, mut ctx) = setup();
    ctx.deadline = Duration::from_millis(50);
    ctx.data_dir = std::path::PathBuf::from("/no/such/directory/at/all");
    let started = std::time::Instant::now();
    for _ in 0..200 {
        let _ = run_hook("session-start", "", &ctx);
        let _ = run_hook("pre-tool", r#"{"tool_name":"Grep","tool_input":{}}"#, &ctx);
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

/// The deadline is enforced by the caller: with no time at all, the hook answers `{}` and returns
/// instead of waiting for standard input, which is the invariant that keeps an agent responsive.
#[test]
fn the_deadline_is_enforced() {
    let (_repo, mut ctx) = setup();
    ctx.deadline = Duration::from_millis(1);
    let started = std::time::Instant::now();
    assert!(hook::run_hook_stdio("session-start", &ctx).is_ok());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
}
