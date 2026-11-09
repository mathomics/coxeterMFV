//! The one error type of this crate.
//!
//! The JavaScript modules throw `Error`s whose message is what the user reads
//! (`eco.json: duplicate reaction id: PGI.`), so the message is kept verbatim
//! and a [`ErrorKind`] is added for code that wants to branch on the cause.

/// What went wrong, broadly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The text is not JSON (`JSON.parse` threw).
    InvalidJson,
    /// The document parsed but is not a usable model.
    InvalidModel,
    /// The text is not an SBML document this reader understands.
    InvalidSbml,
    /// `decodeURIComponent` met a malformed `%` escape (`URIError`).
    UriMalformed,
    /// A model edit was refused (duplicate id, bad stoichiometry…).
    Edit,
    /// An identifier cannot be written as an SBML SId.
    SbmlWrite,
    /// A MATLAB Level 5 file was refused.
    Mat5,
    /// An .xlsx workbook could not be read.
    Xlsx,
    /// A reproducibility bundle was refused.
    Reproducibility,
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
