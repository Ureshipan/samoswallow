use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

/// Open the SQLite pool and apply embedded migrations.
///
/// Migrations live in `crates/swallowd/migrations` and are baked into the binary
/// at compile time, so a fresh server needs no separate migration step.
pub async fn connect(database_url: &str) -> anyhow::Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}
