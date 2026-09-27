// SPDX-License-Identifier: Apache-2.0
//! Tests of `FsSourceTree` on temporary directories: listing rules, path safety, atomic writes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests unwrap and panic to fail loudly"
)]

use std::fs;
use std::path::Path;

use pn_ultramemory_core::{SourceError, SourceFile, SourceTree};
use pn_ultramemory_index::FsSourceTree;

/// Writes a file, creating its directories.
fn put(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Lists a tree and returns only the paths.
fn paths(tree: &FsSourceTree) -> Vec<String> {
    tree.list().unwrap().into_iter().map(|f| f.path).collect()
}

/// Opens a tree over a directory with a generous size limit.
fn open(root: &Path) -> FsSourceTree {
    FsSourceTree::new(root, 1_000_000).unwrap()
}

/// Only files of known languages are listed, sorted, with forward-slash relative paths, sizes
/// and modification times.
#[test]
fn lists_source_files_sorted() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "src/main.rs", "fn main() {}\n");
    put(dir.path(), "src/lib.rs", "pub fn f() {}\n");
    put(dir.path(), "src/deep/nested/mod.py", "def f(): pass\n");
    put(dir.path(), "Zeta.java", "class Zeta {}\n");
    put(dir.path(), "alpha.kt", "fun a() {}\n");
    put(dir.path(), "README.md", "# readme\n");
    put(dir.path(), "data.json", "{}\n");
    put(dir.path(), "image.png", "not really a png");
    put(dir.path(), "Makefile", "all:\n");
    let tree = open(dir.path());
    let files = tree.list().unwrap();
    let listed: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        listed,
        [
            "Zeta.java",
            "alpha.kt",
            "src/deep/nested/mod.py",
            "src/lib.rs",
            "src/main.rs"
        ]
    );
    let main = files.iter().find(|f| f.path == "src/main.rs").unwrap();
    assert_eq!(main.size, "fn main() {}\n".len() as u64);
    assert!(main.mtime_secs > 1_500_000_000, "{}", main.mtime_secs);
    assert_eq!(files, tree.list().unwrap(), "listing is deterministic");
}

/// `.gitignore`, `.ignore` and nested ignore files are honoured, with negation, even without a
/// git repository.
#[test]
fn respects_ignore_files() {
    let dir = tempfile::tempdir().unwrap();
    put(
        dir.path(),
        ".gitignore",
        "generated/\n*.gen.rs\n!keep.gen.rs\n",
    );
    put(dir.path(), ".ignore", "scratch.py\n");
    put(dir.path(), "a.rs", "fn a() {}\n");
    put(dir.path(), "b.gen.rs", "fn b() {}\n");
    put(dir.path(), "keep.gen.rs", "fn k() {}\n");
    put(dir.path(), "generated/c.rs", "fn c() {}\n");
    put(dir.path(), "scratch.py", "x = 1\n");
    put(dir.path(), "sub/.gitignore", "local.go\n");
    put(dir.path(), "sub/local.go", "package p\n");
    put(dir.path(), "sub/kept.go", "package p\n");
    assert_eq!(
        paths(&open(dir.path())),
        ["a.rs", "keep.gen.rs", "sub/kept.go"]
    );
}

/// A `.git/info/exclude` file is an ignore source too.
#[test]
fn respects_git_exclude() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), ".git/info/exclude", "private.rs\n");
    put(dir.path(), ".git/HEAD", "ref: refs/heads/main\n");
    put(dir.path(), "private.rs", "fn p() {}\n");
    put(dir.path(), "public.rs", "fn q() {}\n");
    assert_eq!(paths(&open(dir.path())), ["public.rs"]);
}

