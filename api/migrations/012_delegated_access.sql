-- Phase 3: an owner role, access limited to selected inventories, invitations that carry that
-- access, and requests for access that a manager decides.

ALTER TABLE memberships DROP CONSTRAINT memberships_role_check;
ALTER TABLE memberships ADD CONSTRAINT memberships_role_check
  CHECK (role IN ('owner', 'admin', 'manager', 'technician', 'auditor', 'viewer'));
ALTER TABLE auth_tokens DROP CONSTRAINT auth_tokens_role_check;
ALTER TABLE auth_tokens ADD CONSTRAINT auth_tokens_role_check
  CHECK (role IS NULL OR role IN ('owner', 'admin', 'manager', 'technician', 'auditor', 'viewer'));

-- Each workspace's longest-standing active admin becomes its owner. Owners keep every admin
-- permission, so nobody loses access.
UPDATE memberships m SET role = 'owner'
  FROM (SELECT DISTINCT ON (org_id) org_id, user_id FROM memberships
         WHERE role = 'admin' AND is_active ORDER BY org_id, created_at, user_id) first_admin
 WHERE m.org_id = first_admin.org_id AND m.user_id = first_admin.user_id;

-- 'all': everything in the workspace (the default, and the only choice for owners and admins).
-- 'inventories': only items in the granted inventories and their sub-inventories.
ALTER TABLE memberships
  ADD COLUMN access_scope text NOT NULL DEFAULT 'all' CHECK (access_scope IN ('all', 'inventories')),
  ADD CONSTRAINT memberships_admins_unscoped CHECK (access_scope = 'all' OR role NOT IN ('owner', 'admin'));

CREATE TABLE inventory_grants (
  org_id       uuid NOT NULL,
  user_id      uuid NOT NULL,
  inventory_id uuid NOT NULL,
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (org_id, user_id, inventory_id),
  FOREIGN KEY (org_id, user_id) REFERENCES memberships (org_id, user_id) ON DELETE CASCADE,
  FOREIGN KEY (org_id, inventory_id) REFERENCES inventories (org_id, id) ON DELETE CASCADE
);

ALTER TABLE auth_tokens
  ADD COLUMN access_scope  text CHECK (access_scope IN ('all', 'inventories')),
  ADD COLUMN inventory_ids uuid[];

CREATE TABLE access_requests (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id        uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  user_id       uuid NOT NULL,
  inventory_id  uuid NOT NULL,
  note          text,
  status        text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected', 'cancelled')),
  decided_by    uuid REFERENCES users(id),
  decided_at    timestamptz,
  decision_note text,
  created_at    timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, id),
  FOREIGN KEY (org_id, user_id) REFERENCES memberships (org_id, user_id) ON DELETE CASCADE,
  FOREIGN KEY (org_id, inventory_id) REFERENCES inventories (org_id, id) ON DELETE CASCADE,
  CHECK ((status = 'pending') = (decided_at IS NULL))
);
CREATE UNIQUE INDEX access_requests_one_pending ON access_requests (org_id, user_id, inventory_id) WHERE status = 'pending';
CREATE INDEX access_requests_org_idx ON access_requests (org_id, status, created_at DESC);
