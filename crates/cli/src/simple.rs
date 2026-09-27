// SPDX-License-Identifier: Apache-2.0
//! The commands that need no repository: TOON conversion, MCP configuration and shell
//! completions.

use std::io::{Read, Write};

use clap::CommandFactory;
use clap_complete::Shell;
use pn_ultramemory_toon::{DecodeOptions, Delimiter, EncodeOptions};
use serde_json::{Value, json};

use crate::args::{Cli, CompletionsArgs, DelimiterArg, McpConfigArgs, ShellArg, ToonArgs};
use crate::error::CliError;

/// Converts the delimiter argument into the library's type.
#[must_use]
pub const fn delimiter_of(arg: DelimiterArg) -> Delimiter {
    match arg {
        DelimiterArg::Comma => Delimiter::Comma,
        DelimiterArg::Tab => Delimiter::Tab,
        DelimiterArg::Pipe => Delimiter::Pipe,
    }
}

/// Reads a file, or standard input when the name is `-`.
///
/// # Errors
/// Returns a failure when the file cannot be read or is not valid UTF-8.
pub fn read_input(file: &str) -> Result<String, CliError> {
    if file == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|error| CliError::failure(format!("cannot read standard input: {error}")))?;
        return Ok(text);
    }
    std::fs::read_to_string(file)
        .map_err(|error| CliError::failure(format!("cannot read `{file}`: {error}")))
}

/// Converts JSON text to TOON text.
///
/// # Errors
/// Returns an invalid-request error when the input is not valid JSON.
pub fn json_to_toon(input: &str, delimiter: Delimiter, indent: usize) -> Result<String, CliError> {
    let value: Value = serde_json::from_str(input)
        .map_err(|error| CliError::invalid(format!("the input is not valid JSON: {error}")))?;
    Ok(pn_ultramemory_toon::encode(
        &value,
        &EncodeOptions { indent, delimiter },
    ))
}

/// Converts TOON text to pretty-printed JSON text.
///
/// # Errors
/// Returns an invalid-request error, with the line number, when the input is not valid TOON.
pub fn toon_to_json(input: &str, indent: usize, strict: bool) -> Result<String, CliError> {
    let value =
        pn_ultramemory_toon::decode(input, &DecodeOptions { indent, strict }).map_err(|error| {
            CliError::invalid(format!(
                "the input is not valid TOON (line {}): {}",
                error.line(),
                error.message()
            ))
        })?;
    serde_json::to_string_pretty(&value).map_err(|error| CliError::failure(error.to_string()))
}

/// Runs `toon encode`.
///
/// # Errors
/// Returns a failure when the input cannot be read or converted.
pub fn toon_encode(args: &ToonArgs, delimiter: Delimiter) -> Result<String, CliError> {
    json_to_toon(&read_input(&args.file)?, delimiter, args.indent)
}

/// Runs `toon decode`.
///
/// # Errors
/// Returns a failure when the input cannot be read or converted.
pub fn toon_decode(args: &ToonArgs) -> Result<String, CliError> {
    toon_to_json(&read_input(&args.file)?, args.indent, !args.lenient)
}

/// The configuration snippet that registers this tool as an MCP server: the common
/// `mcpServers` shape understood by MCP clients.
///
/// # Examples
/// ```text
/// {
///   "mcpServers": {
///     "pn-ultramemory": { "command": "pn-ultramemory", "args": ["serve"] }
///   }
/// }
/// ```
///
/// # Errors
/// Returns a failure when the snippet cannot be serialized.
pub fn mcp_config(args: &McpConfigArgs, repo: Option<&str>) -> Result<String, CliError> {
    let command = args
        .command
        .clone()
        .unwrap_or_else(|| "pn-ultramemory".to_owned());
    let mut server_args = Vec::new();
    if let Some(repo) = repo {
        server_args.push("--repo".to_owned());
        server_args.push(repo.to_owned());
    }
    server_args.push("serve".to_owned());
    let snippet = json!({
        "mcpServers": { args.name.clone(): { "command": command, "args": server_args } }
    });
    serde_json::to_string_pretty(&snippet).map_err(|error| CliError::failure(error.to_string()))
}

/// Writes the completion script of a shell to standard output.
///
/// # Errors
/// Returns a failure when standard output cannot be written.
pub fn completions(args: &CompletionsArgs) -> Result<(), CliError> {
    let shell = match args.shell {
        ShellArg::Bash => Shell::Bash,
        ShellArg::Zsh => Shell::Zsh,
        ShellArg::Fish => Shell::Fish,
        ShellArg::Powershell => Shell::PowerShell,
        ShellArg::Elvish => Shell::Elvish,
    };
    let mut command = Cli::command();
    let mut buffer: Vec<u8> = Vec::new();
    clap_complete::generate(shell, &mut command, "pn-ultramemory", &mut buffer);
    std::io::stdout().write_all(&buffer)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{json_to_toon, mcp_config, toon_to_json};
    use crate::args::McpConfigArgs;
    use pn_ultramemory_toon::Delimiter;

    /// JSON survives a trip through TOON and back.
    #[test]
    fn json_round_trips_through_toon() {
        let json = r#"{"symbols":[{"id":1,"name":"a"},{"id":2,"name":"b"}],"ok":true}"#;
        let toon = json_to_toon(json, Delimiter::Comma, 2).expect("encode");
        assert!(toon.contains("symbols[2]{id,name}:"));
        let back = toon_to_json(&toon, 2, true).expect("decode");
        let original: serde_json::Value = serde_json::from_str(json).expect("json");
        let restored: serde_json::Value = serde_json::from_str(&back).expect("json");
        assert_eq!(original, restored);
    }

    /// Bad input is an invalid-request error with a useful message, never a panic.
    #[test]
    fn bad_input_is_reported() {
        let error = json_to_toon("{not json", Delimiter::Comma, 2).expect_err("invalid json");
        assert_eq!(error.code, 2);
        let error = toon_to_json("items[3]: a,b", 2, true).expect_err("count mismatch");
        assert!(error.message.contains("line 1"), "{error}");
        assert!(toon_to_json("items[3]: a,b", 2, false).is_ok());
    }

    /// The MCP snippet names the server, starts `serve` and can pin the repository.
    #[test]
    fn mcp_snippet_is_well_formed() {
        let args = McpConfigArgs {
            name: "pn-ultramemory".into(),
            command: None,
        };
        let snippet = mcp_config(&args, Some("/work/app")).expect("snippet");
        let value: serde_json::Value = serde_json::from_str(&snippet).expect("json");
        let server = &value["mcpServers"]["pn-ultramemory"];
        assert_eq!(server["command"], "pn-ultramemory");
        assert_eq!(
            server["args"],
            serde_json::json!(["--repo", "/work/app", "serve"])
        );
    }
}
