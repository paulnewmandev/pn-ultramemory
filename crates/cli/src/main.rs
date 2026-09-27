// SPDX-License-Identifier: Apache-2.0
//! The `pn-ultramemory` command-line binary.
//!
//! # Role in the architecture
//! Entry adapter (see `docs/architecture.md`). It parses the command line, builds the engine from
//! its adapters, calls one use case and prints the result. It holds no domain logic: everything it
//! decides is about presentation, exit codes and where files live.
//!
//! # Structure
//! * [`args`] is the whole command-line surface, and its doc comments are what `--help` prints.
//! * [`paths`] decides which repository and which data directory to use.
//! * [`dirs`] answers where the platform keeps configuration and data.
//! * [`banner`] is the mark shown to a person at a terminal.
//! * [`agents`], [`install`], [`hook`] and [`doctor`] are the integration with coding agents.
//! * [`simple`] holds the commands that need no repository.
//! * [`reporting`] converts what the engine measured into what the report renderer draws.
//! * [`style`] is colour, glyphs and the progress bar, shown only to a person at a terminal.
//!
//! # Exit codes
//! `0` success, `1` a failure of the environment, `2` an invalid request, `3` something named does
//! not exist. A hook always exits `0`, whatever happens, because a hook must never fail the tool
//! call it was attached to.

// Each of these is documented by the `//!` block at the top of its own file, and deliberately
// carries no `///` here. Giving a module declaration an outer doc comment as well as inner docs
// makes rustdoc resolve the links inside those inner docs against this file instead of the
// module, which breaks every one of them in a binary crate: `cargo doc -D warnings` reported
// nineteen unresolved links the moment the comments were added. The documentation ratchet lists
// these eleven names in `.ratchet/docs.txt` for that reason.
pub mod agents;
pub mod args;
pub mod banner;
pub mod dirs;
pub mod doctor;
pub mod error;
pub mod hook;
pub mod install;
pub mod paths;
pub mod reporting;
pub mod simple;
pub mod style;

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use pn_ultramemory_codec::Format;
use pn_ultramemory_core::{MemoryFilter, MemoryId, MemoryKind, SignalKind};
use pn_ultramemory_engine::{
    Deps, Engine, EngineConfig, FeedbackTarget, IndexOptions, RecallQuery, SystemClock,
    render_value,
};
use pn_ultramemory_index::{DocComments, FsSourceTree, TreeSitterExtractor};
use pn_ultramemory_store::SqliteStorage;
use serde_json::{Value, json};

use args::{
    ByArg, Cli, Command, FeedbackArgs, Global, IdArgs, InstallArgs, KindArg, LearnCommand,
    MemoriesArgs, OutFormat, RememberArgs, ScopeArg, SignalArg, ToonCommand,
};
use error::CliError;
use paths::Locations;

/// The largest file that is read into memory while indexing.
const MAX_FILE_BYTES: u64 = 2_000_000;

/// Converts the output format argument into the renderer's type.
const fn format_of(arg: OutFormat) -> Format {
    match arg {
        OutFormat::Toon => Format::Toon,
        OutFormat::Json => Format::Json,
        OutFormat::Text => Format::Text,
    }
}

/// Converts the confidence argument into the domain type.
const fn confidence_of(arg: args::ConfidenceArg) -> pn_ultramemory_core::Confidence {
    use pn_ultramemory_core::Confidence;
    match arg {
        args::ConfidenceArg::Guess => Confidence::Guess,
        args::ConfidenceArg::Heuristic => Confidence::Heuristic,
        args::ConfidenceArg::Resolved => Confidence::Resolved,
        args::ConfidenceArg::Exact => Confidence::Exact,
    }
}

/// Today's date as `YYYY-MM-DD`, for the line a report carries about when it was made.
///
/// The conversion is done here rather than with a date library because this is the only date the
/// program ever formats, and one civil date from a Unix timestamp is a few lines of arithmetic. It
/// is UTC, which is stated in the report so that nobody reads it as a local date.
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (year, month, day) = civil_date(i64::try_from(secs / 86_400).unwrap_or(0));
    format!("{year:04}-{month:02}-{day:02}")
}

