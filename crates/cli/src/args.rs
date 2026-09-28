// SPDX-License-Identifier: Apache-2.0
//! The command-line surface: every command, flag and value, with its help text.
//!
//! The doc comments on these items are what `--help` prints, so they are written for people who
//! use the tool, not for people who read the code.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Text printed after the option list of `--help`.
const EXAMPLES: &str = "\
EXAMPLES:
    pn-ultramemory index                       Build or refresh the index of this repository
    pn-ultramemory recall \"parse the config\"    The code that answers a question, within a token budget
    pn-ultramemory recall load_config -b 800   A smaller capsule
    pn-ultramemory impact load_config          What depends on this symbol, and how sure we are
    pn-ultramemory remember decision \"Use a file, not env vars\" --about load_config
    pn-ultramemory docs gaps --context         Undocumented symbols, ready to hand to an agent
    pn-ultramemory report --lang es --as pdf   A report in Spanish, as a PDF
    pn-ultramemory serve                       Run the MCP server on standard input and output

Everything runs on your machine: no network calls, no telemetry, no accounts.
";

/// A code-aware, learning memory for coding agents: fewer tokens, sharper recall.
///
/// It indexes your repository into a graph of symbols and relationships, answers questions with a
/// small budgeted "capsule" of the code that matters, remembers decisions and lessons anchored to
/// the code they describe, and learns from what actually proved useful. It is free, local and
/// works offline.
#[derive(Debug, Parser)]
#[command(
    name = "pn-ultramemory",
    version,
    propagate_version = true,
    arg_required_else_help = true,
    after_help = EXAMPLES
)]
pub struct Cli {
    /// Options that apply to every command.
    #[command(flatten)]
    pub global: Global,
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// Options shared by every command.
#[derive(Debug, Clone, Args)]
pub struct Global {
    /// The repository to work on. Defaults to the closest parent directory that contains `.git`,
    /// or the current directory.
    #[arg(
        short = 'C',
        long,
        global = true,
        value_name = "PATH",
        env = "PN_ULTRAMEMORY_REPO"
    )]
    pub repo: Option<PathBuf>,

    /// Where the index and the usage counters are kept. Defaults to a folder in your user data
    /// directory, one per repository, so nothing is written inside your repository.
    #[arg(long, global = true, value_name = "PATH", env = "PN_ULTRAMEMORY_HOME")]
    pub data_dir: Option<PathBuf>,

    /// Output format for results.
    #[arg(short, long, global = true, value_enum, default_value_t = OutFormat::Toon)]
    pub format: OutFormat,

    /// Column separator of TOON tables. Tab is marginally cheaper in tokens.
    #[arg(long, global = true, value_enum, default_value_t = DelimiterArg::Comma)]
    pub delimiter: DelimiterArg,

    /// Print only the result, no progress or status messages.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Do not record local usage counters (numbers only, never text).
    #[arg(long, global = true, env = "PN_ULTRAMEMORY_NO_METRICS")]
    pub no_metrics: bool,
}

/// How results are printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutFormat {
    /// TOON: compact, token-efficient structure. The default.
    Toon,
    /// JSON, for programs.
    Json,
    /// Short human-readable text.
    Text,
}

/// The separator between the cells of a TOON table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DelimiterArg {
    /// A comma.
    Comma,
    /// A tab character.
    Tab,
    /// A vertical bar.
    Pipe,
}

/// The kinds of memory that can be stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum KindArg {
    /// A choice that was made, with its reasons.
    Decision,
    /// A stable statement about the code or the project.
    Fact,
    /// Something learned from experience.
    Lesson,
    /// An approach that was tried and abandoned.
    DeadEnd,
    /// A recurring error and the fix that resolved it.
    ErrorFix,
    /// A rule the codebase follows.
    Convention,
    /// Something the system must do, anchored to the code that implements it.
    Requirement,
    /// A unit of work.
    Task,
    /// A summary of a working session.
    Session,
}

/// Who wrote a memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ByArg {
    /// A person. The default for commands typed at the terminal.
    User,
    /// A coding agent.
    Agent,
    /// Text captured from a tool or a web page. Treated as untrusted.
    Tool,
}

/// The minimum confidence of an edge to follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ConfidenceArg {
    /// A name match with no supporting evidence.
    Guess,
    /// A structural hint, such as a name that is unique in the repository.
    Heuristic,
    /// Resolved through scopes or imports.
    Resolved,
    /// Stated directly by the syntax.
    Exact,
}

