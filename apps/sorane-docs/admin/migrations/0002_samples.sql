-- Administrators are added explicitly by an operator, after confirming the
-- subject of the intended account. Login alone does not grant privileges.
CREATE TABLE sample_admin (issuer TEXT NOT NULL, sub TEXT NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY (issuer, sub));
CREATE TABLE samples (id TEXT PRIMARY KEY, object_key TEXT NOT NULL UNIQUE, name TEXT NOT NULL, title TEXT NOT NULL, size INTEGER NOT NULL, created_at INTEGER NOT NULL, created_by TEXT NOT NULL);
CREATE INDEX samples_created ON samples(created_at DESC);
