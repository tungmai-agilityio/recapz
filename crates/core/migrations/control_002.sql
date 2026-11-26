-- Workspaces: one person can belong to several. Additive only: users.tenant_id/role, invites and
-- existing sessions stay as they are (old sessions fall back to users.tenant_id).
CREATE TABLE memberships (
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  tenant_id TEXT NOT NULL REFERENCES tenants(id),
  role TEXT NOT NULL CHECK (role IN ('admin','member')),
  created_at INTEGER NOT NULL,
  PRIMARY KEY (user_id, tenant_id)
);
CREATE TABLE workspace_invites (
  email TEXT NOT NULL,
  tenant_id TEXT NOT NULL REFERENCES tenants(id),
  role TEXT NOT NULL CHECK (role IN ('admin','member')),
  invited_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (email, tenant_id)
);
ALTER TABLE sessions ADD COLUMN tenant_id TEXT;
INSERT INTO memberships (user_id, tenant_id, role, created_at)
  SELECT id, tenant_id, role, CAST(strftime('%s','now') AS INTEGER) * 1000 FROM users;
INSERT INTO workspace_invites (email, tenant_id, role, invited_by, created_at)
  SELECT email, tenant_id, role, NULL, CAST(strftime('%s','now') AS INTEGER) * 1000 FROM invites;
