-- Phase 3: people that equipment is assigned to, asset assignment, and parent-child assets
-- (for example a workstation with its monitor, scanner and dock).

CREATE TABLE people (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id      uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  name        text NOT NULL CHECK (length(trim(name)) > 0),
  email       citext,
  employee_id text,
  department  text,
  is_active   boolean NOT NULL DEFAULT true,
  created_at  timestamptz NOT NULL DEFAULT now(),
  updated_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, id)
);
CREATE UNIQUE INDEX people_org_email_uniq ON people (org_id, email) WHERE email IS NOT NULL;
CREATE UNIQUE INDEX people_org_employee_uniq ON people (org_id, employee_id) WHERE employee_id IS NOT NULL;

ALTER TABLE assets ADD CONSTRAINT assets_org_id_id_key UNIQUE (org_id, id);

ALTER TABLE assets
  ADD COLUMN parent_asset_id    uuid,
  ADD COLUMN assigned_person_id uuid,
  ADD COLUMN assigned_at        timestamptz,
  ADD CONSTRAINT assets_parent_fk FOREIGN KEY (org_id, parent_asset_id) REFERENCES assets (org_id, id),
  ADD CONSTRAINT assets_person_fk FOREIGN KEY (org_id, assigned_person_id) REFERENCES people (org_id, id),
  ADD CONSTRAINT assets_not_own_parent CHECK (parent_asset_id IS NULL OR parent_asset_id <> id);

CREATE INDEX assets_parent_idx ON assets (parent_asset_id) WHERE parent_asset_id IS NOT NULL;
CREATE INDEX assets_person_idx ON assets (assigned_person_id) WHERE assigned_person_id IS NOT NULL;