/// Hidden files and directories, and the well-known dependency and build directories, are skipped
/// wherever they are.
#[test]
fn skips_hidden_and_vendored_directories() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), ".hidden.rs", "fn h() {}\n");
    put(dir.path(), ".config/tool.py", "x = 1\n");
    put(
        dir.path(),
        "node_modules/pkg/index.js",
        "module.exports = 1;\n",
    );
    put(dir.path(), "target/debug/build.rs", "fn t() {}\n");
    put(dir.path(), "dist/bundle.js", "var a = 1;\n");
    put(dir.path(), "build/out.c", "int x;\n");
    put(dir.path(), "vendor/lib/lib.go", "package lib\n");
    put(dir.path(), "src/node_modules/inner.js", "x\n");
    put(dir.path(), "src/vendor/inner.rb", "x = 1\n");
    put(dir.path(), "src/keep.rs", "fn k() {}\n");
    put(dir.path(), "builder.rs", "fn b() {}\n");
    assert_eq!(paths(&open(dir.path())), ["builder.rs", "src/keep.rs"]);
}

/// A root that is itself called `build` or `target` is still listed: only nested directories
/// with those names are skipped.
#[test]
fn root_named_like_a_skipped_directory_is_listed() {
    let parent = tempfile::tempdir().unwrap();
    for name in ["build", "target", "vendor", "node_modules"] {
        let root = parent.path().join(name);
        put(&root, "a.rs", "fn a() {}\n");
        put(&root, "build/b.rs", "fn b() {}\n");
        assert_eq!(paths(&open(&root)), ["a.rs"], "root {name}");
    }
}

/// Files above the size limit are skipped; a file exactly at the limit is kept.
#[test]
fn honours_the_size_limit() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "small.rs", "fn a() {}\n");
    put(dir.path(), "exact.rs", &"a".repeat(100));
    put(dir.path(), "big.rs", &"a".repeat(101));
    let tree = FsSourceTree::new(dir.path(), 100).unwrap();
    assert_eq!(paths(&tree), ["exact.rs", "small.rs"]);
    assert_eq!(tree.max_file_bytes(), 100);
    let none = FsSourceTree::new(dir.path(), 0).unwrap();
    assert!(paths(&none).is_empty());
}

/// Minified files are skipped: `.min.js` names and files whose first line is very long.
#[test]
fn skips_minified_files() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "app.min.js", "var a=1;\n");
    put(dir.path(), "APP.MIN.JS", "var a=1;\n");
    put(
        dir.path(),
        "bundle.js",
        &format!("var a = [{}];\n", "1,".repeat(1500)),
    );
    put(
        dir.path(),
        "later.js",
        &format!("var a = 1;\n// {}\n", "x".repeat(5000)),
    );
    put(
        dir.path(),
        "unicode.js",
        &format!("// {}\nvar a = 1;\n", "é".repeat(1500)),
    );
    put(dir.path(), "single_line.py", &"x".repeat(9000));
    put(dir.path(), "normal.js", "var a = 1;\n");
    let listed = paths(&open(dir.path()));
    assert_eq!(listed, ["later.js", "normal.js", "unicode.js"]);
}

/// Symbolic links are not followed: neither linked files nor linked directories are listed.
#[cfg(unix)]
#[test]
fn does_not_follow_symlinks() {
    use std::os::unix::fs::symlink;
    let outside = tempfile::tempdir().unwrap();
    put(outside.path(), "secret.rs", "fn secret() {}\n");
    put(outside.path(), "dir/inner.rs", "fn inner() {}\n");
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "real.rs", "fn real() {}\n");
    symlink(outside.path().join("secret.rs"), dir.path().join("link.rs")).unwrap();
    symlink(outside.path().join("dir"), dir.path().join("linked_dir")).unwrap();
    symlink(dir.path().join("real.rs"), dir.path().join("alias.rs")).unwrap();
    assert_eq!(paths(&open(dir.path())), ["real.rs"]);
}

