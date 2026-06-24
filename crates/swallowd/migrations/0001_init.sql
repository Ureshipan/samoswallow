-- Initial schema for samoswallow.
-- Single-user today, but the `users` table and `owner_id` columns are present
-- so multi-user can be layered on later without a destructive migration.

PRAGMA foreign_keys = ON;

CREATE TABLE users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

-- An App is a git repo + its swallow.yaml manifest. It is a template, not a
-- running thing.
CREATE TABLE apps (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    repo_url    TEXT NOT NULL,
    default_branch TEXT NOT NULL DEFAULT 'master',
    domain      TEXT NOT NULL,            -- third-level label, served at <domain>.<base_domain>
    manifest    TEXT,                     -- last-seen swallow.yaml, raw
    webhook_secret TEXT,                  -- HMAC secret for push webhooks
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(owner_id, name)
);

-- A Build is an immutable image produced from a specific commit.
CREATE TABLE builds (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    app_id      INTEGER NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    commit_sha  TEXT NOT NULL,
    image_tag   TEXT,                     -- docker image tag once built
    status      TEXT NOT NULL DEFAULT 'pending', -- pending|building|success|failed
    logs        TEXT,                     -- build log output
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at TEXT
);

-- An Instance is a running container started from a Build.
CREATE TABLE instances (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    app_id       INTEGER NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    build_id     INTEGER NOT NULL REFERENCES builds(id) ON DELETE CASCADE,
    container_id TEXT,                    -- docker container id
    host_port    INTEGER,                 -- port assigned on the host for proxying
    status       TEXT NOT NULL DEFAULT 'created', -- created|running|stopped|failed
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

-- A Route maps a subdomain to instance(s) of an app (1:N => load-balanced).
CREATE TABLE routes (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    app_id     INTEGER NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    subdomain  TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Per-app environment variables / secrets. Values are encrypted at rest.
CREATE TABLE secrets (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    app_id     INTEGER NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    key        TEXT NOT NULL,
    value_enc  BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(app_id, key)
);

CREATE INDEX idx_builds_app ON builds(app_id);
CREATE INDEX idx_instances_app ON instances(app_id);
