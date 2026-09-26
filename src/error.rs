use std::{fmt, path::PathBuf};

/// Stable failure categories for deciding whether to retry, change limits or reject input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    Io,
    Database,
    CorruptData,
    Unsupported,
    ResourceLimit,
}

/// A location attached to a failure. Contexts retain their enclosing source error.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ErrorContext {
    pub path: Option<PathBuf>,
    pub frame_id: Option<u64>,
    pub table: Option<String>,
    pub field: Option<String>,
    pub byte_offset: Option<u64>,
}
impl fmt::Display for ErrorContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(path) = &self.path {
            parts.push(path.display().to_string());
        }
        if let Some(frame) = self.frame_id {
            parts.push(format!("frame {frame}"));
        }
        if let Some(table) = &self.table {
            parts.push(format!("table {table}"));
        }
        if let Some(field) = &self.field {
            parts.push(format!("field {field}"));
        }
        if let Some(offset) = self.byte_offset {
            parts.push(format!("byte offset {offset}"));
        }
        write!(f, "{}", parts.join(", "))
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("database: {0}")]
    Database(String),
    #[error("database: {0}")]
    DatabaseSource(#[source] Box<turso_core::LimboError>),
    #[error(transparent)]
    Metadata(#[from] serde_json::Error),
    #[error(transparent)]
    ParseInteger(#[from] std::num::ParseIntError),
    #[error("invalid TDF data: {0}")]
    Invalid(String),
    #[error("unsupported TDF feature: {0}")]
    Unsupported(String),
    #[error("resource limit {resource} exceeded: observed {observed}, maximum {maximum}")]
    ResourceLimit {
        resource: &'static str,
        maximum: u64,
        observed: u64,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
    #[error("invalid LZF data: {0}")]
    Lzf(lzf::LzfError),
    #[error("invalid {format} data: {source}")]
    Decoder {
        format: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("{context}: {source}")]
    Context {
        context: Box<ErrorContext>,
        #[source]
        source: Box<Error>,
    },
}
impl Error {
    pub fn kind(&self) -> ErrorKind {
        match self.root_error() {
            Self::Io(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                ErrorKind::CorruptData
            }
            Self::Io(_) => ErrorKind::Io,
            Self::DatabaseSource(error)
                if matches!(
                    error.as_ref(),
                    turso_core::LimboError::Corrupt(_) | turso_core::LimboError::NotADB
                ) =>
            {
                ErrorKind::CorruptData
            }
            Self::DatabaseSource(error)
                if matches!(
                    error.as_ref(),
                    turso_core::LimboError::UnsupportedEncoding(_)
                ) =>
            {
                ErrorKind::Unsupported
            }
            Self::DatabaseSource(error)
                if matches!(
                    error.as_ref(),
                    turso_core::LimboError::TooBig | turso_core::LimboError::OutOfMemory
                ) =>
            {
                ErrorKind::ResourceLimit
            }
            Self::Database(_) | Self::DatabaseSource(_) => ErrorKind::Database,
            Self::Metadata(_)
            | Self::ParseInteger(_)
            | Self::Invalid(_)
            | Self::Lzf(_)
            | Self::Decoder { .. } => ErrorKind::CorruptData,
            Self::Unsupported(_) => ErrorKind::Unsupported,
            Self::ResourceLimit { .. } => ErrorKind::ResourceLimit,
            Self::Context { .. } => unreachable!("root_error removes contextual wrappers"),
        }
    }

    pub fn with_context(self, context: ErrorContext) -> Self {
        Self::Context {
            context: Box::new(context),
            source: Box::new(self),
        }
    }

    pub fn root_error(&self) -> &Self {
        match self {
            Self::Context { source, .. } => source.root_error(),
            other => other,
        }
    }

    pub fn contexts(&self) -> impl Iterator<Item = &ErrorContext> {
        std::iter::successors(Some(self), |error| match error {
            Self::Context { source, .. } => Some(source.as_ref()),
            _ => None,
        })
        .filter_map(|error| match error {
            Self::Context { context, .. } => Some(context.as_ref()),
            _ => None,
        })
    }
}

pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
pub(crate) fn limit(resource: &'static str, maximum: usize, observed: usize) -> Error {
    Error::ResourceLimit {
        resource,
        maximum: maximum as u64,
        observed: observed as u64,
        source: None,
    }
}
pub(crate) fn field(error: Error, table: &str, name: &str) -> Error {
    error.with_context(ErrorContext {
        table: Some(table.into()),
        field: Some(name.into()),
        ..Default::default()
    })
}