/// What happened to something that was recalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SignalArg {
    /// It was useful.
    Useful,
    /// It was used.
    Used,
    /// It was shown and ignored.
    Ignored,
    /// It led nowhere.
    DeadEnd,
    /// It was wrong and had to be corrected.
    Corrected,
}

/// The language of a generated report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LangArg {
    /// English.
    En,
    /// Spanish.
    Es,
}

/// The file type of a generated report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ReportAs {
    /// One self-contained HTML file, with no scripts and no external resources.
    Html,
    /// A PDF document.
    Pdf,
    /// Markdown.
    Md,
}

/// The file type of an exported graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum GraphAs {
    /// A Mermaid flowchart, which GitHub renders.
    Mermaid,
    /// Graphviz DOT.
    Dot,
    /// A standalone SVG picture.
    Svg,
    /// Nodes and edges as JSON.
    Json,
}

/// The shells that completions can be generated for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ShellArg {
    /// Bash.
    Bash,
    /// Zsh.
    Zsh,
    /// Fish.
    Fish,
    /// PowerShell.
    Powershell,
    /// Elvish.
    Elvish,
}

/// Everything the tool can do.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Build or refresh the index of the repository. Only files that changed are read again.
    Index(IndexArgs),
    /// Answer a question with the code that matters, packed to a token budget.
    Recall(RecallArgs),
    /// Show the source of one symbol, in windows of lines.
    Expand(ExpandArgs),
    /// Show what depends on a symbol, and how sure the answer is.
    Impact(ImpactArgs),
    /// Export the code graph as Mermaid, DOT, SVG or JSON.
    Graph(GraphArgs),
    /// Everything a new session needs to know about this repository, inside a token budget.
    Brief(BriefArgs),
    /// Describe one whole file: every symbol it declares, for a fraction of the tokens.
    Outline(OutlineArgs),
    /// Print a compact map of the repository within a token budget.
    Map(MapArgs),
    /// Store a memory, anchored to the code it is about.
    Remember(RememberArgs),
    /// List stored memories.
    Memories(MemoriesArgs),
    /// Delete a memory.
    Forget(IdArgs),
    /// Confirm that a stale memory is still true after the code changed.
    Reanchor(IdArgs),
    /// Tell the tool how something that was recalled turned out, so it can learn.
    Feedback(FeedbackArgs),
    /// Inspect or reset what has been learned.
    #[command(subcommand)]
    Learn(LearnCommand),
    /// Find undocumented symbols, apply documentation, or build an API reference.
    #[command(subcommand)]
    Docs(DocsCommand),
    /// Show index and usage statistics.
    Stats,
    /// Generate a report as HTML, PDF or Markdown, in English or Spanish.
    Report(ReportArgs),
    /// Measure retrieval quality and token use on this repository, offline.
    Bench(BenchArgs),
    /// Run the MCP server on standard input and output.
    Serve,
    /// Print the configuration snippet that connects an MCP client to this tool.
    McpConfig(McpConfigArgs),
    /// Convert between JSON and TOON.
    #[command(subcommand)]
    Toon(ToonCommand),
    /// Print a shell completion script.
    Completions(CompletionsArgs),
    /// Register this tool as an MCP server in the coding agents found on this machine.
    Install(InstallArgs),
    /// Remove the entry this tool wrote, leaving the rest of the file untouched.
    Uninstall(InstallArgs),
    /// Run a lifecycle hook. Reads JSON on standard input and writes JSON on standard output.
    #[command(hide = true)]
    Hook(HookArgs),
    /// Check that everything is set up and print where things are.
    Doctor,
}

/// Arguments of `index`.
#[derive(Debug, Args)]
pub struct IndexArgs {
    /// Read every file again, even if it did not change.
    #[arg(long)]
    pub force: bool,
    /// Parsing threads. Defaults to the number of cores, at most eight.
    #[arg(long, value_name = "N")]
    pub threads: Option<usize>,
    /// Index only these files (paths relative to the repository).
    #[arg(value_name = "PATH")]
    pub paths: Vec<String>,
}

