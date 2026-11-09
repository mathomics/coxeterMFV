//! The one error type of this crate.
//!
//! The JavaScript modules throw `Error`s whose message is what the user reads
//! (`reactions: Bad uses GHOST, which metabolites.txt does not declare`), so
//! the message is kept verbatim and an [`ErrorKind`] says which module spoke.

/// Which part of the crate refused its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// An omics table could not be read (`overlay`, `community_omics`).
    Omics,
    /// GIMME could not be set up.
    Gimme,
    /// The Stolyar 2007 supplement was refused.
    Stolyar,
    /// The four-auxotroph E. coli community could not be built.
    SteadyComEcoli,
    /// The nine-species gut model was refused.
    Gut9,
    /// The Zuñiga 2020 source models or scenarios were refused.
    Zuniga,
    /// A download failed or its bytes were not the pinned ones.
    Download,
    /// A bundled asset of a published community is missing or unreadable.
    Assets,
    /// A model document could not be turned into a model.
    Model,
    /// Reading a file from disk failed.
    Io,
}

/// An error with the message the JavaScript app would show.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Error {
        Error { kind, message: message.into() }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::new(ErrorKind::Io, e.to_string())
    }
}

impl From<coxeter_core::ModelError> for Error {
    fn from(e: coxeter_core::ModelError) -> Error {
        Error::new(ErrorKind::Model, e.to_string())
    }
}

impl From<coxeter_io::Error> for Error {
    fn from(e: coxeter_io::Error) -> Error {
        Error::new(ErrorKind::Assets, e.message)
    }
}