/// A tree cannot be opened on a missing directory or on a file.
#[test]
fn new_rejects_bad_roots() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        FsSourceTree::new(dir.path().join("missing"), 10),
        Err(SourceError::Io(_))
    ));
    put(dir.path(), "file.rs", "x");
    assert!(matches!(
        FsSourceTree::new(dir.path().join("file.rs"), 10),
        Err(SourceError::Io(_))
    ));
    // A root written with `..` is canonicalized.
    put(dir.path(), "sub/a.rs", "fn a() {}\n");
    let tree = FsSourceTree::new(dir.path().join("sub").join(".."), 100).unwrap();
    assert_eq!(tree.root(), dir.path().canonicalize().unwrap());
    assert_eq!(paths(&tree), ["file.rs", "sub/a.rs"]);
}

/// Reading returns the text, and refuses binary content, missing files and directories.
#[test]
fn read_behaviour() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "src/a.rs", "fn a() { /* é日本 */ }\r\n");
    fs::write(dir.path().join("bin.rs"), [0xff, 0xfe, b'a', 0x80]).unwrap();
    let tree = open(dir.path());
    assert_eq!(tree.read("src/a.rs").unwrap(), "fn a() { /* é日本 */ }\r\n");
    assert_eq!(
        tree.read("./src/a.rs").unwrap(),
        "fn a() { /* é日本 */ }\r\n"
    );
    assert_eq!(
        tree.read("bin.rs"),
        Err(SourceError::NotText("bin.rs".to_owned()))
    );
    assert!(matches!(tree.read("missing.rs"), Err(SourceError::Io(_))));
    assert!(matches!(tree.read("src"), Err(SourceError::Io(_))));
    assert!(matches!(tree.read(""), Err(SourceError::Io(_))));
    assert!(matches!(tree.read("a\0b"), Err(SourceError::Io(_))));
}

/// Paths that leave the root are refused before any file is touched.
#[test]
fn read_and_write_refuse_escapes() {
    let outer = tempfile::tempdir().unwrap();
    put(outer.path(), "secret.txt", "top secret");
    put(outer.path(), "repo/inside.rs", "fn i() {}\n");
    let tree = open(&outer.path().join("repo"));
    let absolute = outer.path().join("secret.txt");
    for path in [
        "../secret.txt",
        "sub/../../secret.txt",
        "..",
        "a/../b.rs",
        absolute.to_str().unwrap(),
    ] {
        assert_eq!(
            tree.read(path),
            Err(SourceError::OutsideRoot(path.to_owned())),
            "read {path}"
        );
        assert_eq!(
            tree.write(path, "overwritten"),
            Err(SourceError::OutsideRoot(path.to_owned())),
            "write {path}"
        );
    }
    assert_eq!(
        fs::read_to_string(outer.path().join("secret.txt")).unwrap(),
        "top secret"
    );
    assert_eq!(tree.read("inside.rs").unwrap(), "fn i() {}\n");
}

/// A symbolic link that points outside the root is refused, for reading and for writing.
#[cfg(unix)]
#[test]
fn symlink_escape_is_refused() {
    use std::os::unix::fs::symlink;
    let outer = tempfile::tempdir().unwrap();
    put(outer.path(), "secret.rs", "fn secret() {}\n");
    put(outer.path(), "outside_dir/inner.rs", "fn inner() {}\n");
    put(outer.path(), "repo/real.rs", "fn real() {}\n");
    symlink(
        outer.path().join("secret.rs"),
        outer.path().join("repo/leak.rs"),
    )
    .unwrap();
    symlink(
        outer.path().join("outside_dir"),
        outer.path().join("repo/linked"),
    )
    .unwrap();
    symlink("../secret.rs", outer.path().join("repo/relative.rs")).unwrap();
    let tree = open(&outer.path().join("repo"));
    for path in ["leak.rs", "linked/inner.rs", "relative.rs"] {
        assert_eq!(
            tree.read(path),
            Err(SourceError::OutsideRoot(path.to_owned())),
            "{path}"
        );
        assert_eq!(
            tree.write(path, "x"),
            Err(SourceError::OutsideRoot(path.to_owned())),
            "{path}"
        );
    }
    assert_eq!(
        fs::read_to_string(outer.path().join("secret.rs")).unwrap(),
        "fn secret() {}\n"
    );
    assert_eq!(
        fs::read_to_string(outer.path().join("outside_dir/inner.rs")).unwrap(),
        "fn inner() {}\n"
    );
}

