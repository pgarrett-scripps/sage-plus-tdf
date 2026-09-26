use crate::{Error, Result, invalid};
use std::{path::Path, sync::Arc};
use turso_core::{
    Database, IO, Numeric, OpenFlags, OpenOptions, PlatformIO, SqliteDialect, StepResult, Value,
};

pub type MetadataRow = serde_json::Map<String, serde_json::Value>;
pub struct MetadataDatabase {
    connection: Arc<turso_core::Connection>,
    io: Arc<dyn IO>,
}
fn db_error(e: impl std::fmt::Display) -> Error {
    Error::Database(e.to_string())
}

impl MetadataDatabase {
    /// Open an existing SQLite file read-only. This never creates a missing acquisition.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
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
        let mut statement = self.connection.prepare(sql).map_err(db_error)?;
        let columns: Vec<_> = (0..statement.num_columns())
            .map(|i| statement.get_column_name(i).into_owned())
            .collect();
        let mut rows = Vec::new();
        loop {
            match statement.step().map_err(db_error)? {
                StepResult::Done => return Ok(rows),
                StepResult::IO | StepResult::Yield => self.io.step().map_err(db_error)?,
                StepResult::Row => {
                    let row = statement
                        .row()
                        .ok_or_else(|| invalid("missing SQLite row"))?;
                    let mut values = MetadataRow::new();
                    for (i, name) in columns.iter().enumerate() {
                        let value = match row.get_value(i) {
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
                        values.insert(name.clone(), value);
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
