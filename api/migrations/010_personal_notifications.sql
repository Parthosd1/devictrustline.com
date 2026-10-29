-- Phase 1: personal device management and user-managed notifications.
-- Items can be archived (hidden, restorable) separately from being retired. Each person sets
-- their own notification preferences and their own reminders; reminder occurrences are persisted
-- as jobs, and every channel delivery (email, browser push, SMS) is logged with its outcome.

ALTER TABLE assets ADD COLUMN archived_at timestamptz;
CREATE INDEX assets_org_active_idx ON assets (org_id) WHERE archived_at IS NULL;

-- Personal workspaces get categories for everyday electronics as well.
INSERT INTO categories (org_id, name)
SELECT o.id, t.name
  FROM organizations o
 CROSS JOIN unnest(ARRAY['Laptops & Computers', 'Phones & Tablets', 'TVs & Monitors', 'Printers', 'Routers & Networking',
                         'Gaming', 'Accessories']) AS t(name)
 WHERE o.kind = 'personal'
ON CONFLICT DO NOTHING;

-- One row per account. The email channel stays users.email_notifications, which already exists.
CREATE TABLE notification_settings (
  user_id         uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  enabled         boolean NOT NULL DEFAULT true,
  push            boolean NOT NULL DEFAULT false,
  sms             boolean NOT NULL DEFAULT false,
  phone           text CHECK (phone IS NULL OR phone ~ '^\+[1-9][0-9]{6,14}$'),
  -- Null means the workspace's time zone.
  timezone        text,
  quiet_start     time,
  quiet_end       time,
  -- Local hour at which day-based reminders are delivered.
  send_hour       smallint NOT NULL DEFAULT 9 CHECK (send_hour BETWEEN 0 AND 23),
  default_offsets integer[] NOT NULL DEFAULT '{7,1}',
  updated_at      timestamptz NOT NULL DEFAULT now(),
  CHECK ((quiet_start IS NULL) = (quiet_end IS NULL))
);

-- Browser push subscriptions (Web Push). Endpoints are unique per browser.
CREATE TABLE push_subscriptions (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id         uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  endpoint        text NOT NULL UNIQUE CHECK (endpoint ~ '^https?://'),
  p256dh          text NOT NULL,
  auth            text NOT NULL,
  user_agent      text,
  created_at      timestamptz NOT NULL DEFAULT now(),
  last_success_at timestamptz
);
CREATE INDEX push_subscriptions_user_idx ON push_subscriptions (user_id);

-- Reminders a person sets for themselves. A reminder may follow a date on an item (its warranty,
-- return deadline, maintenance date or loan due date); then the due date tracks that date.
CREATE TABLE user_reminders (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id        uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  asset_id      uuid,
  kind          text NOT NULL CHECK (kind IN ('warranty', 'return', 'maintenance', 'renewal', 'inspection', 'loan_return', 'custom')),
  title         text NOT NULL CHECK (length(trim(title)) BETWEEN 1 AND 160),
  notes         text,
  date_source   text CHECK (date_source IN ('warranty', 'return_deadline', 'maintenance_due', 'loan_due')),
  due_on        date NOT NULL,
  offsets       integer[] NOT NULL CHECK (cardinality(offsets) BETWEEN 1 AND 8 AND 0 <= ALL (offsets) AND 366 > ALL (offsets)),
  repeat        text NOT NULL DEFAULT 'none' CHECK (repeat IN ('none', 'daily', 'weekly', 'monthly', 'yearly')),
  repeat_every  integer NOT NULL DEFAULT 1 CHECK (repeat_every BETWEEN 1 AND 365),
  repeat_until  date,
  status        text NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'paused', 'completed')),
  snoozed_until timestamptz,
  completed_at  timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, id),
  FOREIGN KEY (org_id, asset_id) REFERENCES assets (org_id, id) ON DELETE CASCADE,
  CHECK (date_source IS NULL OR asset_id IS NOT NULL),
  CHECK ((status = 'completed') = (completed_at IS NOT NULL))
);
CREATE INDEX user_reminders_user_idx ON user_reminders (user_id, org_id, status, due_on);
CREATE INDEX user_reminders_asset_idx ON user_reminders (asset_id) WHERE asset_id IS NOT NULL;

