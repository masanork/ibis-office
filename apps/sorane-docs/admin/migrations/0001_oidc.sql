CREATE TABLE login_transaction (
  state_hash TEXT PRIMARY KEY,
  browser_hash TEXT NOT NULL,
  nonce TEXT NOT NULL,
  verifier TEXT NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE TABLE rp_session (
  token_hash TEXT PRIMARY KEY,
  sid TEXT NOT NULL,
  sub TEXT NOT NULL,
  auth_time INTEGER NOT NULL,
  lease_until INTEGER NOT NULL,
  parent_expires_at INTEGER NOT NULL,
  idle_expires_at INTEGER NOT NULL,
  idle_timeout_seconds INTEGER NOT NULL
);
CREATE INDEX rp_session_sid ON rp_session(sid);
CREATE INDEX rp_session_idle ON rp_session(idle_expires_at);
CREATE INDEX rp_session_parent ON rp_session(parent_expires_at);
CREATE TABLE logout_tombstone (
  sid TEXT PRIMARY KEY,
  expires_at INTEGER NOT NULL
);
CREATE INDEX logout_tombstone_expiry ON logout_tombstone(expires_at);
CREATE INDEX login_transaction_expiry ON login_transaction(expires_at);
