-- Daily request counts for the browser assistant. Prompts and completions are not stored.
CREATE TABLE ai_daily_usage (
  issuer TEXT NOT NULL,
  sub TEXT NOT NULL,
  day TEXT NOT NULL,
  requests INTEGER NOT NULL,
  PRIMARY KEY (issuer, sub, day)
);
