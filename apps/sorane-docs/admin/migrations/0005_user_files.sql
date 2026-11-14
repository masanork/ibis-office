-- Private documents for any authenticated Mikaki user. Ownership is the
-- pairwise subject at this RP's issuer. Login alone grants access to one's
-- own rows; it does not grant sample-administration privileges.
CREATE TABLE user_files (
  id TEXT PRIMARY KEY,
  issuer TEXT NOT NULL,
  sub TEXT NOT NULL,
  object_key TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  size INTEGER NOT NULL,
  etag TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX user_files_owner_updated ON user_files(issuer, sub, updated_at DESC);

-- Previous object bodies kept on overwrite. Rows older than the service's
-- retention count are deleted together with their R2 objects.
CREATE TABLE user_file_versions (
  id TEXT NOT NULL,
  version_key TEXT NOT NULL PRIMARY KEY,
  size INTEGER NOT NULL,
  etag TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX user_file_versions_id_created ON user_file_versions(id, created_at DESC);
