-- Release 1: personal and business workspaces, people who belong to several workspaces,
-- one-time tokens (invitations, email verification, password reset), external sign-in
-- identities (Google) and an email outbox.

ALTER TABLE organizations
  ADD COLUMN kind     text NOT NULL DEFAULT 'business' CHECK (kind IN ('personal', 'business')),
  ADD COLUMN timezone text NOT NULL DEFAULT 'UTC';

-- A person's role in one workspace. Rows are never deleted, only deactivated, because history
-- (asset events, audits, work orders) keeps pointing at them.
CREATE TABLE memberships (
  org_id       uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  user_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role         text NOT NULL CHECK (role IN ('admin', 'manager', 'technician', 'auditor', 'viewer')),
  is_active    boolean NOT NULL DEFAULT true,
  created_at   timestamptz NOT NULL DEFAULT now(),
  last_used_at timestamptz,
  PRIMARY KEY (org_id, user_id)
);
CREATE INDEX memberships_user_idx ON memberships (user_id);

INSERT INTO memberships (org_id, user_id, role, is_active, created_at, last_used_at)
SELECT org_id, id, role, is_active, created_at, last_login_at FROM users;

-- Every "this user, in this organization" reference now points at a membership, so a person can
-- act in any workspace they belong to while rows still cannot cross organizations.
DO $$
DECLARE
  r record;
  def text;
BEGIN
  FOR r IN
    SELECT c.conname, c.conrelid::regclass AS tbl, pg_get_constraintdef(c.oid) AS def
      FROM pg_constraint c
     WHERE c.contype = 'f' AND c.confrelid = 'users'::regclass AND array_length(c.conkey, 1) = 2
  LOOP
    def := replace(r.def, 'REFERENCES users(org_id, id)', 'REFERENCES memberships(org_id, user_id)');
    IF def = r.def THEN
      RAISE EXCEPTION 'unexpected foreign key %: %', r.conname, r.def;
    END IF;
    EXECUTE format('ALTER TABLE %s DROP CONSTRAINT %I', r.tbl, r.conname);
    EXECUTE format('ALTER TABLE %s ADD CONSTRAINT %I %s', r.tbl, r.conname, def);
  END LOOP;
END $$;

-- Users are now accounts; workspace and role live on memberships. Deactivation in a workspace is
-- a membership change, so every account starts active.
ALTER TABLE users DROP CONSTRAINT users_org_id_id_key;
ALTER TABLE users DROP COLUMN org_id;
ALTER TABLE users DROP COLUMN role;
UPDATE users SET is_active = true;
ALTER TABLE users
  ALTER COLUMN password_hash DROP NOT NULL,
  ADD COLUMN email_verified_at timestamptz,
  ADD COLUMN email_notifications boolean NOT NULL DEFAULT true;

-- One-time links. Only a hash of the token is stored; a token works once and until it expires.
CREATE TABLE auth_tokens (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  purpose    text NOT NULL CHECK (purpose IN ('verify_email', 'reset_password', 'invite')),
  token_hash bytea NOT NULL UNIQUE,
  user_id    uuid REFERENCES users(id) ON DELETE CASCADE,
  org_id     uuid REFERENCES organizations(id) ON DELETE CASCADE,
  email      citext,
  role       text CHECK (role IS NULL OR role IN ('admin', 'manager', 'technician', 'auditor', 'viewer')),
  created_by uuid REFERENCES users(id),
  expires_at timestamptz NOT NULL,
  used_at    timestamptz,
  revoked_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (purpose <> 'invite' OR (org_id IS NOT NULL AND email IS NOT NULL AND role IS NOT NULL))
);
CREATE INDEX auth_tokens_org_idx ON auth_tokens (org_id, purpose) WHERE used_at IS NULL AND revoked_at IS NULL;

-- Accounts at external identity providers, keyed by the provider's stable subject identifier.
CREATE TABLE user_identities (
  provider   text NOT NULL,
  subject    text NOT NULL,
  user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  email      citext,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (provider, subject),
  UNIQUE (user_id, provider)
);

-- In-flight sign-in redirects. Each state works once, which stops forged and replayed callbacks.
CREATE TABLE oauth_states (
  state_hash    bytea PRIMARY KEY,
  provider      text NOT NULL,
  mode          text NOT NULL CHECK (mode IN ('login', 'link')),
  nonce         text NOT NULL,
  pkce_verifier text NOT NULL,
  user_id       uuid REFERENCES users(id) ON DELETE CASCADE,
  expires_at    timestamptz NOT NULL,
  used_at       timestamptz
);

-- Every email goes through here; a worker delivers it and records what the provider said.
-- 'sent' is only ever set after the provider accepted the message.
CREATE TABLE email_outbox (
  id                  bigserial PRIMARY KEY,
  org_id              uuid REFERENCES organizations(id) ON DELETE CASCADE,
  user_id             uuid REFERENCES users(id) ON DELETE CASCADE,
  to_address          citext NOT NULL,
  subject             text NOT NULL,
  body_text           text NOT NULL,
  kind                text NOT NULL,
  dedupe_key          text UNIQUE,
  status              text NOT NULL DEFAULT 'queued'
                      CHECK (status IN ('queued', 'sending', 'sent', 'failed', 'unavailable')),
  attempts            integer NOT NULL DEFAULT 0,
  next_attempt_at     timestamptz NOT NULL DEFAULT now(),
  locked_at           timestamptz,
  last_error          text,
  provider_message_id text,
  sent_at             timestamptz,
  created_at          timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX email_outbox_due_idx ON email_outbox (next_attempt_at) WHERE status = 'queued';