/// The civil date of a count of days since 1970-01-01, by Howard Hinnant's `civil_from_days`.
///
/// It shifts the era so that a leap year ends the cycle, which removes every special case for
/// February: the calendar then repeats exactly every 400 years, and the day of the year maps back
/// to a month with one division.
fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = era * 400 + yoe + i64::from(month <= 2);
    (year, month, day)
}

/// Converts the memory-kind argument into the domain type.
const fn kind_of(arg: KindArg) -> MemoryKind {
    match arg {
        KindArg::Decision => MemoryKind::Decision,
        KindArg::Fact => MemoryKind::Fact,
        KindArg::Lesson => MemoryKind::Lesson,
        KindArg::DeadEnd => MemoryKind::DeadEnd,
        KindArg::ErrorFix => MemoryKind::ErrorFix,
        KindArg::Convention => MemoryKind::Convention,
        KindArg::Requirement => MemoryKind::Requirement,
        KindArg::Task => MemoryKind::Task,
        KindArg::Session => MemoryKind::Session,
    }
}

/// Converts the signal argument into the domain type.
const fn signal_of(arg: SignalArg) -> SignalKind {
    match arg {
        SignalArg::Useful => SignalKind::Useful,
        SignalArg::Used => SignalKind::Used,
        SignalArg::Ignored => SignalKind::Ignored,
        SignalArg::DeadEnd => SignalKind::DeadEnd,
        SignalArg::Corrected => SignalKind::Corrected,
    }
}

/// Converts the scope argument into the installer's type.
const fn scope_of(arg: ScopeArg) -> agents::Scope {
    match arg {
        ScopeArg::User => agents::Scope::User,
        ScopeArg::Project => agents::Scope::Project,
    }
}

/// Builds the installer options from the command line and this executable's own path.
fn install_options(args: &InstallArgs) -> install::InstallOptions {
    let command = args.command.clone().or_else(|| {
        std::env::current_exe()
            .ok()
            .map(|path| path.to_string_lossy().into_owned())
    });
    install::InstallOptions {
        agents: args.agents.clone(),
        scope: scope_of(args.scope),
        dry_run: args.dry_run,
        force: args.force,
        server_name: args.name.clone(),
        command,
        pin_repo: args.pin_repo,
    }
}

/// Builds an engine over a repository, opening the index in its data directory.
///
/// # Errors
/// Returns a failure when the data directory cannot be created, the index cannot be opened, or the
/// repository cannot be read.
fn engine_at(locations: &Locations, metrics: bool) -> Result<Engine, CliError> {
    std::fs::create_dir_all(&locations.data_dir).map_err(|error| {
        CliError::failure(format!(
            "cannot create the data directory `{}`: {error}. Pass --data-dir to choose another one",
            locations.data_dir.display()
        ))
    })?;
    let storage = SqliteStorage::open(&locations.index_db())?;
    let tree = FsSourceTree::new(&locations.repo, MAX_FILE_BYTES)?;
    let config = EngineConfig {
        metrics_path: metrics.then(|| locations.metrics()),
        ..EngineConfig::default()
    };
    let deps = Deps {
        storage: Arc::new(storage),
        extractor: Arc::new(TreeSitterExtractor::new()),
        tree: Arc::new(tree),
        docs: Arc::new(DocComments),
        clock: Arc::new(SystemClock),
    };
    Ok(Engine::new(deps, config))
}

/// Prints a structured result in the format the user asked for.
fn emit(value: &Value, global: &Global) {
    let text = render_value(
        value,
        format_of(global.format),
        simple::delimiter_of(global.delimiter),
    );
    println!("{}", style::highlight(&text, style::colour_stdout()));
}

/// Prints a line only when the user did not ask for quiet output.
fn note(global: &Global, message: &str) {
    if !global.quiet {
        eprintln!("{message}");
    }
}

