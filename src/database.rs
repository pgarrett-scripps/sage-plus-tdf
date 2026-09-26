use crate::{Error, Result, invalid};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use turso_core::{
    Database, IO, Numeric, OpenFlags, OpenOptions, PlatformIO, SqliteDialect, StepResult, Value,
};

/// Bounds for materialized metadata and query execution. Engine preparation and
/// individual SQLite values may allocate before the output checks run.
#[derive(Clone, Copy, Debug)]
pub struct MetadataLimits {
    pub max_rows: usize,
    pub max_columns: usize,
    pub max_value_bytes: usize,
    pub max_materialized_bytes: usize,
    pub max_vm_operations: u64,
    pub query_timeout: Duration,
}
impl Default for MetadataLimits {
    fn default() -> Self {
        Self {
            max_rows: 2_000_000,
            max_columns: 256,
            max_value_bytes: 16 * 1024 * 1024,
            max_materialized_bytes: 512 * 1024 * 1024,
            max_vm_operations: 100_000_000,
            query_timeout: Duration::from_secs(30),
        }
    }
}

pub type MetadataRow = serde_json::Map<String, serde_json::Value>;
pub struct MetadataDatabase {
    connection: Arc<turso_core::Connection>,
    io: Arc<dyn IO>,
    limits: MetadataLimits,
    query_lock: Mutex<()>,
}
fn db_error(e: impl std::fmt::Display) -> Error {
    Error::Database(e.to_string())
}

impl MetadataDatabase {
    /// Open an existing SQLite file read-only. This never creates a missing acquisition.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::with_limits(path, MetadataLimits::default())
    }

    pub fn with_limits(path: impl AsRef<Path>, limits: MetadataLimits) -> Result<Self> {
        let path = path.as_ref();
        if !path.is_file() {
            return Err(invalid("metadata database is not a regular file"));
        }
        // Completed acquisitions only. Reading a live WAL can yield inconsistent frame metadata.
        for suffix in ["-wal", "-journal"] {
            let mut sidecar = path.as_os_str().to_os_string();
            sidecar.push(suffix);
            if std::fs::metadata(&sidecar).is_ok_and(|m| m.len() > 0) {
                return Err(Error::Unsupported(
                    "acquisition has an active SQLite journal".into(),
                ));
            }
        }
        let io: Arc<dyn IO> = Arc::new(PlatformIO::new().map_err(db_error)?);
        let db = Database::open(
            io.clone(),
            path.to_str()
                .ok_or_else(|| invalid("non-UTF8 database path"))?,
            OpenOptions::new(Arc::new(SqliteDialect)).flags(OpenFlags::ReadOnly),
        )
        .map_err(db_error)?;
        Ok(Self {
            connection: db.connect().map_err(db_error)?,
            io,
            limits,
            query_lock: Mutex::new(()),
        })
    }

    pub fn tables(&self) -> Result<Vec<String>> {
        self.query("SELECT name FROM sqlite_master WHERE type IN ('table', 'view') ORDER BY name")?
            .into_iter()
            .map(|r| {
                r.get("name")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("invalid schema object name"))
            })
            .collect()
    }

    /// Preserve all source columns, including vendor extensions and metadata views.
    pub fn table(&self, name: &str) -> Result<Vec<MetadataRow>> {
        let actual = self
            .tables()?
            .into_iter()
            .find(|s| s.eq_ignore_ascii_case(name))
            .ok_or_else(|| invalid(format!("missing metadata table or view {name}")))?;
        self.query(&format!(
            "SELECT * FROM \"{}\"",
            actual.replace('"', "\"\"")
        ))
    }

    fn query(&self, sql: &str) -> Result<Vec<MetadataRow>> {
        // The progress handler belongs to the connection, so query budgets must not overlap.
        let _guard = self
            .query_lock
            .lock()
            .map_err(|_| invalid("metadata query lock poisoned"))?;
        let started = Instant::now();
        let limits = self.limits;
        let operations = AtomicU64::new(0);
        self.connection.set_progress_handler(
            1_000,
            Some(Box::new(move || {
                operations.fetch_add(1_000, Ordering::Relaxed) >= limits.max_vm_operations
                    || started.elapsed() >= limits.query_timeout
            })),
        );
        let mut statement = self.connection.prepare(sql).map_err(db_error)?;
        if statement.num_columns() > limits.max_columns {
            return Err(invalid("metadata column limit exceeded"));
        }
        let columns: Vec<_> = (0..statement.num_columns())
            .map(|i| statement.get_column_name(i).into_owned())
            .collect();
        let mut rows = Vec::new();
        let mut materialized = 0usize;
        loop {
            if started.elapsed() >= limits.query_timeout {
                return Err(invalid("metadata query timeout exceeded"));
            }
            match statement.step().map_err(db_error)? {
                StepResult::Done => return Ok(rows),
                StepResult::IO | StepResult::Yield => self.io.step().map_err(db_error)?,
                StepResult::Row => {
                    if rows.len() >= limits.max_rows {
                        return Err(invalid("metadata row limit exceeded"));
                    }
                    let row = statement
                        .row()
                        .ok_or_else(|| invalid("missing SQLite row"))?;
                    let mut values = MetadataRow::new();
                    for (i, name) in columns.iter().enumerate() {
                        let source = row.get_value(i);
                        let (source_bytes, output_bytes) = match source {
                            Value::Text(s) => (s.as_str().len(), s.as_str().len()),
                            Value::Blob(b) => (
                                b.len(),
                                b.len()
                                    .saturating_mul(std::mem::size_of::<serde_json::Value>()),
                            ),
                            _ => (8, 8),
                        };
                        materialized = materialized
                            .saturating_add(output_bytes)
                            .saturating_add(name.len())
                            .saturating_add(128);
                        if source_bytes > limits.max_value_bytes
                            || materialized > limits.max_materialized_bytes
                        {
                            return Err(invalid(
                                "metadata value or materialization limit exceeded",
                            ));
                        }
                        let value = match source {
                            Value::Null => serde_json::Value::Null,
                            Value::Numeric(Numeric::Integer(n)) => (*n).into(),
                            Value::Numeric(Numeric::Float(n)) => {
                                serde_json::Number::from_f64(f64::from(*n))
                                    .map(serde_json::Value::Number)
                                    .ok_or_else(|| invalid("nonfinite SQLite number"))?
                            }
                            Value::Text(s) => s.as_str().into(),
                            Value::Blob(b) => {
                                serde_json::Value::Array(b.iter().map(|x| (*x).into()).collect())
                            }
                        };
                        if values.insert(name.clone(), value).is_some() {
                            return Err(invalid("duplicate metadata column name"));
                        }
                    }
                    rows.push(values);
                }
                StepResult::Busy | StepResult::Sleep { .. } => {
                    return Err(Error::Database("database is busy".into()));
                }
                StepResult::Interrupt => return Err(Error::Database("query interrupted".into())),
            }
        }
    }
}
