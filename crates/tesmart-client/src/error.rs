//! What can go wrong, split by who can do something about it.
//!
//! [`ClientError`] is about the conversation: the link broke, the switch said nothing, the
//! command does not exist in this family. The operation layer wraps it in
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

    /// The request never left: the profile has no such command, or the engine was busy.
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