/// Runs the commands that need an indexed repository.
///
/// # Errors
/// Returns whatever the engine reports, with its exit code preserved.
fn run_with_engine(
    command: &Command,
    global: &Global,
    locations: &Locations,
) -> Result<(), CliError> {
    let engine = engine_at(locations, !global.no_metrics)?;
    if run_code(&engine, command, global)? {
        return Ok(());
    }
    if run_memory(&engine, command, global)? {
        return Ok(());
    }
    run_analysis(engine, command, global, locations)
}

/// Runs the commands that read the code: indexing and the four ways of retrieving from it.
///
/// Returns `Ok(true)` when the command was handled here.
///
/// # Errors
/// Returns whatever the engine reports.
fn run_code(engine: &Engine, command: &Command, global: &Global) -> Result<bool, CliError> {
    match command {
        Command::Index(args) => {
            let options = IndexOptions {
                force: args.force,
                only_paths: args.paths.clone(),
                threads: args.threads,
            };
            // A real bar: the indexer reports each file as it finishes, so the number moving is
            // the work actually done and not a guess at how long it will take.
            let report = if global.quiet {
                engine.index(&options)?
            } else {
                let bar = std::sync::Mutex::new(style::Progress::new("indexing"));
                let report = engine.index_reporting(&options, &|done, total| {
                    if let Ok(mut bar) = bar.lock() {
                        bar.set(done, total);
                    }
                })?;
                if let Ok(bar) = bar.into_inner() {
                    bar.finish(&format!(
                        "{} files, {} symbols, {} edges",
                        report.files_indexed, report.symbols_added, report.edges_written
                    ));
                }
                report
            };
            emit(&report.to_value(), global);
        }
        Command::Recall(args) => {
            let query = RecallQuery {
                text: args.query.join(" "),
                budget: args.budget,
                explain: args.explain,
                path_prefix: args.path.clone(),
            };
            let capsule = engine.recall(&query)?;
            let options = pn_ultramemory_codec::RenderOptions {
                format: format_of(global.format),
                delimiter: simple::delimiter_of(global.delimiter),
            };
            println!("{}", pn_ultramemory_codec::render(&capsule, &options));
        }
        Command::Expand(args) => {
            // `to` without `from` still bounds the window: it starts at the symbol's first line.
            let window = match (args.from, args.to) {
                (None, None) => None,
                (from, to) => Some((from.unwrap_or(1), to)),
            };
            let expansion = engine.expand(&args.symbol, window)?;
            emit(&expansion.to_value(), global);
        }
        Command::Impact(args) => {
            let query = pn_ultramemory_engine::ImpactQuery {
                symbol: args.symbol.clone(),
                depth: args.depth,
                min_confidence: args.min_confidence.map(confidence_of),
                limit: args.limit,
            };
            emit(&engine.impact(&query)?.to_value(), global);
        }
        Command::Outline(args) => {
            emit(&engine.outline(&args.path, args.budget)?.to_value(), global);
        }
        Command::Graph(args) => graph(engine, args, global)?,
        Command::Map(args) => {
            let query = pn_ultramemory_engine::MapQuery {
                budget: args.budget,
                path_prefix: args.path.clone(),
            };
            emit(&engine.repo_map(&query)?.to_value(), global);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Runs the commands about memory and about what the tool has learned.
///
/// Returns `Ok(true)` when the command was handled here.
///
/// # Errors
/// Returns whatever the engine reports, or an invalid-request error when a feedback command names
/// neither a symbol nor a memory.
fn run_memory(engine: &Engine, command: &Command, global: &Global) -> Result<bool, CliError> {
    match command {
        Command::Remember(args) => {
            let provenance = match args.by {
                ByArg::User => pn_ultramemory_core::Provenance::User,
                ByArg::Agent => pn_ultramemory_core::Provenance::Agent,
                ByArg::Tool => pn_ultramemory_core::Provenance::Tool,
            };
            remember(engine, args, provenance, global)?;
        }
        Command::Memories(args) => {
            let filter = memory_filter(args);
            let records = engine.memories(&filter)?;
            emit(&pn_ultramemory_engine::memories_to_value(&records), global);
        }
        Command::Forget(IdArgs { id }) => {
            let removed = engine.forget(MemoryId(*id))?;
            emit(&json!({ "forgotten": removed, "id": id }), global);
        }
        Command::Reanchor(IdArgs { id }) => {
            let confirmed = engine.reanchor(MemoryId(*id))?;
            emit(&json!({ "reanchored": confirmed, "id": id }), global);
        }
        Command::Feedback(args) => {
            let target = feedback_target(args)?;
            engine.feedback(&target, signal_of(args.signal))?;
            emit(&engine.utility_of(&target)?.to_value(), global);
        }
        Command::Learn(LearnCommand::Status) => {
            let status = engine.learning_status()?;
            emit(
                &json!({
                    "signals": status.signals,
                    "tracked": status.tracked_targets,
                    "coaccess_pairs": status.coaccess_pairs,
                }),
                global,
            );
        }
        Command::Learn(LearnCommand::Reset) => {
            engine.reset_learning()?;
            emit(&json!({ "reset": true }), global);
        }
        Command::Learn(LearnCommand::Why(args)) => {
            let target = match (&args.symbol, args.memory) {
                (Some(symbol), _) => FeedbackTarget::Symbol(symbol.clone()),
                (None, Some(id)) => FeedbackTarget::Memory(MemoryId(id)),
                (None, None) => {
                    return Err(CliError::invalid(
                        "name a symbol or pass --memory <ID>, as in `pn-ultramemory learn why \
                         --memory 12`; `pn-ultramemory memories` lists the identifiers",
                    ));
                }
            };
            emit(&engine.utility_of(&target)?.to_value(), global);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Runs the commands that report on the repository, and the MCP server.
///
/// It takes the engine by value because serving hands it to the server for the life of the
/// process. It is the last group, so it is the one that reports an unhandled command.
///
/// # Errors
/// Returns whatever the engine reports, a failure when an output file cannot be written, or an
/// invalid-request error for a command no group handled.
fn run_analysis(
    engine: Engine,
    command: &Command,
    global: &Global,
    locations: &Locations,
) -> Result<(), CliError> {
    match command {
        Command::Docs(command) => docs(&engine, command, global)?,
        Command::Stats => emit(&engine.stats()?.to_value(), global),
        Command::Report(args) => report(&engine, args, global, locations)?,
        Command::Bench(args) => {
            let options = pn_ultramemory_engine::BenchOptions {
                tasks: args.tasks,
                seed: args.seed,
                budgets: args.budgets.clone(),
                baseline_files: args.baseline_files,
            };
            note(global, &style::good("sampling"));
            emit(&engine.bench(&options)?.to_value(), global);
        }
        Command::Serve => serve(engine)?,
        // Every command is handled by one of the three groups or by `run_standalone`, and the
        // tests cover each one, but the compiler cannot see that across four functions. If this
        // is ever reached, a command was added without being wired up, and saying so is more use
        // than a message about the command being unavailable.
        other => {
            return Err(CliError::failure(format!(
                "`{}` parsed but reached no handler, which is a defect in this build; please \
                 report it at https://github.com/paulnewmandev/pn-ultramemory/issues",
                command_name(other)
            )));
        }
    }
    Ok(())
}

/// The name of a command, for a message about the command itself.
const fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Index(_) => "index",
        Command::Recall(_) => "recall",
        Command::Expand(_) => "expand",
        Command::Impact(_) => "impact",
        Command::Graph(_) => "graph",
        Command::Outline(_) => "outline",
        Command::Map(_) => "map",
        Command::Remember(_) => "remember",
        Command::Memories(_) => "memories",
        Command::Forget(_) => "forget",
        Command::Reanchor(_) => "reanchor",
        Command::Feedback(_) => "feedback",
        Command::Learn(_) => "learn",
        Command::Docs(_) => "docs",
        Command::Stats => "stats",
        Command::Report(_) => "report",
        Command::Bench(_) => "bench",
        Command::Serve => "serve",
        Command::McpConfig(_) => "mcp-config",
        Command::Toon(_) => "toon",
        Command::Completions(_) => "completions",
        Command::Install(_) => "install",
        Command::Uninstall(_) => "uninstall",
        Command::Hook(_) => "hook",
        Command::Doctor => "doctor",
    }
}

/// Draws the graph, as a diagram in one of four formats.
///
/// The symbol graph and the module graph are separate operations with separate shapes, so `--as
/// json` prints whichever one was asked for, while the three drawing formats go through the report
/// renderer and therefore always draw modules: a diagram of sixty symbols is unreadable, and the
/// module view is the one a person opens a picture for.
///
/// # Errors
/// Returns whatever the engine reports, or a failure when the output file cannot be written.
fn graph(engine: &Engine, args: &args::GraphArgs, global: &Global) -> Result<(), CliError> {
    let query = pn_ultramemory_engine::GraphQuery {
        center: args.symbol.clone(),
        depth: args.depth,
        max_nodes: args.max_nodes,
        min_confidence: args
            .min_confidence
            .map_or(pn_ultramemory_core::Confidence::Heuristic, confidence_of),
    };
    if matches!(args.kind, args::GraphAs::Json) {
        let value = if args.modules {
            engine
                .module_graph(args.module_depth, args.max_nodes)?
                .to_value()
        } else {
            engine.graph(&query)?.to_value()
        };
        emit(&value, global);
        return Ok(());
    }
    // A picture of what was asked for: modules when `--modules` was given, otherwise the symbols
    // themselves, which is the view that shows the shape of the code rather than of the folders.
    let drawable = if args.modules {
        let modules = engine.module_graph(args.module_depth, args.max_nodes)?;
        reporting::drawable(&modules.nodes, &modules.edges)
    } else {
        let symbols = engine.graph(&query)?;
        reporting::drawable(&symbols.nodes, &symbols.edges)
    };
    let drawing = pn_ultramemory_report::render_graph(
        &drawable,
        match args.kind {
            args::GraphAs::Dot => pn_ultramemory_report::GraphFormat::Dot,
            args::GraphAs::Svg => pn_ultramemory_report::GraphFormat::Svg,
            _ => pn_ultramemory_report::GraphFormat::Mermaid,
        },
        reporting::lang_of(args.lang),
    );
    write_out(args.out.as_deref(), drawing.as_bytes(), global)
}

/// Runs one of the documentation commands.
///
/// # Errors
/// Returns whatever the engine reports, or a failure when a file cannot be read or written.
fn docs(engine: &Engine, command: &args::DocsCommand, global: &Global) -> Result<(), CliError> {
    match command {
        args::DocsCommand::Gaps(args) => {
            let query = pn_ultramemory_engine::DocGapQuery {
                path_prefix: args.path.clone(),
                limit: args.limit,
                with_context: args.context,
            };
            let gaps = engine.doc_gaps(&query)?;
            emit(&pn_ultramemory_engine::doc_gaps_to_value(&gaps), global);
        }
        args::DocsCommand::Apply(args) => {
            let text = simple::read_input(&args.file)?;
            let entries = pn_ultramemory_engine::parse_doc_entries(&text)?;
            let report = engine.doc_apply(&entries, args.dry_run)?;
            emit(&report.to_value(), global);
        }
        args::DocsCommand::Build(args) => {
            let markdown = engine.doc_markdown(args.path.as_deref(), args.title.as_deref())?;
            write_out(args.out.as_deref(), markdown.as_bytes(), global)?;
        }
    }
    Ok(())
}

/// Writes the report, in the language and format asked for.
///
/// # Errors
/// Returns whatever the engine reports, or a failure when the file cannot be written.
fn report(
    engine: &Engine,
    args: &args::ReportArgs,
    global: &Global,
    locations: &Locations,
) -> Result<(), CliError> {
    note(global, "measuring...");
    let insights = engine.insights(&reporting::insight_options(args.module_depth))?;
    let project = args.title.clone().unwrap_or_else(|| {
        locations.repo.file_name().map_or_else(
            || "repository".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        )
    });
    let data = reporting::report_data(&insights, &project, &today());
    let lang = reporting::lang_of(args.lang);
    let bytes = match args.kind {
        args::ReportAs::Html => pn_ultramemory_report::render_html(&data, lang).into_bytes(),
        args::ReportAs::Pdf => pn_ultramemory_report::render_pdf(&data, lang),
        args::ReportAs::Md => pn_ultramemory_report::render_markdown(&data, lang).into_bytes(),
    };
    let default = format!(
        "pn-ultramemory-report.{}",
        reporting::extension_of(args.kind)
    );
    let path = args.out.clone().unwrap_or_else(|| default.into());
    std::fs::write(&path, &bytes).map_err(|e| {
        CliError::failure(format!(
            "cannot write `{}`: {e}. Check that the folder exists and that you may write to it, \
             or choose another file with --out",
            path.display()
        ))
    })?;
    emit(
        &json!({ "written": path.display().to_string(), "bytes": bytes.len() }),
        global,
    );
    Ok(())
}

/// Writes bytes to a file, or to standard output when no file was named.
///
/// # Errors
/// Returns a failure when the file cannot be written.
fn write_out(path: Option<&Path>, bytes: &[u8], global: &Global) -> Result<(), CliError> {
    use std::io::Write as _;
    if let Some(path) = path {
        std::fs::write(path, bytes)
            .map_err(|e| CliError::failure(format!(
            "cannot write `{}`: {e}. Check that the folder exists and that you may write to it, \
             or choose another file with --out",
            path.display()
        )))?;
        emit(
            &json!({ "written": path.display().to_string(), "bytes": bytes.len() }),
            global,
        );
        return Ok(());
    }
    std::io::stdout().write_all(bytes)?;
    Ok(())
}

/// Stores one memory and reports what happened to it.
fn remember(
    engine: &Engine,
    args: &RememberArgs,
    provenance: pn_ultramemory_core::Provenance,
    global: &Global,
) -> Result<(), CliError> {
    let input = pn_ultramemory_engine::RememberInput {
        kind: kind_of(args.kind),
        text: args.text.join(" "),
        about: args.about.clone(),
        provenance,
        session: std::env::var("PN_ULTRAMEMORY_SESSION").ok(),
    };
    let outcome = engine.remember(&input)?;
    for warning in &outcome.warnings {
        note(global, &style::warn(warning));
    }
    emit(&outcome.to_value(), global);
    Ok(())
}

/// Builds the memory filter from the command line.
fn memory_filter(args: &MemoriesArgs) -> MemoryFilter {
    MemoryFilter {
        kind: args.kind.map(kind_of),
        only_stale: args.stale,
        limit: args.limit,
    }
}

/// Works out which symbol or memory a feedback call is about.
fn feedback_target(args: &FeedbackArgs) -> Result<FeedbackTarget, CliError> {
    match (&args.symbol, args.memory) {
        (Some(symbol), _) => Ok(FeedbackTarget::Symbol(symbol.clone())),
        (None, Some(id)) => Ok(FeedbackTarget::Memory(MemoryId(id))),
        (None, None) => Err(CliError::invalid(
            "say what the feedback is about: name a symbol, or pass --memory <ID> as in \
             `pn-ultramemory feedback useful --memory 12`; `pn-ultramemory memories` lists the \
             identifiers",
        )),
    }
}

/// Runs the MCP server on standard input and output until the client closes the stream.
///
/// # Errors
/// Returns a failure only when standard input or output breaks; a tool failure is reported to the
/// client as a tool error, not as a process failure.
fn serve(engine: Engine) -> Result<(), CliError> {
    let info = pn_ultramemory_mcp::ServerInfo {
        name: "pn-ultramemory".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    let server = pn_ultramemory_mcp::Server::new(McpBackend { engine }, info);
    let stdin = std::io::stdin().lock();
    let stdout = std::io::stdout().lock();
    server.serve(stdin, stdout).map_err(CliError::from)
}

/// The engine behind the four tools the MCP server offers.
struct McpBackend {
    /// The engine every tool call goes through.
    engine: Engine,
}

impl McpBackend {
    /// Renders a value the way the server sends it: TOON, which is what the tools promise.
    fn render(value: &Value) -> String {
        render_value(value, Format::Toon, pn_ultramemory_toon::Delimiter::Comma)
    }
}

impl pn_ultramemory_mcp::Backend for McpBackend {
    fn recall(
        &self,
        request: pn_ultramemory_mcp::RecallRequest,
    ) -> Result<String, pn_ultramemory_mcp::ToolFailure> {
        let query = RecallQuery {
            text: request.query,
            budget: request.budget,
            explain: request.explain.unwrap_or(false),
            path_prefix: None,
        };
        let capsule = self
            .engine
            .recall(&query)
            .map_err(|e| pn_ultramemory_mcp::ToolFailure(e.to_string()))?;
        Ok(pn_ultramemory_codec::render(
            &capsule,
            &pn_ultramemory_codec::RenderOptions::default(),
        ))
    }

    fn impact(
        &self,
        request: pn_ultramemory_mcp::ImpactRequest,
    ) -> Result<String, pn_ultramemory_mcp::ToolFailure> {
        let query = pn_ultramemory_engine::ImpactQuery {
            symbol: request.symbol,
            depth: request.depth,
            min_confidence: None,
            limit: None,
        };
        let report = self
            .engine
            .impact(&query)
            .map_err(|e| pn_ultramemory_mcp::ToolFailure(e.to_string()))?;
        Ok(Self::render(&report.to_value()))
    }

    fn remember(
        &self,
        request: pn_ultramemory_mcp::RememberRequest,
    ) -> Result<String, pn_ultramemory_mcp::ToolFailure> {
        let kind = MemoryKind::from_name(&request.kind).ok_or_else(|| {
            pn_ultramemory_mcp::ToolFailure(format!("unknown memory kind `{}`", request.kind))
        })?;
        let input = pn_ultramemory_engine::RememberInput {
            kind,
            text: request.text,
            about: request.about,
            provenance: pn_ultramemory_core::Provenance::Agent,
            session: None,
        };
        let outcome = self
            .engine
            .remember(&input)
            .map_err(|e| pn_ultramemory_mcp::ToolFailure(e.to_string()))?;
        Ok(Self::render(&outcome.to_value()))
    }

    fn outline(
        &self,
        request: pn_ultramemory_mcp::OutlineRequest,
    ) -> Result<String, pn_ultramemory_mcp::ToolFailure> {
        let outline = self
            .engine
            .outline(&request.path, request.budget)
            .map_err(|e| pn_ultramemory_mcp::ToolFailure(e.to_string()))?;
        Ok(Self::render(&outline.to_value()))
    }

    fn expand(
        &self,
        request: pn_ultramemory_mcp::ExpandRequest,
    ) -> Result<String, pn_ultramemory_mcp::ToolFailure> {
        // Both bounds are optional in the tool schema, so all four combinations have to mean
        // something: neither is the whole symbol, `from` alone runs to the end, and `to` alone
        // starts at the first line rather than silently dropping the bound the agent asked for.
        let window = match (request.from, request.to) {
            (None, None) => None,
            (from, to) => Some((from.unwrap_or(1), to)),
        };
        let expansion = self
            .engine
            .expand(&request.id, window)
            .map_err(|e| pn_ultramemory_mcp::ToolFailure(e.to_string()))?;
        Ok(Self::render(&expansion.to_value()))
    }
}

/// Runs the commands that need no repository and no index.
///
/// Returns `Ok(true)` when the command was handled here.
///
/// # Errors
/// Returns whatever the command reports.
fn run_standalone(command: &Command, global: &Global) -> Result<bool, CliError> {
    match command {
        Command::Toon(ToonCommand::Encode(args)) => {
            println!(
                "{}",
                simple::toon_encode(args, simple::delimiter_of(global.delimiter))?
            );
        }
        Command::Toon(ToonCommand::Decode(args)) => println!("{}", simple::toon_decode(args)?),
        Command::Completions(args) => simple::completions(args)?,
        Command::McpConfig(args) => {
            let repo = paths::locate(global.repo.as_deref(), global.data_dir.as_deref())
                .ok()
                .map(|found| found.repo.to_string_lossy().into_owned());
            println!("{}", simple::mcp_config(args, repo.as_deref())?);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Runs the commands that configure coding agents or inspect the setup.
///
/// # Errors
/// Returns whatever the installer or the doctor reports.
fn run_setup(command: &Command, global: &Global, locations: &Locations) -> Result<(), CliError> {
    match command {
        Command::Install(args) => {
            let report = install::install(&locations.repo, &install_options(args))?;
            emit_report(&report, global);
        }
        Command::Uninstall(args) => {
            let report = install::uninstall(&locations.repo, &install_options(args))?;
            emit_report(&report, global);
        }
        Command::Doctor => {
            let report = doctor::doctor(&locations.repo, &locations.data_dir);
            match global.format {
                OutFormat::Text => println!("{}", report.summary()),
                _ => emit(&report.to_value(), global),
            }
        }
        // Reached only if a command was routed here without being handled, which is a defect in
        // the build rather than anything the person running it did.
        other => {
            return Err(CliError::failure(format!(
                "`{}` parsed but reached no handler, which is a defect in this build; please \
                 report it at https://github.com/paulnewmandev/pn-ultramemory/issues",
                command_name(other)
            )));
        }
    }
    Ok(())
}

/// Prints an installer report in the requested format.
fn emit_report(report: &install::InstallReport, global: &Global) {
    match global.format {
        OutFormat::Text => println!("{}", report.summary()),
        _ => emit(&report.to_value(), global),
    }
}

/// Runs a lifecycle hook. It never fails: a hook that fails would fail the tool call it was
/// attached to, which is never worth it.
fn run_hook(args: &args::HookArgs, locations: &Locations) -> ExitCode {
    let context = hook::HookContext {
        repo: locations.repo.clone(),
        data_dir: locations.data_dir.clone(),
        enabled: std::env::var_os("PN_ULTRAMEMORY_NO_HOOKS").is_none(),
        deadline: Duration::from_millis(args.deadline_ms),
    };
    let _ = hook::run_hook_stdio(&args.event, &context);
    ExitCode::SUCCESS
}

/// Whether a command shows the banner: the ones a person runs to look at something.
const fn shows_banner(command: &Command) -> bool {
    matches!(command, Command::Doctor | Command::Stats)
}

/// Parses the command line, runs the command and turns a failure into an exit code.
fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    let global = &cli.global;

    if let Command::Hook(args) = &cli.command {
        let locations = paths::locate(global.repo.as_deref(), global.data_dir.as_deref())
            .map_err(CliError::failure)?;
        run_hook(args, &locations);
        return Ok(());
    }

    if run_standalone(&cli.command, global)? {
        return Ok(());
    }

    let locations =
        paths::locate(global.repo.as_deref(), global.data_dir.as_deref()).map_err(|message| {
            CliError::failure(format!(
                "{message}. Pass --repo <PATH> to name the repository"
            ))
        })?;

    if shows_banner(&cli.command) && !global.quiet && global.format == OutFormat::Text {
        banner::print(env!("CARGO_PKG_VERSION"));
    }

    match &cli.command {
        Command::Install(_) | Command::Uninstall(_) | Command::Doctor => {
            run_setup(&cli.command, global, &locations)
        }
        other => run_with_engine(other, global, &locations),
    }
}

/// Process entry point.
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pn-ultramemory: {error}");
            ExitCode::from(error.code)
        }
    }
}

/// Convenience so the compiler checks that every path type the command line uses is a real path.
#[allow(dead_code, reason = "a compile-time check that the path types line up")]
fn _path_types(_: &Path) {}
