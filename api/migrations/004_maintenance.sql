-- Phase 6: maintenance work orders with an append-only note trail.

ALTER TABLE organizations ADD COLUMN next_work_order_number integer NOT NULL DEFAULT 1001;

CREATE TABLE work_orders (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id        uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  number        text NOT NULL,
  asset_id      uuid NOT NULL,
  title         text NOT NULL CHECK (length(trim(title)) > 0),
  description   text,
  priority      text NOT NULL DEFAULT 'normal' CHECK (priority IN ('low', 'normal', 'high', 'urgent')),
  status        text NOT NULL DEFAULT 'open'
                CHECK (status IN ('open', 'in_progress', 'waiting_parts', 'resolved', 'cancelled')),
  assignee_id   uuid,
  vendor        text,
  cost          numeric(12, 2) CHECK (cost IS NULL OR cost >= 0),
  due_date      date,
  resolution    text,
  opened_by     uuid NOT NULL,
  opened_at     timestamptz NOT NULL DEFAULT now(),
  closed_at     timestamptz,
  updated_at    timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, number),
  UNIQUE (org_id, id),
  FOREIGN KEY (org_id, asset_id) REFERENCES assets (org_id, id),
  FOREIGN KEY (org_id, assignee_id) REFERENCES users (org_id, id),
  FOREIGN KEY (org_id, opened_by) REFERENCES users (org_id, id),
  CHECK ((status IN ('resolved', 'cancelled')) = (closed_at IS NOT NULL))
);
CREATE INDEX work_orders_org_status_idx ON work_orders (org_id, status);
CREATE INDEX work_orders_asset_idx ON work_orders (asset_id);

CREATE TABLE work_order_notes (
  id            bigserial PRIMARY KEY,
  org_id        uuid NOT NULL,
  work_order_id uuid NOT NULL,
  author_id     uuid,
  kind          text NOT NULL DEFAULT 'note' CHECK (kind IN ('note', 'status')),
  body          text NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (org_id, work_order_id) REFERENCES work_orders (org_id, id),
  FOREIGN KEY (org_id, author_id) REFERENCES users (org_id, id)
);
CREATE INDEX work_order_notes_wo_idx ON work_order_notes (work_order_id, id);

CREATE FUNCTION append_only() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION '% is append-only', TG_TABLE_NAME;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER work_order_notes_no_update_delete
  BEFORE UPDATE OR DELETE ON work_order_notes
  FOR EACH ROW EXECUTE FUNCTION append_only();
