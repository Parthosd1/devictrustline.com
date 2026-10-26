-- Release 1: the device companion. A computer is paired to one workspace with a one-time code
-- and gets its own revocable credential. What it finds waits for review; nothing becomes an
-- item until a person adds or links it.

CREATE TABLE device_agents (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id        uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  name          text NOT NULL,
  platform      text NOT NULL CHECK (platform IN ('windows', 'linux', 'macos')),
  agent_version text,
  token_hash    bytea NOT NULL UNIQUE,
  paused        boolean NOT NULL DEFAULT false,
  created_by    uuid REFERENCES users(id),
  created_at    timestamptz NOT NULL DEFAULT now(),
  last_seen_at  timestamptz,
  revoked_at    timestamptz,
  UNIQUE (org_id, id)
);

CREATE TABLE agent_pairing_codes (
  code_hash  bytea PRIMARY KEY,
  org_id     uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  created_by uuid REFERENCES users(id),
  expires_at timestamptz NOT NULL,
  used_at    timestamptz
);

CREATE TABLE discovered_devices (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id        uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  agent_id      uuid NOT NULL,
  -- How the same device is recognized again: its reported serial (with maker and class), or
  -- failing that the reporting computer plus the device's local id. Never the model name.
  fingerprint   text NOT NULL,
  class         text NOT NULL,
  name          text NOT NULL,
  manufacturer  text,
  model         text,
  serial        text,
  local_id      text NOT NULL,
  details       jsonb NOT NULL DEFAULT '{}'::jsonb,
  state         text NOT NULL DEFAULT 'new' CHECK (state IN ('new', 'linked', 'ignored')),
  asset_id      uuid,
  first_seen_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at  timestamptz NOT NULL DEFAULT now(),
  missing_since timestamptz,
  UNIQUE (org_id, fingerprint),
  FOREIGN KEY (org_id, agent_id) REFERENCES device_agents (org_id, id) ON DELETE CASCADE,
  FOREIGN KEY (org_id, asset_id) REFERENCES assets (org_id, id) ON DELETE SET NULL (asset_id)
);
CREATE INDEX discovered_devices_agent_idx ON discovered_devices (agent_id);
CREATE INDEX discovered_devices_asset_idx ON discovered_devices (asset_id) WHERE asset_id IS NOT NULL;
