-- Document Git repositories backed by Cloudflare Artifacts.
-- Ownership is bound to the Mikaki OIDC issuer + pairwise subject so each user
-- only accesses and manages their own repositories through the web hub.
CREATE TABLE doc_repos (
  id TEXT PRIMARY KEY,
  issuer TEXT NOT NULL,
  sub TEXT NOT NULL,
  repo_name TEXT NOT NULL UNIQUE,
  slug TEXT NOT NULL,
  title TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  default_branch TEXT NOT NULL DEFAULT 'main',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX doc_repos_owner_updated ON doc_repos(issuer, sub, updated_at DESC);
