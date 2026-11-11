-- Only a digest is stored; a separately operated, short-lived invitation is
-- required even when the person has already authenticated with Mikaki.
CREATE TABLE sample_admin_invite (token_hash TEXT PRIMARY KEY, expires_at INTEGER NOT NULL, created_at INTEGER NOT NULL);
CREATE INDEX sample_admin_invite_expiry ON sample_admin_invite(expires_at);
