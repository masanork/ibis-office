-- Preserve one-use semantics even if an invitation row survives an ambiguous
-- result or is mistakenly restored. Operator-created administrator rows may
-- leave invite_hash NULL.
ALTER TABLE sample_admin ADD COLUMN invite_hash TEXT;
CREATE UNIQUE INDEX sample_admin_invite_hash ON sample_admin(invite_hash);
