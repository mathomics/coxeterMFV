//! The one error type of the store.
//!
//! The JavaScript store throws `Error`s whose message is what the user reads
//! ("Select a species/model.", "Enable at least one species."), and an
//! `AbortError` when a calculation was cancelled or superseded. Both are kept:
//! [`StoreError::Message`] carries the message verbatim and
//! [`StoreError::Aborted`] is the `AbortError`.

use coxeter_core::model::ModelError;
use coxeter_map::editable_map::MapError;
use coxeter_map::map_document::MapDocumentError;
use coxeter_render::theme::ThemeError;
use coxeter_solver::steadycom::SteadyComError;

/// Why a store action failed: a message for the user, or a cancellation.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum StoreError {
    /// `throw new Error(message)`.
    #[error("{0}")]
    Message(String),
    /// `error.name === 'AbortError'`: cancelled, timed out or superseded.
    #[error("{0}")]
    Aborted(String),
}

impl StoreError {
    /// [`StoreError::Message`] with this text.
    pub fn msg(message: impl Into<String>) -> StoreError {
        StoreError::Message(message.into())
    }

    /// [`StoreError::Aborted`] with this text.
    pub fn aborted(message: impl Into<String>) -> StoreError {
        StoreError::Aborted(message.into())
    }

    /// `error.name === 'AbortError'`.
    pub fn is_abort(&self) -> bool {
        matches!(self, StoreError::Aborted(_))
    }
}

/// The result of a store action.
pub type Result<T, E = StoreError> = std::result::Result<T, E>;

impl From<coxeter_io::Error> for StoreError {
    fn from(e: coxeter_io::Error) -> Self {
        StoreError::Message(e.message)
    }
}

impl From<coxeter_analysis::Error> for StoreError {
    fn from(e: coxeter_analysis::Error) -> Self {
        StoreError::Message(e.message)
    }
}

impl From<ModelError> for StoreError {
    fn from(e: ModelError) -> Self {
        StoreError::Message(e.to_string())
    }
}

impl From<SteadyComError> for StoreError {
    fn from(e: SteadyComError) -> Self {
        match e {
            SteadyComError::Cancelled => StoreError::Aborted(e.to_string()),
            other => StoreError::Message(other.to_string()),
        }
    }
}

impl From<MapDocumentError> for StoreError {
    fn from(e: MapDocumentError) -> Self {
        StoreError::Message(e.to_string())
    }
}

impl From<MapError> for StoreError {
    fn from(e: MapError) -> Self {
        StoreError::Message(e.to_string())
    }
}

impl From<ThemeError> for StoreError {
    fn from(e: ThemeError) -> Self {
        StoreError::Message(e.to_string())
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError::Message(e.to_string())
    }
}