/// A symbolic link that stays inside the root is followed for reading and writes the real file.
#[cfg(unix)]
#[test]
fn symlink_inside_the_root_is_allowed() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "real.rs", "fn real() {}\n");
    symlink(dir.path().join("real.rs"), dir.path().join("alias.rs")).unwrap();
    let tree = open(dir.path());
    assert_eq!(tree.read("alias.rs").unwrap(), "fn real() {}\n");
    tree.write("alias.rs", "fn changed() {}\n").unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("real.rs")).unwrap(),
        "fn changed() {}\n"
    );
    assert!(
        fs::symlink_metadata(dir.path().join("alias.rs"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

/// Writing replaces the content atomically and leaves no temporary file behind.
#[test]
fn write_replaces_content_and_leaves_no_temporary_files() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "src/a.rs", "fn old() {}\n");
    let tree = open(dir.path());
    tree.write("src/a.rs", "fn new() { /* é */ }\r\n").unwrap();
    assert_eq!(tree.read("src/a.rs").unwrap(), "fn new() { /* é */ }\r\n");
    let big = "x".repeat(2_000_000);
    tree.write("src/a.rs", &big).unwrap();
    assert_eq!(tree.read("src/a.rs").unwrap().len(), big.len());
    tree.write("src/a.rs", "").unwrap();
    assert_eq!(tree.read("src/a.rs").unwrap(), "");
    let mut names: Vec<String> = fs::read_dir(dir.path().join("src"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["a.rs"], "no temporary file may remain");
}

/// Writing needs an existing file: it never creates one, and never writes into a directory.
#[test]
fn write_requires_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "src/a.rs", "fn a() {}\n");
    let tree = open(dir.path());
    assert!(matches!(
        tree.write("src/new.rs", "x"),
        Err(SourceError::Io(_))
    ));
    assert!(matches!(tree.write("src", "x"), Err(SourceError::Io(_))));
    assert!(!dir.path().join("src/new.rs").exists());
}

/// Permissions of the original file are kept by a write.
#[cfg(unix)]
#[test]
fn write_preserves_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "run.sh.py", "print(1)\n");
    let path = dir.path().join("run.sh.py");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
    let tree = open(dir.path());
    tree.write("run.sh.py", "print(2)\n").unwrap();
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o750);
    assert_eq!(fs::read_to_string(&path).unwrap(), "print(2)\n");
}

/// Files written while another thread lists the tree never show a partial state.
#[test]
fn concurrent_reads_see_whole_files() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "a.rs", &"a".repeat(50_000));
    let tree = std::sync::Arc::new(open(dir.path()));
    let writer = {
        let tree = tree.clone();
        std::thread::spawn(move || {
            for i in 0..50 {
                let content = if i % 2 == 0 { "a" } else { "b" }.repeat(50_000);
                tree.write("a.rs", &content).unwrap();
            }
        })
    };
    for _ in 0..200 {
        let text = tree.read("a.rs").unwrap();
        assert_eq!(text.len(), 50_000);
        assert!(text.bytes().all(|b| b == text.as_bytes()[0]), "torn read");
    }
    writer.join().unwrap();
}

/// The adapter is usable through the port and across threads.
#[test]
fn works_as_a_port_object() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<FsSourceTree>();
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "a.go", "package a\n");
    let tree: Box<dyn SourceTree> = Box::new(open(dir.path()));
    let files: Vec<SourceFile> = tree.list().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "a.go");
}
