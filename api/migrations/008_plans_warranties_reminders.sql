-- Release 1: plans and their limits, warranties, loans, reminders and in-app notifications.

-- Plans: limits are enforced by the API and can be changed by the operator at any time
-- (devicetrustline plans / set-plan-limit). A null limit means no limit. Prices are not stored
-- here; they belong to the billing provider once one is connected.
CREATE TABLE plan_definitions (
  code       text PRIMARY KEY,
  name       text NOT NULL,
  kind       text NOT NULL CHECK (kind IN ('personal', 'business')),
  limits     jsonb NOT NULL DEFAULT '{}'::jsonb,
  position   integer NOT NULL DEFAULT 0,
  updated_at timestamptz NOT NULL DEFAULT now()
);
INSERT INTO plan_definitions (code, name, kind, limits, position) VALUES
  ('personal_basic', 'Personal Basic', 'personal', '{"items": 100, "members": 2, "storageMb": 500, "devices": 2}', 1),
  ('personal_plus', 'Personal Plus', 'personal', '{"items": 2000, "members": 6, "storageMb": 10240, "devices": 10}', 2),
  ('business', 'Business', 'business', '{"items": null, "members": null, "storageMb": 102400, "devices": null}', 3);

ALTER TABLE organizations ADD COLUMN plan text REFERENCES plan_definitions(code);
UPDATE organizations SET plan = CASE kind WHEN 'personal' THEN 'personal_basic' ELSE 'business' END;
ALTER TABLE organizations ALTER COLUMN plan SET NOT NULL;

-- Warranties: several per item (manufacturer, extended, ...). An unknown end date is allowed.
-- assets.warranty_expires is kept as the latest end date across an item's warranties.
CREATE TABLE warranties (
  id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id              uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  asset_id            uuid NOT NULL,
  kind                text NOT NULL DEFAULT 'manufacturer' CHECK (kind IN ('manufacturer', 'extended', 'other')),
  provider            text,
  starts_on           date,
  ends_on             date,
  coverage            text,
  claim_reference     text,
  proof_attachment_id uuid,
  created_at          timestamptz NOT NULL DEFAULT now(),
  updated_at          timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, id),
  FOREIGN KEY (org_id, asset_id) REFERENCES assets (org_id, id) ON DELETE CASCADE,
  FOREIGN KEY (org_id, proof_attachment_id) REFERENCES attachments (org_id, id) ON DELETE SET NULL (proof_attachment_id),
  CHECK (ends_on IS NULL OR starts_on IS NULL OR ends_on >= starts_on)
);
CREATE INDEX warranties_asset_idx ON warranties (org_id, asset_id);
CREATE INDEX warranties_ends_idx ON warranties (org_id, ends_on) WHERE ends_on IS NOT NULL;

INSERT INTO warranties (org_id, asset_id, kind, ends_on)
SELECT org_id, id, 'manufacturer', warranty_expires FROM assets WHERE warranty_expires IS NOT NULL;

CREATE FUNCTION warranties_sync_asset() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  target uuid := COALESCE(NEW.asset_id, OLD.asset_id);
BEGIN
  UPDATE assets SET warranty_expires = (SELECT max(ends_on) FROM warranties WHERE asset_id = target)
   WHERE id = target;
  RETURN NULL;
END $$;
CREATE TRIGGER warranties_sync_asset AFTER INSERT OR UPDATE OR DELETE ON warranties
  FOR EACH ROW EXECUTE FUNCTION warranties_sync_asset();

-- Loans: when an assigned item is due back.
ALTER TABLE assets ADD COLUMN loan_due date;

-- Reminders are derived from the data (warranty end dates, return windows, maintenance, audits,
-- loans) by a job that runs on every server. The dedupe key includes the deadline, so a changed
-- date cancels the old reminder and creates a new one, and a reminder is never created twice.
CREATE TABLE reminders (
  id          bigserial PRIMARY KEY,
  org_id      uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  asset_id    uuid,
  kind        text NOT NULL CHECK (kind IN ('warranty', 'return_window', 'maintenance', 'audit', 'loan')),
  source_id   text NOT NULL,
  due_on      date NOT NULL,
  remind_on   date NOT NULL,
  days_before integer NOT NULL,
  title       text NOT NULL,
  dedupe_key  text NOT NULL UNIQUE,
  status      text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'sent', 'cancelled', 'missed')),
  fired_at    timestamptz,
  created_at  timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (org_id, asset_id) REFERENCES assets (org_id, id) ON DELETE CASCADE
);
CREATE INDEX reminders_due_idx ON reminders (remind_on) WHERE status = 'pending';
CREATE INDEX reminders_org_idx ON reminders (org_id, status, due_on);

CREATE TABLE notifications (
  id          bigserial PRIMARY KEY,
  org_id      uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  user_id     uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  reminder_id bigint REFERENCES reminders(id) ON DELETE SET NULL,
  asset_id    uuid,
  kind        text NOT NULL,
  title       text NOT NULL,
  body        text NOT NULL,
  email_id    bigint REFERENCES email_outbox(id) ON DELETE SET NULL,
  read_at     timestamptz,
  created_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (reminder_id, user_id)
);
CREATE INDEX notifications_user_idx ON notifications (user_id, org_id, created_at DESC);
