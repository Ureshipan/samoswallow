use rand::Rng;
use serde::Serialize;
use sqlx::SqlitePool;

/// Generate a random alphanumeric token of the given length.
pub fn random_token(len: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| rng.sample(rand::distributions::Alphanumeric) as char)
        .collect()
}

/// An App: a git repo plus its manifest. A template instances are created from.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct App {
    pub id: i64,
    pub owner_id: i64,
    pub name: String,
    pub repo_url: String,
    pub default_branch: String,
    pub domain: String,
    pub manifest: Option<String>,
    /// Secret used to verify incoming push webhooks (HMAC-SHA256).
    pub webhook_secret: Option<String>,
    pub created_at: String,
}

impl App {
    pub async fn list(db: &SqlitePool, owner_id: i64) -> sqlx::Result<Vec<App>> {
        sqlx::query_as::<_, App>(
            "SELECT id, owner_id, name, repo_url, default_branch, domain, manifest, \
             webhook_secret, created_at \
             FROM apps WHERE owner_id = ? ORDER BY created_at DESC, id DESC",
        )
        .bind(owner_id)
        .fetch_all(db)
        .await
    }

    pub async fn get(db: &SqlitePool, id: i64) -> sqlx::Result<App> {
        sqlx::query_as::<_, App>(
            "SELECT id, owner_id, name, repo_url, default_branch, domain, manifest, \
             webhook_secret, created_at \
             FROM apps WHERE id = ?",
        )
        .bind(id)
        .fetch_one(db)
        .await
    }

    pub async fn create(
        db: &SqlitePool,
        owner_id: i64,
        name: &str,
        repo_url: &str,
        default_branch: &str,
        domain: &str,
    ) -> sqlx::Result<App> {
        let secret = random_token(40);
        let id = sqlx::query(
            "INSERT INTO apps (owner_id, name, repo_url, default_branch, domain, webhook_secret) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(owner_id)
        .bind(name)
        .bind(repo_url)
        .bind(default_branch)
        .bind(domain)
        .bind(&secret)
        .execute(db)
        .await?
        .last_insert_rowid();

        App::get(db, id).await
    }

    pub async fn delete(db: &SqlitePool, id: i64) -> sqlx::Result<bool> {
        let affected = sqlx::query("DELETE FROM apps WHERE id = ?")
            .bind(id)
            .execute(db)
            .await?
            .rows_affected();
        Ok(affected > 0)
    }
}

/// A Build: an immutable image produced from a specific commit.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Build {
    pub id: i64,
    pub app_id: i64,
    pub commit_sha: String,
    pub image_tag: Option<String>,
    pub status: String,
    pub logs: Option<String>,
    pub created_at: String,
    pub finished_at: Option<String>,
}

impl Build {
    pub async fn create(db: &SqlitePool, app_id: i64, commit_sha: &str) -> sqlx::Result<i64> {
        Ok(sqlx::query(
            "INSERT INTO builds (app_id, commit_sha, status) VALUES (?, ?, 'building')",
        )
        .bind(app_id)
        .bind(commit_sha)
        .execute(db)
        .await?
        .last_insert_rowid())
    }

    pub async fn mark_success(
        db: &SqlitePool,
        id: i64,
        image_tag: &str,
        logs: &str,
    ) -> sqlx::Result<()> {
        sqlx::query(
            "UPDATE builds SET status='success', image_tag=?, logs=?, finished_at=datetime('now') \
             WHERE id=?",
        )
        .bind(image_tag)
        .bind(logs)
        .bind(id)
        .execute(db)
        .await?;
        Ok(())
    }

    pub async fn mark_failed(db: &SqlitePool, id: i64, logs: &str) -> sqlx::Result<()> {
        sqlx::query(
            "UPDATE builds SET status='failed', logs=?, finished_at=datetime('now') WHERE id=?",
        )
        .bind(logs)
        .bind(id)
        .execute(db)
        .await?;
        Ok(())
    }

    pub async fn list(db: &SqlitePool, app_id: i64) -> sqlx::Result<Vec<Build>> {
        sqlx::query_as::<_, Build>(
            "SELECT id, app_id, commit_sha, image_tag, status, logs, created_at, finished_at \
             FROM builds WHERE app_id=? ORDER BY created_at DESC, id DESC",
        )
        .bind(app_id)
        .fetch_all(db)
        .await
    }

