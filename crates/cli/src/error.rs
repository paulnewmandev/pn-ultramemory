// SPDX-License-Identifier: Apache-2.0
//! The error type of the command line and how it maps to exit codes.

use core::fmt;
use std::error::Error;

use pn_ultramemory_engine::EngineError;

/// A failure to report to the user, with the exit code the process should end with.
///
/// Exit codes: `1` for failures of the environment (files, database), `2` for a request that is
/// invalid or refused, `3` when something asked for does not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    /// What to tell the user.
    pub message: String,
    /// The process exit code.
    pub code: u8,
}

impl CliError {
    /// A failure of the environment: exit code 1.
    #[must_use]
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 1,
        }
    }

    /// An invalid request: exit code 2.
    #[must_use]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 2,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for CliError {}

impl From<EngineError> for CliError {
    fn from(error: EngineError) -> Self {
        Self {
            code: error.exit_code(),
            message: error.to_string(),
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self::failure(error.to_string())
    }
}

impl From<pn_ultramemory_core::StorageError> for CliError {
    fn from(error: pn_ultramemory_core::StorageError) -> Self {
        Self::failure(error.to_string())
    }
}

impl From<pn_ultramemory_core::SourceError> for CliError {
    fn from(error: pn_ultramemory_core::SourceError) -> Self {
        Self::failure(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::CliError;
    use pn_ultramemory_engine::EngineError;

    /// Engine errors keep their exit code.
    #[test]
    fn engine_errors_keep_their_exit_code() {
        assert_eq!(CliError::from(EngineError::NotFound("x".into())).code, 3);
        assert_eq!(CliError::from(EngineError::Invalid("x".into())).code, 2);
        assert_eq!(CliError::failure("x").code, 1);
    }
}
