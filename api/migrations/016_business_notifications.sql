-- Phase 7: per-person choices for workspace event notifications, by group (transfers, audits,
-- loans, ...). A row overrides the workspace default (organizations.settings.notifications) for
-- that person in that workspace; no row means the workspace default applies.

CREATE TABLE notification_preferences (
  user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  org_id     uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  grp        text NOT NULL CHECK (grp IN ('transfers', 'access', 'audits', 'missing', 'loans', 'assignments', 'tasks', 'invitations')),
  in_app     boolean NOT NULL,
  email      boolean NOT NULL,
  push       boolean NOT NULL,
  sms        boolean NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, org_id, grp)
);

-- Whether a member may see an item: full access, or the item's inventory is inside one they were
-- granted. Event notifications about an item go only to members who may see it.
CREATE FUNCTION member_sees_asset(org uuid, member uuid, asset uuid) RETURNS boolean LANGUAGE sql STABLE AS $$
  SELECT COALESCE((SELECT m.access_scope = 'all' FROM memberships m WHERE m.org_id = org AND m.user_id = member), false)
      OR EXISTS (
        WITH RECURSIVE t AS (
          SELECT g.inventory_id AS id, 0 AS depth FROM inventory_grants g WHERE g.org_id = org AND g.user_id = member
          UNION SELECT c.id, t.depth + 1 FROM inventories c JOIN t ON c.parent_id = t.id WHERE t.depth < 20)
        SELECT 1 FROM t JOIN assets a ON a.inventory_id = t.id WHERE a.id = asset)
$$;
