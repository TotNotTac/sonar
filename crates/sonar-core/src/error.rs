use std::io;
use std::path::{Path, PathBuf};

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("The folder \"{}\" does not exist", .0.display())]
    NotFound(PathBuf),

    #[error("You do not have permission to open \"{}\"", .0.display())]
    PermissionDenied(PathBuf),

    #[error("\"{}\" is not a folder", .0.display())]
    NotADirectory(PathBuf),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("{0}")]
    InvalidName(String),

    #[error("\"{}\" already exists", display_name(.0))]
    AlreadyExists(PathBuf),

    /// A failure that already carries a complete, user-facing message.
    #[error("{0}")]
    Other(String),

    /// An encrypted archive needs a (different) password: see
    /// `archive::set_password`. `archive` is the path the user sees.
    #[error("{}", password_message(archive, *wrong))]
    PasswordRequired { archive: PathBuf, wrong: bool },

    #[error("Could not open \"{}\": {message}", path.display())]
    Open { path: PathBuf, message: String },

    #[error("Could not watch \"{}\": {message}", path.display())]
    Watch { path: PathBuf, message: String },

    #[error("Could not read \"{}\": {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl CoreError {
    pub fn from_io(path: &Path, err: io::Error) -> Self {
        let path = path.to_path_buf();
        match err.kind() {
            io::ErrorKind::NotFound => Self::NotFound(path),
            io::ErrorKind::PermissionDenied => Self::PermissionDenied(path),
            io::ErrorKind::NotADirectory => Self::NotADirectory(path),
            io::ErrorKind::AlreadyExists => Self::AlreadyExists(path),
            _ => Self::Io { path, source: err },
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "notFound",
            Self::PermissionDenied(_) => "permissionDenied",
            Self::NotADirectory(_) => "notADirectory",
            Self::InvalidPath(_) => "invalidPath",
            Self::InvalidName(_) => "invalidName",
            Self::AlreadyExists(_) => "alreadyExists",
            Self::Other(_) => "other",
            Self::PasswordRequired { .. } => "passwordRequired",
            Self::Open { .. } => "open",
            Self::Watch { .. } => "watch",
            Self::Io { .. } => "io",
        }
    }
}

fn password_message(archive: &Path, wrong: bool) -> String {
    if wrong {
        format!("Wrong password for \"{}\"", display_name(archive))
    } else {
        format!("\"{}\" is password protected", display_name(archive))
    }
}

fn display_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// Serialized as `{ code, message }` so the frontend can branch on `code`
/// and show `message` directly.
impl Serialize for CoreError {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("CoreError", 2)?;
        st.serialize_field("code", self.code())?;
        st.serialize_field("message", &self.to_string())?;
        if let Self::PasswordRequired { archive, wrong } = self {
            st.serialize_field("path", &crate::uipath::encode(archive.as_os_str()))?;
            st.serialize_field("wrong", wrong)?;
        }
        st.end()
    }
}
