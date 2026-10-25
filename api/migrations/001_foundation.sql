-- Phase 1 foundation: organizations, users and roles, buildings and locations,
-- assets, and an append-only asset event history.

CREATE EXTENSION IF NOT EXISTS citext;

CREATE TABLE organizations (
  id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name              text NOT NULL CHECK (length(trim(name)) > 0),
  next_asset_number integer NOT NULL DEFAULT 1001,
  created_at        timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE users (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id        uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  email         citext NOT NULL UNIQUE,
  name          text NOT NULL,
  password_hash text NOT NULL,
  role          text NOT NULL CHECK (role IN ('admin', 'manager', 'technician', 'auditor', 'viewer')),
  is_active     boolean NOT NULL DEFAULT true,
  -- Bumped on password change or reset; older sessions stop working.
  session_version integer NOT NULL DEFAULT 1,
  last_login_at timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, id)
);
CREATE INDEX users_org_idx ON users (org_id);

CREATE TABLE buildings (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id     uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  name       text NOT NULL CHECK (length(trim(name)) > 0),
  address    text,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name),
  UNIQUE (org_id, id)
);

CREATE TABLE locations (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id      uuid NOT NULL,
  building_id uuid NOT NULL,
  name        text NOT NULL CHECK (length(trim(name)) > 0),
  created_at  timestamptz NOT NULL DEFAULT now(),
  -- Composite keys keep a location inside its own organization and building.
  FOREIGN KEY (org_id, building_id) REFERENCES buildings (org_id, id) ON DELETE CASCADE,
  UNIQUE (building_id, name),
  UNIQUE (org_id, building_id, id)
);

CREATE TABLE assets (
  id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id           uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  asset_tag        text NOT NULL,
  name             text NOT NULL CHECK (length(trim(name)) > 0),
  type             text NOT NULL,
  serial           text,
  manufacturer     text,
  model            text,
  status           text NOT NULL DEFAULT 'Available'
                   CHECK (status IN ('Available', 'Deployed', 'Maintenance', 'Retired')),
  building_id      uuid,
  location_id      uuid,
  notes            text,
  purchase_date    date,
  warranty_expires date,
  created_by       uuid,
  created_at       timestamptz NOT NULL DEFAULT now(),
  updated_at       timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, asset_tag),
  FOREIGN KEY (org_id, building_id) REFERENCES buildings (org_id, id),
  -- A location, when set, must belong to the asset's building.
  FOREIGN KEY (org_id, building_id, location_id) REFERENCES locations (org_id, building_id, id),
  FOREIGN KEY (org_id, created_by) REFERENCES users (org_id, id),
  CHECK (location_id IS NULL OR building_id IS NOT NULL)
);
CREATE UNIQUE INDEX assets_org_serial_uniq ON assets (org_id, lower(serial)) WHERE serial IS NOT NULL;
CREATE INDEX assets_org_status_idx ON assets (org_id, status);
CREATE INDEX assets_org_building_idx ON assets (org_id, building_id);

-- Every change to an asset is recorded here. Rows can never be edited or removed,
-- which is what later audit and reporting phases rely on.
CREATE TABLE asset_events (
  id         bigserial PRIMARY KEY,
  org_id     uuid NOT NULL REFERENCES organizations(id),
  asset_id   uuid NOT NULL,
  actor_id   uuid,
  event_type text NOT NULL,
  changes    jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (asset_id) REFERENCES assets (id),
  FOREIGN KEY (org_id, actor_id) REFERENCES users (org_id, id)
);
CREATE INDEX asset_events_asset_idx ON asset_events (asset_id, id);

CREATE FUNCTION asset_events_immutable() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION 'asset_events is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER asset_events_no_update_delete
  BEFORE UPDATE OR DELETE ON asset_events
  FOR EACH ROW EXECUTE FUNCTION asset_events_immutable();
