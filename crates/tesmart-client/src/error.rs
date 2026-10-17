//! What can go wrong, split by who can do something about it.
//!
//! [`ClientError`] is about the conversation: the link broke, the switch said nothing, the
//! command does not exist in this protocol. The operation layer wraps it in
//! [`OpError`](crate::OpError), which adds what was being attempted and whether the result could
//! be trusted.

use core::time::Duration;

use tesmart_proto::engine::{ExchangeError, SubmitError};

use tesmart_proto::{Port, PortCount, RequestKind, ResponseKind, Support};

use crate::transport::TransportError;

/// A failed exchange.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClientError {
    /// The link failed.
    #[error(transparent)]
    Transport(#[from] TransportError),

    /// The request never left: the protocol has no such command, or the engine was busy.
    #[error(transparent)]
    Rejected(#[from] SubmitError),

    /// The switch was asked and said nothing usable, after every retry.
    ///
    /// On this hardware that is not proof of failure: read replies are known to go missing, and
    /// several models answer nothing at all.
    #[error("the switch did not reply to {} after {attempts} attempt(s)", describe(*.kind))]
    NoReply {
        /// What was asked, when it was a typed request.
        kind: Option<RequestKind>,
        /// How many times it was sent.
        attempts: u8,
    },

    /// The whole exchange ran out of time, including connecting and waiting for the command gap.
    #[error("the operation did not finish within {after:?}")]
    Timeout {
        /// The budget that was exceeded.
        after: Duration,
    },

    /// The exchange was abandoned, usually because the session was reset underneath it.
    #[error("the exchange was abandoned")]
    Aborted,

    /// The engine failed the exchange for a reason this crate does not interpret.
    #[error(transparent)]
    Exchange(ExchangeError),
}

impl ClientError {
    /// Whether retrying could plausibly help.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Transport(_)
            | Self::NoReply { .. }
            | Self::Timeout { .. }
            | Self::Exchange(_) => true,
            Self::Rejected(_) | Self::Aborted => false,
        }
    }
}

fn describe(kind: Option<RequestKind>) -> &'static str {
    match kind {
        Some(kind) => kind.name(),
        None => "the raw command",
    }
}

/// A failed operation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OpError {
    /// The exchange underneath failed.
    #[error(transparent)]
    Client(#[from] ClientError),

    /// A port number that does not exist on this switch.
    #[error("port {port} does not exist on a {ports}-port switch")]
    PortOutOfRange {
        /// The port that was asked for.
        port: Port,
        /// How many the switch has.
        ports: PortCount,
    },

    /// An output number that does not exist on this matrix.
    #[error("output {output} does not exist on a matrix with {outputs} output(s)")]
    OutputOutOfRange {
        /// The output that was asked for.
        output: Port,
        /// How many the matrix has.
        outputs: PortCount,
    },

    /// The command does not exist on this model, or is known not to work.
    #[error("{kind} is {support} on {}", model.unwrap_or("this switch"))]
    Unsupported {
        /// The command.
        kind: RequestKind,
        /// What the registry says about it.
        support: Support,
        /// The model, when it came from the registry.
        model: Option<&'static str>,
    },

    /// The switch was asked to change input and reports a different one.
    ///
    /// The one case where failing loudly is right: the command was accepted, the switch
    /// answered, and the answer is wrong.
    #[error("asked for port {expected} but the switch reports port {found}")]
    VerificationFailed {
        /// The port that was requested.
        expected: Port,
        /// The port the switch reports.
        found: Port,
    },

    /// The switch answered something that does not belong to the question.
    ///
    /// Only reachable if the switch is not what it claims to be, such as a matrix answering a
    /// KVM command.
    #[error("the switch answered with {kind}, which does not answer {asked}")]
    UnexpectedResponse {
        /// What came back.
        kind: ResponseKind,
        /// What was asked.
        asked: RequestKind,
    },
}

impl OpError {
    /// A stable classification, for exit codes and for deciding what to retry.
    pub fn class(&self) -> ErrorClass {
        match self {
            Self::Client(ClientError::Transport(_)) => ErrorClass::Transport,
            Self::Client(ClientError::Timeout { .. } | ClientError::NoReply { .. }) => {
                ErrorClass::Timeout
            }
            Self::Client(ClientError::Rejected(_)) | Self::Unsupported { .. } => {
                ErrorClass::Unsupported
            }
            Self::Client(ClientError::Aborted | ClientError::Exchange(_))
            | Self::UnexpectedResponse { .. } => ErrorClass::Protocol,
            Self::PortOutOfRange { .. } | Self::OutputOutOfRange { .. } => ErrorClass::Usage,
            Self::VerificationFailed { .. } => ErrorClass::Verification,
        }
    }
}

/// What kind of failure an [`OpError`] is, independent of its details.
///
/// Exists so a command line tool can map failures to exit codes with one `match` that will not
/// drift as variants are added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorClass {
    /// The request was impossible: a port that does not exist, a command for another protocol.
    Usage,
    /// The link failed.
    Transport,
    /// Nothing came back in time.
    Timeout,
    /// This switch cannot do that.
    Unsupported,
    /// The switch did something, and it was not what was asked.
    Verification,
    /// The switch said something that makes no sense.
    Protocol,
}

impl ErrorClass {
    /// A stable, hyphenated name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::Transport => "transport",
            Self::Timeout => "timeout",
            Self::Unsupported => "unsupported",
            Self::Verification => "verification",
            Self::Protocol => "protocol",
        }
    }
}

impl core::fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_the_request() {
        let typed = ClientError::NoReply { kind: Some(RequestKind::ReadActivePort), attempts: 6 };
        assert_eq!(
            typed.to_string(),
            "the switch did not reply to read-active-port after 6 attempt(s)"
        );

        let raw = ClientError::NoReply { kind: None, attempts: 1 };
        assert_eq!(
            raw.to_string(),
            "the switch did not reply to the raw command after 1 attempt(s)"
        );
    }

    #[test]
    fn unsupported_commands_are_not_transient() {
        assert!(!ClientError::Aborted.is_transient());
        assert!(ClientError::Timeout { after: Duration::from_secs(1) }.is_transient());
    }
}
