use std::str::FromStr;
use std::time::Duration;

use serde_json::Value;
use sqlx::postgres::{PgArguments, PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::{Arguments, Encode, PgConnection, PgPool, Postgres, Type};

use crate::config::Config;
use crate::error::{AppError, Result};

pub type Db = PgConnection;

pub async fn connect(config: &Config) -> std::result::Result<PgPool, sqlx::Error> {
    let mut options = PgConnectOptions::from_str(&config.database_url)?;
    // DATABASE_SSL=true for managed databases that require TLS with a verified certificate;
    // 'no-verify' when the provider's certificate is not publicly trusted. Unset on a private network.
    match config.database_ssl.as_deref() {
        Some("true") => options = options.ssl_mode(PgSslMode::VerifyFull),
        Some("no-verify") => options = options.ssl_mode(PgSslMode::Require),
        _ => {}
    }
    Ok(PgPoolOptions::new()
        .max_connections(config.database_pool_size)
        .acquire_timeout(Duration::from_secs(10))
        .connect_lazy_with(options))
}

/// A query with its parameters. Rows come back as JSON objects keyed by column name, which keeps
/// the SQL (and its camelCase column aliases) the single description of each API response.
pub struct Q {
    sql: String,
    args: PgArguments,
}

impl Q {
    pub fn new(sql: impl Into<String>) -> Q {
        Q { sql: sql.into(), args: PgArguments::default() }
    }

    pub fn bind<'q, T>(mut self, value: T) -> Q
    where
        T: 'q + Encode<'q, Postgres> + Type<Postgres>,
    {
        self.args.add(value).expect("encode query parameter");
        self
    }

    fn wrapped(&self) -> String {
        format!("WITH t AS ({}) SELECT row_to_json(t) FROM t", self.sql)
    }

    pub async fn all(self, db: &mut Db) -> Result<Vec<Value>> {
        let sql = self.wrapped();
        Ok(sqlx::query_scalar_with::<_, Value, _>(&sql, self.args).fetch_all(db).await?)
    }

    pub async fn opt(self, db: &mut Db) -> Result<Option<Value>> {
        let sql = self.wrapped();
        Ok(sqlx::query_scalar_with::<_, Value, _>(&sql, self.args).fetch_optional(db).await?)
    }

    pub async fn one(self, db: &mut Db) -> Result<Value> {
        self.opt(db).await?.ok_or_else(|| AppError::Internal("query returned no rows".into()))
    }

    /// Runs a statement and returns the number of rows it touched.
    pub async fn run(self, db: &mut Db) -> Result<u64> {
        Ok(sqlx::query_with(&self.sql, self.args).execute(db).await?.rows_affected())
    }

    /// The first column of every row.
    pub async fn scalars<T>(self, db: &mut Db) -> Result<Vec<T>>
    where
        T: for<'r> sqlx::Decode<'r, Postgres> + Type<Postgres> + Send + Unpin,
    {
        Ok(sqlx::query_scalar_with::<_, T, _>(&self.sql, self.args).fetch_all(db).await?)
    }

    /// The first column of the first row.
    pub async fn scalar<T>(self, db: &mut Db) -> Result<T>
    where
        T: for<'r> sqlx::Decode<'r, Postgres> + Type<Postgres> + Send + Unpin,
    {
        Ok(sqlx::query_scalar_with::<_, T, _>(&self.sql, self.args).fetch_one(db).await?)
    }
}

/// Reads a UUID out of a JSON row.
pub fn uuid_of(v: &Value) -> Option<uuid::Uuid> {
    v.as_str().and_then(|s| uuid::Uuid::parse_str(s).ok())
}

const MIGRATIONS: &[(&str, &str)] = &[
    ("001_foundation.sql", include_str!("../migrations/001_foundation.sql")),
    ("002_assignments_relationships.sql", include_str!("../migrations/002_assignments_relationships.sql")),
    ("003_audits.sql", include_str!("../migrations/003_audits.sql")),
    ("004_maintenance.sql", include_str!("../migrations/004_maintenance.sql")),
    ("005_settings_scheduling.sql", include_str!("../migrations/005_settings_scheduling.sql")),
    ("006_workspaces_accounts.sql", include_str!("../migrations/006_workspaces_accounts.sql")),
    ("007_item_registry.sql", include_str!("../migrations/007_item_registry.sql")),
    ("008_plans_warranties_reminders.sql", include_str!("../migrations/008_plans_warranties_reminders.sql")),
    ("009_device_agents.sql", include_str!("../migrations/009_device_agents.sql")),
    ("010_personal_notifications.sql", include_str!("../migrations/010_personal_notifications.sql")),
    ("011_inventories.sql", include_str!("../migrations/011_inventories.sql")),
    ("012_delegated_access.sql", include_str!("../migrations/012_delegated_access.sql")),
];

/// Applies every migration not yet recorded in schema_migrations, in order, each in its own
/// transaction. Safe to run from several servers at once.
pub async fn migrate(pool: &PgPool, log: bool) -> Result<()> {
    let mut conn = pool.acquire().await?;
    sqlx::query("SELECT pg_advisory_lock(727274)").execute(&mut *conn).await?;
    let result = apply_migrations(&mut conn, log).await;
    let _ = sqlx::query("SELECT pg_advisory_unlock(727274)").execute(&mut *conn).await;
    result
}

async fn apply_migrations(conn: &mut Db, log: bool) -> Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS schema_migrations (name text PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())",
    )
    .execute(&mut *conn)
    .await?;
    let applied: Vec<String> = sqlx::query_scalar("SELECT name FROM schema_migrations").fetch_all(&mut *conn).await?;
    for (name, sql) in MIGRATIONS {
        if applied.iter().any(|a| a == name) {
            continue;
        }
        let mut tx = sqlx::Connection::begin(&mut *conn).await?;
        sqlx::raw_sql(sql)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::Internal(format!("Migration {name} failed: {e}")))?;
        sqlx::query("INSERT INTO schema_migrations (name) VALUES ($1)").bind(name).execute(&mut *tx).await?;
        tx.commit().await?;
        if log {
            println!("applied {name}");
        }
    }
    Ok(())
}

/// A parameter whose type is only known at run time.
pub enum Param {
    Text(Option<String>),
    Bool(Option<bool>),
}

impl Q {
    pub fn bind_param(self, p: Param) -> Q {
        match p {
            Param::Text(v) => self.bind(v),
            Param::Bool(v) => self.bind(v),
        }
    }
}

/// Builds the `SET a = $3, b = $4::date` list of an UPDATE whose columns depend on the request.
pub struct Assign {
    next: usize,
    sets: Vec<String>,
    params: Vec<Param>,
}

impl Assign {
    /// `first` is the placeholder number of the first assigned value.
    pub fn new(first: usize) -> Assign {
        Assign { next: first, sets: vec![], params: vec![] }
    }

    /// Assigns a value; `cast` (such as `::uuid`) types a text value for its column.
    pub fn set(&mut self, column: &str, cast: &str, value: Param) {
        self.sets.push(format!("{column} = ${}{cast}", self.next));
        self.params.push(value);
        self.next += 1;
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    pub fn sql(&self) -> String {
        self.sets.join(", ")
    }

    pub fn bind_to(self, mut q: Q) -> Q {
        for p in self.params {
            q = q.bind_param(p);
        }
        q
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_migration_file_is_embedded_in_order() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|f| f.ends_with(".sql"))
            .collect();
        files.sort();
        let embedded: Vec<&str> = super::MIGRATIONS.iter().map(|(n, _)| *n).collect();
        assert_eq!(files, embedded);
    }
}
