-- Phase 4: loaners. A loan follows Reserved → Deployed → Returned → Inspected (the item is then
-- available again), with exceptions: a reservation can be cancelled, a deployed loan extended or
-- reported lost, and an inspection can send the item for repair. Every step is a row in
-- loan_events, which never changes.

ALTER TABLE organizations ADD COLUMN next_loan_number integer NOT NULL DEFAULT 1001;

CREATE TABLE loans (
  id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id           uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  number           text NOT NULL,
  asset_id         uuid NOT NULL,
  person_id        uuid NOT NULL,
  status           text NOT NULL CHECK (status IN ('reserved', 'deployed', 'returned', 'inspected', 'cancelled', 'lost')),
  starts_on        date NOT NULL,
  due_on           date NOT NULL,
  purpose          text,
  reserved_by      uuid NOT NULL,
  reserved_at      timestamptz NOT NULL DEFAULT now(),
  deployed_by      uuid,
  deployed_at      timestamptz,
  -- The borrower's acknowledgment at hand-over: the name they confirmed and when.
  acknowledged_name text,
  acknowledged_at  timestamptz,
  checkout_condition text,
  returned_by      uuid,
  returned_at      timestamptz,
  return_condition text,
  return_note      text,
  damage_reported  boolean NOT NULL DEFAULT false,
  inspected_by     uuid,
  inspected_at     timestamptz,
  inspection_result text CHECK (inspection_result IN ('ready', 'repair')),
  inspection_note  text,
  closed_at        timestamptz,
  extensions       integer NOT NULL DEFAULT 0,
  UNIQUE (org_id, id),
  UNIQUE (org_id, number),
  FOREIGN KEY (org_id, asset_id) REFERENCES assets (org_id, id),
  FOREIGN KEY (org_id, person_id) REFERENCES people (org_id, id),
  FOREIGN KEY (org_id, reserved_by) REFERENCES memberships (org_id, user_id),
  CHECK (due_on >= starts_on),
  CHECK ((status IN ('inspected', 'cancelled', 'lost')) = (closed_at IS NOT NULL)),
  CHECK (status = 'reserved' OR status = 'cancelled' OR deployed_at IS NOT NULL),
  CHECK (status NOT IN ('returned', 'inspected') OR returned_at IS NOT NULL),
  CHECK (status <> 'inspected' OR inspected_at IS NOT NULL)
);
-- An item is on at most one open loan.
CREATE UNIQUE INDEX loans_one_open ON loans (asset_id) WHERE status IN ('reserved', 'deployed', 'returned');
CREATE INDEX loans_org_idx ON loans (org_id, status, due_on);
CREATE INDEX loans_person_idx ON loans (org_id, person_id);

CREATE TABLE loan_events (
  id         bigserial PRIMARY KEY,
  org_id     uuid NOT NULL,
  loan_id    uuid NOT NULL,
  kind       text NOT NULL CHECK (kind IN ('reserved', 'deployed', 'extended', 'returned', 'inspected', 'cancelled', 'lost', 'overdue')),
  actor_id   uuid REFERENCES users(id),
  details    jsonb NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (org_id, loan_id) REFERENCES loans (org_id, id)
);
CREATE INDEX loan_events_loan_idx ON loan_events (loan_id, id);

-- History is permanent, and a closed loan never changes.
CREATE FUNCTION loan_events_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'loan_events is append-only';
END $$;
CREATE TRIGGER loan_events_append_only BEFORE UPDATE OR DELETE ON loan_events
  FOR EACH ROW EXECUTE FUNCTION loan_events_append_only();

CREATE FUNCTION loans_closed_final() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    RAISE EXCEPTION 'loans are never deleted';
  END IF;
  IF OLD.closed_at IS NOT NULL THEN
    RAISE EXCEPTION 'a closed loan cannot change';
  END IF;
  IF NEW.asset_id <> OLD.asset_id OR NEW.number <> OLD.number OR NEW.reserved_by <> OLD.reserved_by THEN
    RAISE EXCEPTION 'a loan cannot be moved to another item';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER loans_closed_final BEFORE UPDATE OR DELETE ON loans
  FOR EACH ROW EXECUTE FUNCTION loans_closed_final();
