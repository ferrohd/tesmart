//! What can go wrong, and what the shell should see.

use std::io;
use std::process::ExitCode;

use tesmart::client::{BuildError, ErrorClass, OpError};

/// Anything that stops a command from finishing.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// The arguments did not describe something this tool can do.
    #[error("{0}")]
    Usage(String),

    /// An operation against the switch failed.
    #[error(transparent)]
    Op(#[from] OpError),

    /// The switch could not be described well enough to talk to it.
    #[error(transparent)]
    Build(#[from] BuildError),

    /// A command was sent but its effect could not be confirmed, and `--require-verified`
    /// was given.
    #[error("{0}")]
    Unverified(String),

    /// Writing output, reading a file, or binding a socket failed.
    #[error("{context}")]
    Io {
        /// What was being attempted.
        context: String,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },
}

impl CliError {
    /// A usage error, for arguments that parse but do not make sense.
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    /// An IO failure, with what was being attempted.
    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io { context: context.into(), source }
    }

    /// How this failure is classified, for the exit code and for JSON output.
    pub fn class(&self) -> ErrorClass {
        match self {
            // A model that cannot be resolved is as much a usage problem as a bad flag.
            Self::Usage(_) | Self::Build(_) => ErrorClass::Usage,
            Self::Op(error) => error.class(),
            Self::Unverified(_) => ErrorClass::Verification,
            // An IO failure here is this machine's, not the switch's; "transport" is the
            // closest honest classification.
            Self::Io { .. } => ErrorClass::Transport,
        }
    }

    /// What the shell sees.
    pub fn exit_code(&self) -> ExitCode {
        exit_code_for(self.class())
    }

    /// The whole chain, as one line: `cannot connect to tcp://…: connection refused`.
    pub fn chain(&self) -> String {
        let mut message = self.to_string();
        let mut source = std::error::Error::source(self);
        while let Some(error) = source {
            let text = error.to_string();
            // thiserror's `transparent` repeats the inner message; do not say it twice.
            if !message.ends_with(&text) {
                message.push_str(": ");
                message.push_str(&text);
            }
            source = error.source();
        }
        message
    }
}

/// The documented exit code for a class of failure.
///
/// Kept as one exhaustive match so a new [`ErrorClass`] cannot slip through unmapped.
pub fn exit_code_for(class: ErrorClass) -> ExitCode {
    ExitCode::from(match class {
        ErrorClass::Usage => 2,
        ErrorClass::Transport => 3,
        ErrorClass::Timeout => 4,
        ErrorClass::Unsupported => 5,
        ErrorClass::Verification => 6,
        ErrorClass::Protocol => 7,
        // `ErrorClass` is non-exhaustive: anything new is "something unexpected" until it is
        // given a code of its own.
        _ => 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(class: ErrorClass) -> u8 {
        // `ExitCode` cannot be inspected, so compare against freshly built ones.
        for candidate in 0..=10u8 {
            if format!("{:?}", exit_code_for(class)) == format!("{:?}", ExitCode::from(candidate)) {
                return candidate;
            }
        }
        unreachable!("exit codes are small")
    }

    #[test]
    fn every_class_has_its_documented_code() {
        assert_eq!(code(ErrorClass::Usage), 2);
        assert_eq!(code(ErrorClass::Transport), 3);
        assert_eq!(code(ErrorClass::Timeout), 4);
        assert_eq!(code(ErrorClass::Unsupported), 5);
        assert_eq!(code(ErrorClass::Verification), 6);
        assert_eq!(code(ErrorClass::Protocol), 7);
    }

    #[test]
    fn a_missing_model_is_a_usage_problem() {
        let error = CliError::from(BuildError::NoFamily);
        assert_eq!(error.class(), ErrorClass::Usage);
        assert!(error.chain().contains("no protocol family"), "{}", error.chain());
    }

    #[test]
    fn the_chain_reads_as_one_sentence() {
        let error = CliError::io(
            "cannot write the manual page",
            io::Error::new(io::ErrorKind::PermissionDenied, "permission denied"),
        );
        assert_eq!(error.chain(), "cannot write the manual page: permission denied");
    }
}