/// Arguments of `recall`.
#[derive(Debug, Args)]
pub struct RecallArgs {
    /// The question or the name to look for. Several words are joined.
    #[arg(required = true, value_name = "QUERY")]
    pub query: Vec<String>,
    /// Token budget of the answer.
    #[arg(short, long, value_name = "TOKENS")]
    pub budget: Option<u32>,
    /// Explain why each result was chosen.
    #[arg(long)]
    pub explain: bool,
    /// Only look inside paths that start with this prefix.
    #[arg(long, value_name = "PREFIX")]
    pub path: Option<String>,
}

/// Arguments of `expand`.
#[derive(Debug, Args)]
pub struct ExpandArgs {
    /// A symbol name, a file and name such as `src/lib.rs:parse`, or a numeric id.
    #[arg(value_name = "SYMBOL")]
    pub symbol: String,
    /// First line to show, counted from the start of the symbol, starting at 1.
    #[arg(long, value_name = "LINE")]
    pub from: Option<u32>,
    /// Last line to show, counted from the start of the symbol.
    #[arg(long, value_name = "LINE")]
    pub to: Option<u32>,
}

/// Arguments of `impact`.
#[derive(Debug, Args)]
pub struct ImpactArgs {
    /// A symbol name, a file and name such as `src/lib.rs:parse`, or a numeric id.
    #[arg(value_name = "SYMBOL")]
    pub symbol: String,
    /// How many steps of callers to follow, at most five.
    #[arg(long, value_name = "N")]
    pub depth: Option<u32>,
    /// The weakest kind of edge to follow.
    #[arg(long, value_enum)]
    pub min_confidence: Option<ConfidenceArg>,
    /// Stop after this many callers.
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,
}

/// Arguments of `graph`.
#[derive(Debug, Args)]
pub struct GraphArgs {
    /// Center the graph on this symbol. Without it, the most connected symbols are shown.
    #[arg(value_name = "SYMBOL")]
    pub symbol: Option<String>,
    /// Show modules (directories) instead of symbols.
    #[arg(long)]
    pub modules: bool,
    /// How many directory levels form a module.
    #[arg(long, value_name = "N", default_value_t = 2)]
    pub module_depth: usize,
    /// How many steps away from the center to include.
    #[arg(long, value_name = "N", default_value_t = 2)]
    pub depth: u32,
    /// The most nodes to draw.
    #[arg(long, value_name = "N", default_value_t = 60)]
    pub max_nodes: usize,
    /// The weakest kind of edge to follow.
    #[arg(long, value_enum)]
    pub min_confidence: Option<ConfidenceArg>,
    /// The kind of file to produce.
    #[arg(long = "as", value_enum, default_value_t = GraphAs::Mermaid)]
    pub kind: GraphAs,
    /// Language of the labels and legends.
    #[arg(long, value_enum, default_value_t = LangArg::En)]
    pub lang: LangArg,
    /// Write to this file instead of standard output.
    #[arg(short, long, value_name = "FILE")]
    pub out: Option<PathBuf>,
}

/// Arguments of `brief`.
#[derive(Debug, Args)]
pub struct BriefArgs {
    /// Token budget. Sections are trimmed from the bottom to fit it.
    #[arg(short, long, value_name = "TOKENS")]
    pub budget: Option<u32>,
}

/// Arguments of `outline`.
#[derive(Debug, Args)]
pub struct OutlineArgs {
    /// The file to describe, as a path inside the repository.
    #[arg(value_name = "PATH")]
    pub path: String,
    /// Token budget. Detail is lowered to fit it; symbols are never dropped.
    #[arg(short, long, value_name = "TOKENS")]
    pub budget: Option<u32>,
}

/// Arguments of `map`.
#[derive(Debug, Args)]
pub struct MapArgs {
    /// Token budget of the map.
    #[arg(short, long, value_name = "TOKENS")]
    pub budget: Option<u32>,
    /// Only include paths that start with this prefix.
    #[arg(long, value_name = "PREFIX")]
    pub path: Option<String>,
}

/// Arguments of `remember`.
#[derive(Debug, Args)]
pub struct RememberArgs {
    /// What kind of memory this is.
    #[arg(value_enum)]
    pub kind: KindArg,
    /// The text of the memory. Several words are joined.
    #[arg(required = true, value_name = "TEXT")]
    pub text: Vec<String>,
    /// A symbol this memory is about. Repeat for several. The memory becomes stale if it changes.
    #[arg(long, value_name = "SYMBOL")]
    pub about: Vec<String>,
    /// Who wrote it.
    #[arg(long, value_enum, default_value_t = ByArg::User)]
    pub by: ByArg,
}

