// SPDX-License-Identifier: Apache-2.0
//! The error type of every engine operation.

use core::fmt;
use std::error::Error;

use pn_ultramemory_core::{DocError, ExtractError, SourceError, StorageError};

/// Why an engine operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    /// Something the caller named does not exist, with a helpful description that may suggest
    /// alternatives.
    NotFound(String),
    /// A name matches several symbols and none is clearly meant. `candidates` lists them as
    /// `id path qualified_name`, at most eight, so the caller can retry with an id.
    Ambiguous {
        /// What the caller asked for.
        query: String,
        /// The matching symbols.
        candidates: Vec<String>,
    },
    /// The request itself is invalid, for example an empty query or a window that starts after the
    /// end of the symbol.
    Invalid(String),
    /// The request was refused for safety, for example a memory that looks like an attempt to
    /// take over the reader.
    Rejected(String),
    /// The storage adapter failed.
    Storage(StorageError),
    /// Reading or writing a source file failed.
    Source(SourceError),
    /// Inserting documentation failed.
    Doc(DocError),
    /// Extracting symbols failed.
    Extract(ExtractError),
    /// Any other input or output failure, with the message.
    Io(String),
}

impl EngineError {
    /// A stable exit code for command-line use: 3 for not found or ambiguous, 2 for invalid or
    /// rejected requests, and 1 for failures of the environment.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::NotFound(_) | Self::Ambiguous { .. } => 3,
            Self::Invalid(_) | Self::Rejected(_) => 2,
            _ => 1,
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(message) => write!(f, "not found: {message}"),
            Self::Ambiguous { query, candidates } => {
                write!(
                    f,
                    "`{query}` matches several symbols; use an id: {}",
                    candidates.join("; ")
                )
            }
            Self::Invalid(message) => write!(f, "invalid request: {message}"),
            Self::Rejected(message) => write!(f, "refused: {message}"),
            Self::Storage(error) => write!(f, "{error}"),
            Self::Source(error) => write!(f, "{error}"),
            Self::Doc(error) => write!(f, "{error}"),
            Self::Extract(error) => write!(f, "{error}"),
            Self::Io(message) => write!(f, "{message}"),
        }
    }
}

impl Error for EngineError {}

impl From<StorageError> for EngineError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<SourceError> for EngineError {
    fn from(error: SourceError) -> Self {
        Self::Source(error)
    }
}

impl From<DocError> for EngineError {
    fn from(error: DocError) -> Self {
        Self::Doc(error)
    }
}

impl From<ExtractError> for EngineError {
    fn from(error: ExtractError) -> Self {
        Self::Extract(error)
    }
}

#[cfg(test)]
mod tests {
    use super::EngineError;
    use pn_ultramemory_core::StorageError;

    /// Exit codes group errors the way a shell script would want to react to them.
    #[test]
    fn exit_codes_are_grouped() {
        assert_eq!(EngineError::NotFound("x".into()).exit_code(), 3);
        assert_eq!(EngineError::Invalid("x".into()).exit_code(), 2);
        assert_eq!(
            EngineError::from(StorageError::Backend("x".into())).exit_code(),
            1
        );
    }

    /// The ambiguity message tells the caller how to disambiguate.
    #[test]
    fn ambiguity_lists_candidates() {
        let error = EngineError::Ambiguous {
            query: "parse".into(),
            candidates: vec!["1 a.rs parse".into(), "2 b.rs parse".into()],
        };
        assert!(error.to_string().contains("use an id"));
    }
}