    pub async fn get(db: &SqlitePool, id: i64) -> sqlx::Result<Build> {
        sqlx::query_as::<_, Build>(
            "SELECT id, app_id, commit_sha, image_tag, status, logs, created_at, finished_at \
             FROM builds WHERE id=?",
        )
        .bind(id)
        .fetch_one(db)
        .await
    }
}

/// An Instance: a running container started from a Build.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Instance {
    pub id: i64,
    pub app_id: i64,
    pub build_id: i64,
    pub container_id: Option<String>,
    pub host_port: Option<i64>,
    pub status: String,
    pub created_at: String,
}

impl Instance {
    pub async fn create(
        db: &SqlitePool,
        app_id: i64,
        build_id: i64,
        container_id: &str,
        host_port: i64,
    ) -> sqlx::Result<i64> {
        Ok(sqlx::query(
            "INSERT INTO instances (app_id, build_id, container_id, host_port, status) \
             VALUES (?, ?, ?, ?, 'running')",
        )
        .bind(app_id)
        .bind(build_id)
        .bind(container_id)
        .bind(host_port)
        .execute(db)
        .await?
        .last_insert_rowid())
    }

    pub async fn list_for_app(db: &SqlitePool, app_id: i64) -> sqlx::Result<Vec<Instance>> {
        sqlx::query_as::<_, Instance>(
            "SELECT id, app_id, build_id, container_id, host_port, status, created_at \
             FROM instances WHERE app_id=? ORDER BY created_at DESC, id DESC",
        )
        .bind(app_id)
        .fetch_all(db)
        .await
    }

    pub async fn list_running_for_app(db: &SqlitePool, app_id: i64) -> sqlx::Result<Vec<Instance>> {
        sqlx::query_as::<_, Instance>(
            "SELECT id, app_id, build_id, container_id, host_port, status, created_at \
             FROM instances WHERE app_id=? AND status='running' ORDER BY created_at DESC, id DESC",
        )
        .bind(app_id)
        .fetch_all(db)
        .await
    }

    pub async fn get(db: &SqlitePool, id: i64) -> sqlx::Result<Instance> {
        sqlx::query_as::<_, Instance>(
            "SELECT id, app_id, build_id, container_id, host_port, status, created_at \
             FROM instances WHERE id=?",
        )
        .bind(id)
        .fetch_one(db)
        .await
    }

    pub async fn set_status(db: &SqlitePool, id: i64, status: &str) -> sqlx::Result<()> {
        sqlx::query("UPDATE instances SET status=? WHERE id=?")
            .bind(status)
            .bind(id)
            .execute(db)
            .await?;
        Ok(())
    }
}

/// Set the cached manifest YAML on an app.
pub async fn set_app_manifest(db: &SqlitePool, app_id: i64, manifest: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE apps SET manifest=? WHERE id=?")
        .bind(manifest)
        .bind(app_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Fetch a user's stored password hash by id.
pub async fn user_password_hash(db: &SqlitePool, user_id: i64) -> sqlx::Result<String> {
    sqlx::query_scalar::<_, String>("SELECT password_hash FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_one(db)
        .await
}

/// Replace a user's password hash.
pub async fn set_user_password(db: &SqlitePool, user_id: i64, hash: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Ensure a single default user exists (single-user mode) and return its id.
///
/// The password hash is a placeholder until auth lands; this just guarantees an
/// `owner_id` exists for apps to hang off of.
pub async fn ensure_default_user(db: &SqlitePool) -> sqlx::Result<i64> {
    if let Some(row) = sqlx::query_scalar::<_, i64>("SELECT id FROM users ORDER BY id LIMIT 1")
        .fetch_optional(db)
        .await?
    {
        return Ok(row);
    }

    let id = sqlx::query("INSERT INTO users (username, password_hash) VALUES (?, ?)")
        .bind("admin")
        .bind("!unset")
        .execute(db)
        .await?
        .last_insert_rowid();
    Ok(id)
}