/// Arguments of `memories`.
#[derive(Debug, Args)]
pub struct MemoriesArgs {
    /// Only memories of this kind.
    #[arg(long, value_enum)]
    pub kind: Option<KindArg>,
    /// Only memories whose code changed since they were written.
    #[arg(long)]
    pub stale: bool,
    /// The most memories to list.
    #[arg(long, value_name = "N", default_value_t = 50)]
    pub limit: usize,
}

/// An argument that is a memory id.
#[derive(Debug, Args)]
pub struct IdArgs {
    /// The numeric id of the memory.
    #[arg(value_name = "ID")]
    pub id: i64,
}

/// Arguments of `feedback`.
#[derive(Debug, Args)]
pub struct FeedbackArgs {
    /// How the thing turned out.
    #[arg(value_enum)]
    pub signal: SignalArg,
    /// The symbol it is about: a name, a file and name, or a numeric id.
    #[arg(
        value_name = "SYMBOL",
        required_unless_present = "memory",
        conflicts_with = "memory"
    )]
    pub symbol: Option<String>,
    /// The id of a memory it is about, instead of a symbol.
    #[arg(long, value_name = "ID")]
    pub memory: Option<i64>,
}

/// What can be done with learned evidence.
#[derive(Debug, Subcommand)]
pub enum LearnCommand {
    /// Show how much has been learned.
    Status,
    /// Forget everything that was learned. Symbols and memories are kept.
    Reset,
    /// Explain how one symbol or memory is ranked, and why.
    Why(WhyArgs),
}

/// Arguments of `learn why`.
#[derive(Debug, Args)]
pub struct WhyArgs {
    /// The symbol: a name, a file and name, or a numeric id.
    #[arg(
        value_name = "SYMBOL",
        required_unless_present = "memory",
        conflicts_with = "memory"
    )]
    pub symbol: Option<String>,
    /// The id of a memory, instead of a symbol.
    #[arg(long, value_name = "ID")]
    pub memory: Option<i64>,
}

/// What can be done about documentation.
#[derive(Debug, Subcommand)]
pub enum DocsCommand {
    /// List public symbols that have no documentation, optionally with the context an agent needs.
    Gaps(GapsArgs),
    /// Insert documentation written for those symbols into the source files.
    Apply(ApplyArgs),
    /// Write a Markdown API reference of the repository, for any language.
    Build(BuildArgs),
}

/// Arguments of `docs gaps`.
#[derive(Debug, Args)]
pub struct GapsArgs {
    /// Only paths that start with this prefix.
    #[arg(long, value_name = "PREFIX")]
    pub path: Option<String>,
    /// The most symbols to list.
    #[arg(long, value_name = "N", default_value_t = 50)]
    pub limit: usize,
    /// Include a short context for each symbol: callers, callees and the first lines.
    #[arg(long)]
    pub context: bool,
}

/// Arguments of `docs apply`.
#[derive(Debug, Args)]
pub struct ApplyArgs {
    /// A TOON or JSON file with the documentation, or `-` (the default) to read standard input.
    /// Format: `docs[N]{symbol,text}:` rows, or a JSON array of `{"symbol", "text"}` objects.
    #[arg(value_name = "FILE", default_value = "-")]
    pub file: String,
    /// Show what would change without touching any file.
    #[arg(long)]
    pub dry_run: bool,
}

/// Arguments of `docs build`.
#[derive(Debug, Args)]
pub struct BuildArgs {
    /// Only paths that start with this prefix.
    #[arg(long, value_name = "PREFIX")]
    pub path: Option<String>,
    /// The title of the document.
    #[arg(long, value_name = "TEXT")]
    pub title: Option<String>,
    /// Write to this file instead of standard output.
    #[arg(short, long, value_name = "FILE")]
    pub out: Option<PathBuf>,
}

/// Arguments of `report`.
#[derive(Debug, Args)]
pub struct ReportArgs {
    /// The language of the report.
    #[arg(long, value_enum, default_value_t = LangArg::En)]
    pub lang: LangArg,
    /// The kind of file to produce.
    #[arg(long = "as", value_enum, default_value_t = ReportAs::Html)]
    pub kind: ReportAs,
    /// Where to write it. Defaults to `pn-ultramemory-report.<extension>` in the current directory.
    #[arg(short, long, value_name = "FILE")]
    pub out: Option<PathBuf>,
    /// How many directory levels form a module in the module table and graph.
    #[arg(long, value_name = "N", default_value_t = 2)]
    pub module_depth: usize,
    /// A name for the project in the report. Defaults to the repository folder name.
    #[arg(long, value_name = "TEXT")]
    pub title: Option<String>,
}

