-- Time-series resource metrics per instance, sampled periodically by the daemon.
-- Retention is enforced in code (keep the last N points per instance) so the
-- table stays small and doesn't grow unbounded.

CREATE TABLE metrics (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    instance_id        INTEGER NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
    ts                 TEXT NOT NULL DEFAULT (datetime('now')),
    cpu_percent        REAL NOT NULL,
    memory_bytes       INTEGER NOT NULL,
    memory_limit_bytes INTEGER NOT NULL
);

CREATE INDEX idx_metrics_instance ON metrics(instance_id, id);