-- Persisted reminder occurrences. The dedupe key (reminder, due date, days before; or a snooze
-- time) makes each occurrence exist once, so a reminder can never be delivered twice.
CREATE TABLE reminder_jobs (
  id              bigserial PRIMARY KEY,
  org_id          uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  reminder_id     uuid NOT NULL REFERENCES user_reminders(id) ON DELETE CASCADE,
  dedupe_key      text NOT NULL UNIQUE,
  due_on          date NOT NULL,
  days_before     integer,
  fire_at         timestamptz NOT NULL,
  status          text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'sent', 'cancelled', 'skipped', 'failed')),
  attempts        integer NOT NULL DEFAULT 0,
  last_error      text,
  notification_id bigint,
  fired_at        timestamptz,
  created_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX reminder_jobs_due_idx ON reminder_jobs (fire_at) WHERE status = 'pending';
CREATE INDEX reminder_jobs_reminder_idx ON reminder_jobs (reminder_id, id);

ALTER TABLE notifications
  ADD COLUMN user_reminder_id uuid REFERENCES user_reminders(id) ON DELETE SET NULL,
  ADD COLUMN dedupe_key       text UNIQUE,
  ADD COLUMN link             text;
ALTER TABLE reminder_jobs
  ADD CONSTRAINT reminder_jobs_notification_fk FOREIGN KEY (notification_id) REFERENCES notifications(id) ON DELETE SET NULL;

-- Every attempt to deliver a notification outside the app. Email rows point at the email outbox,
-- which has its own retrying worker; push and SMS rows are delivered by the notification worker.
CREATE TABLE notification_deliveries (
  id              bigserial PRIMARY KEY,
  notification_id bigint NOT NULL REFERENCES notifications(id) ON DELETE CASCADE,
  user_id         uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  channel         text NOT NULL CHECK (channel IN ('email', 'push', 'sms')),
  subscription_id uuid REFERENCES push_subscriptions(id) ON DELETE SET NULL,
  email_id        bigint REFERENCES email_outbox(id) ON DELETE SET NULL,
  status          text NOT NULL DEFAULT 'queued'
                  CHECK (status IN ('queued', 'sending', 'sent', 'failed', 'unavailable')),
  attempts        integer NOT NULL DEFAULT 0,
  next_attempt_at timestamptz NOT NULL DEFAULT now(),
  locked_at       timestamptz,
  last_error      text,
  provider_ref    text,
  sent_at         timestamptz,
  created_at      timestamptz NOT NULL DEFAULT now()
);
-- One email and one text message per notification, one push per notification and browser.
CREATE UNIQUE INDEX notification_deliveries_once ON notification_deliveries (notification_id, channel) WHERE channel <> 'push';
CREATE UNIQUE INDEX notification_deliveries_push_once ON notification_deliveries (notification_id, subscription_id)
  WHERE subscription_id IS NOT NULL;
CREATE UNIQUE INDEX notification_deliveries_push_unavailable ON notification_deliveries (notification_id)
  WHERE channel = 'push' AND status = 'unavailable';
CREATE INDEX notification_deliveries_due_idx ON notification_deliveries (next_attempt_at) WHERE status = 'queued';

-- When a message may go out: `at`, or the end of the person's quiet hours if `at` falls inside them.
CREATE FUNCTION quiet_hours_end(tz text, qs time, qe time, at timestamptz) RETURNS timestamptz
LANGUAGE sql STABLE AS $$
  SELECT CASE
    WHEN qs IS NULL OR qe IS NULL OR qs = qe THEN at
    WHEN qs < qe AND (at AT TIME ZONE tz)::time >= qs AND (at AT TIME ZONE tz)::time < qe
      THEN ((at AT TIME ZONE tz)::date + qe) AT TIME ZONE tz
    WHEN qs > qe AND (at AT TIME ZONE tz)::time >= qs
      THEN ((at AT TIME ZONE tz)::date + 1 + qe) AT TIME ZONE tz
    WHEN qs > qe AND (at AT TIME ZONE tz)::time < qe
      THEN ((at AT TIME ZONE tz)::date + qe) AT TIME ZONE tz
    ELSE at
  END
$$;