/// Arguments of `bench`.
#[derive(Debug, Args)]
pub struct BenchArgs {
    /// How many tasks to sample.
    #[arg(long, value_name = "N", default_value_t = 100)]
    pub tasks: usize,
    /// Seed of the sampling, so a run can be repeated exactly.
    #[arg(long, value_name = "N", default_value_t = 1)]
    pub seed: u64,
    /// A token budget to test. Repeat for several.
    #[arg(long = "budget", short, value_name = "TOKENS")]
    pub budgets: Vec<u32>,
    /// How many whole files the baseline reads.
    #[arg(long, value_name = "N", default_value_t = 3)]
    pub baseline_files: usize,
}

/// Arguments of `mcp-config`.
#[derive(Debug, Args)]
pub struct McpConfigArgs {
    /// The name the client shows for this server.
    #[arg(long, value_name = "NAME", default_value = "pn-ultramemory")]
    pub name: String,
    /// The command that starts the tool. Defaults to this executable.
    #[arg(long, value_name = "PATH")]
    pub command: Option<String>,
}

/// Conversions between JSON and TOON.
#[derive(Debug, Subcommand)]
pub enum ToonCommand {
    /// Read JSON and write TOON.
    Encode(ToonArgs),
    /// Read TOON and write JSON.
    Decode(ToonArgs),
}

/// Arguments of `toon encode` and `toon decode`.
#[derive(Debug, Args)]
pub struct ToonArgs {
    /// The input file, or `-` (the default) to read standard input.
    #[arg(value_name = "FILE", default_value = "-")]
    pub file: String,
    /// Spaces per indentation level.
    #[arg(long, value_name = "N", default_value_t = 2)]
    pub indent: usize,
    /// Skip the strict checks of the TOON specification when decoding.
    #[arg(long)]
    pub lenient: bool,
}

/// Whether a configuration change applies to the whole machine or only to this repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ScopeArg {
    /// The agent's own configuration, for every repository you open with it.
    User,
    /// A file inside this repository, so the setting travels with the project.
    Project,
}

/// Arguments of `install` and `uninstall`.
#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Only this agent. Repeat for several. Without it, every agent found on this machine.
    #[arg(long = "agents", value_name = "ID")]
    pub agents: Vec<String>,
    /// Whether to write the agent's own configuration or a file inside this repository.
    #[arg(long, value_enum, default_value_t = ScopeArg::User)]
    pub scope: ScopeArg,
    /// Show what would change without touching any file.
    #[arg(long)]
    pub dry_run: bool,
    /// Replace an entry that was edited by hand.
    #[arg(long)]
    pub force: bool,
    /// The name the agent will show for this server.
    #[arg(long, value_name = "NAME", default_value = "pn-ultramemory")]
    pub name: String,
    /// The command that starts the server. Defaults to this executable's own path.
    #[arg(long, value_name = "PATH")]
    pub command: Option<String>,
    /// Pin the entry to this repository, so the agent always indexes it.
    #[arg(long)]
    pub pin_repo: bool,
}

/// Arguments of `hook`.
#[derive(Debug, Args)]
pub struct HookArgs {
    /// Which lifecycle event fired.
    #[arg(value_name = "EVENT")]
    pub event: String,
    /// How long the hook may take before it gives up and says nothing.
    #[arg(long, value_name = "MS", default_value_t = 400)]
    pub deadline_ms: u64,
}

/// Arguments of `completions`.
#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// The shell to generate the script for.
    #[arg(value_enum)]
    pub shell: ShellArg,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    /// The definition is internally consistent: clap checks every flag, default and conflict.
    #[test]
    fn the_command_line_definition_is_valid() {
        Cli::command().debug_assert();
    }

    /// Every command documents itself, so `--help` never prints an empty description.
    #[test]
    fn every_command_has_help_text() {
        let command = Cli::command();
        for subcommand in command.get_subcommands() {
            assert!(
                subcommand.get_about().is_some(),
                "{} has no help",
                subcommand.get_name()
            );
        }
    }
}
