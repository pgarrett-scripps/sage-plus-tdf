use std::{fmt, path::PathBuf};

/// Stable failure categories for deciding whether to retry, change limits or reject input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The file could not be opened, read or positioned.
    Io,
    /// `analysis.tdf` or `analysis.tsf` could not be opened or queried as a
    /// read-only SQLite database.
    Database,
    /// The bytes or the metadata contradict the format, so nothing can be
    /// decoded from them. Retrying or raising a limit will not help.
    CorruptData,
    /// The file is readable but asks for something this reader does not
    /// implement, such as a calibration `ModelType` it has no equation for.
    Unsupported,
    /// A [`crate::Limits`] ceiling would have been exceeded. The allocation was
    /// never made; raising that ceiling is what changes the outcome.
    ResourceLimit,
}

/// A location attached to a failure. Contexts retain their enclosing source error.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ErrorContext {
    /// File the failure is in, not the `.d` directory.
    pub path: Option<PathBuf>,
    /// `Frames.Id` of the frame being read.
    pub frame_id: Option<u64>,
    /// SQLite table being read, or which row of it, when the fault is in one
    /// row rather than the table.
    pub table: Option<String>,
    /// Column or binary header field whose value is at fault.
    pub field: Option<String>,
    /// Offset in the binary file where the blob being decoded starts.
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

/// Everything that can go wrong while reading an acquisition. Match on
/// [`Error::kind`] rather than on a variant: the enum is `#[non_exhaustive]`
/// and several variants share one [`ErrorKind`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The file could not be opened, read or positioned.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The metadata database is unusable for a reason it reported as text: a
    /// missing table, an unexpected column type, a failed query.
    #[error("database: {0}")]
    Database(String),
    /// The SQLite engine failed. A corrupt or non-database file reports
    /// [`ErrorKind::CorruptData`], anything else [`ErrorKind::Database`].
    #[error("database: {0}")]
    DatabaseSource(#[source] Box<turso_core::LimboError>),
    /// A metadata value that must be an integer is not one.
    #[error(transparent)]
    ParseInteger(#[from] std::num::ParseIntError),
    /// The bytes or the metadata contradict the format.
    #[error("invalid TDF data: {0}")]
    Invalid(String),
    /// The file asks for something this reader does not implement.
    #[error("unsupported TDF feature: {0}")]
    Unsupported(String),
    /// A [`crate::Limits`] ceiling would have been exceeded, so the allocation
    /// was never made.
    #[error("resource limit {resource} exceeded: observed {observed}, maximum {maximum}")]
    ResourceLimit {
        /// Which ceiling, by the field name in [`crate::Limits`] or
        /// [`crate::MetadataLimits`].
        resource: &'static str,
        /// The ceiling in force, in that field's own unit.
        maximum: u64,
        /// What the file asked for, in the same unit.
        observed: u64,
        /// The decoder error that reported the excess, when one did.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
    /// An LZF frame could not be decompressed.
    #[error("invalid LZF data: {0}")]
    Lzf(lzf::LzfError),
    /// A decoder rejected its input.
    #[error("invalid {format} data: {source}")]
    Decoder {
        /// The format whose decoder failed, such as `Zstandard` or `Parquet`.
        format: &'static str,
        /// What that decoder reported.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Another error with the file, frame, table, field or byte offset it
    /// happened at. [`Error::kind`] looks through this to the enclosed error.
    #[error("{context}: {source}")]
    Context {
        /// Where the enclosed failure happened.
        context: Box<ErrorContext>,
        /// The failure itself.
        #[source]
        source: Box<Error>,
    },
}
impl Error {
    /// The failure category to act on, taken from the innermost error: a
    /// contextual wrapper reports the kind of what it wraps.
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
            Self::ParseInteger(_) | Self::Invalid(_) | Self::Lzf(_) | Self::Decoder { .. } => {
                ErrorKind::CorruptData
            }
            Self::Unsupported(_) => ErrorKind::Unsupported,
            Self::ResourceLimit { .. } => ErrorKind::ResourceLimit,
            Self::Context { .. } => unreachable!("root_error removes contextual wrappers"),
        }
    }

    /// Wrap this error in the file, frame, table, field or byte offset it
    /// happened at. Wrappers nest, outermost first.
    pub fn with_context(self, context: ErrorContext) -> Self {
        Self::Context {
            context: Box::new(context),
            source: Box::new(self),
        }
    }

    /// The error itself, with every contextual wrapper removed.
    pub fn root_error(&self) -> &Self {
        match self {
            Self::Context { source, .. } => source.root_error(),
            other => other,
        }
    }

    /// Every context attached to this error, outermost first.
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
